//! Synthetic PD encoder for round-trip testing. Produces continuous-phase
//! FM audio matching the encoder side of the SSTV protocol.
//!
//! Test-only — gated behind `cfg(any(test, feature = "test-support"))`. Lives
//! in its own file (rather than inside [`crate::mode_pd`]) so the
//! production decoder stays under the 500-LOC ceiling.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::modespec::SstvMode;
use crate::resample::WORKING_SAMPLE_RATE_HZ;
use crate::test_tone::{lum_to_freq, ToneWriter, PORCH_HZ, SYNC_HZ};

/// Encode an image as PD-family audio (PD120 / PD180 / PD240). `ycrcb`
/// is row-major `[Y, Cr, Cb]` triples of length `width * height`. Pairs
/// of rows share averaged chroma, matching how the decoder will recover
/// them.
#[must_use]
#[doc(hidden)]
#[allow(dead_code)]
pub fn encode_pd(mode: SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
    assert!(matches!(
        mode,
        SstvMode::Pd120 | SstvMode::Pd180 | SstvMode::Pd240
    ));
    let spec = crate::modespec::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    assert_eq!(ycrcb.len() as u32, w * h);
    assert_eq!(h % 2, 0);

    let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
    let mut tone = ToneWriter::new();

    // Cumulative time tracker (seconds). Targets are computed as
    // `(running_t * sr).round()` so per-event rounding doesn't drift.
    let mut t = 0.0_f64;
    let advance = |t: &mut f64, secs: f64| -> usize {
        *t += secs;
        (*t * sr).round() as usize
    };

    for y_pair in 0..h / 2 {
        tone.fill_to(SYNC_HZ, advance(&mut t, spec.sync_seconds));
        tone.fill_to(PORCH_HZ, advance(&mut t, spec.porch_seconds));

        // Y(odd row).
        for x in 0..w {
            let lum = ycrcb[((y_pair * 2) * w + x) as usize][0];
            tone.fill_to(lum_to_freq(lum), advance(&mut t, spec.pixel_seconds));
        }
        // Cr (averaged across pair).
        for x in 0..w {
            let cr_a = ycrcb[((y_pair * 2) * w + x) as usize][1];
            let cr_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][1];
            let cr = u8::midpoint(cr_a, cr_b);
            tone.fill_to(lum_to_freq(cr), advance(&mut t, spec.pixel_seconds));
        }
        // Cb (averaged).
        for x in 0..w {
            let cb_a = ycrcb[((y_pair * 2) * w + x) as usize][2];
            let cb_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][2];
            let cb = u8::midpoint(cb_a, cb_b);
            tone.fill_to(lum_to_freq(cb), advance(&mut t, spec.pixel_seconds));
        }
        // Y(even row).
        for x in 0..w {
            let lum = ycrcb[((y_pair * 2 + 1) * w + x) as usize][0];
            tone.fill_to(lum_to_freq(lum), advance(&mut t, spec.pixel_seconds));
        }
    }
    tone.into_vec()
}
