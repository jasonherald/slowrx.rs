//! `slowrx-cli --fallback-mode`: the flag parses mode names (case-
//! insensitively, with a helpful error for unknown ones) and is applied to
//! the decoder, so a transmission whose VIS bits were lost decodes only
//! when it is set. The WAV is synthesized: a double-leader header with the
//! bits replaced by noise, then a Robot 36 image.

#![cfg(all(feature = "cli", feature = "test-support"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use slowrx::{SstvMode, WORKING_SAMPLE_RATE_HZ};

/// Unique scratch directory per test (tests run in parallel).
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("slowrx-cli-fallback-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write the lost-VIS Robot 36 transmission as a 16-bit mono WAV.
fn write_lost_vis_wav(path: &Path) {
    let (mut audio, _) = slowrx::__test_support::vis::lost_bits_header(610.0, true, 0.0);
    let ycrcb = slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36);
    audio.extend(slowrx::__test_support::mode_robot::encode_robot(
        SstvMode::Robot36,
        &ycrcb,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WORKING_SAMPLE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in audio {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

fn cli() -> Command {
    Command::cargo_bin("slowrx-cli").expect("binary built")
}

#[test]
fn fallback_mode_recovers_a_lost_vis_image() {
    let dir = scratch("on");
    let wav = dir.join("lost.wav");
    write_lost_vis_wav(&wav);
    let out = dir.join("out");
    // Mode names are case-insensitive.
    cli()
        .args(["--quiet", "--fallback-mode", "ROBOT36", "--input"])
        .arg(&wav)
        .arg("--output")
        .arg(&out)
        .assert()
        .success();
    assert!(out.join("img-001-robot36.png").is_file(), "image written");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_a_fallback_the_lost_vis_image_is_not_decoded() {
    let dir = scratch("off");
    let wav = dir.join("lost.wav");
    write_lost_vis_wav(&wav);
    let out = dir.join("out");
    let assert = cli()
        .args(["--quiet", "--input"])
        .arg(&wav)
        .arg("--output")
        .arg(&out)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("no SSTV images"), "stderr: {stderr}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_mode_name_is_rejected_with_the_valid_names() {
    let assert = cli()
        .args([
            "--input",
            "x.wav",
            "--output",
            "out",
            "--fallback-mode",
            "robot99",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("unknown mode 'robot99'") && stderr.contains("robot36"),
        "stderr: {stderr}"
    );
}
