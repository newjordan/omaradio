//! openradio — a small night-dial TUI for Soma, old-time, liquid DnB, and midnight jazz.

mod app;
mod analyze;
mod player;
mod stations;
mod tap;
mod ui;
mod visual;

use anyhow::Result;
use app::App;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout};
use std::time::{Duration, Instant};

fn main() {
    if let Err(err) = run() {
        eprintln!("openradio: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut app = App::new()?;
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let tick = Duration::from_millis(33);
    let mut last = Instant::now();
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

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
        KeyCode::Esc if app.show_credits => app.show_credits = false,
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('?') | KeyCode::Char('h') => app.toggle_credits(),
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
            if n >= 1 && n <= crate::stations::DIAL.len() {
                app.tune(n - 1);
            }
        }
        _ => {}
    }
}
