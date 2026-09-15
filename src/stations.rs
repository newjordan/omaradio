//! The dial: built-in stations, or whatever lives in the user's config.
//!
//! `~/.config/omaradio/stations.json` (or `$OMARADIO_CONFIG_DIR/stations.json`)
//! replaces the built-ins entirely when present. Agents and scripts edit it
//! through `omaradio ctl add / remove`, which rewrites the file and reloads
//! the live dial.

use crate::visual::Mood;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Station {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub source: &'static str,
    pub homepage: &'static str,
    pub url: &'static str,
    pub mood: Mood,
    pub accent: (u8, u8, u8),
}

pub const BUILTIN: &[Station] = &[
    Station {
        id: "otr",
        name: "Old Time Radio",
        blurb: "golden-age drama, mystery, and variety",
        source: "WALM Radio",
        homepage: "https://walmradio.com",
        url: "https://icecast.walmradio.com:8443/otr",
        mood: Mood::Voice,
        accent: (212, 165, 106),
    },
    Station {
        id: "liquid",
        name: "Liquid DnB",
        blurb: "rolling liquid funk, pads over breaks",
        source: "Liquid DnB",
        homepage: "https://antares.dribbcast.com/proxy/dave1/stream/",
        url: "https://antares.dribbcast.com/proxy/dave1/stream/",
        mood: Mood::Liquid,
        accent: (80, 220, 170),
    },
    Station {
        id: "space",
        name: "Space Station",
        blurb: "tune in, turn on, space out",
        source: "SomaFM",
        homepage: "https://somafm.com/spacestation/",
        url: "https://somafm.com/spacestation.pls",
        mood: Mood::Space,
        accent: (120, 170, 255),
    },
    Station {
        id: "mission",
        name: "Mission Control",
        blurb: "NASA comms over deep-space ambient",
        source: "SomaFM",
        homepage: "https://somafm.com/missioncontrol/",
        url: "https://somafm.com/missioncontrol.pls",
        mood: Mood::Mission,
        accent: (255, 170, 70),
    },
    Station {
        id: "jazz",
        name: "Jazz Groove",
        blurb: "after-midnight slow jazz, no chatter",
        source: "The Jazz Groove",
        homepage: "https://www.thejazzgroove.com",
        url: "http://east-mp3-128.streamthejazzgroove.com/stream",
        mood: Mood::Jazz,
        accent: (210, 140, 255),
    },
    Station {
        id: "blues",
        name: "Midnight Blues",
        blurb: "slow blues after hours",
        source: "Jazz Radio — Blues",
        homepage: "https://www.jazzradio.fr",
        url: "http://jazzblues.ice.infomaniak.ch/jazzblues-high.mp3",
        mood: Mood::Blues,
        accent: (220, 90, 70),
    },
    Station {
        id: "wwoz",
        name: "WWOZ New Orleans",
        blurb: "live from the Quarter, jazz and blues",
        source: "WWOZ 90.7 FM",
        homepage: "https://www.wwoz.org",
        url: "https://wwoz-sc.streamguys1.com/wwoz-hi.mp3",
        mood: Mood::Brass,
        accent: (255, 196, 80),
    },
    Station {
        id: "portland",
        name: "All Classical Portland",
        blurb: "Oregon's classical station, 24 hours, real hosts",
        source: "All Classical Radio",
        homepage: "https://www.allclassical.org",
        url: "https://allclassical.streamguys1.com/ac128kmp3",
        mood: Mood::Classical,
        accent: (200, 185, 255),
    },
    Station {
        id: "swiss",
        name: "Radio Swiss Classic",
        blurb: "slow, peaceful classical, no ads, no talk",
        source: "SRG SSR",
        homepage: "https://www.radioswissclassic.ch",
        url: "https://stream.srg-ssr.ch/m/rsc_de/mp3_128",
        mood: Mood::Classical,
        accent: (170, 200, 255),
    },
];


/// One station as it appears in `stations.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationSpec {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub blurb: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub homepage: String,
    /// voice | liquid | space | mission | jazz | blues | brass
    #[serde(default)]
    pub mood: String,
    /// Accent colour for the visualizers; defaults from the mood.
    #[serde(default)]
    pub accent: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StationsFile {
    pub stations: Vec<StationSpec>,
}

static DIAL: RwLock<Option<&'static [Station]>> = RwLock::new(None);

/// The live dial. Loads the config on first use; falls back to the built-ins.
pub fn dial() -> &'static [Station] {
    if let Some(d) = *DIAL.read().unwrap_or_else(|e| e.into_inner()) {
        return d;
    }
    let _ = reload();
    DIAL.read()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(BUILTIN)
}

pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OMARADIO_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("omaradio");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("omaradio")
}

pub fn config_path() -> PathBuf {
    config_dir().join("stations.json")
}

/// Re-read the config (or the built-ins when there is none) into the live dial.
pub fn reload() -> Result<usize, String> {
    let path = config_path();
    let list: &'static [Station] = if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: StationsFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        validate(&file.stations)?;
        Box::leak(file.stations.iter().map(materialize).collect::<Vec<_>>().into_boxed_slice())
    } else {
        BUILTIN
    };
    *DIAL.write().unwrap_or_else(|e| e.into_inner()) = Some(list);
    Ok(list.len())
}

pub fn validate(list: &[StationSpec]) -> Result<(), String> {
    if list.is_empty() {
        return Err("stations list is empty".into());
    }
    let mut ids = std::collections::HashSet::new();
    for s in list {
        if s.id.trim().is_empty() || !s.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("station id {:?} must be [A-Za-z0-9_-]", s.id));
        }
        if !ids.insert(s.id.as_str()) {
            return Err(format!("duplicate station id {:?}", s.id));
        }
        if s.name.trim().is_empty() {
            return Err(format!("station {:?} has no name", s.id));
        }
        if !(s.url.starts_with("http://") || s.url.starts_with("https://")) {
            return Err(format!("station {:?} url must be http(s): {:?}", s.id, s.url));
        }
        if !s.mood.is_empty() && mood_from_str(&s.mood).is_none() {
            return Err(format!("station {:?} mood {:?} unknown", s.id, s.mood));
        }
    }
    Ok(())
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn materialize(s: &StationSpec) -> Station {
    let mood = mood_from_str(&s.mood).unwrap_or(Mood::Space);
    Station {
        id: leak(&s.id),
        name: leak(&s.name),
        blurb: leak(&s.blurb),
        source: leak(if s.source.is_empty() { &s.name } else { &s.source }),
        homepage: leak(if s.homepage.is_empty() { &s.url } else { &s.homepage }),
        url: leak(&s.url),
        mood,
        accent: s.accent.map(|a| (a[0], a[1], a[2])).unwrap_or_else(|| accent_for(mood)),
    }
}

pub fn spec_of(s: &Station) -> StationSpec {
    StationSpec {
        id: s.id.into(),
        name: s.name.into(),
        url: s.url.into(),
        blurb: s.blurb.into(),
        source: s.source.into(),
        homepage: s.homepage.into(),
        mood: mood_name(s.mood).into(),
        accent: Some([s.accent.0, s.accent.1, s.accent.2]),
    }
}

/// Current dial as config specs (built-ins included, so the first `add`
/// writes a complete file).
pub fn specs() -> Vec<StationSpec> {
    dial().iter().map(spec_of).collect()
}

/// Write the list to the config and reload the live dial.
pub fn save(list: &[StationSpec]) -> Result<usize, String> {
    validate(list)?;
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = StationsFile { stations: list.to_vec() };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    reload()
}

pub fn add(spec: StationSpec) -> Result<usize, String> {
    let mut list = specs();
    if let Some(existing) = list.iter_mut().find(|s| s.id == spec.id) {
        *existing = spec;
    } else {
        list.push(spec);
    }
    save(&list)
}

pub fn remove(id: &str) -> Result<usize, String> {
    let mut list = specs();
    let before = list.len();
    list.retain(|s| s.id != id);
    if list.len() == before {
        return Err(format!("no station with id {id:?}"));
    }
    save(&list)
}

pub fn mood_from_str(s: &str) -> Option<Mood> {
    Some(match s.to_ascii_lowercase().as_str() {
        "voice" | "talk" | "otr" => Mood::Voice,
        "liquid" | "dnb" => Mood::Liquid,
        "space" | "ambient" => Mood::Space,
        "mission" => Mood::Mission,
        "jazz" => Mood::Jazz,
        "blues" => Mood::Blues,
        "brass" | "funk" | "soul" => Mood::Brass,
        "classical" | "orchestral" | "chamber" => Mood::Classical,
        _ => return None,
    })
}

pub fn mood_name(m: Mood) -> &'static str {
    match m {
        Mood::Voice => "voice",
        Mood::Liquid => "liquid",
        Mood::Space => "space",
        Mood::Mission => "mission",
        Mood::Jazz => "jazz",
        Mood::Blues => "blues",
        Mood::Brass => "brass",
        Mood::Classical => "classical",
    }
}

fn accent_for(m: Mood) -> (u8, u8, u8) {
    match m {
        Mood::Voice => (212, 165, 106),
        Mood::Liquid => (80, 220, 170),
        Mood::Space => (120, 170, 255),
        Mood::Mission => (255, 170, 70),
        Mood::Jazz => (240, 200, 120),
        Mood::Blues => (110, 140, 230),
        Mood::Brass => (255, 140, 90),
        Mood::Classical => (200, 185, 255),
    }
}

/// Index to tune when nothing else was asked for: Mission Control if it is
/// on the dial, else the first station.
pub fn default_index() -> usize {
    dial().iter().position(|s| s.id == "mission").unwrap_or(0)
}

/// Resolve a user/agent reference: station id, 1-based dial number, or name.
pub fn find(reference: &str) -> Option<usize> {
    let d = dial();
    if let Ok(n) = reference.parse::<usize>() {
        return (n >= 1 && n <= d.len()).then(|| n - 1);
    }
    let r = reference.to_ascii_lowercase();
    d.iter()
        .position(|s| s.id.eq_ignore_ascii_case(&r))
        .or_else(|| d.iter().position(|s| s.name.to_ascii_lowercase() == r))
        .or_else(|| d.iter().position(|s| s.name.to_ascii_lowercase().contains(&r)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn dial_has_the_requested_kinds() {
        let ids: HashSet<_> = BUILTIN.iter().map(|s| s.id).collect();
        assert!(ids.contains("otr"));
        assert!(ids.contains("liquid"));
        assert!(ids.contains("space"));
        assert!(ids.contains("mission"));
        assert!(ids.contains("jazz") || ids.contains("blues"));
    }

    #[test]
    fn ids_and_urls_are_unique_and_nonempty() {
        let mut ids = HashSet::new();
        let mut urls = HashSet::new();
        for s in BUILTIN {
            assert!(!s.name.is_empty());
            assert!(!s.blurb.is_empty());
            assert!(!s.homepage.is_empty());
            assert!(s.homepage.starts_with("http"), "{}", s.homepage);
            assert!(s.url.starts_with("http"), "{}", s.url);
            assert!(ids.insert(s.id), "duplicate id {}", s.id);
            assert!(urls.insert(s.url), "duplicate url {}", s.url);
        }
        assert!(BUILTIN.len() >= 5);
        assert!(BUILTIN.len() <= 9);
    }

    #[test]
    fn builtins_roundtrip_through_the_config_format() {
        let specs: Vec<StationSpec> = BUILTIN.iter().map(spec_of).collect();
        validate(&specs).unwrap();
        let text = serde_json::to_string(&StationsFile { stations: specs.clone() }).unwrap();
        let back: StationsFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.stations, specs);
        let again: Vec<Station> = back.stations.iter().map(materialize).collect();
        for (a, b) in again.iter().zip(BUILTIN) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.url, b.url);
            assert_eq!(a.mood, b.mood);
            assert_eq!(a.accent, b.accent);
        }
    }

    #[test]
    fn validate_rejects_bad_specs() {
        let ok = StationSpec { id: "x".into(), name: "X".into(), url: "https://x".into(), blurb: String::new(), source: String::new(), homepage: String::new(), mood: String::new(), accent: None };
        assert!(validate(&[ok.clone()]).is_ok());
        let mut bad = ok.clone(); bad.url = "ftp://x".into();
        assert!(validate(&[bad]).is_err());
        let mut bad = ok.clone(); bad.id = "has space".into();
        assert!(validate(&[bad]).is_err());
        let mut bad = ok.clone(); bad.mood = "grunge".into();
        assert!(validate(&[bad]).is_err());
        assert!(validate(&[ok.clone(), ok]).is_err(), "duplicate ids");
        assert!(validate(&[]).is_err());
    }

    #[test]
    fn minimal_spec_gets_sane_defaults() {
        let s: StationSpec = serde_json::from_str(r#"{"id":"kexp","name":"KEXP","url":"https://kexp-mp3-128.streamguys1.com/kexp128.mp3"}"#).unwrap();
        let st = materialize(&s);
        assert_eq!(st.source, "KEXP");
        assert_eq!(st.homepage, st.url);
        assert_eq!(st.mood, Mood::Space);
    }

    #[test]
    fn find_accepts_id_number_and_name() {
        assert_eq!(find("mission").map(|i| dial()[i].id), Some("mission"));
        assert_eq!(find("1").map(|i| dial()[i].id), Some("otr"));
        assert_eq!(find("Jazz Groove").map(|i| dial()[i].id), Some("jazz"));
        assert_eq!(find("wwoz").map(|i| dial()[i].id), Some("wwoz"));
        assert_eq!(find("0"), None);
        assert_eq!(find("nope-nope"), None);
    }

    #[test]
    fn default_tunes_mission_control() {
        assert_eq!(dial()[default_index()].id, "mission");
    }
}
