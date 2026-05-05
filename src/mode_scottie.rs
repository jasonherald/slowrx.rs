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
///
/// **Phase 1 stub:** this function is a no-op until the Phase 3 commit
/// lands the demodulation. Round-trip tests will fail with a non-zero
/// per-pixel mean diff until then; that's the TDD red.
#[allow(clippy::too_many_arguments)]
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
    // Phase 1 stub — Phase 3 implements the real demodulation.
    let _ = (
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

#[cfg(test)]
mod tests {
    // Tests land in Phase 3 alongside the implementation.
}
