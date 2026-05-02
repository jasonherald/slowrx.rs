//! Integration test for the `slowrx-cli` binary.
//!
//! Skips silently if no ARISS fixture is available locally — `docs/wav_files/`
//! is gitignored. CI does not run this; it's for local validation that the
//! installed binary works end-to-end against real audio.

#![cfg(feature = "cli")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;

fn first_aris_fixture() -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR is the slowrx.rs root.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("docs/wav_files/201712-ISS_SSTV");
    if !dir.is_dir() {
        return None;
    }
    fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "wav"))
}

#[test]
fn cli_decodes_aris_fixture_to_png() {
    let Some(wav) = first_aris_fixture() else {
        eprintln!("skipping: no ARISS fixture in docs/wav_files/201712-ISS_SSTV/");
        return;
    };

    let out_dir = std::env::temp_dir().join(format!("slowrx-cli-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out_dir);

    Command::cargo_bin("slowrx-cli")
        .expect("binary built")
        .arg("--input")
        .arg(&wav)
        .arg("--output")
        .arg(&out_dir)
        .arg("--quiet")
        .assert()
        .success();

    let pngs: Vec<_> = fs::read_dir(&out_dir)
        .expect("output dir exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .collect();
    assert!(
        !pngs.is_empty(),
        "expected at least one PNG written to {}",
        out_dir.display()
    );

    let _ = fs::remove_dir_all(&out_dir);
}
