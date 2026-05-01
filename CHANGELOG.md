# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed (PR-2 CR round 2)
- `SstvDecoder::reset()` now also clears polyphase FIR resampler state and
  PdDemod state. Previously residual FIR history could contaminate audio
  after a user reset.
- Strengthened `reset_during_decoding` test to actually trigger VIS detection
  (added FIR-group-delay padding + explicit VIS assertion) before testing reset.
- Fixed CHANGELOG symbol path: `SstvDecoder.sample_offset` →
  `SstvEvent::VisDetected.sample_offset`.

### Fixed (PR-2 CR round 1)
- `SstvDecoder::process` now preserves trailing audio after `ImageComplete`,
  so a back-to-back VIS burst (ARISS multi-image case) is not lost.
- `PdDemod::pixel_freq` no longer allocates a `Vec<Complex<f32>>` per pixel
  — reuses a preallocated `fft_buf` field. ~635k allocs/image saved on PD180.
- `SstvEvent::ImageComplete::partial` doc clarified — V1 always emits
  `partial: false`; `reset()` discards in-flight state silently. The
  `partial: true` case is reserved for future mid-image VIS handling.

### Added (PR-2)
- PD120 and PD180 mode decoding: `mode_pd::PdDemod` (256-pt FFT-based per-pixel
  demod with Gaussian-log peak interpolation, matching slowrx `video.c:391-394`),
  `mode_pd::decode_pd_line_pair` (Y(odd)/Cr/Cb/Y(even) channel layout, chroma
  sharing). New `rustfft = "6"` dependency for slowrx algorithmic parity.
- Polyphase FIR resampler (`resample.rs`) replacing PR-0's passthrough. 64-tap
  Hann-windowed sinc kernel, stride-walked phase accumulator. Caller's audio
  rate → 11025 Hz working rate, with state preserved across `process` calls.
- `SstvEvent::LineDecoded { mode, line_index, pixels }` and
  `SstvEvent::ImageComplete { image, partial }` are now produced during a
  decoded pass.
- `SstvEvent::VisDetected.sample_offset` documents working-rate (11025 Hz)
  units; was inconsistent across input rates before this PR.
- Synthetic encode → decode round-trip integration tests
  (`tests/roundtrip.rs`, gated on `test-support` cargo feature).
- VIS detector now preserves leading post-stop-bit audio for the decoder
  (was previously lost, breaking line alignment).

### Deferred
- Mid-image VIS detection: requires VIS-window re-alignment after a previous
  burst's residual buffer transition; tracked as a follow-up issue.

### Added (PR-1)
- VIS header detection via `Goertzel` filter + parity check (`src/vis.rs`).
- `SstvDecoder` now emits `SstvEvent::VisDetected` when a clean PD120 or
  PD180 VIS burst is recognized in the audio stream.
- 92% per-file coverage gate via `cargo-llvm-cov`.
- Property tests against arbitrary audio inputs.

### Added (PR-0)
- Public API skeleton: `SstvDecoder`, `SstvEvent`, `SstvImage`, `SstvMode`,
  `ModeSpec`, `Error`, `Result`, `Resampler`.
- Mode specifications for PD120 and PD180 (timing constants translated
  from slowrx's `modespec.c`).
- Crate-level rustdoc with a runnable `## Example` snippet.
- MIT-licensed crate with ISC notice preservation for slowrx attribution.

### Fixed (PR-0 CR round 1)
- `SstvImage::pixel` and `SstvImage::put_pixel` now degrade gracefully
  (return `None` / no-op) when caller-mutated metadata desyncs from the
  pixel buffer length. Previously could panic from safe code paths.
- `modespec.rs` module rustdoc clarified — V2 modes are planned, not
  "listed in the enum."
