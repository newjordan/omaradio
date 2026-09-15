//! Non-blocking observation of the native (kitty `--vo=kitty`) mpv child so
//! the UI can tell connecting / playing / stalled / failed apart, instead of
//! leaving the pane Pending forever.

use std::process::Child;
use std::time::{Duration, Instant};

/// JSON IPC `observe_property` line (mpv CLI has no `--observe-property=`).
pub fn observe_property_command(id: i64, name: &str) -> String {
    let mut line = serde_json::json!({ "command": ["observe_property", id, name] }).to_string();
    line.push('\n');
    line
}

/// Observed playback health of the native child. Child existence alone is
/// never "playing" — only decoded output (detected via mpv's IPC when
/// available) or sustained uptime after a successful IPC handshake upgrades
/// the state, and progress-time movement is what keeps it playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeHealth {
    /// Child spawned, IPC handshake not yet confirmed.
    Connecting,
    /// IPC confirms demuxer/decode progress (playback-time moving).
    Playing,
    /// IPC confirmed but playback-time has not moved for `stall_after`.
    Stalled(Duration),
    /// Child exited, or IPC reports a fatal file error.
    Failed(String),
}

const STALL_AFTER: Duration = Duration::from_secs(4);

#[derive(Debug, Clone)]
pub struct NativeWatcher {
    last_progress: Option<Instant>,
    handshake: bool,
    last_time: f64,
    failed: Option<String>,
}

impl NativeWatcher {
    pub fn new() -> Self {
        Self {
            last_progress: None,
            handshake: false,
            last_time: 0.0,
            failed: None,
        }
    }

    /// Feed raw IPC output lines (already split) — mpv `--input-ipc-server`
    /// JSON, same shapes `player.rs` consumes.
    pub fn feed_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        self.handshake = true;
        if let Some(err) = v.get("error").and_then(serde_json::Value::as_str) {
            // Observe-property replies with "property unavailable" (and similar)
            // while the file is still opening — not a source failure.
            if err != "success"
                && self.failed.is_none()
                && !err.to_ascii_lowercase().contains("unavailable")
            {
                self.failed = Some(err.to_string());
            }
        }
        // property-change for playback-time / eof-reached
        if v.get("event").and_then(serde_json::Value::as_str) == Some("property-change") {
            let name = v
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            match name {
                "playback-time" | "time-pos" => {
                    let Some(t) = v.get("data").and_then(serde_json::Value::as_f64) else {
                        // null / missing: not progress (and not a baseline).
                        return;
                    };
                    if t > self.last_time + 0.01 {
                        self.last_time = t;
                        self.last_progress = Some(Instant::now());
                    } else if t + 0.01 < self.last_time {
                        // Loop / seek backwards: reset the baseline so the
                        // next *forward* tick counts; do not treat rewind as
                        // progress and do not wait for the old maximum.
                        self.last_time = t;
                    }
                    // Zero, equal, or non-advancing first sample: ignore.
                }
                "eof-reached" => {
                    // Looping playlist: EOF is not fatal, but it is also not
                    // decode progress — Playing requires playback-time advance.
                }
                _ => {}
            }
        }
        if v.get("event").and_then(serde_json::Value::as_str) == Some("end-file")
            && v.get("reason").and_then(serde_json::Value::as_str) == Some("error")
        {
            let detail = v
                .get("file_error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("source failed");
            self.failed = Some(detail.to_string());
        }
    }

    /// Health derived only from fed IPC lines (no child) — usable without
    /// spawning mpv.
    fn observe_childless(&mut self) -> NativeHealth {
        if let Some(err) = &self.failed {
            return NativeHealth::Failed(err.clone());
        }
        if !self.handshake {
            return NativeHealth::Connecting;
        }
        match self.last_progress {
            Some(t) if t.elapsed() >= STALL_AFTER => NativeHealth::Stalled(t.elapsed()),
            Some(_) => NativeHealth::Playing,
            None => NativeHealth::Connecting,
        }
    }

    pub fn observe(&mut self, child: &mut Child) -> NativeHealth {
        if let Some(status) = child.try_wait().ok().flatten() {
            return NativeHealth::Failed(format!("mpv exited ({status})"));
        }
        self.observe_childless()
    }
}

impl Default for NativeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(json: &str) -> String {
        json.to_string()
    }

    #[test]
    fn connecting_until_handshake_then_playing_on_progress() {
        let mut w = NativeWatcher::new();
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
        w.feed_line(&line(r#"{"error":"success"}"#));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":1.5}"#,
        ));
        assert_eq!(w.observe_childless(), NativeHealth::Playing);
    }

    #[test]
    fn stalled_when_time_stops_moving() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(r#"{"error":"success"}"#));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":3.0}"#,
        ));
        // Force the stall window by backdating progress.
        w.last_progress = Some(Instant::now() - Duration::from_secs(9));
        assert!(matches!(w.observe_childless(), NativeHealth::Stalled(_)));
    }

    #[test]
    fn failed_on_end_file_error_keeps_first_cause() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(
            r#"{"event":"end-file","reason":"error","file_error":"No such stream"}"#,
        ));
        assert_eq!(
            w.observe_childless(),
            NativeHealth::Failed("No such stream".into())
        );
        // First cause is kept, later lines do not overwrite it.
        w.feed_line(&line(r#"{"error":"invalid parameter"}"#));
        assert_eq!(
            w.observe_childless(),
            NativeHealth::Failed("No such stream".into())
        );
    }

    #[test]
    fn backward_time_is_not_progress_but_does_not_stall_immediately() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(r#"{"error":"success"}"#));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":10.0}"#,
        ));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":2.0}"#,
        ));
        assert_eq!(w.last_time, 2.0);
        assert_eq!(w.observe_childless(), NativeHealth::Playing);
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":2.2}"#,
        ));
        assert_eq!(w.last_time, 2.2);
        assert_eq!(w.observe_childless(), NativeHealth::Playing);
    }

    #[test]
    fn null_or_zero_time_is_not_playing() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(r#"{"error":"success"}"#));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":null}"#,
        ));
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
        w.feed_line(&line(
            r#"{"event":"property-change","name":"playback-time","data":0}"#,
        ));
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
        assert!(w.last_progress.is_none());
    }

    #[test]
    fn property_unavailable_is_not_file_failure() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(r#"{"error":"property unavailable"}"#));
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
        w.feed_line(&line(
            r#"{"event":"end-file","reason":"error","file_error":"No such stream"}"#,
        ));
        assert_eq!(
            w.observe_childless(),
            NativeHealth::Failed("No such stream".into())
        );
    }

    #[test]
    fn observe_property_is_ipc_command_not_cli_flag() {
        let line = observe_property_command(1, "playback-time");
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            v["command"],
            serde_json::json!(["observe_property", 1, "playback-time"])
        );
        assert!(!line.contains("--observe-property"));
    }

    #[test]
    fn eof_or_handshake_alone_is_not_playing() {
        let mut w = NativeWatcher::new();
        w.feed_line(&line(r#"{"error":"success"}"#));
        w.feed_line(&line(
            r#"{"event":"property-change","name":"eof-reached","data":true}"#,
        ));
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
        w.feed_line(&line(r#"{"event":"end-file","reason":"eof"}"#));
        assert_eq!(w.observe_childless(), NativeHealth::Connecting);
    }
}
