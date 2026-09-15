//! Control surface for agents and scripts.
//!
//! The running app listens on a Unix socket and speaks newline-delimited
//! JSON: one request object per line, one response object per line.
//! `omaradio ctl …` is the bundled client. See AGENTS.md for the contract.

use crate::stations::StationSpec;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

pub const PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Everything an agent needs in one call.
    Status,
    /// The dial, with ids and urls.
    Stations,
    /// `station` is an id, a 1-based dial number, a name, or a raw http(s) url.
    Tune { station: String },
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Prev,
    /// Absolute `value` (0–130) or relative `delta`.
    Volume {
        #[serde(default)]
        value: Option<f64>,
        #[serde(default)]
        delta: Option<f64>,
    },
    /// bars | wave | milkdrop | iss
    Viz { kind: String },
    /// next (default) | builtin | collection | a preset name
    Preset {
        #[serde(default)]
        action: Option<String>,
    },
    /// Add or replace a station in the config and reload the dial.
    Add {
        station: StationSpec,
        #[serde(default)]
        tune: bool,
    },
    Remove { id: String },
    Reload,
    Quit,
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("OMARADIO_CTL_SOCKET") {
        return PathBuf::from(p);
    }
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(d).join("omaradio.ctl.sock");
    }
    std::env::temp_dir().join(format!("omaradio-{}.ctl.sock", unsafe { libc::getuid() }))
}

/// A request from a client plus the channel its answer goes back on.
pub type Inbound = (Request, Sender<Value>);

pub struct CtlServer {
    rx: Receiver<Inbound>,
    path: PathBuf,
}

impl CtlServer {
    /// Bind the control socket. A stale socket file is replaced; a live one
    /// (another omaradio) is left alone and this returns an error.
    pub fn start() -> Result<Self> {
        let path = socket_path();
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(anyhow!("another omaradio owns {}", path.display()));
            }
            let _ = std::fs::remove_file(&path);
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let listener = UnixListener::bind(&path).with_context(|| format!("bind {}", path.display()))?;
        let (tx, rx) = channel::<Inbound>();
        std::thread::Builder::new()
            .name("omaradio-ctl".into())
            .spawn(move || {
                for conn in listener.incoming().flatten() {
                    let tx = tx.clone();
                    std::thread::spawn(move || serve(conn, tx));
                }
            })
            .context("spawn ctl thread")?;
        Ok(Self { rx, path })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Non-blocking: everything that arrived since the last tick.
    pub fn drain(&self) -> Vec<Inbound> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            out.push(m);
        }
        out
    }
}

impl Drop for CtlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(conn: UnixStream, tx: Sender<Inbound>) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(30)));
    let mut writer = match conn.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(conn);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                let (rtx, rrx) = channel::<Value>();
                if tx.send((req, rtx)).is_err() {
                    json!({"ok": false, "error": "app is shutting down"})
                } else {
                    rrx.recv_timeout(Duration::from_secs(5))
                        .unwrap_or_else(|_| json!({"ok": false, "error": "app did not answer in 5s"}))
                }
            }
            Err(e) => json!({"ok": false, "error": format!("bad request: {e}"), "hint": "see AGENTS.md"}),
        };
        let mut text = reply.to_string();
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            break;
        }
    }
}

/// Send one request to the running app and return its answer.
pub fn send(req: &Request) -> Result<Value> {
    let path = socket_path();
    let mut conn = UnixStream::connect(&path)
        .with_context(|| format!("no omaradio listening at {} — is it running?", path.display()))?;
    conn.set_read_timeout(Some(Duration::from_secs(6)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    conn.write_all(line.as_bytes())?;
    let mut reader = BufReader::new(conn);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    serde_json::from_str(reply.trim()).with_context(|| format!("unparseable reply: {reply:?}"))
}

pub const USAGE: &str = "\
omaradio ctl — drive a running omaradio

  omaradio ctl [--json] <command> [args]

  status                     what is playing, volume, viz, tap health
  stations                   the dial: number, id, name, url
  tune <id|n|name|url>       tune a station (or any http(s) stream url)
  play | pause | toggle      transport
  stop | next | prev
  volume <0-130 | +n | -n>   absolute or relative
  viz <bars|wave|milkdrop|iss>
  preset [next|builtin|collection|<name>]   milkdrop preset control
  add <id> <name> <url> [--source S] [--blurb B] [--homepage H] [--mood M] [--tune]
                             add/replace a station in stations.json and reload
  remove <id>
  reload                     re-read stations.json
  quit                       exit the app

  --json    print the raw JSON reply (default is JSON too, pretty-printed)
  Socket: $XDG_RUNTIME_DIR/omaradio.ctl.sock (override: OMARADIO_CTL_SOCKET)
  Protocol: one JSON object per line, see AGENTS.md
";

/// Parse `omaradio ctl …` argv (after `ctl`) into a request.
pub fn parse_args(args: &[String]) -> Result<(Request, bool)> {
    let mut json = false;
    let mut rest: Vec<&str> = Vec::new();
    for a in args {
        if a == "--json" {
            json = true;
        } else {
            rest.push(a.as_str());
        }
    }
    let Some((cmd, tail)) = rest.split_first() else {
        return Ok((Request::Status, json));
    };
    let need = |n: usize, what: &str| -> Result<()> {
        if tail.len() < n {
            Err(anyhow!("{cmd} needs {what}\n\n{USAGE}"))
        } else {
            Ok(())
        }
    };
    let req = match *cmd {
        "status" => Request::Status,
        "stations" | "list" => Request::Stations,
        "tune" => {
            need(1, "a station id, number, name, or url")?;
            Request::Tune { station: tail.join(" ") }
        }
        "play" => Request::Play,
        "pause" => Request::Pause,
        "toggle" => Request::Toggle,
        "stop" => Request::Stop,
        "next" => Request::Next,
        "prev" | "previous" => Request::Prev,
        "volume" | "vol" => {
            need(1, "a level or +n/-n")?;
            let v = tail[0];
            if let Some(d) = v.strip_prefix('+') {
                Request::Volume { value: None, delta: Some(d.parse()?) }
            } else if v.starts_with('-') {
                Request::Volume { value: None, delta: Some(v.parse()?) }
            } else {
                Request::Volume { value: Some(v.parse()?), delta: None }
            }
        }
        "viz" => {
            need(1, "bars|wave|milkdrop|iss")?;
            Request::Viz { kind: tail[0].to_string() }
        }
        "preset" => Request::Preset { action: tail.first().map(|s| s.to_string()) },
        "add" => {
            need(3, "<id> <name> <url>")?;
            let mut spec = StationSpec {
                id: tail[0].into(),
                name: tail[1].into(),
                url: tail[2].into(),
                blurb: String::new(),
                source: String::new(),
                homepage: String::new(),
                mood: String::new(),
                accent: None,
            };
            let mut tune = false;
            let mut i = 3;
            while i < tail.len() {
                let key = tail[i];
                let val = || -> Result<String> {
                    tail.get(i + 1).map(|s| s.to_string()).ok_or_else(|| anyhow!("{key} needs a value"))
                };
                match key {
                    "--source" => spec.source = val()?,
                    "--blurb" => spec.blurb = val()?,
                    "--homepage" => spec.homepage = val()?,
                    "--mood" => spec.mood = val()?,
                    "--tune" => {
                        tune = true;
                        i += 1;
                        continue;
                    }
                    _ => return Err(anyhow!("unknown option {key}\n\n{USAGE}")),
                }
                i += 2;
            }
            Request::Add { station: spec, tune }
        }
        "remove" | "rm" => {
            need(1, "a station id")?;
            Request::Remove { id: tail[0].into() }
        }
        "reload" => Request::Reload,
        "quit" | "exit" => Request::Quit,
        "help" | "--help" | "-h" => return Err(anyhow!("{USAGE}")),
        other => return Err(anyhow!("unknown command {other:?}\n\n{USAGE}")),
    };
    Ok((req, json))
}

/// `omaradio ctl …` entry point.
pub fn run_cli(args: &[String]) -> Result<()> {
    let (req, compact) = parse_args(args)?;
    let reply = send(&req)?;
    if compact {
        println!("{reply}");
    } else {
        println!("{}", serde_json::to_string_pretty(&reply)?);
    }
    if reply.get("ok").and_then(Value::as_bool) == Some(false) {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_tagged_json() {
        let r: Request = serde_json::from_str(r#"{"cmd":"tune","station":"jazz"}"#).unwrap();
        assert_eq!(r, Request::Tune { station: "jazz".into() });
        let r: Request = serde_json::from_str(r#"{"cmd":"volume","delta":5}"#).unwrap();
        assert_eq!(r, Request::Volume { value: None, delta: Some(5.0) });
        let r: Request = serde_json::from_str(r#"{"cmd":"status"}"#).unwrap();
        assert_eq!(r, Request::Status);
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"dance"}"#).is_err());
    }

    #[test]
    fn cli_args_map_to_requests() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&s(&[])).unwrap().0, Request::Status);
        assert_eq!(parse_args(&s(&["tune", "Jazz", "Groove"])).unwrap().0, Request::Tune { station: "Jazz Groove".into() });
        assert_eq!(parse_args(&s(&["volume", "+5"])).unwrap().0, Request::Volume { value: None, delta: Some(5.0) });
        assert_eq!(parse_args(&s(&["volume", "-5"])).unwrap().0, Request::Volume { value: None, delta: Some(-5.0) });
        assert_eq!(parse_args(&s(&["volume", "60"])).unwrap().0, Request::Volume { value: Some(60.0), delta: None });
        let (r, json) = parse_args(&s(&["--json", "add", "kexp", "KEXP", "https://kexp-mp3-128.streamguys1.com/kexp128.mp3", "--mood", "brass", "--tune"])).unwrap();
        assert!(json);
        match r {
            Request::Add { station, tune } => {
                assert_eq!(station.id, "kexp");
                assert_eq!(station.mood, "brass");
                assert!(tune);
            }
            other => panic!("{other:?}"),
        }
        assert!(parse_args(&s(&["tune"])).is_err());
        assert!(parse_args(&s(&["bogus"])).is_err());
    }

    #[test]
    fn socket_path_honours_override() {
        std::env::set_var("OMARADIO_CTL_SOCKET", "/tmp/omr-test.sock");
        assert_eq!(socket_path(), PathBuf::from("/tmp/omr-test.sock"));
        std::env::remove_var("OMARADIO_CTL_SOCKET");
    }
}
