//! Speaker-mix tap — Pulse monitor of the sink **omaradio is playing into**.
//!
//! Live PipeWire graph (2026-09-15): `pw-record --target {sink}.monitor` does
//! **not** connect to a monitor. There is no PW node named `*.monitor`. The
//! running recorders were linked to the default **microphone** (Razer Kraken
//! source) while mpv played to SPDIF. Bars then ignored the radio.
//!
//! `pacat --record --device={sink}.monitor` **does** capture the mix
//! (play RMS ~0.10 on the SPDIF monitor in the same session).
//!
//! Rules:
//! - Primary: Pulse `pacat` on `{omaradio_sink}.monitor`
//! - Never pass `*.monitor` to `pw-record --target`
//! - Never treat a live child as proof of audio — HUD shows RMS
//! - Follow the omaradio sink-input, not a one-shot default sink

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const RING: usize = 16_384;
const SAMPLE_RATE: u32 = 44_100;
const RETARGET_EVERY: Duration = Duration::from_millis(1000);
const SILENT_RESTART: Duration = Duration::from_secs(1);
const MAX_RESTARTS_PER_SEC: usize = 2;

#[derive(Clone, Debug, Default)]
pub struct MixHud {
    pub sink: String,
    pub backend: String,
    pub rms: f32,
    pub peak_bar: usize,
    pub fft_ok: bool,
    pub err: String,
    pub samples: usize,
}

impl MixHud {
    pub fn line(&self) -> String {
        let sink = short_sink(&self.sink);
        let fft = if self.fft_ok { "ok" } else { "--" };
        let err = if self.err.is_empty() {
            String::new()
        } else {
            format!("  {}", self.err)
        };
        format!(
            " {sink}  {}  rms={:.3}  peak={}  fft={fft}{err} ",
            self.backend, self.rms, self.peak_bar
        )
    }
}

pub fn short_sink(sink: &str) -> String {
    sink.rsplit(['.', '/'])
        .next()
        .filter(|s| !s.is_empty() && *s != "monitor")
        .unwrap_or(sink)
        .chars()
        .take(28)
        .collect()
}

pub struct OutputTap {
    proc: Option<TapProc>,
    last_check: Instant,
    started: Instant,
    got_audio: bool,
    backend: u32,
    mpv_pid: Option<u32>,
    want_audio: bool,
    restarts: VecDeque<Instant>,
    last_rms: f32,
    last_samples: usize,
    pub sample_rate: u32,
}

struct TapProc {
    child: Child,
    stop: Arc<AtomicBool>,
    buf: Arc<Mutex<Vec<f32>>>,
    err: Arc<Mutex<String>>,
    thread: Option<JoinHandle<()>>,
    err_thread: Option<JoinHandle<()>>,
    sink: String,
    backend: &'static str,
}

impl OutputTap {
    pub fn new() -> Self {
        let mut tap = Self {
            proc: None,
            last_check: Instant::now() - RETARGET_EVERY,
            started: Instant::now(),
            got_audio: false,
            backend: 0,
            mpv_pid: None,
            want_audio: true,
            restarts: VecDeque::new(),
            last_rms: 0.0,
            last_samples: 0,
            sample_rate: SAMPLE_RATE,
        };
        tap.ensure();
        tap
    }

    pub fn bind_pid(&mut self, pid: u32) {
        self.mpv_pid = Some(pid);
        self.last_check = Instant::now() - RETARGET_EVERY;
    }

    pub fn set_want_audio(&mut self, want: bool) {
        self.want_audio = want;
    }

    pub fn active(&self) -> bool {
        self.proc.is_some()
    }

    pub fn sink_name(&self) -> Option<&str> {
        self.proc.as_ref().map(|p| p.sink.as_str())
    }

    pub fn backend_name(&self) -> &'static str {
        self.proc.as_ref().map(|p| p.backend).unwrap_or("-")
    }

    pub fn last_err(&self) -> String {
        self.proc
            .as_ref()
            .and_then(|p| p.err.lock().ok().map(|e| e.clone()))
            .unwrap_or_default()
    }

    pub fn last_rms(&self) -> f32 {
        self.last_rms
    }

    pub fn drain(&mut self, dest: &mut Vec<f32>) {
        self.ensure();
        dest.clear();
        if let Some(proc) = &self.proc {
            if let Ok(mut ring) = proc.buf.lock() {
                dest.extend_from_slice(&ring);
                ring.clear();
            }
        }
        self.last_samples = dest.len();
        self.last_rms = rms_of(dest);
        if !dest.is_empty() {
            self.got_audio = true;
        }
    }

    fn ensure(&mut self) {
        let dead = self.proc.as_mut().map(TapProc::dead).unwrap_or(true);
        let stale_silent = !dead
            && self.want_audio
            && !self.got_audio
            && self.started.elapsed() >= SILENT_RESTART;
        if !dead && !stale_silent && self.last_check.elapsed() < RETARGET_EVERY {
            return;
        }
        self.last_check = Instant::now();
        let sink = playback_sink(self.mpv_pid);
        let sink_changed = match (self.proc.as_ref(), sink.as_ref()) {
            (Some(proc), Some(name)) => proc.sink != *name,
            (Some(_), None) => true,
            _ => false,
        };
        if let (Some(_), Some(_)) = (self.proc.as_ref(), sink.as_ref()) {
            if !dead && !stale_silent && !sink_changed {
                return;
            }
        }
        if !dead && !stale_silent && !sink_changed && sink.is_none() {
            return;
        }
        if stale_silent && !sink_changed {
            self.backend = self.backend.wrapping_add(1);
        } else if sink_changed {
            self.backend = 0;
        }
        if !self.can_restart() && !sink_changed {
            return;
        }
        self.proc = None;
        self.got_audio = false;
        if let Some(name) = sink {
            self.proc = spawn_tap(&name, self.backend);
            self.started = Instant::now();
        }
    }

    fn can_restart(&mut self) -> bool {
        let now = Instant::now();
        while self
            .restarts
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
        {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= MAX_RESTARTS_PER_SEC {
            return false;
        }
        self.restarts.push_back(now);
        true
    }
}

impl TapProc {
    fn dead(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        }
    }
}

impl Drop for TapProc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.err_thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_tap(sink: &str, prefer: u32) -> Option<TapProc> {
    for i in 0..3u32 {
        let backend = (prefer + i) % 3;
        let (mut child, name) = match spawn_backend(sink, backend) {
            Some(c) => c,
            None => continue,
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => continue,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let buf = Arc::new(Mutex::new(Vec::with_capacity(RING)));
        let err = Arc::new(Mutex::new(String::new()));
        let stop_t = Arc::clone(&stop);
        let buf_t = Arc::clone(&buf);
        let thread = thread::Builder::new()
            .name("omaradio-tap".into())
            .spawn(move || reader_loop(stdout, buf_t, stop_t))
            .ok()?;
        let err_thread = child.stderr.take().and_then(|stderr| {
            let stop_e = Arc::clone(&stop);
            let err_e = Arc::clone(&err);
            thread::Builder::new()
                .name("omaradio-tap-err".into())
                .spawn(move || stderr_loop(stderr, err_e, stop_e))
                .ok()
        });
        return Some(TapProc {
            child,
            stop,
            buf,
            err,
            thread: Some(thread),
            err_thread,
            sink: sink.to_string(),
            backend: name,
        });
    }
    None
}

fn spawn_backend(sink: &str, backend: u32) -> Option<(Child, &'static str)> {
    let monitor = pulse_monitor_name(sink);
    match backend % 3 {
        0 => spawn_cmd(
            "pacat",
            &[
                "--record",
                "--raw",
                "--format=float32le",
                "--rate=44100",
                "--channels=1",
                "--latency-msec=25",
                "--device",
                &monitor,
            ],
        )
        .map(|c| (c, "pacat")),
        1 => spawn_pw(sink).map(|c| (c, "pw-record")),
        _ => spawn_cmd(
            "pacat",
            &[
                "--record",
                "--raw",
                "--format=float32le",
                "--rate=44100",
                "--channels=1",
                "--latency-msec=25",
                "--device=@DEFAULT_MONITOR@",
            ],
        )
        .map(|c| (c, "pacat-default")),
    }
}

fn spawn_pw(target: &str) -> Option<Child> {
    if forbidden_pw_target(target) {
        return None;
    }
    spawn_cmd(
        "pw-record",
        &[
            "--media-type=Audio",
            "--media-category=Capture",
            "--rate",
            "44100",
            "--channels",
            "1",
            "--format",
            "f32",
            "--latency",
            "20ms",
            "-a",
            "--target",
            target,
            "-",
        ],
    )
}

pub fn forbidden_pw_target(target: &str) -> bool {
    target.ends_with(".monitor")
}

fn spawn_cmd(bin: &str, args: &[&str]) -> Option<Child> {
    Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()
}

fn reader_loop(stdout: std::process::ChildStdout, buf: Arc<Mutex<Vec<f32>>>, stop: Arc<AtomicBool>) {
    let mut reader = BufReader::new(stdout);
    let mut bytes = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                let usable = n - (n % 4);
                if usable == 0 {
                    continue;
                }
                if let Ok(mut ring) = buf.lock() {
                    for chunk in bytes[..usable].chunks_exact(4) {
                        let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        if sample.is_finite() {
                            ring.push(sample);
                        }
                    }
                    if ring.len() > RING {
                        let extra = ring.len() - RING;
                        ring.drain(..extra);
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn stderr_loop(
    stderr: std::process::ChildStderr,
    tail: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    while !stop.load(Ordering::Relaxed) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let clipped: String = line.trim().chars().take(120).collect();
                if !clipped.is_empty() {
                    if let Ok(mut t) = tail.lock() {
                        *t = clipped;
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn playback_sink(prefer_pid: Option<u32>) -> Option<String> {
    let sinks = short_sinks();
    if let Some(idx) = omaradio_sink_index(prefer_pid) {
        if let Some(name) = sinks
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, n)| n.clone())
        {
            return Some(name);
        }
    }
    default_sink()
}

fn default_sink() -> Option<String> {
    let out = Command::new("pactl").arg("get-default-sink").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8(out.stdout).ok()?;
    let sink = sink.trim();
    if sink.is_empty() {
        None
    } else {
        Some(sink.to_string())
    }
}

fn short_sinks() -> Vec<(u32, String)> {
    let out = Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output();
    match out {
        Ok(out) if out.status.success() => {
            parse_short_sinks(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    }
}

fn omaradio_sink_index(prefer_pid: Option<u32>) -> Option<u32> {
    let out = Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_omaradio_sink_index(&String::from_utf8_lossy(&out.stdout), prefer_pid)
}

pub fn pulse_monitor_name(sink: &str) -> String {
    if sink.ends_with(".monitor") {
        sink.to_string()
    } else {
        format!("{sink}.monitor")
    }
}

pub fn parse_short_sinks(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut cols = line.split('\t');
        let Some(idx) = cols.next().and_then(|s| s.trim().parse().ok()) else {
            continue;
        };
        let Some(name) = cols.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push((idx, name.to_string()));
    }
    out
}

pub fn parse_omaradio_sink_index(text: &str, prefer_pid: Option<u32>) -> Option<u32> {
    struct Block {
        sink: Option<u32>,
        is_omaradio: bool,
        pid: Option<u32>,
    }
    let mut cur = Block {
        sink: None,
        is_omaradio: false,
        pid: None,
    };
    let mut found: Vec<(u32, Option<u32>)> = Vec::new();
    let flush = |cur: &mut Block, found: &mut Vec<(u32, Option<u32>)>| {
        if cur.is_omaradio {
            if let Some(s) = cur.sink {
                found.push((s, cur.pid));
            }
        }
        *cur = Block {
            sink: None,
            is_omaradio: false,
            pid: None,
        };
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Sink Input #") {
            flush(&mut cur, &mut found);
            continue;
        }
        if let Some(rest) = t.strip_prefix("Sink:") {
            cur.sink = rest
                .trim()
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok());
        }
        let is_name = t.contains("application.name") || t.contains("node.name");
        if is_name && t.contains("omaradio") {
            cur.is_omaradio = true;
        }
        if t.contains("application.process.id") {
            cur.pid = t.split('"').nth(1).and_then(|s| s.parse().ok());
        }
    }
    flush(&mut cur, &mut found);
    if let Some(pid) = prefer_pid {
        if let Some((sink, _)) = found.iter().find(|(_, p)| *p == Some(pid)) {
            return Some(*sink);
        }
    }
    found.last().map(|(s, _)| *s)
}

pub fn resolve_playback_sink(
    inputs: &str,
    sinks: &str,
    default: &str,
    prefer_pid: Option<u32>,
) -> Option<String> {
    let sinks = parse_short_sinks(sinks);
    if let Some(idx) = parse_omaradio_sink_index(inputs, prefer_pid) {
        if let Some((_, name)) = sinks.iter().find(|(i, _)| *i == idx) {
            return Some(name.clone());
        }
    }
    let d = default.trim();
    if d.is_empty() {
        None
    } else {
        Some(d.to_string())
    }
}

fn rms_of(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = "\
75\talsa_output.usb-BTD_600_BTD_600_E7E27A41E4B588AB3C08-02.analog-stereo\tPipeWire\ts24le 2ch 48000Hz\tRUNNING
77\talsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink\tPipeWire\ts16le 2ch 48000Hz\tSUSPENDED
";

    const INPUTS: &str = r#"
Sink Input #28028
	Driver: protocol-native.c
	Owner Module: 2
	Sink: 75
	Sample Specification: float32le 2ch 44100Hz
	Properties:
		node.name = "omaradio"
		application.name = "omaradio"
		media.name = "missioncontrol-128-mp3 - mpv"
"#;

    const INPUTS_WITH_PID: &str = r#"
Sink Input #1
	Sink: 77
	Properties:
		node.name = "omaradio"
		application.name = "omaradio"
		application.process.id = "111"
Sink Input #2
	Sink: 75
	Properties:
		node.name = "omaradio"
		application.name = "omaradio"
		application.process.id = "222"
"#;

    const SPDIF_INPUTS: &str = r#"
Sink Input #9
	Sink: 77
	Properties:
		application.name = "firefox"
"#;

    #[test]
    fn monitor_name_appends_once() {
        let sink = "alsa_output.pci-0000_00_1f.3.analog-stereo";
        assert_eq!(pulse_monitor_name(sink), format!("{sink}.monitor"));
        assert_eq!(
            pulse_monitor_name(&format!("{sink}.monitor")),
            format!("{sink}.monitor")
        );
    }

    #[test]
    fn pw_record_must_not_target_pulse_monitor_alias() {
        assert!(forbidden_pw_target(
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink.monitor"
        ));
        assert!(!forbidden_pw_target(
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink"
        ));
        assert!(!forbidden_pw_target("@DEFAULT_AUDIO_SINK@"));
    }

    #[test]
    fn short_sinks_map_index_to_name() {
        let sinks = parse_short_sinks(SINKS);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].0, 75);
        assert!(sinks[0].1.contains("BTD_600"));
        assert_eq!(sinks[1].0, 77);
    }

    #[test]
    fn omaradio_input_wins_over_default() {
        let sink = resolve_playback_sink(
            INPUTS,
            SINKS,
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink",
            None,
        )
        .unwrap();
        assert!(sink.contains("BTD_600"), "expected BTD sink, got {sink}");
        assert!(!sink.contains("SPDIF"));
        assert!(!sink.ends_with(".monitor"));
    }

    #[test]
    fn prefer_pid_picks_that_omaradio() {
        let sink = resolve_playback_sink(INPUTS_WITH_PID, SINKS, "default", Some(111)).unwrap();
        assert!(sink.contains("SPDIF"), "pid 111 should be SPDIF, got {sink}");
        let sink = resolve_playback_sink(INPUTS_WITH_PID, SINKS, "default", Some(222)).unwrap();
        assert!(sink.contains("BTD_600"), "pid 222 should be BTD, got {sink}");
    }

    #[test]
    fn missing_omaradio_falls_back_to_default() {
        let sink = resolve_playback_sink(SPDIF_INPUTS, SINKS, "  default_sink  \n", None).unwrap();
        assert_eq!(sink, "default_sink");
    }

    #[test]
    fn empty_default_is_none() {
        assert!(resolve_playback_sink(SPDIF_INPUTS, SINKS, "  \n", None).is_none());
    }

    #[test]
    fn parse_sink_index_from_pactl_block() {
        assert_eq!(parse_omaradio_sink_index(INPUTS, None), Some(75));
        assert_eq!(parse_omaradio_sink_index(SPDIF_INPUTS, None), None);
    }
}
