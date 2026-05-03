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
            encode_r36_or_r24(&mut out, &mut phase, &mut t, advance, &spec, ycrcb);
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

        // Pad to spec.line_seconds boundary. R72's per-line content
        // (sync + porch + 3 channels + 2 septr) sums to ~297.4 ms but
        // ModeSpec.line_seconds is 300 ms — the decoder advances at
        // 300 ms per line. Without this pad, the synthetic audio drifts
        // from the crate's own timing model and weakens the round-trip
        // as a regression test. Fill the gap with PORCH_HZ (real radio
        // emits a 1500 Hz tone during inter-line idle, not silence).
        let line_end_target = f64::from(y + 1) * spec.line_seconds;
        let pad_secs = line_end_target - *t;
        if pad_secs > 0.0 {
            fill_to(out, PORCH_HZ, advance(t, pad_secs), phase);
        }
    }
}

/// Robot 36 / Robot 24 channel layout per slowrx `video.c:60-70` (R36/R24 case):
///   `ChanLen[0]` = `pixel_seconds` * width * 2   (Y allocated 2× per-channel time)
///   `ChanLen[1]` = `ChanLen[2]` = `pixel_seconds` * width   (chroma at 1×)
///   `ChanStart[0]` = sync + porch
///   `ChanStart[1]` = `ChanStart[0]` + `ChanLen[0]` + septr
///   `ChanStart[2]` = `ChanStart[1]`   (chroma channel time slot reused — actual
///                                  channel determined by row parity)
///
/// Per radio line N, we emit:
///   - Sync at `SYNC_HZ`
///   - Porch at `PORCH_HZ`
///   - Y for image row N at `pixel_seconds * 2` per pixel (so total Y
///     duration = `pixel_seconds` * 2 * width = `ChanLen[0]`)
///   - Septr at `PORCH_HZ`
///   - Chroma for image row N at `pixel_seconds` per pixel: Cr if `N%2==0`,
///     Cb if `N%2==1`
#[allow(clippy::too_many_arguments)]
fn encode_r36_or_r24(
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

        // Y channel — emit at `pixel_seconds * 2` per source pixel so
        // the total Y allocation equals ChanLen[0] = pixel_seconds *
        // width * 2 per slowrx video.c:60-70 (R36/R24 case).
        for x in 0..w {
            let lum = ycrcb[(y * w + x) as usize][0];
            fill_to(
                out,
                lum_to_freq(lum),
                advance(t, spec.pixel_seconds * 2.0),
                phase,
            );
        }

        // Septr between Y and chroma.
        fill_to(out, PORCH_HZ, advance(t, spec.septr_seconds), phase);

        // Chroma — Cr (ycrcb index 1) on even rows, Cb (ycrcb index 2)
        // on odd. One sample per `pixel_seconds`.
        let chroma_idx = if y % 2 == 0 { 1_usize } else { 2_usize };
        for x in 0..w {
            let chroma = ycrcb[(y * w + x) as usize][chroma_idx];
            fill_to(
                out,
                lum_to_freq(chroma),
                advance(t, spec.pixel_seconds),
                phase,
            );
        }
    }
}
