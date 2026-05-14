//! PD-family mode decoder.
//!
//! PD modes encode each radio "frame" as four channels — Y(odd line),
//! Cr (shared), Cb (shared), Y(even line) — producing two image rows
//! per radio frame with chroma subsampling between them.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Channel layout: video.c lines 81-93. YUV→RGB matrix: video.c lines
//! 446-451. See `NOTICE.md` for full attribution.

use rustfft::{num_complex::Complex, FftPlanner};

/// Map a demodulated FM frequency (Hz) to an 8-bit luminance value.
///
/// SSTV video lives in 1500–2300 Hz: 1500 Hz = black (0), 2300 Hz = white (255).
/// Linear scaling: `lum = (freq - 1500) / (2300 - 1500) * 255`.
/// Out-of-band frequencies are clamped. `hedr_shift_hz` shifts the band
/// to compensate for radio mistuning detected at VIS time:
/// `lum = (freq - 1500 - hedr_shift_hz) / 3.1372549`.
///
/// Translated from slowrx's `video.c:406` + `common.c:49-53`:
/// `StoredLum[SampleNum] = clip((Freq - 1500 - HedrShift) / 3.1372549);`
/// where `clip(a)` returns `(guchar)round(a)` clamped to \[0, 255\].
/// The `round()` call means values like 127.7 map to 128, not 127.
///
/// `3.1372549 = (2300 - 1500) / 255`.
#[must_use]
pub(crate) fn freq_to_luminance(freq_hz: f64, hedr_shift_hz: f64) -> u8 {
    let v = (freq_hz - 1500.0 - hedr_shift_hz) / 3.137_254_9;
    // Round-to-nearest before casting, matching slowrx's `(guchar)round(a)`
    // in `common.c::clip()`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lum = v.clamp(0.0, 255.0).round() as u8;
    lum
}

/// Convert a single PD-family `[Y, Cr, Cb]` triple to `[R, G, B]`.
///
/// Translated from slowrx's `video.c:447-450` + `common.c:49-53`:
/// ```text
/// R = clip((100*Y + 140*Cr - 17850) / 100.0)
/// G = clip((100*Y -  71*Cr -  33*Cb + 13260) / 100.0)
/// B = clip((100*Y + 178*Cb - 22695) / 100.0)
/// ```
/// where `clip(a)` is `(guchar)round(a)` clamped to \[0, 255\] (`common.c:49-53`).
///
/// slowrx uses `/ 100.0` (float division), which produces a `double` that is
/// then **rounded** by `clip()` before clamping. The previous implementation
/// used integer division (`/ 100`), which **truncates toward zero** — this
/// produced a 1-LSB darker bias on R and B channels for neutral grey and many
/// other combinations. Fixed in round-2 audit Finding 1.
#[must_use]
#[doc(hidden)]
pub fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3] {
    let yi = f64::from(y);
    let cri = f64::from(cr);
    let cbi = f64::from(cb);
    // Float divide then round, matching slowrx's `clip(double)` in common.c:49-53:
    //   `return (guchar)round(a)` after clamping to [0, 255].
    // i32 intermediate magnitudes are well within f64 precision (max 255*178=45390).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((100.0 * yi + 140.0 * cri - 17_850.0) / 100.0)
        .clamp(0.0, 255.0)
        .round() as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let g = ((100.0 * yi - 71.0 * cri - 33.0 * cbi + 13_260.0) / 100.0)
        .clamp(0.0, 255.0)
        .round() as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((100.0 * yi + 178.0 * cbi - 22_695.0) / 100.0)
        .clamp(0.0, 255.0)
        .round() as u8;
    [r, g, b]
}

/// FFT length used for per-pixel demod. Matches slowrx's bin spacing:
/// `1024/11025 Hz` ≈ 10.77 Hz/bin — 4× finer than slowrx's `1024/44100 Hz`
/// = 43.07 Hz/bin (deliberate 0.3.3 divergence; see
/// `docs/intentional-deviations.md`).
pub(crate) const FFT_LEN: usize = crate::snr::FFT_LEN;

/// Per-pixel demod context: holds an FFT plan + reusable buffers + the
/// adaptive Hann-window bank. Construct once per decoder; reuse for many
/// [`PdDemod::pixel_freq`] calls.
pub(crate) struct PdDemod {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    hann_bank: crate::snr::HannBank,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl PdDemod {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann_bank: crate::snr::HannBank::new(),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
        }
    }

    /// Estimate the dominant tone frequency in a Hann-windowed FFT
    /// centered near `center_sample`. `win_idx` selects the Hann length
    /// from [`crate::snr::HANN_LENS`]; the FFT length stays fixed at
    /// [`FFT_LEN`] (= 1024), with leading/trailing zero-pad. Out-of-bounds
    /// `audio` reads as silence. `hedr_shift_hz` shifts the peak-search
    /// range from `[1500, 2300]` Hz to `[1500 + hedr, 2300 + hedr]` Hz to
    /// follow a mistuned radio's pixel band.
    ///
    /// Translated from slowrx `video.c:369-395` (windowed FFT + bin
    /// search + Gaussian interp).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn pixel_freq(
        &mut self,
        audio: &[f32],
        center_sample: i64,
        hedr_shift_hz: f64,
        win_idx: usize,
    ) -> f64 {
        let win_idx = win_idx.min(crate::snr::HANN_LENS.len() - 1);
        let hann = self.hann_bank.get(win_idx);
        let win_len = hann.len();

        // Zero-fill, then apply the windowed support centered on
        // `center_sample`. slowrx (`video.c:369-375`) writes the
        // `WinLength` windowed samples into the FIRST `WinLength` bins
        // of the FFT input; the magnitude spectrum is invariant to that
        // offset (just a phase rotation).
        for c in &mut self.fft_buf {
            *c = Complex { re: 0.0, im: 0.0 };
        }
        let half = (win_len as i64) / 2;
        for (i, (&w, dst)) in hann.iter().zip(self.fft_buf.iter_mut()).enumerate() {
            let idx = center_sample - half + i as i64;
            let s = if idx >= 0 && (idx as usize) < audio.len() {
                audio[idx as usize]
            } else {
                0.0
            };
            *dst = Complex { re: s * w, im: 0.0 };
        }

        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        // Search peak in bins corresponding to (1500+HedrShift)..(2300+HedrShift) Hz.
        // Use slowrx-equivalent truncation (not `.round()`) via `crate::dsp::get_bin`.
        // See `crate::dsp::get_bin` for rationale.
        let bin_for = |hz: f64| -> usize {
            crate::dsp::get_bin(hz, FFT_LEN, crate::resample::WORKING_SAMPLE_RATE_HZ)
        };
        let lo = bin_for(1500.0 + hedr_shift_hz).saturating_sub(1).max(1);
        let hi = bin_for(2300.0 + hedr_shift_hz)
            .saturating_add(1)
            .min(FFT_LEN / 2 - 1);

        let mut max_bin = lo;
        let mut max_p = crate::dsp::power(self.fft_buf[lo]);
        for (k, &c) in self.fft_buf.iter().enumerate().take(hi + 1).skip(lo + 1) {
            let p = crate::dsp::power(c);
            if p > max_p {
                max_p = p;
                max_bin = k;
            }
        }

        // Boundary clip + Gaussian-log peak interpolation (slowrx video.c:389-398).
        //
        // slowrx's guard (`video.c:390`):
        //   if (MaxBin > GetBin(1500+HedrShift) - 1 &&
        //       MaxBin < GetBin(2300+HedrShift) + 1) { interpolate }
        //   else { Freq = (MaxBin > GetBin(1900+HedrShift)) ? 2300 : 1500 + HedrShift; }
        //
        // `lo` = GetBin(1500+hedr) - 1 and `hi` = GetBin(2300+hedr) + 1 (as above),
        // so the guard translates to: `max_bin > lo && max_bin < hi`. When the peak
        // lands on one of the padded boundary bins, slowrx returns a hard-clipped
        // value rather than interpolating into the neighbor noise bin (round-2 audit
        // Finding 9).
        let mid_bin = bin_for(1900.0 + hedr_shift_hz);
        if max_bin <= lo || max_bin >= hi {
            // Clip to band edge (slowrx video.c:397).
            let clipped_hz = if max_bin > mid_bin {
                2300.0 + hedr_shift_hz
            } else {
                1500.0 + hedr_shift_hz
            };
            return clipped_hz;
        }

        // Freq_bin = MaxBin + log(P[k+1]/P[k-1]) / (2 * log(P[k]^2 / (P[k+1] * P[k-1])))
        let p_prev = crate::dsp::power(self.fft_buf[max_bin - 1]);
        let p_curr = max_p;
        let p_next = crate::dsp::power(self.fft_buf[max_bin + 1]);

        // If any neighbor power is non-positive, skip interpolation
        // (log of zero blows up). slowrx falls back to a clipped centre.
        let interp_ok = p_prev > 0.0 && p_curr > 0.0 && p_next > 0.0;
        let freq_bin = if interp_ok {
            let num = (p_next / p_prev).ln();
            let denom = 2.0 * (p_curr * p_curr / (p_next * p_prev)).ln();
            if denom.abs() > 1e-12 {
                (max_bin as f64) + num / denom
            } else {
                max_bin as f64
            }
        } else {
            max_bin as f64
        };

        freq_bin * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ) / (FFT_LEN as f64)
    }
}

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

/// Distance, in working-rate samples, between successive SNR
/// re-estimates. slowrx re-estimates every 256 samples at `44_100` Hz
/// (`video.c:343`); scaled to `11_025` Hz that's 64.
pub(crate) const SNR_REESTIMATE_STRIDE: i64 = 64;

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
///   [`crate::snr::window_idx_for_snr_with_hysteresis`] and the
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
    demod: &mut PdDemod,
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
/// and [`crate::snr::window_idx_for_snr_with_hysteresis`]).
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
    demod: &mut PdDemod,
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
    // initial value is the last index of `crate::snr::HANN_LENS` —
    // i.e. the longest, most-noise-rejecting window — which is the
    // conservative cold-start default. The hysteresis selector
    // ratchets one band per FFT toward `window_idx_for_snr(snr_db)`,
    // so with `snr_db = 0.0` (baseline idx 4 — matching slowrx's
    // pure-threshold value at SNR=0.0: ≥ -5 → 4) the cold-start
    // convergence is 6 → 5 → 4 across the first two FFTs. Once
    // `snr_db` updates from `SNR_REESTIMATE_STRIDE` the selector
    // tracks the actual SNR with the same one-band-per-call ratchet.
    let mut prev_win_idx = crate::snr::HANN_LENS.len() - 1;

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
            // `crate::snr::window_idx_for_snr_with_hysteresis` and the
            // 0.3.2 entry in `docs/intentional-deviations.md`.
            let mut win_idx = crate::snr::window_idx_for_snr_with_hysteresis(snr_db, prev_win_idx);
            prev_win_idx = win_idx;
            // slowrx C video.c:367 — Scottie DX bumps WinIdx up by one when not
            // already at saturation, giving SDX's 1.08 ms/pixel a longer
            // integration window. Applied AFTER the hysteresis selector so
            // `prev_win_idx` continues tracking the un-bumped SNR-derived index
            // (the bump shouldn't compound across pixels).
            if spec.mode == crate::modespec::SstvMode::ScottieDx
                && win_idx < crate::snr::HANN_LENS.len() - 1
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
    use super::*;

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

    #[test]
    fn freq_1500_is_black() {
        assert_eq!(freq_to_luminance(1500.0, 0.0), 0);
    }

    #[test]
    fn freq_2300_is_white() {
        assert_eq!(freq_to_luminance(2300.0, 0.0), 255);
    }

    #[test]
    fn freq_below_band_clamps_to_zero() {
        assert_eq!(freq_to_luminance(1000.0, 0.0), 0);
        assert_eq!(freq_to_luminance(0.0, 0.0), 0);
    }

    #[test]
    fn freq_above_band_clamps_to_max() {
        assert_eq!(freq_to_luminance(3000.0, 0.0), 255);
    }

    #[test]
    fn freq_midband_is_midgrey() {
        // 1900 Hz: v = (1900 - 1500) / 3.1372549 ≈ 127.5 → rounds to 128
        let v = freq_to_luminance(1900.0, 0.0);
        assert!(
            (i32::from(v) - 128).abs() <= 1,
            "midband should be ~128 after rounding, got {v}"
        );
    }

    #[test]
    fn freq_to_luminance_rounds_to_nearest_not_truncates() {
        // Slowrx uses `(guchar)round(a)` in common.c::clip(), not truncation.
        // Solve (freq - 1500) / 3.1372549 = 127.7 → freq ≈ 1900.6294 Hz.
        // round(127.7) = 128, not 127.
        let freq = 1500.0 + 127.7 * 3.137_254_9; // ≈ 1900.629...
        assert_eq!(
            freq_to_luminance(freq, 0.0),
            128,
            "127.7 should round to 128, not truncate to 127"
        );
    }

    #[test]
    fn freq_to_luminance_with_hedr_shift_scales_band() {
        // With +50 Hz HedrShift, the band shifts to 1550..2350.
        // freq=1550 should be black, freq=2350 should be white.
        assert_eq!(freq_to_luminance(1550.0, 50.0), 0);
        assert_eq!(freq_to_luminance(2350.0, 50.0), 255);
        // 1500 (would be black at zero shift) is now sub-band, still clamps to 0.
        assert_eq!(freq_to_luminance(1500.0, 50.0), 0);
        // 1950 with +50 shift is the new midband ≈ same as 1900 with 0 shift.
        let a = freq_to_luminance(1950.0, 50.0);
        let b = freq_to_luminance(1900.0, 0.0);
        assert!(
            i32::from(a).abs_diff(i32::from(b)) <= 1,
            "shifted midband {a} vs unshifted {b}"
        );
    }

    #[test]
    fn ycbcr_neutral_grey_is_grey() {
        // Y=128, Cr=128, Cb=128 → neutral grey.
        // Exact values (slowrx float-divide + round):
        //   R = (100*128 + 140*128 - 17850) / 100.0 = 12870/100.0 = 128.7 → round → 129
        //   G = (100*128 -  71*128 -  33*128 + 13260)/100.0 = 12748/100.0 = 127.48 → round → 127
        //   B = (100*128 + 178*128 - 22695)/100.0 = 12889/100.0 = 128.89 → round → 129
        let rgb = ycbcr_to_rgb(128, 128, 128);
        for ch in &rgb {
            assert!((i32::from(*ch) - 128).abs() <= 2, "got {rgb:?}");
        }
    }

    /// Verify round-to-nearest parity with slowrx's `clip()` (`common.c:49-53`).
    ///
    /// slowrx uses `clip((100*Y + 140*Cr - 17850) / 100.0)` where `clip` calls
    /// `round()`. Y=128, Cr=128, Cb=128 produces:
    ///   R numerator = 12870 → 128.70 → round → **129** (not 128 from integer division)
    ///   B numerator = 12889 → 128.89 → round → **129** (not 128 from integer division)
    ///
    /// This test would fail with the old integer-division implementation.
    #[test]
    fn ycbcr_rounds_to_nearest_matching_slowrx_clip() {
        let [r, g, b] = ycbcr_to_rgb(128, 128, 128);
        assert_eq!(
            r, 129,
            "R should be 129 (round(128.70)), not 128 (truncate)"
        );
        assert_eq!(g, 127, "G should be 127 (round(127.48))");
        assert_eq!(
            b, 129,
            "B should be 129 (round(128.89)), not 128 (truncate)"
        );
    }

    #[test]
    fn ycbcr_pure_red() {
        // Roughly: max Cr, mid Y, mid Cb → strong red.
        let rgb = ycbcr_to_rgb(76, 255, 85);
        assert!(rgb[0] > 200, "red channel should dominate, got {rgb:?}");
        assert!(rgb[2] < 100);
    }

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

    /// Default `win_idx` for tests: index 4 (Hann length 64 at our
    /// `11_025` Hz working rate), slowrx's middle-of-the-band default.
    const DEFAULT_WIN_IDX: usize = 4;

    #[test]
    fn pdfft_recovers_known_tone_within_5hz() {
        let mut d = PdDemod::new();
        // 100 ms of pure tone at 1900 Hz; ample for FFT_LEN=1024.
        let audio = synth_tone(1900.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est = d.pixel_freq(&audio, center, 0.0, DEFAULT_WIN_IDX);
        assert!((est - 1900.0).abs() < 5.0, "expected ≈1900, got {est}");
    }

    #[test]
    fn pdfft_recovers_band_edges() {
        let mut d = PdDemod::new();
        for &f in &[1500.0_f64, 1700.0, 2100.0, 2300.0] {
            let audio = synth_tone(f, 0.100);
            let center = (audio.len() / 2) as i64;
            let est = d.pixel_freq(&audio, center, 0.0, DEFAULT_WIN_IDX);
            assert!(
                (est - f).abs() < 8.0,
                "f={f} estimate={est} (band edge precision)"
            );
        }
    }

    #[test]
    fn pdfft_returns_finite_for_silence() {
        let mut d = PdDemod::new();
        let audio = vec![0.0_f32; 1024];
        let est = d.pixel_freq(&audio, 512, 0.0, DEFAULT_WIN_IDX);
        assert!(est.is_finite(), "got {est}");
        // Silence has no peak; the search expands by ±1 bin around the
        // 1500-2300 band, and bin width is ~43 Hz, so the fallback may
        // land within ~50 Hz of either edge.
        assert!((1450.0..=2350.0).contains(&est), "out of band: {est}");
    }

    #[test]
    fn pixel_freq_with_hedr_shift() {
        // A 1950 Hz tone with hedr_shift=+50 should yield the same luminance
        // as a 1900 Hz tone with hedr_shift=0, since the search band shifts.
        let mut d = PdDemod::new();
        let audio = synth_tone(1950.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est_shifted = d.pixel_freq(&audio, center, 50.0, DEFAULT_WIN_IDX);
        let est_unshifted_baseline = {
            let baseline = synth_tone(1900.0, 0.100);
            d.pixel_freq(&baseline, (baseline.len() / 2) as i64, 0.0, DEFAULT_WIN_IDX)
        };
        // Tone-frequency itself is recovered correctly; what matters is that
        // the shifted estimator finds 1950, the unshifted finds 1900, and
        // mapping each to luminance gives ≈ same value.
        assert!((est_shifted - 1950.0).abs() < 5.0, "got {est_shifted}");
        let lum_shifted = freq_to_luminance(est_shifted, 50.0);
        let lum_unshifted = freq_to_luminance(est_unshifted_baseline, 0.0);
        assert!(
            i32::from(lum_shifted).abs_diff(i32::from(lum_unshifted)) <= 2,
            "lum_shifted={lum_shifted} lum_unshifted={lum_unshifted}"
        );
    }

    #[test]
    fn pixel_freq_clamps_out_of_range_win_idx() {
        // Defensive: an out-of-range win_idx must not panic.
        let mut d = PdDemod::new();
        let audio = synth_tone(1900.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est = d.pixel_freq(&audio, center, 0.0, 99);
        // Falls back to the longest window (idx 6) and still recovers the tone.
        assert!((est - 1900.0).abs() < 10.0, "clamp recover got {est}");
    }

    #[test]
    fn pixel_freq_short_window_still_recovers_tone() {
        // The shortest window (idx 0, length 12) is intended for high-SNR
        // signals. With a clean synthetic tone it should still localize the
        // peak inside the video band, just with coarser precision than a
        // long window.
        let mut d = PdDemod::new();
        let audio = synth_tone(1900.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est = d.pixel_freq(&audio, center, 0.0, 0);
        assert!(
            (1500.0..=2300.0).contains(&est),
            "short-window estimate {est} out of video band"
        );
    }

    /// Verify that a tone below the search band clips to 1500 Hz (round-2 audit
    /// Finding 9 — boundary clip matching slowrx `video.c:395-397`).
    ///
    /// At 1480 Hz the FFT peak lands on the padded boundary bin (lo) or below.
    /// slowrx's guard:
    ///   `if (MaxBin > GetBin(1500+HedrShift)-1 && MaxBin < GetBin(2300+HedrShift)+1)`
    /// fails, so it returns `1500 + HedrShift` (the lower clip).
    /// The estimate must be ≤ 1500 Hz and within one bin-width (~43 Hz) of 1500 Hz.
    #[test]
    fn pixel_freq_clips_below_band_to_1500hz() {
        let mut d = PdDemod::new();
        // 1480 Hz is 1 bin-width (~43 Hz) below the search range start (1500 Hz).
        // The FFT peak for this tone will land at or below the padded lo boundary.
        let audio = synth_tone(1480.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est = d.pixel_freq(&audio, center, 0.0, DEFAULT_WIN_IDX);
        // Must be clipped to ≈1500 Hz, not freely interpolated toward a DC neighbor.
        assert!(
            est <= 1500.0 + 50.0,
            "below-band tone should clip to ≈1500 Hz, got {est}"
        );
        assert!(
            est >= 1400.0,
            "clip floor should be near 1500 Hz, got {est}"
        );
    }
}
