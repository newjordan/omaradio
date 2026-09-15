//! CrabMusic-inspired spectrum bars — braille columns, peak gravity, genre mood.
//!
//! Visual language adapted from CrabMusic
//! (https://github.com/newjordan/crabmusic), MIT License
//! Copyright (c) 2025 Frosty40. See ATTRIBUTION.md.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const BAR_COUNT: usize = 48;
const ATTACK: f32 = 0.28;
const RELEASE: f32 = 0.14;
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

impl Mood {
    fn envelope(self, pos: f32) -> f32 {
        let p = pos.clamp(0.0, 1.0);
        match self {
            Mood::Voice => gauss(p, 0.38, 0.16) * 0.85 + gauss(p, 0.18, 0.10) * 0.25,
            Mood::Liquid => {
                gauss(p, 0.10, 0.07) * 1.15
                    + gauss(p, 0.32, 0.12) * 0.70
                    + gauss(p, 0.78, 0.10) * 0.95
            }
            Mood::Space => gauss(p, 0.22, 0.18) * 0.70 + gauss(p, 0.55, 0.22) * 0.55,
            Mood::Mission => {
                gauss(p, 0.20, 0.16) * 0.65 + gauss(p, 0.62, 0.18) * 0.50 + gauss(p, 0.90, 0.05) * 0.45
            }
            Mood::Jazz => {
                gauss(p, 0.12, 0.08) * 0.80 + gauss(p, 0.40, 0.14) * 0.55 + gauss(p, 0.72, 0.10) * 0.35
            }
            Mood::Blues => gauss(p, 0.18, 0.12) * 0.95 + gauss(p, 0.42, 0.14) * 0.70,
            Mood::Brass => {
                gauss(p, 0.14, 0.09) * 0.85
                    + gauss(p, 0.45, 0.12) * 0.90
                    + gauss(p, 0.70, 0.10) * 0.55
            }
        }
    }

    fn tempo(self) -> f32 {
        match self {
            Mood::Voice => 0.55,
            Mood::Liquid => 2.35,
            Mood::Space => 0.28,
            Mood::Mission => 0.34,
            Mood::Jazz => 0.72,
            Mood::Blues => 0.48,
            Mood::Brass => 1.05,
        }
    }

    fn sparsity(self) -> f32 {
        match self {
            Mood::Voice => 0.35,
            Mood::Liquid => 0.08,
            Mood::Space => 0.22,
            Mood::Mission => 0.18,
            Mood::Jazz => 0.42,
            Mood::Blues => 0.28,
            Mood::Brass => 0.16,
        }
    }
}

fn gauss(x: f32, mu: f32, sigma: f32) -> f32 {
    let z = (x - mu) / sigma;
    (-0.5 * z * z).exp()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x as f32) / (u32::MAX as f32)
}

pub struct Spectrum {
    bars: Vec<f32>,
    peaks: Vec<f32>,
    peak_vel: Vec<f32>,
    phase: Vec<f32>,
    mood: Mood,
    playing: bool,
    volume: f32,
    t: f32,
}

impl Spectrum {
    pub fn new(mood: Mood) -> Self {
        Self {
            bars: vec![0.0; BAR_COUNT],
            peaks: vec![0.0; BAR_COUNT],
            peak_vel: vec![0.0; BAR_COUNT],
            phase: (0..BAR_COUNT)
                .map(|i| hash01(i as u32 * 17 + 91) * std::f32::consts::TAU)
                .collect(),
            mood,
            playing: false,
            volume: 0.7,
            t: 0.0,
        }
    }

    pub fn set_mood(&mut self, mood: Mood) {
        if self.mood != mood {
            self.mood = mood;
        }
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    #[cfg(test)]
    pub fn mean_energy(&self) -> f32 {
        if self.bars.is_empty() {
            return 0.0;
        }
        self.bars.iter().sum::<f32>() / self.bars.len() as f32
    }

    pub fn tick(&mut self, dt: f32) {
        self.t += dt;
        let n = self.bars.len();
        let tempo = self.mood.tempo();
        let sparse = self.mood.sparsity();
        let live = if self.playing { 1.0 } else { 0.0 };
        let vol = (self.volume * 0.55 + 0.45) * live;

        for i in 0..n {
            let pos = i as f32 / (n.saturating_sub(1).max(1) as f32);
            let env = self.mood.envelope(pos);
            let wobble = (self.t * tempo * (0.7 + pos * 1.8) + self.phase[i]).sin() * 0.5 + 0.5;
            let hat = (self.t * tempo * 3.4 + self.phase[i] * 2.1).sin().abs();
            let gate = if hash01((i as u32) ^ ((self.t * 4.0) as u32).wrapping_mul(2654435761)) < sparse
            {
                0.12
            } else {
                1.0
            };
            let spark = hash01(i as u32 * 31 + ((self.t * 20.0) as u32)) * 0.10;
            let target = (env * (0.40 + 0.50 * wobble) + hat * env * 0.18 + spark) * gate * vol;
            let target = target.clamp(0.0, 1.0);

            let rate = if target > self.bars[i] { ATTACK } else { RELEASE };
            self.bars[i] = lerp(self.bars[i], target, rate);

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
        let mut s = Spectrum::new(Mood::Liquid);
        s.set_playing(true);
        s.set_volume(0.8);
        for _ in 0..40 {
            s.tick(1.0 / 30.0);
        }
        let live = s.mean_energy();
        assert!(live > 0.05, "live energy {live}");
        s.set_playing(false);
        for _ in 0..90 {
            s.tick(1.0 / 30.0);
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

    #[test]
    fn moods_have_distinct_envelopes() {
        let space = Mood::Space.envelope(0.2);
        let liquid = Mood::Liquid.envelope(0.1);
        assert!(liquid > space);
        assert!(Mood::Voice.envelope(0.9) < Mood::Voice.envelope(0.38));
    }
}
