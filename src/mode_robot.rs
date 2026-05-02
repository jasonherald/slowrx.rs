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
//!
//! V2.2 stub — `decode_line` writes zeros pending Phase 3/4 fill-in.
//! See `docs/superpowers/plans/2026-05-02-v2.2-robot.md`.

use crate::modespec::{ModeSpec, SstvMode};

/// Decode one Robot radio line into `image`. The R24/R36 path also
/// writes duplicated chroma to the neighbor image row (with bounds
/// guard) per slowrx `video.c:424-425`.
///
/// `line_index` is the 0-based image row this radio line emits Y for;
/// `line_seconds_offset` is `f64::from(line_index) * spec.line_seconds`
/// (un-rounded — the per-pixel time computation does the single
/// `round()` to match slowrx `video.c:140-142`).
///
/// V2.2 stub — fills the row with zeros. Replaced by per-mode decode
/// in Phases 3 and 4.
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
pub(crate) fn decode_line(
    spec: ModeSpec,
    mode: SstvMode,
    line_index: u32,
    _audio: &[f32],
    _skip_samples: i64,
    _line_seconds_offset: f64,
    _rate_hz: f64,
    image: &mut crate::image::SstvImage,
    _demod: &mut crate::mode_pd::PdDemod,
    _snr_est: &mut crate::snr::SnrEstimator,
    _hedr_shift_hz: f64,
) {
    debug_assert!(matches!(
        mode,
        SstvMode::Robot24 | SstvMode::Robot36 | SstvMode::Robot72
    ));
    // Stub: write zeros. Replaced in Phase 3 (R72) and Phase 4 (R36/R24)
    // of the V2.2 implementation plan.
    for x in 0..spec.line_pixels {
        image.put_pixel(x, line_index, [0, 0, 0]);
    }
}
