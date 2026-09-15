//! Terminal radio state.

use crate::analyze::Analyzer;
use crate::player::MpvPlayer;
use crate::stations::{self, Station, DIAL};
use crate::tap::OutputTap;
use crate::visual::Spectrum;
use anyhow::Result;

pub struct App {
    pub selected: usize,
    pub playing: Option<usize>,
    pub spectrum: Spectrum,
    pub player: MpvPlayer,
    pub status: String,
    pub show_credits: bool,
    pub should_quit: bool,
    tap: Option<OutputTap>,
    analyzer: Analyzer,
    pcm: Vec<f32>,
}

impl App {
    pub fn new() -> Result<Self> {
        let selected = stations::default_index();
        let tap = OutputTap::try_open();
        let sample_rate = tap.as_ref().map(|t| t.sample_rate).unwrap_or(44_100);
        Ok(Self {
            selected,
            playing: None,
            spectrum: Spectrum::new(),
            player: MpvPlayer::spawn()?,
            status: "idle — pick a station and hit enter".into(),
            show_credits: false,
            should_quit: false,
            analyzer: Analyzer::new(sample_rate),
            tap,
            pcm: Vec::with_capacity(4096),
        })
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
    }

    pub fn volume_delta(&mut self, delta: f64) {
        if let Err(err) = self.player.bump_volume(delta) {
            self.status = format!("volume: {err}");
        }
    }

    pub fn tick(&mut self, _dt: f32) {
        self.player.poll();

        let bands = if let Some(tap) = &self.tap {
            tap.drain(&mut self.pcm);
            self.analyzer.ingest(&self.pcm)
        } else {
            None
        };
        self.spectrum.tick(bands.as_ref().map(|b| b.as_slice()));

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
