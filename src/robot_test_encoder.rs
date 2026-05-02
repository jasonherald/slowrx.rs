//! Synthetic Robot encoder for round-trip testing. Produces continuous-
//! phase FM audio matching the encoder side of the SSTV protocol.
//!
//! Test-only — gated behind `cfg(any(test, feature = "test-support"))`.
//! Lives in its own file so the production decoder stays under the
//! 500-LOC ceiling.
//!
//! **R36/R24 round-trip constraint:** the source ycrcb buffer must have
//! adjacent rows share chroma (`ycrcb[2k][1] == ycrcb[2k+1][1]` for Cr;
//! `ycrcb[2k+1][2] == ycrcb[2k+2][2]` for Cb), because the decoder
//! duplicates each chroma sample to the neighbor row (slowrx
//! `video.c:424-425`). Source images that violate this constraint cannot
//! round-trip losslessly. R72 has no such constraint (full per-line
//! chroma).

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

/// Emit samples up to absolute sample index `target_n`, advancing phase.
/// Cumulative-target pattern (matches `pd_test_encoder::fill_to`) so
/// per-tone rounding doesn't compound across the line.
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

/// Encode an image as Robot 24 / 36 / 72 audio. `ycrcb` is row-major
/// `[Y, Cr, Cb]` triples of length `width * height`.
///
/// R72: emits Y / U / V sequentially per radio line, with septr between.
/// R36/R24: emits Y / (Cr if y%2==0 else Cb) per radio line, with septr
/// between Y and chroma. The decoder duplicates the chroma sample to
/// the neighbor row, so the source ycrcb buffer must have adjacent rows
/// share chroma (see file-level doc).
#[must_use]
#[doc(hidden)]
#[allow(dead_code)]
pub fn encode_robot(mode: SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
    assert!(matches!(
        mode,
        SstvMode::Robot24 | SstvMode::Robot36 | SstvMode::Robot72
    ));
    let spec = crate::modespec::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    assert_eq!(ycrcb.len() as u32, w * h);

    let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
    let mut out = Vec::new();
    let mut phase = 0.0_f64;

    let mut t = 0.0_f64;
    let advance = |t: &mut f64, secs: f64| -> usize {
        *t += secs;
        (*t * sr).round() as usize
    };

    match mode {
        SstvMode::Robot72 => encode_r72(&mut out, &mut phase, &mut t, advance, &spec, ycrcb),
        SstvMode::Robot24 | SstvMode::Robot36 => {
            // Filled in V2.2 Phase 4 (R36/R24 path).
            unimplemented!("R36/R24 encoder lands in V2.2 Phase 4")
        }
        _ => unreachable!(),
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn encode_r72(
    out: &mut Vec<f32>,
    phase: &mut f64,
    t: &mut f64,
    mut advance: impl FnMut(&mut f64, f64) -> usize,
    spec: &crate::modespec::ModeSpec,
    ycrcb: &[[u8; 3]],
) {
    let w = spec.line_pixels;
    let h = spec.image_lines;
    for y in 0..h {
        // Sync + porch.
        fill_to(out, SYNC_HZ, advance(t, spec.sync_seconds), phase);
        fill_to(out, PORCH_HZ, advance(t, spec.porch_seconds), phase);

        // Y channel.
        for x in 0..w {
            let lum = ycrcb[(y * w + x) as usize][0];
            fill_to(out, lum_to_freq(lum), advance(t, spec.pixel_seconds), phase);
        }

        // Septr between Y and U (Cr).
        fill_to(out, PORCH_HZ, advance(t, spec.septr_seconds), phase);

        // U (Cr) channel.
        for x in 0..w {
            let cr = ycrcb[(y * w + x) as usize][1];
            fill_to(out, lum_to_freq(cr), advance(t, spec.pixel_seconds), phase);
        }

        // Septr between U and V.
        fill_to(out, PORCH_HZ, advance(t, spec.septr_seconds), phase);

        // V (Cb) channel.
        for x in 0..w {
            let cb = ycrcb[(y * w + x) as usize][2];
            fill_to(out, lum_to_freq(cb), advance(t, spec.pixel_seconds), phase);
        }
    }
}
