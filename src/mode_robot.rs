//! Robot-family mode decoder.
//!
//! Robot 24 / Robot 36: 2-channel layout per radio line — Y (full width,
//! 2× pixel-time per channel allocation) followed by alternating Cr/Cb
//! (Cr on even-indexed Y rows, Cb on odd, each chroma sample duplicated
//! to the neighbor image row). Robot 72: 3-channel layout per radio
//! line — Y, U, V each at full pixel-time. All three share
//! [`crate::modespec::ChannelLayout::RobotYuv`] in the public API; the
//! per-mode case split mirrors slowrx `video.c:60-101` (channel-time
//! switch) and `:104-130` (`NumChans` switch).
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Per-mode chroma layout: video.c lines 60-101, 178-209, 421-425.
//! YUV→RGB matrix: video.c lines 446-451 (shared with PD; we re-use
//! `mode_pd::ycbcr_to_rgb`). See `NOTICE.md` for full attribution.

use crate::modespec::{ModeSpec, SstvMode};

/// Decode one Robot radio line into `image`. The R24/R36 path also
/// writes duplicated chroma to the neighbor image row (with bounds
/// guard) per slowrx `video.c:424-425`.
///
/// `line_index` is the 0-based image row this radio line emits Y for;
/// `line_seconds_offset` is `f64::from(line_index) * spec.line_seconds`
/// (un-rounded — the per-pixel time computation does the single
/// `round()` to match slowrx `video.c:140-142`).
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
pub(crate) fn decode_line(
    spec: ModeSpec,
    mode: SstvMode,
    line_index: u32,
    audio: &[f32],
    skip_samples: i64,
    line_seconds_offset: f64,
    rate_hz: f64,
    image: &mut crate::image::SstvImage,
    demod: &mut crate::mode_pd::PdDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    match mode {
        SstvMode::Robot72 => decode_r72_line(
            spec,
            line_index,
            audio,
            skip_samples,
            line_seconds_offset,
            rate_hz,
            image,
            demod,
            snr_est,
            hedr_shift_hz,
        ),
        SstvMode::Robot24 | SstvMode::Robot36 => {
            // Filled in V2.2 Phase 4 (R36/R24 chroma-planes plumbing).
            unimplemented!("R36/R24 decoder lands in V2.2 Phase 4")
        }
        _ => unreachable!("decode_line must be called with a Robot variant"),
    }
}

/// Decode one R72 radio line into image[`line_index`].
///
/// R72 channel layout per slowrx `video.c:95-100` (default case for
/// non-PD/non-Scottie/non-Robot-alt modes):
///   `ChanLen[0..3]` = `pixel_seconds` * width   for each of Y, U, V
///   `ChanStart[0]` = sync + porch
///   `ChanStart[1]` = `ChanStart[0]` + `chan_len` + septr
///   `ChanStart[2]` = `ChanStart[1]` + `chan_len` + septr
///
/// Reuses [`crate::mode_pd::decode_one_channel_into`] for per-channel
/// FFT-based demod — that helper is mode-agnostic (reads
/// `pixel_seconds` from `spec`, walks audio slice between channel
/// bounds, fills a `&mut [u8]`).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
fn decode_r72_line(
    spec: ModeSpec,
    line_index: u32,
    audio: &[f32],
    skip_samples: i64,
    line_seconds_offset: f64,
    rate_hz: f64,
    image: &mut crate::image::SstvImage,
    demod: &mut crate::mode_pd::PdDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;
    let chan_len = f64::from(width) * pixel_secs;

    // R72 channel start times — translated from slowrx video.c:95-100
    // (default case; R72 falls here, not in the named PD/Scottie/R36/R24
    // cases).
    let chan_starts_sec = [
        sync_secs + porch_secs,                                     // Y
        sync_secs + porch_secs + chan_len + septr_secs,             // U (Cr)
        sync_secs + porch_secs + 2.0 * chan_len + 2.0 * septr_secs, // V (Cb)
    ];

    let width_us = width as usize;

    // Channel sample-range bounds, computed once with a single
    // `round()` per bound (matches slowrx video.c:140-142 to avoid
    // per-pair rounding drift).
    let chan_bounds_abs: [(i64, i64); 3] = std::array::from_fn(|i| {
        let start_sec = chan_starts_sec[i];
        let end_sec = start_sec + chan_len;
        let start_abs = skip_samples + ((line_seconds_offset + start_sec) * rate_hz).round() as i64;
        let end_abs = skip_samples + ((line_seconds_offset + end_sec) * rate_hz).round() as i64;
        (start_abs, end_abs)
    });

    let mut y = vec![0_u8; width_us];
    let mut cr = vec![0_u8; width_us];
    let mut cb = vec![0_u8; width_us];

    let buffers: [&mut [u8]; 3] = [&mut y, &mut cr, &mut cb];
    for (chan_idx, buf) in buffers.into_iter().enumerate() {
        crate::mode_pd::decode_one_channel_into(
            buf,
            chan_starts_sec[chan_idx],
            chan_bounds_abs[chan_idx],
            spec,
            audio,
            skip_samples,
            line_seconds_offset,
            rate_hz,
            demod,
            snr_est,
            hedr_shift_hz,
        );
    }

    for x in 0..width_us {
        let rgb = crate::mode_pd::ycbcr_to_rgb(y[x], cr[x], cb[x]);
        image.put_pixel(x as u32, line_index, rgb);
    }
}
