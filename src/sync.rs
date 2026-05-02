//! Slant correction + line-zero phase alignment.
//!
//! Translated from slowrx's `sync.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution.
//!
//! Two responsibilities:
//! 1. [`SyncTracker`] — per-sample boolean stream "is the 1200 Hz sync pulse
//!    dominant here?" Equivalent of slowrx's `Praw`/`Psync` ratio in
//!    `video.c` lines 271-297.
//! 2. [`find_sync`] — given the captured `has_sync` track + estimated rate
//!    + mode spec, Hough-transform out the slant angle, adjust the rate to
//!    cancel it, then locate the line-zero `Skip` via 8-tap convolution on
//!    a column-summed sync image. Equivalent of slowrx's
//!    `sync.c::FindSync` (lines 18-133).
//!
//! ## Adaptation to our streaming model
//!
//! slowrx is offline-batch: capture all audio into PCM ring → first
//! `GetVideo` populates `HasSync[]` → `FindSync` adjusts → second
//! `GetVideo` re-reads cached `StoredLum` at corrected pixel times.
//!
//! Our decoder is streaming. To keep parity we adapt as follows:
//! - The decoder accumulates `image_lines × line_seconds × headroom` worth
//!   of working-rate audio in its `Decoding` state.
//! - During accumulation it probes the audio at [`SYNC_PROBE_STRIDE`]
//!   samples through [`SyncTracker::has_sync_at`], producing a `Vec<bool>`
//!   of the same shape slowrx's `HasSync[]` array has.
//! - Once the buffer is full, [`find_sync`] runs once. Its returned
//!   `(adjusted_rate_hz, skip_samples)` then drives a single per-pixel
//!   decode pass over the buffered audio.
//!
//! This means `LineDecoded` events fire in fast succession at the end of
//! the per-image audio window rather than incrementally during decode.
//! That is acceptable for V1 — callers still get every event, just shifted
//! in time.

use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;

use crate::modespec::ModeSpec;
use crate::resample::WORKING_SAMPLE_RATE_HZ;

/// Stride between sync-band probes (working-rate samples).
///
/// slowrx samples `HasSync[]` every 13 audio samples at 44.1 kHz
/// (`video.c:295`). At our 11.025 kHz working rate that becomes
/// `13 × 11025 / 44100 = 3.25` samples. We round down to 3 to keep the
/// stride a small integer; slowrx's `t * Rate / 13.0` arithmetic uses a
/// `13.0/44100` time base, so the numerical equivalence is preserved by
/// scaling consistently in [`find_sync`].
pub(crate) const SYNC_PROBE_STRIDE: usize = 3;

/// Hann-windowed audio length used per sync probe (samples). slowrx uses
/// a 64-sample window at 44.1 kHz (`video.c:278`); we scale by 1/4 to
/// keep the time span (~1.5 ms) constant.
pub(crate) const SYNC_FFT_WINDOW_SAMPLES: usize = 16;

/// Zero-padded FFT length used per sync probe. slowrx zero-pads to 1024
/// at 44.1 kHz, giving 43 Hz/bin. We zero-pad to 256 at 11.025 kHz
/// (`11025 / 256 = 43.07 Hz/bin`) for matching bin density.
pub(crate) const SYNC_FFT_LEN: usize = 256;

/// Hough-transform slant search bounds (slowrx `common.h:4-5`:
/// `MINSLANT=30`, `MAXSLANT=150`; step `q ++` in 0.5° units via `q/2.0`).
const MIN_SLANT_DEG: f64 = 30.0;
/// Upper-exclusive bound on the slant search.
const MAX_SLANT_DEG: f64 = 150.0;
/// Half-degree resolution used by the Hough accumulator.
const SLANT_STEP_DEG: f64 = 0.5;
/// Slant lock window (degrees). Matches slowrx `sync.c:83`
/// (`slantAngle > 89 && slantAngle < 91`).
const SLANT_OK_LO_DEG: f64 = 89.0;
/// Upper edge of the slant lock window.
const SLANT_OK_HI_DEG: f64 = 91.0;
/// Maximum slant retries before giving up. Matches slowrx `sync.c:86`.
const MAX_SLANT_RETRIES: usize = 3;
/// Image-X resolution of the column accumulator. Matches slowrx
/// `xAcc[700]` in `sync.c:23`.
const X_ACC_BINS: usize = 700;
/// Length of slowrx's sync-image Y axis. Matches `SyncImg[700][630]`
/// (`sync.c:26`); only `NumLines` rows are actually populated.
const SYNC_IMG_Y_BINS: usize = 630;
/// Length of slowrx's Hough-line accumulator's d axis. Matches
/// `lines[600][...]` (`sync.c:24`).
const LINES_D_BINS: usize = 600;

/// Convert degrees to radians. Matches slowrx `common.c::deg2rad`.
fn deg2rad(deg: f64) -> f64 {
    deg * std::f64::consts::PI / 180.0
}

/// Per-sample sync-band probe context.
///
/// Allocates the FFT plan + reusable buffers once. Reuse across probes.
pub(crate) struct SyncTracker {
    fft: Arc<dyn rustfft::Fft<f32>>,
    hann: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Center bin of the 1200 Hz sync target (offset by `hedr_shift_hz`).
    sync_target_bin: usize,
    /// Lower bin of the 1500 Hz video band (offset by `hedr_shift_hz`).
    video_lo_bin: usize,
    /// Upper bin of the 2300 Hz video band (offset by `hedr_shift_hz`).
    video_hi_bin: usize,
}

impl SyncTracker {
    /// Construct a tracker with the radio mistuning offset extracted at
    /// VIS time. `hedr_shift_hz` shifts both the sync target (`1200 Hz`)
    /// and the video band (`1500..=2300 Hz`) so a mistuned radio's bands
    /// remain centered on the FFT bins searched here.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn new(hedr_shift_hz: f64) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(SYNC_FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();

        let bin_for = |hz: f64| -> usize {
            let raw = hz * (SYNC_FFT_LEN as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
            raw.round() as usize
        };

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
        // Fill the FFT input: SYNC_FFT_WINDOW_SAMPLES of Hann-windowed audio
        // around center_sample, zero-padded out to SYNC_FFT_LEN.
        let half = (SYNC_FFT_WINDOW_SAMPLES as i64) / 2;
        for slot in &mut self.fft_buf {
            *slot = Complex { re: 0.0, im: 0.0 };
        }
        for i in 0..SYNC_FFT_WINDOW_SAMPLES {
            let idx = (center_sample as i64) - half + (i as i64);
            let s = if idx >= 0 && (idx as usize) < audio.len() {
                audio[idx as usize]
            } else {
                0.0
            };
            self.fft_buf[i] = Complex {
                re: s * self.hann[i],
                im: 0.0,
            };
        }

        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        let power = |c: Complex<f32>| -> f64 {
            let r = f64::from(c.re);
            let i = f64::from(c.im);
            r * r + i * i
        };

        // Praw = average power per bin across the video band.
        // (slowrx video.c:282-288: sum(power) / num_bins)
        let mut p_raw = 0.0_f64;
        let lo = self.video_lo_bin.max(1);
        let hi = self.video_hi_bin.min(SYNC_FFT_LEN / 2 - 1);
        if hi >= lo {
            for k in lo..=hi {
                p_raw += power(self.fft_buf[k]);
            }
            let denom = (hi - lo).max(1) as f64;
            p_raw /= denom;
        }

        // Psync = triangle-weighted sum across [target-1, target, target+1] / 2,
        // matching slowrx video.c:285-289:
        //     for (i = bin-1; i <= bin+1; i++)
        //         Psync += power * (1 - 0.5 * abs(bin - i))
        //     Psync /= 2.0
        let mut p_sync = 0.0_f64;
        let bin = self
            .sync_target_bin
            .max(1)
            .min(SYNC_FFT_LEN / 2 - 1);
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

/// Result of [`find_sync`]: corrected sample rate + line-zero `Skip`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SyncResult {
    /// Adjusted working-rate sample rate (Hz). For drift-free synthetic
    /// audio this returns close to [`WORKING_SAMPLE_RATE_HZ`]; for real
    /// recordings it absorbs the typical fraction-of-a-percent drift.
    pub adjusted_rate_hz: f64,
    /// Sample offset from the start of the sync track at which line 0's
    /// video data begins. Negative-equivalent values are clamped to 0;
    /// callers should treat this as a positive offset.
    pub skip_samples: i64,
    /// Detected slant angle (degrees). Diagnostic — values close to 90°
    /// mean a clean lock; a lock that gave up returns the last estimate.
    pub slant_deg: f64,
}

/// Linear Hough transform + line-zero localization.
///
/// `has_sync` is the per-stride boolean track produced by [`SyncTracker`]
/// covering at least one full image's worth of audio. `initial_rate_hz`
/// is the assumed working rate ([`WORKING_SAMPLE_RATE_HZ`] under normal
/// operation, but the function will adjust it). `spec` provides the
/// mode's `LineTime`/`SyncTime`/`PixelTime`/`NumLines`.
///
/// Translated from slowrx `sync.c::FindSync` (lines 18-133).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_lines
)]
pub(crate) fn find_sync(
    has_sync: &[bool],
    initial_rate_hz: f64,
    spec: ModeSpec,
) -> SyncResult {
    // Constants matching slowrx sync.c.
    let line_width: usize = ((spec.line_seconds / spec.sync_seconds) * 4.0) as usize;
    // Number of slant bins between MIN_SLANT_DEG and MAX_SLANT_DEG at
    // SLANT_STEP_DEG resolution. `(150-30) / 0.5 = 240`.
    let n_slant_bins =
        ((MAX_SLANT_DEG - MIN_SLANT_DEG) / SLANT_STEP_DEG).round() as usize;

    let mut rate = initial_rate_hz;
    let mut slant_deg_last = 90.0;

    let num_lines = spec.image_lines as usize;

    // We mirror slowrx's "every 13 samples at 44100 Hz" indexing using
    // SYNC_PROBE_STRIDE at the working rate. The original index
    // `(int)(t * Rate / 13.0)` is equivalent here to
    // `(int)(t * rate / SYNC_PROBE_STRIDE)` because we sample HasSync
    // every SYNC_PROBE_STRIDE working-rate samples (the "13" in the C
    // index simply normalizes into stride units).
    let probe_index = |t_secs: f64, rate_hz: f64| -> usize {
        let raw = t_secs * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    // ---- Hough transform, repeated up to MAX_SLANT_RETRIES times. ----
    let mut sync_img = vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS];
    // lines[d][q] — flat layout is `lines[d * n_slant_bins + q]`.
    let mut lines = vec![0u16; LINES_D_BINS * n_slant_bins];

    for retry in 0..=MAX_SLANT_RETRIES {
        // Draw the 2D sync signal at current rate.
        for slot in sync_img.iter_mut().take(line_width.min(X_ACC_BINS) * num_lines) {
            *slot = false;
        }
        // (Easier: just clear all and redraw.)
        for slot in sync_img.iter_mut() {
            *slot = false;
        }
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
        for slot in lines.iter_mut() {
            *slot = 0;
        }
        let mut d_most = 0_usize;
        let mut q_most = 0_usize;
        let mut max_count = 0_u16;

        for cy in 0..num_lines.min(SYNC_IMG_Y_BINS) {
            for cx in 0..line_width.min(X_ACC_BINS) {
                if !sync_img[cx * SYNC_IMG_Y_BINS + cy] {
                    continue;
                }
                for q in 0..n_slant_bins {
                    let angle_deg = MIN_SLANT_DEG + (q as f64) * SLANT_STEP_DEG;
                    let theta = deg2rad(angle_deg);
                    let d_signed = (line_width as f64)
                        + (-(cx as f64) * theta.sin() + (cy as f64) * theta.cos()).round();
                    if d_signed > 0.0 && d_signed < (line_width as f64) {
                        let d = d_signed as usize;
                        if d < LINES_D_BINS {
                            let cell = &mut lines[d * n_slant_bins + q];
                            *cell = cell.saturating_add(1);
                            if *cell > max_count {
                                max_count = *cell;
                                d_most = d;
                                q_most = q;
                            }
                        }
                    }
                }
            }
        }

        let _ = d_most; // d_most is informational; not used downstream

        if max_count == 0 {
            // No sync signal detected.
            break;
        }

        let slant_angle =
            MIN_SLANT_DEG + (q_most as f64) * SLANT_STEP_DEG;
        slant_deg_last = slant_angle;

        // Adjust rate (slowrx sync.c:81).
        rate += (deg2rad(90.0 - slant_angle).tan() / (line_width as f64)) * rate;

        if (SLANT_OK_LO_DEG..SLANT_OK_HI_DEG).contains(&slant_angle) {
            break;
        }
        if retry == MAX_SLANT_RETRIES {
            // Slowrx sync.c:86-90: gives up and resets to 44100. We
            // instead keep our last estimate — re-anchoring to a wildly
            // different rate would do more harm than good for a clean
            // synthetic input that just happens to land at 90.0° on the
            // first try.
            break;
        }
    }

    // ---- Column accumulator + 8-tap convolution edge-find. ----
    let mut x_acc = vec![0u32; X_ACC_BINS];
    for y in 0..num_lines {
        for x in 0..X_ACC_BINS {
            let t = (y as f64) * spec.line_seconds
                + ((x as f64) / (X_ACC_BINS as f64)) * spec.line_seconds;
            let idx = probe_index(t, rate);
            if idx < has_sync.len() && has_sync[idx] {
                x_acc[x] = x_acc[x].saturating_add(1);
            }
        }
    }

    // 8-point convolution kernel [1,1,1,1,-1,-1,-1,-1].
    // slowrx sync.c:106-113 finds the maximum and snapshots `xmax = x+4`
    // as the falling-edge position.
    let kernel: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];
    let mut xmax: i32 = 0;
    let mut max_convd: i32 = i32::MIN;
    for x in 0..X_ACC_BINS - 8 {
        let mut convd = 0_i32;
        for (i, &k) in kernel.iter().enumerate() {
            // x_acc fits comfortably in i32 (at most NumLines, ~500).
            convd += (x_acc[x + i] as i32) * k;
        }
        if convd > max_convd {
            max_convd = convd;
            xmax = (x as i32) + 4;
        }
    }

    // Slowrx sync.c:117: pulse near the right edge slipped from the
    // previous line's left edge.
    if xmax > 350 {
        xmax -= 350;
    }

    // Skip = (xmax/700 * LineTime - SyncTime) * Rate (slowrx sync.c:120,127).
    // We do not implement Scottie offset (sync.c:123-125) — V1 supports
    // PD-family modes only, so the Scottie branch is unreachable.
    let s_secs = (f64::from(xmax) / (X_ACC_BINS as f64)) * spec.line_seconds - spec.sync_seconds;
    let skip_samples = (s_secs * rate).round() as i64;

    SyncResult {
        adjusted_rate_hz: rate,
        skip_samples,
        slant_deg: slant_deg_last,
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
        let center = audio.len() / 2;
        assert!(
            tracker.has_sync_at(&audio, center),
            "expected sync detection at 1200 Hz tone"
        );
    }

    #[test]
    fn has_sync_at_rejects_1900_hz_tone() {
        let mut tracker = SyncTracker::new(0.0);
        let audio = synth_tone(1900.0, 0.050);
        let center = audio.len() / 2;
        assert!(
            !tracker.has_sync_at(&audio, center),
            "1900 Hz video band should NOT trigger sync detection"
        );
    }

    #[test]
    fn has_sync_at_rejects_silence() {
        let mut tracker = SyncTracker::new(0.0);
        let audio = vec![0.0_f32; 1024];
        assert!(!tracker.has_sync_at(&audio, 512));
    }

    /// Build a synthetic `has_sync` track with a sync pulse at the start
    /// of every line (simulating a perfect, drift-free PD signal).
    fn synth_has_sync(spec: ModeSpec, rate_hz: f64) -> Vec<bool> {
        let total_secs = (spec.image_lines as f64) * spec.line_seconds;
        let total_probes = (total_secs * rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize + 16;
        let mut track = vec![false; total_probes];
        let track_len = track.len();
        for y in 0..spec.image_lines {
            // Sync pulse occupies the first `sync_seconds` of every line.
            let line_start_secs = (y as f64) * spec.line_seconds;
            let line_end_secs = line_start_secs + spec.sync_seconds;
            let i_start =
                (line_start_secs * rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
            let i_end =
                (line_end_secs * rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
            for slot in track.iter_mut().take(i_end.min(track_len)).skip(i_start) {
                *slot = true;
            }
        }
        track
    }

    #[test]
    fn find_sync_locks_clean_track_to_90_degrees() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let track = synth_has_sync(spec, rate);
        let result = find_sync(&track, rate, spec);
        // A drift-free track should be locked at 90° on the first pass and
        // produce a near-zero Skip (line 0 sync starts at sample 0).
        assert!(
            (result.slant_deg - 90.0).abs() < 1.0,
            "expected ≈90°, got {:.2}",
            result.slant_deg
        );
        // adjusted rate stays close to the input
        assert!(
            (result.adjusted_rate_hz - rate).abs() / rate < 0.005,
            "rate drift too large: {} vs {}",
            result.adjusted_rate_hz,
            rate
        );
        // Skip is small (line 0 sync starts at the buffer head).
        assert!(
            result.skip_samples.abs() < (0.05 * rate) as i64,
            "Skip too large: {} samples",
            result.skip_samples
        );
    }

    #[test]
    fn find_sync_recovers_known_offset() {
        // Same as above but offset the sync pulses by a constant so line 0
        // does not start at index 0.
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let mut track = synth_has_sync(spec, rate);
        // Shift the entire track right by ~10 ms (a real radio's pre-line
        // settling gap).
        let shift_probes =
            ((0.010 * rate) / (SYNC_PROBE_STRIDE as f64)) as usize;
        let mut shifted = vec![false; shift_probes];
        shifted.append(&mut track);
        let result = find_sync(&shifted, rate, spec);
        // Skip should be close to the shift (in working-rate samples).
        let expected_skip = (0.010 * rate) as i64;
        let diff = (result.skip_samples - expected_skip).abs();
        // The convolutional edge detector lands within one bin of the
        // 700-bin row (~LineTime/700 ≈ 0.7 ms = ~8 samples for PD120).
        // Allow a few times that to tolerate Hough-quantization wobble.
        assert!(
            diff < (0.005 * rate) as i64,
            "Skip recovery off by {diff} samples (expected ≈ {expected_skip}, got {})",
            result.skip_samples
        );
    }

    #[test]
    fn find_sync_handles_empty_track() {
        // No sync pulses anywhere — the algorithm should return without
        // panicking. Skip + rate may be garbage but must be finite.
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let track = vec![false; 16384];
        let result = find_sync(&track, rate, spec);
        assert!(result.adjusted_rate_hz.is_finite());
        // No detection should leave rate unchanged.
        assert!((result.adjusted_rate_hz - rate).abs() < 1.0);
    }
}
