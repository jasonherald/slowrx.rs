//! Starting an image from its line-sync rhythm when no VIS was heard
//! (#115), and forcing every image into one mode (#113).
//!
//! With a fallback mode ([`SstvDecoder::set_fallback_mode`]) or a forced
//! mode ([`SstvDecoder::set_forced_mode`]) set, the decoder recognises that
//! mode's line-sync pulses directly and starts decoding at the first synced
//! line, emitting [`SstvEvent::SyncStart`]. That recovers images whose whole
//! header faded and images the receiver joined mid-transmission (AOS during
//! a transmission). A header, when heard, still wins.

#![cfg(feature = "test-support")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode, WORKING_SAMPLE_RATE_HZ};

const RATE: usize = WORKING_SAMPLE_RATE_HZ as usize;
const ROBOT36_VIS: u8 = 0x08;
const ROBOT72_VIS: u8 = 0x0C;
/// A well-formed 7-bit VIS code that maps to no mode.
const UNKNOWN_VIS: u8 = 0x01;

/// `ms` of white noise — an FM channel with no signal.
fn noise(ms: usize, seed: u32) -> Vec<f32> {
    slowrx::__test_support::noise::white(ms * RATE / 1000, seed)
}

/// The shared synthetic Robot 36 image and its audio (no header).
fn robot36() -> (Vec<[u8; 3]>, Vec<f32>) {
    let ycrcb = slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36);
    let audio = slowrx::__test_support::mode_robot::encode_robot(SstvMode::Robot36, &ycrcb);
    (ycrcb, audio)
}

fn line_samples() -> usize {
    (slowrx::for_mode(SstvMode::Robot36).line_seconds * RATE as f64).round() as usize
}

/// Run `audio` (plus decoder-threshold padding) through a decoder
/// configured by `setup`.
fn decode(mut audio: Vec<f32>, setup: impl FnOnce(&mut SstvDecoder)) -> Vec<SstvEvent> {
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    setup(&mut d);
    d.process(&audio)
}

fn count(events: &[SstvEvent], pred: impl Fn(&SstvEvent) -> bool) -> usize {
    events.iter().filter(|e| pred(e)).count()
}

fn images(events: &[SstvEvent]) -> Vec<&SstvImage> {
    events
        .iter()
        .filter_map(|e| match e {
            SstvEvent::ImageComplete { image, .. } => Some(image),
            _ => None,
        })
        .collect()
}

/// Mean absolute RGB error of decoded `rows` against source rows offset by
/// `src_offset`.
fn mean_error(
    img: &SstvImage,
    ycrcb: &[[u8; 3]],
    rows: std::ops::Range<usize>,
    src_offset: usize,
) -> f64 {
    let w = img.width as usize;
    let (mut sum, mut n) = (0_u64, 0_u64);
    for row in rows {
        for x in 0..w {
            let src = ycrcb[(row + src_offset) * w + x];
            let rgb = slowrx::__test_support::mode_pd::ycbcr_to_rgb(src[0], src[1], src[2]);
            for (want, got) in rgb.iter().zip(img.pixels[row * w + x]) {
                sum += u64::from(want.abs_diff(got));
                n += 1;
            }
        }
    }
    sum as f64 / n as f64
}

fn is_sync_start(e: &SstvEvent) -> bool {
    matches!(
        e,
        SstvEvent::SyncStart {
            mode: SstvMode::Robot36,
            ..
        }
    )
}

#[test]
fn a_headerless_image_decodes_in_the_fallback_mode() {
    let (ycrcb, image) = robot36();
    let mut audio = noise(1000, 7);
    audio.extend(&image);
    let events = decode(audio, |d| d.set_fallback_mode(Some(SstvMode::Robot36)));
    assert_eq!(count(&events, is_sync_start), 1, "{:?}", events.len());
    assert_eq!(
        count(&events, |e| matches!(
            e,
            SstvEvent::VisDetected { .. } | SstvEvent::VisFallback { .. }
        )),
        0
    );
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    let err = mean_error(imgs[0], &ycrcb, 1..240, 0);
    assert!(err < 5.0, "headerless image mean error {err:.2}");
}

#[test]
fn without_a_fallback_or_forced_mode_a_headerless_image_decodes_nothing() {
    let (_, image) = robot36();
    let mut audio = noise(1000, 7);
    audio.extend(&image);
    assert!(decode(audio, |_| {}).is_empty(), "sync start is opt-in");
}

#[test]
fn an_intact_header_is_not_duplicated_by_the_sync_start() {
    let (ycrcb, image) = robot36();
    let mut audio = noise(500, 3);
    audio.extend(slowrx::__test_support::vis::synth_vis(ROBOT36_VIS, 0.0));
    audio.extend(&image);
    let events = decode(audio, |d| d.set_fallback_mode(Some(SstvMode::Robot36)));
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::VisDetected { .. })),
        1
    );
    assert_eq!(count(&events, is_sync_start), 0, "the header wins");
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    assert!(mean_error(imgs[0], &ycrcb, 1..240, 0) < 5.0);
}

#[test]
fn a_mid_image_join_on_an_odd_line_decodes_the_rest_with_the_right_chroma() {
    // AOS during a transmission: the receiver hears radio line 101 onward.
    // The decoder backs off to line 100 (R36 picks Cr/Cb by row parity),
    // so decoded row r is source row 100 + r. Row 0 is the noise before
    // AOS and row 1 borrows its Cr, so compare from row 2.
    let (ycrcb, image) = robot36();
    let mut audio = noise(500, 9);
    audio.extend(&image[101 * line_samples()..]);
    // The receiver keeps listening after the transmission ends; the decoder
    // completes once a full image's worth of audio has arrived.
    audio.extend(noise(16_000, 10));
    let events = decode(audio, |d| d.set_fallback_mode(Some(SstvMode::Robot36)));
    assert_eq!(count(&events, is_sync_start), 1);
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    let err = mean_error(imgs[0], &ycrcb, 2..139, 100);
    assert!(err < 5.0, "joined image mean error {err:.2}");
}

#[test]
fn back_to_back_headerless_images_decode_as_two() {
    let (ycrcb, image) = robot36();
    let mut audio = noise(1000, 5);
    audio.extend(&image);
    audio.extend(noise(3000, 6));
    audio.extend(&image);
    let events = decode(audio, |d| d.set_fallback_mode(Some(SstvMode::Robot36)));
    assert_eq!(count(&events, is_sync_start), 2);
    let imgs = images(&events);
    assert_eq!(imgs.len(), 2);
    for img in imgs {
        assert!(mean_error(img, &ycrcb, 1..240, 0) < 5.0);
    }
}

#[test]
fn a_forced_mode_decodes_a_header_for_another_mode_as_the_forced_mode() {
    let (ycrcb, image) = robot36();
    let mut audio = slowrx::__test_support::vis::synth_vis(ROBOT72_VIS, 0.2);
    audio.extend(&image);
    let events = decode(audio, |d| d.set_forced_mode(Some(SstvMode::Robot36)));
    assert_eq!(
        count(&events, |e| matches!(
            e,
            SstvEvent::VisDetected {
                mode: SstvMode::Robot36,
                ..
            }
        )),
        1,
        "the header marks the start; its mode is overridden"
    );
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].mode, SstvMode::Robot36);
    assert!(mean_error(imgs[0], &ycrcb, 1..240, 0) < 5.0);
}

#[test]
fn a_forced_mode_decodes_an_unknown_vis_code_instead_of_dropping_it() {
    let (ycrcb, image) = robot36();
    let mut audio = slowrx::__test_support::vis::synth_vis(UNKNOWN_VIS, 0.2);
    audio.extend(&image);
    let events = decode(audio, |d| d.set_forced_mode(Some(SstvMode::Robot36)));
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::UnknownVis { .. })),
        0
    );
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    assert!(mean_error(imgs[0], &ycrcb, 1..240, 0) < 5.0);
}

#[test]
fn a_forced_mode_starts_a_headerless_image_from_its_sync_rhythm() {
    let (ycrcb, image) = robot36();
    let mut audio = noise(1000, 11);
    audio.extend(&image);
    let events = decode(audio, |d| d.set_forced_mode(Some(SstvMode::Robot36)));
    assert_eq!(count(&events, is_sync_start), 1);
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    assert!(mean_error(imgs[0], &ycrcb, 1..240, 0) < 5.0);
}

#[test]
fn a_forced_mode_takes_precedence_over_the_fallback() {
    // Fallback PD120, forced Robot 36: both the sync-rhythm start and the
    // lost-VIS fallback must look for, and decode in, the forced mode.
    let (ycrcb, image) = robot36();
    let both = |d: &mut SstvDecoder| {
        d.set_fallback_mode(Some(SstvMode::Pd120));
        d.set_forced_mode(Some(SstvMode::Robot36));
    };

    let mut headerless = noise(1000, 17);
    headerless.extend(&image);
    let events = decode(headerless, both);
    assert_eq!(count(&events, is_sync_start), 1, "sync start in R36");
    let imgs = images(&events);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].mode, SstvMode::Robot36);
    assert!(mean_error(imgs[0], &ycrcb, 1..240, 0) < 5.0);

    let (mut lost_bits, _) = slowrx::__test_support::vis::lost_bits_header(610.0, true, 0.0);
    lost_bits.extend(&image);
    let events = decode(lost_bits, both);
    assert_eq!(
        count(&events, |e| matches!(
            e,
            SstvEvent::VisFallback {
                mode: SstvMode::Robot36,
                ..
            }
        )),
        1,
        "lost-VIS fallback assumes the forced mode"
    );
    assert_eq!(images(&events)[0].mode, SstvMode::Robot36);
}

#[test]
fn forced_mode_round_trips_through_the_accessor_and_survives_reset() {
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    assert_eq!(d.forced_mode(), None, "off by default");
    d.set_forced_mode(Some(SstvMode::Pd120));
    assert_eq!(d.forced_mode(), Some(SstvMode::Pd120));
    d.reset();
    assert_eq!(d.forced_mode(), Some(SstvMode::Pd120), "reset keeps it");
    d.set_forced_mode(None);
    assert_eq!(d.forced_mode(), None);
}
