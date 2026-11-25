//! Config-file dial: add, replace, remove, reload, and lookup.
//!
//! Runs in its own process so `OMARADIO_CONFIG_DIR` cannot leak into the
//! unit tests.

use omaradio::stations::{self, StationSpec};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "omaradio-dial-{}-{}-{}",
        std::process::id(),
        nanos,
        n
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // This process only runs the tests in this file. The one test below owns
    // the variable for the whole sequence.
    std::env::set_var("OMARADIO_CONFIG_DIR", &dir);
    dir
}

fn spec(id: &str, name: &str, url: &str) -> StationSpec {
    StationSpec {
        id: id.into(),
        name: name.into(),
        url: url.into(),
        blurb: String::new(),
        source: String::new(),
        homepage: String::new(),
        mood: "jazz".into(),
        accent: None,
    }
}

#[test]
fn config_file_add_replace_remove_and_reload() {
    let dir = scratch();
    let builtin = stations::reload().unwrap();
    assert_eq!(builtin, stations::builtin_stations().len());
    assert!(!stations::config_path().exists());

    let n = stations::add(spec(
        "kexp",
        "KEXP",
        "https://kexp-mp3-128.streamguys1.com/kexp128.mp3",
    ))
    .unwrap();
    assert_eq!(n, builtin + 1);
    assert!(stations::config_path().is_file());
    assert_eq!(
        stations::find("kexp").map(|i| stations::dial()[i].name.clone()),
        Some("KEXP".into())
    );

    // Same id replaces, it does not grow the dial.
    let n = stations::add(spec(
        "kexp",
        "KEXP FM",
        "https://kexp-mp3-128.streamguys1.com/kexp128.mp3",
    ))
    .unwrap();
    assert_eq!(n, builtin + 1);
    assert_eq!(
        stations::dial()
            .iter()
            .find(|s| s.id == "kexp")
            .map(|s| s.name.clone()),
        Some("KEXP FM".into())
    );

    // A numeric id is reachable even when it is not a dial position.
    stations::add(spec("10", "Ten", "https://example.test/ten")).unwrap();
    assert_eq!(
        stations::find("10").map(|i| stations::dial()[i].id.clone()),
        Some("10".into())
    );

    let text = fs::read_to_string(stations::config_path()).unwrap();
    assert!(text.contains("\"id\": \"kexp\""), "{text}");

    // A hand edit is picked up by reload.
    let mut edited = text.replace("KEXP FM", "KEXP Seattle");
    edited.push('\n');
    fs::write(stations::config_path(), edited).unwrap();
    stations::reload().unwrap();
    assert_eq!(
        stations::dial()
            .iter()
            .find(|s| s.id == "kexp")
            .map(|s| s.name.clone()),
        Some("KEXP Seattle".into())
    );

    let after = stations::remove("kexp").unwrap();
    assert_eq!(after, builtin + 1);
    assert!(stations::find("kexp").is_none());
    assert!(stations::remove("kexp").is_err());

    fs::write(stations::config_path(), "{not json").unwrap();
    let err = stations::reload().unwrap_err();
    assert!(err.contains("stations.json"), "{err}");
    // A failed reload leaves the previous dial in place.
    assert!(stations::find("10").is_some());

    let _ = fs::remove_dir_all(&dir);
}
