//! Decode an SSTV recording (WAV) to PNG(s) using the slowrx decoder.
//!
//! Usage: `cargo run --example decode_wav -- <input.wav> [output_prefix]`
//!
//! Output: one PNG per `ImageComplete` event, named
//! `<prefix>-<index>.png` (or `<prefix>.png` for a single image).
//! Default prefix is the input WAV's basename without extension.
//!
//! Exit code: 0 if at least one image was decoded, 1 otherwise.

use slowrx::{SstvDecoder, SstvEvent};
use std::path::PathBuf;
use std::process::ExitCode;

#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let mut args = std::env::args();
    let _ = args.next();
    let Some(input) = args.next() else {
        eprintln!("usage: decode_wav <input.wav> [output_prefix]");
        return ExitCode::from(2);
    };
    let input_path = PathBuf::from(&input);

    let prefix = match args.next() {
        Some(p) => p,
        None => input_path.file_stem().map_or_else(
            || "decoded".to_string(),
            |s| s.to_string_lossy().into_owned(),
        ),
    };

    let mut reader = match hound::WavReader::open(&input_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to open {input}: {e}");
            return ExitCode::from(1);
        }
    };
    let spec = reader.spec();
    eprintln!(
        "input: {} Hz, {} channel(s), {} bits, {} samples",
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample,
        reader.duration()
    );

    // Read all samples as f32 in [-1, 1], collapse to mono if stereo.
    // Read errors fail the example rather than silently dropping samples;
    // a partial decode of a corrupted WAV would mask real bugs.
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            if !matches!(bits, 8 | 16 | 24 | 32) {
                eprintln!("error: unsupported integer bit depth: {bits}-bit");
                return ExitCode::from(1);
            }
            // Normalize signed PCM by its positive full-scale magnitude.
            // Matches what `samples::<i16>() / i16::MAX` did before, generalized.
            #[allow(clippy::cast_precision_loss)]
            let divisor = ((1_i64 << (bits - 1)) - 1) as f32;
            let mut buf = Vec::with_capacity(reader.len() as usize);
            for sample in reader.samples::<i32>() {
                match sample {
                    Ok(s) => {
                        #[allow(clippy::cast_precision_loss)]
                        let f = (s as f32) / divisor;
                        buf.push(f);
                    }
                    Err(e) => {
                        eprintln!("error: sample read failed: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            buf
        }
        hound::SampleFormat::Float => {
            let mut buf = Vec::with_capacity(reader.len() as usize);
            for sample in reader.samples::<f32>() {
                match sample {
                    Ok(s) => buf.push(s),
                    Err(e) => {
                        eprintln!("error: sample read failed: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            buf
        }
    };
    let mono = collapse_to_mono(raw, spec.channels as usize);

    let mut decoder = match SstvDecoder::new(spec.sample_rate) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: SstvDecoder::new({}): {e}", spec.sample_rate);
            return ExitCode::from(1);
        }
    };

    eprintln!("decoding {} mono samples...", mono.len());
    let events = decoder.process(&mono);

    let mut image_count = 0;
    let mut vis_count = 0;
    let mut line_count = 0;

    for event in events {
        match event {
            SstvEvent::VisDetected {
                mode,
                sample_offset,
                hedr_shift_hz,
            } => {
                vis_count += 1;
                eprintln!(
                    "  VIS: mode={mode:?} at sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)"
                );
            }
            SstvEvent::LineDecoded { .. } => {
                line_count += 1;
            }
            SstvEvent::ImageComplete { image, partial } => {
                image_count += 1;
                let path = if image_count == 1 {
                    format!("{prefix}.png")
                } else {
                    format!("{prefix}-{image_count}.png")
                };
                if let Err(e) = save_image(&image, &path) {
                    eprintln!("error: write {path}: {e}");
                    return ExitCode::from(1);
                }
                eprintln!(
                    "  ImageComplete: {} x {} (partial={partial}) → {path}",
                    image.width, image.height
                );
            }
            _ => {}
        }
    }

    eprintln!("done: {vis_count} VIS, {line_count} lines, {image_count} image(s)");
    if image_count == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn collapse_to_mono(samples: Vec<f32>, channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples;
    }
    #[allow(clippy::cast_precision_loss)]
    let inv = 1.0 / (channels as f32);
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * inv)
        .collect()
}

fn save_image(image: &slowrx::SstvImage, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let w = image.width;
    let h = image.height;
    let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for pixel in &image.pixels {
        buf.extend_from_slice(pixel);
    }
    let img: image::ImageBuffer<image::Rgb<u8>, _> =
        image::ImageBuffer::from_vec(w, h, buf).ok_or("image buffer size mismatch")?;
    img.save(path)?;
    Ok(())
}
