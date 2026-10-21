//! Numerical comparison of the ARISS Fram2 decodes with their reference
//! JPGs (#119), replacing the by-eye check in
//! `tests/ariss_fram2_validation.md` and giving #71 (vertical squiggle
//! artifacts) a number to improve.
//!
//! Each slide's WAV is decoded and compared with its reference JPG
//! (320 × 240, like our output). The comparison searches integer shifts of
//! up to [`MAX_SHIFT`] pixels, since the reference decoder's line-zero and
//! slant alignment differ slightly from ours. At the best shift it reports
//! the mean absolute error over all three channels (0–255), its 99th
//! percentile, and the maximum, within a [`MAX_SHIFT`]-pixel inset. JPEG
//! compression keeps the error above zero even for a perfect decode, so the
//! test guards against *regressions* past the committed baseline
//! (`tests/ariss_fram2_baseline.tsv`), not against an absolute bar.
//!
//! The fixtures are gitignored (see the validation doc for the download), so
//! the real-data test is `#[ignore]`d and skips without them:
//!
//! ```text
//! cargo test --release --features cli --test fram2_pixel_diff -- --ignored --nocapture
//! SLOWRX_FRAM2_BLESS=1 cargo test …   # rewrite the baseline after an intended change
//! ```
//!
//! The alignment and error maths are plain functions with synthetic unit
//! tests, which run in CI.

#![cfg(feature = "cli")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use slowrx::{SstvDecoder, SstvEvent};

/// Largest integer shift (pixels, each axis) searched when aligning a
/// decode with its reference.
const MAX_SHIFT: i32 = 4;

/// How far a slide's mean absolute error may rise above its baseline
/// before the real-data test fails (0–255 scale).
const REGRESSION_TOLERANCE: f64 = 0.05;

/// Row-major RGB image.
struct Rgb {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

impl Rgb {
    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.pixels[y * self.width + x]
    }
}

/// Error of a decode against its reference at the best shift.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Score {
    /// Shift applied to the decode: reference `(x, y)` is compared with
    /// decode `(x + dx, y + dy)`.
    dx: i32,
    dy: i32,
    /// Mean absolute error over R, G and B (0–255).
    mae: f64,
    /// 99th-percentile absolute channel error.
    p99: u8,
    /// Largest absolute channel error.
    max: u8,
}

/// Absolute channel errors over the reference's [`MAX_SHIFT`] inset, with
/// the decode shifted by `(dx, dy)`. Both images must be the same size.
fn channel_errors(decoded: &Rgb, reference: &Rgb, dx: i32, dy: i32) -> Vec<u8> {
    let m = MAX_SHIFT as usize;
    let mut errors = Vec::with_capacity(reference.pixels.len() * 3);
    for y in m..reference.height - m {
        for x in m..reference.width - m {
            let want = reference.at(x, y);
            let got = decoded.at((x as i32 + dx) as usize, (y as i32 + dy) as usize);
            errors.extend(want.iter().zip(got).map(|(&w, g)| w.abs_diff(g)));
        }
    }
    errors
}

fn mean(errors: &[u8]) -> f64 {
    errors.iter().map(|&e| f64::from(e)).sum::<f64>() / errors.len().max(1) as f64
}

/// Best integer alignment of `decoded` to `reference` by mean absolute
/// error, searching shifts in `-MAX_SHIFT..=MAX_SHIFT` on each axis.
fn score(decoded: &Rgb, reference: &Rgb) -> Score {
    assert_eq!(
        (decoded.width, decoded.height),
        (reference.width, reference.height),
        "decode and reference sizes differ"
    );
    // (shift x, shift y, mean absolute error) of the best alignment so far.
    let mut best = (0, 0, f64::INFINITY);
    for dy in -MAX_SHIFT..=MAX_SHIFT {
        for dx in -MAX_SHIFT..=MAX_SHIFT {
            let mae = mean(&channel_errors(decoded, reference, dx, dy));
            if mae < best.2 {
                best = (dx, dy, mae);
            }
        }
    }
    let mut errors = channel_errors(decoded, reference, best.0, best.1);
    errors.sort_unstable();
    Score {
        dx: best.0,
        dy: best.1,
        mae: best.2,
        p99: errors[(errors.len() - 1) * 99 / 100],
        max: errors.last().copied().unwrap_or(0),
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ariss-fram2")
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ariss_fram2_baseline.tsv")
}

/// Decode `wav` with a default decoder; its first completed image.
fn decode_first_image(wav: &Path) -> Rgb {
    let mut reader = hound::WavReader::open(wav).expect("open WAV");
    let mut decoder = SstvDecoder::new(reader.spec().sample_rate).expect("decoder");
    let mut first = None;
    slowrx::wav::decode_wav_stream(&mut reader, &mut decoder, |event| {
        if let SstvEvent::ImageComplete { image, .. } = event {
            first.get_or_insert(image);
        }
        Ok::<(), slowrx::Error>(())
    })
    .expect("decode");
    let image = first.expect("the slide decodes to an image");
    Rgb {
        width: image.width as usize,
        height: image.height as usize,
        pixels: image.pixels,
    }
}

fn load_reference(jpg: &Path) -> Rgb {
    let img = image::open(jpg).expect("open reference JPG").to_rgb8();
    Rgb {
        width: img.width() as usize,
        height: img.height() as usize,
        pixels: img.pixels().map(|p| p.0).collect(),
    }
}

/// `slide → baseline MAE` from the committed TSV (`#` lines are comments).
fn load_baseline() -> Vec<(String, f64)> {
    std::fs::read_to_string(baseline_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let slide = cols.next()?.to_string();
            let mae = cols.next()?.parse().ok()?;
            Some((slide, mae))
        })
        .collect()
}

fn write_baseline(rows: &[(String, Score)]) {
    let mut out = String::from(
        "# Fram2 decode vs reference JPG: mean absolute error (0-255) at the best shift.\n\
         # Written by `SLOWRX_FRAM2_BLESS=1 cargo test --release --features cli \
         --test fram2_pixel_diff -- --ignored`.\n# slide\tmae\tp99\tmax\tdx\tdy\n",
    );
    for (slide, s) in rows {
        writeln!(
            out,
            "{slide}\t{:.3}\t{}\t{}\t{}\t{}",
            s.mae, s.p99, s.max, s.dx, s.dy
        )
        .expect("writing to a String");
    }
    std::fs::write(baseline_path(), out).expect("write baseline");
}

/// `false` when none of the 12 WAV + JPG pairs are present (skip); panics
/// on a partial set, which would silently compare fewer slides than the
/// baseline.
fn fixtures_present(dir: &Path) -> bool {
    let expected: Vec<PathBuf> = (1..=12)
        .flat_map(|n| {
            let slide = format!("Slide{n:02}");
            [
                dir.join(format!("{slide}.wav")),
                dir.join(format!("{slide}-Robot-36-Color.jpg")),
            ]
        })
        .collect();
    let missing: Vec<&PathBuf> = expected.iter().filter(|p| !p.is_file()).collect();
    if missing.len() == expected.len() {
        return false;
    }
    assert!(
        missing.is_empty(),
        "incomplete Fram2 fixture set: {missing:?}"
    );
    true
}

/// Rewrite the baseline only for `SLOWRX_FRAM2_BLESS=1` (not `=0` or any
/// other value).
fn bless_requested() -> bool {
    std::env::var("SLOWRX_FRAM2_BLESS").is_ok_and(|v| v == "1")
}

fn print_table(rows: &[(String, Score)]) {
    eprintln!("slide    mae     p99  max  shift");
    for (slide, s) in rows {
        eprintln!(
            "{slide}  {:6.3}  {:3}  {:3}  ({:+},{:+})",
            s.mae, s.p99, s.max, s.dx, s.dy
        );
    }
    let mean_mae = rows.iter().map(|(_, s)| s.mae).sum::<f64>() / rows.len() as f64;
    eprintln!("mean     {mean_mae:6.3}");
}

#[test]
#[ignore = "needs the gitignored ARISS Fram2 fixtures; run with --ignored"]
fn fram2_slides_match_their_reference_jpgs() {
    let dir = fixtures_dir();
    if !fixtures_present(&dir) {
        eprintln!("skipping: no Fram2 fixtures in {}", dir.display());
        return;
    }
    let rows: Vec<(String, Score)> = (1..=12)
        .map(|n| {
            let slide = format!("Slide{n:02}");
            let decoded = decode_first_image(&dir.join(format!("{slide}.wav")));
            let reference = load_reference(&dir.join(format!("{slide}-Robot-36-Color.jpg")));
            (slide, score(&decoded, &reference))
        })
        .collect();
    print_table(&rows);
    if bless_requested() {
        write_baseline(&rows);
        return;
    }
    let baseline = load_baseline();
    assert_eq!(baseline.len(), rows.len(), "baseline covers every slide");
    for ((slide, s), (base_slide, base_mae)) in rows.iter().zip(&baseline) {
        assert_eq!(slide, base_slide);
        assert!(
            s.mae <= base_mae + REGRESSION_TOLERANCE,
            "{slide} regressed: MAE {:.3} vs baseline {base_mae:.3}",
            s.mae
        );
    }
}

/// A synthetic image with structure in both axes, so a shift is
/// recoverable.
fn pattern(width: usize, height: usize, offset: (i32, i32)) -> Rgb {
    let pixels = (0..height as i32)
        .flat_map(|y| {
            (0..width as i32).map(move |x| {
                let (u, v) = (x + offset.0, y + offset.1);
                [
                    ((u * 37 + v * 11) % 256) as u8,
                    ((u * u + v * 3) % 256) as u8,
                    ((v * 53) % 256) as u8,
                ]
            })
        })
        .collect();
    Rgb {
        width,
        height,
        pixels,
    }
}

#[test]
fn identical_images_score_zero_at_no_shift() {
    let img = pattern(32, 24, (0, 0));
    let s = score(&img, &pattern(32, 24, (0, 0)));
    assert_eq!(
        s,
        Score {
            dx: 0,
            dy: 0,
            mae: 0.0,
            p99: 0,
            max: 0
        }
    );
}

#[test]
fn a_shifted_decode_is_realigned() {
    // The decode is the reference moved 2 px left and 1 px down, so
    // reference (x, y) sits at decode (x - 2, y + 1).
    let reference = pattern(32, 24, (0, 0));
    let decoded = pattern(32, 24, (2, -1));
    let s = score(&decoded, &reference);
    assert_eq!((s.dx, s.dy), (-2, 1));
    assert!(s.mae.abs() < 1e-12 && s.max == 0, "{s:?}");
}

#[test]
fn a_uniform_error_is_its_own_mean_p99_and_max() {
    let reference = Rgb {
        width: 16,
        height: 16,
        pixels: vec![[100, 100, 100]; 256],
    };
    let decoded = Rgb {
        width: 16,
        height: 16,
        pixels: vec![[110, 90, 110]; 256],
    };
    let s = score(&decoded, &reference);
    assert!((s.mae - 10.0).abs() < 1e-12, "{s:?}");
    assert_eq!((s.p99, s.max), (10, 10));
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slowrx-fram2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn no_fixtures_means_skip() {
    let dir = scratch_dir("none");
    assert!(!fixtures_present(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[should_panic(expected = "incomplete Fram2 fixture set")]
fn a_partial_fixture_set_fails_instead_of_skipping() {
    let dir = scratch_dir("partial");
    std::fs::write(dir.join("Slide01.wav"), b"").expect("touch");
    fixtures_present(&dir);
}
