# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (Phase 2: FindSync parity)

Faithful translation of slowrx's `sync.c::FindSync` (Hough-transform
slant correction + 8-tap convolution edge-find for line-zero `Skip`)
into a new `src/sync.rs` module, plus a refactor of the decoder's
`Decoding` state to a two-pass flow that mirrors slowrx's offline-batch
algorithm within our streaming model. Closes #19, #20, #21, #22.

**Background.** Phase 1 fixed VIS detection. Real-audio decoding still
silently failed at line ~248 of PD180 because the line decoder assumed
the working sample rate was exactly 11025 Hz (no drift correction), that
line 0 starts at sample 0 of the residual buffer (no `Skip`
computation), and that there is no settling gap between the VIS stop
bit and the first sync pulse. Sound-card clock drift over 248 line
pairs accumulates past the captured audio length, exiting the per-pair
loop early without `ImageComplete`.

**What landed.**

- New `src/sync.rs` module:
  - `SyncTracker::has_sync_at` — translates slowrx `video.c:271-297`'s
    `Praw`/`Psync` ratio probe with a 16-sample Hann window into a
    256-bin zero-padded FFT (matching slowrx's 64-sample window into a
    1024-bin FFT at the time-span / bin-density level).
  - `find_sync` — translates slowrx `sync.c:18-133`: `(150-30)/0.5 = 240`
    half-degree Hough bins on a `700 × NumLines` sync image, the slant
    deadband described below, an 8-tap `[1,1,1,1,-1,-1,-1,-1]`
    convolution edge-find on the column accumulator, and the
    `xmax/700 × LineTime - SyncTime` skip formula. The Scottie offset
    branch (`sync.c:123-125`) is unreachable in V1's PD-only port and
    is omitted.

- Decoder refactor (`src/decoder.rs`):
  - The `Decoding` state accumulates one image's worth of audio
    (`image_lines/2 × line_seconds × work_rate`, matching
    `video.c:252`'s `Length = LineTime × NumLines/2 × 44100` for the
    PD-mode `NumChans == 4` path).
  - During accumulation the audio is probed every `SYNC_PROBE_STRIDE`
    samples through `SyncTracker`, building a `Vec<bool>` equivalent
    to slowrx's `HasSync[]` global.
  - Once the audio target is reached, `find_sync` runs once. The
    returned `(rate, skip)` drives a single per-pair decode pass:
    `pair_start_sample = round(pair × line_seconds × rate) + skip`.
  - `LineDecoded` events now fire in a fast burst at end-of-buffer
    rather than incrementally. Callers still get every event; the
    timing shifts.
  - `DecodingState` is boxed inside the `State` enum
    (`clippy::large_enum_variant` balance — the unit `AwaitingVis`
    variant should not pay the FFT-plan + audio-buffer footprint).
  - Trailing audio after `ImageComplete` still flows back into a fresh
    `VisDetector` for the ARISS multi-image case.

- Per-pair decode (`src/mode_pd.rs::decode_pd_line_pair`):
  - Now accepts `(rate_hz, pair_start_sample)` rather than assuming the
    buffer begins at the pair's first sample. Channel start times are
    computed at the corrected rate so per-pair drift is absorbed by
    `Skip` + `Rate`. Channel-slice isolation (preventing FFT bleed
    across channel edges) is preserved.

**Hough deadband at 90°.** Slowrx's `Rate += tan(deg2rad(90 -
slantAngle)) / LineWidth × Rate` (`sync.c:81`) runs unconditionally,
even when the angle is within the lock window. The 0.5° quantization
means a perfectly drift-free input lands at 90.5° on the first try, and
the un-needed correction injects a 0.0085% error that compounds across
xAcc's per-line projection and corrupts the falling-edge convolution.
We added a deadband: if `|angle - 90°| ≤ SLANT_STEP_DEG` the rate is
left untouched. In slowrx this is masked by the second-pass `StoredLum`
re-read (rate-driven pixel times against cached luminance); our
streaming per-pixel-FFT model is more sensitive, so the deadband is
needed for clean synthetic-audio parity. Real-audio drift typically
lands well outside this band, so the correction still applies in
practice.

**Tests.**

- New `sync::tests`: `has_sync_at_detects_1200_hz_burst`,
  `has_sync_at_rejects_1900_hz_tone`, `has_sync_at_rejects_silence`,
  `find_sync_locks_clean_track_to_90_degrees`,
  `find_sync_recovers_known_offset` (10 ms left-shift),
  `find_sync_handles_empty_track` (no panic on a zeroed track).
- Synthetic round-trip integration tests (PD120, PD180) preserved and
  tightened: PD120 max=11 mean=0.75 (previously max ≤ 25, mean < 5);
  PD180 max=11 mean=0.55. Margins improved across the board.
- Test count: 53 → 59 unit tests, 2 round-trip, 1 doctest unchanged.

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
