//! SNR estimation + Hann-window bank for the PD-family per-pixel demod.
//!
//! Translated from slowrx's `video.c` lines 53-58 (Hann bank initialization)
//! and lines 302-343 (SNR-estimator FFT and bandwidth-corrected SNR formula).
//! ISC License — see `NOTICE.md`.
//!
//! ## Working-rate scaling
//!
//! slowrx operates at `44_100` Hz with `FFTLen = 1024` and Hann lengths
//! `[48, 64, 96, 128, 256, 512, 1024]`. We operate at
//! [`crate::resample::WORKING_SAMPLE_RATE_HZ`] = `11_025` Hz with
//! `FFT_LEN = 256` (a 4× reduction that preserves the same Hz-per-bin
//! resolution: `44100/1024 ≈ 11025/256 ≈ 43.07` Hz/bin). The Hann lengths
//! shrink by the same factor: `[12, 16, 24, 32, 64, 128, 256]`.

use rustfft::{num_complex::Complex, FftPlanner};

/// FFT length used for both per-pixel demod and SNR estimation. Must
/// match [`crate::mode_pd::FFT_LEN`].
pub(crate) const FFT_LEN: usize = 256;

/// Hann-window lengths at the `11_025` Hz working rate (slowrx's
/// `[48, 64, 96, 128, 256, 512, 1024]` divided by 4). Index 6 (length
/// = `FFT_LEN`) is the "longest, lowest-SNR" window and is also reused
/// by the SNR estimator. Translated from `video.c:54`.
pub(crate) const HANN_LENS: [usize; 7] = [12, 16, 24, 32, 64, 128, 256];

/// Pre-computed Hann window of length [`FFT_LEN`]. Used inside the
/// SNR estimator and as the longest-window entry of the bank.
#[allow(clippy::cast_precision_loss)]
fn build_hann(len: usize) -> Vec<f32> {
    if len == 0 {
        return Vec::new();
    }
    if len == 1 {
        return vec![0.0_f32];
    }
    (0..len)
        .map(|i| {
            let m = (len - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / m).cos())
        })
        .collect()
}

/// Bank of seven Hann windows, indexed by SNR-derived window selector.
/// Construct once per decoder; the inner `Vec<f32>`s have lengths matching
/// [`HANN_LENS`].
pub(crate) struct HannBank {
    windows: [Vec<f32>; 7],
}

impl HannBank {
    pub fn new() -> Self {
        Self {
            windows: [
                build_hann(HANN_LENS[0]),
                build_hann(HANN_LENS[1]),
                build_hann(HANN_LENS[2]),
                build_hann(HANN_LENS[3]),
                build_hann(HANN_LENS[4]),
                build_hann(HANN_LENS[5]),
                build_hann(HANN_LENS[6]),
            ],
        }
    }

    /// Borrow window `idx` (0..=6). Length is `HANN_LENS[idx]`.
    #[must_use]
    pub fn get(&self, idx: usize) -> &[f32] {
        &self.windows[idx]
    }
}

impl Default for HannBank {
    fn default() -> Self {
        Self::new()
    }
}

/// Pick the SNR-adaptive Hann-window index. Translated from `video.c:356-364`:
///
/// ```text
/// SNR ≥ 20 → 0   (shortest window, sharpest time resolution)
/// SNR ≥ 10 → 1
/// SNR ≥  9 → 2
/// SNR ≥  3 → 3
/// SNR ≥ -5 → 4   (64-sample window at 11_025 Hz; 256 in slowrx at 44_100 Hz)
/// SNR ≥ -10 → 5
/// otherwise → 6  (longest window, max noise rejection)
/// ```
///
/// slowrx also bumps the index up by one for Scottie DX (`Mode == SDX`)
/// when `WinIdx < 6`. We don't yet support Scottie modes, so that branch
/// is omitted; once the SDX decoder lands it will pass `is_sdx: bool`
/// or call a separate selector.
#[must_use]
pub(crate) fn window_idx_for_snr(snr_db: f64) -> usize {
    if snr_db >= 20.0 {
        0
    } else if snr_db >= 10.0 {
        1
    } else if snr_db >= 9.0 {
        2
    } else if snr_db >= 3.0 {
        3
    } else if snr_db >= -5.0 {
        4
    } else if snr_db >= -10.0 {
        5
    } else {
        6
    }
}

/// Hysteresis variant of [`window_idx_for_snr`]. Takes a `prev_idx`
/// (the window index used by the previous FFT in this channel decode)
/// and applies a 1 dB hysteresis band at every threshold to prevent
/// flip-flop on real-radio SNR fluctuations near boundary values.
///
/// **Algorithm:** ratchet by at most one band toward the `baseline`
/// lookup per call, applying a 0.5 dB hysteresis at the adjacent
/// boundary. Concretely:
///
/// 1. Compute `baseline = window_idx_for_snr(snr_db)`. If it equals
///    `prev_idx` the SNR is in `prev_idx`'s band — return immediately.
/// 2. Pick `target_idx` one band closer to `baseline` than `prev_idx`
///    (`prev_idx + 1` if degrading, `prev_idx - 1` if improving).
/// 3. Re-evaluate `window_idx_for_snr` at a shifted SNR: if moving to
///    a shorter window (lower idx) require an extra 0.5 dB headroom;
///    if moving to a longer window require 0.5 dB more noise.
/// 4. If the shifted lookup confirms the SNR is past `target_idx`'s
///    side of the boundary, accept `target_idx`. Otherwise we're
///    inside the 1 dB hysteresis band — stay at `prev_idx`.
///
/// Ratcheting one band per call (rather than jumping straight to
/// `baseline`) keeps the selector convergent even when `prev_idx` is
/// far from `baseline` — e.g. cold-start at idx 6 with a strong signal
/// — without breaking the hysteresis guarantee at any single boundary.
/// Per-pixel FFTs converge in O(`n_bands`) calls.
///
/// **Direction semantics:** `prev_idx` lower than `baseline` means SNR
/// is degrading (longer window needed). `prev_idx` higher than
/// `baseline` means SNR is improving (shorter window allowed).
///
/// **Deliberate divergence from slowrx C** (`video.c:354-367`), which
/// uses pure-threshold logic with no hysteresis. See
/// `docs/intentional-deviations.md` for rationale.
#[must_use]
pub(crate) fn window_idx_for_snr_with_hysteresis(snr_db: f64, prev_idx: usize) -> usize {
    /// Half-band size in dB. Total hysteresis band at each threshold
    /// is `2 * HYSTERESIS_DB_HALF` = 1.0 dB.
    const HYSTERESIS_DB_HALF: f64 = 0.5;

    let baseline = window_idx_for_snr(snr_db);
    if baseline == prev_idx {
        return prev_idx;
    }

    // Ratchet one band toward `baseline`. `prev_idx > 0` is guaranteed
    // when `baseline < prev_idx` because indices are non-negative, so
    // the subtraction below is safe.
    let target_idx = if baseline > prev_idx {
        prev_idx + 1
    } else {
        prev_idx - 1
    };

    // Hysteresis: shift SNR away from `target_idx`. If the shifted
    // lookup still indicates we're past `target_idx`'s side of the
    // boundary, the move is robust.
    let shifted_snr = if target_idx < prev_idx {
        // Moving to shorter window: require extra SNR headroom.
        snr_db - HYSTERESIS_DB_HALF
    } else {
        // Moving to longer window: require extra noise.
        snr_db + HYSTERESIS_DB_HALF
    };
    let shifted_idx = window_idx_for_snr(shifted_snr);

    let robust = if target_idx < prev_idx {
        shifted_idx <= target_idx
    } else {
        shifted_idx >= target_idx
    };

    if robust {
        target_idx
    } else {
        prev_idx
    }
}

/// Per-decoder SNR estimator. Owns its own FFT plan + scratch buffer
/// (separate from the per-pixel demod's plan so concurrent calls never
/// fight over the same scratch slice). One full-length Hann window is
/// pre-computed and reused on every estimate.
///
/// Translated from `video.c:302-343`. Each `estimate` call mirrors one
/// pass through that block: FFT a 256-sample window, integrate power
/// over `[1500+hedr, 2300+hedr]` Hz (video band) and over
/// `[400+hedr, 800+hedr] ∪ [2700+hedr, 3400+hedr]` Hz (noise band),
/// apply the bandwidth correction in `video.c:336-338`, and return
/// `10·log10(Psignal / Pnoise)` floored at -20 dB.
pub(crate) struct SnrEstimator {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    hann_long: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl SnrEstimator {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann_long: build_hann(FFT_LEN),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
        }
    }

    /// Estimate SNR in dB for a window of audio centered on
    /// `center_sample`. `hedr_shift_hz` shifts the video band as in
    /// `video.c:316-326`. Out-of-bounds samples zero-pad.
    ///
    /// Returns SNR in dB; floored at -20 dB to match `video.c:340-341`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn estimate(&mut self, audio: &[f32], center_sample: i64, hedr_shift_hz: f64) -> f64 {
        // Fill FFT buffer: zero-pad, then window the live samples.
        let half = (FFT_LEN as i64) / 2;
        for i in 0..FFT_LEN {
            let idx = center_sample - half + i as i64;
            let s = if idx >= 0 && (idx as usize) < audio.len() {
                audio[idx as usize]
            } else {
                0.0
            };
            self.fft_buf[i] = Complex {
                re: s * self.hann_long[i],
                im: 0.0,
            };
        }

        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        // Bin helper — uses slowrx-equivalent truncation via `crate::get_bin`.
        // **Do NOT change to `.round()`**: that shifts 5 of the 8 production
        // frequencies by ±1 bin, changing `VideoPlusNoiseBins`, `NoiseOnlyBins`,
        // and `ReceiverBins` by 1–4 % vs. slowrx's values (see round-2 audit
        // Finding 2/3 for the full bandwidth-correction impact).
        let bin_for = |hz: f64| -> usize {
            crate::get_bin(hz, FFT_LEN, crate::resample::WORKING_SAMPLE_RATE_HZ)
                .min(FFT_LEN / 2 - 1)
        };

        let power = |c: Complex<f32>| -> f64 {
            let r = f64::from(c.re);
            let i = f64::from(c.im);
            r * r + i * i
        };

        // Integrate power over the video band (1500-2300 Hz, hedr-shifted).
        let video_lo = bin_for(1500.0 + hedr_shift_hz);
        let video_hi = bin_for(2300.0 + hedr_shift_hz);
        let mut p_video_plus_noise = 0.0_f64;
        for n in video_lo..=video_hi {
            p_video_plus_noise += power(self.fft_buf[n]);
        }

        // Integrate noise band: 400-800 Hz ∪ 2700-3400 Hz (hedr-shifted).
        let n_lo_a = bin_for(400.0 + hedr_shift_hz);
        let n_hi_a = bin_for(800.0 + hedr_shift_hz);
        let n_lo_b = bin_for(2700.0 + hedr_shift_hz);
        let n_hi_b = bin_for(3400.0 + hedr_shift_hz);
        let mut p_noise_only = 0.0_f64;
        for n in n_lo_a..=n_hi_a {
            p_noise_only += power(self.fft_buf[n]);
        }
        for n in n_lo_b..=n_hi_b {
            p_noise_only += power(self.fft_buf[n]);
        }

        // Bandwidth corrections — `video.c:329-334` (computed against an
        // un-shifted reference band, matching slowrx).
        let video_plus_noise_bins = bin_for(2300.0) - bin_for(1500.0) + 1;
        let noise_only_bins =
            (bin_for(800.0) - bin_for(400.0) + 1) + (bin_for(3400.0) - bin_for(2700.0) + 1);
        let receiver_bins = bin_for(3400.0) - bin_for(400.0);

        if noise_only_bins == 0 {
            return -20.0;
        }

        // Eq 15 from slowrx (`video.c:336-338`).
        let p_noise = p_noise_only * (receiver_bins as f64) / (noise_only_bins as f64);
        let p_signal = p_video_plus_noise
            - p_noise_only * (video_plus_noise_bins as f64) / (noise_only_bins as f64);

        if p_noise <= 0.0 || p_signal / p_noise < 0.01 {
            -20.0
        } else {
            10.0 * (p_signal / p_noise).log10()
        }
    }
}

impl Default for SnrEstimator {
    fn default() -> Self {
        Self::new()
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
    use crate::resample::WORKING_SAMPLE_RATE_HZ;
    use std::f64::consts::PI;

    fn synth_tone(freq_hz: f64, secs: f64) -> Vec<f32> {
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let n = (secs * sr).round() as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq_hz * (i as f64) / sr).sin() as f32)
            .collect()
    }

    fn synth_noise(secs: f64, amp: f32, seed: u32) -> Vec<f32> {
        // Deterministic tiny LCG → no rand dep needed for tests.
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let n = (secs * sr).round() as usize;
        let mut s: u32 = seed.max(1);
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let v = ((s >> 8) & 0xFFFF) as f32 / 32_768.0 - 1.0;
                v * amp
            })
            .collect()
    }

    #[test]
    fn hann_lens_match_slowrx_at_workingrate() {
        // Sanity: lengths are slowrx [48,64,96,128,256,512,1024] / 4.
        assert_eq!(HANN_LENS, [12, 16, 24, 32, 64, 128, 256]);
    }

    #[test]
    fn hann_bank_lengths_correct() {
        let bank = HannBank::new();
        for (i, &expected_len) in HANN_LENS.iter().enumerate() {
            assert_eq!(bank.get(i).len(), expected_len, "idx={i}");
        }
    }

    #[test]
    fn hann_window_endpoints_are_zero() {
        // True Hann: w[0] = w[N-1] = 0, peak at the middle.
        let bank = HannBank::new();
        for idx in 0..7 {
            let w = bank.get(idx);
            assert!(w[0].abs() < 1e-6, "idx={idx} w[0]={}", w[0]);
            assert!(
                w[w.len() - 1].abs() < 1e-6,
                "idx={idx} w[end]={}",
                w[w.len() - 1]
            );
        }
    }

    #[test]
    fn window_idx_thresholds_match_slowrx() {
        // Snapshot of `video.c:356-364`.
        assert_eq!(window_idx_for_snr(30.0), 0);
        assert_eq!(window_idx_for_snr(20.0), 0);
        assert_eq!(window_idx_for_snr(19.999), 1);
        assert_eq!(window_idx_for_snr(10.0), 1);
        assert_eq!(window_idx_for_snr(9.999), 2);
        assert_eq!(window_idx_for_snr(9.0), 2);
        assert_eq!(window_idx_for_snr(8.999), 3);
        assert_eq!(window_idx_for_snr(3.0), 3);
        assert_eq!(window_idx_for_snr(2.999), 4);
        assert_eq!(window_idx_for_snr(-5.0), 4);
        assert_eq!(window_idx_for_snr(-5.001), 5);
        assert_eq!(window_idx_for_snr(-10.0), 5);
        assert_eq!(window_idx_for_snr(-10.001), 6);
        assert_eq!(window_idx_for_snr(-100.0), 6);
    }

    #[test]
    fn snr_silence_floors_at_minus_twenty() {
        let mut est = SnrEstimator::new();
        let audio = vec![0.0_f32; 1024];
        let snr = est.estimate(&audio, 512, 0.0);
        assert!(
            (snr - -20.0).abs() < 1e-9,
            "silence should floor at -20 dB, got {snr}"
        );
    }

    #[test]
    fn snr_pure_video_tone_is_high() {
        // Pure 1900 Hz tone (mid-video band) → very large p_video_plus_noise,
        // tiny p_noise_only → huge SNR.
        let mut est = SnrEstimator::new();
        let audio = synth_tone(1900.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let snr = est.estimate(&audio, center, 0.0);
        assert!(snr > 25.0, "expected high SNR, got {snr}");
    }

    #[test]
    fn snr_pure_noise_band_is_negative() {
        // Pure 600 Hz (in 400-800 Hz noise band): all power in the noise
        // bins → bandwidth-corrected SNR is very negative (floors at -20).
        let mut est = SnrEstimator::new();
        let audio = synth_tone(600.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let snr = est.estimate(&audio, center, 0.0);
        assert!(snr <= 0.0, "expected ≤ 0 dB SNR, got {snr}");
    }

    #[test]
    fn snr_tone_plus_noise_intermediate() {
        // 1900 Hz tone at amp ~0.3 + white noise at amp ~1.0 →
        // intermediate SNR (signal partially buried).
        let mut est = SnrEstimator::new();
        let mut audio = synth_tone(1900.0, 0.100);
        for (i, n) in synth_noise(0.100, 1.0, 0xCAFE).into_iter().enumerate() {
            if i < audio.len() {
                audio[i] = audio[i] * 0.3 + n;
            }
        }
        let center = (audio.len() / 2) as i64;
        let snr = est.estimate(&audio, center, 0.0);
        assert!(
            (-20.0..30.0).contains(&snr),
            "intermediate SNR expected, got {snr}"
        );
    }

    #[test]
    fn snr_hedr_shift_tracks_band() {
        // 1950 Hz tone with hedr=+50 → tone sits in shifted video band → high SNR.
        // Same tone at hedr=-200 → tone sits OUTSIDE shifted video band but IN
        // shifted noise band (400-800 Hz hedr-shifted to 200-600 ... 2700-3400
        // hedr-shifted to 2500-3200). 1950 is between those, so no power lands
        // in noise; signal still mostly leaks across both bands and is clipped.
        let mut est = SnrEstimator::new();
        let audio = synth_tone(1950.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let snr_aligned = est.estimate(&audio, center, 50.0);
        assert!(snr_aligned > 25.0, "aligned: got {snr_aligned}");
    }

    #[test]
    fn snr_estimator_default_constructs() {
        let _ = SnrEstimator::default();
        let _ = HannBank::default();
    }

    #[test]
    fn build_hann_zero_and_one_length_safe() {
        // Defensive: degenerate inputs do not panic.
        assert!(build_hann(0).is_empty());
        assert_eq!(build_hann(1), vec![0.0]);
    }

    /// Verify SNR bandwidth-correction bin counts match slowrx's `GetBin`
    /// truncation semantics (round-2 audit Finding 2/3).
    ///
    /// At `FFT_LEN=256, SR=11025` with `hedr=0`:
    ///
    /// | Quantity                    | Expected (slowrx `GetBin` trunc) |
    /// |-----------------------------|----------------------------------|
    /// | `video_plus_noise_bins`     | 53 − 34 + 1 = 20                |
    /// | `noise_only_bins`           | (18−9+1) + (78−62+1) = 10+17 = 27 |
    /// | `receiver_bins`             | 78 − 9 = 69                      |
    /// | Pnoise multiplier      | 69/27 ≈ 2.5556                |
    /// | Psignal subtractor     | 20/27 ≈ 0.7407                |
    ///
    /// These values differ from what `.round()` would give (70/28 = 2.5 and
    /// 19/28 ≈ 0.6786 respectively), so this test is a regression guard for
    /// the `get_bin` truncation in `snr.rs::estimate`.
    #[test]
    fn snr_bandwidth_correction_bins_match_slowrx() {
        let get_bin =
            |hz: f64| crate::get_bin(hz, FFT_LEN, crate::resample::WORKING_SAMPLE_RATE_HZ);

        let video_lo = get_bin(1500.0);
        let video_hi = get_bin(2300.0);
        let n_lo_a = get_bin(400.0);
        let n_hi_a = get_bin(800.0);
        let n_lo_b = get_bin(2700.0);
        let n_hi_b = get_bin(3400.0);

        let video_plus_noise_bins = video_hi - video_lo + 1;
        let noise_only_bins = (n_hi_a - n_lo_a + 1) + (n_hi_b - n_lo_b + 1);
        let receiver_bins = n_hi_b - n_lo_a;

        assert_eq!(
            video_plus_noise_bins, 20,
            "video+noise bins: got {video_plus_noise_bins}"
        );
        assert_eq!(
            noise_only_bins, 27,
            "noise-only bins: got {noise_only_bins}"
        );
        assert_eq!(receiver_bins, 69, "receiver bins: got {receiver_bins}");

        // Pnoise multiplier ≈ 2.5556 (slowrx) vs 2.5000 (with .round())
        let pnoise_mult = receiver_bins as f64 / noise_only_bins as f64;
        assert!(
            (pnoise_mult - 69.0 / 27.0).abs() < 1e-9,
            "Pnoise mult: {pnoise_mult}"
        );

        // Psignal subtractor ≈ 0.7407 (slowrx) vs 0.6786 (with .round())
        let psignal_sub = video_plus_noise_bins as f64 / noise_only_bins as f64;
        assert!(
            (psignal_sub - 20.0 / 27.0).abs() < 1e-9,
            "Psignal sub: {psignal_sub}"
        );
    }

    /// Verify `sync_target_bin` for 1200 Hz is 27 (slowrx-correct truncation),
    /// not 28 (what `.round()` gives) — round-2 audit Finding 4 / #51.
    #[test]
    fn sync_target_bin_for_1200hz_is_27() {
        // At SYNC_FFT_LEN=256, SR=11025: 1200 * 256 / 11025 = 27.89... → trunc = 27.
        let bin = crate::get_bin(
            1200.0,
            crate::sync::SYNC_FFT_LEN,
            crate::resample::WORKING_SAMPLE_RATE_HZ,
        );
        assert_eq!(
            bin, 27,
            "sync_target_bin for 1200 Hz should be 27 (trunc), got {bin}"
        );
    }

    #[test]
    fn hysteresis_in_band_stays_put() {
        // SNR 9.3, just above the 9 dB threshold (win_idx 2 boundary).
        // Currently at prev=3. Shifted SNR (9.3 - 0.5) = 8.8 < 9, so the
        // shifted lookup disagrees with baseline (2). Stay at prev_idx=3.
        assert_eq!(window_idx_for_snr_with_hysteresis(9.3, 3), 3);
    }

    #[test]
    fn hysteresis_robust_change_propagates() {
        // SNR 9.5, comfortably above the 9 dB threshold.
        // Currently at prev=3. Shifted SNR (9.5 - 0.5) = 9.0 still ≥ 9,
        // shifted lookup agrees with baseline (2). Switch to 2.
        assert_eq!(window_idx_for_snr_with_hysteresis(9.5, 3), 2);
    }

    #[test]
    fn hysteresis_symmetric_in_band() {
        // SNR 8.5, just below 9 dB. Currently at prev=2.
        // Baseline says 3 (since 8.5 < 9). Shifted SNR (8.5 + 0.5) = 9.0
        // still ≥ 9, so shifted lookup says 2 — disagrees with baseline.
        // Stay at prev_idx=2.
        assert_eq!(window_idx_for_snr_with_hysteresis(8.5, 2), 2);
    }

    #[test]
    fn hysteresis_symmetric_robust() {
        // SNR 8.0, comfortably below 9. Currently at prev=2.
        // Baseline says 3 (8.0 < 9). Shifted SNR (8.0 + 0.5) = 8.5 < 9,
        // shifted lookup also says 3. Both agree. Switch to 3.
        assert_eq!(window_idx_for_snr_with_hysteresis(8.0, 2), 3);
    }

    #[test]
    fn hysteresis_no_change_when_in_equilibrium() {
        // SNR 15, prev=1. Baseline lookup at 15 returns 1 (≥ 10, < 20).
        // Fast-path: baseline == prev_idx, return prev_idx without
        // computing shifted lookup.
        assert_eq!(window_idx_for_snr_with_hysteresis(15.0, 1), 1);
    }

    #[test]
    fn hysteresis_at_extreme_thresholds() {
        // High end: SNR 20.5, prev=1. Baseline 0 (≥ 20). Shifted
        // (20.5 - 0.5) = 20.0 still ≥ 20, lookup also 0. Switch to 0.
        assert_eq!(window_idx_for_snr_with_hysteresis(20.5, 1), 0);

        // Low end: SNR -10.5, prev=5. Baseline 6 (< -10). Shifted
        // (-10.5 + 0.5) = -10.0 still ≥ -10, lookup says 5 — disagrees.
        // Stay at prev_idx=5.
        assert_eq!(window_idx_for_snr_with_hysteresis(-10.5, 5), 5);

        // SNR -11.0 from prev=5. Baseline 6 (-11 < -10). Shifted
        // (-11 + 0.5) = -10.5 < -10, lookup also 6. Switch to 6.
        assert_eq!(window_idx_for_snr_with_hysteresis(-11.0, 5), 6);
    }

    #[test]
    fn hysteresis_ratchets_from_distant_prev_low_snr() {
        // CodeRabbit-flagged regression case: SNR 9.2 with prev=4.
        // Baseline 2 (≥ 9), prev=4 → target=3 (one band toward
        // baseline). Shifted SNR (9.2 - 0.5) = 8.7 → idx 3, which is
        // ≤ target=3, so accept the ratchet. The earlier algorithm
        // returned `prev` because shifted (3) ≠ baseline (2),
        // permanently stranding the selector at idx 4.
        assert_eq!(window_idx_for_snr_with_hysteresis(9.2, 4), 3);
    }

    #[test]
    fn hysteresis_ratchets_from_distant_prev_high_snr() {
        // Strong improvement: SNR 20.2 with prev=4. Baseline 0 (≥ 20),
        // prev=4 → target=3. Shifted SNR (20.2 - 0.5) = 19.7 → idx 1,
        // 1 ≤ 3 → robust, accept target=3. The earlier algorithm
        // returned prev=4 because shifted (1) ≠ baseline (0).
        assert_eq!(window_idx_for_snr_with_hysteresis(20.2, 4), 3);
    }

    #[test]
    fn hysteresis_converges_high_snr_from_cold_start() {
        // Cold-start (prev=6, longest window) with a high SNR signal
        // ratchets one band per call until it reaches baseline=0,
        // then settles via the equilibrium fast-path.
        let mut idx = 6;
        let snr = 25.0;
        for expected in [5, 4, 3, 2, 1, 0] {
            idx = window_idx_for_snr_with_hysteresis(snr, idx);
            assert_eq!(
                idx, expected,
                "ratchet should land at {expected}, got {idx}"
            );
        }
        // Settled: baseline (0) == prev (0), fast-path return.
        assert_eq!(window_idx_for_snr_with_hysteresis(snr, idx), 0);
    }

    #[test]
    fn hysteresis_converges_degrading_snr() {
        // SNR collapses from a clean signal (prev=0) to deep noise
        // (baseline=6). Ratchet one band per call.
        let mut idx = 0;
        let snr = -50.0;
        for expected in [1, 2, 3, 4, 5, 6] {
            idx = window_idx_for_snr_with_hysteresis(snr, idx);
            assert_eq!(
                idx, expected,
                "ratchet should land at {expected}, got {idx}"
            );
        }
        assert_eq!(window_idx_for_snr_with_hysteresis(snr, idx), 6);
    }
}
