//! MilkDrop-style feedback visualizer — original software renderer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

pub const PIX_W: u32 = 160;
pub const PIX_H: u32 = 90;

pub struct Milkdrop {
    prev: Vec<f32>,
    t: f32,
}

impl Milkdrop {
    pub fn new() -> Self {
        Self {
            prev: vec![0.0; (PIX_W * PIX_H * 3) as usize],
            t: 0.0,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.t += dt;
    }

    pub fn render_rgb(&mut self, wave: &[f32], bands: &[f32], accent: (u8, u8, u8)) -> Vec<u8> {
        let w = PIX_W as usize;
        let h = PIX_H as usize;
        let bass = mean(bands, 0, 8);
        let mid = mean(bands, 8, 24);
        let treble = mean(bands, 24, bands.len());
        let zoom = 0.97 - bass * 0.06;
        let rot = self.t * (0.18 + bass * 0.9);
        let mut next = vec![0.0f32; w * h * 3];
        let ax = accent.0 as f32 / 255.0;
        let ay = accent.1 as f32 / 255.0;
        let az = accent.2 as f32 / 255.0;

        for y in 0..h {
            for x in 0..w {
                let u = x as f32 / w as f32 - 0.5;
                let v = y as f32 / h as f32 - 0.5;
                let (su, sv) = rotate(u * zoom, v * zoom, rot * 0.04);
                let (pr, pg, pb) = sample(&self.prev, w, h, su + 0.5, sv + 0.5);
                let rpol = (u * u + v * v).sqrt();
                let ang = v.atan2(u) + rot;
                let plasma = ((ang * 3.0 + self.t).sin() * (rpol * 9.0 - self.t * 2.2).sin())
                    * 0.5
                    + 0.5;
                let band_i = (((ang / std::f32::consts::TAU).rem_euclid(1.0)) * bands.len().max(1) as f32)
                    as usize
                    % bands.len().max(1);
                let sp = bands.get(band_i).copied().unwrap_or(0.0);
                let ring = (1.0 - (rpol - 0.18 - sp * 0.32).abs() * 18.0).clamp(0.0, 1.0);
                let wi = if wave.is_empty() {
                    0
                } else {
                    (x * wave.len() / w.max(1)).min(wave.len() - 1)
                };
                let wv = wave.get(wi).copied().unwrap_or(0.0);
                let wy = 0.5 + wv * (0.18 + treble * 0.15);
                let wave_g = (1.0 - ((y as f32 / h as f32) - wy).abs() * 36.0).clamp(0.0, 1.0);

                let decay = 0.86 + mid * 0.04;
                let mut r = pr * decay + plasma * (0.20 + bass * 0.45) + ring * ax * 0.7 + wave_g;
                let mut g = pg * decay + plasma * (0.12 + mid * 0.35) + ring * ay * 0.7 + wave_g * 0.85;
                let mut b = pb * decay + plasma * (0.18 + treble * 0.55) + ring * az * 0.7 + wave_g * 0.7;
                r = r.clamp(0.0, 1.5);
                g = g.clamp(0.0, 1.5);
                b = b.clamp(0.0, 1.5);
                let i = (y * w + x) * 3;
                next[i] = r;
                next[i + 1] = g;
                next[i + 2] = b;
            }
        }
        self.prev = next;
        let mut rgb = vec![0u8; w * h * 3];
        for (i, c) in self.prev.iter().enumerate() {
            rgb[i] = (c.clamp(0.0, 1.0) * 255.0) as u8;
        }
        rgb
    }

    pub fn render_cells(
        &self,
        buf: &mut Buffer,
        area: Rect,
        wave: &[f32],
        bands: &[f32],
        accent: (u8, u8, u8),
    ) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let w = area.width as usize;
        let h = area.height as usize;
        let bass = mean(bands, 0, 8);
        let mid = mean(bands, 8, 24);
        let treble = mean(bands, 24, bands.len());
        for cy in 0..h {
            for cx in 0..w {
                let u = cx as f32 / w.max(1) as f32 - 0.5;
                let v = cy as f32 / h.max(1) as f32 - 0.5;
                let rpol = (u * u + v * v).sqrt();
                let ang = v.atan2(u) + self.t * (0.2 + bass);
                let plasma = ((ang * 3.0 + self.t).sin() * (rpol * 8.0 - self.t * 2.0).sin()) * 0.5 + 0.5;
                let wi = if wave.is_empty() {
                    0
                } else {
                    (cx * wave.len() / w.max(1)).min(wave.len() - 1)
                };
                let wv = wave.get(wi).copied().unwrap_or(0.0);
                let wy = 0.5 + wv * 0.28;
                let wave_g = (1.0 - ((cy as f32 / h.max(1) as f32) - wy).abs() * 14.0).clamp(0.0, 1.0);
                let glow = (plasma * (0.35 + mid * 0.4) + wave_g).clamp(0.0, 1.0);
                let r = ((accent.0 as f32 * (0.25 + bass) + 80.0 * treble + 255.0 * wave_g) * glow)
                    .clamp(0.0, 255.0) as u8;
                let g = ((accent.1 as f32 * (0.25 + mid) + 40.0 + 220.0 * wave_g) * glow)
                    .clamp(0.0, 255.0) as u8;
                let b = ((accent.2 as f32 * (0.25 + treble) + 90.0 * plasma + 200.0 * wave_g) * glow)
                    .clamp(0.0, 255.0) as u8;
                let ch = if wave_g > 0.55 {
                    '━'
                } else if glow > 0.7 {
                    '█'
                } else if glow > 0.45 {
                    '▓'
                } else if glow > 0.22 {
                    '▒'
                } else if glow > 0.08 {
                    '░'
                } else {
                    ' '
                };
                if ch != ' ' {
                    let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                    cell.set_char(ch);
                    cell.set_fg(Color::Rgb(r, g, b));
                }
            }
        }
    }
}

fn mean(bands: &[f32], lo: usize, hi: usize) -> f32 {
    let slice = if bands.is_empty() {
        return 0.0;
    } else {
        let lo = lo.min(bands.len());
        let hi = hi.clamp(lo + 1, bands.len());
        &bands[lo..hi]
    };
    slice.iter().sum::<f32>() / slice.len() as f32
}

fn rotate(x: f32, y: f32, a: f32) -> (f32, f32) {
    let (s, c) = a.sin_cos();
    (x * c - y * s, x * s + y * c)
}

fn sample(buf: &[f32], w: usize, h: usize, u: f32, v: f32) -> (f32, f32, f32) {
    let x = (u.clamp(0.0, 0.999) * w as f32) as usize;
    let y = (v.clamp(0.0, 0.999) * h as f32) as usize;
    let i = (y.min(h - 1) * w + x.min(w - 1)) * 3;
    (buf[i], buf[i + 1], buf[i + 2])
}
