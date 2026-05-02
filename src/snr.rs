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

        // Bin helper. Bin spacing matches `crate::mode_pd::FFT_LEN` /
        // working-rate, so `bin = round(hz * FFT_LEN / sample_rate)`.
        let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
        let bin_for = |hz: f64| -> usize {
            let raw = (hz * (FFT_LEN as f64) / work_rate).round();
            let clamped = raw.clamp(0.0, (FFT_LEN / 2 - 1) as f64);
            clamped as usize
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
}
