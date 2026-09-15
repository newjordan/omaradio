//! omaradio — a small night-dial TUI for Soma, old-time, liquid DnB, and midnight jazz.

mod app;
mod analyze;
mod ctl;
mod kitty;
mod milk;
mod mpvwatch;
mod player;
mod pm;
mod space;
mod stations;
mod tap;
mod ui;
mod visual;

use anyhow::Result;
use app::App;
use crossterm::event::{self, DisableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout};
use std::time::{Duration, Instant};

fn main() {
    if let Err(err) = run() {
        eprintln!("omaradio: {err:#}");
        std::process::exit(1);
    }
}

const USAGE: &str = "\
omaradio — night-dial terminal radio

  omaradio                 run the radio
  omaradio ctl …           drive a running radio (see `omaradio ctl help`)
  omaradio --prove-tap     play a test tone and prove the FFT hears the mix
  omaradio --version

  Stations: ~/.config/omaradio/stations.json (built-ins until you add one)
  Agents:   AGENTS.md in the repo, or `omaradio ctl help`
";

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ctl") => return ctl::run_cli(&args[1..]),
        Some("--prove-tap") => return prove_tap(),
        Some("--version") | Some("-V") => {
            println!("omaradio {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help") | Some("-h") | Some("help") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown argument {other:?}\n\n{USAGE}"),
        None => {}
    }
    let mut app = App::new()?;
    let ctl = match ctl::CtlServer::start() {
        Ok(c) => Some(c),
        Err(e) => {
            app.ctl_note = e.to_string();
            None
        }
    };
    // Children and libraries (projectM, mpv) write warnings to stderr; on a
    // TUI that lands in the middle of the screen. Park stderr in a log for
    // the session and restore it before we print anything ourselves.
    let saved_stderr = park_stderr();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let result = event_loop(&mut terminal, &mut app, ctl.as_ref());
    app.shutdown();
    drop(ctl);
    // Close any Kitty graphics chunk still open (a killed mpv can leave one),
    // then drop every image we or mpv placed.
    let _ = kitty::close_chunk(&mut stdout());
    let _ = kitty::delete_all(&mut stdout());
    // mpv's kitty VO switches on any-motion mouse tracking (?1003h) and hides
    // the cursor; if it died hard those stay on and every mouse move types
    // `CXG`-style reports into the shell. Reset them whether or not we think
    // they were set.
    let _ = stdout().execute(DisableMouseCapture);
    let _ = stdout().execute(crossterm::cursor::Show);
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    restore_stderr(saved_stderr);
    result
}

/// Redirect fd 2 to `$XDG_RUNTIME_DIR/omaradio.log` (else /dev/null); return
/// a dup of the original so it can be put back.
fn park_stderr() -> Option<i32> {
    use std::os::fd::AsRawFd;
    let saved = unsafe { libc::dup(2) };
    if saved < 0 {
        return None;
    }
    let log = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| std::path::PathBuf::from(d).join("omaradio.log"))
        .and_then(|p| std::fs::File::create(p).ok())
        .or_else(|| std::fs::OpenOptions::new().write(true).open("/dev/null").ok());
    match log {
        Some(f) => {
            unsafe { libc::dup2(f.as_raw_fd(), 2) };
            Some(saved)
        }
        None => {
            unsafe { libc::close(saved) };
            None
        }
    }
}

fn restore_stderr(saved: Option<i32>) {
    if let Some(fd) = saved {
        unsafe {
            libc::dup2(fd, 2);
            libc::close(fd);
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    ctl: Option<&ctl::CtlServer>,
) -> Result<()> {
    let tick = Duration::from_millis(33);
    let mut last = Instant::now();
    loop {
        if let Some(c) = ctl {
            for (req, reply) in c.drain() {
                let _ = reply.send(app.apply(req));
            }
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        app.sync_space_layout();
        if app.clear_kitty {
            let _ = kitty::delete_all(&mut stdout());
            app.clear_kitty = false;
        }
        if app.wants_kitty_blit() {
            let area = app.viz_area;
            if app.viz == crate::app::VizKind::Iss {
                if let Some(frame) = app.take_space_blit() {
                    let _ = kitty::blit_contain(
                        &mut stdout(),
                        frame.rgb.as_slice(),
                        frame.w,
                        frame.h,
                        area.x,
                        area.y,
                        area.width,
                        area.height,
                    );
                }
            } else {
                let (rgb, w, h) = app.milk_frame();
                let _ = kitty::blit_rgb(
                    &mut stdout(),
                    &rgb,
                    w,
                    h,
                    area.x,
                    area.y,
                    area.width,
                    area.height,
                );
            }
        }

        let timeout = tick.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key.code, key.modifiers);
                }
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().clamp(0.0, 0.1);
        last = now;
        app.tick(dt);

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if mods.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }
    if app.show_credits {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Enter => {
                app.show_credits = false;
            }
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Esc if app.fullscreen => app.toggle_fullscreen(),
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('?') | KeyCode::Char('h') => app.toggle_credits(),
        KeyCode::Char('v') => app.cycle_viz(),
        KeyCode::Char('m') => app.next_milk_preset(),
        KeyCode::Char('M') => app.next_collection_preset(),
        KeyCode::Char('c') => app.cycle_iss_cam(),
        KeyCode::Char('f') => app.toggle_fullscreen(),
        KeyCode::Up | KeyCode::Char('k') => app.select_delta(-1),
        KeyCode::Down | KeyCode::Char('j') => app.select_delta(1),
        KeyCode::Enter | KeyCode::Char('l') => app.tune_selected(),
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char('s') => app.stop(),
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => app.volume_delta(5.0),
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Left => app.volume_delta(-5.0),
        KeyCode::Char('[') | KeyCode::Char('p') => {
            app.select_delta(-1);
            app.tune_selected();
        }
        KeyCode::Char(']') | KeyCode::Char('n') => {
            app.select_delta(1);
            app.tune_selected();
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = c.to_digit(10).unwrap_or(0) as usize;
            if n >= 1 && n <= crate::stations::dial().len() {
                app.tune(n - 1);
            }
        }
        _ => {}
    }
}

/// Live mix proof: play an 8 kHz tone through the same mpv client-name,
/// tap the sink monitor, FFT it, pause, and require the 8 kHz bar to drop.
fn prove_tap() -> Result<()> {
    use crate::analyze::{self, Analyzer, FFT_SIZE};
    use crate::player::MpvPlayer;
    use crate::tap::OutputTap;

    eprintln!("omaradio prove-tap — 8 kHz sine through omaradio mpv → monitor → FFT");
    // This plays an audible 8 kHz tone through the default sink. Never do that
    // on top of someone's radio: refuse if another omaradio is running.
    if let Some(pid) = other_omaradio_pid() {
        if std::env::var_os("OMARADIO_PROVE_ANYWAY").is_none() {
            anyhow::bail!(
                "another omaradio (pid {pid}) is playing — the proof would blast an 8 kHz tone over it. \
                 Quit it first, or set OMARADIO_PROVE_ANYWAY=1."
            );
        }
    }
    eprintln!("WARNING: an 8 kHz test tone will play at low level for ~10 s.");

    let mut cal = Analyzer::new(44_100);
    let cal_bands = cal
        .ingest(&analyze::sine_wave(8000.0, 44_100, FFT_SIZE))
        .expect("in-process 8 kHz FFT");
    let expect = analyze::peak_index(&cal_bands);
    eprintln!(
        "calibrate in-process 8 kHz → bar {expect} e={:.3}",
        cal_bands[expect]
    );
    anyhow::ensure!(
        (38..48).contains(&expect),
        "analyzer maps 8 kHz to bar {expect}, not treble — FFT mapping is wrong"
    );

    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let wav = dir.join(format!("omaradio-prove-{}.wav", std::process::id()));
    write_tone_wav(&wav, 8000.0, 44_100, 14.0)?;

    let mut player = MpvPlayer::spawn()?;
    player.set_volume(PROVE_VOLUME)?;
    player.play_url(&format!("file://{}", wav.display()))?;
    let mut tap = OutputTap::new();
    tap.bind_pid(player.pid());
    tap.set_want_audio(true);
    let mut analyzer = Analyzer::new(tap.sample_rate);
    let mut pcm = Vec::new();
    let warm = Instant::now();
    while warm.elapsed() < Duration::from_millis(1100) {
        player.poll();
        tap.drain(&mut pcm);
        std::thread::sleep(Duration::from_millis(40));
    }
    player.poll();
    eprintln!(
        "mpv title='{}' paused={} idle={}",
        player.now_playing(),
        player.paused,
        player.idle
    );

    let mut play_rms = 0.0f32;
    let mut play_e = 0.0f32;
    let mut play_peak = 0usize;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        player.poll();
        tap.drain(&mut pcm);
        play_rms = play_rms.max(analyze::rms(&pcm));
        if let Some(bands) = analyzer.ingest(&pcm) {
            play_e = play_e.max(bands[expect]);
            play_peak = analyze::peak_index(&bands);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    eprintln!(
        "PLAY  sink={} backend={} rms={:.4} bar{expect}_e={:.3} global_peak={} err={}",
        tap.sink_name().unwrap_or("-"),
        tap.backend_name(),
        play_rms,
        play_e,
        play_peak,
        tap.last_err()
    );

    player.set_paused(true)?;
    std::thread::sleep(Duration::from_millis(800));
    let mut analyzer = Analyzer::new(tap.sample_rate);
    let mut pause_rms = 0.0f32;
    let mut pause_e = 0.0f32;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        player.poll();
        tap.drain(&mut pcm);
        pause_rms = pause_rms.max(analyze::rms(&pcm));
        if let Some(bands) = analyzer.ingest(&pcm) {
            pause_e = pause_e.max(bands[expect]);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    eprintln!(
        "PAUSE sink={} backend={} rms={:.4} bar{expect}_e={:.3}",
        tap.sink_name().unwrap_or("-"),
        tap.backend_name(),
        pause_rms,
        pause_e
    );
    // Resume: how long until the 8 kHz bar is back? This is the end-to-end
    // transport → sink monitor → tap → FFT latency the visualizers see.
    player.set_paused(false)?;
    let resume = Instant::now();
    let mut analyzer = Analyzer::new(tap.sample_rate);
    let mut resume_ms: Option<u128> = None;
    while resume.elapsed() < Duration::from_secs(3) {
        player.poll();
        tap.drain(&mut pcm);
        if let Some(bands) = analyzer.ingest(&pcm) {
            if bands[expect] > 0.15 {
                resume_ms = Some(resume.elapsed().as_millis());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!(
        "RESUME 8 kHz bar {expect} back above 0.15 after {} ms",
        resume_ms.map(|m| m.to_string()).unwrap_or_else(|| "never".into())
    );
    let _ = player.stop();
    let _ = std::fs::remove_file(&wav);

    anyhow::ensure!(
        play_rms > 0.002,
        "play RMS {play_rms:.4} — monitor is silent (tap still wrong)"
    );
    anyhow::ensure!(
        play_e > 0.15,
        "8 kHz bar {expect} energy {play_e:.3} too low — tone never reached the mix tap"
    );
    anyhow::ensure!(
        pause_e < play_e * 0.55,
        "pause did not drop 8 kHz bar {expect} ({pause_e:.3} vs play {play_e:.3}) — not following transport"
    );
    anyhow::ensure!(
        resume_ms.is_some(),
        "8 kHz bar {expect} never came back within 3 s of unpausing"
    );
    eprintln!("prove-tap OK");
    Ok(())
}

/// Test tone level: -12 dBFS sine at mpv volume 50. Loud enough for the FFT
/// gate, not a tinnitus test for whoever is in the room.
const PROVE_AMPLITUDE: f32 = 0.25;
const PROVE_VOLUME: f64 = 50.0;

/// Any other process named `omaradio` (a running radio), by scanning /proc.
fn other_omaradio_pid() -> Option<u32> {
    let me = std::process::id();
    let dir = std::fs::read_dir("/proc").ok()?;
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == me {
            continue;
        }
        if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
            if comm.trim() == "omaradio" {
                return Some(pid);
            }
        }
    }
    None
}

fn write_tone_wav(path: &std::path::Path, freq: f32, sr: u32, secs: f32) -> Result<()> {
    use std::io::Write;
    let n = (sr as f32 * secs) as usize;
    let mut pcm = Vec::with_capacity(n);
    for i in 0..n {
        let s = (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin();
        pcm.push((s * PROVE_AMPLITUDE * i16::MAX as f32) as i16);
    }
    let data_len = (pcm.len() * 2) as u32;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&sr.to_le_bytes())?;
    f.write_all(&(sr * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}
