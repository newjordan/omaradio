//! Night-dial layout.

use crate::app::{App, VizKind};
use crate::space::SpaceState;
use crate::stations;
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
        draw_cinema(frame, app, area);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(6),
                Constraint::Length((stations::dial().len() as u16).saturating_add(2).min(12)),
                Constraint::Length(2),
            ])
            .split(area);

        draw_header(frame, app, chunks[0]);
        draw_now(frame, app, chunks[1]);
        draw_viz(frame, app, chunks[2]);
        draw_dial(frame, app, chunks[3]);
        draw_keys(frame, app, chunks[4], false);
    }
    if app.show_credits {
        draw_credits(frame, area);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let on = app.on_air();
    let current = app.current();
    let accent = on.as_ref().unwrap_or(&current).accent;
    let accent = Color::Rgb(accent.0, accent.1, accent.2);
    let vol = app.player.props.volume;
    let title = match on.as_ref() {
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

    let loaded = on.is_some();
    let paused = app.player.props.paused && !app.player.props.idle;
    let live = loaded && !paused && !app.player.props.idle && app.player.alive;
    let badge = if !app.player.alive {
        Span::styled(
            " mpv down ",
            Style::default().fg(LIVE).add_modifier(Modifier::BOLD),
        )
    } else if live {
        Span::styled(
            " ● LIVE ",
            Style::default().fg(LIVE).add_modifier(Modifier::BOLD),
        )
    } else if loaded && paused {
        Span::styled(" ■ PAUSE", Style::default().fg(Color::Rgb(255, 196, 80)))
    } else {
        Span::styled(" ○ IDLE ", Style::default().fg(MUTED))
    };
    frame.render_widget(Paragraph::new(Line::from(badge)), cols[0]);

    let gauge_fg = if vol > 100.0 {
        Color::Rgb(255, 176, 90)
    } else {
        accent
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_fg).bg(Color::Rgb(32, 28, 36)))
        .ratio(crate::player::volume_ratio(vol))
        .label(format!("vol {vol:.0}"));
    frame.render_widget(gauge, cols[1]);
}

fn draw_now(frame: &mut Frame, app: &App, area: Rect) {
    let shown = app.shown_status();
    let text = if shown.is_empty() {
        app.current().blurb
    } else {
        shown
    };
    let text = fit_width(&text, area.width.saturating_sub(2) as usize);
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(text, Style::default().fg(PAPER)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn paint_mix_hud(frame: &mut Frame, app: &App, area: Rect) -> Rect {
    if !matches!(app.viz, VizKind::Bars | VizKind::Wave | VizKind::Milk) || area.height < 2 {
        return area;
    }
    let hud_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            app.mix_hud_line(),
            Style::default().fg(MUTED),
        ))),
        hud_area,
    );
    Rect {
        height: area.height.saturating_sub(1),
        ..area
    }
}

fn draw_cinema(frame: &mut Frame, app: &mut App, area: Rect) {
    let hint_h = if area.height > 8 { 1 } else { 0 };
    let viz_area = Rect {
        height: area.height.saturating_sub(hint_h),
        ..area
    };
    let current = app.current();
    let accent = app.on_air().unwrap_or(current).accent;
    let area = paint_mix_hud(frame, app, viz_area);
    app.viz_area = area;
    match app.viz {
        VizKind::Bars => app.spectrum.render(frame.buffer_mut(), area, accent),
        VizKind::Wave => visual::render_wave(frame.buffer_mut(), area, app.waveform(), accent),
        VizKind::Milk => {
            if !app.kitty {
                app.milk_cells(frame.buffer_mut(), area);
            }
        }
        VizKind::Iss if app.space_native() => {
            crate::space::skip_rect(frame.buffer_mut(), area);
        }
        VizKind::Iss => {
            let frame_rgb = app.space_frame();
            if !app.kitty || frame_rgb.is_none() {
                crate::space::render_cells(frame.buffer_mut(), area, frame_rgb.as_ref());
            }
        }
    }
    if hint_h == 1 {
        let hint = Rect {
            y: viz_area.y + viz_area.height,
            height: 1,
            ..viz_area
        };
        draw_keys(frame, app, hint, true);
    }
}

fn draw_viz(frame: &mut Frame, app: &mut App, area: Rect) {
    let current = app.current();
    let accent = app.on_air().unwrap_or(current).accent;
    let (state, detail) = if app.viz == VizKind::Iss {
        app.space_status()
    } else {
        (SpaceState::Idle, String::new())
    };
    let title = match app.viz {
        VizKind::Bars => " bars ".into(),
        VizKind::Wave => " wave ".into(),
        VizKind::Milk => format!(" milkdrop · {} ", app.milk_preset()),
        VizKind::Iss => format!(" ISS · {} ", app.space_source()),
    };
    let title = if app.viz == VizKind::Iss {
        format!("{}{} ", title.trim_end(), state_badge(state))
    } else {
        title
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Span::styled(
            title,
            Style::default()
                .fg(state_color(state))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = paint_mix_hud(frame, app, inner);
    app.viz_area = inner;
    // No frame yet or an error: say so truthfully instead of showing old pixels.
    if app.viz == VizKind::Iss
        && matches!(
            state,
            SpaceState::Pending | SpaceState::Error | SpaceState::Stale
        )
    {
        let msg = if detail.is_empty() {
            state_text(state).to_string()
        } else if matches!(state, SpaceState::Pending | SpaceState::Stale) {
            detail
        } else {
            format!("{} — {}", state_text(state), detail)
        };
        crate::space::render_status(frame.buffer_mut(), inner, &msg, state_color(state));
        return;
    }
    match app.viz {
        VizKind::Bars => app.spectrum.render(frame.buffer_mut(), inner, accent),
        VizKind::Wave => visual::render_wave(frame.buffer_mut(), inner, app.waveform(), accent),
        VizKind::Milk => {
            if !app.kitty {
                app.milk_cells(frame.buffer_mut(), inner);
            }
        }
        VizKind::Iss if app.space_native() => {
            crate::space::skip_rect(frame.buffer_mut(), inner);
        }
        VizKind::Iss => {
            let frame_rgb = app.space_frame();
            if !app.kitty || frame_rgb.is_none() {
                crate::space::render_cells(frame.buffer_mut(), inner, frame_rgb.as_ref());
            }
        }
    }
}

fn state_color(state: SpaceState) -> Color {
    match state {
        SpaceState::Live | SpaceState::Local => OK,
        SpaceState::Pending => Color::Rgb(255, 196, 80),
        SpaceState::Error => LIVE,
        SpaceState::Stale => Color::Rgb(255, 140, 60),
        SpaceState::Idle => MUTED,
    }
}

fn state_badge(state: SpaceState) -> String {
    match state {
        SpaceState::Live => " · live",
        SpaceState::Local => " · local",
        SpaceState::Pending => " · pending",
        SpaceState::Stale => " · stale",
        SpaceState::Error => " · error",
        SpaceState::Idle => "",
    }
    .into()
}

fn state_text(state: SpaceState) -> &'static str {
    match state {
        SpaceState::Pending => "locking on…",
        SpaceState::Stale => "signal stalled — last frame is old",
        SpaceState::Error => "signal error",
        _ => "waiting",
    }
}

fn draw_dial(frame: &mut Frame, app: &App, area: Rect) {
    let dial = stations::dial();
    let adhoc = app.adhoc_station().cloned();
    let inner_rows = area.height.saturating_sub(2) as usize;
    let adhoc_rows = usize::from(adhoc.is_some() && inner_rows > 0);
    let window = dial_window(
        dial.len(),
        app.selected,
        inner_rows.saturating_sub(adhoc_rows),
    );
    let title = if window.start == 0 && window.end == dial.len() {
        " dial ".to_string()
    } else {
        format!(" dial  {}–{}/{} ", window.start + 1, window.end, dial.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Span::styled(title, Style::default().fg(MUTED)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let name_w = dial
        .iter()
        .map(|s| s.name.chars().count().min(28))
        .max()
        .unwrap_or(18)
        .max(18);
    let width = inner.width as usize;
    let mut lines = Vec::new();
    if let Some(st) = adhoc {
        lines.push(dial_row(DialRow {
            marker: "▶",
            num: "·",
            name: &st.name,
            detail: "stream by url",
            name_w,
            width,
            selected: false,
            on_air: true,
            accent: Color::Rgb(st.accent.0, st.accent.1, st.accent.2),
        }));
    }
    for i in window {
        let station = &dial[i];
        let selected = i == app.selected;
        let on_air = app.playing == Some(i);
        let marker = if on_air {
            "▶"
        } else if selected {
            "›"
        } else {
            " "
        };
        let detail = format!("{}  ·  {}", station.source, station.blurb);
        let num = format!("{}", i + 1);
        lines.push(dial_row(DialRow {
            marker,
            num: &num,
            name: &station.name,
            detail: &detail,
            name_w,
            width,
            selected,
            on_air,
            accent: Color::Rgb(station.accent.0, station.accent.1, station.accent.2),
        }));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

struct DialRow<'a> {
    marker: &'a str,
    num: &'a str,
    name: &'a str,
    detail: &'a str,
    name_w: usize,
    width: usize,
    selected: bool,
    on_air: bool,
    accent: Color,
}

fn dial_row(row: DialRow<'_>) -> Line<'static> {
    let DialRow {
        marker,
        num,
        name,
        detail,
        name_w,
        width,
        selected,
        on_air,
        accent,
    } = row;
    let bg = if selected {
        Color::Rgb(42, 34, 52)
    } else {
        INK
    };
    let name_style = if selected {
        Style::default()
            .fg(PAPER)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else if on_air {
        Style::default().fg(accent).bg(bg)
    } else {
        Style::default().fg(MUTED).bg(bg)
    };
    let mark_style = if on_air {
        Style::default().fg(OK).bg(bg).add_modifier(Modifier::BOLD)
    } else if selected {
        Style::default().fg(accent).bg(bg)
    } else {
        Style::default().fg(LINE).bg(bg)
    };
    let detail_style = Style::default()
        .fg(if selected { PAPER } else { MUTED })
        .bg(bg);
    let head = format!(" {marker} {num:>2}  ");
    let name_cell = format!("{:<name_w$}", fit_width(name, name_w));
    let used = head.chars().count() + name_cell.chars().count();
    let room = width.saturating_sub(used);
    let detail = if room == 0 {
        String::new()
    } else {
        let body = format!("  {detail}");
        let body = fit_width(&body, room);
        let pad = room.saturating_sub(body.chars().count());
        format!("{body}{}", " ".repeat(pad))
    };
    Line::from(vec![
        Span::styled(head, mark_style),
        Span::styled(name_cell, name_style),
        Span::styled(detail, detail_style),
    ])
}

fn draw_keys(frame: &mut Frame, app: &App, area: Rect, fullscreen: bool) {
    let text = if !app.ctl_note.is_empty() {
        format!("  ctl offline · {}", app.ctl_note)
    } else if fullscreen {
        KEYS_FULLSCREEN.to_string()
    } else if area.height >= 2 {
        format!("{KEYS_TOP}\n{KEYS_BOTTOM}")
    } else {
        KEYS_BOTTOM.to_string()
    };
    let lines: Vec<Line> = text
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                fit_width(line, area.width as usize),
                Style::default().fg(if app.ctl_note.is_empty() {
                    MUTED
                } else {
                    Color::Rgb(255, 176, 90)
                }),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Two footer rows. Each is kept within 80 columns so an 80-wide terminal
/// still shows quit.
const KEYS_TOP: &str = "  ↑↓/jk select   ⏎ play   space pause   ←→ vol   [ ]/np station   1-9 tune";
const KEYS_BOTTOM: &str = "  v viz   m/M milk   c ISS cam   f cinema   s stop   ? credits   q quit";
const KEYS_FULLSCREEN: &str =
    "  f exit   v viz   m/M preset   c cam   space pause   ←→ vol   q quit";

fn draw_credits(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 170, 255)))
        .style(Style::default().bg(INK).fg(PAPER))
        .title(Span::styled(
            " credits · ? or esc closes ",
            Style::default()
                .fg(Color::Rgb(120, 170, 255))
                .add_modifier(Modifier::BOLD),
        ));

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
    for station in stations::dial().iter() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<24}", station.name),
                Style::default().fg(PAPER),
            ),
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
        Line::from(Span::styled(
            "  NASA ISS    official HD earth-view livestream  public NASA stream",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "  Sen 4K      optional third ISS cam             © Sen, via YouTube",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "  projectM    MilkDrop engine for M (optional)   LGPL, loaded at runtime",
            Style::default().fg(PAPER),
        )),
        Line::from(Span::styled(
            "  presets     Cream of the Crop pack             © their authors, via projectM",
            Style::default().fg(PAPER),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "SomaFM® is a trademark of SomaFM. WWOZ is New Orleans Public Radio.",
            Style::default().fg(MUTED),
        )),
        Line::from(Span::styled(
            "Full text in ATTRIBUTION.md",
            Style::default().fg(MUTED),
        )),
    ]);

    // Size the card to its text so nothing is clipped on short terminals.
    let height = (lines.len() as u16 + 2).min(area.height);
    let width = 80.min(area.width);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
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

/// Keep `selected` on screen. The window is centered on the selection when
/// the dial is longer than the pane.
fn dial_window(len: usize, selected: usize, rows: usize) -> std::ops::Range<usize> {
    if len == 0 || rows == 0 {
        return 0..0;
    }
    let rows = rows.min(len);
    let selected = selected.min(len - 1);
    let max_start = len - rows;
    let start = selected.saturating_sub(rows / 2).min(max_start);
    start..start + rows
}

fn fit_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".into();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_window_keeps_the_selection_visible() {
        assert_eq!(dial_window(0, 0, 5), 0..0);
        assert_eq!(dial_window(4, 3, 10), 0..4);
        assert_eq!(dial_window(20, 0, 5), 0..5);
        let end = dial_window(20, 19, 5);
        assert!(end.contains(&19), "{end:?}");
        assert_eq!(end.end - end.start, 5);
        let mid = dial_window(20, 10, 5);
        assert!(mid.contains(&10), "{mid:?}");
        assert_eq!(mid.end - mid.start, 5);
    }

    #[test]
    fn fit_width_ellipsizes_without_exceeding_the_column() {
        assert_eq!(fit_width("abcd", 4), "abcd");
        assert_eq!(fit_width("abcdef", 4), "abc…");
        assert_eq!(fit_width("áéíóú!", 3).chars().count(), 3);
        assert_eq!(fit_width("hi", 0), "");
    }

    #[test]
    fn footer_hints_fit_an_eighty_column_terminal() {
        for line in [KEYS_TOP, KEYS_BOTTOM, KEYS_FULLSCREEN] {
            assert!(
                line.chars().count() <= 80,
                "{line} is {} cols",
                line.chars().count()
            );
        }
    }
}
