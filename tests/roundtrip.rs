//! Synthetic encode → decode round-trip for PD120 and PD180.

#![cfg(feature = "test-support")]
#![allow(clippy::expect_used, clippy::cast_possible_truncation)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, WORKING_SAMPLE_RATE_HZ};

/// Build a synthetic image: horizontal luma gradient + alternating chroma stripes.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn test_image(mode: SstvMode) -> (u32, u32, Vec<[u8; 3]>) {
    let spec = slowrx::for_mode(mode);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    let mut ycrcb = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let lum = ((f64::from(x)) / (f64::from(w)) * 255.0) as u8;
            // Smooth chroma (so adjacent-row averaging in the encoder doesn't
            // discard high-frequency chroma the decoder can't recover).
            let cr = if y % 4 < 2 { 200 } else { 56 };
            let cb = if (y / 2) % 2 == 0 { 200 } else { 56 };
            ycrcb.push([lum, cr, cb]);
        }
    }
    (w, h, ycrcb)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn run_roundtrip(mode: SstvMode) {
    let (w, h, ycrcb) = test_image(mode);

    // Build VIS + image audio.
    let vis_code = match mode {
        SstvMode::Pd120 => 0x5F,
        SstvMode::Pd180 => 0x60,
        _ => unreachable!(),
    };
    let mut audio = slowrx::__test_support::vis::synth_vis(vis_code, 0.0);
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(mode, &ycrcb));
    // Padding to absorb resampler group delay.
    audio.extend(std::iter::repeat_n(0.0_f32, 2048));

    let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
    let events = d.process(&audio);

    let img = events
        .iter()
        .find_map(|e| match e {
            SstvEvent::ImageComplete {
                image,
                partial: false,
            } => Some(image.clone()),
            _ => None,
        })
        .expect("ImageComplete event");

    assert_eq!(img.mode, mode);
    assert_eq!(img.width, w);
    assert_eq!(img.height, h);

    // Compare per-pixel against the encoded source.
    let mut max_diff = 0_u8;
    let mut sum_diff: u64 = 0;
    let mut n: u64 = 0;
    for (i, src) in ycrcb.iter().enumerate() {
        let src_rgb = slowrx::__test_support::mode_pd::ycbcr_to_rgb(src[0], src[1], src[2]);
        let dec = img.pixels[i];
        for ch in 0..3 {
            let d = (i32::from(src_rgb[ch]) - i32::from(dec[ch])).unsigned_abs() as u8;
            if d > max_diff {
                max_diff = d;
            }
            sum_diff += u64::from(d);
            n += 1;
        }
    }
    let mean = sum_diff as f64 / n as f64;
    // Tier B for synthetic round-trip: looser than slowrx-cross-validate
    // because no continuous-phase porch transitions and no anti-aliasing
    // filter on the synthetic encoder. ≤ 25 max, mean < 5 is healthy.
    assert!(
        max_diff <= 25,
        "{mode:?}: max_diff={max_diff} mean={mean:.2}"
    );
    assert!(mean < 5.0, "{mode:?}: max_diff={max_diff} mean={mean:.2}");
}

#[test]
fn pd120_roundtrip() {
    run_roundtrip(SstvMode::Pd120);
}

#[test]
fn pd180_roundtrip() {
    run_roundtrip(SstvMode::Pd180);
}
