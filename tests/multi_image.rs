//! Multi-image streaming — back-to-back SSTV transmissions must all decode
//! within a single `SstvDecoder::process` call, not one-per-call (audit #90:
//! A2 + D4). Counterpart to `tests/roundtrip.rs` (single image) and
//! `tests/unknown_vis.rs` (post-unknown-VIS reseed).

#![cfg(feature = "test-support")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, WORKING_SAMPLE_RATE_HZ};

/// PD120 == VIS code 0x5F.
const PD120_CODE: u8 = 0x5F;

/// A small synthetic `YCrCb` image for PD120 (luma gradient + smooth chroma
/// stripes — same shape as `tests/roundtrip.rs`'s `test_image`, so the
/// encoder's adjacent-row chroma averaging has something it can reproduce).
fn pd120_test_image() -> Vec<[u8; 3]> {
    let spec = slowrx::for_mode(SstvMode::Pd120);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    let mut ycrcb = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let lum = ((f64::from(x)) / (f64::from(w)) * 255.0) as u8;
            let cr = if y % 4 < 2 { 200 } else { 56 };
            let cb = if (y / 2) % 2 == 0 { 200 } else { 56 };
            ycrcb.push([lum, cr, cb]);
        }
    }
    ycrcb
}

/// Mean absolute RGB error of a decoded image against its `Y'CbCr` source.
fn mean_error(image: &slowrx::SstvImage, ycrcb: &[[u8; 3]]) -> f64 {
    let mut sum = 0_u64;
    for (src, got) in ycrcb.iter().zip(&image.pixels) {
        let want = slowrx::__test_support::mode_pd::ycbcr_to_rgb(src[0], src[1], src[2]);
        for (w, g) in want.iter().zip(got) {
            sum += u64::from((i32::from(*w) - i32::from(*g)).unsigned_abs());
        }
    }
    sum as f64 / (ycrcb.len() * 3) as f64
}

#[test]
fn decoder_decodes_two_back_to_back_images() {
    let img1 = pd120_test_image();
    let img2 = pd120_test_image();

    // Two complete PD120 transmissions, concatenated, then a pad to absorb
    // the resampler FIR group delay so image 2's last line + VIS 2 produce
    // full analysis windows (2048 matches tests/roundtrip.rs's PD padding).
    let mut audio = slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0);
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(
        SstvMode::Pd120,
        &img1,
    ));
    audio.extend(slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0));
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(
        SstvMode::Pd120,
        &img2,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 2048));

    let mut decoder = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder construct");
    let events = decoder.process(&audio); // ONE call

    let vis_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(
                e,
                SstvEvent::VisDetected {
                    mode: SstvMode::Pd120,
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect();
    let complete_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SstvEvent::ImageComplete { partial: false, .. }))
        .map(|(i, _)| i)
        .collect();

    assert_eq!(
        vis_positions.len(),
        2,
        "expected 2 VisDetected{{Pd120}} in one process() call, got {} (event count {})",
        vis_positions.len(),
        events.len()
    );
    assert_eq!(
        complete_positions.len(),
        2,
        "expected 2 ImageComplete{{partial:false}} in one process() call, got {}",
        complete_positions.len()
    );
    // Order: VisDetected1 < ImageComplete1 < VisDetected2 < ImageComplete2.
    assert!(
        vis_positions[0] < complete_positions[0]
            && complete_positions[0] < vis_positions[1]
            && vis_positions[1] < complete_positions[1],
        "events out of order: vis@{vis_positions:?} complete@{complete_positions:?}"
    );
    // Both completed images are PD120 (the carry-forward correctly delivered
    // image 2's VIS + video to the re-armed detector) and decode at the
    // round-trip quality bar — image 2 starts at the right post-stop-bit
    // boundary even though its detector was re-armed mid-stream (#99).
    for (&i, src) in complete_positions.iter().zip([&img1, &img2]) {
        if let SstvEvent::ImageComplete { image, .. } = &events[i] {
            assert_eq!(image.mode, SstvMode::Pd120, "ImageComplete @{i} mode");
            let err = mean_error(image, src);
            assert!(err < 5.0, "ImageComplete @{i} mean error {err:.2}");
        }
    }
}

/// `VisDetected.sample_offset` must be an absolute working-rate stream
/// position for *every* detection, not just the first (issue #99). Before
/// the fix, a detector re-armed after an image counted hops from zero, so
/// the second VIS reported an offset relative to the carry-forward audio
/// (observed in the field as near-identical ~1.36 M offsets across
/// unrelated passes).
#[test]
fn second_vis_sample_offset_is_absolute() {
    let img = pd120_test_image();
    let vis1 = slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0);
    let video1 = slowrx::__test_support::mode_pd::encode_pd(SstvMode::Pd120, &img);
    let vis2 = slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0);
    // True stream position where VIS 2's stop bit ends.
    let vis2_stop_end = vis1.len() + video1.len() + vis2.len();

    let mut audio = vis1;
    audio.extend(video1);
    audio.extend(vis2);
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(
        SstvMode::Pd120,
        &img,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 2048));

    let mut decoder = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    let offsets: Vec<u64> = decoder
        .process(&audio)
        .iter()
        .filter_map(|e| match e {
            SstvEvent::VisDetected { sample_offset, .. } => Some(*sample_offset),
            _ => None,
        })
        .collect();
    assert_eq!(offsets.len(), 2, "two VIS detections");
    // Detection lands within a few 10 ms hops of the stop-bit end.
    let tolerance = (WORKING_SAMPLE_RATE_HZ as usize) * 30 / 1000;
    let got = offsets[1] as usize;
    assert!(
        got.abs_diff(vis2_stop_end) <= tolerance,
        "second VIS sample_offset {got} should be ≈ {vis2_stop_end} (absolute)"
    );
}
