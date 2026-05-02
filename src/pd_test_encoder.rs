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
use std::f64::consts::PI;

const SYNC_HZ: f64 = 1200.0;
const PORCH_HZ: f64 = 1500.0;
const BLACK_HZ: f64 = 1500.0;
const WHITE_HZ: f64 = 2300.0;

fn lum_to_freq(lum: u8) -> f64 {
    BLACK_HZ + (WHITE_HZ - BLACK_HZ) * f64::from(lum) / 255.0
}

/// Encoder helper: emit samples up to sample index `target_n` (exclusive)
/// at frequency `freq_hz`, advancing both the running sample count `n`
/// and the running phase. Using cumulative sample targets prevents the
/// per-tone rounding error that would otherwise compound over the line
/// (640 pixels × ~0.094 sample/pixel rounding error per pixel adds up to
/// 60+ samples per line at PD120, breaking decoder line alignment).
fn fill_to(out: &mut Vec<f32>, freq_hz: f64, target_n: usize, phase: &mut f64) {
    let dphi = 2.0 * PI * freq_hz / f64::from(WORKING_SAMPLE_RATE_HZ);
    while out.len() < target_n {
        out.push(phase.sin() as f32);
        *phase += dphi;
        if *phase > 2.0 * PI {
            *phase -= 2.0 * PI;
        }
    }
}

/// Encode an image as PD120 / PD180 audio. `ycrcb` is row-major
/// `[Y, Cr, Cb]` triples of length `width * height`. Pairs of rows
/// share averaged chroma, matching how the decoder will recover them.
#[must_use]
#[doc(hidden)]
#[allow(dead_code)]
pub fn encode_pd(mode: SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
    assert!(matches!(mode, SstvMode::Pd120 | SstvMode::Pd180));
    let spec = crate::modespec::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    assert_eq!(ycrcb.len() as u32, w * h);
    assert_eq!(h % 2, 0);

    let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
    let mut out = Vec::new();
    let mut phase = 0.0_f64;

    // Cumulative time tracker (seconds). Targets are computed as
    // `(running_t * sr).round()` so per-event rounding doesn't drift.
    let mut t = 0.0_f64;
    let advance = |t: &mut f64, secs: f64| -> usize {
        *t += secs;
        (*t * sr).round() as usize
    };

    for y_pair in 0..h / 2 {
        fill_to(
            &mut out,
            SYNC_HZ,
            advance(&mut t, spec.sync_seconds),
            &mut phase,
        );
        fill_to(
            &mut out,
            PORCH_HZ,
            advance(&mut t, spec.porch_seconds),
            &mut phase,
        );

        // Y(odd row).
        for x in 0..w {
            let lum = ycrcb[((y_pair * 2) * w + x) as usize][0];
            fill_to(
                &mut out,
                lum_to_freq(lum),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }
        // Cr (averaged across pair).
        for x in 0..w {
            let cr_a = ycrcb[((y_pair * 2) * w + x) as usize][1];
            let cr_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][1];
            let cr = u8::midpoint(cr_a, cr_b);
            fill_to(
                &mut out,
                lum_to_freq(cr),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }
        // Cb (averaged).
        for x in 0..w {
            let cb_a = ycrcb[((y_pair * 2) * w + x) as usize][2];
            let cb_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][2];
            let cb = u8::midpoint(cb_a, cb_b);
            fill_to(
                &mut out,
                lum_to_freq(cb),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }
        // Y(even row).
        for x in 0..w {
            let lum = ycrcb[((y_pair * 2 + 1) * w + x) as usize][0];
            fill_to(
                &mut out,
                lum_to_freq(lum),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }
    }
    out
}
