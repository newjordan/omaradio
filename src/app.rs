//! Terminal radio state.

use crate::analyze::Analyzer;
use crate::milk::Milkdrop;
use crate::player::MpvPlayer;
use crate::space::{SpaceCam, SpaceFeed};
use crate::ctl::Request;
use crate::pm::ProjectM;
use crate::stations::{self, Station};
use serde_json::{json, Value};
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
    /// projectM, brought up on the first `M`.
    pm: Option<ProjectM>,
    pm_error: Option<String>,
    /// true = milkdrop shows the projectM collection, false = built-ins.
    collection: bool,
    /// A stream tuned by url that is not on the dial.
    adhoc: Option<&'static Station>,
    /// Why the control socket is not up, if it is not.
    pub ctl_note: String,
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
            pm: None,
            pm_error: None,
            collection: false,
            adhoc: None,
            ctl_note: String::new(),
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
        let d = stations::dial();
        &d[idx.min(d.len() - 1)]
    }

    pub fn current(&self) -> &'static Station {
        self.station(self.selected)
    }

    pub fn on_air(&self) -> Option<&'static Station> {
        self.playing.map(|i| self.station(i)).or(self.adhoc)
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
        let n = stations::dial().len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(n);
        self.selected = next as usize;
    }

    pub fn tune_selected(&mut self) {
        self.tune(self.selected);
    }

    pub fn tune(&mut self, idx: usize) {
        if idx >= stations::dial().len() {
            return;
        }
        self.adhoc = None;
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

    /// Play any http(s) stream that is not on the dial (agents do this a lot).
    pub fn tune_url(&mut self, url: &str) {
        let host = url
            .split("//")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or(url)
            .to_string();
        let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
        let st: &'static Station = Box::leak(Box::new(Station {
            id: "url",
            name: leak(&host),
            blurb: "stream by url",
            source: leak(&host),
            homepage: leak(url),
            url: leak(url),
            mood: crate::visual::Mood::Space,
            accent: (120, 170, 255),
        }));
        match self.player.play_url(url) {
            Ok(()) => {
                self.playing = None;
                self.adhoc = Some(st);
                self.status = format!("tuning {}…", st.name);
            }
            Err(err) => self.status = format!("tune failed: {err}"),
        }
    }

    /// After the dial changed shape, point `playing`/`selected` at the same
    /// stations by id rather than by the old positions.
    fn reindex(&mut self, playing_id: Option<&'static str>, selected_id: &'static str, was_on_air: Option<&'static Station>) {
        let d = stations::dial();
        self.playing = playing_id.and_then(|id| d.iter().position(|s| s.id == id));
        if self.playing.is_none() && playing_id.is_some() {
            // The station that is on air was removed; keep showing it truthfully.
            self.adhoc = was_on_air;
        }
        self.selected = d
            .iter()
            .position(|s| s.id == selected_id)
            .unwrap_or_else(|| self.selected.min(d.len().saturating_sub(1)));
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

    /// `m`: next built-in preset; switches the viz to milkdrop, and back to
    /// the built-in engine if the collection was showing.
    pub fn next_milk_preset(&mut self) {
        if self.viz != VizKind::Milk {
            self.set_viz(VizKind::Milk);
        }
        if self.collection {
            self.collection = false;
            if self.kitty {
                self.clear_kitty = true;
            }
            self.status = format!("milkdrop · {}", self.milk.preset_name());
            return;
        }
        let name = self.milk.next_preset();
        self.status = format!("milkdrop · {name}");
    }

    /// `M`: the MilkDrop collection through projectM. The first press brings
    /// the engine up and shows a preset; each press after that is the next one.
    pub fn next_collection_preset(&mut self) {
        if self.viz != VizKind::Milk {
            self.set_viz(VizKind::Milk);
        }
        if self.pm.is_none() {
            match ProjectM::new(crate::pm::W, crate::pm::H) {
                Ok(pm) => {
                    self.pm_error = None;
                    self.status = format!("milkdrop · {} · {} presets on {}", pm.name(), pm.count(), pm.renderer);
                    self.pm = Some(pm);
                    self.collection = true;
                    if self.kitty {
                        self.clear_kitty = true;
                    }
                }
                Err(e) => {
                    self.collection = false;
                    self.status = format!("collection unavailable · {e}");
                    self.pm_error = Some(e);
                }
            }
            return;
        }
        if !self.collection {
            self.collection = true;
            if self.kitty {
                self.clear_kitty = true;
            }
            let name = self.pm.as_ref().map(|p| p.name().to_string()).unwrap_or_default();
            self.status = format!("milkdrop · {name}");
            return;
        }
        let name = self.pm.as_mut().map(|p| p.next().to_string()).unwrap_or_default();
        self.status = format!("milkdrop · {name}");
    }

    pub fn milk_preset(&self) -> String {
        match (&self.pm, self.collection) {
            (Some(pm), true) => format!("✦ {}", pm.name()),
            _ => self.milk.preset_name().to_string(),
        }
    }

    pub fn viz_name(&self) -> &'static str {
        match self.viz {
            VizKind::Bars => "bars",
            VizKind::Wave => "wave",
            VizKind::Milk => "milkdrop",
            VizKind::Iss => "iss",
        }
    }

    fn viz_from(name: &str) -> Option<VizKind> {
        Some(match name.to_ascii_lowercase().as_str() {
            "bars" | "spectrum" => VizKind::Bars,
            "wave" | "scope" => VizKind::Wave,
            "milkdrop" | "milk" => VizKind::Milk,
            "iss" | "space" | "earth" => VizKind::Iss,
            _ => return None,
        })
    }

    /// One control request in, one JSON answer out. See AGENTS.md.
    pub fn apply(&mut self, req: Request) -> Value {
        let err = |m: String| json!({ "ok": false, "error": m });
        let dial_ids = |me: &Self| -> (Option<&'static str>, &'static str, Option<&'static Station>) {
            (me.playing.map(|i| me.station(i).id), me.station(me.selected).id, me.on_air())
        };
        match req {
            Request::Status => self.status_json(),
            Request::Stations => json!({
                "ok": true,
                "stations": self.stations_json(),
                "config": stations::config_path(),
            }),
            Request::Tune { station } => {
                if station.starts_with("http://") || station.starts_with("https://") {
                    self.tune_url(&station);
                } else if let Some(i) = stations::find(&station) {
                    self.tune(i);
                } else {
                    return err(format!("no station matches {station:?}; try `stations`"));
                }
                self.status_json()
            }
            Request::Play => {
                if self.on_air().is_none() {
                    self.tune_selected();
                } else if let Err(e) = self.player.set_paused(false) {
                    return err(e.to_string());
                }
                self.status_json()
            }
            Request::Pause => {
                if let Err(e) = self.player.set_paused(true) {
                    return err(e.to_string());
                }
                self.status_json()
            }
            Request::Toggle => {
                self.toggle_pause();
                self.status_json()
            }
            Request::Stop => {
                self.stop();
                self.adhoc = None;
                self.status_json()
            }
            Request::Next => {
                self.select_delta(1);
                self.tune_selected();
                self.status_json()
            }
            Request::Prev => {
                self.select_delta(-1);
                self.tune_selected();
                self.status_json()
            }
            Request::Volume { value, delta } => {
                let r = match (value, delta) {
                    (Some(v), _) => self.player.set_volume(v),
                    (None, Some(d)) => self.player.bump_volume(d),
                    _ => return err("volume needs `value` or `delta`".into()),
                };
                if let Err(e) = r {
                    return err(e.to_string());
                }
                self.status_json()
            }
            Request::Viz { kind } => match Self::viz_from(&kind) {
                Some(k) => {
                    self.set_viz(k);
                    self.status_json()
                }
                None => err(format!("unknown viz {kind:?}; use bars|wave|milkdrop|iss")),
            },
            Request::Preset { action } => {
                let a = action.unwrap_or_else(|| "next".into());
                match a.as_str() {
                    "next" => {
                        if self.collection {
                            self.next_collection_preset();
                        } else {
                            self.next_milk_preset();
                        }
                    }
                    "builtin" => {
                        if self.collection {
                            self.next_milk_preset();
                        } else if self.viz != VizKind::Milk {
                            self.set_viz(VizKind::Milk);
                        }
                    }
                    "collection" => self.next_collection_preset(),
                    name => {
                        if self.pm.is_none() {
                            self.next_collection_preset();
                        }
                        let hit = self.pm.as_mut().and_then(|p| p.find(name).map(|s| s.to_string()));
                        match hit {
                            Some(n) => {
                                self.collection = true;
                                if self.viz != VizKind::Milk {
                                    self.set_viz(VizKind::Milk);
                                }
                                self.status = format!("milkdrop · {n}");
                            }
                            None if self.pm.is_some() => {
                                return err(format!("no collection preset matching {name:?}"))
                            }
                            None => {}
                        }
                    }
                }
                if let (Some(e), true) = (&self.pm_error, a != "builtin") {
                    return err(e.clone());
                }
                self.status_json()
            }
            Request::Add { station, tune } => {
                let id = station.id.clone();
                let (pid, sid, on_air) = dial_ids(self);
                match stations::add(station) {
                    Ok(n) => {
                        self.reindex(pid, sid, on_air);
                        if tune {
                            if let Some(i) = stations::dial().iter().position(|s| s.id == id) {
                                self.tune(i);
                            }
                        }
                        let mut m = self.status_json();
                        m["stations"] = json!(n);
                        m["added"] = json!(id);
                        m
                    }
                    Err(e) => err(e),
                }
            }
            Request::Remove { id } => {
                let (pid, sid, on_air) = dial_ids(self);
                match stations::remove(&id) {
                    Ok(n) => {
                        self.reindex(pid, sid, on_air);
                        let mut m = self.status_json();
                        m["stations"] = json!(n);
                        m["removed"] = json!(id);
                        m
                    }
                    Err(e) => err(e),
                }
            }
            Request::Reload => {
                let (pid, sid, on_air) = dial_ids(self);
                match stations::reload() {
                    Ok(n) => {
                        self.reindex(pid, sid, on_air);
                        let mut m = self.status_json();
                        m["stations"] = json!(n);
                        m
                    }
                    Err(e) => err(e),
                }
            }
            Request::Quit => {
                self.should_quit = true;
                json!({ "ok": true, "quitting": true })
            }
        }
    }

    fn status_json(&self) -> Value {
        let station = self.on_air().map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "source": s.source,
                "url": s.url,
                "n": self.playing.map(|i| i + 1),
            })
        });
        let (sstate, sdetail) = self.space.status();
        json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": crate::ctl::PROTOCOL,
            "station": station,
            "selected": self.station(self.selected).id,
            "now_playing": self.player.now_playing(),
            "paused": self.player.paused,
            "idle": self.player.idle,
            "volume": self.player.volume,
            "viz": self.viz_name(),
            "fullscreen": self.fullscreen,
            "milkdrop": {
                "engine": if self.collection { "collection" } else { "builtin" },
                "preset": self.milk_preset(),
                "collection": self.pm.as_ref().map(|p| json!({
                    "presets": p.count(),
                    "index": p.index(),
                    "renderer": p.renderer,
                    "projectm": p.version,
                    "dir": p.preset_dir,
                })),
                "collection_error": self.pm_error,
            },
            "tap": {
                "sink": self.tap.sink_name(),
                "backend": self.tap.backend_name(),
                "rms": self.mix_rms,
                "peak_bar": self.peak_bar,
                "fft_ok": self.fft_ok,
                "error": self.tap.last_err(),
            },
            "iss": if self.viz == VizKind::Iss {
                json!({ "state": format!("{sstate:?}").to_lowercase(), "detail": sdetail, "source": self.space.source_name() })
            } else {
                Value::Null
            },
            "stations": stations::dial().len(),
            "config": stations::config_path(),
            "status": self.status,
        })
    }

    fn stations_json(&self) -> Value {
        json!(stations::dial()
            .iter()
            .enumerate()
            .map(|(i, s)| json!({
                "n": i + 1,
                "id": s.id,
                "name": s.name,
                "source": s.source,
                "blurb": s.blurb,
                "homepage": s.homepage,
                "url": s.url,
                "mood": stations::mood_name(s.mood),
                "playing": self.playing == Some(i),
            }))
            .collect::<Vec<_>>())
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

    /// The milkdrop frame to show: (top-down RGB, width, height).
    pub fn milk_frame(&mut self) -> (Vec<u8>, u32, u32) {
        if self.collection {
            if let Some(pm) = self.pm.as_mut() {
                return (pm.render().to_vec(), crate::pm::W, crate::pm::H);
            }
        }
        let accent = self.on_air().unwrap_or_else(|| self.current()).accent;
        let wave = self.analyzer.waveform().to_vec();
        let bands = self.spectrum.levels().to_vec();
        (self.milk.render_rgb(&wave, &bands, accent), crate::milk::PIX_W, crate::milk::PIX_H)
    }

    /// Non-Kitty terminals: the same frame as shade cells.
    pub fn milk_cells(&mut self, buf: &mut ratatui::buffer::Buffer, area: Rect) {
        let (rgb, w, h) = self.milk_frame();
        crate::milk::cells_from_rgb(buf, area, &rgb, w, h);
    }

    pub fn tick(&mut self, dt: f32) {
        self.player.poll();

        self.tap.set_want_audio(self.playing.is_some() && !self.player.paused);
        self.tap.drain(&mut self.pcm);
        if let Some(pm) = self.pm.as_mut() {
            pm.feed(&self.pcm);
        }
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
