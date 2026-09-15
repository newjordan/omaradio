//! ISS live camera, straight into the terminal.
//!
//! On Kitty, ISS is fused into mpv's native `--vo=kitty` (optional shm): the
//! decoder paints the pane directly. No RGB pipe, no base64, no second blit.
//! Other terminals keep the raw-frame fallback.

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::mpvwatch::{NativeHealth, NativeWatcher};

const ISS_CAMS: &[(&str, &str)] = &[
    (
        "https://www.youtube.com/watch?v=M3HKLzjvKPc",
        "NASA ISS live",
    ),
    (
        "https://www.youtube.com/watch?v=awQzjn72bI0",
        "NASA HD cams",
    ),
    (
        "https://www.youtube.com/watch?v=fO9e9jnhYK8",
        "Sen 4K (HUD)",
    ),
];
const ISS_W: u32 = 640;
const ISS_H: u32 = 360;

const LOCAL_VIDEO_ENV: &str = "OMARADIO_SPACE_VIDEO";
const STALE_AFTER: Duration = Duration::from_secs(5);
/// Video-only 720p first (HLS often has no muxed AV); combined remains fallback.
const ISS_YTDL_FORMAT: &str = "bestvideo[height<=720]/best[height<=720]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceState {
    Idle,
    Pending,
    Live,
    /// Live local-file playback: set once the decoder publishes a frame.
    Local,
    Stale,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceFeed {
    Iss,
}

/// Truthful state: a spawned native child alone is `Pending`, never `Live`.
pub fn apply_stale(state: SpaceState, since_publish: Option<Duration>) -> SpaceState {
    match (state, since_publish) {
        (_s @ (SpaceState::Live | SpaceState::Local), Some(age)) if age >= STALE_AFTER => {
            SpaceState::Stale
        }
        (s, _) => s,
    }
}

/// Selected local video override, if any. Presence of the variable (even
/// empty) opts the ISS pane out of yt-dlp/mpv live resolution.
fn local_video_requested() -> Option<OsString> {
    std::env::var_os(LOCAL_VIDEO_ENV)
}

fn validate_local_file(path: &PathBuf) -> Result<(), String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("{LOCAL_VIDEO_ENV}: cannot open {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "{LOCAL_VIDEO_ENV}: {} is not a regular file",
            path.display()
        ));
    }
    if meta.len() == 0 {
        return Err(format!(
            "{LOCAL_VIDEO_ENV}: {} is empty (0 bytes)",
            path.display()
        ));
    }
    Ok(())
}

/// Local override path when set AND valid. Validation errors surface as strings.
fn resolve_local_video() -> Result<Option<PathBuf>, String> {
    resolve_local_video_from(local_video_requested())
}

/// Unset → `Ok(None)` (live cameras). Explicit empty → `Err`. Valid path → `Ok(Some)`.
fn resolve_local_video_from(value: Option<OsString>) -> Result<Option<PathBuf>, String> {
    match value {
        None => Ok(None),
        Some(v) => resolve_local_video_with(v),
    }
}

fn resolve_local_video_with(value: OsString) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Err(format!(
            "{LOCAL_VIDEO_ENV} is set but empty — give a video file path \
             or unset the variable to use live cameras"
        ));
    }
    let path = PathBuf::from(&value);
    validate_local_file(&path)?;
    Ok(Some(path))
}

pub(crate) fn cam_label(cam: usize) -> String {
    format!("ISS Earth viewing · {}", ISS_CAMS[cam % ISS_CAMS.len()].1)
}

#[derive(Clone)]
pub struct SpaceFrame {
    pub rgb: Arc<Vec<u8>>,
    pub w: u32,
    pub h: u32,
    pub seq: u64,
    pub label: String,
}

pub struct SpaceCam {
    feed: Option<SpaceFeed>,
    native: bool,
    area: Rect,
    mpv: Option<Child>,
    /// IPC socket of the native child, for a polite `quit` before the axe.
    ipc: Option<PathBuf>,
    last_spawn: Option<Instant>,
    frame: Arc<Mutex<Option<SpaceFrame>>>,
    stop: Arc<AtomicBool>,
    gen: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
    cam: usize,
    local: bool,
    watcher: Arc<Mutex<NativeWatcher>>,
    state: Arc<Mutex<SpaceState>>,
    detail: Arc<Mutex<String>>,
    last_pub: Arc<Mutex<Option<Instant>>>,
}

impl SpaceCam {
    pub fn new() -> Self {
        Self {
            feed: None,
            native: false,
            area: Rect::default(),
            mpv: None,
            ipc: None,
            last_spawn: None,
            frame: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            gen: Arc::new(AtomicU64::new(0)),
            thread: None,
            cam: 0,
            local: false,
            watcher: Arc::new(Mutex::new(NativeWatcher::new())),
            state: Arc::new(Mutex::new(SpaceState::Idle)),
            detail: Arc::new(Mutex::new(String::new())),
            last_pub: Arc::new(Mutex::new(None)),
        }
    }

    fn set_state(&self, state: SpaceState, detail: &str) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
        if let Ok(mut d) = self.detail.lock() {
            *d = detail.to_string();
        }
        if matches!(state, SpaceState::Live | SpaceState::Local) {
            if let Ok(mut t) = self.last_pub.lock() {
                *t = Some(Instant::now());
            }
        }
    }

    fn clear_frame(&self) {
        if let Ok(mut g) = self.frame.lock() {
            *g = None;
        }
        if let Ok(mut t) = self.last_pub.lock() {
            *t = None;
        }
    }

    /// (state, human detail) for the currently selected space source.
    pub fn status(&self) -> (SpaceState, String) {
        let st = self.state.lock().map(|s| *s).unwrap_or(SpaceState::Idle);
        let age = self
            .last_pub
            .lock()
            .ok()
            .and_then(|t| t.map(|t| t.elapsed()));
        let st = apply_stale(st, age);
        let detail = self.detail.lock().map(|d| d.clone()).unwrap_or_default();
        (st, detail)
    }

    /// Source-correct label for the current selection.
    pub fn source_name(&self) -> String {
        if self.local {
            let base = local_video_requested()
                .map(|p| {
                    if p.is_empty() {
                        return "local video".to_string();
                    }
                    PathBuf::from(&p)
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "local video".into());
            format!("local video · {base}")
        } else {
            self.cam_name().to_string()
        }
    }

    pub fn set(&mut self, feed: SpaceFeed, kitty: bool) {
        let local = feed == SpaceFeed::Iss
            && match resolve_local_video() {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(err) => {
                    // Bad override: never fall back to live silently; surface it.
                    // The requested local source identity is preserved so the
                    // pane never claims a live camera it is not using.
                    self.stop();
                    self.feed = Some(feed);
                    self.native = false;
                    self.local = true;
                    self.set_state(SpaceState::Error, &err);
                    return;
                }
            };
        let native = kitty && feed == SpaceFeed::Iss && !local;
        if self.feed == Some(feed) && self.native == native && self.local == local {
            return;
        }
        self.stop();
        self.feed = Some(feed);
        self.native = native;
        self.local = local;
        if native {
            self.stop_flag_reset();
            self.set_state(SpaceState::Pending, "connecting…");
            if self.area.width >= 8 && self.area.height >= 4 {
                self.spawn_kitty();
            }
        } else {
            self.stop_flag_reset();
            let gen = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
            let flag = Arc::clone(&self.stop);
            let slot = Arc::clone(&self.frame);
            let gen_slot = Arc::clone(&self.gen);
            let state = Arc::clone(&self.state);
            let detail = Arc::clone(&self.detail);
            let last_pub = Arc::clone(&self.last_pub);
            let (url, label, is_local) = if local {
                let p = resolve_local_video().ok().flatten().unwrap_or_default();
                (p.to_string_lossy().into_owned(), self.source_name(), true)
            } else {
                (self.iss_url().to_string(), cam_label(self.cam), false)
            };
            self.thread = thread::Builder::new()
                .name("omaradio-space".into())
                .spawn(move || {
                    worker(
                        feed, url, label, is_local, slot, flag, gen_slot, gen, state, detail,
                        last_pub,
                    )
                })
                .ok();
        }
    }

    pub fn layout(&mut self, area: Rect) {
        if let Some(child) = &mut self.mpv {
            let health = self
                .watcher
                .lock()
                .map(|mut w| w.observe(child))
                .unwrap_or(NativeHealth::Connecting);
            match health {
                NativeHealth::Playing => {
                    // Upgrade to Live (or clear stale/error detail) once the
                    // watcher sees real decode progress again.
                    if !matches!(self.status().0, SpaceState::Live) {
                        self.set_state(SpaceState::Live, "");
                    }
                }
                NativeHealth::Connecting => {
                    let detail = self.detail.lock().map(|d| d.clone()).unwrap_or_default();
                    if detail.is_empty() {
                        self.set_state(SpaceState::Pending, "connecting…");
                    }
                }
                NativeHealth::Stalled(secs) => {
                    self.set_state(
                        SpaceState::Stale,
                        &format!("buffering / stalled for {}s", secs.as_secs()),
                    );
                }
                NativeHealth::Failed(err) => {
                    if let Some(mut child) = self.mpv.take() {
                        // end-file error can fire while the child is still
                        // alive; dropping without kill+wait leaks the group.
                        if child.try_wait().ok().flatten().is_none() {
                            reap_process_group(&mut child);
                        }
                    }
                    self.set_state(SpaceState::Error, &format!("native mpv: {err}"));
                }
            }
        }
        let same = area == self.area;
        self.area = area;
        if !(self.native
            && self.feed == Some(SpaceFeed::Iss)
            && area.width >= 8
            && area.height >= 4)
        {
            return;
        }
        if same && self.mpv.is_some() {
            return;
        }
        if self.mpv.is_none() {
            // Keep Error visible — do not wipe it with a 750ms reconnect loop.
            if matches!(self.status().0, SpaceState::Error) {
                return;
            }
            let ready = self
                .last_spawn
                .map(|t| t.elapsed() >= Duration::from_millis(750))
                .unwrap_or(true);
            if !ready {
                return;
            }
        }
        self.spawn_kitty();
    }

    pub fn is_native(&self) -> bool {
        self.native && self.mpv.is_some()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.gen.fetch_add(1, Ordering::Relaxed);
        self.thread.take();
        self.stop_native();
        self.feed = None;
        self.native = false;
        self.local = false;
        self.clear_frame();
        self.set_state(SpaceState::Idle, "");
    }

    pub fn snapshot(&self) -> Option<SpaceFrame> {
        self.frame.lock().ok().and_then(|g| g.clone())
    }

    fn stop_flag_reset(&mut self) {
        self.stop = Arc::new(AtomicBool::new(false));
    }

    fn iss_url(&self) -> &'static str {
        ISS_CAMS[self.cam % ISS_CAMS.len()].0
    }

    pub fn cam_name(&self) -> &'static str {
        ISS_CAMS[self.cam % ISS_CAMS.len()].1
    }

    pub fn cycle_cam(&mut self) -> &'static str {
        if self.local {
            return "local video";
        }
        self.cam = (self.cam + 1) % ISS_CAMS.len();
        // Source changed: stale previous-camera frames must not survive the switch.
        self.clear_frame();
        if self.feed == Some(SpaceFeed::Iss) {
            if self.native {
                self.stop_flag_reset();
                self.set_state(SpaceState::Pending, "switching camera…");
                self.spawn_kitty();
            } else {
                self.stop.store(true, Ordering::Relaxed);
                self.gen.fetch_add(1, Ordering::Relaxed);
                self.thread.take();
                self.stop_flag_reset();
                let gen = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
                let flag = Arc::clone(&self.stop);
                let slot = Arc::clone(&self.frame);
                let gen_slot = Arc::clone(&self.gen);
                let state = Arc::clone(&self.state);
                let detail = Arc::clone(&self.detail);
                let last_pub = Arc::clone(&self.last_pub);
                let url = self.iss_url().to_string();
                let label = cam_label(self.cam);
                self.thread = thread::Builder::new()
                    .name("omaradio-space".into())
                    .spawn(move || {
                        worker(
                            SpaceFeed::Iss,
                            url,
                            label,
                            false,
                            slot,
                            flag,
                            gen_slot,
                            gen,
                            state,
                            detail,
                            last_pub,
                        )
                    })
                    .ok();
            }
        }
        self.cam_name()
    }

    /// Ask the native mpv to quit over IPC so its Kitty VO can finish the
    /// escape sequence it is in the middle of and delete its image; only then
    /// kill the process group. A SIGKILL mid-chunk leaves the terminal
    /// swallowing bytes as image data after we exit.
    fn stop_native(&mut self) {
        let Some(mut child) = self.mpv.take() else { return };
        if let Some(ipc) = self.ipc.take() {
            if let Ok(mut s) = UnixStream::connect(&ipc) {
                let _ = s.set_write_timeout(Some(Duration::from_millis(100)));
                let _ = s.write_all(b"{\"command\":[\"quit\"]}\n");
            }
            let deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    let _ = std::fs::remove_file(&ipc);
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _ = std::fs::remove_file(&ipc);
        }
        reap_process_group(&mut child);
    }

    fn spawn_kitty(&mut self) {
        self.stop_native();
        self.watcher = Arc::new(Mutex::new(NativeWatcher::new()));
        self.gen.fetch_add(1, Ordering::Relaxed);
        let area = self.area;
        let ipc = unique_ipc_path();
        let _ = std::fs::remove_file(&ipc);
        let args = native_mpv_cli_args(area, &ipc, self.iss_url(), NativeVo::Kitty);
        let mut cmd = Command::new("mpv");
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .process_group(0);
        // If omaradio dies without running shutdown (SIGHUP from a closed
        // tab, SIGKILL, a crash), the kernel kills mpv with us. Otherwise it
        // lives on as an orphan writing Kitty graphics into the shell.
        // SAFETY: prctl is async-signal-safe and takes no Rust state.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(child) => {
                self.ipc = Some(ipc.clone());
                self.last_spawn = Some(Instant::now());
                let watcher_flag = Arc::clone(&self.stop);
                let watcher_gen = Arc::clone(&self.gen);
                let gen_at_spawn = self.gen.load(Ordering::Relaxed);
                let shared_watcher = Arc::clone(&self.watcher);
                thread::Builder::new()
                    .name("omaradio-space-ipc".into())
                    .spawn(move || {
                        run_native_ipc_loop(
                            &ipc,
                            &watcher_flag,
                            &watcher_gen,
                            gen_at_spawn,
                            shared_watcher,
                        );
                    })
                    .ok();
                self.mpv = Some(child);
            }
            Err(err) => {
                self.last_spawn = Some(Instant::now());
                self.mpv = None;
                self.set_state(SpaceState::Error, &format!("mpv spawn failed: {err}"));
            }
        }
    }
}

fn reap_process_group(child: &mut Child) {
    let pid = child.id();
    let _ = Command::new("kill")
        .args(["-9", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

fn unique_ipc_path() -> PathBuf {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    // Linux sockaddr_un.sun_path is typically 108 bytes including NUL.
    let name = format!("omr{pid}-{n}.sock");
    if let Some(dir) = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
    {
        return dir.join(&name);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty() && p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    dir.join(name)
}

#[derive(Clone, Copy)]
enum NativeVo {
    Kitty,
    Null,
}

/// Production mpv argv (no `--observe-property=` — that is IPC, not CLI).
fn native_mpv_cli_args(area: Rect, ipc: &std::path::Path, media: &str, vo: NativeVo) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--no-config".into(),
        "--really-quiet".into(),
        "--force-window=no".into(),
        "--no-input-default-bindings".into(),
        "--input-terminal=no".into(),
        "--input-vo-keyboard=no".into(),
        "--ao=null".into(),
        "--no-audio".into(),
    ];
    match vo {
        NativeVo::Kitty => {
            args.extend(kitty_vo_placement(
                area,
                crate::kitty::pane_pixel_size(area.width, area.height),
            ));
            args.push("--hwdec=no".into());
        }
        NativeVo::Null => {
            args.push("--vo=null".into());
        }
    }
    args.extend([
        "--profile=low-latency".into(),
        "--video-latency-hacks=yes".into(),
        "--video-sync=desync".into(),
        "--interpolation=no".into(),
        "--framedrop=vo".into(),
        "--cache=no".into(),
        "--demuxer-readahead-secs=0.4".into(),
        "--vd-lavc-threads=1".into(),
        format!("--ytdl-format={ISS_YTDL_FORMAT}").into(),
        "--loop-playlist=inf".into(),
        "--msg-level=all=no".into(),
        format!("--input-ipc-server={}", ipc.display()),
        media.into(),
    ]);
    args
}

/// mpv kitty VO: cols/rows are terminal cells; width/height are pane pixels;
/// left/top are 1-based cell origins (manual: first cell is 1).
fn kitty_vo_placement(area: Rect, (px_w, px_h): (u32, u32)) -> Vec<String> {
    let left = area.x.saturating_add(1);
    let top = area.y.saturating_add(1);
    vec![
        "--vo=kitty".into(),
        "--vo-kitty-alt-screen=no".into(),
        "--vo-kitty-config-clear=no".into(),
        "--vo-kitty-use-shm=yes".into(),
        format!("--vo-kitty-left={left}"),
        format!("--vo-kitty-top={top}"),
        format!("--vo-kitty-cols={}", area.width),
        format!("--vo-kitty-rows={}", area.height),
        format!("--vo-kitty-width={px_w}"),
        format!("--vo-kitty-height={px_h}"),
    ]
}

fn subscribe_native_properties(stream: &mut UnixStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.write_all(crate::mpvwatch::observe_property_command(1, "playback-time").as_bytes())?;
    stream.write_all(crate::mpvwatch::observe_property_command(2, "eof-reached").as_bytes())?;
    stream.set_nonblocking(true)?;
    Ok(())
}

fn run_native_ipc_loop(
    ipc: &std::path::Path,
    watcher_flag: &AtomicBool,
    watcher_gen: &AtomicU64,
    gen_at_spawn: u64,
    shared_watcher: Arc<Mutex<NativeWatcher>>,
) {
    let Some(stream) = wait_for_ipc_cancellable(
        ipc,
        Duration::from_secs(10),
        watcher_flag,
        watcher_gen,
        gen_at_spawn,
    ) else {
        let _ = std::fs::remove_file(ipc);
        return;
    };
    let mut stream = stream;
    let _ = stream.set_nonblocking(true);
    let _ = subscribe_native_properties(&mut stream);
    let mut buf = [0u8; 4096];
    let mut carry: Vec<u8> = Vec::new();
    while !watcher_flag.load(Ordering::Relaxed) && watcher_gen.load(Ordering::Relaxed) == gen_at_spawn
    {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        }
        while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = carry.drain(..=pos).collect();
            if let Ok(text) = std::str::from_utf8(&line) {
                if let Ok(mut w) = shared_watcher.lock() {
                    w.feed_line(text.trim());
                }
            }
        }
    }
    let _ = std::fs::remove_file(ipc);
}

impl Drop for SpaceCam {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker(
    feed: SpaceFeed,
    url: String,
    label: String,
    is_local: bool,
    slot: Arc<Mutex<Option<SpaceFrame>>>,
    stop: Arc<AtomicBool>,
    gen_slot: Arc<AtomicU64>,
    gen: u64,
    state: Arc<Mutex<SpaceState>>,
    detail: Arc<Mutex<String>>,
    last_pub: Arc<Mutex<Option<Instant>>>,
) {
    let live = || !stop.load(Ordering::Relaxed) && gen_slot.load(Ordering::Relaxed) == gen;
    match feed {
        SpaceFeed::Iss => loop {
            if !live() {
                break;
            }
            if is_local {
                let _ = run_local(
                    &url, &label, &slot, &stop, &gen_slot, gen, &state, &detail, &last_pub,
                );
            } else {
                let _ = run_iss(
                    &url, &label, &slot, &stop, &gen_slot, gen, &state, &detail, &last_pub,
                );
            }
            if !live() {
                break;
            }
            set_shared(&state, SpaceState::Pending);
            set_detail(&detail, "reconnecting…");
            thread::sleep(Duration::from_secs(3));
        },
    }
}

fn set_shared<T: Clone>(slot: &Mutex<T>, v: T) {
    if let Ok(mut g) = slot.lock() {
        *g = v;
    }
}

fn set_detail(detail: &Mutex<String>, v: &str) {
    if let Ok(mut d) = detail.lock() {
        *d = v.to_string();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_local(
    path: &str,
    label: &str,
    slot: &Mutex<Option<SpaceFrame>>,
    stop: &AtomicBool,
    gen_slot: &AtomicU64,
    gen: u64,
    state: &Mutex<SpaceState>,
    detail: &Mutex<String>,
    last_pub: &Mutex<Option<Instant>>,
) -> Result<(), ()> {
    let mut child = spawn_local_ffmpeg(path, ISS_W, ISS_H).map_err(|err| {
        set_shared(state, SpaceState::Error);
        set_detail(detail, &err);
    })?;
    set_shared(state, SpaceState::Pending);
    set_detail(detail, "decoding local file…");
    let published = pump_rgb(
        &mut child,
        ISS_W,
        ISS_H,
        label,
        slot,
        stop,
        gen_slot,
        gen,
        state,
        detail,
        last_pub,
        SpaceState::Local,
    );
    let err = drain_stderr(&mut child);
    let _ = child.kill();
    let _ = child.wait();
    if published == 0 {
        set_shared(state, SpaceState::Error);
        set_detail(
            detail,
            &format!(
                "local decoder produced no frames{}",
                err.map(|e| format!(": {e}")).unwrap_or_default()
            ),
        );
    }
    Ok(())
}

fn run_iss(
    page: &str,
    label: &str,
    slot: &Mutex<Option<SpaceFrame>>,
    stop: &AtomicBool,
    gen_slot: &AtomicU64,
    gen: u64,
    state: &Mutex<SpaceState>,
    detail: &Mutex<String>,
    last_pub: &Mutex<Option<Instant>>,
) -> Result<(), ()> {
    set_shared(state, SpaceState::Pending);
    set_detail(detail, "resolving live stream (yt-dlp)…");
    if let Some(mut child) = spawn_iss_mpv(page, ISS_W, ISS_H) {
        if pump_rgb(
            &mut child,
            ISS_W,
            ISS_H,
            label,
            slot,
            stop,
            gen_slot,
            gen,
            state,
            detail,
            last_pub,
            SpaceState::Live,
        ) > 0
        {
            return Ok(());
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let url = resolve_iss(page).ok_or(())?;
    if stop.load(Ordering::Relaxed) || gen_slot.load(Ordering::Relaxed) != gen {
        return Ok(());
    }
    let mut child = spawn_ffmpeg_live(&url, ISS_W, ISS_H).ok_or(())?;
    set_detail(detail, "streaming live camera…");
    let _ = pump_rgb(
        &mut child,
        ISS_W,
        ISS_H,
        label,
        slot,
        stop,
        gen_slot,
        gen,
        state,
        detail,
        last_pub,
        SpaceState::Live,
    );
    Ok(())
}

fn fit_pad(w: u32, h: u32) -> String {
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=fast_bilinear,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black"
    )
}

/// Decode-and-publish loop. Publication policy: every fully decoded frame is
/// published into the single-slot swap (no queue), so nothing is dropped and no
/// unbounded buffering can occur — the latest frame simply overwrites the
/// previous one. The UI consumes at its own render cadence.
#[allow(clippy::too_many_arguments)]
fn pump_rgb(
    child: &mut Child,
    w: u32,
    h: u32,
    label: &str,
    slot: &Mutex<Option<SpaceFrame>>,
    stop: &AtomicBool,
    gen_slot: &AtomicU64,
    gen: u64,
    state: &Mutex<SpaceState>,
    detail: &Mutex<String>,
    last_pub_slot: &Mutex<Option<Instant>>,
    live_state: SpaceState,
) -> u64 {
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return 0;
    };
    let frame_len = (w * h * 3) as usize;
    let mut buf = vec![0u8; frame_len];
    let mut seq = 0u64;
    let mut published = 0u64;
    while !stop.load(Ordering::Relaxed) && gen_slot.load(Ordering::Relaxed) == gen {
        if !read_exact(&mut stdout, &mut buf) {
            break;
        }
        seq += 1;
        published += 1;
        let now = Instant::now();
        if let Ok(mut t) = last_pub_slot.lock() {
            *t = Some(now);
        }
        if published == 1 {
            set_shared(state, live_state);
            set_detail(detail, "");
        }
        let frame = SpaceFrame {
            rgb: Arc::new(std::mem::replace(&mut buf, vec![0u8; frame_len])),
            w,
            h,
            seq,
            label: label.to_string(),
        };
        if let Ok(mut g) = slot.lock() {
            *g = Some(frame);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if published == 0 {
        if let Some(err) = drain_stderr(child) {
            if !err.is_empty() {
                set_detail(detail, &err);
            }
        }
    }
    published
}

fn drain_reader(r: &mut impl Read) -> Option<String> {
    let mut s = String::new();
    r.read_to_string(&mut s).ok()?;
    let t = s.trim().to_string();
    Some(t.lines().last().unwrap_or("").to_string())
}

fn drain_stderr(child: &mut Child) -> Option<String> {
    let mut pipe = child.stderr.take()?;
    drain_reader(&mut pipe)
}

fn spawn_iss_mpv(url: &str, w: u32, h: u32) -> Option<Child> {
    Command::new("mpv")
        .args([
            "--no-config",
            "--really-quiet",
            "--no-terminal",
            "--force-window=no",
            "--ao=null",
            "--profile=low-latency",
            "--video-latency-hacks=yes",
            "--cache=no",
            "--demuxer-readahead-secs=0.3",
            "--hwdec=auto-copy",
            &format!("--ytdl-format={ISS_YTDL_FORMAT}"),
            &format!("--vf=lavfi=[{},format=rgb24]", fit_pad(w, h)),
            "--of=rawvideo",
            "--ovc=rawvideo",
            "--o=-",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn resolve_iss_ytdl_args(watch: &str) -> [&str; 7] {
    [
        "--no-warnings",
        "--socket-timeout",
        "12",
        "-g",
        "-f",
        ISS_YTDL_FORMAT,
        watch,
    ]
}

fn resolve_iss(watch: &str) -> Option<String> {
    let out = Command::new("yt-dlp")
        .args(resolve_iss_ytdl_args(watch))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
}

fn spawn_ffmpeg_live(url: &str, w: u32, h: u32) -> Option<Child> {
    spawn_ffmpeg_args(
        [
            "-hide_banner",
            "-loglevel",
            "error",
            "-fflags",
            "nobuffer+discardcorrupt",
            "-flags",
            "low_delay",
            "-probesize",
            "32768",
            "-analyzeduration",
            "0",
            "-live_start_index",
            "-1",
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-i",
            url,
            "-an",
        ],
        w,
        h,
    )
}

/// Continuous local-file decode: same real RGB pipe the live path uses.
/// `-re` paces at native frame rate; `-stream_loop -1` loops when the file ends.
fn spawn_local_ffmpeg(path: &str, w: u32, h: u32) -> Result<Child, String> {
    if which("ffmpeg").is_none() {
        return Err("ffmpeg not found on PATH — required for local video".into());
    }
    spawn_ffmpeg_args(
        [
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-stream_loop",
            "-1",
            "-threads",
            "1",
            "-i",
            path,
            "-an",
        ],
        w,
        h,
    )
    .ok_or_else(|| format!("failed to spawn ffmpeg for {path}"))
}

fn spawn_ffmpeg_args<const N: usize>(pre: [&str; N], w: u32, h: u32) -> Option<Child> {
    Command::new("ffmpeg")
        .args(pre)
        .args([
            "-vf",
            &fit_pad(w, h),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()
}

/// Like `wait_for_ipc` but polling in short slices so a stop flag or generation
/// bump ends the wait promptly (no blocked thread when the user switches away).
fn wait_for_ipc_cancellable(
    path: &std::path::Path,
    timeout: Duration,
    stop: &AtomicBool,
    gen_slot: &AtomicU64,
    gen: u64,
) -> Option<UnixStream> {
    let start = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) || gen_slot.load(Ordering::Relaxed) != gen {
            return None;
        }
        if start.elapsed() >= timeout {
            return None;
        }
        if let Ok(s) = UnixStream::connect(path) {
            return Some(s);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn which(prog: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(prog))
            .find(|p| p.is_file())
    })
}


fn read_exact(r: &mut impl Read, buf: &mut [u8]) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => return false,
            Ok(n) => filled += n,
            Err(_) => return false,
        }
    }
    true
}

pub fn render_status(buf: &mut Buffer, area: Rect, msg: &str, color: Color) {
    let y = area.y + area.height / 2;
    let x = area.x + 1;
    for (i, ch) in msg
        .chars()
        .take(area.width.saturating_sub(2) as usize)
        .enumerate()
    {
        let cell = &mut buf[(x + i as u16, y)];
        cell.set_char(ch);
        cell.set_fg(color);
    }
}

pub fn render_cells(buf: &mut Buffer, area: Rect, frame: Option<&SpaceFrame>) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let Some(frame) = frame else {
        Paragraphish("locking onto earth…").paint(buf, area);
        return;
    };
    let fw = frame.w.max(1) as usize;
    let fh = frame.h.max(1) as usize;
    let cells_w = area.width as usize;
    let cells_h = area.height as usize;
    for cy in 0..cells_h {
        for cx in 0..cells_w {
            let x = cx * fw / cells_w;
            let y_top = (cy * 2) * fh / (cells_h * 2).max(1);
            let y_bot = ((cy * 2) + 1) * fh / (cells_h * 2).max(1);
            let top = pix(&frame.rgb, fw, fh, x, y_top);
            let bot = pix(&frame.rgb, fw, fh, x, y_bot.min(fh - 1));
            let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
            cell.set_char('▀');
            cell.set_fg(Color::Rgb(top.0, top.1, top.2));
            cell.set_bg(Color::Rgb(bot.0, bot.1, bot.2));
        }
    }
    if !frame.label.is_empty() {
        Paragraphish(&frame.label).paint(
            buf,
            Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(1),
                width: area.width,
                height: 1,
            },
        );
    }
}

fn pix(rgb: &[u8], w: usize, h: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let x = x.min(w.saturating_sub(1));
    let y = y.min(h.saturating_sub(1));
    let i = (y * w + x) * 3;
    if i + 2 >= rgb.len() {
        return (0, 0, 0);
    }
    (rgb[i], rgb[i + 1], rgb[i + 2])
}

struct Paragraphish<'a>(&'a str);

impl Paragraphish<'_> {
    fn paint(self, buf: &mut Buffer, area: Rect) {
        let s = self.0;
        let y = area.y + area.height / 2;
        let x = area.x + 1;
        for (i, ch) in s
            .chars()
            .take(area.width.saturating_sub(2) as usize)
            .enumerate()
        {
            let cell = &mut buf[(x + i as u16, y)];
            cell.set_char(ch);
            cell.set_fg(Color::Rgb(140, 170, 210));
        }
    }
}

pub fn skip_rect(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            buf[(x, y)].set_diff_option(CellDiffOption::Skip);
        }
    }
}

#[cfg(test)]
mod media_step1_tests {
    use super::*;

    /// Drives the real pump_rgb over a paced fake ffmpeg child: writes
    /// n_frames frames at the given interval on a thread, then counts how
    /// many the pump publishes. Verifies the publisher keeps the fixture's
    /// cadence (no drop policy) and exits on stop without leaking.
    struct FakeFf {
        child: Child,
    }

    impl FakeFf {
        fn paced(n_frames: u32, interval: Duration) -> Self {
            use std::io::Write;
            let mut child = Command::new("cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn cat");
            let mut stdin = child.stdin.take().unwrap();
            let frame = vec![7u8; (ISS_W * ISS_H * 3) as usize];
            thread::spawn(move || {
                for _ in 0..n_frames {
                    if stdin.write_all(&frame).is_err() {
                        break;
                    }
                    thread::sleep(interval);
                }
                // Dropping stdin closes the pipe and ends `cat`.
            });
            Self { child }
        }
    }

    #[test]
    fn pump_publishes_every_frame_at_15fps_without_dropping() {
        let mut ff = FakeFf::paced(15, Duration::from_millis(66)); // ~15fps over 1s
        let slot = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let gen_slot = Arc::new(AtomicU64::new(0));
        let state = Arc::new(Mutex::new(SpaceState::Pending));
        let detail = Arc::new(Mutex::new(String::new()));
        let last_pub = Arc::new(Mutex::new(None));
        let published = {
            let stop = Arc::clone(&stop);
            let gen_slot = Arc::clone(&gen_slot);
            let slot = Arc::clone(&slot);
            let state = Arc::clone(&state);
            let detail = Arc::clone(&detail);
            let last_pub = Arc::clone(&last_pub);
            let handle = thread::spawn(move || {
                pump_rgb(
                    &mut ff.child,
                    ISS_W,
                    ISS_H,
                    "test",
                    &slot,
                    &stop,
                    &gen_slot,
                    0,
                    &state,
                    &detail,
                    &last_pub,
                    SpaceState::Local,
                )
            });
            handle.join().expect("pump thread")
        };
        // All 15 frames must be published: no publication-interval drops.
        assert_eq!(published, 15, "every decoded frame must be published");
        let slot = slot.lock().unwrap();
        let frame = slot.as_ref().expect("latest frame retained");
        assert_eq!(frame.seq, 15);
    }

    #[test]
    fn pump_stops_promptly_on_cancel() {
        let mut ff = FakeFf::paced(600, Duration::from_millis(5));
        let slot = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let gen_slot = Arc::new(AtomicU64::new(1)); // generation mismatch → dead
        let state = Arc::new(Mutex::new(SpaceState::Pending));
        let detail = Arc::new(Mutex::new(String::new()));
        let last_pub = Arc::new(Mutex::new(None));
        let published = pump_rgb(
            &mut ff.child,
            ISS_W,
            ISS_H,
            "test",
            &slot,
            &stop,
            &gen_slot,
            0,
            &state,
            &detail,
            &last_pub,
            SpaceState::Local,
        );
        assert_eq!(published, 0);
        assert!(ff.child.try_wait().ok().flatten().is_some(), "child reaped");
    }

    #[test]
    fn stale_promotes_live_only_after_threshold() {
        assert_eq!(
            apply_stale(SpaceState::Live, Some(Duration::from_millis(100))),
            SpaceState::Live
        );
        assert_eq!(
            apply_stale(SpaceState::Live, Some(Duration::from_secs(6))),
            SpaceState::Stale
        );
        assert_eq!(
            apply_stale(SpaceState::Local, Some(Duration::from_secs(6))),
            SpaceState::Stale
        );
        assert_eq!(apply_stale(SpaceState::Pending, None), SpaceState::Pending);
        assert_eq!(apply_stale(SpaceState::Error, None), SpaceState::Error);
        assert_eq!(apply_stale(SpaceState::Idle, None), SpaceState::Idle);
    }

    #[test]
    fn cam_labels_are_source_correct() {
        assert_eq!(cam_label(0), "ISS Earth viewing · NASA ISS live");
        assert_eq!(cam_label(2), "ISS Earth viewing · Sen 4K (HUD)");
        assert_eq!(cam_label(3), "ISS Earth viewing · NASA ISS live");
    }

    #[test]
    fn local_fixture_decodes_via_real_ffmpeg_pipe() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_16x9.mp4");
        // Checked-in fixture is immutable input; never rewrite tests/fake_16x9.mp4.
        let meta = std::fs::metadata(path).unwrap_or_else(|e| {
            panic!("required fixture {path} missing: {e}")
        });
        assert!(
            meta.is_file() && meta.len() > 0,
            "required fixture {path} missing or empty"
        );
        let mut child = spawn_local_ffmpeg(path, ISS_W, ISS_H).expect("spawn local ffmpeg");
        let frame_len = (ISS_W * ISS_H * 3) as usize;
        let mut stdout = child.stdout.take().unwrap();
        let mut buf = vec![0u8; frame_len];
        assert!(read_exact(&mut stdout, &mut buf), "decode one full frame");
        assert!(buf.iter().any(|b| *b != 0), "frame is not all-black");
        let t0 = Instant::now();
        assert!(read_exact(&mut stdout, &mut buf), "second frame (loop or next)");
        assert!(
            t0.elapsed() < Duration::from_millis(400),
            "loop/next frame continuity {:?}",
            t0.elapsed()
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn local_override_validation_rejects_missing_and_empty() {
        let missing = std::path::PathBuf::from("/nonexistent/omaradio-test.mp4");
        assert!(validate_local_file(&missing).is_err());
        let empty_env = std::ffi::OsString::from("");
        // Empty override is an explicit, actionable local-input error — it must
        // never silently fall back to live.
        let err = resolve_local_video_with(empty_env.clone()).unwrap_err();
        assert!(err.contains("set but empty"), "detail: {err}");
        assert!(resolve_local_video_with(empty_env).is_err());
        assert!(
            resolve_local_video_from(None).unwrap().is_none(),
            "unset must be Ok(None), not empty-string error"
        );
        assert!(resolve_local_video_from(Some(std::ffi::OsString::from(""))).is_err());
        let tmp = std::env::temp_dir().join("omaradio-empty-test.mp4");
        std::fs::write(&tmp, b"").unwrap();
        assert!(validate_local_file(&tmp).is_err());
        let _ = std::fs::remove_file(&tmp);
        let ok = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/space.rs");
        assert!(validate_local_file(&ok).is_ok());
    }

    #[test]
    fn native_cli_has_ipc_server_not_observe_property_flags() {
        let ipc = std::path::PathBuf::from("/tmp/omaradio-test.sock");
        let args = native_mpv_cli_args(
            Rect {
                x: 1,
                y: 2,
                width: 40,
                height: 12,
            },
            &ipc,
            "file.mp4",
            NativeVo::Null,
        );
        assert!(args.iter().any(|a| a.starts_with("--input-ipc-server=")));
        assert!(!args.iter().any(|a| a.contains("observe-property")));
        assert!(args.iter().any(|a| a == "--vo=null"));
        assert_eq!(args.last().unwrap(), "file.mp4");
        assert!(!args.iter().any(|a| a == "--idle=yes"));
        assert!(args.iter().any(|a| a == "--vd-lavc-threads=1"));
    }

    #[test]
    fn native_kitty_vo_is_one_based_and_pixel_contained() {
        let area = Rect {
            x: 0,
            y: 3,
            width: 40,
            height: 12,
        };
        let px = crate::kitty::pane_pixel_size_with(area.width, area.height, Some((14, 29)));
        let flags = kitty_vo_placement(area, px);
        assert_eq!(
            flags.iter().find(|a| a.starts_with("--vo-kitty-left=")).map(String::as_str),
            Some("--vo-kitty-left=1")
        );
        assert_eq!(
            flags.iter().find(|a| a.starts_with("--vo-kitty-top=")).map(String::as_str),
            Some("--vo-kitty-top=4")
        );
        assert!(flags.iter().any(|a| a == "--vo-kitty-cols=40"));
        assert!(flags.iter().any(|a| a == "--vo-kitty-rows=12"));
        assert!(flags.iter().any(|a| a == "--vo-kitty-width=560"));
        assert!(flags.iter().any(|a| a == "--vo-kitty-height=348"));
        let ipc = std::path::PathBuf::from("/tmp/omaradio-kitty.sock");
        let args = native_mpv_cli_args(area, &ipc, "file.mp4", NativeVo::Kitty);
        assert!(args.iter().any(|a| a.starts_with("--vo-kitty-width=")));
        assert!(args.iter().any(|a| a.starts_with("--vo-kitty-height=")));
        let smaller = kitty_vo_placement(
            Rect {
                x: 0,
                y: 3,
                width: 20,
                height: 6,
            },
            crate::kitty::pane_pixel_size_with(20, 6, Some((14, 29))),
        );
        let w = |v: &Vec<String>| {
            v.iter()
                .find(|a| a.starts_with("--vo-kitty-width="))
                .unwrap()
                .trim_start_matches("--vo-kitty-width=")
                .parse::<u32>()
                .unwrap()
        };
        assert!(w(&smaller) < w(&flags));
    }

    #[test]
    fn shared_ytdl_prefers_video_only_then_combined_720() {
        assert_eq!(
            ISS_YTDL_FORMAT,
            "bestvideo[height<=720]/best[height<=720]"
        );
        let ipc = std::path::PathBuf::from("/tmp/omaradio-ytdl.sock");
        let native = native_mpv_cli_args(Rect::default(), &ipc, "media", NativeVo::Null);
        let flag = format!("--ytdl-format={ISS_YTDL_FORMAT}");
        assert!(
            native.iter().any(|a| a == &flag),
            "native mpv must share production selector"
        );
        let rgb = resolve_iss_ytdl_args("https://example/watch");
        assert!(rgb.contains(&"-f"));
        assert!(rgb.contains(&ISS_YTDL_FORMAT));
        assert_eq!(rgb[rgb.iter().position(|&a| a == "-f").unwrap() + 1], ISS_YTDL_FORMAT);
    }

    #[test]
    fn installed_mpv_native_ipc_observes_advancing_playback_and_reaps() {
        let mpv = "/usr/bin/mpv";
        if !std::path::Path::new(mpv).is_file() {
            panic!("installed /usr/bin/mpv required for native acceptance");
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_16x9.mp4");
        let meta = std::fs::metadata(path).unwrap_or_else(|e| {
            panic!("required fixture {path} missing: {e}")
        });
        assert!(
            meta.is_file() && meta.len() > 0,
            "required fixture {path} missing or empty"
        );
        let ipc = unique_ipc_path();
        let _ = std::fs::remove_file(&ipc);
        let args = native_mpv_cli_args(Rect::default(), &ipc, path, NativeVo::Null);
        let mut child = Command::new(mpv)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("spawn installed mpv");
        let stop = Arc::new(AtomicBool::new(false));
        let gen = Arc::new(AtomicU64::new(1));
        let watcher = Arc::new(Mutex::new(NativeWatcher::new()));
        let ipc_t = ipc.clone();
        let stop_t = Arc::clone(&stop);
        let gen_t = Arc::clone(&gen);
        let w_t = Arc::clone(&watcher);
        let jh = thread::spawn(move || {
            run_native_ipc_loop(&ipc_t, &stop_t, &gen_t, 1, w_t);
        });
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut playing = false;
        let mut last = NativeHealth::Connecting;
        while Instant::now() < deadline {
            last = watcher.lock().unwrap().observe(&mut child);
            if matches!(last, NativeHealth::Playing) {
                playing = true;
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        struct ReapOnDrop<'a>(&'a mut std::process::Child);
        impl Drop for ReapOnDrop<'_> {
            fn drop(&mut self) {
                crate::space::reap_process_group(self.0);
            }
        }
        let _reap = ReapOnDrop(&mut child);
        stop.store(true, Ordering::Relaxed);
        let _ = jh.join();
        drop(_reap);
        let errlog = {
            let mut s = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut s);
            }
            s
        };
        assert!(
            playing,
            "playback-time must advance via IPC observe_property; last={last:?} stderr={errlog}"
        );
        assert!(
            child.try_wait().ok().flatten().is_some() || {
                thread::sleep(Duration::from_millis(50));
                child.try_wait().ok().flatten().is_some()
            }
        );
        let _ = std::fs::remove_file(&ipc);
    }

    #[test]
    fn installed_mpv_invalid_local_bytes_fail_promptly() {
        let mpv = "/usr/bin/mpv";
        if !std::path::Path::new(mpv).is_file() {
            panic!("installed /usr/bin/mpv required for native acceptance");
        }
        let junk = std::env::temp_dir().join(format!(
            "omaradio-invalid-{}.bin",
            std::process::id()
        ));
        std::fs::write(&junk, b"not a media container\x00\x01\x02 garbage").unwrap();
        let ipc = unique_ipc_path();
        let _ = std::fs::remove_file(&ipc);
        let args = native_mpv_cli_args(
            Rect::default(),
            &ipc,
            junk.to_str().unwrap(),
            NativeVo::Null,
        );
        let mut child = Command::new(mpv)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("spawn installed mpv");
        let stop = Arc::new(AtomicBool::new(false));
        let gen = Arc::new(AtomicU64::new(1));
        let watcher = Arc::new(Mutex::new(NativeWatcher::new()));
        let ipc_t = ipc.clone();
        let stop_t = Arc::clone(&stop);
        let gen_t = Arc::clone(&gen);
        let w_t = Arc::clone(&watcher);
        let jh = thread::spawn(move || {
            run_native_ipc_loop(&ipc_t, &stop_t, &gen_t, 1, w_t);
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut last = NativeHealth::Connecting;
        while Instant::now() < deadline {
            last = watcher.lock().unwrap().observe(&mut child);
            if matches!(last, NativeHealth::Failed(_)) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        struct ReapOnDrop<'a>(&'a mut std::process::Child);
        impl Drop for ReapOnDrop<'_> {
            fn drop(&mut self) {
                crate::space::reap_process_group(self.0);
            }
        }
        let failed = matches!(last, NativeHealth::Failed(_));
        let _reap = ReapOnDrop(&mut child);
        stop.store(true, Ordering::Relaxed);
        let _ = jh.join();
        drop(_reap);
        let errlog = {
            let mut s = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut s);
            }
            s
        };
        let _ = std::fs::remove_file(&junk);
        let _ = std::fs::remove_file(&ipc);
        assert!(
            failed,
            "invalid bytes must Failed promptly via child exit or end-file; last={last:?} stderr={errlog}"
        );
    }
}
