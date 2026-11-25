//! mpv JSON IPC — internet radio that actually stays up.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// mpv accepts 0–130. Values outside that are clamped; non-finite values are rejected.
pub const VOLUME_MIN: f64 = 0.0;
pub const VOLUME_MAX: f64 = 130.0;

/// Playback fields learned from mpv's JSON IPC, separate from the process handle
/// so the parser can be tested without spawning mpv.
#[derive(Debug, Clone)]
pub struct IpcProps {
    pub paused: bool,
    pub volume: f64,
    pub title: String,
    pub icy_title: String,
    pub idle: bool,
    pub last_error: Option<String>,
}

impl IpcProps {
    fn fresh() -> Self {
        Self {
            paused: false,
            volume: 70.0,
            title: String::new(),
            icy_title: String::new(),
            idle: true,
            last_error: None,
        }
    }

    pub fn apply_line(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if value.get("event").and_then(Value::as_str) == Some("property-change") {
            let name = value.get("name").and_then(Value::as_str).unwrap_or("");
            let data = value.get("data");
            match name {
                "media-title" => {
                    if let Some(s) = data.and_then(Value::as_str) {
                        if !s.is_empty() && s != "none" {
                            self.title = s.to_string();
                        }
                    }
                }
                "metadata" => {
                    if let Some(map) = data.and_then(Value::as_object) {
                        for key in ["icy-title", "icy_title", "title", "TITLE"] {
                            if let Some(s) = map.get(key).and_then(Value::as_str) {
                                if !s.is_empty() {
                                    self.icy_title = s.to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
                "pause" => self.paused = data.and_then(Value::as_bool).unwrap_or(self.paused),
                "volume" => {
                    if let Some(v) = data.and_then(Value::as_f64) {
                        self.volume = v;
                    }
                }
                "idle-active" => {
                    self.idle = data.and_then(Value::as_bool).unwrap_or(self.idle);
                }
                _ => {}
            }
            return;
        }
        if value.get("event").and_then(Value::as_str) == Some("end-file") {
            let reason = value.get("reason").and_then(Value::as_str).unwrap_or("");
            if reason == "error" {
                let detail = value
                    .get("file_error")
                    .and_then(Value::as_str)
                    .unwrap_or("stream error");
                self.last_error = Some(detail.to_string());
            }
        }
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            if err != "success" {
                self.last_error = Some(err.to_string());
            }
        }
    }

    pub fn now_playing(&self) -> String {
        if !self.icy_title.is_empty() {
            self.icy_title.clone()
        } else if !self.title.is_empty() {
            self.title.clone()
        } else {
            String::new()
        }
    }

    fn mark_loaded(&mut self) {
        self.icy_title.clear();
        self.title.clear();
        self.last_error = None;
        self.paused = false;
        self.idle = false;
    }

    fn mark_stopped(&mut self) {
        self.idle = true;
        self.paused = false;
        self.title.clear();
        self.icy_title.clear();
    }
}

/// Finite volume inside mpv's 0–130 range.
pub fn normalize_volume(volume: f64) -> Result<f64> {
    if !volume.is_finite() {
        anyhow::bail!("volume must be a finite number");
    }
    Ok(volume.clamp(VOLUME_MIN, VOLUME_MAX))
}

/// Gauge fill for a volume in the 0–130 range. Non-finite input draws empty.
pub fn volume_ratio(volume: f64) -> f64 {
    if !volume.is_finite() {
        return 0.0;
    }
    (volume.clamp(VOLUME_MIN, VOLUME_MAX) / VOLUME_MAX).clamp(0.0, 1.0)
}

pub struct MpvPlayer {
    child: Child,
    sock_path: PathBuf,
    stream: UnixStream,
    buf: Vec<u8>,
    req: i64,
    pub props: IpcProps,
    pub alive: bool,
}

impl MpvPlayer {
    pub fn spawn() -> Result<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let sock_path = dir.join(format!("omaradio-{}.sock", std::process::id()));
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }

        let mut cmd = Command::new("mpv");
        cmd.args([
            "--no-config",
            "--no-video",
            "--no-terminal",
            "--really-quiet",
            "--idle=yes",
            "--force-window=no",
            "--audio-display=no",
            "--no-input-default-bindings",
            "--input-terminal=no",
            "--cache=yes",
            "--demuxer-max-bytes=12MiB",
            "--demuxer-readahead-secs=8",
            "--volume=70",
            "--audio-client-name=omaradio",
            "--msg-level=all=no",
        ])
        .arg(format!("--input-ipc-server={}", sock_path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        // Die with omaradio: an orphaned player would keep the radio going
        // after a crash or a closed terminal.
        // SAFETY: prctl is async-signal-safe and takes no Rust state.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = cmd
            .spawn()
            .context("mpv is required — install mpv and try again")?;

        let stream = wait_for_socket(&sock_path, Duration::from_secs(4))?;
        stream
            .set_nonblocking(true)
            .context("mpv socket nonblocking")?;

        let mut player = Self {
            child,
            sock_path,
            stream,
            buf: Vec::with_capacity(4096),
            req: 1,
            props: IpcProps::fresh(),
            alive: true,
        };
        player.observe("media-title")?;
        player.observe("metadata")?;
        player.observe("pause")?;
        player.observe("volume")?;
        player.observe("idle-active")?;
        Ok(player)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn observe(&mut self, name: &str) -> Result<()> {
        let id = self.req;
        self.req += 1;
        self.send(&json!({ "command": ["observe_property", id, name] }))
    }

    fn send(&mut self, payload: &Value) -> Result<()> {
        let mut line = payload.to_string();
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .context("write mpv ipc")?;
        Ok(())
    }

    pub fn play_url(&mut self, url: &str) -> Result<()> {
        self.props.mark_loaded();
        let load_id = self.next_req();
        self.send(&json!({
            "command": ["loadfile", url, "replace"],
            "request_id": load_id,
        }))?;
        let pause_id = self.next_req();
        self.send(&json!({
            "command": ["set_property", "pause", false],
            "request_id": pause_id,
        }))?;
        Ok(())
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<()> {
        let request_id = self.next_req();
        self.send(&json!({
            "command": ["set_property", "pause", paused],
            "request_id": request_id,
        }))?;
        self.props.paused = paused;
        Ok(())
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        self.set_paused(!self.props.paused)
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        let volume = normalize_volume(volume)?;
        let request_id = self.next_req();
        self.send(&json!({
            "command": ["set_property", "volume", volume],
            "request_id": request_id,
        }))?;
        self.props.volume = volume;
        Ok(())
    }

    pub fn bump_volume(&mut self, delta: f64) -> Result<()> {
        self.set_volume(self.props.volume + delta)
    }

    pub fn stop(&mut self) -> Result<()> {
        let request_id = self.next_req();
        self.send(&json!({
            "command": ["stop"],
            "request_id": request_id,
        }))?;
        self.props.mark_stopped();
        Ok(())
    }

    fn next_req(&mut self) -> i64 {
        let id = self.req;
        self.req += 1;
        id
    }

    pub fn poll(&mut self) {
        if !self.alive {
            return;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.alive = false;
            self.props.last_error = Some(format!("mpv exited ({status})"));
            return;
        }

        let mut tmp = [0u8; 4096];
        loop {
            match self.stream.read(&mut tmp) {
                Ok(0) => {
                    self.alive = false;
                    self.props.last_error = Some("mpv socket closed".into());
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    self.props.last_error = Some(err.to_string());
                    break;
                }
            }
        }

        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            if let Ok(text) = std::str::from_utf8(&line) {
                self.handle_line(text.trim());
            }
        }
    }

    fn handle_line(&mut self, line: &str) {
        self.props.apply_line(line);
    }

    pub fn now_playing(&self) -> String {
        self.props.now_playing()
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        let _ = self.stream.write_all(b"{\"command\":[\"quit\"]}\n");
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

fn wait_for_socket(path: &Path, timeout: Duration) -> Result<UnixStream> {
    let start = Instant::now();
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(_) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => {
                return Err(anyhow!("waiting for mpv socket: {err}"));
            }
        }
    }
}

#[cfg(test)]
pub fn loadfile_command(url: &str, request_id: i64) -> String {
    json!({
        "command": ["loadfile", url, "replace"],
        "request_id": request_id,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadfile_json_is_mpv_shaped() {
        let line = loadfile_command("https://somafm.com/spacestation.pls", 7);
        let v: Value = serde_json::from_str(&line).unwrap();
        let cmd = v["command"].as_array().unwrap();
        assert_eq!(cmd[0], "loadfile");
        assert_eq!(cmd[1], "https://somafm.com/spacestation.pls");
        assert_eq!(cmd[2], "replace");
        assert_eq!(v["request_id"], 7);
    }

    #[test]
    fn ipc_lines_update_title_pause_volume_and_errors() {
        let mut p = IpcProps::fresh();
        p.apply_line(r#"{"event":"property-change","name":"metadata","data":{"icy-title":"Artist - Track"}}"#);
        assert_eq!(p.now_playing(), "Artist - Track");
        p.apply_line(r#"{"event":"property-change","name":"pause","data":true}"#);
        assert!(p.paused);
        p.apply_line(r#"{"event":"property-change","name":"volume","data":55}"#);
        assert_eq!(p.volume, 55.0);
        p.apply_line(r#"{"event":"property-change","name":"idle-active","data":false}"#);
        assert!(!p.idle);
        p.apply_line(r#"{"error":"success"}"#);
        assert!(p.last_error.is_none());
        p.apply_line(r#"{"event":"end-file","reason":"error","file_error":"No such stream"}"#);
        assert_eq!(p.last_error.as_deref(), Some("No such stream"));
        p.mark_stopped();
        assert!(p.idle);
        assert!(!p.paused);
        assert!(p.now_playing().is_empty());
    }

    #[test]
    fn volume_rejects_non_finite_and_clamps() {
        assert!(normalize_volume(f64::NAN).is_err());
        assert!(normalize_volume(f64::INFINITY).is_err());
        assert_eq!(normalize_volume(200.0).unwrap(), VOLUME_MAX);
        assert_eq!(normalize_volume(-4.0).unwrap(), VOLUME_MIN);
        assert_eq!(normalize_volume(70.0).unwrap(), 70.0);
        assert!((volume_ratio(0.0) - 0.0).abs() < 1e-9);
        assert!((volume_ratio(130.0) - 1.0).abs() < 1e-9);
        assert!((volume_ratio(65.0) - 0.5).abs() < 1e-9);
        assert_eq!(volume_ratio(f64::NAN), 0.0);
    }
}
