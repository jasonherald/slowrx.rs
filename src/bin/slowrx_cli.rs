//! `slowrx-cli` — decode SSTV recordings from WAV files to PNG images.
//!
//! Usage:
//!
//! ```text
//! slowrx-cli --input recording.wav --output ./out
//! ```
//!
//! Reads the WAV at `--input`, decodes every SSTV image found, and writes
//! one PNG per `ImageComplete` event to
//! `<output>/<input-stem>-NNN-{mode}.png` (see [Output names](#output-names)).
//! WAV reading is [`slowrx::wav`]: 8/16/24/32-bit integer or 32-bit float
//! samples, any channel count (folded to mono).
//!
//! # Output names
//!
//! A PNG is named `<prefix>-NNN-{mode}.png`: a prefix, a per-run sequence
//! number starting at 001, and the lowercased mode tag. The prefix is the
//! input file's stem, so `recording.wav` gives `recording-001-pd120.png`.
//! Deriving the name from the input keeps runs on differently named inputs
//! from overwriting each other in a shared output directory; decoding the
//! same input again replaces its own PNGs.
//!
//! The stem is made safe first: letters and digits of any script, `-`, `_`
//! and `.` are kept, every other character becomes `_`, leading `.` and `-`
//! are dropped, a Windows device name (`nul`, `con`, `com1`, …) gains a
//! leading `_`, and the result is cut to 200 bytes (`img` if nothing is
//! left). A PNG is therefore always a plain, visible file directly inside
//! `--output`, whatever the input was called. One limit: an accent stored
//! as a separate combining character (as macOS often stores file names)
//! is not a letter, so it becomes `_` too.
//!
//! `--prefix NAME` sets the prefix instead. Use it for inputs that would
//! get the same PNG names: `pass1/audio.wav` and `pass2/audio.wav`, or
//! `a b.wav` and `a_b.wav`. The same goes for names the output directory's
//! file system treats as equal: `pass.wav` and `PASS.wav` where it ignores
//! letter case (the default on Windows and macOS), or two spellings of one
//! name that differ only in Unicode normalization (`가` as one character
//! or as two) on macOS. Use it too for piped input (`--input /dev/stdin`
//! has the stem `stdin`), or as `--prefix img` to get the
//! `img-NNN-{mode}.png` names used before 0.9.0.
//!
//! # Exit status
//!
//! - `0` — at least one image was written.
//! - `2` — the input decoded without error, but no image was written
//!   (no SSTV found, or every image dropped by `--drop-noise`).
//! - `1` — error: bad arguments, an unreadable or unsupported WAV, or an
//!   output directory / PNG that couldn't be written.
//!
//! Requires the `cli` feature: `cargo install --features cli`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode};

/// Exit status: the input decoded, but no image was written.
const EXIT_NO_IMAGES: u8 = 2;
/// Exit status: any error, including bad arguments.
const EXIT_ERROR: u8 = 1;

/// The exit-status contract, shown at the end of `--help`.
const EXIT_STATUS_HELP: &str = "\
Exit status:
  0  at least one image was written
  2  the input decoded, but no image was written
  1  error (bad arguments, unreadable or unsupported WAV, unwritable output)";

#[derive(Parser, Debug)]
#[command(
    name = "slowrx-cli",
    version,
    about = "Decode SSTV recordings (WAV) to PNG images",
    after_help = EXIT_STATUS_HELP
)]
struct Args {
    /// Path to a mono or multi-channel WAV file containing SSTV audio.
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Output directory (must be a directory). Created if it does not
    /// exist. PNGs are written here as `<input-stem>-NNN-{mode}.png`.
    #[arg(short, long, value_name = "DIR")]
    output: PathBuf,

    /// Name the PNGs `NAME-NNN-{mode}.png` instead of after the input file
    /// (`--prefix img` gives the names used before 0.9.0). A plain name:
    /// letters, digits, `-`, `_` and `.`, not starting with `.` or `-`.
    #[arg(long, value_name = "NAME", value_parser = parse_prefix)]
    prefix: Option<String>,

    /// Suppress progress output and the final summary. Errors and
    /// warnings still go to stderr.
    #[arg(short, long)]
    quiet: bool,

    /// Mode to assume when a VIS header's leaders arrive but its bits are
    /// lost (e.g. `robot36`, the same names as the output files); images
    /// with no header at all then start from this mode's sync rhythm. Off
    /// by default. See `SstvDecoder::set_fallback_mode`.
    #[arg(long, value_name = "MODE", value_parser = parse_mode)]
    fallback_mode: Option<SstvMode>,

    /// Decode every image as MODE, ignoring the mode bits of VIS headers
    /// (headers still mark where images start; images without one start
    /// from the sync rhythm). Overrides `--fallback-mode`. See
    /// `SstvDecoder::set_forced_mode`.
    #[arg(long, value_name = "MODE", value_parser = parse_mode)]
    mode: Option<SstvMode>,

    /// Don't write images whose line-sync quality says they are noise
    /// (`SyncQuality::looks_like_noise`, e.g. a VIS followed by a faded or
    /// ended transmission). They are still reported on stderr.
    #[arg(long)]
    drop_noise: bool,

    /// Demodulate pixels from the analytic signal: removes the fine diagonal
    /// cross-hatch slowrx's real-signal demod leaves in dark areas, with no
    /// loss of sharpness. Off by default (slowrx's behaviour). See
    /// `SstvDecoder::set_analytic_demod`.
    #[arg(long)]
    analytic_demod: bool,
}

/// Resolve a mode by its `ModeSpec::short_name` (case-insensitive).
fn parse_mode(name: &str) -> std::result::Result<SstvMode, String> {
    let specs: Vec<_> = (0u8..0x80).filter_map(slowrx::lookup_vis).collect();
    specs
        .iter()
        .find(|s| s.short_name.eq_ignore_ascii_case(name))
        .map(|s| s.mode)
        .ok_or_else(|| {
            let names: Vec<_> = specs.iter().map(|s| s.short_name).collect();
            format!(
                "unknown mode '{name}'; expected one of: {}",
                names.join(", ")
            )
        })
}

/// Accept a `--prefix` only if it is already a safe name: one that
/// [`safe_name`] returns unchanged. A name the user chose is refused, not
/// rewritten, when it isn't.
fn parse_prefix(name: &str) -> std::result::Result<String, String> {
    if safe_name(name) == name {
        Ok(name.to_string())
    } else {
        Err(format!(
            "must be a plain name: letters, digits, '-', '_' and '.', \
             not starting with '.' or '-', at most {MAX_PREFIX_BYTES} bytes, \
             and not a Windows device name such as 'nul'"
        ))
    }
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            // `--help` / `--version` land here too, and succeed.
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(EXIT_ERROR)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match run(&args) {
        Ok(0) => {
            eprintln!("warning: no SSTV images decoded from input");
            ExitCode::from(EXIT_NO_IMAGES)
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Decode `--input` and write its images into `--output`. Returns how many
/// PNGs were written, which `main` turns into the exit status.
fn run(args: &Args) -> Result<u32> {
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("create output dir {}", args.output.display()))?;

    let mut reader = hound::WavReader::open(&args.input)
        .with_context(|| format!("open WAV {}", args.input.display()))?;
    let spec = reader.spec();
    if !args.quiet {
        eprintln!(
            "input: {} Hz, {} channel(s), {} bits, {} samples",
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample,
            reader.duration()
        );
    }
    slowrx::wav::check_spec(spec)?;

    let mut decoder =
        SstvDecoder::new(spec.sample_rate).with_context(|| "construct SstvDecoder")?;
    decoder.set_fallback_mode(args.fallback_mode);
    decoder.set_forced_mode(args.mode);
    decoder.set_analytic_demod(args.analytic_demod);

    if !args.quiet {
        eprintln!("decoding {} samples (streaming)...", reader.duration());
    }

    // Streams in fixed-size chunks (flat memory, events as they come).
    let prefix = output_prefix(args.prefix.as_deref(), &args.input);
    let mut tally = Tally::default();
    slowrx::wav::decode_wav_stream(&mut reader, &mut decoder, |event| {
        on_event(args, &prefix, &mut tally, event)
    })?;

    if !args.quiet {
        eprintln!("{}", tally.summary());
    }
    Ok(tally.images)
}

/// Running counts for the end-of-run summary.
#[derive(Default)]
struct Tally {
    images: u32,
    vis: u32,
    fallback: u32,
    sync_starts: u32,
    dropped: u32,
    lines: u32,
}

impl Tally {
    /// The end-of-run summary line.
    fn summary(&self) -> String {
        format!(
            "done: {} VIS ({} assumed), {} sync start(s), {} lines, {} image(s), {} dropped as noise",
            self.vis, self.fallback, self.sync_starts, self.lines, self.images, self.dropped
        )
    }
}

/// Handle one decoder event: count it, report it, and write completed
/// images (unless `--drop-noise` says the image is noise).
fn on_event(args: &Args, prefix: &str, tally: &mut Tally, event: SstvEvent) -> Result<()> {
    let note = match event {
        SstvEvent::VisDetected {
            mode,
            sample_offset,
            hedr_shift_hz,
        } => {
            tally.vis += 1;
            format!("  VIS: mode={mode:?} at sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)")
        }
        SstvEvent::VisFallback {
            mode,
            sample_offset,
            hedr_shift_hz,
        } => {
            tally.vis += 1;
            tally.fallback += 1;
            format!("  VIS lost, assuming fallback: mode={mode:?} image from sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)")
        }
        SstvEvent::SyncStart {
            mode,
            sample_offset,
        } => {
            tally.sync_starts += 1;
            format!("  no VIS, sync rhythm: mode={mode:?} image from sample {sample_offset}")
        }
        SstvEvent::LineDecoded { .. } => {
            tally.lines += 1;
            return Ok(());
        }
        SstvEvent::ImageComplete {
            image,
            sync_quality,
            ..
        } => return on_image(args, prefix, tally, &image, sync_quality),
        _ => return Ok(()),
    };
    if !args.quiet {
        eprintln!("{note}");
    }
    Ok(())
}

/// Write (or, with `--drop-noise`, skip) one completed image.
fn on_image(
    args: &Args,
    prefix: &str,
    tally: &mut Tally,
    image: &SstvImage,
    sync_quality: slowrx::SyncQuality,
) -> Result<()> {
    let noise = sync_quality.looks_like_noise();
    if args.drop_noise && noise {
        tally.dropped += 1;
        if !args.quiet {
            eprintln!(
                "  ImageComplete: {} x {} dropped (sync quality {:.2} looks like noise)",
                image.width,
                image.height,
                sync_quality.value()
            );
        }
        return Ok(());
    }
    tally.images += 1;
    let path = args.output.join(png_name(prefix, tally.images, image.mode));
    save_image(image, &path)?;
    if !args.quiet {
        eprintln!(
            "  ImageComplete: {} x {} (sync quality {:.2}{}) → {}",
            image.width,
            image.height,
            sync_quality.value(),
            if noise { ", looks like noise" } else { "" },
            path.display()
        );
    }
    Ok(())
}

/// The prefix of this run's PNG names: `--prefix` when given, otherwise the
/// input file's stem (`recording.wav` → `recording`). Either way it goes
/// through [`safe_name`], the one gate between a name from outside and an
/// output path. A `--prefix` that passed [`parse_prefix`] is already safe,
/// so it comes out unchanged.
fn output_prefix(prefix: Option<&str>, input: &Path) -> String {
    let raw = prefix.map_or_else(
        || input.file_stem().unwrap_or_default().to_string_lossy(),
        std::borrow::Cow::Borrowed,
    );
    safe_name(&raw)
}

/// The longest prefix [`safe_name`] returns, in bytes. The rest of a PNG's
/// name is at most 25 bytes, so the whole name stays well inside the 255
/// bytes the common filesystems allow.
const MAX_PREFIX_BYTES: usize = 200;

/// The prefix [`safe_name`] returns when `raw` has nothing usable in it.
const DEFAULT_PREFIX: &str = "img";

/// Turn `raw` (an input file's stem) into a prefix that is safe to put in
/// an output file name, whatever the input was called:
///
/// - letters and digits of any script, `-`, `_` and `.` are kept; every
///   other character becomes `_`. That covers path separators, spaces,
///   control and escape characters, and shell or HTML punctuation.
/// - leading `.` and `-` are dropped, so the file is never hidden and never
///   reads as a command-line option.
/// - a name that Windows would open as a device gains a leading `_` (see
///   [`is_windows_device`]).
/// - the result is cut to [`MAX_PREFIX_BYTES`] on a character boundary.
/// - if nothing is left, the result is [`DEFAULT_PREFIX`].
///
/// A name that is already safe comes back unchanged.
fn safe_name(raw: &str) -> String {
    let mut name = String::new();
    for c in raw.chars() {
        let c = if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
            c
        } else {
            '_'
        };
        if name.is_empty() && matches!(c, '.' | '-') {
            continue;
        }
        name.push(c);
    }
    if is_windows_device(name.split('.').next().unwrap_or_default()) {
        name.insert(0, '_');
    }
    if name.len() > MAX_PREFIX_BYTES {
        let mut end = MAX_PREFIX_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    if name.is_empty() {
        name.push_str(DEFAULT_PREFIX);
    }
    name
}

/// Whether `stem` (a file name up to its first `.`) is a Windows device
/// name: `con`, `prn`, `aux`, `nul`, or `com` / `lpt` and one digit (`1` to
/// `9`, `¹`, `²`, `³`), in any letter case. Before Windows 11, opening
/// `nul.anything` in any directory opens the device, so a PNG written
/// under such a name would be lost without an error.
fn is_windows_device(stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") {
        return true;
    }
    let Some(port) = stem
        .strip_prefix("com")
        .or_else(|| stem.strip_prefix("lpt"))
    else {
        return false;
    };
    let mut digits = port.chars();
    matches!(
        (digits.next(), digits.next()),
        (Some('1'..='9' | '¹' | '²' | '³'), None)
    )
}

/// The file name of the `index`th image written in a run:
/// `{prefix}-NNN-{mode}.png`.
fn png_name(prefix: &str, index: u32, mode: SstvMode) -> String {
    format!(
        "{prefix}-{index:03}-{}.png",
        slowrx::for_mode(mode).short_name
    )
}

/// Write `image` as a PNG, repeating each row `line_height` times (2 for
/// Robot 12/8 B/W, whose 120 lines slowrx shows twice each) so the picture
/// keeps its aspect, as slowrx's saved images do.
fn save_image(image: &SstvImage, path: &Path) -> Result<()> {
    let w = image.width;
    let line_height = slowrx::for_mode(image.mode).line_height.max(1);
    let h = image.height * line_height;
    let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for row in image.pixels.chunks(w.max(1) as usize) {
        for _ in 0..line_height {
            buf.extend(row.iter().flatten());
        }
    }
    let img: image::ImageBuffer<image::Rgb<u8>, _> = image::ImageBuffer::from_vec(w, h, buf)
        .ok_or_else(|| anyhow!("image buffer size mismatch"))?;
    img.save(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The names recordings usually have pass through untouched.
    #[test]
    fn a_plain_name_is_kept() {
        for name in [
            "recording",
            "gqrx_20261110_193000_145800000",
            "pass.2026-11-10",
            "_x",
        ] {
            assert_eq!(safe_name(name), name);
        }
    }

    /// Cyrillic, CJK and precomposed accented letters are letters too.
    #[test]
    fn letters_and_digits_of_any_script_are_kept() {
        for name in ["запись7", "録音", "café"] {
            assert_eq!(safe_name(name), name);
        }
    }

    /// Spaces, path separators, shell syntax and control characters all
    /// go the same way, one `_` each.
    #[test]
    fn every_other_character_becomes_an_underscore() {
        assert_eq!(safe_name("pass 1 (final)"), "pass_1__final_");
        assert_eq!(safe_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(safe_name("a$(id);b`c`"), "a__id__b_c_");
        // A newline, an ANSI escape, a right-to-left override, a NUL.
        assert_eq!(safe_name("a\nb\u{1b}[31mc\u{202e}d\0e"), "a_b__31mc_d_e");
    }

    /// Every character there is, one at a time: none reaches a name as a
    /// control, whitespace, separator or invisible formatting character,
    /// or as ASCII punctuation other than `-`, `_` and `.`.
    #[test]
    fn no_unsafe_character_reaches_a_name() {
        // Zero-width and bidirectional formatting characters, and the BOM.
        let invisible = |c: char| {
            matches!(
                c,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        };
        for c in '\0'..=char::MAX {
            let name = safe_name(&format!("a{c}"));
            assert_eq!(name.chars().count(), 2, "U+{:04X}", u32::from(c));
            for kept in name.chars() {
                assert!(
                    !kept.is_control()
                        && !kept.is_whitespace()
                        && !invisible(kept)
                        && (!kept.is_ascii_punctuation() || "-_.".contains(kept)),
                    "U+{:04X} gave {name:?}",
                    u32::from(c)
                );
            }
        }
    }

    /// An accent stored as its own combining character is not a letter, so
    /// a decomposed name (as macOS often stores them) loses it.
    #[test]
    fn a_separately_encoded_accent_becomes_an_underscore() {
        assert_eq!(safe_name("cafe\u{301}"), "cafe_");
    }

    /// No hidden files and nothing that reads as an option; dots and
    /// dashes further in are fine.
    #[test]
    fn leading_dots_and_dashes_are_dropped() {
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name("-rf"), "rf");
        assert_eq!(safe_name("..-.x.-y"), "x.-y");
    }

    /// Empty, or only dots and dashes: there is always a prefix.
    #[test]
    fn a_name_with_nothing_left_is_img() {
        for name in ["", ".", "..", "---", "-.-"] {
            assert_eq!(safe_name(name), "img", "{name:?}");
        }
    }

    /// The cap is in bytes and never splits a character.
    #[test]
    fn a_long_name_is_cut_on_a_character_boundary() {
        assert_eq!(safe_name(&"a".repeat(300)), "a".repeat(MAX_PREFIX_BYTES));
        // Three bytes each: 66 fit in 200 bytes, a 67th would not.
        assert_eq!(safe_name(&"録".repeat(100)), "録".repeat(66));
        // The underscore a device name gains counts towards the cap.
        let device = safe_name(&format!("nul.{}", "a".repeat(300)));
        assert!(device.starts_with("_nul.aaa"), "{device}");
        assert_eq!(device.len(), MAX_PREFIX_BYTES);
    }

    /// Before Windows 11, a file whose name up to its first `.` is a device
    /// name opens that device instead of a file: `nul.x-001-pd120.png`
    /// would be thrown away, and `con.x-…` written to the console.
    #[test]
    fn a_windows_device_name_gains_a_leading_underscore() {
        for (raw, safe) in [
            ("nul", "_nul"),
            ("NUL.x", "_NUL.x"),
            ("con.2026", "_con.2026"),
            ("Aux", "_Aux"),
            ("prn.a.b", "_prn.a.b"),
            ("com1.pass", "_com1.pass"),
            ("LPT9", "_LPT9"),
            ("com¹.x", "_com¹.x"),
            // Checked after the leading dot is dropped.
            (".nul.x", "_nul.x"),
        ] {
            assert_eq!(safe_name(raw), safe, "{raw:?}");
        }
    }

    /// Only an exact device name up to the first `.` counts.
    #[test]
    fn a_name_that_only_resembles_a_device_name_is_kept() {
        for name in [
            "null",
            "console.x",
            "icon.x",
            "nul_x",
            "com",
            "lpt",
            "com10",
            "com1x.y",
            "_nul",
            // Windows reserves ports 1 to 9 only.
            "com0",
            "LPT0",
            "com0.x",
        ] {
            assert_eq!(safe_name(name), name);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Whatever the input is called, its PNG is one plain, visible,
        /// bounded file name directly inside the output directory.
        #[test]
        fn any_name_gives_a_plain_file_inside_the_output_dir(
            raw in prop_oneof![
                // Printable text only: proptest leaves out control,
                // format, private-use and unassigned characters here.
                any::<String>(),
                // Dense in the ASCII that matters, and long enough to cut.
                "[-._/ $a-zа-я0-9\\x00-\\x1f]{0,300}",
                // Any characters at all, those included.
                proptest::collection::vec(any::<char>(), 0..300)
                    .prop_map(|chars| chars.into_iter().collect::<String>()),
            ]
        ) {
            let name = safe_name(&raw);
            prop_assert!(!name.is_empty());
            prop_assert!(name.len() <= MAX_PREFIX_BYTES);
            prop_assert!(!name.starts_with(['.', '-']));
            prop_assert!(name.chars().all(|c| !c.is_control() && !c.is_whitespace()));
            prop_assert!(name
                .chars()
                .all(|c| !c.is_ascii_punctuation() || "-_.".contains(c)));
            let file = png_name(&name, 1, SstvMode::Robot36);
            let path = Path::new("out").join(&file);
            prop_assert_eq!(path.parent(), Some(Path::new("out")));
            prop_assert_eq!(path.file_name(), Some(std::ffi::OsStr::new(&file)));
            // A safe name is left alone, which is how `--prefix` is checked.
            prop_assert_eq!(safe_name(&name), name.clone());
        }
    }

    /// The directory and the last extension go; earlier dots stay.
    #[test]
    fn the_prefix_is_the_input_files_stem() {
        assert_eq!(
            output_prefix(None, Path::new("recordings/pass1.wav")),
            "pass1"
        );
        assert_eq!(
            output_prefix(None, Path::new("pass.2026.11.10.wav")),
            "pass.2026.11.10"
        );
        assert_eq!(output_prefix(None, Path::new("/dev/stdin")), "stdin");
    }

    /// The stem goes through `safe_name` on its way to the prefix.
    #[test]
    fn a_hostile_input_name_gives_a_safe_prefix() {
        assert_eq!(output_prefix(None, Path::new(".hidden.wav")), "hidden");
        assert_eq!(output_prefix(None, Path::new("dir/-rf.wav")), "rf");
        assert_eq!(
            output_prefix(None, Path::new("a b\u{1b}[31m.wav")),
            "a_b__31m"
        );
    }

    /// Paths with no final component have no stem to use.
    #[test]
    fn an_input_with_no_file_name_gives_img() {
        for input in ["..", "/", "recordings/.."] {
            assert_eq!(output_prefix(None, Path::new(input)), "img", "{input:?}");
        }
    }

    /// Bytes that are not UTF-8 have a defined result: `_`.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_input_name_gives_a_safe_prefix() {
        use std::os::unix::ffi::OsStrExt;
        let input = std::ffi::OsStr::from_bytes(b"caf\xe9.wav");
        assert_eq!(output_prefix(None, Path::new(input)), "caf_");
    }

    /// `--prefix` wins over the input's name.
    #[test]
    fn an_explicit_prefix_replaces_the_input_stem() {
        assert_eq!(output_prefix(Some("iss"), Path::new("pass1.wav")), "iss");
    }

    /// `--prefix` is checked when the arguments are parsed, but nothing
    /// reaches a file name without going through `safe_name`.
    #[test]
    fn a_prefix_that_skipped_the_argument_check_is_still_made_safe() {
        assert_eq!(output_prefix(Some("../up"), Path::new("pass1.wav")), "_up");
    }

    /// A `--prefix` that is already safe is taken as given.
    #[test]
    fn a_plain_prefix_is_accepted() {
        for name in ["img", "iss-2026.11.10", "запись_1", "com0"] {
            assert_eq!(parse_prefix(name), Ok(name.to_string()));
        }
    }

    /// Anything `safe_name` would have to change is an argument error:
    /// a name the user chose is refused, not rewritten.
    #[test]
    fn a_prefix_that_is_not_already_safe_is_refused() {
        let too_long = "a".repeat(MAX_PREFIX_BYTES + 1);
        for name in [
            "",
            ".",
            "..",
            "../up",
            "a/b",
            "a\\b",
            ".hidden",
            "-x",
            "two words",
            "a\nb",
            "nul",
            "con.x",
            too_long.as_str(),
        ] {
            assert!(parse_prefix(name).is_err(), "{name:?}");
        }
    }

    /// The number is padded to three digits and grows past them.
    #[test]
    fn a_png_is_named_prefix_number_mode() {
        assert_eq!(
            png_name("pass1", 1, SstvMode::Robot36),
            "pass1-001-robot36.png"
        );
        assert_eq!(png_name("img", 1000, SstvMode::Pd120), "img-1000-pd120.png");
    }

    /// The prefix cap leaves room for the rest of the name: 255 bytes is
    /// the limit on the common filesystems.
    #[test]
    fn the_longest_png_name_fits_a_255_byte_file_name() {
        let prefix = safe_name(&"a".repeat(1000));
        for spec in (0u8..0x80).filter_map(slowrx::lookup_vis) {
            let name = png_name(&prefix, u32::MAX, spec.mode);
            assert!(name.len() <= 255, "{} bytes: {name}", name.len());
        }
    }
}
