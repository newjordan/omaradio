//! Terminal radio state.

use crate::player::MpvPlayer;
use crate::stations::{self, Station, DIAL};
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
}

impl App {
    pub fn new() -> Result<Self> {
        let selected = stations::default_index();
        let station = &DIAL[selected];
        let mut spectrum = Spectrum::new(station.mood);
        spectrum.set_volume(0.70);
        Ok(Self {
            selected,
            playing: None,
            spectrum,
            player: MpvPlayer::spawn()?,
            status: "idle — pick a station and hit enter".into(),
            show_credits: false,
            should_quit: false,
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
        self.spectrum.set_mood(station.mood);
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
        self.spectrum.set_playing(false);
        self.status = "stopped".into();
    }

    pub fn toggle_credits(&mut self) {
        self.show_credits = !self.show_credits;
    }

    pub fn volume_delta(&mut self, delta: f64) {
        if let Err(err) = self.player.bump_volume(delta) {
            self.status = format!("volume: {err}");
        }
        self.spectrum
            .set_volume((self.player.volume / 100.0).clamp(0.0, 1.0) as f32);
    }

    pub fn tick(&mut self, dt: f32) {
        self.player.poll();
        let live = self.playing.is_some() && self.player.alive && !self.player.paused && !self.player.idle;
        self.spectrum.set_playing(live);
        self.spectrum
            .set_volume((self.player.volume / 100.0).clamp(0.0, 1.0) as f32);
        self.spectrum.tick(dt);

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
        if !self.player.alive {
            self.spectrum.set_playing(false);
        }
    }
}
