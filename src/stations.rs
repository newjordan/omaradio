//! Night dial — the stations this radio actually lives on.

use crate::visual::Mood;

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

pub const DIAL: &[Station] = &[
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
];

pub fn default_index() -> usize {
    DIAL.iter()
        .position(|s| s.id == "space")
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn dial_has_the_requested_kinds() {
        let ids: HashSet<_> = DIAL.iter().map(|s| s.id).collect();
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
        for s in DIAL {
            assert!(!s.name.is_empty());
            assert!(!s.blurb.is_empty());
            assert!(!s.homepage.is_empty());
            assert!(s.homepage.starts_with("http"), "{}", s.homepage);
            assert!(s.url.starts_with("http"), "{}", s.url);
            assert!(ids.insert(s.id), "duplicate id {}", s.id);
            assert!(urls.insert(s.url), "duplicate url {}", s.url);
        }
        assert!(DIAL.len() >= 5);
        assert!(DIAL.len() <= 9);
    }

    #[test]
    fn default_tunes_space_station() {
        assert_eq!(DIAL[default_index()].id, "space");
    }
}
