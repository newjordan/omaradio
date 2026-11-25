//! The dial: built-in stations, or whatever lives in the user's config.
//!
//! `~/.config/omaradio/stations.json` (or `$OMARADIO_CONFIG_DIR/stations.json`)
//! replaces the built-ins entirely when present. Agents and scripts edit it
//! through `omaradio ctl add / remove`, which rewrites the file and reloads
//! the live dial.

use crate::visual::Mood;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub source: String,
    pub homepage: String,
    pub url: String,
    pub mood: Mood,
    pub accent: (u8, u8, u8),
}

impl Station {
    /// Stand-in used only when the dial is empty, so the UI never indexes off the end.
    pub fn unknown() -> Self {
        Self {
            id: "none".into(),
            name: "omaradio".into(),
            blurb: String::new(),
            source: String::new(),
            homepage: String::new(),
            url: String::new(),
            mood: Mood::Space,
            accent: (120, 170, 255),
        }
    }
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    blurb: &'static str,
    source: &'static str,
    homepage: &'static str,
    url: &'static str,
    mood: Mood,
    accent: (u8, u8, u8),
}

const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "otr",
        name: "Old Time Radio",
        blurb: "golden-age drama, mystery, and variety",
        source: "WALM Radio",
        homepage: "https://walmradio.com",
        url: "https://icecast.walmradio.com:8443/otr",
        mood: Mood::Voice,
        accent: (212, 165, 106),
    },
    Builtin {
        id: "liquid",
        name: "Liquid DnB",
        blurb: "rolling liquid funk, pads over breaks",
        source: "Liquid DnB",
        homepage: "https://antares.dribbcast.com/proxy/dave1/stream/",
        url: "https://antares.dribbcast.com/proxy/dave1/stream/",
        mood: Mood::Liquid,
        accent: (80, 220, 170),
    },
    Builtin {
        id: "space",
        name: "Space Station",
        blurb: "tune in, turn on, space out",
        source: "SomaFM",
        homepage: "https://somafm.com/spacestation/",
        url: "https://somafm.com/spacestation.pls",
        mood: Mood::Space,
        accent: (120, 170, 255),
    },
    Builtin {
        id: "mission",
        name: "Mission Control",
        blurb: "NASA comms over deep-space ambient",
        source: "SomaFM",
        homepage: "https://somafm.com/missioncontrol/",
        url: "https://somafm.com/missioncontrol.pls",
        mood: Mood::Mission,
        accent: (255, 170, 70),
    },
    Builtin {
        id: "jazz",
        name: "Jazz Groove",
        blurb: "after-midnight slow jazz, no chatter",
        source: "The Jazz Groove",
        homepage: "https://www.thejazzgroove.com",
        url: "http://east-mp3-128.streamthejazzgroove.com/stream",
        mood: Mood::Jazz,
        accent: (210, 140, 255),
    },
    Builtin {
        id: "blues",
        name: "Midnight Blues",
        blurb: "slow blues after hours",
        source: "Jazz Radio — Blues",
        homepage: "https://www.jazzradio.fr",
        url: "http://jazzblues.ice.infomaniak.ch/jazzblues-high.mp3",
        mood: Mood::Blues,
        accent: (220, 90, 70),
    },
    Builtin {
        id: "wwoz",
        name: "WWOZ New Orleans",
        blurb: "live from the Quarter, jazz and blues",
        source: "WWOZ 90.7 FM",
        homepage: "https://www.wwoz.org",
        url: "https://wwoz-sc.streamguys1.com/wwoz-hi.mp3",
        mood: Mood::Brass,
        accent: (255, 196, 80),
    },
    Builtin {
        id: "portland",
        name: "All Classical Portland",
        blurb: "Oregon's classical station, 24 hours, real hosts",
        source: "All Classical Radio",
        homepage: "https://www.allclassical.org",
        url: "https://allclassical.streamguys1.com/ac128kmp3",
        mood: Mood::Classical,
        accent: (200, 185, 255),
    },
    Builtin {
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

/// The stations shipped with the binary. A config file replaces this list.
pub fn builtin_stations() -> Vec<Station> {
    BUILTIN
        .iter()
        .map(|s| Station {
            id: s.id.into(),
            name: s.name.into(),
            blurb: s.blurb.into(),
            source: s.source.into(),
            homepage: s.homepage.into(),
            url: s.url.into(),
            mood: s.mood,
            accent: s.accent,
        })
        .collect()
}

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
    /// voice | liquid | space | mission | jazz | blues | brass | classical
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

static DIAL: RwLock<Option<Arc<Vec<Station>>>> = RwLock::new(None);

/// The live dial. Loads the config on first use; falls back to the built-ins.
///
/// The returned `Arc` is a snapshot. A later `reload` swaps in a new list and
/// drops the previous one once nothing else holds it.
pub fn dial() -> Arc<Vec<Station>> {
    if let Some(d) = DIAL.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return d;
    }
    let _ = reload();
    DIAL.read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(builtin_stations()))
}

pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OMARADIO_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("omaradio");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("omaradio")
}

pub fn config_path() -> PathBuf {
    config_dir().join("stations.json")
}

/// Re-read the config (or the built-ins when there is none) into the live dial.
pub fn reload() -> Result<usize, String> {
    let path = config_path();
    let list = if path.is_file() {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: StationsFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        validate(&file.stations)?;
        file.stations.iter().map(materialize).collect()
    } else {
        builtin_stations()
    };
    let n = list.len();
    *DIAL.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(list));
    Ok(n)
}

pub fn validate(list: &[StationSpec]) -> Result<(), String> {
    if list.is_empty() {
        return Err("stations list is empty".into());
    }
    let mut ids = std::collections::HashSet::new();
    for s in list {
        if s.id.trim().is_empty()
            || !s
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("station id {:?} must be [A-Za-z0-9_-]", s.id));
        }
        if !ids.insert(s.id.as_str()) {
            return Err(format!("duplicate station id {:?}", s.id));
        }
        if s.name.trim().is_empty() {
            return Err(format!("station {:?} has no name", s.id));
        }
        if !(s.url.starts_with("http://") || s.url.starts_with("https://")) {
            return Err(format!(
                "station {:?} url must be http(s): {:?}",
                s.id, s.url
            ));
        }
        if !s.mood.is_empty() && mood_from_str(&s.mood).is_none() {
            return Err(format!("station {:?} mood {:?} unknown", s.id, s.mood));
        }
    }
    Ok(())
}

fn materialize(s: &StationSpec) -> Station {
    let mood = mood_from_str(&s.mood).unwrap_or(Mood::Space);
    Station {
        id: s.id.clone(),
        name: s.name.clone(),
        blurb: s.blurb.clone(),
        source: if s.source.is_empty() {
            s.name.clone()
        } else {
            s.source.clone()
        },
        homepage: if s.homepage.is_empty() {
            s.url.clone()
        } else {
            s.homepage.clone()
        },
        url: s.url.clone(),
        mood,
        accent: s
            .accent
            .map(|a| (a[0], a[1], a[2]))
            .unwrap_or_else(|| accent_for(mood)),
    }
}

pub fn spec_of(s: &Station) -> StationSpec {
    StationSpec {
        id: s.id.clone(),
        name: s.name.clone(),
        url: s.url.clone(),
        blurb: s.blurb.clone(),
        source: s.source.clone(),
        homepage: s.homepage.clone(),
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
    let file = StationsFile {
        stations: list.to_vec(),
    };
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
    default_index_in(&dial())
}

pub fn default_index_in(d: &[Station]) -> usize {
    d.iter().position(|s| s.id == "mission").unwrap_or(0)
}

/// Resolve a user/agent reference against the live dial.
///
/// Precedence: exact id, then a 1-based dial number, then an exact name,
/// then a name fragment. An id like `"10"` wins over "the 10th station".
pub fn find(reference: &str) -> Option<usize> {
    find_in(&dial(), reference)
}

pub fn find_in(d: &[Station], reference: &str) -> Option<usize> {
    let r = reference.trim();
    if r.is_empty() {
        return None;
    }
    if let Some(i) = d.iter().position(|s| s.id.eq_ignore_ascii_case(r)) {
        return Some(i);
    }
    if let Ok(n) = r.parse::<usize>() {
        if (1..=d.len()).contains(&n) {
            return Some(n - 1);
        }
    }
    let folded = r.to_ascii_lowercase();
    d.iter()
        .position(|s| s.name.eq_ignore_ascii_case(&folded))
        .or_else(|| {
            d.iter()
                .position(|s| s.name.to_ascii_lowercase().contains(&folded))
        })
}

/// Host (and port, and userinfo) of an http(s) url, for labelling a url tune.
pub fn stream_host(url: &str) -> String {
    let rest = url.split("//").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn dial_has_the_requested_kinds() {
        let stations = builtin_stations();
        let ids: HashSet<_> = stations.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains("otr"));
        assert!(ids.contains("liquid"));
        assert!(ids.contains("space"));
        assert!(ids.contains("mission"));
        assert!(ids.contains("jazz") || ids.contains("blues"));
    }

    #[test]
    fn ids_and_urls_are_unique_and_nonempty() {
        let stations = builtin_stations();
        let mut ids = HashSet::new();
        let mut urls = HashSet::new();
        for s in &stations {
            assert!(!s.name.is_empty());
            assert!(!s.blurb.is_empty());
            assert!(!s.homepage.is_empty());
            assert!(s.homepage.starts_with("http"), "{}", s.homepage);
            assert!(s.url.starts_with("http"), "{}", s.url);
            assert!(ids.insert(s.id.as_str()), "duplicate id {}", s.id);
            assert!(urls.insert(s.url.as_str()), "duplicate url {}", s.url);
        }
        assert_eq!(stations.len(), 9);
    }

    #[test]
    fn builtins_roundtrip_through_the_config_format() {
        let stations = builtin_stations();
        let specs: Vec<StationSpec> = stations.iter().map(spec_of).collect();
        validate(&specs).unwrap();
        let text = serde_json::to_string(&StationsFile {
            stations: specs.clone(),
        })
        .unwrap();
        let back: StationsFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.stations, specs);
        let again: Vec<Station> = back.stations.iter().map(materialize).collect();
        for (a, b) in again.iter().zip(&stations) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.url, b.url);
            assert_eq!(a.mood, b.mood);
            assert_eq!(a.accent, b.accent);
        }
    }

    #[test]
    fn validate_rejects_bad_specs() {
        let ok = StationSpec {
            id: "x".into(),
            name: "X".into(),
            url: "https://x".into(),
            blurb: String::new(),
            source: String::new(),
            homepage: String::new(),
            mood: String::new(),
            accent: None,
        };
        assert!(validate(std::slice::from_ref(&ok)).is_ok());
        let mut bad = ok.clone();
        bad.url = "ftp://x".into();
        assert!(validate(&[bad]).is_err());
        let mut bad = ok.clone();
        bad.id = "has space".into();
        assert!(validate(&[bad]).is_err());
        let mut bad = ok.clone();
        bad.mood = "grunge".into();
        assert!(validate(&[bad]).is_err());
        assert!(validate(&[ok.clone(), ok]).is_err(), "duplicate ids");
        assert!(validate(&[]).is_err());
    }

    #[test]
    fn minimal_spec_gets_sane_defaults() {
        let s: StationSpec = serde_json::from_str(
            r#"{"id":"kexp","name":"KEXP","url":"https://kexp-mp3-128.streamguys1.com/kexp128.mp3"}"#,
        )
        .unwrap();
        let st = materialize(&s);
        assert_eq!(st.source, "KEXP");
        assert_eq!(st.homepage, st.url);
        assert_eq!(st.mood, Mood::Space);
    }

    #[test]
    fn find_accepts_id_number_and_name() {
        let d = builtin_stations();
        assert_eq!(
            find_in(&d, "mission").map(|i| d[i].id.as_str()),
            Some("mission")
        );
        assert_eq!(find_in(&d, "1").map(|i| d[i].id.as_str()), Some("otr"));
        assert_eq!(
            find_in(&d, "Jazz Groove").map(|i| d[i].id.as_str()),
            Some("jazz")
        );
        assert_eq!(find_in(&d, "wwoz").map(|i| d[i].id.as_str()), Some("wwoz"));
        assert_eq!(
            find_in(&d, "groove").map(|i| d[i].id.as_str()),
            Some("jazz")
        );
        assert_eq!(find_in(&d, "0"), None);
        assert_eq!(find_in(&d, "nope-nope"), None);
        assert_eq!(find_in(&d, "   "), None);
    }

    #[test]
    fn numeric_id_wins_over_dial_position() {
        let d = vec![
            Station {
                id: "10".into(),
                name: "Ten".into(),
                blurb: String::new(),
                source: String::new(),
                homepage: String::new(),
                url: "https://example.test/10".into(),
                mood: Mood::Space,
                accent: (1, 2, 3),
            },
            Station {
                id: "other".into(),
                name: "Other".into(),
                blurb: String::new(),
                source: String::new(),
                homepage: String::new(),
                url: "https://example.test/other".into(),
                mood: Mood::Jazz,
                accent: (4, 5, 6),
            },
        ];
        // "10" is an id, not "the 10th row" (there is no 10th row).
        assert_eq!(find_in(&d, "10").map(|i| d[i].id.as_str()), Some("10"));
        assert_eq!(find_in(&d, "2").map(|i| d[i].id.as_str()), Some("other"));
        assert_eq!(find_in(&d, "1").map(|i| d[i].id.as_str()), Some("10"));
    }

    #[test]
    fn default_tunes_mission_control() {
        let d = builtin_stations();
        assert_eq!(d[default_index_in(&d)].id, "mission");
    }

    #[test]
    fn stream_host_keeps_port_and_drops_the_path() {
        assert_eq!(
            stream_host("https://example.test:8443/otr"),
            "example.test:8443"
        );
        assert_eq!(stream_host("http://east.example/stream"), "east.example");
    }
}
