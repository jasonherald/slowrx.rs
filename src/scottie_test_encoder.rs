//! Synthetic RGB-sequential encoder for round-trip testing —
//! handles the Scottie (1/2/DX), Martin (1/2) and Wraase SC-2 (120/180)
//! families.
//!
//! Test-only — gated behind `cfg(any(test, feature = "test-support"))`.
//!
//! **Per-line tone emission order branches on
//! [`crate::modespec::SyncPosition`] and
//! [`crate::modespec::ColorEncoding`]:**
//!
//! ```text
//! Scottie (sync_position::Scottie):
//!   [septr 1500 Hz][G pixels 1500-2300 Hz][septr 1500 Hz]
//!   [B pixels 1500-2300 Hz][SYNC 1200 Hz][porch 1500 Hz]
//!   [R pixels 1500-2300 Hz]
//!
//! Martin (sync_position::LineStart, color_encoding::Gbr):
//!   [SYNC 1200 Hz][porch 1500 Hz][G pixels 1500-2300 Hz]
//!   [septr 1500 Hz][B pixels 1500-2300 Hz][septr 1500 Hz]
//!   [R pixels 1500-2300 Hz]
//!
//! Wraase SC-2 (sync_position::LineStart, color_encoding::Rgb):
//!   [SYNC 1200 Hz][porch 1500 Hz][R pixels 1500-2300 Hz]
//!   [G pixels 1500-2300 Hz][B pixels 1500-2300 Hz]
//! ```
//!
//! Total per line = `LineTime` exactly: Scottie and Martin pad a line that
//! float arithmetic rounds short; SC-2 lines are pinned to the line clock
//! (see [`encode_wraase`]).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::modespec::{ColorEncoding, ModeSpec, SstvMode, SyncPosition};
use crate::resample::WORKING_SAMPLE_RATE_HZ;
use crate::test_tone::{lum_to_freq, ToneWriter, PORCH_HZ, SEPTR_HZ, SYNC_HZ};

/// Index of each colour in an `[R, G, B]` source pixel.
const RED: usize = 0;
const GREEN: usize = 1;
const BLUE: usize = 2;

/// Encode an RGB image as continuous-phase FM audio for Scottie
/// (S1/S2/DX), Martin (M1/M2) or Wraase SC-2 (120/180). `rgb` is
/// row-major, `line_pixels × image_lines` `[R, G, B]` triples (320×256 for
/// all of these modes). Returns f32 PCM at [`WORKING_SAMPLE_RATE_HZ`]
/// (`11_025` Hz).
///
/// The per-line tone emission order branches on `spec.sync_position`
/// and, for SC-2, `spec.color_encoding`:
///
/// - [`SyncPosition::Scottie`] —
///   `[septr][G][septr][B][SYNC][porch][R]` (sync mid-line).
/// - [`SyncPosition::LineStart`] —
///   `[SYNC][porch][G][septr][B][septr][R]` (Martin / standard
///   PD-Robot order).
/// - [`ColorEncoding::Rgb`] — `[SYNC][porch][R][G][B]` (Wraase SC-2; see
///   [`encode_wraase`]).
///
/// Panics if `mode` is not one of the seven supported variants or if
/// `rgb.len() != line_pixels * image_lines`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn encode_scottie(mode: SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
    assert!(matches!(
        mode,
        SstvMode::Scottie1
            | SstvMode::Scottie2
            | SstvMode::ScottieDx
            | SstvMode::Martin1
            | SstvMode::Martin2
            | SstvMode::Wraase120
            | SstvMode::Wraase180
    ));
    let spec = crate::modespec::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    assert_eq!(rgb.len() as u32, w * h);

    let mut tone = ToneWriter::new();
    if spec.color_encoding == ColorEncoding::Rgb {
        encode_wraase(&mut tone, &spec, rgb);
        return tone.into_vec();
    }

    let mut t = 0.0_f64;
    for (y, row) in (0..h).zip(rgb.chunks_exact(w as usize)) {
        match spec.sync_position {
            SyncPosition::Scottie => scottie_line(&mut tone, &mut t, &spec, row),
            SyncPosition::LineStart => martin_line(&mut tone, &mut t, &spec, row),
        }

        // Defensive pad to the line_seconds boundary (existing logic
        // — shape unchanged).
        let line_end_target = f64::from(y + 1) * spec.line_seconds;
        let pad_secs = line_end_target - t;
        if pad_secs > 0.0 {
            tone.fill_to(PORCH_HZ, advance(&mut t, pad_secs));
        }
    }

    tone.into_vec()
}

/// Advance the running time `t` by `secs`; the sample index it now ends at.
fn advance(t: &mut f64, secs: f64) -> usize {
    *t += secs;
    (*t * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize
}

/// One colour channel: a `pixel_seconds` tone for component `colour`
/// ([`RED`] / [`GREEN`] / [`BLUE`]) of each pixel in `row`.
fn channel(tone: &mut ToneWriter, t: &mut f64, spec: &ModeSpec, row: &[[u8; 3]], colour: usize) {
    for px in row {
        tone.fill_to(lum_to_freq(px[colour]), advance(t, spec.pixel_seconds));
    }
}

/// Scottie line: `[septr][G][septr][B][SYNC][porch][R]` — sync between B
/// and R.
fn scottie_line(tone: &mut ToneWriter, t: &mut f64, spec: &ModeSpec, row: &[[u8; 3]]) {
    tone.fill_to(SEPTR_HZ, advance(t, spec.septr_seconds));
    channel(tone, t, spec, row, GREEN);
    tone.fill_to(SEPTR_HZ, advance(t, spec.septr_seconds));
    channel(tone, t, spec, row, BLUE);
    // Sync (mid-line, between B and R), then porch.
    tone.fill_to(SYNC_HZ, advance(t, spec.sync_seconds));
    tone.fill_to(PORCH_HZ, advance(t, spec.porch_seconds));
    channel(tone, t, spec, row, RED);
}

/// Martin line: sync at line start, then porch, then G/septr/B/septr/R.
fn martin_line(tone: &mut ToneWriter, t: &mut f64, spec: &ModeSpec, row: &[[u8; 3]]) {
    tone.fill_to(SYNC_HZ, advance(t, spec.sync_seconds));
    tone.fill_to(PORCH_HZ, advance(t, spec.porch_seconds));
    channel(tone, t, spec, row, GREEN);
    tone.fill_to(SEPTR_HZ, advance(t, spec.septr_seconds));
    channel(tone, t, spec, row, BLUE);
    tone.fill_to(SEPTR_HZ, advance(t, spec.septr_seconds));
    channel(tone, t, spec, row, RED);
}

/// Wraase SC-2 120 / 180: per radio line, the sync, the porch, then R, G
/// and B — slowrx's `default:` layout (`video.c:95-100`; `ChanStart[0]` =
/// sync + porch, each channel `pixel_seconds` × width, `SeptrTime` 0 so no
/// separator) in `ColorEnc = RGB` order (`video.c:434-437`).
///
/// Each line is pinned to `y × line_seconds`, the decoder's timing model.
/// SC-2 120's sync + porch + pixels (475.5000 ms) fall 0.03 ms short of
/// its 475.530018 ms line and are padded with `PORCH_HZ`; SC-2 180's come
/// to 711.17322 ms against 711.0225 ms — slowrx's 0.734532 ms `PixelTime`
/// is a little over 235 ms / 320 = 0.734375 ms — so the last blue pixel is
/// cut at the line end instead of every later line slipping 0.15 ms (as
/// `robot_test_encoder::encode_bw` does for Robot 24 / 12 B/W).
fn encode_wraase(tone: &mut ToneWriter, spec: &ModeSpec, rgb: &[[u8; 3]]) {
    let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
    let at = |secs: f64| (secs * sr).round() as usize;
    for (y, row) in (0..spec.image_lines).zip(rgb.chunks_exact(spec.line_pixels as usize)) {
        let line_end = f64::from(y + 1) * spec.line_seconds;
        let mut t = f64::from(y) * spec.line_seconds + spec.sync_seconds;
        tone.fill_to(SYNC_HZ, at(t));
        t += spec.porch_seconds;
        tone.fill_to(PORCH_HZ, at(t));
        for colour in [RED, GREEN, BLUE] {
            for px in row {
                t = (t + spec.pixel_seconds).min(line_end);
                tone.fill_to(lum_to_freq(px[colour]), at(t));
            }
        }
        tone.fill_to(PORCH_HZ, at(line_end));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: `lum_to_freq` endpoint coverage lives in
    // `crate::test_tone::tests::lum_to_freq_endpoints_match_black_and_white`
    // (its canonical home post-#86). Testing it from a consumer is
    // misleading about ownership.

    #[test]
    fn scottie1_encode_total_length() {
        let rgb = vec![[128u8; 3]; 320 * 256];
        let audio = encode_scottie(SstvMode::Scottie1, &rgb);
        let spec = crate::modespec::for_mode(SstvMode::Scottie1);
        let expected_len = (spec.line_seconds
            * f64::from(spec.image_lines)
            * f64::from(WORKING_SAMPLE_RATE_HZ)) as usize;
        // Allow 1-sample rounding slack at end-of-image.
        assert!(audio.len() >= expected_len);
        assert!(audio.len() <= expected_len + 1);
    }

    /// Every SC-2 line opens with the 1200 Hz sync exactly on the
    /// `line_seconds` clock — including SC-2 180, whose sync + porch +
    /// pixels (711.17 ms) overrun its 711.0225 ms line; unpinned, its last
    /// line would start ~39 ms late — and the audio is exactly
    /// `image_lines × line_seconds` long.
    #[test]
    fn encode_wraase_lines_stay_on_the_line_clock() {
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        for mode in [SstvMode::Wraase120, SstvMode::Wraase180] {
            let spec = crate::modespec::for_mode(mode);
            // White pixels: sync and video are easy to tell apart.
            let img = vec![[255_u8; 3]; (spec.line_pixels * spec.image_lines) as usize];
            let audio = encode_scottie(mode, &img);
            let h = f64::from(spec.image_lines);
            let expected = (h * spec.line_seconds * rate).round() as usize;
            assert_eq!(audio.len(), expected, "{mode:?}: audio length");
            let last_line = ((h - 1.0) * spec.line_seconds * rate).round() as usize;
            let seg = &audio[last_line..last_line + (spec.sync_seconds * rate) as usize];
            let p_sync = crate::dsp::goertzel_power(seg, SYNC_HZ);
            let p_white = crate::dsp::goertzel_power(seg, crate::test_tone::WHITE_HZ);
            assert!(
                p_sync > 10.0 * p_white,
                "{mode:?}: last line starts with sync (p_sync={p_sync}, p_white={p_white})"
            );
        }
    }

    /// SC-2 sends red, then green, then blue after the sync and porch
    /// (slowrx `ColorEnc = RGB`, `video.c:434-437`). A red-white,
    /// green-black, blue-grey image puts 2300 / 1500 / 1900 Hz mid-channel.
    #[test]
    fn encode_wraase_sends_red_then_green_then_blue() {
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        for mode in [SstvMode::Wraase120, SstvMode::Wraase180] {
            let spec = crate::modespec::for_mode(mode);
            let img = vec![[255_u8, 0, 128]; (spec.line_pixels * spec.image_lines) as usize];
            let audio = encode_scottie(mode, &img);
            let chan_len = f64::from(spec.line_pixels) * spec.pixel_seconds;
            let tones = [lum_to_freq(255), lum_to_freq(0), lum_to_freq(128)];
            for (chan, &want) in tones.iter().enumerate() {
                let mid = spec.sync_seconds + spec.porch_seconds + (chan as f64 + 0.5) * chan_len;
                let at = (mid * rate) as usize;
                let seg = &audio[at..at + 64];
                let p_want = crate::dsp::goertzel_power(seg, want);
                let others = tones.iter().enumerate().filter(|&(k, _)| k != chan);
                for (_, &other) in others {
                    let p_other = crate::dsp::goertzel_power(seg, other);
                    assert!(
                        p_want > 10.0 * p_other,
                        "{mode:?} channel {chan}: {want} Hz {p_want} vs {other} Hz {p_other}"
                    );
                }
            }
        }
    }
}
