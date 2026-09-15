//! PipeWire sink-monitor tap — the actual speaker mix, not a mood oscillator.

use std::io::{BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

const RING: usize = 16_384;
const SAMPLE_RATE: u32 = 44_100;

pub struct OutputTap {
    child: Child,
    stop: Arc<AtomicBool>,
    buf: Arc<Mutex<Vec<f32>>>,
    thread: Option<JoinHandle<()>>,
    pub sample_rate: u32,
}

impl OutputTap {
    pub fn try_open() -> Option<Self> {
        let target = monitor_target()?;
        let mut child = Command::new("pw-record")
            .args([
                "--media-type=Audio",
                "--media-category=Capture",
                "--media-role=Music",
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
                &target,
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let stop = Arc::new(AtomicBool::new(false));
        let buf = Arc::new(Mutex::new(Vec::with_capacity(RING)));
        let stop_t = Arc::clone(&stop);
        let buf_t = Arc::clone(&buf);
        let thread = thread::Builder::new()
            .name("omaradio-tap".into())
            .spawn(move || reader_loop(stdout, buf_t, stop_t))
            .ok()?;
        Some(Self {
            child,
            stop,
            buf,
            thread: Some(thread),
            sample_rate: SAMPLE_RATE,
        })
    }

    pub fn drain(&self, dest: &mut Vec<f32>) {
        dest.clear();
        if let Ok(mut ring) = self.buf.lock() {
            dest.extend_from_slice(&ring);
            ring.clear();
        }
    }
}

impl Drop for OutputTap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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

fn monitor_target() -> Option<String> {
    let out = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8(out.stdout).ok()?;
    let sink = sink.trim();
    if sink.is_empty() {
        return None;
    }
    Some(format!("{sink}.monitor"))
}
