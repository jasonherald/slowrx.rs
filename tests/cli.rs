//! Integration tests for the `slowrx-cli` binary.
//!
//! Most cases synthesize their WAVs, so they run in CI: the exit-code
//! contract (0 = at least one image written, 2 = no images, 1 = error),
//! input / output validation, and — with `test-support` — synthetic
//! transmissions decoded to correctly named PNGs: named after the input
//! file, safe whatever that file is called, or named by `--prefix` (#136).
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

/// `--help` shows how PNGs are named and that `--prefix` changes it.
#[test]
fn help_documents_the_png_names() {
    let assert = cli().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        stdout.contains("<input-stem>-NNN-{mode}.png") && stdout.contains("--prefix <NAME>"),
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
    assert_eq!(pngs_in(&out), [out.join("pd120-001-pd120.png")]);
    let _ = fs::remove_dir_all(&dir);
}

/// A synthetic Robot 12 B/W transmission (VIS + image + padding), as mono.
/// At 12 s it is the shortest image there is, for tests that only need a
/// decode.
#[cfg(feature = "test-support")]
#[allow(clippy::cast_possible_truncation)]
fn robot12bw_mono() -> Vec<i16> {
    use slowrx::SstvMode;
    let spec = slowrx::for_mode(SstvMode::Robot12Bw);
    let ycrcb = vec![[200_u8, 128, 128]; (spec.line_pixels * spec.image_lines) as usize];
    let mut audio = slowrx::__test_support::vis::synth_vis(spec.vis_code, 0.0);
    audio.extend(slowrx::__test_support::mode_robot::encode_robot(
        SstvMode::Robot12Bw,
        &ycrcb,
    ));
    audio.extend(std::iter::repeat_n(0.0_f32, 8192));
    audio
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect()
}

/// Robot 12 B/W sends 120 lines that slowrx shows twice each
/// (`ModeSpec::line_height` = 2): the PNG is written 320 × 240 so it keeps
/// the picture's aspect, like slowrx's saved images.
#[cfg(feature = "test-support")]
#[test]
fn a_robot12bw_png_is_stretched_to_its_line_height() {
    let dir = scratch("r12bw");
    let input = dir.join("r12bw.wav");
    write_wav(&input, 1, &robot12bw_mono());
    let out = dir.join("out");
    run(&input, &out).code(0);
    let png = out.join("r12bw-001-robot12bw.png");
    assert_eq!(
        image::image_dimensions(&png).expect("png"),
        (320, 240),
        "120 lines × line_height 2"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// PNGs are named after their input file, so decoding a second recording
/// into the same directory keeps the first one's image (#136).
#[cfg(feature = "test-support")]
#[test]
fn two_inputs_decoded_into_one_directory_keep_both_images() {
    let dir = scratch("two-inputs");
    let out = dir.join("out");
    let audio = robot12bw_mono();
    for name in ["pass1.wav", "pass2.wav"] {
        let input = dir.join(name);
        write_wav(&input, 1, &audio);
        run(&input, &out).code(0);
    }
    let mut pngs = pngs_in(&out);
    pngs.sort();
    assert_eq!(
        pngs,
        [
            out.join("pass1-001-robot12bw.png"),
            out.join("pass2-001-robot12bw.png")
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Whatever the input file is called, its PNG is a plain, visible file in
/// the output directory, and the PNG's name, which the CLI prints, carries
/// no escape characters (#136). This input starts with a dot and a dash,
/// and has a space, an ANSI escape and shell syntax in it.
#[cfg(all(unix, feature = "test-support"))]
#[test]
fn a_hostile_input_name_still_gives_a_plain_png_name() {
    let dir = scratch("hostile");
    let input = dir.join(".-pass 1\u{1b}[31m$(id).wav");
    write_wav(&input, 1, &robot12bw_mono());
    let out = dir.join("out");
    let err = stderr(&run(&input, &out).code(0));
    assert_eq!(
        pngs_in(&out),
        [out.join("pass_1__31m__id_-001-robot12bw.png")]
    );
    assert!(!err.contains('\u{1b}'), "{err:?}");
    let _ = fs::remove_dir_all(&dir);
}

/// `--prefix NAME` names the PNGs instead of the input file; `--prefix img`
/// gives the names used before 0.9.0.
#[cfg(feature = "test-support")]
#[test]
fn prefix_flag_names_the_pngs() {
    let dir = scratch("prefix");
    let input = dir.join("pass1.wav");
    write_wav(&input, 1, &robot12bw_mono());
    let out = dir.join("out");
    cli()
        .args(["--prefix", "img", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(&out)
        .assert()
        .code(0);
    assert_eq!(pngs_in(&out), [out.join("img-001-robot12bw.png")]);
    let _ = fs::remove_dir_all(&dir);
}

/// `--prefix` takes a plain name only: nothing that could leave the output
/// directory, hide the file, or need quoting. It is refused while the
/// arguments are parsed, before anything is read or written.
#[test]
fn a_prefix_that_is_not_a_plain_name_is_rejected() {
    let dir = scratch("bad-prefix");
    let out = dir.join("out");
    for bad in ["../up", "a/b", ".hidden", "-x", "two words", ""] {
        let assert = cli()
            .arg("--input")
            .arg(dir.join("missing.wav"))
            .arg("--output")
            .arg(&out)
            .arg(format!("--prefix={bad}"))
            .assert()
            .code(1);
        let err = stderr(&assert);
        assert!(err.contains("letters, digits"), "{bad:?}: {err}");
        assert!(!out.exists(), "{bad:?}: output dir created");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// A name holding an ANSI escape, a BEL, a newline and a right-to-left
/// override. A file name can hold any character, and it is not always the
/// user's own choice, so the CLI shows these as escapes instead of sending
/// them to the terminal (#137).
#[cfg(unix)]
const HOSTILE: &str = "x\u{1b}[31m\u{7}\n\u{202e}y";
/// How `HOSTILE` is printed: on one line, with nothing for a terminal to act
/// on and nothing that reorders the text after it.
#[cfg(unix)]
const HOSTILE_SHOWN: &str = "x\\u{1b}[31m\\u{7}\\n\\u{202e}y";

/// `err` prints a path as `shown`, its escaped form, and nothing from the
/// path reached stderr raw: no control character other than the newline
/// that ends each message, and no right-to-left override. (A raw newline
/// from the path would break `shown` up, so the first check catches that.)
#[cfg(unix)]
fn assert_shown_escaped(err: &str, shown: &str) {
    assert!(err.contains(shown), "{err:?}");
    assert!(
        !err.contains(|c: char| (c.is_control() && c != '\n') || c == '\u{202e}'),
        "{err:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_control_character_in_the_input_name_is_escaped_when_it_cannot_be_opened() {
    let dir = scratch("esc-input");
    let input = dir.join(format!("{HOSTILE}.wav"));
    fs::write(&input, b"this is not a RIFF/WAVE file").unwrap();
    let err = stderr(&run(&input, &dir.join("out")).code(1));
    assert!(err.contains("error: open WAV"), "{err:?}");
    assert_shown_escaped(&err, &format!("{HOSTILE_SHOWN}.wav"));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_control_character_in_the_output_name_is_escaped_when_it_cannot_be_created() {
    let dir = scratch("esc-outfile");
    let input = dir.join("silence.wav");
    write_wav(&input, 1, &[0; 64]);
    // A regular file where the output directory should be.
    let out = dir.join(HOSTILE);
    fs::write(&out, b"").unwrap();
    let err = stderr(&run(&input, &out).code(1));
    assert!(err.contains("create output dir"), "{err:?}");
    assert_shown_escaped(&err, HOSTILE_SHOWN);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(all(unix, feature = "test-support"))]
#[test]
fn a_control_character_in_the_output_name_is_escaped_when_an_image_is_written() {
    let dir = scratch("esc-outdir");
    let input = dir.join("pass.wav");
    write_wav(&input, 1, &robot12bw_mono());
    let out = dir.join(HOSTILE);
    let err = stderr(&run(&input, &out).code(0));
    assert!(out.join("pass-001-robot12bw.png").is_file());
    assert_shown_escaped(&err, &format!("{HOSTILE_SHOWN}/pass-001-robot12bw.png"));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(all(unix, feature = "test-support"))]
#[test]
fn a_control_character_in_the_output_name_is_escaped_when_a_write_fails() {
    let dir = scratch("esc-write");
    let input = dir.join("pass.wav");
    write_wav(&input, 1, &robot12bw_mono());
    let out = dir.join(HOSTILE);
    // A directory where the PNG should go, so the write fails.
    fs::create_dir_all(out.join("pass-001-robot12bw.png")).unwrap();
    let err = stderr(&run(&input, &out).code(1));
    assert!(err.contains("error: write"), "{err:?}");
    assert_shown_escaped(&err, &format!("{HOSTILE_SHOWN}/pass-001-robot12bw.png"));
    let _ = fs::remove_dir_all(&dir);
}
