# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (Phase 1: VIS parity)

Faithful rewrite of `vis.rs` to mirror slowrx's `vis.c` algorithm
(closes #13, #14, #15, #17, #30, #36, #37, #38). The PR-1/PR-2 detector
classified non-overlapping 30 ms Goertzel windows against absolute
1900/1200/1300/1100 Hz tones with a 5× dominance threshold, never saw
the leader's actual frequency, and so could not catch real-radio bursts
mistuned by tens of Hz. The new detector matches slowrx exactly:

- 10 ms hop / 20 ms Hann-windowed sliding window (slowrx vis.c lines 30,
  45-48). Closes #13 (no Hann window) and #14 (no overlap).
- 512-FFT (zero-padded from 220 samples), Gaussian-log peak interpolation
  in 500-3300 Hz (slowrx vis.c lines 54-70). Closes #37 (Goertzel-bank
  vs FFT) and #30 (FIR-transient sensitivity — overlap dominates).
- 45-entry circular frequency history feeding a 9-iteration `i × j`
  pattern matcher with **relative** ±25 Hz tolerance from the observed
  leader (slowrx vis.c lines 82-104). Closes #15 (absolute-vs-relative
  tolerance), #36 (5× dominance threshold), #38 (single-alignment scan).
- `HedrShift = leader_observed - 1900` is now extracted at VIS time
  (slowrx vis.c line 106) and plumbed through `SstvEvent::VisDetected`,
  the decoder state, `decode_pd_line_pair`, `PdDemod::pixel_freq`
  (peak-search range), and `freq_to_luminance` (luminance base) — so a
  mistuned radio's pixel band shifts in lock-step. Closes #17.
- New tests: `detects_pd120_with_50hz_offset`,
  `detects_pd180_with_minus_70hz_offset`,
  `handles_misaligned_burst` (off-grid pre-silence proves overlap),
  `rejects_constant_off_band_tone`, `pixel_freq_with_hedr_shift`,
  `freq_to_luminance_with_hedr_shift_scales_band`.
- `synth_vis` is now continuous-phase across tone boundaries — phase
  discontinuities at bit edges previously pulled FFT peaks off-tone
  enough to mask alignment bugs in 30 ms-window unit tests.

`SstvEvent::VisDetected` gains a `hedr_shift_hz: f64` field. The struct
is `#[non_exhaustive]` so this is additive at the wire level for callers
that match by name; positional `match` patterns must add the new field.
The synthetic round-trip integration tests for PD120/PD180 still pass
unchanged (max_diff ≤ 25, mean < 5 per-channel against encoder source).

### Fixed (PR-2 CR round 3)
- Polyphase FIR resampler: `build_kernel` was generating a single
  fixed-phase kernel and `process()` was dropping `center.fract()`,
  making the resampler a quantized integer-delay FIR rather than a
  true fractional-delay polyphase. For non-integer ratios (48k →
  11.025k), this injected ~0.5 samples of phase jitter per output =
  ~10 Hz wobble at the recovered tone = ~3 grey levels of pixel
  noise. Now computes each tap on-the-fly using `frac = phase.fract()`
  to shift the sinc, with the Hann window staying anchored to the
  kernel grid. Added a 48k → 11.025k quality test asserting >50×
  SNR ratio at 1900 Hz that would have caught this regression.

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
