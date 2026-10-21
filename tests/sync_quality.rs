//! `ImageComplete::sync_quality` separates real images from "images" that
//! are just noise (#116). On weak ISS passes a VIS can be followed by noise
//! — the transmission fades or ends, or a VIS matched by chance in noise —
//! and the decoder dutifully decodes ~36 s of noise into a junk image.
//! The quality score lets callers drop those.
//!
//! Real-data reference (local; fixtures not committed): nine 2026-10 ISS
//! passes scored 0.43-0.95 for every real image and 0.03-0.10 for the
//! three junk ones; the 12 ARISS Fram2 slides all score ~0.955.

#![cfg(feature = "test-support")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, SyncQuality, WORKING_SAMPLE_RATE_HZ};

const ROBOT36_VIS: u8 = 0x08;

fn first_completion(audio: &[f32]) -> SyncQuality {
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    d.process(audio)
        .into_iter()
        .find_map(|e| match e {
            SstvEvent::ImageComplete { sync_quality, .. } => Some(sync_quality),
            _ => None,
        })
        .expect("an image completes")
}

#[test]
fn a_real_image_does_not_look_like_noise() {
    let ycrcb = slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36);
    let mut audio = slowrx::__test_support::vis::synth_vis(ROBOT36_VIS, 0.0);
    audio.extend(slowrx::__test_support::mode_robot::encode_robot(
        SstvMode::Robot36,
        &ycrcb,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    let q = first_completion(&audio);
    assert!(q.value() > 0.9, "clean image quality {}", q.value());
    assert!(!q.looks_like_noise());
}

#[test]
fn a_vis_followed_by_noise_looks_like_noise() {
    let mut audio = slowrx::__test_support::vis::synth_vis(ROBOT36_VIS, 0.0);
    audio.extend(slowrx::__test_support::noise::white(
        40 * WORKING_SAMPLE_RATE_HZ as usize,
        0x5EED,
    ));
    let q = first_completion(&audio);
    assert!(q.value() < 0.1, "noise image quality {}", q.value());
    assert!(q.looks_like_noise());
}

// Real-data bounds from the module doc: junk <= 0.10, legible >= 0.43.
// The threshold must sit strictly between them (checked at compile time).
const _: () = assert!(SyncQuality::NOISE_THRESHOLD > 0.10 && SyncQuality::NOISE_THRESHOLD < 0.43);
