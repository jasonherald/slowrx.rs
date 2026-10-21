//! RGB-sequential mode decoder — Scottie 1/2/DX, Martin 1/2 and Wraase
//! SC-2 120/180.
//!
//! All three families use [`crate::modespec::ChannelLayout::RgbSequential`]:
//! three colour channels per radio line, written to the image in-place
//! via `image.put_pixel`. They differ in **where the sync pulse sits
//! within a radio line** and in **the order the colours are sent**:
//!
//! - **Scottie** ([`crate::modespec::SyncPosition::Scottie`]): sync
//!   sits between the B and R channels (mid-line). `find_sync`
//!   applies a Scottie-specific correction so `skip_samples` lands
//!   at line 0's start.
//! - **Martin** ([`crate::modespec::SyncPosition::LineStart`]): sync
//!   at line start (standard SSTV convention); same as PD and Robot.
//! - **Wraase SC-2** ([`crate::modespec::SyncPosition::LineStart`]): sync
//!   at line start like Martin, no separator, and the colours sent R, G, B
//!   ([`crate::modespec::ColorEncoding::Rgb`]) where Martin and Scottie
//!   send G, B, R ([`crate::modespec::ColorEncoding::Gbr`]).
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
//!
//! Wraase SC-2 line layout (SeptrTime = 0):
//!   [SYNC][porch][R pixels][G pixels][B pixels]
//! ```
//!
//! Translated from slowrx's `video.c:72-79` (Scottie `ChanStart`) and
//! the `video.c` "default" case (Martin/SC-2/PD/Robot `ChanStart`). The
//! channels are composed into RGB by slowrx's pixel switch
//! (`video.c:432-455`) via [`crate::modespec::ColorEncoding`].
//!
//! See `NOTICE.md` for full slowrx attribution.
//!
//! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
//! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
//! verified at audit #94 (2026-05-15).

// NOTE (audit #94 B15): the file is named `mode_scottie.rs` but its
// `decode_line` is shared by Martin 1 / Martin 2 and Wraase SC-2 120 / 180
// (all `ChannelLayout::RgbSequential` per ModeSpec). A rename to
// `mode_rgb_sequential.rs` is tracked but deferred — the existing
// name is grep-able and the per-mode behavior branches on
// `spec.sync_position` (mid-line for Scottie, line-start for Martin and
// SC-2) and `spec.color_encoding` (the colour order) rather than file
// boundaries.

use crate::decoder::ModeDecoder;
use crate::demod::{ChannelDecodeCtx, DemodState};
use crate::image::SstvImage;
use crate::modespec::{ModeSpec, SyncPosition};

/// [`ModeDecoder`] for [`crate::modespec::ChannelLayout::RgbSequential`]
/// (Scottie, Martin, Wraase SC-2): one image row per radio line
/// ([`decode_line`]). Mid-line sync handling and the colour order live
/// inside [`decode_line`]. No `chroma_planes` — RGB is composed in-place
/// per line (no deferred chroma like R36/R24).
#[derive(Debug)]
pub(crate) struct RgbSequentialDecoder {
    image_lines: u32,
}

impl RgbSequentialDecoder {
    /// The decoder for one image of RGB-sequential mode `spec`.
    pub(crate) fn new(spec: ModeSpec) -> Self {
        Self {
            image_lines: spec.image_lines,
        }
    }
}

impl ModeDecoder for RgbSequentialDecoder {
    fn radio_lines(&self) -> u32 {
        self.image_lines
    }

    fn image_rows(&self, index: u32) -> std::ops::Range<u32> {
        index..index + 1
    }

    fn decode_radio_line(
        &mut self,
        ctx: &ChannelDecodeCtx<'_>,
        index: u32,
        image: &mut SstvImage,
        state: &mut DemodState<'_>,
    ) {
        decode_line(ctx, index, image, state);
    }
}

/// Decode one RGB-sequential radio line (Scottie, Martin or Wraase SC-2)
/// into image row `line_index`. Per-channel start times come from
/// [`chan_starts_seconds`]; the radio line's time offset is
/// `line_index × line_seconds` (un-rounded — the per-pixel time
/// computation does the single `round()` to match slowrx
/// `video.c:140-142`).
///
/// The three channels are decoded in transmit order and composed into the
/// pixel by `spec.color_encoding` (slowrx `video.c:432-444`): G, B, R for
/// Martin / Scottie, R, G, B for SC-2. RGB is composed in place via
/// `image.put_pixel`; no chroma side-buffer is needed (cf. R36/R24's
/// `chroma_planes`).
pub(crate) fn decode_line(
    ctx: &ChannelDecodeCtx<'_>,
    line_index: u32,
    image: &mut SstvImage,
    state: &mut DemodState<'_>,
) {
    let spec = ctx.spec;
    let chan_starts_sec = chan_starts_seconds(spec);
    let line_seconds_offset = f64::from(line_index) * spec.line_seconds;
    let width_us = spec.line_pixels as usize;

    // Hoisted line buffers (Audit #93 D3); returned to the demod below.
    let mut bufs = state.demod.take_line_bufs(width_us);
    let [c0, c1, c2, _] = &mut bufs;
    let buffers: [&mut [u8]; 3] = [c0, c1, c2];
    for (buf, &chan_start) in buffers.into_iter().zip(&chan_starts_sec) {
        crate::demod::decode_one_channel_into(buf, chan_start, line_seconds_offset, ctx, state);
    }

    let columns = c0.iter().zip(c1.iter()).zip(c2.iter());
    for (x, ((&a, &b), &c)) in (0..spec.line_pixels).zip(columns) {
        image.put_pixel(x, line_index, spec.color_encoding.to_rgb([a, b, c]));
    }
    state.demod.restore_line_bufs(bufs);
}

/// Channel start times, in transmit order, relative to *line start*:
///
/// - [`SyncPosition::Scottie`] — `[septr, 2·septr+chan_len,
///   2·septr+2·chan_len+sync+porch]` (slowrx `video.c:72-79`). The
///   mid-line sync sits at `2·septr + 2·chan_len`; `find_sync` corrects
///   `skip_samples` to land at line 0's start — `s = s − chan_len/2 +
///   2·porch` (slowrx C `sync.c:123-125`) — so all three offsets stay
///   positive.
/// - [`SyncPosition::LineStart`] — `[sync+porch, sync+porch+chan_len+septr,
///   sync+porch+2·chan_len+2·septr]`, all post-sync (slowrx `video.c`
///   default case, Martin and SC-2). `find_sync` uses the unmodified
///   PD/Robot formula. Same shape as Robot 72 with RGB instead of `YCrCb`.
fn chan_starts_seconds(spec: ModeSpec) -> [f64; 3] {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let septr_secs = spec.septr_seconds;
    let chan_len = f64::from(spec.line_pixels) * spec.pixel_seconds;
    match spec.sync_position {
        SyncPosition::Scottie => [
            septr_secs,                                                 // G (post-septr1)
            2.0 * septr_secs + chan_len,                                // B (post-septr2)
            2.0 * septr_secs + 2.0 * chan_len + sync_secs + porch_secs, // R (post-sync+porch)
        ],
        SyncPosition::LineStart => [
            // Martin / SC-2 layout — slowrx C video.c default case:
            //   ChanStart[0] = sync + porch
            //   ChanStart[1] = ChanStart[0] + chan_len + septr
            //   ChanStart[2] = ChanStart[1] + chan_len + septr
            sync_secs + porch_secs, // G (Martin) / R (SC-2)
            sync_secs + porch_secs + chan_len + septr_secs, // B (Martin) / G (SC-2)
            sync_secs + porch_secs + 2.0 * chan_len + 2.0 * septr_secs, // R (Martin) / B (SC-2)
        ],
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::modespec::{for_mode, SstvMode};

    /// SC-2's channels follow slowrx's `default:` layout (`video.c:95-100`):
    /// `ChanStart[0]` = sync + porch, each next one `ChanLen + SeptrTime`
    /// later, with `SeptrTime` 0 — R, G, B back to back.
    #[test]
    fn wraase_channels_follow_slowrx_default_layout() {
        for mode in [SstvMode::Wraase120, SstvMode::Wraase180] {
            let spec = for_mode(mode);
            let chan_len = 320.0 * spec.pixel_seconds;
            let first = 5.5225e-3 + 0.5e-3;
            let starts = chan_starts_seconds(spec);
            assert_eq!(starts[0], first, "{mode:?}");
            assert!((starts[1] - (first + chan_len)).abs() < 1e-12, "{mode:?}");
            assert!(
                (starts[2] - (first + 2.0 * chan_len)).abs() < 1e-12,
                "{mode:?}"
            );
        }
    }
}
