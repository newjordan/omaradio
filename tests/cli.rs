//! The `omaradio` binary's argument handling, without a terminal or mpv.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_omaradio"))
}

#[test]
fn version_and_help_exit_zero() {
    let version = bin().arg("--version").output().unwrap();
    assert!(version.status.success(), "{version:?}");
    let text = String::from_utf8(version.stdout).unwrap();
    assert!(
        text.contains(env!("CARGO_PKG_VERSION")),
        "stdout was {text:?}"
    );

    let help = bin().arg("--help").output().unwrap();
    assert!(help.status.success(), "{help:?}");
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("omaradio ctl"));
    assert!(text.contains("--prove-tap"));

    let ctl = bin().args(["ctl", "help"]).output().unwrap();
    assert!(
        ctl.status.success(),
        "ctl help stderr={}",
        String::from_utf8_lossy(&ctl.stderr)
    );
    let text = String::from_utf8(ctl.stdout).unwrap();
    assert!(text.contains("volume"));
    assert!(text.contains("stations.json"));
    assert!(
        !String::from_utf8_lossy(&ctl.stderr).contains("omaradio:"),
        "help should not be reported as an error"
    );
}

#[test]
fn unknown_argument_and_missing_radio_fail() {
    let bad = bin().arg("--not-a-flag").output().unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown argument"));

    let sock = std::env::temp_dir().join(format!("omaradio-missing-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let status = bin()
        .env("OMARADIO_CTL_SOCKET", &sock)
        .args(["ctl", "status"])
        .output()
        .unwrap();
    assert!(!status.status.success());
    let err = String::from_utf8_lossy(&status.stderr);
    assert!(err.contains("no omaradio listening"), "stderr was {err}");
}
