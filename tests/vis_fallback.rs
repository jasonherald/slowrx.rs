//! Fallback-mode decoding when a VIS header's bits are lost (opt-in via
//! [`SstvDecoder::set_fallback_mode`]).
//!
//! **Origin.** An ISS ARISS pass on 2026-10-06 carried a clean 37 s Robot 36
//! image (~17 dB) whose VIS header was hit by a ~0.5 s fade: the first
//! leader, the 10 ms break and the start of the second leader came through,
//! then noise covered the rest of the leader, the start bit, all eight VIS
//! bits and the stop bit. With no VIS the decoder never started, and the
//! whole image was lost. These tests rebuild that shape synthetically (the
//! capture itself can't be committed — see `tests/no_vis.rs` for the
//! fixture policy).
//!
//! A standard VIS header is `leader 300 ms (1900 Hz) · break 10 ms (1200 Hz)
//! · leader 300 ms · start bit 30 ms (1200 Hz) · 8 bits × 30 ms
//! (1100 / 1300 Hz) · stop bit 30 ms (1200 Hz)`; image audio follows the
//! stop bit. (`__test_support::vis::synth_vis` emits only the single
//! trailing leader the VIS matcher needs, so it can't exercise this path.)

#![cfg(feature = "test-support")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, WORKING_SAMPLE_RATE_HZ};

const ROBOT36_VIS: u8 = 0x08;
const LEADER_HZ: f64 = 1900.0;
const BREAK_HZ: f64 = 1200.0;
const BIT_ONE_HZ: f64 = 1100.0;
const BIT_ZERO_HZ: f64 = 1300.0;

/// Continuous-phase tone writer at the working sample rate (a phase jump at
/// a tone boundary would smear the detector's FFT peaks).
struct Tones {
    out: Vec<f32>,
    phase: f64,
}

impl Tones {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            phase: 0.0,
        }
    }

    fn tone(&mut self, hz: f64, ms: f64) -> &mut Self {
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let n = (ms * sr / 1000.0).round() as usize;
        for _ in 0..n {
            self.phase += 2.0 * std::f64::consts::PI * hz / sr;
            self.out.push(self.phase.sin() as f32);
        }
        self
    }

    /// Deterministic white noise (LCG) at full scale — an FM fade.
    fn noise(&mut self, ms: f64, seed: u32) -> &mut Self {
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let n = (ms * sr / 1000.0).round() as usize;
        let mut s = seed;
        for _ in 0..n {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.out
                .push((f64::from(s) / f64::from(u32::MAX) * 2.0 - 1.0) as f32);
        }
        self
    }

    fn len(&self) -> usize {
        self.out.len()
    }
}

/// Start bit, 7 data bits LSB-first, even parity, stop bit.
fn vis_bits(t: &mut Tones, code: u8) {
    t.tone(BREAK_HZ, 30.0);
    let mut parity = 0u8;
    for b in 0..7 {
        let bit = (code >> b) & 1;
        parity ^= bit;
        t.tone(if bit == 1 { BIT_ONE_HZ } else { BIT_ZERO_HZ }, 30.0);
    }
    t.tone(if parity == 1 { BIT_ONE_HZ } else { BIT_ZERO_HZ }, 30.0);
    t.tone(BREAK_HZ, 30.0);
}

/// The shared synthetic Robot 36 test image (`Y'CbCr`).
fn robot36_image() -> Vec<[u8; 3]> {
    slowrx::__test_support::mode_robot::synthetic_ycrcb(SstvMode::Robot36)
}

/// Append the Robot 36 image audio plus decoder-threshold padding.
fn push_image(t: &mut Tones, ycrcb: &[[u8; 3]]) {
    t.out
        .extend(slowrx::__test_support::mode_robot::encode_robot(
            SstvMode::Robot36,
            ycrcb,
        ));
    t.out.extend(std::iter::repeat_n(0.0_f32, 8192));
}

/// The 2026-10-06 shape: leader, break, 160 ms of the second leader, then
/// noise over the rest of the header; the image follows on time.
fn lost_bits_transmission(ycrcb: &[[u8; 3]]) -> Vec<f32> {
    let mut t = Tones::new();
    t.noise(200.0, 7);
    t.tone(LEADER_HZ, 300.0)
        .tone(BREAK_HZ, 10.0)
        .tone(LEADER_HZ, 160.0);
    t.noise(140.0 + 300.0, 11); // rest of leader 2 + start/8 bits/stop
    push_image(&mut t, ycrcb);
    t.out
}

/// Mean absolute RGB error of `img` against the source `Y'CbCr` image.
fn mean_error(img: &slowrx::SstvImage, ycrcb: &[[u8; 3]]) -> f64 {
    let mut sum = 0_u64;
    for (i, src) in ycrcb.iter().enumerate() {
        let rgb = slowrx::__test_support::mode_pd::ycbcr_to_rgb(src[0], src[1], src[2]);
        for (want, got) in rgb.iter().zip(img.pixels[i]) {
            sum += u64::from((i32::from(*want) - i32::from(got)).unsigned_abs());
        }
    }
    sum as f64 / (ycrcb.len() * 3) as f64
}

fn count(events: &[SstvEvent], pred: impl Fn(&SstvEvent) -> bool) -> usize {
    events.iter().filter(|e| pred(e)).count()
}

fn completed_image(events: &[SstvEvent]) -> Option<slowrx::SstvImage> {
    events.iter().find_map(|e| match e {
        SstvEvent::ImageComplete { image, .. } => Some(image.clone()),
        _ => None,
    })
}

#[test]
fn lost_vis_bits_decode_in_the_fallback_mode() {
    let ycrcb = robot36_image();
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    d.set_fallback_mode(Some(SstvMode::Robot36));
    let events = d.process(&lost_bits_transmission(&ycrcb));

    assert_eq!(
        count(&events, |e| matches!(
            e,
            SstvEvent::VisFallback {
                mode: SstvMode::Robot36,
                ..
            }
        )),
        1,
        "one VisFallback for the lost header"
    );
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::VisDetected { .. })),
        0
    );
    let img = completed_image(&events).expect("image decoded via fallback");
    assert_eq!(img.mode, SstvMode::Robot36);
    let err = mean_error(&img, &ycrcb);
    assert!(err < 5.0, "fallback image mean error {err:.2}");
}

#[test]
fn lost_vis_bits_without_a_fallback_decode_nothing() {
    let ycrcb = robot36_image();
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    let events = d.process(&lost_bits_transmission(&ycrcb));
    assert!(completed_image(&events).is_none(), "fallback is opt-in");
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::VisFallback { .. })),
        0
    );
}

#[test]
fn fallback_without_a_detectable_break_aligns_on_the_leader_start() {
    // Break lost too: one 610 ms leader run, then the bits are noise.
    let ycrcb = robot36_image();
    let mut t = Tones::new();
    t.noise(200.0, 3);
    t.tone(LEADER_HZ, 610.0);
    t.noise(300.0, 5);
    push_image(&mut t, &ycrcb);
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    d.set_fallback_mode(Some(SstvMode::Robot36));
    let events = d.process(&t.out);
    let img = completed_image(&events).expect("image decoded via fallback");
    let err = mean_error(&img, &ycrcb);
    assert!(err < 5.0, "fallback image mean error {err:.2}");
}

#[test]
fn an_intact_header_still_reports_vis_detected_with_a_fallback_set() {
    let ycrcb = robot36_image();
    let mut t = Tones::new();
    t.noise(200.0, 13);
    t.tone(LEADER_HZ, 300.0)
        .tone(BREAK_HZ, 10.0)
        .tone(LEADER_HZ, 300.0);
    vis_bits(&mut t, ROBOT36_VIS);
    push_image(&mut t, &ycrcb);
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    d.set_fallback_mode(Some(SstvMode::Robot36));
    let events = d.process(&t.out);
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::VisDetected { .. })),
        1
    );
    assert_eq!(
        count(&events, |e| matches!(e, SstvEvent::VisFallback { .. })),
        0,
        "the fallback must not fire when the real VIS decodes"
    );
    let img = completed_image(&events).expect("image");
    assert!(mean_error(&img, &ycrcb) < 5.0);
}

#[test]
fn noise_and_stray_tones_never_trigger_the_fallback() {
    let cases: Vec<(&str, Vec<f32>)> = vec![
        ("white noise", Tones::new().noise(10_000.0, 17).out.clone()),
        ("steady 1900 Hz", {
            let mut t = Tones::new();
            t.tone(LEADER_HZ, 3_000.0).noise(1_000.0, 19);
            t.out
        }),
        ("short 1900 Hz burst", {
            let mut t = Tones::new();
            t.noise(500.0, 23).tone(LEADER_HZ, 200.0).noise(2_000.0, 29);
            t.out
        }),
    ];
    for (name, audio) in cases {
        let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
        d.set_fallback_mode(Some(SstvMode::Robot36));
        let events = d.process(&audio);
        assert!(
            events.is_empty(),
            "{name}: expected no events, got {events:?}"
        );
    }
}

#[test]
fn fallback_mode_round_trips_through_the_accessor() {
    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    assert_eq!(d.fallback_mode(), None, "off by default");
    d.set_fallback_mode(Some(SstvMode::Pd120));
    assert_eq!(d.fallback_mode(), Some(SstvMode::Pd120));
    d.reset();
    assert_eq!(
        d.fallback_mode(),
        Some(SstvMode::Pd120),
        "reset keeps configuration"
    );
    d.set_fallback_mode(None);
    assert_eq!(d.fallback_mode(), None);
}

/// The synthetic image length check keeps `Tones::len` exercised.
#[test]
fn lost_bits_transmission_has_the_expected_header_length() {
    let mut t = Tones::new();
    t.tone(LEADER_HZ, 300.0)
        .tone(BREAK_HZ, 10.0)
        .tone(LEADER_HZ, 300.0);
    vis_bits(&mut t, ROBOT36_VIS);
    let expected = (910.0 * f64::from(WORKING_SAMPLE_RATE_HZ) / 1000.0).round() as usize;
    assert!(
        t.len().abs_diff(expected) <= 10,
        "header {} vs {expected}",
        t.len()
    );
}
