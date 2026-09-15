//! MilkDrop-style feedback visualizer — an original software renderer.
//!
//! A bank of presets (warp field × shape layer × palette) driven by the live
//! FFT bands, the raw waveform, and a bass onset detector. Presets auto-rotate
//! on a beat once they have been up for a while; `m` skips ahead. None of
//! this is MilkDrop or projectM code; see ATTRIBUTION.md.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::f32::consts::TAU;

pub const PIX_W: u32 = 160;
pub const PIX_H: u32 = 90;
const ASPECT: f32 = PIX_W as f32 / PIX_H as f32;
/// A preset may rotate on a beat once this old…
const HOLD_MIN: f32 = 22.0;
/// …and rotates unconditionally at this age.
const HOLD_MAX: f32 = 36.0;
/// Minimum spacing between detected onsets (≈ 375 BPM ceiling).
const BEAT_GAP: f32 = 0.16;
/// Fresh light added per frame. Feedback with decay d settles near
/// gain / (1 - d), so this keeps sustained music glowing, not white.
const SHAPE_GAIN: f32 = 0.55;

/// Filmic-ish knee: keeps hue in hot zones instead of clipping to white.
fn tonemap(c: f32) -> f32 {
    (1.0 - (-c.max(0.0) * 1.35).exp()).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Warp {
    Zoom,
    Tunnel,
    Swirl,
    Ripple,
    Drift,
    Kaleido,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Ring,
    PolarBars,
    WaveLine,
    Orbit,
    Dots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Palette {
    Accent,
    HueCycle,
    Fire,
    Ice,
    Neon,
}

#[derive(Debug, Clone, Copy)]
struct Preset {
    name: &'static str,
    warp: Warp,
    shape: Shape,
    palette: Palette,
    decay: f32,
}

const PRESETS: &[Preset] = &[
    Preset { name: "liquid tunnel", warp: Warp::Tunnel, shape: Shape::Ring, palette: Palette::Accent, decay: 0.85 },
    Preset { name: "solar swirl", warp: Warp::Swirl, shape: Shape::PolarBars, palette: Palette::Fire, decay: 0.88 },
    Preset { name: "ice ripple", warp: Warp::Ripple, shape: Shape::WaveLine, palette: Palette::Ice, decay: 0.86 },
    Preset { name: "neon kaleido", warp: Warp::Kaleido, shape: Shape::Orbit, palette: Palette::Neon, decay: 0.90 },
    Preset { name: "drift rain", warp: Warp::Drift, shape: Shape::Dots, palette: Palette::HueCycle, decay: 0.92 },
    Preset { name: "deep zoom", warp: Warp::Zoom, shape: Shape::Ring, palette: Palette::HueCycle, decay: 0.89 },
    Preset { name: "orbit fire", warp: Warp::Tunnel, shape: Shape::Orbit, palette: Palette::Fire, decay: 0.87 },
    Preset { name: "bar storm", warp: Warp::Kaleido, shape: Shape::PolarBars, palette: Palette::Accent, decay: 0.90 },
];

pub struct Milkdrop {
    prev: Vec<f32>,
    t: f32,
    preset: usize,
    preset_age: f32,
    /// Slow running average of bass energy; onsets are jumps above it.
    bass_avg: f32,
    /// Beat envelope, 1.0 at an onset, exponential decay.
    pulse: f32,
    since_beat: f32,
    /// Re-armed once bass falls back to the average after an onset, so a
    /// held bass note is one beat, not a strobe.
    armed: bool,
    beats: u32,
    /// White flash on preset change.
    flash: f32,
    hue: f32,
}

impl Milkdrop {
    pub fn new() -> Self {
        Self {
            prev: vec![0.0; (PIX_W * PIX_H * 3) as usize],
            t: 0.0,
            preset: 0,
            preset_age: 0.0,
            bass_avg: 0.0,
            pulse: 0.0,
            since_beat: BEAT_GAP,
            armed: true,
            beats: 0,
            flash: 0.0,
            hue: 0.0,
        }
    }

    pub fn preset_name(&self) -> &'static str {
        PRESETS[self.preset].name
    }

    #[allow(dead_code)]
    pub fn preset_count() -> usize {
        PRESETS.len()
    }

    /// Detected bass onsets so far (diagnostics / tests).
    #[allow(dead_code)]
    pub fn beats(&self) -> u32 {
        self.beats
    }

    /// Current beat envelope in 0..=1.
    #[allow(dead_code)]
    pub fn pulse(&self) -> f32 {
        self.pulse
    }

    pub fn next_preset(&mut self) -> &'static str {
        self.set_preset((self.preset + 1) % PRESETS.len())
    }

    fn set_preset(&mut self, idx: usize) -> &'static str {
        self.preset = idx % PRESETS.len();
        self.preset_age = 0.0;
        self.flash = 0.45;
        PRESETS[self.preset].name
    }

    /// Advance time and the beat detector. `bands` are the live (already
    /// attack/release smoothed) spectrum levels for this frame.
    pub fn tick(&mut self, dt: f32, bands: &[f32]) {
        let dt = dt.clamp(0.0, 0.1);
        self.t += dt;
        self.preset_age += dt;
        self.since_beat += dt;
        self.pulse *= (-dt * 5.5).exp();
        self.flash *= (-dt * 6.0).exp();
        self.hue = (self.hue + dt * 0.02).rem_euclid(1.0);

        let bass = mean(bands, 0, 6);
        // Jump above the running average, with an absolute floor so a near-
        // silent stream (or the first frame, when the average is still zero)
        // never counts as a kick.
        let onset = self.armed
            && bass > 0.12
            && bass > self.bass_avg * 1.35 + 0.05
            && self.since_beat >= BEAT_GAP;
        if !self.armed && bass <= self.bass_avg * 1.05 + 0.02 {
            self.armed = true;
        }
        // ~half-second average at 30 fps.
        self.bass_avg = lerp(self.bass_avg, bass, 0.07);
        if onset {
            self.armed = false;
            self.pulse = 1.0;
            self.since_beat = 0.0;
            self.beats = self.beats.wrapping_add(1);
        }
        if self.preset_age >= HOLD_MAX || (onset && self.preset_age >= HOLD_MIN) {
            self.next_preset();
        }
    }

    pub fn render_rgb(&mut self, wave: &[f32], bands: &[f32], accent: (u8, u8, u8)) -> Vec<u8> {
        let w = PIX_W as usize;
        let h = PIX_H as usize;
        let p = PRESETS[self.preset];
        let bass = mean(bands, 0, 8);
        let mid = mean(bands, 8, 24);
        let treble = mean(bands, 24, bands.len());
        let env = Env {
            t: self.t,
            bass,
            mid,
            treble,
            pulse: self.pulse,
            hue: self.hue,
            accent: (
                accent.0 as f32 / 255.0,
                accent.1 as f32 / 255.0,
                accent.2 as f32 / 255.0,
            ),
        };
        let decay = (p.decay + mid * 0.05).min(0.965);
        let drift = matches!(p.palette, Palette::HueCycle | Palette::Neon);
        let drift_k = 0.02 + treble * 0.05;
        let flash = self.flash * 0.7;

        let mut next = vec![0.0f32; w * h * 3];
        for y in 0..h {
            let v = y as f32 / h as f32 - 0.5;
            for x in 0..w {
                let u = (x as f32 / w as f32 - 0.5) * ASPECT;
                let (su, sv) = warp(p.warp, u, v, &env);
                let (pr, pg, pb) = sample(&self.prev, w, h, su / ASPECT + 0.5, sv + 0.5);
                let (mut r, mut g, mut b) = (pr * decay, pg * decay, pb * decay);
                if drift {
                    let (r0, g0, b0) = (r, g, b);
                    r = r0 * (1.0 - drift_k) + g0 * drift_k;
                    g = g0 * (1.0 - drift_k) + b0 * drift_k;
                    b = b0 * (1.0 - drift_k) + r0 * drift_k;
                }
                let (li, lhue) = shape(p.shape, u, v, x, w, wave, bands, &env);
                if li > 0.002 {
                    let (cr, cg, cb) = paint(p.palette, li.min(1.0), lhue, &env);
                    r += cr * SHAPE_GAIN;
                    g += cg * SHAPE_GAIN;
                    b += cb * SHAPE_GAIN;
                }
                if flash > 0.005 {
                    r += flash;
                    g += flash;
                    b += flash;
                }
                let i = (y * w + x) * 3;
                next[i] = r.clamp(0.0, 1.6);
                next[i + 1] = g.clamp(0.0, 1.6);
                next[i + 2] = b.clamp(0.0, 1.6);
            }
        }
        self.prev = next;
        let mut rgb = vec![0u8; w * h * 3];
        for (i, c) in self.prev.iter().enumerate() {
            rgb[i] = (tonemap(*c) * 255.0) as u8;
        }
        rgb
    }

    /// Non-Kitty fallback: the same preset frame, quantised to shade cells.
    pub fn render_cells(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        wave: &[f32],
        bands: &[f32],
        accent: (u8, u8, u8),
    ) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let rgb = self.render_rgb(wave, bands, accent);
        cells_from_rgb(buf, area, &rgb, PIX_W, PIX_H);
    }
}

/// Quantise any top-down RGB frame to shade cells (the non-Kitty fallback).
pub fn cells_from_rgb(buf: &mut Buffer, area: Rect, rgb: &[u8], pix_w: u32, pix_h: u32) {
    if area.width < 2 || area.height < 2 || rgb.len() < (pix_w * pix_h * 3) as usize {
        return;
    }
    {
        let w = pix_w as usize;
        let h = pix_h as usize;
        let cols = area.width as usize;
        let rows = area.height as usize;
        for cy in 0..rows {
            let py = (cy * h / rows).min(h - 1);
            for cx in 0..cols {
                let px = (cx * w / cols).min(w - 1);
                let i = (py * w + px) * 3;
                let (r, g, b) = (rgb[i], rgb[i + 1], rgb[i + 2]);
                let lum = r.max(g).max(b) as f32 / 255.0;
                let ch = if lum > 0.75 {
                    '█'
                } else if lum > 0.5 {
                    '▓'
                } else if lum > 0.28 {
                    '▒'
                } else if lum > 0.10 {
                    '░'
                } else {
                    ' '
                };
                if ch != ' ' {
                    let boost = |c: u8| ((c as f32 / lum.max(0.2)).min(255.0)) as u8;
                    let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                    cell.set_char(ch);
                    cell.set_fg(Color::Rgb(boost(r), boost(g), boost(b)));
                }
            }
        }
    }
}

struct Env {
    t: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    pulse: f32,
    hue: f32,
    accent: (f32, f32, f32),
}

/// Where this pixel copies its feedback from, in aspect-corrected centred
/// coordinates. Small per-frame displacements accumulate into motion.
fn warp(kind: Warp, u: f32, v: f32, e: &Env) -> (f32, f32) {
    let r = (u * u + v * v).sqrt();
    let ang = v.atan2(u);
    match kind {
        Warp::Zoom => {
            let z = 0.978 - e.bass * 0.05 - e.pulse * 0.05;
            let da = 0.012 * (e.t * 0.13).sin() + e.bass * 0.015 + e.pulse * 0.01;
            rotate(u * z, v * z, da)
        }
        Warp::Tunnel => {
            let r2 = (r - (0.010 + e.bass * 0.025 + e.pulse * 0.02)).max(0.0);
            let a2 = ang + 0.02 * (e.t * 0.5).sin() + e.pulse * 0.01;
            polar(r2, a2)
        }
        Warp::Swirl => {
            let dir = if (e.t * 0.05).sin() >= 0.0 { 1.0 } else { -1.0 };
            let a2 = ang + (0.42 - r).max(0.0) * (0.10 + e.bass * 0.35) * dir;
            let r2 = (r * 0.985 - e.pulse * 0.01).max(0.0);
            polar(r2, a2)
        }
        Warp::Ripple => {
            let r2 = (r * 0.985 + 0.004 * (r * 45.0 - e.t * 5.0).sin() - e.pulse * 0.015).max(0.0);
            let a2 = ang + 0.003 * e.t.sin();
            polar(r2, a2)
        }
        Warp::Drift => {
            let u2 = u + 0.006 * (v * 9.0 + e.t * 1.3).sin();
            let v2 = v - (0.012 + e.bass * 0.02 + e.pulse * 0.01);
            (u2, v2)
        }
        Warp::Kaleido => {
            let wedge = TAU / 6.0;
            let mut a = ang.rem_euclid(wedge);
            if a > wedge * 0.5 {
                a = wedge - a;
            }
            a += e.t * 0.1;
            let r2 = (r * 0.988 - e.pulse * 0.01).max(0.0);
            polar(r2, a)
        }
    }
}

/// Fresh light painted this frame: (intensity 0..1, hue position 0..1).
#[allow(clippy::too_many_arguments)]
fn shape(kind: Shape, u: f32, v: f32, x: usize, w: usize, wave: &[f32], bands: &[f32], e: &Env) -> (f32, f32) {
    let n = bands.len().max(1);
    let r = (u * u + v * v).sqrt();
    let ang = v.atan2(u);
    // Mirror left/right so the low end sits on one axis and treble on the other.
    let a = (ang / TAU).rem_euclid(1.0);
    let a2 = if a > 0.5 { 1.0 - a } else { a } * 2.0;
    let band_at = |t: f32| bands.get(((t * n as f32) as usize).min(n - 1)).copied().unwrap_or(0.0);
    match kind {
        Shape::Ring => {
            let sp = band_at(a2);
            let radius = 0.14 + sp * 0.30 + e.pulse * 0.04;
            let ring = (1.0 - (r - radius).abs() * 22.0).clamp(0.0, 1.0);
            // Soft core that breathes with the bass so the middle never goes dead.
            let core = (-r * r * 110.0).exp() * (0.06 + e.bass * 0.30 + e.pulse * 0.20);
            ((ring * (0.35 + sp) + core).min(1.0), a)
        }
        Shape::PolarBars => {
            let sp = band_at(a2);
            let len = 0.08 + sp * 0.46 + e.pulse * 0.05;
            let frac = (a2 * n as f32).fract();
            let edge = ((1.0 - (frac - 0.5).abs() * 2.0) * 1.8).clamp(0.0, 1.0);
            let radial = if r < len { 1.0 - (r / len) * 0.5 } else { 0.0 };
            (radial * edge * (0.3 + sp * 0.7), a2)
        }
        Shape::WaveLine => {
            let wi = if wave.is_empty() { 0 } else { (x * wave.len() / w.max(1)).min(wave.len() - 1) };
            let wv = wave.get(wi).copied().unwrap_or(0.0);
            let amp = 0.16 + e.mid * 0.25 + e.pulse * 0.10;
            let wy = wv * amp;
            let line = (1.0 - (v - wy).abs() * 30.0).clamp(0.0, 1.0);
            let echo = (1.0 - (v + wy * 0.6).abs() * 30.0).clamp(0.0, 1.0);
            ((line * (0.5 + wv.abs() * 2.0) + echo * 0.35).min(1.0), x as f32 / w as f32)
        }
        Shape::Orbit => {
            const N: usize = 5;
            let mut blob = 0.0f32;
            let mut hue = a;
            let mut best = f32::MAX;
            for i in 0..N {
                let ai = e.t * (0.6 + i as f32 * 0.17) + i as f32 * TAU / N as f32;
                let radius = 0.12 + e.bass * 0.22 + band_at(i as f32 / N as f32) * 0.10;
                let (cx, cy) = polar(radius, ai);
                let d2 = (u - cx) * (u - cx) + (v - cy) * (v - cy);
                blob += (-d2 * (700.0 - e.pulse * 350.0)).exp();
                if d2 < best {
                    best = d2;
                    hue = i as f32 / N as f32;
                }
            }
            let wi = if wave.is_empty() { 0 } else { ((a * wave.len() as f32) as usize).min(wave.len() - 1) };
            let wv = wave.get(wi).copied().unwrap_or(0.0);
            let rw = 0.22 + wv * 0.10 * (1.0 + e.mid);
            let ring = (1.0 - (r - rw).abs() * 40.0).clamp(0.0, 1.0) * 0.6;
            ((blob * (0.6 + e.bass) + ring).min(1.0), hue)
        }
        Shape::Dots => {
            const GX: usize = 8;
            const GY: usize = 6;
            let fx = (u / ASPECT + 0.5).clamp(0.0, 0.9999) * GX as f32;
            let fy = (v + 0.5).clamp(0.0, 0.9999) * GY as f32;
            let (cx, cy) = (fx as usize, fy as usize);
            let bidx = ((cy * GX + cx) * n / (GX * GY)).min(n - 1);
            let sp = bands[bidx];
            let du = fx.fract() - 0.5;
            let dv = fy.fract() - 0.5;
            let d = (du * du + dv * dv).sqrt();
            let radius = 0.08 + sp * 0.34 + e.pulse * 0.06;
            let dot = (1.0 - d / radius).clamp(0.0, 1.0);
            (dot * dot * (0.45 + sp), bidx as f32 / n as f32)
        }
    }
}

fn paint(kind: Palette, k: f32, hue: f32, e: &Env) -> (f32, f32, f32) {
    let (ar, ag, ab) = e.accent;
    match kind {
        Palette::Accent => {
            let white = (k - 0.85).max(0.0) * 1.5;
            ((ar * k + white).min(1.4), (ag * k + white).min(1.4), (ab * k + white).min(1.4))
        }
        Palette::HueCycle => {
            let (r, g, b) = hsv(hue + e.hue + e.treble * 0.2, 0.85, 1.0);
            (r * k, g * k, b * k)
        }
        Palette::Fire => ramp(k, &[(0.0, 0.0, 0.0), (0.85, 0.08, 0.0), (1.0, 0.55, 0.0), (1.0, 1.0, 0.6)]),
        Palette::Ice => ramp(k, &[(0.0, 0.0, 0.0), (0.05, 0.12, 0.6), (0.2, 0.7, 1.0), (0.9, 1.0, 1.0)]),
        Palette::Neon => {
            let comp = (1.0 - ar, 1.0 - ag, 1.0 - ab);
            let (r, g, b) = if ((hue * 6.0) as usize).is_multiple_of(2) { (ar, ag, ab) } else { comp };
            let sat = |c: f32| ((c - 0.5) * 1.6 + 0.5).clamp(0.05, 1.0);
            (sat(r) * k * 1.2, sat(g) * k * 1.2, sat(b) * k * 1.2)
        }
    }
}

fn ramp(k: f32, stops: &[(f32, f32, f32)]) -> (f32, f32, f32) {
    let segs = (stops.len() - 1) as f32;
    let pos = k.clamp(0.0, 1.0) * segs;
    let i = (pos as usize).min(stops.len() - 2);
    let f = pos - i as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    (lerp(a.0, b.0, f), lerp(a.1, b.1, f), lerp(a.2, b.2, f))
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h as usize % 6;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

fn mean(bands: &[f32], lo: usize, hi: usize) -> f32 {
    if bands.is_empty() {
        return 0.0;
    }
    let lo = lo.min(bands.len());
    let hi = hi.clamp(lo + 1, bands.len());
    let slice = &bands[lo..hi];
    slice.iter().sum::<f32>() / slice.len() as f32
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn rotate(x: f32, y: f32, a: f32) -> (f32, f32) {
    let (s, c) = a.sin_cos();
    (x * c - y * s, x * s + y * c)
}

fn polar(r: f32, a: f32) -> (f32, f32) {
    let (s, c) = a.sin_cos();
    (r * c, r * s)
}

/// Bilinear feedback sample; anything warped in from outside the frame is
/// black so tunnels and drifts refill with fresh light instead of smearing.
fn sample(buf: &[f32], w: usize, h: usize, u: f32, v: f32) -> (f32, f32, f32) {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return (0.0, 0.0, 0.0);
    }
    let x = u * (w - 1) as f32;
    let y = v * (h - 1) as f32;
    let x0 = (x as usize).min(w - 1);
    let y0 = (y as usize).min(h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let px = |xx: usize, yy: usize| {
        let i = (yy * w + xx) * 3;
        (buf[i], buf[i + 1], buf[i + 2])
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let top = (lerp(a.0, b.0, fx), lerp(a.1, b.1, fx), lerp(a.2, b.2, fx));
    let bot = (lerp(c.0, d.0, fx), lerp(c.1, d.1, fx), lerp(c.2, d.2, fx));
    (lerp(top.0, bot.0, fy), lerp(top.1, bot.1, fy), lerp(top.2, bot.2, fy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::BAR_COUNT;

    fn brightness(rgb: &[u8]) -> f32 {
        rgb.iter().map(|&c| c as f32).sum::<f32>() / rgb.len() as f32 / 255.0
    }

    #[test]
    fn presets_cycle_and_wrap() {
        let mut m = Milkdrop::new();
        let first = m.preset_name();
        let mut seen = vec![first];
        for _ in 1..Milkdrop::preset_count() {
            let n = m.next_preset();
            assert!(!seen.contains(&n), "duplicate preset name {n}");
            seen.push(n);
        }
        assert_eq!(m.next_preset(), first);
        assert!(Milkdrop::preset_count() >= 6);
    }

    #[test]
    fn every_preset_lights_up_on_audio_within_one_frame() {
        let quiet = [0.0f32; BAR_COUNT];
        let loud = [0.7f32; BAR_COUNT];
        let wave: Vec<f32> = (0..256).map(|i| (i as f32 * 0.2).sin() * 0.6).collect();
        for _ in 0..Milkdrop::preset_count() {
            let mut m = Milkdrop::new();
            let name = m.preset_name();
            m.flash = 0.0;
            let dark = brightness(&m.render_rgb(&[0.0; 256], &quiet, (120, 170, 255)));
            let mut m = Milkdrop::new();
            m.flash = 0.0;
            let lit = brightness(&m.render_rgb(&wave, &loud, (120, 170, 255)));
            assert!(lit > dark + 0.02, "{name}: quiet {dark:.3} vs loud {lit:.3}");
            assert!(dark < 0.05, "{name}: silence should be near black, got {dark:.3}");
            let _ = m.next_preset();
        }
    }

    #[test]
    fn bass_jump_registers_as_a_beat_immediately() {
        let mut m = Milkdrop::new();
        let mut quiet = [0.05f32; BAR_COUNT];
        for _ in 0..40 {
            m.tick(1.0 / 30.0, &quiet);
        }
        assert_eq!(m.beats(), 0);
        quiet[..6].iter_mut().for_each(|b| *b = 0.8);
        m.tick(1.0 / 30.0, &quiet);
        assert_eq!(m.beats(), 1);
        assert!(m.pulse() > 0.8, "pulse {}", m.pulse());
        // Sustained bass is not a stream of beats.
        for _ in 0..10 {
            m.tick(1.0 / 30.0, &quiet);
        }
        assert_eq!(m.beats(), 1);
        assert!(m.pulse() < 0.5, "pulse should decay, got {}", m.pulse());
    }

    #[test]
    fn presets_auto_rotate_on_a_late_beat_and_at_the_hard_cap() {
        let mut m = Milkdrop::new();
        let start = m.preset_name();
        let bands = [0.05f32; BAR_COUNT];
        let mut t = 0.0;
        while t < HOLD_MIN + 0.5 {
            m.tick(1.0 / 30.0, &bands);
            t += 1.0 / 30.0;
        }
        assert_eq!(m.preset_name(), start, "no beat, under the cap: hold");
        let mut kick = bands;
        kick[..6].iter_mut().for_each(|b| *b = 0.9);
        m.tick(1.0 / 30.0, &kick);
        assert_ne!(m.preset_name(), start, "late beat rotates the preset");
        let second = m.preset_name();
        let mut t = 0.0;
        while t < HOLD_MAX + 0.5 {
            m.tick(1.0 / 30.0, &bands);
            t += 1.0 / 30.0;
        }
        assert_ne!(m.preset_name(), second, "hard cap rotates without a beat");
    }

    #[test]
    fn feedback_stays_bounded_and_finite() {
        let mut m = Milkdrop::new();
        let loud = [1.0f32; BAR_COUNT];
        let wave = vec![0.9f32; 256];
        for _ in 0..120 {
            m.tick(1.0 / 30.0, &loud);
            let rgb = m.render_rgb(&wave, &loud, (255, 255, 255));
            assert_eq!(rgb.len(), (PIX_W * PIX_H * 3) as usize);
        }
        assert!(m.prev.iter().all(|c| c.is_finite() && *c <= 1.6));
    }

    /// `OMARADIO_MILK_DUMP=/dir cargo test --release -- --ignored dump_frames`
    /// writes a PPM per preset after a few seconds of synthetic music.
    #[test]
    #[ignore]
    fn dump_frames() {
        let Some(dir) = std::env::var_os("OMARADIO_MILK_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        let mut m = Milkdrop::new();
        for pi in 0..Milkdrop::preset_count() {
            m.flash = 0.0;
            let mut rgb = Vec::new();
            for f in 0..90 {
                let t = f as f32 / 30.0;
                let beat = if f % 15 == 0 { 0.9 } else { 0.25 };
                let bands: Vec<f32> = (0..BAR_COUNT)
                    .map(|i| {
                        let x = i as f32 / BAR_COUNT as f32;
                        ((1.0 - x) * beat + 0.3 * (t * 3.0 + x * 9.0).sin().abs() * (1.0 - x * 0.5)).clamp(0.0, 1.0)
                    })
                    .collect();
                let wave: Vec<f32> = (0..256).map(|i| (i as f32 * 0.15 + t * 20.0).sin() * 0.5 * (0.3 + beat)).collect();
                m.tick(1.0 / 30.0, &bands);
                rgb = m.render_rgb(&wave, &bands, (80, 220, 170));
            }
            let mut out = format!("P6 {} {} 255\n", PIX_W, PIX_H).into_bytes();
            out.extend_from_slice(&rgb);
            std::fs::write(dir.join(format!("{pi}-{}.ppm", m.preset_name().replace(' ', "_"))), out).unwrap();
            let _ = m.next_preset();
        }
    }

    #[test]
    fn frame_time_report() {
        let mut m = Milkdrop::new();
        let bands = [0.5f32; BAR_COUNT];
        let wave: Vec<f32> = (0..256).map(|i| (i as f32 * 0.1).sin()).collect();
        let start = std::time::Instant::now();
        let frames = 60;
        for _ in 0..frames {
            m.tick(1.0 / 30.0, &bands);
            let _ = m.render_rgb(&wave, &bands, (80, 220, 170));
        }
        let per = start.elapsed().as_secs_f32() * 1000.0 / frames as f32;
        eprintln!("milkdrop render: {per:.2} ms/frame ({} px)", PIX_W * PIX_H);
    }
}
