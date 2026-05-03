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
    chroma_planes: Option<&mut [Vec<u8>; 2]>,
    demod: &mut crate::mode_pd::PdDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    match mode {
        SstvMode::Robot72 => {
            // R72 doesn't need chroma_planes — composes RGB in-place.
            // Drop the param if the caller passed it.
            let _ = chroma_planes;
            decode_r72_line(
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
            );
        }
        SstvMode::Robot24 | SstvMode::Robot36 => {
            // chroma_planes is constructed in DecodingState ctor for
            // every RobotYuv mode; absence here would indicate a
            // dispatch bug, not a runtime error.
            #[allow(clippy::expect_used)]
            let planes = chroma_planes
                .expect("R36/R24 require chroma_planes; DecodingState should populate them");
            decode_r36_or_r24_line(
                spec,
                line_index,
                audio,
                skip_samples,
                line_seconds_offset,
                rate_hz,
                image,
                planes,
                demod,
                snr_est,
                hedr_shift_hz,
            );
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

/// Decode one R36 or R24 radio line. Writes Y into image[`line_index`],
/// writes own chroma into the appropriate plane in `chroma_planes`, and
/// duplicates the chroma sample to the next row's slot in the same
/// plane (bounds-guarded at the last image row).
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
/// Per slowrx `video.c:182-191`, the chroma channel is:
///   - Cr (`chroma_planes[0]`) when `y % 2 == 0`
///   - Cb (`chroma_planes[1]`) when `y % 2 == 1`
///
/// Per slowrx `video.c:421-425`, each chroma sample is duplicated to
/// `Image[x][y+1][channel]`. When `y+1 >= image_lines`, slowrx C silently
/// writes past the end (undefined); we explicitly guard.
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
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
fn decode_r36_or_r24_line(
    spec: ModeSpec,
    line_index: u32,
    audio: &[f32],
    skip_samples: i64,
    line_seconds_offset: f64,
    rate_hz: f64,
    image: &mut crate::image::SstvImage,
    chroma_planes: &mut [Vec<u8>; 2],
    demod: &mut crate::mode_pd::PdDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    hedr_shift_hz: f64,
) {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let septr_secs = spec.septr_seconds;
    let width = spec.line_pixels;
    let chan_len_y = f64::from(width) * pixel_secs * 2.0;
    let chan_len_chroma = f64::from(width) * pixel_secs;

    let chan_start_y = sync_secs + porch_secs;
    let chan_start_chroma = chan_start_y + chan_len_y + septr_secs;

    let width_us = width as usize;

    let y_bounds_abs = {
        let start_abs =
            skip_samples + ((line_seconds_offset + chan_start_y) * rate_hz).round() as i64;
        let end_abs = skip_samples
            + ((line_seconds_offset + chan_start_y + chan_len_y) * rate_hz).round() as i64;
        (start_abs, end_abs)
    };
    let chroma_bounds_abs = {
        let start_abs =
            skip_samples + ((line_seconds_offset + chan_start_chroma) * rate_hz).round() as i64;
        let end_abs = skip_samples
            + ((line_seconds_offset + chan_start_chroma + chan_len_chroma) * rate_hz).round()
                as i64;
        (start_abs, end_abs)
    };

    let mut y_buf = vec![0_u8; width_us];
    let mut chroma_buf = vec![0_u8; width_us];

    // Y channel — synthesize a temporary ModeSpec with doubled
    // pixel_seconds so decode_one_channel_into reads at the correct
    // R36/R24 Y spacing (pixel_seconds * 2 per pixel — slowrx
    // video.c:60-70 R36/R24 case).
    let mut spec_y = spec;
    spec_y.pixel_seconds = pixel_secs * 2.0;

    crate::mode_pd::decode_one_channel_into(
        &mut y_buf,
        chan_start_y,
        y_bounds_abs,
        spec_y,
        audio,
        skip_samples,
        line_seconds_offset,
        rate_hz,
        demod,
        snr_est,
        hedr_shift_hz,
    );

    // Chroma channel — use spec's native pixel_seconds.
    crate::mode_pd::decode_one_channel_into(
        &mut chroma_buf,
        chan_start_chroma,
        chroma_bounds_abs,
        spec,
        audio,
        skip_samples,
        line_seconds_offset,
        rate_hz,
        demod,
        snr_est,
        hedr_shift_hz,
    );

    // Determine which chroma plane this line wrote into.
    // Even rows write Cr (plane 0); odd rows write Cb (plane 1) — per
    // slowrx `video.c:182-191`.
    let chroma_plane_idx = (line_index % 2) as usize;

    let line_off = (line_index as usize) * width_us;
    let next_off = ((line_index + 1) as usize) * width_us;
    let plane_len = chroma_planes[chroma_plane_idx].len();

    // Write own chroma into the plane at this row.
    chroma_planes[chroma_plane_idx][line_off..line_off + width_us].copy_from_slice(&chroma_buf);

    // Duplicate to next row (bounds-guarded for last image row).
    if next_off + width_us <= plane_len {
        chroma_planes[chroma_plane_idx][next_off..next_off + width_us].copy_from_slice(&chroma_buf);
    }

    // Compose RGB. cr and cb come from the two chroma planes at the
    // current row. For even line_index: cr is just-written own chroma;
    // cb is duplicated-from-(line_index-1) (or zero-init at line 0).
    // For odd line_index: cb is just-written own chroma; cr is
    // duplicated-from-(line_index-1).
    for x in 0..width_us {
        let y = y_buf[x];
        let cr = chroma_planes[0][line_off + x];
        let cb = chroma_planes[1][line_off + x];
        let rgb = crate::mode_pd::ycbcr_to_rgb(y, cr, cb);
        image.put_pixel(x as u32, line_index, rgb);
    }
}
