//! SSTV per-channel demod machinery.
//!
//! The per-pixel FFT, adaptive Hann-window selection, and YCbCr→RGB
//! conversion that turn the working-rate audio of one image-line channel
//! into pixel bytes. Consumed by `mode_pd` (PD-family), `mode_robot`
//! (Robot-family), and `mode_scottie` (Scottie/Martin RGB-sequential).
//!
//! `HannBank` / `HANN_LENS` / `window_idx_for_snr{_with_hysteresis}` moved
//! here from `crate::snr` (#85 B1): they're per-pixel-demod machinery, not
//! SNR-estimation logic. The `SnrEstimator` itself stays in `crate::snr` and
//! carries its own separate 1024-sample SNR-analysis Hann.

/// Hann-window lengths at the `11_025` Hz working rate (slowrx's
/// `[48, 64, 96, 128, 256, 512, 1024]` divided by 4). Index 6
/// (length 256) is the "longest, lowest-SNR" window in the per-pixel
/// demod's bank. The SNR estimator carries its own Hann window of
/// length [`crate::snr::FFT_LEN`] = 1024 — see [`crate::snr::SnrEstimator`]
/// for the size rationale. Translated from `video.c:54`.
pub(crate) const HANN_LENS: [usize; 7] = [12, 16, 24, 32, 64, 128, 256];

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
                crate::dsp::build_hann(HANN_LENS[0]),
                crate::dsp::build_hann(HANN_LENS[1]),
                crate::dsp::build_hann(HANN_LENS[2]),
                crate::dsp::build_hann(HANN_LENS[3]),
                crate::dsp::build_hann(HANN_LENS[4]),
                crate::dsp::build_hann(HANN_LENS[5]),
                crate::dsp::build_hann(HANN_LENS[6]),
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
/// when `WinIdx < 6`. The Scottie family decoder applies that bump
/// post-hoc inside [`crate::mode_pd::decode_one_channel_into`]
/// (matching slowrx C `video.c:367` exactly), so this bare selector
/// — and the [`window_idx_for_snr_with_hysteresis`] variant — stay
/// mode-agnostic.
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

    #[test]
    fn hann_bank_default_constructs() {
        let _ = HannBank::default();
    }
}
