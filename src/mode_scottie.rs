//! Scottie-family mode decoder (Scottie 1, Scottie 2, Scottie DX).
//!
//! Three-channel sequential RGB layout per radio line, with sync sitting
//! between the B and R channels (mid-line). This is the V2.3 mid-line-
//! sync forcing-function — `decoder.rs`'s line-clock advance and
//! `find_sync`'s detection both stay generic; the mid-line idiosyncrasy
//! lives entirely inside `decode_line`.
//!
//! Translated from slowrx's `video.c:72-79` (Scottie `ChanStart` layout)
//! and `video.c:367` (SDX Hann-bump). slowrx's GBR storage convention
//! (`video.c:440-444`) is bypassed: we write RGB directly via
//! [`crate::image::SstvImage::put_pixel`].
//!
//! ```text
//! Scottie radio line layout:
//!   [septr][ G pixels ][septr][ B pixels ][SYNC][porch][ R pixels ]
//!     ^                                       ^
//!     |                                       |
//!     line start                              find_sync detects this
//!                                             (mid-line)
//! ```
//!
//! See `NOTICE.md` for full slowrx attribution.

use crate::modespec::ModeSpec;

/// Decode one Scottie radio line into `image`. Reads G, B from before
/// the sync (negative offsets relative to `sync_time`) and R from after,
/// composing RGB in place.
///
/// `line_index` is the 0-based image row this radio line emits;
/// `line_seconds_offset` is `f64::from(line_index) * spec.line_seconds`
/// (un-rounded — the per-pixel time computation does the single
/// `round()` to match slowrx `video.c:140-142`).
#[allow(clippy::too_many_arguments, clippy::cast_possible_truncation)]
pub(crate) fn decode_line(
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
    let pixel_secs = spec.pixel_seconds;
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;
    let chan_len = f64::from(width) * pixel_secs;

    // Scottie channel start times relative to *line start* (the start
    // of the first septr at the beginning of line N). Translated from
    // slowrx `video.c:72-79`:
    //
    //   [septr][G pixels][septr][B pixels][SYNC][porch][R pixels]
    //   ^      ^                ^                       ^
    //   0      septr            2·septr + chan_len      2·septr + 2·chan_len + sync + porch
    //
    // `find_sync` returns a `skip_samples` already corrected to point at
    // line 0's start (its `SyncPosition::Scottie` branch applies
    // `s = s - chan_len/2 + 2·porch` per slowrx `sync.c:123-125`), so
    // these offsets are positive and identical to slowrx C's `ChanStart`
    // values.
    let chan_starts_sec: [f64; 3] = [
        septr_secs,                                                 // G
        2.0 * septr_secs + chan_len,                                // B
        2.0 * septr_secs + 2.0 * chan_len + sync_secs + porch_secs, // R
    ];

    let width_us = width as usize;

    // Channel sample-range bounds, computed once per channel with a
    // single `round()` per bound (matches slowrx `video.c:140-142` to
    // avoid per-pixel rounding drift).
    let chan_bounds_abs: [(i64, i64); 3] = std::array::from_fn(|i| {
        let start_sec = chan_starts_sec[i];
        let end_sec = start_sec + chan_len;
        let start_abs = skip_samples + ((line_seconds_offset + start_sec) * rate_hz).round() as i64;
        let end_abs = skip_samples + ((line_seconds_offset + end_sec) * rate_hz).round() as i64;
        (start_abs, end_abs)
    });

    // Decode each channel into its own buffer.
    let mut g = vec![0_u8; width_us];
    let mut b = vec![0_u8; width_us];
    let mut r = vec![0_u8; width_us];

    let buffers: [&mut [u8]; 3] = [&mut g, &mut b, &mut r];
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

    // Compose RGB and write to the image. Scottie is already RGB; no
    // chroma conversion needed (cf. Robot's YCrCb→RGB).
    for x in 0..width_us {
        image.put_pixel(x as u32, line_index, [r[x], g[x], b[x]]);
    }
}

#[cfg(test)]
mod tests {
    // Tests land in Phase 3 alongside the implementation.
}
