//! PD-family mode decoder.
//!
//! PD modes encode each radio "frame" as four channels — Y(odd line),
//! Cr (shared), Cb (shared), Y(even line) — producing two image rows
//! per radio frame with chroma subsampling between them.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Channel layout: video.c lines 81-93. YUV→RGB matrix: video.c lines
//! 446-451. See `NOTICE.md` for full attribution.

use crate::demod::{freq_to_luminance, ycbcr_to_rgb, ChannelDemod, FFT_LEN, SNR_REESTIMATE_STRIDE};

/// Distance, in working-rate samples, between successive per-pixel
/// FFTs. slowrx takes an FFT every 6 samples at `44_100` Hz
/// (`video.c:350` — `if (SampleNum % 6 == 0)`). At our 4×-lower
/// `11_025` Hz working rate that scales to ~1.5 samples; we use
/// stride=1 (FFT every working-rate sample) so the pixel-time
/// readout of `stored_lum` is always exactly at-or-very-near the
/// pixel center, with no stride-induced positional error. The cost
/// difference vs slowrx's stride=6 is negligible on offline-batch
/// decoding at `11_025` Hz.
const PIXEL_FFT_STRIDE: i64 = 1;

/// Decode one PD radio frame (`Y(odd)`/`Cr`/`Cb`/`Y(even)`) into two image
/// rows of `image`. Translated from slowrx `video.c:259-486`.
///
/// Closes audit issues:
/// - **#24** time-base alignment: every pixel uses slowrx's exact
///   `Skip + round(rate * (chan_start_sec + pixel_secs * (x + 0.5)))`
///   single-round formula (`video.c:140-142`); `pair_seconds` is folded
///   in here, NOT pre-rounded by the caller, so per-pair rounding
///   error never accumulates.
/// - **#23** FFT-every-N + `StoredLum`: one FFT every
///   [`PIXEL_FFT_STRIDE`] samples produces the latest `Freq`, which
///   fills `StoredLum` at every sample; pixel times read out via
///   `StoredLum[pixel_time]` (`video.c:350-406`).
/// - **#18** SNR estimator: [`crate::snr::SnrEstimator`] is recomputed
///   every [`SNR_REESTIMATE_STRIDE`] samples (`video.c:302-343`).
///
/// **Deviations from slowrx (deliberate):**
/// - **#44 lifted with hysteresis (0.3.2)**: per-pixel Hann window
///   length is SNR-adaptive (slowrx `video.c:354-367`) plus a 1 dB
///   hysteresis band at each threshold to prevent flip-flop on real-
///   radio SNR fluctuations near boundary values. See
///   [`crate::demod::window_idx_for_snr_with_hysteresis`] and the
///   `SNR hysteresis on adaptive Hann window selection` entry in
///   `docs/intentional-deviations.md`.
/// - **#32 lifted via #45**: [`decode_one_channel_into`] reads
///   `audio` directly across channel boundaries — `chan_bounds_abs`
///   is accepted but ignored — matching slowrx C
///   (`video.c::GetVideo`). The earlier zero-pad-outside-channel
///   behavior caused visible vertical banding at every channel edge
///   on real radio (verified against Dec-2017 ARISS captures); the
///   1500–2300 Hz peak search keeps the FFT locked onto the dominant
///   video tone even when adjacent-channel content leaks into the
///   windowed support. The synthetic-corpus boundary-pixel `max_diff`
///   regression that this lift introduced is captured by the
///   "Synthetic round-trip `max_diff` tolerance" entry in
///   `docs/intentional-deviations.md`.
///
/// `skip_samples` is the absolute sample index inside `audio` where
/// pair zero's sync pulse begins; `pair_seconds` is `pair_index *
/// line_seconds` (un-rounded); `rate_hz` is the slant-corrected rate
/// from [`crate::sync::find_sync`].
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
pub(crate) fn decode_pd_line_pair(
    spec: crate::modespec::ModeSpec,
    pair_index: u32,
    audio: &[f32],
    skip_samples: i64,
    pair_seconds: f64,
    rate_hz: f64,
    image: &mut crate::image::SstvImage,
    demod: &mut ChannelDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;

    // PD channel time offsets (seconds from start of line pair):
    // Y(odd) → Cr → Cb → Y(even). Mirrors slowrx video.c:88-92:
    //   ChanStart[n+1] = ChanStart[n] + ChanLen[n] + SeptrTime
    // where ChanLen[n] = PixelTime * ImgWidth.
    // SeptrTime = 0 for the entire PD family (PD120/PD180/PD240, modespec.c),
    // so septr_secs is a no-op for current modes — but having it here
    // prevents a silent break when non-PD modes (Robot, Scottie, Martin —
    // all with non-zero SeptrTime) are added in V2.
    let chan_len = f64::from(width) * pixel_secs;
    let chan_starts_sec = [
        sync_secs + porch_secs,                                     // Y(odd): 0 septr
        sync_secs + porch_secs + chan_len + septr_secs,             // Cr:     1 septr
        sync_secs + porch_secs + 2.0 * chan_len + 2.0 * septr_secs, // Cb:     2 septr
        sync_secs + porch_secs + 3.0 * chan_len + 3.0 * septr_secs, // Y(even):3 septr
    ];

    let row0 = pair_index * 2;
    let row1 = row0 + 1;
    let width_us = width as usize;

    // Channel sample-range bounds (absolute audio indices). Used to
    // zero-pad the FFT windowed support outside the active channel —
    // see the doc comment's #32 deviation note. Computed using a
    // SINGLE `round()` per bound (slowrx `video.c:140-142`); the
    // per-pair `pair_seconds` offset is folded in here, NOT
    // pre-rounded by the caller, so per-pair rounding error does not
    // accumulate.
    let chan_bounds_abs: [(i64, i64); 4] = std::array::from_fn(|i| {
        let start_sec = chan_starts_sec[i];
        let end_sec = start_sec + f64::from(width) * pixel_secs;
        let start_abs = skip_samples + ((pair_seconds + start_sec) * rate_hz).round() as i64;
        let end_abs = skip_samples + ((pair_seconds + end_sec) * rate_hz).round() as i64;
        (start_abs, end_abs)
    });

    let mut y_odd = vec![0_u8; width_us];
    let mut cr = vec![0_u8; width_us];
    let mut cb = vec![0_u8; width_us];
    let mut y_even = vec![0_u8; width_us];

    let buffers: [&mut [u8]; 4] = [&mut y_odd, &mut cr, &mut cb, &mut y_even];
    for (chan_idx, buf) in buffers.into_iter().enumerate() {
        decode_one_channel_into(
            buf,
            chan_starts_sec[chan_idx],
            chan_bounds_abs[chan_idx],
            spec,
            audio,
            skip_samples,
            pair_seconds,
            rate_hz,
            demod,
            snr_est,
            hedr_shift_hz,
        );
    }

    for x in 0..width_us {
        let rgb_odd = ycbcr_to_rgb(y_odd[x], cr[x], cb[x]);
        let rgb_even = ycbcr_to_rgb(y_even[x], cr[x], cb[x]);
        image.put_pixel(x as u32, row0, rgb_odd);
        image.put_pixel(x as u32, row1, rgb_even);
    }
}

/// Decode one PD channel (`Y_odd`, `Cr`, `Cb`, or `Y_even`) into `out`.
///
/// Implements slowrx's per-pixel demod inner loop (`video.c:259-410`)
/// for a single channel: an FFT every [`PIXEL_FFT_STRIDE`] samples
/// produces the most-recent `Freq`, which fills `StoredLum` at every
/// sample. Pixel times read out of `StoredLum`. SNR is re-estimated
/// every [`SNR_REESTIMATE_STRIDE`] samples and feeds the SNR-adaptive
/// Hann window selector with 1 dB hysteresis (see
/// [`decode_pd_line_pair`]'s `#44 lifted with hysteresis (0.3.2)` note
/// and [`crate::demod::window_idx_for_snr_with_hysteresis`]).
///
/// `chan_bounds_abs` is `(start_abs, end_abs)` of the channel in the
/// audio stream. It is accepted for API compatibility but currently
/// unused: the FFT windowed support reads `audio` directly across
/// channel boundaries to match slowrx C. See [`decode_pd_line_pair`]'s
/// `#32 lifted via #45` deviation note for the rationale.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
pub(crate) fn decode_one_channel_into(
    out: &mut [u8],
    chan_start_sec: f64,
    chan_bounds_abs: (i64, i64),
    spec: crate::modespec::ModeSpec,
    audio: &[f32],
    skip_samples: i64,
    time_offset_seconds: f64,
    rate_hz: f64,
    demod: &mut ChannelDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    let pixel_secs = spec.pixel_seconds;
    let width = spec.line_pixels as usize;

    // Pixel sample times (slowrx video.c:140-142) — absolute audio indices.
    // SINGLE `round()` over `(time_offset_seconds + chan_start_sec + (x +
    // 0.5) * pixel_secs) * rate`; matches slowrx exactly.
    //
    // `time_offset_seconds` is the time-base offset of the radio frame the
    // caller is decoding: PD passes `pair_index * line_seconds`; Robot
    // passes `line_index * line_seconds`. The helper itself is mode-
    // agnostic — it only sees an additive seconds offset folded inside
    // the single `round()`.
    let mut pixel_times: Vec<i64> = Vec::with_capacity(width);
    for x in 0..width {
        let secs_in_frame = chan_start_sec + pixel_secs * (x as f64 + 0.5);
        let abs = skip_samples + ((time_offset_seconds + secs_in_frame) * rate_hz).round() as i64;
        pixel_times.push(abs);
    }

    let first_time = pixel_times[0];
    let last_time = pixel_times[width - 1];
    let half_fft = (FFT_LEN as i64) / 2;
    let sweep_start = first_time - half_fft;
    let sweep_end = last_time + half_fft + 1;

    let sweep_len = (sweep_end - sweep_start).max(0) as usize;
    let mut stored_lum = vec![0_u8; sweep_len];

    // SNR is sticky across the sweep; slowrx initializes with `SNR = 0`
    // (`video.c:36`) so the first `WinIdx` lookup uses index 4 (slowrx C's
    // 256-sample Hann window; equivalently `HANN_LENS[4] = 64` samples
    // in slowrx.rs at our 11_025 Hz working rate, applied inside a
    // [`FFT_LEN`] = 1024 FFT with the rest zero-padded).
    let mut snr_db = 0.0_f64;
    let mut current_freq = 1500.0_f64 + hedr_shift_hz;

    // Per-channel local state for SNR-adaptive Hann selection. The
    // initial value is the last index of `crate::demod::HANN_LENS` —
    // i.e. the longest, most-noise-rejecting window — which is the
    // conservative cold-start default. The hysteresis selector
    // ratchets one band per FFT toward `window_idx_for_snr(snr_db)`,
    // so with `snr_db = 0.0` (baseline idx 4 — matching slowrx's
    // pure-threshold value at SNR=0.0: ≥ -5 → 4) the cold-start
    // convergence is 6 → 5 → 4 across the first two FFTs. Once
    // `snr_db` updates from `SNR_REESTIMATE_STRIDE` the selector
    // tracks the actual SNR with the same one-band-per-call ratchet.
    let mut prev_win_idx = crate::demod::HANN_LENS.len() - 1;

    // Read absolute audio with no channel-boundary mask. slowrx FFTs
    // across channel boundaries (`video.c::GetVideo`); the peak search in
    // 1500-2300 Hz still locks onto the dominant video tone even when
    // adjacent channels' content leaks into the windowed FFT support.
    // The previous channel-bounded mask (#45) hurt the leftmost/rightmost
    // ~60 pixels of every channel on real radio — verified against
    // Dec-2017 ARISS captures where the masked decode showed visible
    // vertical banding at every channel edge.
    let _ = chan_bounds_abs;

    let read_audio = |abs_idx: i64| -> f32 {
        if abs_idx >= 0 && (abs_idx as usize) < audio.len() {
            audio[abs_idx as usize]
        } else {
            0.0
        }
    };

    // Pre-fill a scratch buffer the FFT can index linearly. Cheaper than
    // copying `audio[…]` per sample inside the inner loop.
    let scratch_audio: Vec<f32> = (sweep_start..sweep_end).map(read_audio).collect();

    let mod_round = |s: i64, stride: i64| -> i64 { s.rem_euclid(stride) };

    for s in sweep_start..sweep_end {
        if mod_round(s, SNR_REESTIMATE_STRIDE) == 0 {
            // SNR estimator reads the absolute audio (across channels);
            // we want the SNR of the entire signal, not just this channel.
            snr_db = snr_est.estimate(audio, s, hedr_shift_hz);
        }

        if mod_round(s, PIXEL_FFT_STRIDE) == 0 {
            // SNR-adaptive Hann window length WITH 1 dB hysteresis band.
            // The bare `window_idx_for_snr` function flip-flops at threshold
            // boundaries when real-radio SNR fluctuates ~0.5 dB across the
            // SNR re-estimation cadence (5.8 ms = ~21 R36-Y pixels) — that
            // produced the vertical squiggle artifact in V2.2's Fram2 output
            // (#71). The hysteresis variant requires SNR to move past the
            // threshold by ≥ 0.5 dB in the direction of the new index before
            // accepting a change. See
            // `crate::demod::window_idx_for_snr_with_hysteresis` and the
            // 0.3.2 entry in `docs/intentional-deviations.md`.
            let mut win_idx =
                crate::demod::window_idx_for_snr_with_hysteresis(snr_db, prev_win_idx);
            prev_win_idx = win_idx;
            // slowrx C video.c:367 — Scottie DX bumps WinIdx up by one when not
            // already at saturation, giving SDX's 1.08 ms/pixel a longer
            // integration window. Applied AFTER the hysteresis selector so
            // `prev_win_idx` continues tracking the un-bumped SNR-derived index
            // (the bump shouldn't compound across pixels).
            if spec.mode == crate::modespec::SstvMode::ScottieDx
                && win_idx < crate::demod::HANN_LENS.len() - 1
            {
                win_idx += 1;
            }
            let center_in_scratch = s - sweep_start;
            current_freq =
                demod.pixel_freq(&scratch_audio, center_in_scratch, hedr_shift_hz, win_idx);
        }

        let lum = freq_to_luminance(current_freq, hedr_shift_hz);
        let idx = (s - sweep_start) as usize;
        if idx < stored_lum.len() {
            stored_lum[idx] = lum;
        }
    }

    for x in 0..width {
        let pixel_time = pixel_times[x];
        let rel = pixel_time - sweep_start;
        let lum = if rel >= 0 && (rel as usize) < stored_lum.len() {
            stored_lum[rel as usize]
        } else {
            0
        };
        out[x] = lum;
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
mod tests {
    /// Verify that with `septr_seconds = 0` the `chan_starts_sec` formula
    /// gives the same values as the pre-#25 direct formula (numeric equivalence).
    /// This confirms the field is a V2 expansion that is a no-op for PD modes.
    #[test]
    fn chan_starts_sec_septr_zero_is_numerically_equivalent_to_old_formula() {
        for spec in [
            crate::modespec::for_mode(crate::modespec::SstvMode::Pd120),
            crate::modespec::for_mode(crate::modespec::SstvMode::Pd180),
            crate::modespec::for_mode(crate::modespec::SstvMode::Pd240),
        ] {
            let sync = spec.sync_seconds;
            let porch = spec.porch_seconds;
            let px = spec.pixel_seconds;
            let septr = spec.septr_seconds;
            let w = f64::from(spec.line_pixels);
            let chan_len = w * px;

            // New formula (with septr_seconds term from slowrx video.c:88-92)
            let new_starts = [
                sync + porch,
                sync + porch + chan_len + septr,
                sync + porch + 2.0 * chan_len + 2.0 * septr,
                sync + porch + 3.0 * chan_len + 3.0 * septr,
            ];
            // Old formula (pre-#25, septr omitted)
            let old_starts = [
                sync + porch,
                sync + porch + w * px,
                sync + porch + 2.0 * w * px,
                sync + porch + 3.0 * w * px,
            ];
            for (n, (n_val, o_val)) in new_starts.iter().zip(old_starts.iter()).enumerate() {
                assert!(
                    (n_val - o_val).abs() < 1e-12,
                    "mode {:?} chan {} new={n_val} old={o_val}",
                    spec.mode,
                    n
                );
            }
        }
    }
}
