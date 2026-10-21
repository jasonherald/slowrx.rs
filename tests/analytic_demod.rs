//! Opt-in analytic-signal demodulation (#71): [`SstvDecoder::set_analytic_demod`].
//!
//! With a short pixel window a real signal's FFT peak is pulled by its own
//! negative-frequency mirror by an amount that depends on the tone's phase,
//! which draws a diagonal cross-hatch in dark areas. Demodulating the
//! analytic signal removes the mirror. It's off by default (slowrx
//! demodulates the real signal); these tests pin the setting and check that
//! decoding with it on stays exact on synthetic images.

#![cfg(feature = "test-support")]
#![allow(clippy::expect_used, clippy::cast_precision_loss)]

use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode, WORKING_SAMPLE_RATE_HZ};

const ROBOT36_VIS: u8 = 0x08;

#[test]
fn analytic_demod_is_off_by_default_and_survives_reset() {
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    assert!(!d.analytic_demod(), "slowrx's real-signal demod by default");
    d.set_analytic_demod(true);
    assert!(d.analytic_demod());
    d.reset();
    assert!(d.analytic_demod(), "reset keeps configuration");
    d.set_analytic_demod(false);
    assert!(!d.analytic_demod());
}

/// Mean absolute RGB error of `img` against the source `Y'CbCr` image,
/// skipping the first column: slowrx's `(x - 0.5)` pixel centre samples it
/// half a pixel before the channel starts.
fn mean_error(img: &SstvImage, ycrcb: &[[u8; 3]]) -> f64 {
    let w = img.width as usize;
    let (mut sum, mut n) = (0_u64, 0_u64);
    for (i, src) in ycrcb.iter().enumerate().filter(|(i, _)| i % w != 0) {
        let rgb = slowrx::__test_support::mode_pd::ycbcr_to_rgb(src[0], src[1], src[2]);
        for (want, got) in rgb.iter().zip(img.pixels[i]) {
            sum += u64::from(want.abs_diff(got));
            n += 1;
        }
    }
    sum as f64 / n as f64
}

fn decode_robot36(analytic: bool) -> f64 {
    let ycrcb = slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36);
    let mut audio = slowrx::__test_support::vis::synth_vis(ROBOT36_VIS, 0.0);
    audio.extend(slowrx::__test_support::mode_robot::encode_robot(
        SstvMode::Robot36,
        &ycrcb,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    d.set_analytic_demod(analytic);
    let image = d
        .process(&audio)
        .into_iter()
        .find_map(|e| match e {
            SstvEvent::ImageComplete { image, .. } => Some(image),
            _ => None,
        })
        .expect("image");
    mean_error(&image, &ycrcb)
}

#[test]
fn a_robot36_round_trip_with_analytic_demod_is_as_exact_as_without() {
    let real = decode_robot36(false);
    let analytic = decode_robot36(true);
    assert!(
        analytic < 5.0,
        "analytic round-trip mean error {analytic:.2}"
    );
    assert!(
        analytic <= real + 0.5,
        "analytic {analytic:.2} vs real {real:.2}"
    );
}

/// The setting is latched when an image starts: toggling it while an image
/// is being received leaves that image on the setting it started with, and
/// the next image picks up the new one.
#[test]
fn toggling_mid_image_applies_from_the_next_image() {
    let ycrcb = slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36);
    let mut one = slowrx::__test_support::vis::synth_vis(ROBOT36_VIS, 0.0);
    one.extend(slowrx::__test_support::mode_robot::encode_robot(
        SstvMode::Robot36,
        &ycrcb,
    ));
    let mut audio = one.clone();
    audio.extend(std::iter::repeat_n(0.0_f32, 11_025));
    audio.extend(&one);
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));

    let images = |toggle_at: Option<usize>| -> Vec<SstvImage> {
        let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
        let split = toggle_at.unwrap_or(audio.len());
        let mut events = d.process(&audio[..split]);
        d.set_analytic_demod(true);
        events.extend(d.process(&audio[split..]));
        events
            .into_iter()
            .filter_map(|e| match e {
                SstvEvent::ImageComplete { image, .. } => Some(image),
                _ => None,
            })
            .collect()
    };
    // Real demod throughout, and analytic throughout, for reference.
    let real = images(None);
    let analytic = images(Some(0));
    // Toggle halfway through the first image.
    let toggled = images(Some(one.len() / 2));
    assert_eq!(toggled.len(), 2);
    assert_eq!(toggled[0].pixels, real[0].pixels, "first image stays real");
    assert_eq!(
        toggled[1].pixels, analytic[1].pixels,
        "second image is analytic"
    );
    assert_ne!(real[0].pixels, analytic[0].pixels, "the setting matters");
}
