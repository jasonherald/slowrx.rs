//! Integration tests for the `slowrx-cli` binary.
//!
//! Most cases synthesize their WAVs, so they run in CI: the exit-code
//! contract (0 = at least one image written, 2 = no images, 1 = error),
//! input / output validation, and — with `test-support` — a synthetic
//! PD120 transmission decoded to a correctly named PNG.
//!
//! `cli_decodes_aris_fixture_to_png` also checks the binary end-to-end
//! against real audio, but skips silently unless an ARISS fixture is
//! available locally (`docs/wav_files/` is gitignored).

#![cfg(feature = "cli")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

/// Sample rate of the synthesized WAVs (the decoder's working rate).
const RATE_HZ: u32 = 11_025;

fn first_aris_fixture() -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR is the slowrx.rs root.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("docs/wav_files/201712-ISS_SSTV");
    if !dir.is_dir() {
        return None;
    }
    fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "wav"))
}

/// Unique scratch directory per test (tests run in parallel).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slowrx-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli() -> Command {
    Command::cargo_bin("slowrx-cli").expect("binary built")
}

/// Run the binary on `input`, writing into `output`.
fn run(input: &Path, output: &Path) -> assert_cmd::assert::Assert {
    cli()
        .arg("--input")
        .arg(input)
        .arg("--output")
        .arg(output)
        .assert()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
}

fn pngs_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .expect("output dir exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect()
}

/// Write interleaved 16-bit `samples` as a `channels`-channel WAV.
fn write_wav(path: &Path, channels: u16, samples: &[i16]) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in samples {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

/// Write a mono PCM WAV `hound` can't produce: any `bits` /
/// `block_align` pairing, `data` as the raw sample bytes.
fn write_raw_pcm(path: &Path, bits: u16, block_align: u16, data: &[u8]) {
    let data_len = u32::try_from(data.len()).unwrap();
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16_u32.to_le_bytes()); // PCMWAVEFORMAT chunk
    b.extend_from_slice(&1_u16.to_le_bytes()); // WAVE_FORMAT_PCM
    b.extend_from_slice(&1_u16.to_le_bytes()); // mono
    b.extend_from_slice(&RATE_HZ.to_le_bytes());
    b.extend_from_slice(&(RATE_HZ * u32::from(block_align)).to_le_bytes());
    b.extend_from_slice(&block_align.to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    b.extend_from_slice(data);
    fs::write(path, b).unwrap();
}

#[test]
fn cli_decodes_aris_fixture_to_png() {
    let Some(wav) = first_aris_fixture() else {
        eprintln!("skipping: no ARISS fixture in docs/wav_files/201712-ISS_SSTV/");
        return;
    };

    let out_dir = std::env::temp_dir().join(format!("slowrx-cli-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out_dir);

    Command::cargo_bin("slowrx-cli")
        .expect("binary built")
        .arg("--input")
        .arg(&wav)
        .arg("--output")
        .arg(&out_dir)
        .arg("--quiet")
        .assert()
        .success();

    let pngs = pngs_in(&out_dir);
    assert!(
        !pngs.is_empty(),
        "expected at least one PNG written to {}",
        out_dir.display()
    );

    let _ = fs::remove_dir_all(&out_dir);
}

#[test]
fn help_documents_the_exit_codes_and_output_directory() {
    let assert = cli().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("(must be a directory)"), "{stdout}");
    assert!(
        stdout.contains("0  at least one image was written")
            && stdout.contains("2  ")
            && stdout.contains("1  "),
        "{stdout}"
    );
}

#[test]
fn a_usage_error_exits_1() {
    let err = stderr(&cli().args(["--input", "x.wav"]).assert().code(1));
    assert!(err.contains("--output"), "{err}");
}

#[test]
fn a_file_that_is_not_a_wav_exits_1() {
    let dir = scratch("garbage");
    let input = dir.join("garbage.wav");
    fs::write(&input, b"this is not a RIFF/WAVE file").unwrap();
    let err = stderr(&run(&input, &dir.join("out")).code(1));
    assert!(err.contains("error: open WAV"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn silence_exits_2_with_a_warning_and_no_pngs() {
    let dir = scratch("silence");
    let input = dir.join("silence.wav");
    write_wav(&input, 1, &vec![0; 2 * RATE_HZ as usize]);
    let out = dir.join("out");
    let err = stderr(&run(&input, &out).code(2));
    assert!(err.contains("warning: no SSTV images"), "{err}");
    assert_eq!(pngs_in(&out), Vec::<PathBuf>::new());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_output_path_that_is_a_regular_file_exits_1() {
    let dir = scratch("outfile");
    let input = dir.join("silence.wav");
    write_wav(&input, 1, &[0; 64]);
    let out = dir.join("not-a-dir");
    fs::write(&out, b"").unwrap();
    let err = stderr(&run(&input, &out).code(1));
    assert!(err.contains("create output dir"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unsupported_bit_depth_is_named() {
    let dir = scratch("bits");
    let input = dir.join("40bit.wav");
    write_raw_pcm(&input, 40, 5, &[0; 10]);
    let err = stderr(&run(&input, &dir.join("out")).code(1));
    assert!(
        err.contains("unsupported integer bit depth: 40-bit"),
        "{err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_padded_sample_container_is_named() {
    // 16-bit samples in 4-byte slots: the bit depth is fine, but hound
    // has no reader for the layout.
    let dir = scratch("padded");
    let input = dir.join("padded.wav");
    write_raw_pcm(&input, 16, 4, &[0; 16]);
    let err = stderr(&run(&input, &dir.join("out")).code(1));
    assert!(err.contains("unsupported sample container layout"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

/// A synthetic PD120 transmission (VIS + image + padding), as stereo.
#[cfg(feature = "test-support")]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pd120_stereo() -> Vec<i16> {
    use slowrx::SstvMode;
    let spec = slowrx::for_mode(SstvMode::Pd120);
    let ycrcb: Vec<[u8; 3]> = (0..spec.image_lines)
        .flat_map(|_| (0..spec.line_pixels).map(|x| [(x % 256) as u8, 128, 128]))
        .collect();
    let mut audio = slowrx::__test_support::vis::synth_vis(0x5F, 0.0);
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(
        SstvMode::Pd120,
        &ycrcb,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    audio
        .iter()
        .flat_map(|&s| {
            let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            [v, v]
        })
        .collect()
}

#[cfg(feature = "test-support")]
#[test]
fn a_synthetic_pd120_wav_decodes_to_a_named_png() {
    let dir = scratch("pd120");
    let input = dir.join("pd120.wav");
    write_wav(&input, 2, &pd120_stereo());
    let out = dir.join("out");
    run(&input, &out).code(0);
    assert_eq!(pngs_in(&out), [out.join("img-001-pd120.png")]);
    let _ = fs::remove_dir_all(&dir);
}
