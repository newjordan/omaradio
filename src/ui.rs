//! Night-dial layout.

use crate::app::{App, VizKind};
use crate::stations::DIAL;
use crate::visual;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};
use ratatui::Frame;

const INK: Color = Color::Rgb(18, 16, 22);
const PAPER: Color = Color::Rgb(232, 224, 210);
const MUTED: Color = Color::Rgb(140, 132, 124);
const LINE: Color = Color::Rgb(58, 50, 62);
const LIVE: Color = Color::Rgb(255, 92, 70);
const OK: Color = Color::Rgb(90, 210, 140);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(INK).fg(PAPER)),
        area,
    );

    if app.fullscreen {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(area);
        draw_header(frame, app, chunks[0]);
        draw_now(frame, app, chunks[1]);
        draw_viz(frame, app, chunks[2]);
        draw_keys(frame, chunks[3], true);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(6),
                Constraint::Length((DIAL.len() as u16).saturating_add(2).min(12)),
                Constraint::Length(2),
            ])
            .split(area);

        draw_header(frame, app, chunks[0]);
        draw_now(frame, app, chunks[1]);
        draw_viz(frame, app, chunks[2]);
        draw_dial(frame, app, chunks[3]);
        draw_keys(frame, chunks[4], false);
    }
    if app.show_credits {
        draw_credits(frame, area);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let on = app.on_air();
    let accent = on.unwrap_or_else(|| app.current()).accent;
    let accent = Color::Rgb(accent.0, accent.1, accent.2);
    let vol = app.player.volume.clamp(0.0, 100.0);
    let title = match on {
        Some(s) => format!(" OMARADIO  ·  {}", s.name.to_uppercase()),
        None => " OMARADIO  ·  NIGHT DIAL".into(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(14), Constraint::Min(10)])
        .split(inner);

    let live = app.playing.is_some() && !app.player.paused && app.player.alive;
    let badge = if !app.player.alive {
        Span::styled(" mpv down ", Style::default().fg(LIVE).add_modifier(Modifier::BOLD))
    } else if live {
        Span::styled(" ● LIVE ", Style::default().fg(LIVE).add_modifier(Modifier::BOLD))
    } else if app.player.paused {
        Span::styled(" ■ PAUSE", Style::default().fg(Color::Rgb(255, 196, 80)))
    } else {
        Span::styled(" ○ IDLE ", Style::default().fg(MUTED))
    };
    frame.render_widget(Paragraph::new(Line::from(badge)), cols[0]);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(accent).bg(Color::Rgb(32, 28, 36)))
        .ratio((vol / 100.0).clamp(0.0, 1.0))
        .label(format!("vol {vol:.0}"));
    frame.render_widget(gauge, cols[1]);
}

fn draw_now(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.status.is_empty() {
        app.current().blurb.to_string()
    } else {
        app.status.clone()
    };
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(PAPER)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_viz(frame: &mut Frame, app: &mut App, area: Rect) {
    let accent = app.on_air().unwrap_or_else(|| app.current()).accent;
    let title = match app.viz {
        VizKind::Bars => " bars ",
        VizKind::Wave => " wave ",
        VizKind::Milk => " milkdrop ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Span::styled(title, Style::default().fg(MUTED)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.viz_area = inner;
    match app.viz {
        VizKind::Bars => app.spectrum.render(frame.buffer_mut(), inner, accent),
        VizKind::Wave => visual::render_wave(frame.buffer_mut(), inner, app.waveform(), accent),
        VizKind::Milk => {
            if !app.kitty {
                let wave = app.waveform().to_vec();
                let bands = app.spectrum.levels().to_vec();
                app.milk
                    .render_cells(frame.buffer_mut(), inner, &wave, &bands, accent);
            }
        }
    }
}

fn draw_dial(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Span::styled(" dial ", Style::default().fg(MUTED)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::with_capacity(DIAL.len());
    for (i, station) in DIAL.iter().enumerate() {
        let selected = i == app.selected;
        let on_air = app.playing == Some(i);
        let marker = if on_air { "▶" } else if selected { "›" } else { " " };
        let num = format!("{}", i + 1);
        let accent = Color::Rgb(station.accent.0, station.accent.1, station.accent.2);
        let name_style = if selected {
            Style::default().fg(PAPER).add_modifier(Modifier::BOLD)
        } else if on_air {
            Style::default().fg(accent)
        } else {
            Style::default().fg(MUTED)
        };
        let mark_style = if on_air {
            Style::default().fg(OK).add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default().fg(accent)
        } else {
            Style::default().fg(LINE)
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {marker} {num}  "), mark_style),
            Span::styled(format!("{:<18}", station.name), name_style),
            Span::styled(
                format!("  {}  ·  {}", station.source, station.blurb),
                Style::default().fg(if selected { PAPER } else { MUTED }),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_keys(frame: &mut Frame, area: Rect, fullscreen: bool) {
    let keys = if fullscreen {
        "  f exit milkdrop   v viz   space pause   +/- vol   ? credits   q quit"
    } else {
        "  ↑↓/jk select   ⏎ play   space pause   +/- vol   v viz   f milkdrop   1-7 tune   s stop   ? credits   q quit"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(keys, Style::default().fg(MUTED))),
        area,
    );
}

fn draw_credits(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 78, 20);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 170, 255)))
        .style(Style::default().bg(INK).fg(PAPER))
        .title(Span::styled(
            " credits ",
            Style::default()
                .fg(Color::Rgb(120, 170, 255))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![
        Line::from(Span::styled(
            "OMARADIO  ·  unofficial night dial  ·  not affiliated with any station",
            Style::default().fg(MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Broadcasters — streams belong to them. Support the stations.",
            Style::default()
                .fg(Color::Rgb(255, 196, 80))
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for station in DIAL {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<20}", station.name), Style::default().fg(PAPER)),
            Span::styled(
                format!("{}  {}", station.source, station.homepage),
                Style::default().fg(MUTED),
            ),
        ]));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Software",
            Style::default()
                .fg(Color::Rgb(255, 196, 80))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  CrabMusic   braille spectrum + peak gravity  MIT © 2025 Frosty40",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "              https://github.com/newjordan/crabmusic",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "  mpv         playback engine                    https://mpv.io",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "  ratatui     terminal UI                        https://ratatui.rs",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "  crossterm / serde / serde_json / anyhow",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "  Omarchy     the desktop this was built for     https://omarchy.org",
            Style::default().fg(PAPER),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "SomaFM® is a trademark of SomaFM. WWOZ is New Orleans Public Radio.",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "? or Esc closes this card. Full text in ATTRIBUTION.md",
            Style::default().fg(MUTED),
        )),
    ]);

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true }),
        inner,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
