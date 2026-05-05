//! Synthetic Scottie encoder for round-trip testing. Produces
//! continuous-phase FM audio matching the encoder side of the SSTV
//! protocol for Scottie 1, Scottie 2, and Scottie DX.
//!
//! Test-only — gated behind `cfg(any(test, feature = "test-support"))`.
//! Lives in its own file so the production decoder stays under the
//! 500-LOC ceiling.
//!
//! **Line layout per radio line:**
//!
//! ```text
//! [septr 1500 Hz][G pixels 1500-2300 Hz][septr 1500 Hz]
//! [B pixels 1500-2300 Hz][SYNC 1200 Hz][porch 1500 Hz]
//! [R pixels 1500-2300 Hz]
//! ```
//!
//! Total per line = 2·SeptrTime + 2·ImgWidth·PixelTime + SyncTime +
//! PorchTime + ImgWidth·PixelTime = LineTime exactly (verified against
//! the `ModeSpec` table for Scottie 1 / 2 / DX in `modespec.rs`).

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
const SEPTR_HZ: f64 = 1500.0;
const BLACK_HZ: f64 = 1500.0;
const WHITE_HZ: f64 = 2300.0;

fn lum_to_freq(lum: u8) -> f64 {
    BLACK_HZ + (WHITE_HZ - BLACK_HZ) * f64::from(lum) / 255.0
}

/// Emit samples up to absolute sample index `target_n`, advancing phase.
/// Cumulative-target pattern (matches `robot_test_encoder::fill_to`) so
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

/// Encode an image as Scottie 1 / 2 / DX audio. `rgb` is row-major,
/// `line_pixels × image_lines` `[R, G, B]` triples (320×256 for all
/// three Scottie modes). Returns f32 PCM at [`WORKING_SAMPLE_RATE_HZ`]
/// (11_025 Hz).
///
/// Per radio line, emits in this order:
///   1. Septr 1 at `SEPTR_HZ`
///   2. G channel at `pixel_seconds` per pixel
///   3. Septr 2 at `SEPTR_HZ`
///   4. B channel at `pixel_seconds` per pixel
///   5. Sync at `SYNC_HZ` (mid-line, between B and R)
///   6. Porch at `PORCH_HZ`
///   7. R channel at `pixel_seconds` per pixel
#[must_use]
#[doc(hidden)]
#[allow(dead_code)]
pub fn encode_scottie(mode: SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
    assert!(matches!(
        mode,
        SstvMode::Scottie1 | SstvMode::Scottie2 | SstvMode::ScottieDx
    ));
    let spec = crate::modespec::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    assert_eq!(rgb.len() as u32, w * h);

    let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
    let mut out: Vec<f32> = Vec::new();
    let mut phase = 0.0_f64;

    let mut t = 0.0_f64;
    let advance = |t: &mut f64, secs: f64| -> usize {
        *t += secs;
        (*t * sr).round() as usize
    };

    for y in 0..h {
        // Septr 1.
        fill_to(
            &mut out,
            SEPTR_HZ,
            advance(&mut t, spec.septr_seconds),
            &mut phase,
        );

        // G channel.
        for x in 0..w {
            let g = rgb[(y * w + x) as usize][1];
            fill_to(
                &mut out,
                lum_to_freq(g),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }

        // Septr 2.
        fill_to(
            &mut out,
            SEPTR_HZ,
            advance(&mut t, spec.septr_seconds),
            &mut phase,
        );

        // B channel.
        for x in 0..w {
            let b = rgb[(y * w + x) as usize][2];
            fill_to(
                &mut out,
                lum_to_freq(b),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }

        // Sync (mid-line, between B and R).
        fill_to(
            &mut out,
            SYNC_HZ,
            advance(&mut t, spec.sync_seconds),
            &mut phase,
        );

        // Porch.
        fill_to(
            &mut out,
            PORCH_HZ,
            advance(&mut t, spec.porch_seconds),
            &mut phase,
        );

        // R channel.
        for x in 0..w {
            let r = rgb[(y * w + x) as usize][0];
            fill_to(
                &mut out,
                lum_to_freq(r),
                advance(&mut t, spec.pixel_seconds),
                &mut phase,
            );
        }

        // Defensive pad to the line_seconds boundary. The Scottie line
        // layout sums to exactly LineTime per the ModeSpec table, but
        // float rounding could leave us a sample short — top up with
        // PORCH_HZ (1500 Hz, the inter-line idle frequency) so the
        // decoder's per-line timing model lines up with the audio.
        let line_end_target = f64::from(y + 1) * spec.line_seconds;
        let pad_secs = line_end_target - t;
        if pad_secs > 0.0 {
            fill_to(&mut out, PORCH_HZ, advance(&mut t, pad_secs), &mut phase);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lum_to_freq_endpoints() {
        assert!((lum_to_freq(0) - BLACK_HZ).abs() < 1e-9);
        assert!((lum_to_freq(255) - WHITE_HZ).abs() < 1e-9);
    }

    #[test]
    fn scottie1_encode_total_length() {
        let rgb = vec![[128u8; 3]; 320 * 256];
        let audio = encode_scottie(SstvMode::Scottie1, &rgb);
        let spec = crate::modespec::for_mode(SstvMode::Scottie1);
        let expected_len = (spec.line_seconds
            * f64::from(spec.image_lines)
            * f64::from(WORKING_SAMPLE_RATE_HZ)) as usize;
        // Allow 1-sample rounding slack at end-of-image.
        assert!(audio.len() >= expected_len);
        assert!(audio.len() <= expected_len + 1);
    }
}
