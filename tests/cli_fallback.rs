//! `slowrx-cli` flags exercised against synthesized WAVs:
//! - `--fallback-mode`: parses mode names (case-insensitively, with a
//!   helpful error for unknown ones) and is applied to the decoder, so a
//!   transmission whose VIS bits were lost decodes only when it is set
//!   (a double-leader header with the bits replaced by noise, then a
//!   Robot 36 image).
//! - `--drop-noise`: an "image" decoded from noise after a VIS (#116) is
//!   written by default but skipped with the flag.

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
    write_wav(path, &audio);
}

/// Write `audio` (working rate, [-1, 1]) as a 16-bit mono WAV.
fn write_wav(path: &Path, audio: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WORKING_SAMPLE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in audio {
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

/// A clean Robot 36 VIS followed by 40 s of noise: the decoder produces a
/// full-size image of noise.
fn write_vis_then_noise_wav(path: &Path) {
    let mut audio = slowrx::__test_support::vis::synth_vis(0x08, 0.0);
    let mut seed = 0x5EED_u32;
    let n = 40 * WORKING_SAMPLE_RATE_HZ as usize;
    audio.extend((0..n).map(|_| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        #[allow(clippy::cast_possible_truncation)]
        let v = (f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0) as f32;
        v
    }));
    write_wav(path, &audio);
}

#[test]
fn drop_noise_skips_an_image_decoded_from_noise() {
    let dir = scratch("noise");
    let wav = dir.join("noise.wav");
    write_vis_then_noise_wav(&wav);
    // Default: the noise image is still written (no behaviour change).
    let kept = dir.join("kept");
    cli()
        .args(["--quiet", "--input"])
        .arg(&wav)
        .arg("--output")
        .arg(&kept)
        .assert()
        .success();
    assert!(kept.join("img-001-robot36.png").is_file());
    // --drop-noise: nothing written, so the run reports no images.
    let dropped = dir.join("dropped");
    let assert = cli()
        .args(["--drop-noise", "--input"])
        .arg(&wav)
        .arg("--output")
        .arg(&dropped)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("looks like noise"), "stderr: {stderr}");
    assert!(!dropped.join("img-001-robot36.png").exists());
    let _ = fs::remove_dir_all(&dir);
}
