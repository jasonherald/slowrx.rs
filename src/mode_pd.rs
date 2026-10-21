//! PD-family mode decoder.
//!
//! PD modes encode each radio "frame" as four channels — Y(odd line),
//! Cr (shared), Cb (shared), Y(even line) — producing two image rows
//! per radio frame with chroma subsampling between them.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Channel layout: video.c lines 81-93. YUV→RGB matrix: video.c lines
//! 446-451. See `NOTICE.md` for full attribution.
//!
//! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
//! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
//! verified at audit #94 (2026-05-15).

use crate::decoder::ModeDecoder;
use crate::demod::{decode_one_channel_into, ycbcr_to_rgb, ChannelDecodeCtx, DemodState};
use crate::image::SstvImage;
use crate::modespec::ModeSpec;

/// [`ModeDecoder`] for [`crate::modespec::ChannelLayout::PdYcbcr`]
/// (PD120 / PD180 / PD240): each radio line is a line pair that writes two
/// image rows ([`decode_pd_line_pair`]).
#[derive(Debug)]
pub(crate) struct PdDecoder {
    image_lines: u32,
}

impl PdDecoder {
    /// The decoder for one image of PD mode `spec`.
    pub(crate) fn new(spec: ModeSpec) -> Self {
        Self {
            image_lines: spec.image_lines,
        }
    }
}

impl ModeDecoder for PdDecoder {
    /// Half the image rows (slowrx `video.c:251-254`); every PD mode's
    /// `image_lines` is even (pinned by a `modespec` test).
    fn radio_lines(&self) -> u32 {
        self.image_lines / 2
    }

    fn image_rows(&self, index: u32) -> std::ops::Range<u32> {
        index * 2..index * 2 + 2
    }

    fn decode_radio_line(
        &mut self,
        ctx: &ChannelDecodeCtx<'_>,
        index: u32,
        image: &mut SstvImage,
        state: &mut DemodState<'_>,
    ) {
        decode_pd_line_pair(ctx, index, image, state);
    }
}

/// Decode PD radio frame `pair_index` (`Y(odd)`/`Cr`/`Cb`/`Y(even)`) into
/// image rows `2·pair_index` and `2·pair_index + 1`. Translated from slowrx
/// `video.c:259-486`.
///
/// Every pixel uses slowrx's exact
/// `Skip + round(rate * (chan_start_sec + pixel_secs * (x + 0.5)))`
/// single-round formula (`video.c:140-142`); `pair_seconds` is folded
/// in there, NOT pre-rounded, so per-pair rounding error never
/// accumulates.
///
/// One FFT per working-rate sample produces the latest `Freq`, which
/// fills `StoredLum` at every sample; pixel times read out via
/// `StoredLum[pixel_time]` (`video.c:350-406`). See
/// [`crate::demod::decode_one_channel_into`] for the per-channel
/// inner loop. [`crate::snr::SnrEstimator`] is recomputed every
/// [`crate::demod::SNR_REESTIMATE_STRIDE`] samples (`video.c:302-343`).
///
/// The per-pixel Hann window length is SNR-adaptive, as in slowrx
/// (`video.c:354-367`; [`crate::demod::window_idx_for_snr`]).
///
/// `ctx.skip_samples` is the absolute sample index inside `ctx.audio`
/// where pair zero's sync pulse begins; `pair_seconds` is `pair_index *
/// line_seconds` (un-rounded); `ctx.rate_hz` is the slant-corrected rate
/// from [`crate::sync::find_sync`].
// HISTORY (audit #94 E12 — issue archaeology moved out of rustdoc):
//   #18 — SNR estimator: SnrEstimator recomputed every
//          SNR_REESTIMATE_STRIDE samples (video.c:302-343; closed).
//   #23 — FFT-every-N + StoredLum: one FFT per working-rate sample,
//          pixel times via StoredLum[pixel_time] (video.c:350-406; closed).
//   #24 — time-base alignment: single-round formula, pair_seconds folded
//          in here rather than pre-rounded by caller (closed).
//   #44 — SNR-adaptive Hann window lifted with 1 dB hysteresis band
//          (0.3.2 deviation); hysteresis removed in #130, back to
//          slowrx's pure thresholds (see intentional-deviations.md).
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn decode_pd_line_pair(
    ctx: &ChannelDecodeCtx<'_>,
    pair_index: u32,
    image: &mut SstvImage,
    state: &mut DemodState<'_>,
) {
    let spec = ctx.spec;
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;

    // slowrx `video.c:140-142` computes pixel time as
    // `Skip + round(Rate * (y/2 * LineTime + ChanStart +
    // PixelTime * (x + 0.5)))`. Compute `pair_seconds = y/2 *
    // LineTime` here (un-rounded) and let
    // [`crate::demod::decode_one_channel_into`] fold it into its
    // own `round()`, so per-pair rounding error never
    // accumulates.
    let pair_seconds = f64::from(pair_index) * spec.line_seconds;

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

    // Borrow the hoisted line buffers (Audit #93 D3) so `state.demod` can be
    // mutably borrowed inside decode_one_channel_into; they go back to the
    // demod after the pair, keeping their capacity.
    let mut bufs = state.demod.take_line_bufs(width_us);
    let [y_odd, cr, cb, y_even] = &mut bufs;

    let buffers: [&mut [u8]; 4] = [y_odd, cr, cb, y_even];
    for (chan_idx, buf) in buffers.into_iter().enumerate() {
        decode_one_channel_into(buf, chan_starts_sec[chan_idx], pair_seconds, ctx, state);
    }

    for x in 0..width_us {
        let rgb_odd = ycbcr_to_rgb(y_odd[x], cr[x], cb[x]);
        let rgb_even = ycbcr_to_rgb(y_even[x], cr[x], cb[x]);
        image.put_pixel(x as u32, row0, rgb_odd);
        image.put_pixel(x as u32, row1, rgb_even);
    }

    state.demod.restore_line_bufs(bufs);
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
