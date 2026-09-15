//! Log-spaced FFT bands from real PCM. No oscillators.

pub const BAR_COUNT: usize = 48;
pub const FFT_SIZE: usize = 2048;
pub const HOP: usize = 1024;
const FREQ_MIN: f32 = 40.0;
const FREQ_MAX: f32 = 16_000.0;
const DB_FLOOR: f32 = -70.0;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    pending: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    sample_rate: u32,
    waveform: Vec<f32>,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window = hann(FFT_SIZE);
        Self {
            fft,
            window,
            pending: Vec::with_capacity(FFT_SIZE * 2),
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            sample_rate: sample_rate.max(1),
            waveform: vec![0.0; 256],
        }
    }

    pub fn waveform(&self) -> &[f32] {
        &self.waveform
    }

    pub fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        let cap = FFT_SIZE * 6;
        if self.pending.len() > cap {
            let drop = self.pending.len() - cap;
            self.pending.drain(..drop);
        }
    }

    pub fn ingest(&mut self, samples: &[f32]) -> Option<[f32; BAR_COUNT]> {
        self.push(samples);
        let mut latest = None;
        while self.pending.len() >= FFT_SIZE {
            let windowed = &self.pending[..FFT_SIZE];
            latest = Some(spectrum_bands(
                windowed,
                &self.window,
                &self.fft,
                &mut self.scratch,
                self.sample_rate,
            ));
            self.waveform = downsample(windowed, 256);
            self.pending.drain(..HOP);
        }
        latest
    }
}

pub fn spectrum_bands(
    samples: &[f32],
    window: &[f32],
    fft: &Arc<dyn Fft<f32>>,
    scratch: &mut [Complex<f32>],
    sample_rate: u32,
) -> [f32; BAR_COUNT] {
    debug_assert_eq!(samples.len(), FFT_SIZE);
    debug_assert_eq!(window.len(), FFT_SIZE);
    debug_assert_eq!(scratch.len(), FFT_SIZE);

    for i in 0..FFT_SIZE {
        scratch[i] = Complex::new(samples[i] * window[i], 0.0);
    }
    fft.process(scratch);

    let n_bins = FFT_SIZE / 2;
    let mut mags = vec![0.0f32; n_bins];
    let norm = 2.0 / FFT_SIZE as f32;
    for i in 0..n_bins {
        mags[i] = scratch[i].norm() * norm;
    }
    map_log_bands(&mags, sample_rate)
}

pub fn map_log_bands(mags: &[f32], sample_rate: u32) -> [f32; BAR_COUNT] {
    let mut out = [0.0f32; BAR_COUNT];
    let nyquist = sample_rate as f32 / 2.0;
    let bins = mags.len().max(1);
    let hz_per_bin = nyquist / bins as f32;
    if hz_per_bin <= 0.0 {
        return out;
    }

    for i in 0..BAR_COUNT {
        let t0 = i as f32 / BAR_COUNT as f32;
        let t1 = (i + 1) as f32 / BAR_COUNT as f32;
        let f0 = FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf(t0);
        let f1 = FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf(t1);
        let b0 = ((f0 / hz_per_bin).floor() as usize).min(bins - 1);
        let b1 = ((f1 / hz_per_bin).ceil() as usize).clamp(b0 + 1, bins);
        let mut acc = 0.0;
        let mut n = 0.0;
        for mag in &mags[b0..b1] {
            acc += *mag * *mag;
            n += 1.0;
        }
        let rms = if n > 0.0 { (acc / n).sqrt() } else { 0.0 };
        let db = 20.0 * rms.max(1e-9).log10();
        out[i] = ((db - DB_FLOOR) / (0.0 - DB_FLOOR)).clamp(0.0, 1.0);
        out[i] = out[i].powf(0.65);
    }
    out
}

fn downsample(src: &[f32], n: usize) -> Vec<f32> {
    if src.is_empty() || n == 0 {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| src[i * src.len() / n])
        .collect()
}

pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f32::consts::TAU * i as f32 / n.max(1) as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect()
}

#[cfg(test)]
pub fn sine_wave(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            (std::f32::consts::TAU * freq * t).sin()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bands_of(freq: f32) -> [f32; BAR_COUNT] {
        let sr = 44_100;
        let samples = sine_wave(freq, sr, FFT_SIZE);
        let window = hann(FFT_SIZE);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let mut scratch = vec![Complex::new(0.0, 0.0); FFT_SIZE];
        spectrum_bands(&samples, &window, &fft, &mut scratch, sr)
    }

    fn peak_bar(bands: &[f32]) -> usize {
        bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap()
    }

    #[test]
    fn sine_440_peaks_in_the_low_mids() {
        let bands = bands_of(440.0);
        let peak = peak_bar(&bands);
        assert!(
            (12..28).contains(&peak),
            "440 Hz peaked at bar {peak}, energy={:.3}",
            bands[peak]
        );
        assert!(bands[peak] > 0.25, "peak too quiet: {}", bands[peak]);
    }

    #[test]
    fn sine_80_is_left_of_sine_4000() {
        let bass = peak_bar(&bands_of(80.0));
        let treble = peak_bar(&bands_of(4000.0));
        assert!(bass < treble, "bass={bass} treble={treble}");
    }

    #[test]
    fn silence_is_flat() {
        let sr = 44_100;
        let samples = vec![0.0f32; FFT_SIZE];
        let window = hann(FFT_SIZE);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let mut scratch = vec![Complex::new(0.0, 0.0); FFT_SIZE];
        let bands = spectrum_bands(&samples, &window, &fft, &mut scratch, sr);
        let mean = bands.iter().sum::<f32>() / bands.len() as f32;
        assert!(mean < 0.02, "silence mean {mean}");
    }
}
