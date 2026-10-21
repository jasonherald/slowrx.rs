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
//! **Time vs image-row layout:** R36/R24 are 2-channel per *radio line*
//! in time-allocation terms (Y at 2× pixel-time, then alternating
//! Cr/Cb), but each radio line populates TWO image rows because the
//! chroma sample is duplicated to the neighbor image row (`video.c:421-425`).
//! R72 is 3-channel per radio line, one image row per radio line, no
//! chroma duplication.
//!
//! **B/W:** Robot 24 / 12 / 8 B/W carry a single luminance channel per
//! radio line ([`crate::modespec::ChannelLayout::Monochrome`]; slowrx
//! `video.c:103-109` `NumChans = 1`) and are decoded by
//! `MonochromeDecoder` (`decode_bw_line`), not `RobotYuvDecoder`.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Per-mode chroma layout: video.c lines 60-101, 178-209, 421-425.
//! YUV→RGB matrix: video.c lines 446-451 (shared with PD; we re-use
//! [`crate::demod::ycbcr_to_rgb`]). See `NOTICE.md` for full attribution.
//!
//! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
//! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
//! verified at audit #94 (2026-05-15).

use crate::decoder::ModeDecoder;
use crate::demod::{decode_one_channel_into, ycbcr_to_rgb, ChannelDecodeCtx, DemodState};
use crate::image::SstvImage;
use crate::modespec::{ModeSpec, SstvMode};

/// [`ModeDecoder`] for [`crate::modespec::ChannelLayout::RobotYuv`]
/// (Robot 24 / 36 / 72): one image row per radio line. The R24/R36 path
/// also writes duplicated chroma to the neighbor image row (with bounds
/// guard) per slowrx `video.c:424-425`, so it owns the image's chroma
/// planes ([`RobotChroma::Alternating`]).
///
/// Radio line `index` emits Y for image row `index`; its time offset is
/// `f64::from(index) * spec.line_seconds` (un-rounded — the per-pixel time
/// computation does the single `round()` to match slowrx
/// `video.c:140-142`).
#[derive(Debug)]
pub(crate) struct RobotYuvDecoder {
    image_lines: u32,
    chroma: RobotChroma,
}

/// How a colour Robot mode carries chroma across radio lines.
#[derive(Debug)]
enum RobotChroma {
    /// Robot 72: Y, U, V on every radio line, composed into RGB in place
    /// (`decode_r72_line`); no cross-line state.
    PerLine,
    /// Robot 24 / 36: alternating Cr / Cb, each duplicated to the next
    /// image row (`decode_r36_or_r24_line`).
    Alternating {
        /// Per-mode chroma planes side buffer, `[cr_plane, cb_plane]`.
        ///
        /// Each plane is `image_lines * line_pixels` bytes, populated as
        /// radio lines are decoded: each radio line N writes its own
        /// chroma + duplicates to the next row's chroma slot (slowrx
        /// `video.c:421-425`); RGB composition for row N reads the
        /// duplicated-from-N-1 chroma channel that the line N-1 decode
        /// wrote earlier.
        ///
        /// PD composes RGB in-place per pair (see
        /// `mode_pd::decode_pd_line_pair`); Robot 72 and Scottie 1/2/DX
        /// also compose RGB in-place per radio line (see `decode_r72_line`
        /// and `mode_scottie::decode_line`). Only the R36/R24
        /// chroma-alternation + duplication path requires the side buffer;
        /// any future mode that does need cross-radio-line chroma state
        /// will need its own variant here.
        chroma_planes: [Vec<u8>; 2],
    },
}

impl RobotYuvDecoder {
    /// The decoder for one image of colour Robot mode `spec`. The per-mode
    /// case split mirrors slowrx `video.c:60-101` and `:104-130`.
    pub(crate) fn new(spec: ModeSpec) -> Self {
        let chroma = match spec.mode {
            // R72 doesn't need chroma_planes — composes RGB in-place.
            SstvMode::Robot72 => RobotChroma::PerLine,
            SstvMode::Robot24 | SstvMode::Robot36 => {
                let n = (spec.image_lines as usize) * (spec.line_pixels as usize);
                RobotChroma::Alternating {
                    chroma_planes: [vec![0_u8; n], vec![0_u8; n]],
                }
            }
            // The B/W modes have their own layout and line decoder
            // (`MonochromeDecoder`).
            SstvMode::Robot24Bw | SstvMode::Robot12Bw | SstvMode::Robot8Bw => {
                unreachable!("RobotYuvDecoder built for B/W mode {:?}", spec.mode);
            }
            // Listed, not `_`: inside this crate `#[non_exhaustive]` doesn't
            // apply, so a new mode forces a decision here at compile time
            // instead of landing on a runtime panic (audit C17).
            SstvMode::Pd120
            | SstvMode::Pd180
            | SstvMode::Pd240
            | SstvMode::Scottie1
            | SstvMode::Scottie2
            | SstvMode::ScottieDx
            | SstvMode::Martin1
            | SstvMode::Martin2
            | SstvMode::Wraase120
            | SstvMode::Wraase180 => {
                unreachable!("RobotYuvDecoder built for non-Robot mode {:?}", spec.mode);
            }
        };
        Self {
            image_lines: spec.image_lines,
            chroma,
        }
    }
}

impl ModeDecoder for RobotYuvDecoder {
    fn radio_lines(&self) -> u32 {
        self.image_lines
    }

    fn image_rows(&self, index: u32) -> std::ops::Range<u32> {
        index..index + 1
    }

    /// Robot is per-line (no PD line-pairing). For R36/R24 the
    /// chroma-duplication writes to the next image row; `LineDecoded` for
    /// image row N is emitted after radio-line N's decode — for R36/R24
    /// row 0 the Cb channel is at zero-init at this point (slowrx C does
    /// the same; final `ImageComplete` carries the populated state).
    fn decode_radio_line(
        &mut self,
        ctx: &ChannelDecodeCtx<'_>,
        index: u32,
        image: &mut SstvImage,
        state: &mut DemodState<'_>,
    ) {
        match &mut self.chroma {
            RobotChroma::PerLine => decode_r72_line(ctx, index, image, state),
            RobotChroma::Alternating { chroma_planes } => {
                decode_r36_or_r24_line(ctx, index, image, chroma_planes, state);
            }
        }
    }
}

/// [`ModeDecoder`] for [`crate::modespec::ChannelLayout::Monochrome`]
/// (Robot 24 / 12 / 8 B/W): one luminance channel, one grey image row, per
/// radio line (slowrx `video.c:103-109`, `video.c:453`) — see
/// [`decode_bw_line`].
#[derive(Debug)]
pub(crate) struct MonochromeDecoder {
    image_lines: u32,
}

impl MonochromeDecoder {
    /// The decoder for one image of Robot B/W mode `spec`.
    pub(crate) fn new(spec: ModeSpec) -> Self {
        Self {
            image_lines: spec.image_lines,
        }
    }
}

impl ModeDecoder for MonochromeDecoder {
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
        decode_bw_line(ctx, index, image, state);
    }
}

/// Decode one Robot B/W radio line into image row `line_index`.
///
/// One channel per line (slowrx `video.c:103-109`, `NumChans = 1`), laid
/// out by the `default:` branch of the channel-time switch
/// (`video.c:95-100`): `ChanStart[0]` = sync + porch, `ChanLen[0]` =
/// `pixel_seconds` × width. Each pixel is grey — `p[0] = p[1] = p[2] =
/// Image[tx][y][0]` (`video.c:453`). The radio line's time offset is
/// `line_index × line_seconds` (un-rounded; the per-pixel time does the
/// single `round()`, as for every mode).
pub(crate) fn decode_bw_line(
    ctx: &ChannelDecodeCtx<'_>,
    line_index: u32,
    image: &mut SstvImage,
    state: &mut DemodState<'_>,
) {
    let spec = ctx.spec;
    // Hoisted line buffers (Audit #93 D3); returned to the demod below.
    let mut bufs = state.demod.take_line_bufs(spec.line_pixels as usize);
    let [lum, _, _, _] = &mut bufs;
    decode_one_channel_into(
        lum,
        spec.sync_seconds + spec.porch_seconds,
        f64::from(line_index) * spec.line_seconds,
        ctx,
        state,
    );
    for (x, &y) in (0..spec.line_pixels).zip(lum.iter()) {
        image.put_pixel(x, line_index, [y, y, y]);
    }
    state.demod.restore_line_bufs(bufs);
}

/// Decode one R72 radio line into image row `line_index`.
///
/// R72 channel layout per slowrx `video.c:95-100` (default case for
/// non-PD/non-Scottie/non-Robot-alt modes):
///   `ChanLen[0..3]` = `pixel_seconds` * width   for each of Y, U, V
///   `ChanStart[0]` = sync + porch
///   `ChanStart[1]` = `ChanStart[0]` + `chan_len` + septr
///   `ChanStart[2]` = `ChanStart[1]` + `chan_len` + septr
///
/// Reuses [`crate::demod::decode_one_channel_into`] for per-channel
/// FFT-based demod — that helper is mode-agnostic (reads
/// `pixel_seconds` from `spec`, walks audio slice, fills a `&mut [u8]`).
#[allow(clippy::cast_possible_truncation)]
fn decode_r72_line(
    ctx: &ChannelDecodeCtx<'_>,
    line_index: u32,
    image: &mut SstvImage,
    state: &mut DemodState<'_>,
) {
    let spec = ctx.spec;
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
    let line_seconds_offset = f64::from(line_index) * spec.line_seconds;

    let width_us = width as usize;

    // Hoisted line buffers (Audit #93 D3); returned to the demod below.
    let mut bufs = state.demod.take_line_bufs(width_us);
    let [y, cr, cb, _] = &mut bufs;

    let buffers: [&mut [u8]; 3] = [y, cr, cb];
    for (chan_idx, buf) in buffers.into_iter().enumerate() {
        decode_one_channel_into(
            buf,
            chan_starts_sec[chan_idx],
            line_seconds_offset,
            ctx,
            state,
        );
    }

    for x in 0..width_us {
        let rgb = ycbcr_to_rgb(y[x], cr[x], cb[x]);
        image.put_pixel(x as u32, line_index, rgb);
    }
    state.demod.restore_line_bufs(bufs);
}

/// Decode one R36 or R24 radio line. Writes Y into image row `line_index`,
/// writes own chroma into the appropriate plane in `chroma_planes`, and
/// duplicates the chroma sample to the next row's slot in the same
/// plane (bounds-guarded at the last image row; [`store_chroma`]).
///
/// Channel layout per slowrx `video.c:60-70` (R36/R24 case):
///   `ChanLen[0]`   = `pixel_seconds` * width * 2   (Y allocated 2× pixel-time)
///   `ChanLen[1]`   = `ChanLen[2]` = `pixel_seconds` * width   (chroma at 1×)
///   `ChanStart[0]` = sync + porch
///   `ChanStart[1]` = `ChanStart[0]` + `ChanLen[0]` + septr
///   `ChanStart[2]` = `ChanStart[1]`   (chroma slot reused; actual channel
///                                  determined by row parity per
///                                  `video.c:182-191`)
///
/// RGB composition for row `line_index` reads:
///   - Y from the local Y buffer (just decoded)
///   - Cr from `chroma_planes[0][line_index * width + x]`. For even
///     `line_index` this was just written above; for odd `line_index`
///     this was written by the previous radio line's duplication
///     (`line_index - 1`, even, duplicated forward).
///   - Cb from `chroma_planes[1][line_index * width + x]`. For odd
///     `line_index` this was just written above; for even `line_index`
///     this was written by the previous radio line's duplication
///     (`line_index - 1`, odd, duplicated forward), EXCEPT `line_index` 0
///     has no previous line — Cb stays at zero-init (slowrx C does
///     the same; visible artifact in the top row).
#[allow(clippy::cast_possible_truncation)]
fn decode_r36_or_r24_line(
    ctx: &ChannelDecodeCtx<'_>,
    line_index: u32,
    image: &mut SstvImage,
    chroma_planes: &mut [Vec<u8>; 2],
    state: &mut DemodState<'_>,
) {
    let spec = ctx.spec;
    let pixel_secs = spec.pixel_seconds;
    let [chan_start_y, chan_start_chroma] = r36_or_r24_chan_starts(spec);
    let line_seconds_offset = f64::from(line_index) * spec.line_seconds;

    let width_us = spec.line_pixels as usize;

    // Hoisted line buffers (Audit #93 D3); returned to the demod below.
    let mut bufs = state.demod.take_line_bufs(width_us);
    let [y_buf, chroma_buf, _, _] = &mut bufs;

    // Y channel — synthesize a temporary ModeSpec with doubled
    // pixel_seconds so decode_one_channel_into reads at the correct
    // R36/R24 Y spacing (pixel_seconds * 2 per pixel — slowrx
    // video.c:60-70 R36/R24 case).
    let mut spec_y = spec;
    spec_y.pixel_seconds = pixel_secs * 2.0;
    let ctx_y = ChannelDecodeCtx {
        spec: spec_y,
        ..*ctx
    };
    decode_one_channel_into(y_buf, chan_start_y, line_seconds_offset, &ctx_y, state);

    // Chroma channel — use spec's native pixel_seconds.
    decode_one_channel_into(
        chroma_buf,
        chan_start_chroma,
        line_seconds_offset,
        ctx,
        state,
    );

    store_chroma(chroma_planes, line_index, chroma_buf);

    // Compose RGB. cr and cb come from the two chroma planes at the
    // current row. For even line_index: cr is just-written own chroma;
    // cb is duplicated-from-(line_index-1) (or zero-init at line 0).
    // For odd line_index: cb is just-written own chroma; cr is
    // duplicated-from-(line_index-1).
    let line_off = (line_index as usize) * width_us;
    for x in 0..width_us {
        let y = y_buf[x];
        let cr = chroma_planes[0][line_off + x];
        let cb = chroma_planes[1][line_off + x];
        let rgb = ycbcr_to_rgb(y, cr, cb);
        image.put_pixel(x as u32, line_index, rgb);
    }
    state.demod.restore_line_bufs(bufs);
}

/// R36/R24 channel start times, `[Y, chroma]`, from line start — the
/// `ChanStart[0]` / `ChanStart[1]` of the layout on
/// [`decode_r36_or_r24_line`] (slowrx `video.c:60-70`), with Y allocated
/// 2× pixel-time.
fn r36_or_r24_chan_starts(spec: ModeSpec) -> [f64; 2] {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;
    let chan_len_y = f64::from(width) * pixel_secs * 2.0;

    let chan_start_y = sync_secs + porch_secs;
    // slowrx C carries 3 ChanStart[] entries (Y, Cr, Cb at distinct
    // offsets). Rust collapses Cr and Cb into a single
    // `chan_start_chroma` because for R36/R24 the per-line parity
    // determines which channel actually lives in that time slot (the
    // per-line decode reads `chan_start_chroma`, knowing from `line % 2`
    // whether it's Cr or Cb). For R72 all three offsets are used
    // distinctly — the per-mode dispatch handles that path separately.
    // (Audit #94 E9.)
    let chan_start_chroma = chan_start_y + chan_len_y + septr_secs;
    [chan_start_y, chan_start_chroma]
}

/// Store R36/R24 radio line `line_index`'s chroma samples (`chroma`, one
/// image row wide) in `chroma_planes`, and duplicate them to the next row.
///
/// Per slowrx `video.c:182-191`, the chroma channel is:
///   - Cr (`chroma_planes[0]`) when `y % 2 == 0`
///   - Cb (`chroma_planes[1]`) when `y % 2 == 1`
///
/// Per slowrx `video.c:421-425`, each chroma sample is duplicated to
/// `Image[x][y+1][channel]`. When `y+1 >= image_lines`, slowrx C silently
/// writes past the end (undefined); we explicitly guard.
fn store_chroma(chroma_planes: &mut [Vec<u8>; 2], line_index: u32, chroma: &[u8]) {
    let width_us = chroma.len();
    // Determine which chroma plane this line wrote into.
    // Even rows write Cr (plane 0); odd rows write Cb (plane 1) — per
    // slowrx `video.c:182-191`.
    let chroma_plane_idx = (line_index % 2) as usize;

    let line_off = (line_index as usize) * width_us;
    let next_off = ((line_index + 1) as usize) * width_us;
    let plane = &mut chroma_planes[chroma_plane_idx];
    let plane_len = plane.len();

    // Write own chroma into the plane at this row.
    plane[line_off..line_off + width_us].copy_from_slice(chroma);

    // Duplicate to next row (bounds-guarded for last image row).
    if next_off + width_us <= plane_len {
        plane[next_off..next_off + width_us].copy_from_slice(chroma);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modespec::for_mode;

    /// Robot 24 / 36 decoders own zeroed `[Cr, Cb]` chroma planes one image
    /// wide and tall; Robot 72 composes in place and holds none (#125).
    #[test]
    fn robot_yuv_decoder_owns_chroma_planes_for_r24_and_r36_only() {
        for mode in [SstvMode::Robot24, SstvMode::Robot36, SstvMode::Robot72] {
            let spec = for_mode(mode);
            let n = (spec.image_lines * spec.line_pixels) as usize;
            match RobotYuvDecoder::new(spec).chroma {
                RobotChroma::Alternating { chroma_planes } => {
                    assert_ne!(mode, SstvMode::Robot72);
                    for plane in chroma_planes {
                        assert_eq!(plane, vec![0_u8; n], "{mode:?}");
                    }
                }
                RobotChroma::PerLine => assert_eq!(mode, SstvMode::Robot72),
            }
        }
    }

    /// Each line's chroma lands in its own row and is duplicated to the next
    /// (slowrx `video.c:421-425`): Cr on even lines, Cb on odd. Row 0's Cb
    /// stays zero, as in slowrx, and the last line has no next row to write.
    #[test]
    fn store_chroma_writes_its_row_and_duplicates_it_to_the_next() {
        const W: usize = 3;
        let mut planes = [vec![0_u8; W * 4], vec![0_u8; W * 4]];
        for (line, value) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
            store_chroma(&mut planes, line, &[value; W]);
        }
        let rows = |plane: &[u8]| plane.chunks(W).map(|r| r[0]).collect::<Vec<_>>();
        assert_eq!(rows(&planes[0]), [1, 1, 3, 3], "Cr");
        assert_eq!(rows(&planes[1]), [0, 2, 2, 4], "Cb");
    }
}
