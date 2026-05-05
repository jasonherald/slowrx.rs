//! RGB-sequential mode decoder — Scottie 1/2/DX and Martin 1/2.
//!
//! Both families use [`crate::modespec::ChannelLayout::RgbSequential`]:
//! three GBR channels per radio line, written to the image in-place
//! via `image.put_pixel`. The two families differ in **where the sync
//! pulse sits within a radio line**:
//!
//! - **Scottie** ([`crate::modespec::SyncPosition::Scottie`]): sync
//!   sits between the B and R channels (mid-line). `find_sync`
//!   applies a Scottie-specific correction so `skip_samples` lands
//!   at line 0's start.
//! - **Martin** ([`crate::modespec::SyncPosition::LineStart`]): sync
//!   at line start (standard SSTV convention); same as PD and Robot.
//!
//! ```text
//! Scottie line layout:
//!   [septr][G pixels][septr][B pixels][SYNC][porch][R pixels]
//!     ^                                  ^
//!     |                                  |
//!     line start                         find_sync detects this
//!                                        (mid-line — Scottie branch
//!                                        in find_sync corrects skip)
//!
//! Martin line layout:
//!   [SYNC][porch][G pixels][septr][B pixels][septr][R pixels]
//!   ^
//!   |
//!   line start (sync at line start; standard PD/Robot path)
//! ```
//!
//! Translated from slowrx's `video.c:72-79` (Scottie `ChanStart`) and
//! the `video.c` "default" case (Martin/PD/Robot `ChanStart`). slowrx's
//! GBR storage convention (`video.c:440-444`) is bypassed — we write
//! RGB directly via [`crate::image::SstvImage::put_pixel`].
//!
//! See `NOTICE.md` for full slowrx attribution.

use crate::modespec::ModeSpec;

/// Decode one RGB-sequential radio line (Scottie or Martin) into
/// `image`. Per-channel start times are line-start-relative and
/// branch on `spec.sync_position`:
///
/// - [`crate::modespec::SyncPosition::Scottie`] — channels are
///   `[septr, 2·septr+chan_len, 2·septr+2·chan_len+sync+porch]` from
///   line start. The mid-line sync sits at `2·septr + 2·chan_len`;
///   `find_sync` corrects `skip_samples` to land at line 0's start
///   (slowrx C `sync.c:123-125`), so all three offsets stay positive.
/// - [`crate::modespec::SyncPosition::LineStart`] — channels are
///   `[sync+porch, sync+porch+chan_len+septr, sync+porch+2·chan_len+2·septr]`
///   from line start, all post-sync. `find_sync` uses the unmodified
///   PD/Robot formula. Same shape as Robot 72 with RGB instead of `YCrCb`.
///
/// In both cases RGB is composed in place via `image.put_pixel`; no
/// chroma side-buffer is needed (cf. R36/R24's `chroma_planes`).
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

    // Channel start times relative to *line start*. Scottie's mid-line
    // sync sits at `2·septr + 2·chan_len`; the LineStart branch puts
    // sync at offset 0 (G follows after sync + porch). find_sync
    // returns `skip_samples` already corrected for both — Scottie
    // applies `s = s − chan_len/2 + 2·porch` (slowrx C
    // `sync.c:123-125`), LineStart uses the unmodified PD/Robot
    // formula.
    let chan_starts_sec: [f64; 3] = match spec.sync_position {
        crate::modespec::SyncPosition::Scottie => [
            septr_secs,                                                 // G (post-septr1)
            2.0 * septr_secs + chan_len,                                // B (post-septr2)
            2.0 * septr_secs + 2.0 * chan_len + sync_secs + porch_secs, // R (post-sync+porch)
        ],
        crate::modespec::SyncPosition::LineStart => [
            // Martin layout — slowrx C video.c default case:
            //   ChanStart[0] = sync + porch
            //   ChanStart[1] = ChanStart[0] + chan_len + septr
            //   ChanStart[2] = ChanStart[1] + chan_len + septr
            sync_secs + porch_secs,                                     // G
            sync_secs + porch_secs + chan_len + septr_secs,             // B
            sync_secs + porch_secs + 2.0 * chan_len + 2.0 * septr_secs, // R
        ],
    };

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
