//! Slant correction + line-zero phase alignment.
//!
//! Translated from slowrx's `sync.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md`. Two responsibilities:
//!
//! 1. [`SyncTracker`] — per-sample boolean "is the 1200 Hz sync pulse
//!    dominant here?" Equivalent of slowrx's `Praw`/`Psync` ratio in
//!    `video.c` lines 271-297.
//! 2. [`find_sync`] — Hough-transform a captured `has_sync` track to
//!    detect slant, adjust the rate to cancel it, then locate line 0's
//!    `Skip` via 8-tap convolution on the column-summed sync image.
//!    Equivalent of slowrx's `sync.c::FindSync` (lines 18-133).
//!
//! Slowrx is offline-batch (read-all → first `GetVideo` populates
//! `HasSync[]` → `FindSync` adjusts → second `GetVideo` rereads cached
//! `StoredLum` at corrected pixel times). Our decoder accumulates one
//! image's worth of audio in the `Decoding` state, probes [`SyncTracker`]
//! at every [`SYNC_PROBE_STRIDE`] samples, then runs [`find_sync`] once.
//! The corrected `(rate, skip)` drives a single per-pixel decode pass.
//! `LineDecoded` events fire in fast succession at end-of-buffer rather
//! than incrementally; callers still see every event.

use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;

use crate::modespec::ModeSpec;
use crate::resample::WORKING_SAMPLE_RATE_HZ;

/// Stride between sync-band probes (working-rate samples).
///
/// slowrx uses 13 samples@44.1 kHz (`video.c:295`) ≈ 3.25 samples@11.025 kHz.
/// The fractional equivalence means no integer stride gives exact slowrx parity;
/// we choose 4 (round-up / ceil) rather than 3 (round-down / floor).
///
/// **Probe-count comparison:**
/// - slowrx probes/image ≈ `image_samples / 13` at 44.1 kHz.
/// - Rust probes/image ≈ `image_samples_11025 / 4` at 11.025 kHz.
/// - `image_samples_11025 / 4 ≈ (image_samples_44100 / 4) / 4 ≈ image_samples_44100 / 16`,
///   which is slightly fewer probes than slowrx's `/ 13`.
///
/// With `SYNC_PROBE_STRIDE = 4` Rust's per-image probe count is ≈ 19% fewer
/// than slowrx's. With stride=3 it was ≈ 25% more. Stride=4 is closer in
/// ratio (1.56 vs slowrx) and preserves the ~0.36 ms/probe cadence.  The
/// Hough transform's line-finding is robust to moderate density differences
/// (round-2 audit Finding 8).
pub(crate) const SYNC_PROBE_STRIDE: usize = 4;

/// Hann-windowed audio length per sync probe (samples). 1/4 of slowrx's
/// 64@44.1kHz keeps the time span (~1.5 ms) constant (`video.c:278`).
pub(crate) const SYNC_FFT_WINDOW_SAMPLES: usize = 16;

/// Zero-padded FFT length per sync probe. 256@11025 = 43 Hz/bin matches
/// slowrx's 1024@44100 (`video.c:280`).
pub(crate) const SYNC_FFT_LEN: usize = 256;

// Hough-transform slant search (slowrx `common.h:4-5` MINSLANT/MAXSLANT
// + sync.c step `q++` in 0.5° units via `q/2.0`); slant lock window
// matches sync.c:83 `slantAngle > 89 && slantAngle < 91`.
const MIN_SLANT_DEG: f64 = 30.0;
const MAX_SLANT_DEG: f64 = 150.0;
const SLANT_STEP_DEG: f64 = 0.5;
const SLANT_OK_LO_DEG: f64 = 89.0;
const SLANT_OK_HI_DEG: f64 = 91.0;
const MAX_SLANT_RETRIES: usize = 3;
// `xAcc[700]` (sync.c:23), `SyncImg[700][630]` (sync.c:26),
// `lines[600][...]` (sync.c:24).
const X_ACC_BINS: usize = 700;
const SYNC_IMG_Y_BINS: usize = 630;
const LINES_D_BINS: usize = 600;

/// Convert degrees to radians. Matches slowrx `common.c::deg2rad`.
fn deg2rad(deg: f64) -> f64 {
    deg * std::f64::consts::PI / 180.0
}

/// Per-sample sync-band probe context (FFT plan + buffers reused across
/// probes). `sync_target_bin` / `video_{lo,hi}_bin` are pre-computed bin
/// offsets corresponding to `1200 Hz` and `1500..=2300 Hz` shifted by
/// `hedr_shift_hz`.
pub(crate) struct SyncTracker {
    fft: Arc<dyn rustfft::Fft<f32>>,
    hann: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    sync_target_bin: usize,
    video_lo_bin: usize,
    video_hi_bin: usize,
}

impl SyncTracker {
    /// Construct a tracker with the radio mistuning offset extracted at
    /// VIS time.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn new(hedr_shift_hz: f64) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(SYNC_FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();

        // Use slowrx-equivalent truncation via `crate::get_bin` (not `.round()`).
        // See `crate::get_bin` for rationale.  sync_target_bin for 1200 Hz
        // is 27 (slowrx-correct) not 28 (what `.round()` would give).
        let bin_for =
            |hz: f64| -> usize { crate::get_bin(hz, SYNC_FFT_LEN, WORKING_SAMPLE_RATE_HZ) };

        Self {
            fft,
            hann: build_sync_hann(),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; SYNC_FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(SYNC_FFT_LEN)],
            sync_target_bin: bin_for(1200.0 + hedr_shift_hz),
            video_lo_bin: bin_for(1500.0 + hedr_shift_hz),
            video_hi_bin: bin_for(2300.0 + hedr_shift_hz),
        }
    }

    /// Probe a single window centered at `center_sample` of `audio`.
    /// Returns `true` when the 1200 Hz sync band has more power per Hz
    /// than the 1500-2300 Hz video band by at least 2×.
    ///
    /// Translated from slowrx `video.c` lines 271-297.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn has_sync_at(&mut self, audio: &[f32], center_sample: usize) -> bool {
        let half = (SYNC_FFT_WINDOW_SAMPLES as i64) / 2;
        self.fft_buf.fill(Complex { re: 0.0, im: 0.0 });
        for i in 0..SYNC_FFT_WINDOW_SAMPLES {
            let idx = (center_sample as i64) - half + (i as i64);
            let s = if idx >= 0 && (idx as usize) < audio.len() {
                audio[idx as usize]
            } else {
                0.0
            };
            self.fft_buf[i].re = s * self.hann[i];
        }
        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        let power = |c: Complex<f32>| -> f64 {
            let r = f64::from(c.re);
            let i = f64::from(c.im);
            r * r + i * i
        };

        // Praw = average power per bin across video band (video.c:282-288).
        let mut p_raw = 0.0_f64;
        let lo = self.video_lo_bin.max(1);
        let hi = self.video_hi_bin.min(SYNC_FFT_LEN / 2 - 1);
        if hi >= lo {
            for k in lo..=hi {
                p_raw += power(self.fft_buf[k]);
            }
            p_raw /= (hi - lo).max(1) as f64;
        }

        // Psync = triangle-weighted sum across [bin-1, bin, bin+1] / 2
        // (video.c:285-289).
        let mut p_sync = 0.0_f64;
        let bin = self.sync_target_bin.clamp(1, SYNC_FFT_LEN / 2 - 1);
        for offset in -1_i32..=1 {
            let k = (bin as i32 + offset) as usize;
            let weight = 1.0 - 0.5 * f64::from(offset.abs());
            p_sync += power(self.fft_buf[k]) * weight;
        }
        p_sync /= 2.0;

        // slowrx video.c:293: HasSync = (Psync > 2*Praw)
        p_sync > 2.0 * p_raw
    }
}

/// Build the Hann window used per sync probe.
#[allow(clippy::cast_precision_loss)]
fn build_sync_hann() -> Vec<f32> {
    (0..SYNC_FFT_WINDOW_SAMPLES)
        .map(|i| {
            let m = (SYNC_FFT_WINDOW_SAMPLES - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / m).cos())
        })
        .collect()
}

/// Result of [`find_sync`]: slant-corrected rate + line-zero `Skip`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SyncResult {
    /// Adjusted working-rate sample rate (Hz).
    pub adjusted_rate_hz: f64,
    /// Sample offset from the start of the sync track where line 0's
    /// video data begins. May be slightly negative; the decoder zero-pads
    /// out-of-range reads when computing per-channel slices.
    pub skip_samples: i64,
    /// Detected slant angle (degrees), or `None` when the Hough transform
    /// found no sync pulses at all (degenerate/empty input).
    ///
    /// Diagnostic — read by tests; the decoder consumes only
    /// `adjusted_rate_hz` + `skip_samples`. Using `Option<f64>` avoids the
    /// round-2 audit Finding 10 ambiguity where `90.0` would be returned for
    /// both "perfectly aligned input" and "nothing-detected-at-all input".
    #[allow(dead_code)]
    pub slant_deg: Option<f64>,
}

/// Linear Hough transform + 8-tap convolution edge-find.
///
/// `has_sync` is the per-stride boolean track produced by [`SyncTracker`].
/// `initial_rate_hz` is normally [`WORKING_SAMPLE_RATE_HZ`] but the
/// function may adjust it. Translated from slowrx `sync.c::FindSync`
/// (lines 18-133).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_lines
)]
pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult {
    let line_width: usize = ((spec.line_seconds / spec.sync_seconds) * 4.0) as usize;
    // (150-30) / 0.5 = 240 half-degree bins.
    let n_slant_bins = ((MAX_SLANT_DEG - MIN_SLANT_DEG) / SLANT_STEP_DEG).round() as usize;
    let mut rate = initial_rate_hz;
    let mut slant_deg_detected: Option<f64> = None;
    let num_lines = spec.image_lines as usize;

    // slowrx's `(int)(t * Rate / 13.0)` becomes `(int)(t * rate / STRIDE)`
    // (the "13" in slowrx's index normalizes to stride units).
    let probe_index = |t: f64, rate_hz: f64| -> usize {
        let raw = t * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    let mut sync_img = vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS];
    // lines[d][q] flattened as lines[d * n_slant_bins + q].
    let mut lines = vec![0u16; LINES_D_BINS * n_slant_bins];

    for retry in 0..=MAX_SLANT_RETRIES {
        // Draw the 2D sync signal at current rate.
        sync_img.fill(false);
        for y in 0..num_lines.min(SYNC_IMG_Y_BINS) {
            for x in 0..line_width.min(X_ACC_BINS) {
                let t = ((y as f64) + (x as f64) / (line_width as f64)) * spec.line_seconds;
                let idx = probe_index(t, rate);
                if idx < has_sync.len() {
                    sync_img[x * SYNC_IMG_Y_BINS + y] = has_sync[idx];
                }
            }
        }

        // Linear Hough transform.
        lines.fill(0);
        let mut q_most = 0_usize;
        let mut max_count = 0_u16;
        for cy in 0..num_lines.min(SYNC_IMG_Y_BINS) {
            for cx in 0..line_width.min(X_ACC_BINS) {
                if !sync_img[cx * SYNC_IMG_Y_BINS + cy] {
                    continue;
                }
                for q in 0..n_slant_bins {
                    let theta = deg2rad(MIN_SLANT_DEG + (q as f64) * SLANT_STEP_DEG);
                    let d_signed = (line_width as f64)
                        + (-(cx as f64) * theta.sin() + (cy as f64) * theta.cos()).round();
                    if d_signed > 0.0 && d_signed < (line_width as f64) {
                        let d = d_signed as usize;
                        if d < LINES_D_BINS {
                            let cell = &mut lines[d * n_slant_bins + q];
                            *cell = cell.saturating_add(1);
                            if *cell > max_count {
                                max_count = *cell;
                                q_most = q;
                            }
                        }
                    }
                }
            }
        }

        if max_count == 0 {
            break;
        }

        let slant_angle = MIN_SLANT_DEG + (q_most as f64) * SLANT_STEP_DEG;
        slant_deg_detected = Some(slant_angle);

        // Apply a deadband at 90° so an exact-rate input is not perturbed
        // by half-degree Hough quantization noise — without this, a 90.5°
        // bin lands a 0.0085% rate "correction" that compounds across the
        // per-line xAcc projection and corrupts the falling-edge find.
        if (slant_angle - 90.0).abs() > SLANT_STEP_DEG {
            rate += (deg2rad(90.0 - slant_angle).tan() / (line_width as f64)) * rate;
        }

        // sync.c:86-90 resets to 44100 on retry exhaustion; we keep our
        // last estimate (re-anchoring a near-locked input is harmful).
        //
        // Open interval (89, 91) — matching slowrx `sync.c:83`:
        //   `if (slantAngle > 89 && slantAngle < 91)`.
        // The half-open range syntax `89.0..91.0` would include 89.0° and
        // exclude 91.0°, widening the lock window by one 0.5°-Hough bin vs
        // slowrx. Use explicit comparisons for exact parity (round-2 audit
        // Finding 7).
        if (slant_angle > SLANT_OK_LO_DEG && slant_angle < SLANT_OK_HI_DEG)
            || retry == MAX_SLANT_RETRIES
        {
            break;
        }
    }

    // Column accumulator + 8-tap convolution edge-find (sync.c:96-113).
    let mut x_acc = vec![0u32; X_ACC_BINS];
    for y in 0..num_lines {
        for (x, slot) in x_acc.iter_mut().enumerate() {
            let t = (y as f64) * spec.line_seconds
                + ((x as f64) / (X_ACC_BINS as f64)) * spec.line_seconds;
            let idx = probe_index(t, rate);
            if idx < has_sync.len() && has_sync[idx] {
                *slot = slot.saturating_add(1);
            }
        }
    }

    // 8-tap kernel snapshots `xmax = x+4` at the steepest falling edge.
    // slowrx `sync.c:29-30`: `double maxconvd=0; int xmax=0;`.
    // `max_convd` init must be 0 (not `i32::MIN`) — with zero input every
    // `convd == 0` would beat `i32::MIN` and place xmax at the last window
    // position, diverging from slowrx's "no update" on zero/negative convd
    // (round-2 audit Finding 6).
    let kernel: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];
    let mut xmax: i32 = 0;
    let mut max_convd: i32 = 0;
    for (x, window) in x_acc.windows(8).enumerate() {
        let convd: i32 = window
            .iter()
            .zip(kernel.iter())
            .map(|(&v, &k)| (v as i32) * k)
            .sum();
        if convd > max_convd {
            max_convd = convd;
            xmax = (x as i32) + 4;
        }
    }

    // sync.c:117 — pulse near the right edge slipped from previous left.
    if xmax > 350 {
        xmax -= 350;
    }

    // sync.c:120,127. V1 ports PD-family only; the Scottie branch
    // (sync.c:123-125) is unreachable here.
    let s_secs = (f64::from(xmax) / (X_ACC_BINS as f64)) * spec.line_seconds - spec.sync_seconds;
    let skip_samples = (s_secs * rate).round() as i64;

    SyncResult {
        adjusted_rate_hz: rate,
        skip_samples,
        slant_deg: slant_deg_detected,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
mod tests {
    use super::*;
    use crate::modespec;
    use crate::resample::WORKING_SAMPLE_RATE_HZ;
    use std::f64::consts::PI;

    fn synth_tone(freq_hz: f64, secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
                (2.0 * PI * freq_hz * t).sin() as f32
            })
            .collect()
    }

    #[test]
    fn has_sync_at_detects_1200_hz_burst() {
        let mut tracker = SyncTracker::new(0.0);
        let audio = synth_tone(1200.0, 0.050);
        assert!(tracker.has_sync_at(&audio, audio.len() / 2));
    }

    #[test]
    fn has_sync_at_rejects_1900_hz_tone() {
        let mut tracker = SyncTracker::new(0.0);
        let audio = synth_tone(1900.0, 0.050);
        assert!(!tracker.has_sync_at(&audio, audio.len() / 2));
    }

    #[test]
    fn has_sync_at_rejects_silence() {
        let mut tracker = SyncTracker::new(0.0);
        assert!(!tracker.has_sync_at(&vec![0.0_f32; 1024], 512));
    }

    /// Build a synthetic `has_sync` track with a sync pulse at every line start.
    fn synth_has_sync(spec: ModeSpec, rate_hz: f64) -> Vec<bool> {
        let total = (f64::from(spec.image_lines) * spec.line_seconds * rate_hz
            / (SYNC_PROBE_STRIDE as f64)) as usize
            + 16;
        let mut track = vec![false; total];
        for y in 0..spec.image_lines {
            let i_start =
                (f64::from(y) * spec.line_seconds * rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
            let i_end = ((f64::from(y) * spec.line_seconds + spec.sync_seconds) * rate_hz
                / (SYNC_PROBE_STRIDE as f64)) as usize;
            for slot in track.iter_mut().take(i_end.min(total)).skip(i_start) {
                *slot = true;
            }
        }
        track
    }

    #[test]
    fn find_sync_locks_clean_track_to_90_degrees() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let r = find_sync(&synth_has_sync(spec, rate), rate, spec);
        let slant = r.slant_deg.expect("sync detected");
        assert!((slant - 90.0).abs() < 1.0, "{slant:.2}°");
        assert!((r.adjusted_rate_hz - rate).abs() / rate < 0.005);
        assert!(r.skip_samples.abs() < (0.05 * rate) as i64);
    }

    /// With all-zero `has_sync`, the Hough transform finds nothing.
    /// `slant_deg` must be `None` (not `Some(90.0)`) and `skip_samples`
    /// must encode a negative offset (xmax=0, no sync detected).
    /// Verifies round-2 audit Finding 6 (xmax=0 on zero input) and
    /// Finding 10 (`slant_deg` is None, not the misleading 90.0 default).
    #[test]
    fn find_sync_empty_track_has_no_slant_detected() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let r = find_sync(&vec![false; 16384], rate, spec);
        assert!(
            r.slant_deg.is_none(),
            "empty track should yield slant_deg=None, got {:?}",
            r.slant_deg
        );
        // xmax=0 → s_secs = 0 - sync_seconds → skip is negative.
        assert!(
            r.skip_samples < 0,
            "empty track skip should be negative (xmax=0)"
        );
    }

    #[test]
    fn find_sync_recovers_known_offset() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        // Right-shift the track by ~10 ms (a real-radio settling gap).
        let mut track = synth_has_sync(spec, rate);
        let shift = ((0.010 * rate) / (SYNC_PROBE_STRIDE as f64)) as usize;
        let mut shifted = vec![false; shift];
        shifted.append(&mut track);
        let r = find_sync(&shifted, rate, spec);
        let expected = (0.010 * rate) as i64;
        // 700-bin row ≈ 0.7 ms / bin at PD120; allow a few bins for wobble.
        assert!(
            (r.skip_samples - expected).abs() < (0.005 * rate) as i64,
            "Skip off (expected ≈ {expected}, got {})",
            r.skip_samples
        );
    }

    #[test]
    fn find_sync_handles_empty_track() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let r = find_sync(&vec![false; 16384], rate, spec);
        assert!(r.adjusted_rate_hz.is_finite());
        assert!((r.adjusted_rate_hz - rate).abs() < 1.0);
        // Rate must be bit-exact when no sync detected (no rate correction ran).
        assert!(
            (r.adjusted_rate_hz - rate).abs() < f64::EPSILON,
            "rate should be unchanged, got {}",
            r.adjusted_rate_hz
        );
    }
}
