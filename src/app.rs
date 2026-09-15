//! Terminal radio state.

use crate::analyze::Analyzer;
use crate::milk::Milkdrop;
use crate::player::MpvPlayer;
use crate::space::{SpaceCam, SpaceFeed};
use crate::stations::{self, Station, DIAL};
use crate::tap::{MixHud, OutputTap};
use crate::visual::Spectrum;
use anyhow::Result;
use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizKind {
    Bars,
    Wave,
    Milk,
    Iss,
}

impl VizKind {
    fn blit(self) -> bool {
        matches!(self, VizKind::Milk | VizKind::Iss)
    }

    fn space_feed(self) -> Option<SpaceFeed> {
        match self {
            VizKind::Iss => Some(SpaceFeed::Iss),
            _ => None,
        }
    }
}

pub struct App {
    pub selected: usize,
    pub playing: Option<usize>,
    pub spectrum: Spectrum,
    pub player: MpvPlayer,
    pub status: String,
    pub show_credits: bool,
    pub should_quit: bool,
    pub viz: VizKind,
    pub fullscreen: bool,
    pub milk: Milkdrop,
    pub kitty: bool,
    pub viz_area: Rect,
    pub clear_kitty: bool,
    space: SpaceCam,
    last_space_seq: u64,
    tap: OutputTap,
    analyzer: Analyzer,
    pcm: Vec<f32>,
    mix_rms: f32,
    peak_bar: usize,
    fft_ok: bool,
}

impl App {
    pub fn new() -> Result<Self> {
        let selected = stations::default_index();
        let tap = OutputTap::new();
        let sample_rate = tap.sample_rate;
        let mut app = Self {
            selected,
            playing: None,
            spectrum: Spectrum::new(),
            player: MpvPlayer::spawn()?,
            status: "idle — pick a station and hit enter".into(),
            show_credits: false,
            should_quit: false,
            viz: VizKind::Iss,
            fullscreen: false,
            milk: Milkdrop::new(),
            kitty: crate::kitty::available(),
            viz_area: Rect::default(),
            clear_kitty: false,
            space: SpaceCam::new(),
            last_space_seq: 0,
            analyzer: Analyzer::new(sample_rate),
            tap,
            pcm: Vec::with_capacity(4096),
            mix_rms: 0.0,
            peak_bar: 0,
            fft_ok: false,
        };
        app.tap.bind_pid(app.player.pid());
        app.space.set(SpaceFeed::Iss, app.kitty);
        app.tune(app.selected);
        Ok(app)
    }

    pub fn shutdown(&mut self) {
        self.space.stop();
        let _ = self.player.stop();
    }

    pub fn station(&self, idx: usize) -> &'static Station {
        &DIAL[idx.min(DIAL.len() - 1)]
    }

    pub fn current(&self) -> &'static Station {
        self.station(self.selected)
    }

    pub fn on_air(&self) -> Option<&'static Station> {
        self.playing.map(|i| self.station(i))
    }

    pub fn waveform(&self) -> &[f32] {
        self.analyzer.waveform()
    }

    pub fn tap_live(&self) -> bool {
        self.tap.active() && self.mix_rms > 1e-5
    }

    pub fn mix_hud(&self) -> MixHud {
        MixHud {
            sink: self.tap.sink_name().unwrap_or("-").to_string(),
            backend: self.tap.backend_name().to_string(),
            rms: self.mix_rms,
            peak_bar: self.peak_bar,
            fft_ok: self.fft_ok,
            err: self.tap.last_err(),
            samples: 0,
        }
    }

    pub fn mix_hud_line(&self) -> String {
        self.mix_hud().line()
    }

    pub fn select_delta(&mut self, delta: isize) {
        let n = DIAL.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(n);
        self.selected = next as usize;
    }

    pub fn tune_selected(&mut self) {
        self.tune(self.selected);
    }

    pub fn tune(&mut self, idx: usize) {
        if idx >= DIAL.len() {
            return;
        }
        self.selected = idx;
        let station = self.station(idx);
        match self.player.play_url(station.url) {
            Ok(()) => {
                self.playing = Some(idx);
                self.status = format!("tuning {}…", station.name);
            }
            Err(err) => {
                self.status = format!("tune failed: {err}");
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.playing.is_none() {
            self.tune_selected();
            return;
        }
        if let Err(err) = self.player.toggle_pause() {
            self.status = format!("pause failed: {err}");
        }
    }

    pub fn stop(&mut self) {
        let _ = self.player.stop();
        self.playing = None;
        self.status = "stopped".into();
    }

    pub fn toggle_credits(&mut self) {
        self.show_credits = !self.show_credits;
        // The card is cells; a lingering blit would sit on top of it.
        if self.kitty && self.viz.blit() {
            self.clear_kitty = true;
        }
    }

    pub fn cycle_iss_cam(&mut self) {
        if self.viz != VizKind::Iss {
            return;
        }
        let name = self.space.cycle_cam();
        self.status = format!("ISS cam · {name}");
    }

    pub fn cycle_viz(&mut self) {
        let next = match self.viz {
            VizKind::Bars => VizKind::Wave,
            VizKind::Wave => VizKind::Milk,
            VizKind::Milk => VizKind::Iss,
            VizKind::Iss => VizKind::Bars,
        };
        self.set_viz(next);
    }

    fn set_viz(&mut self, next: VizKind) {
        // Any switch drops whatever image is on screen: the native ISS mpv
        // paints under its own Kitty image id, so replacing ours is not enough.
        if self.kitty && self.viz != next {
            self.clear_kitty = true;
        }
        self.viz = next;
        if let Some(feed) = next.space_feed() {
            self.last_space_seq = 0;
            self.space.set(feed, self.kitty);
        } else {
            self.space.stop();
        }
        let name = match next {
            VizKind::Bars => "bars",
            VizKind::Wave => "wave",
            VizKind::Milk => "milkdrop",
            VizKind::Iss => "ISS earth view",
        };
        self.status = format!("viz · {name}");
    }

    /// Skip to the next milkdrop preset; switches the viz to milkdrop if needed.
    pub fn next_milk_preset(&mut self) {
        if self.viz != VizKind::Milk {
            self.set_viz(VizKind::Milk);
        }
        let name = self.milk.next_preset();
        self.status = format!("milkdrop · {name}");
    }

    pub fn milk_preset(&self) -> &'static str {
        self.milk.preset_name()
    }

    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            if !self.viz.blit() {
                self.set_viz(VizKind::Milk);
            }
            self.status = format!(
                "full · {}",
                match self.viz {
                    VizKind::Milk => "milkdrop",
                    VizKind::Iss => "ISS",
                    _ => "viz",
                }
            );
        } else {
            if self.kitty {
                self.clear_kitty = true;
            }
            self.status = "dial".into();
        }
    }

    pub fn volume_delta(&mut self, delta: f64) {
        if let Err(err) = self.player.bump_volume(delta) {
            self.status = format!("volume: {err}");
        }
    }

    pub fn wants_kitty_blit(&self) -> bool {
        self.kitty && self.viz.blit() && !self.show_credits && !self.space.is_native()
    }

    pub fn space_native(&self) -> bool {
        self.space.is_native()
    }

    pub fn sync_space_layout(&mut self) {
        if self.viz == VizKind::Iss {
            self.space.layout(self.viz_area);
        }
    }

    pub fn space_frame(&self) -> Option<crate::space::SpaceFrame> {
        self.space.snapshot()
    }

    pub fn space_status(&self) -> (crate::space::SpaceState, String) {
        self.space.status()
    }

    pub fn space_source(&self) -> String {
        self.space.source_name()
    }

    pub fn take_space_blit(&mut self) -> Option<crate::space::SpaceFrame> {
        let frame = self.space.snapshot()?;
        if frame.seq == self.last_space_seq {
            return None;
        }
        self.last_space_seq = frame.seq;
        Some(frame)
    }

    pub fn milk_frame(&mut self) -> Vec<u8> {
        let accent = self.on_air().unwrap_or_else(|| self.current()).accent;
        let wave = self.analyzer.waveform().to_vec();
        let bands = self.spectrum.levels().to_vec();
        self.milk.render_rgb(&wave, &bands, accent)
    }

    pub fn tick(&mut self, dt: f32) {
        self.player.poll();

        self.tap.set_want_audio(self.playing.is_some() && !self.player.paused);
        self.tap.drain(&mut self.pcm);
        self.mix_rms = crate::analyze::rms(&self.pcm);
        let bands = self.analyzer.ingest(&self.pcm);
        if let Some(ref b) = bands {
            self.peak_bar = crate::analyze::peak_index(b);
            self.fft_ok = b.iter().copied().fold(0.0_f32, f32::max) > 0.04;
        }
        self.spectrum.tick(bands.as_ref().map(|b| b.as_slice()));
        self.milk.tick(dt, self.spectrum.levels());

        if let Some(err) = self.player.last_error.take() {
            self.status = format!("stream: {err}");
        } else if let Some(station) = self.on_air() {
            if self.player.paused {
                self.status = format!("paused · {}", station.name);
            } else {
                let now = self.player.now_playing();
                self.status = if now.is_empty() {
                    format!("on air · {}", station.name)
                } else {
                    format!("{}  ·  {}", station.name, now)
                };
            }
        }
    }
}
