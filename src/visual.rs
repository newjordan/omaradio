//! CrabMusic-inspired spectrum bars — braille columns, peak gravity.
//! Driven by a real FFT of the speaker mix (PipeWire monitor), not a
//! mood oscillator.
//!
//! Visual language adapted from CrabMusic
//! (https://github.com/newjordan/crabmusic), MIT License
//! Copyright (c) 2025 Frosty40. See ATTRIBUTION.md.

use crate::analyze::BAR_COUNT;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const ATTACK: f32 = 0.35;
const RELEASE: f32 = 0.18;
const GRAVITY: f32 = 0.012;
const PEAK_FLOOR: f32 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Voice,
    Liquid,
    Space,
    Mission,
    Jazz,
    Blues,
    Brass,
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub struct Spectrum {
    bars: Vec<f32>,
    peaks: Vec<f32>,
    peak_vel: Vec<f32>,
}

impl Spectrum {
    pub fn new() -> Self {
        Self {
            bars: vec![0.0; BAR_COUNT],
            peaks: vec![0.0; BAR_COUNT],
            peak_vel: vec![0.0; BAR_COUNT],
        }
    }

    #[cfg(test)]
    pub fn mean_energy(&self) -> f32 {
        if self.bars.is_empty() {
            return 0.0;
        }
        self.bars.iter().sum::<f32>() / self.bars.len() as f32
    }

    pub fn tick(&mut self, bands: Option<&[f32]>) {
        let n = self.bars.len();
        if let Some(bands) = bands {
            for i in 0..n {
                let target = bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let rate = if target > self.bars[i] { ATTACK } else { RELEASE };
                self.bars[i] = lerp(self.bars[i], target, rate);
            }
        }
        for i in 0..n {
            if self.bars[i] > self.peaks[i] && self.bars[i] > PEAK_FLOOR {
                self.peaks[i] = self.bars[i];
                self.peak_vel[i] = 0.0;
            } else if self.peaks[i] > 0.0 {
                self.peak_vel[i] += GRAVITY;
                self.peaks[i] = (self.peaks[i] - self.peak_vel[i]).max(0.0);
            }
        }
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, accent: (u8, u8, u8)) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let width = area.width as usize;
        let height = area.height as usize;
        let mut dots = vec![0u8; width * height];
        let mut colors = vec![(0u8, 0u8, 0u8); width * height];

        let dot_w = width * 2;
        let dot_h = height * 4;
        let bar_w = (dot_w / BAR_COUNT).max(1);

        for (bar_idx, &level) in self.bars.iter().enumerate() {
            let x0 = bar_idx * bar_w;
            if x0 >= dot_w {
                break;
            }
            let x1 = (x0 + bar_w).min(dot_w);
            let boosted = level.min(1.0);
            let bar_dots = (boosted * dot_h as f32) as usize;
            let peak_y = dot_h.saturating_sub((self.peaks[bar_idx] * dot_h as f32) as usize);
            let color = bar_color(bar_idx, BAR_COUNT, boosted, accent);

            for x in x0..x1 {
                for dy in 0..bar_dots {
                    let y = dot_h - 1 - dy;
                    set_dot(&mut dots, &mut colors, width, height, x, y, color);
                }
                if self.peaks[bar_idx] > PEAK_FLOOR {
                    set_dot(
                        &mut dots,
                        &mut colors,
                        width,
                        height,
                        x,
                        peak_y.min(dot_h - 1),
                        peak_color(accent),
                    );
                }
            }
        }

        for cy in 0..height {
            for cx in 0..width {
                let ch = braille_char(dots[cy * width + cx]);
                if ch == ' ' {
                    continue;
                }
                let (r, g, b) = colors[cy * width + cx];
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                cell.set_char(ch);
                cell.set_fg(Color::Rgb(r, g, b));
            }
        }
    }
}

fn set_dot(
    dots: &mut [u8],
    colors: &mut [(u8, u8, u8)],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    color: (u8, u8, u8),
) {
    let cx = x / 2;
    let cy = y / 4;
    if cx >= width || cy >= height {
        return;
    }
    let lx = x % 2;
    let ly = y % 4;
    let bit = match (lx, ly) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0,
    };
    let idx = cy * width + cx;
    dots[idx] |= bit;
    colors[idx] = color;
}

fn braille_char(bits: u8) -> char {
    if bits == 0 {
        ' '
    } else {
        char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
    }
}

fn bar_color(idx: usize, n: usize, intensity: f32, accent: (u8, u8, u8)) -> (u8, u8, u8) {
    let t = idx as f32 / n.max(1) as f32;
    let bass = (255.0, 70.0, 40.0);
    let mid = (accent.0 as f32, accent.1 as f32, accent.2 as f32);
    let treble = (90.0, 210.0, 255.0);
    let (br, bg, bb) = if t < 0.33 {
        let k = t / 0.33;
        (
            lerp(bass.0, mid.0, k),
            lerp(bass.1, mid.1, k),
            lerp(bass.2, mid.2, k),
        )
    } else if t < 0.66 {
        let k = (t - 0.33) / 0.33;
        (
            lerp(mid.0, treble.0, k),
            lerp(mid.1, treble.1, k),
            lerp(mid.2, treble.2, k),
        )
    } else {
        treble
    };
    let i = intensity.clamp(0.0, 1.0);
    let i = 0.18 + i * 0.82;
    (
        (br * i) as u8,
        (bg * i) as u8,
        (bb * i) as u8,
    )
}

fn peak_color(accent: (u8, u8, u8)) -> (u8, u8, u8) {
    (
        accent.0.saturating_add(40).min(255),
        accent.1.saturating_add(40).min(255),
        accent.2.saturating_add(40).min(255),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_spectrum_decays_to_rest() {
        let mut s = Spectrum::new();
        let loud = [0.8f32; crate::analyze::BAR_COUNT];
        for _ in 0..12 {
            s.tick(Some(&loud));
        }
        let live = s.mean_energy();
        assert!(live > 0.2, "live energy {live}");
        let quiet = [0.0f32; crate::analyze::BAR_COUNT];
        for _ in 0..90 {
            s.tick(Some(&quiet));
        }
        let rest = s.mean_energy();
        assert!(rest < 0.03, "rest energy {rest}");
        assert!(s.peaks.iter().all(|&p| p < 0.15));
    }

    #[test]
    fn braille_bits_roundtrip() {
        assert_eq!(braille_char(0), ' ');
        assert_eq!(braille_char(0xFF), '⣿');
        assert_ne!(braille_char(0x01), ' ');
    }
}
