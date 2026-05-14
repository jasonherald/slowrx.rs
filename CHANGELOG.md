# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Internal

- **Extracted `crate::test_tone`** — the continuous-phase FM tone generator
  (`SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` consts,
  `lum_to_freq`, the `ToneWriter` struct with cumulative-target `fill_to` and
  per-tone-duration `fill_secs` methods) — shared by `pd_test_encoder` /
  `robot_test_encoder` / `scottie_test_encoder` / `vis::tests` (four prior
  copies; audit B9). Tightened the three test-encoder modules to `pub(crate)
  mod` so `slowrx::pd_test_encoder::*` / `robot_test_encoder::*` /
  `scottie_test_encoder::*` are no longer reachable externally; `__test_support`
  switched from `pub use` re-exports to thin `pub fn` wrappers (the sole
  consumer-facing path for the synthetic encoders — audit B10). Plus the
  scottie double-doc-comment fix (E2), pointed smoke tests for `encode_pd` /
  `encode_robot` (F11), and `unreachable!`/`assert!` cleanup in
  `robot_test_encoder` (C17). Pure refactor: identical behavior; the
  `slowrx::__test_support::mode_*::encode_*` paths are stable for existing
  consumers (`tests/roundtrip.rs` unchanged). (#86; audit B9/B10/E2/F11/C17.)

- **Extracted `crate::demod` and `crate::dsp` from `mode_pd.rs` / `snr.rs` /
  `vis.rs`.** `crate::demod` now owns the per-channel demod machinery
  (`ChannelDemod` — renamed from `PdDemod`, `decode_one_channel_into`,
  `pixel_freq`, `freq_to_luminance`, `ycbcr_to_rgb` [`#[doc(hidden)]`],
  `FFT_LEN`, `SNR_REESTIMATE_STRIDE`, `HannBank`, `HANN_LENS`,
  `window_idx_for_snr{_with_hysteresis}`); `crate::dsp` consolidates the
  generic Hann-window builder (4 prior copies), `power(Complex<f32>)→f64`,
  `get_bin`, and `goertzel_power`. `decode_one_channel_into`'s 11-arg
  signature collapses to 5 via new `ChannelDecodeCtx`/`DemodState` structs;
  the dead `chan_bounds_abs` parameter (and the three caller
  `array::from_fn` blocks that fed it) is gone; `time_offset_seconds`
  renamed to `radio_frame_offset_seconds`. Pure refactor: identical
  behavior, no public-API change. (#85; audit B1/B3/B5/B8/B16/C6/C20.)

## [0.5.2] - 2026-05-11

The first two items off the post-V2.4 code-review audit backlog (epic #97):
**VIS detector fidelity** (#89) and **multi-image streaming** (#90). Adds the
`SstvEvent::UnknownVis` event for unrecognized-but-well-formed VIS bursts; the
only removal is the never-constructed `Error::UnknownVisCode` variant. No
change to decodable modes — still PD120/180/240 + Robot 24/36/72 + Scottie
1/2/DX + Martin 1/2.

### Added

- **`SstvEvent::UnknownVis { code, hedr_shift_hz, sample_offset }`** — a VIS
  burst that parses and passes parity but maps to no decodable SSTV mode is now
  surfaced to callers (previously dropped silently). `match_vis_pattern` also
  keeps searching its 9 alignments for a *known* code before falling back to an
  unknown one, matching slowrx (#89 A3/C1).

### Fixed

- **Stale VIS detector after an unknown VIS code** — `SstvDecoder::process`
  drained the detector's residual buffer but did not reconstruct it, violating
  the `#40` re-anchor contract (`hops_completed` / `history` were carried over
  into the next detection). It is now reseeded via the same `restart_vis_detection`
  helper the post-image path uses (#89 A1).
- **Multi-image streams now decode all images within a single `process()` call.**
  After `ImageComplete`, `SstvDecoder::process` re-enters VIS detection in-place
  (the loop continues into `AwaitingVis` instead of `break`-ing), so a
  back-to-back transmission's next VIS — and the image after it — surface in the
  same call instead of requiring further `process()` calls (#90 A2). Note:
  `sample_offset` on detections *after* the first is relative to the
  carry-forward start, not absolute (#99).

### Performance

- After `ImageComplete`, the VIS detector is re-armed on only the last few scan
  lines of the decoded image audio (plus everything past), not the entire
  ~1.4–3 M-sample buffer — no more FFT-scanning ~10⁴ sliding-window hops of
  decoded video tones for a VIS burst that can't be there (#90 D4).

### Removed

- **`Error::UnknownVisCode`** — never constructed (`SstvDecoder::process` does
  not return `Result`; unknown codes are now reported via `SstvEvent::UnknownVis`).

### Internal

- `R12BW_VIS_CODE` named constant replacing bare `0x06` literals; faithful
  parity-check shape in `match_vis_pattern` (flip the accumulated parity for
  R12BW, matching slowrx `vis.c:116`) (#89 C5/C15).

## [0.5.1] - 2026-05-05

Patch release bundling the `slowrx-cli` mode-tag fix discovered during
V2.5 Zarya real-radio validation work, plus the negative-regression
integration tests added under #79. Both ride the `[Unreleased]` queue;
no new crate-public API surface.

### Fixed

- **`slowrx-cli` saved Scottie and Martin images as `img-NNN-unknown.png`** —
  the `mode_tag` match in `src/bin/slowrx_cli.rs` was missing arms for the V2.3
  Scottie 1/2/DX and V2.4 Martin 1/2 variants. Same trap V2.1 fixed for PD240
  and V2.2 caught for Robot 24/36/72; the wildcard arm absorbed the new
  variants silently because `SstvMode` is `#[non_exhaustive]`. Surfaced
  during V2.5 Zarya real-radio capture work when 3 Scottie 1 images decoded
  from a 2013 Wolverine Radio shortwave broadcast all came out tagged
  "unknown". Added the 5 missing arms (`scottie1`, `scottie2`, `scottiedx`,
  `martin1`, `martin2`) plus a unit test
  (`mode_tag_covers_all_known_variants`) that iterates every known
  `SstvMode` variant and asserts each maps to a non-"unknown" tag — the
  next mode-family addition can no longer slip through silently.

### Added

- **Negative-regression integration tests** in `tests/no_vis.rs`:
  - `decoder_no_vis_on_white_noise` — 10 s of deterministic LCG white
    noise at 48 kHz (≈ 0.3 RMS, matching the measured level of an
    ISS Zarya non-SSTV pass).
  - `decoder_no_vis_on_silence` — 10 s of pure silence.

  Both assert `SstvDecoder::process` produces zero
  `SstvEvent::ImageComplete { partial: false }` events without panicking.
  This is the no-signal counterpart to the synthetic round-trip suite —
  both must hold for any release. Inspired by an ISS Zarya capture on
  2026-05-04 that turned out to carry no SSTV (Zarya transmits SSTV
  only during ARISS-scheduled events); see [#67] for the capture
  story.

[#67]: https://github.com/jasonherald/slowrx.rs/issues/67

## [0.5.0] - 2026-05-05

Minor release adding Martin 1 (VIS `0x2C`) and Martin 2 (`0x28`) —
both 320×256 GBR with sync at line start (standard SSTV convention).
Reuses the `ChannelLayout::RgbSequential` infrastructure landed in
V2.3 Scottie. Synthetic round-trip-validated; real-radio Martin
capture validation is async ([#66]).

[#66]: https://github.com/jasonherald/slowrx.rs/issues/66

### Added

- **`SstvMode::Martin1`** (VIS `0x2C`), **`Martin2`** (`0x28`).
- Two new `ModeSpec` consts: `MARTIN1`, `MARTIN2`. Values
  transcribed from slowrx C `modespec.c:39-63`.

### Changed

- **`mode_scottie::decode_line`** now branches on
  `spec.sync_position` for `chan_starts_sec`. The `Scottie` branch
  is unchanged from V2.3; the new `LineStart` branch handles Martin
  via slowrx C `video.c` "default" case offsets (`sync + porch`,
  then `+ chan_len + septr`, then `+ chan_len + septr`).
- **`scottie_test_encoder::encode_scottie`** accepts Martin modes
  and branches per-line tone emission on `spec.sync_position`.
  Martin emission order: `[SYNC][porch][G][septr][B][septr][R]`.
- **Module rustdocs** in `mode_scottie` and `scottie_test_encoder`
  expanded to enumerate both Scottie and Martin families with
  layout diagrams.
- **`src/lib.rs` Status block** bumped from `0.4.x — V2.3 published`
  to `0.5.x — V2.4 published`; Martin 1 / Martin 2 added to the
  implemented-modes list.

### Validation

- Two new synthetic round-trips (`martin1_roundtrip`,
  `martin2_roundtrip`) pass at unchanged `mean < 5.0` per-pixel-
  RGB-diff threshold.
- All 9 prior round-trips (PD120/180/240, R24/36/72, S1/S2/SDX)
  continue to pass at the same threshold — regression net intact.
- Coverage ≥ 92% per-file maintained.

### Notes

- Martin's `SyncPosition::LineStart` routes through the existing
  PD/Robot path in `find_sync`; no `find_sync` changes were needed
  (unlike V2.3 Scottie, which required a new branch).
- Module / function names (`mode_scottie`, `encode_scottie`) stay
  despite the family-scope expansion. Renaming was deferred per
  the V2.4 epic's "Cross-mode shared-helper refactoring beyond
  what naturally falls out of Scottie reuse" out-of-scope clause.
- Real-radio Martin capture validation is async (no reference WAVs
  available yet). [#70] (pixel-diff comparator), [#71] (squiggles),
  and [#77] (SIMD multiversioning) remain pending.

[#70]: https://github.com/jasonherald/slowrx.rs/issues/70
[#71]: https://github.com/jasonherald/slowrx.rs/issues/71
[#77]: https://github.com/jasonherald/slowrx.rs/issues/77

## [0.4.0] - 2026-05-04

Minor release adding the Scottie family — Scottie 1, Scottie 2,
Scottie DX. All three at 320×256, GBR color encoding, with **mid-line
sync** (sync sits between B and R within each radio line, not at
line start). Synthetic round-trip-validated; real-radio Scottie
capture validation is async ([#65]).

[#65]: https://github.com/jasonherald/slowrx.rs/issues/65

### Added

- **`SstvMode::Scottie1`** (VIS `0x3C`), **`Scottie2`** (`0x38`),
  **`ScottieDx`** (`0x4C`).
- **`ChannelLayout::RgbSequential`** — three-channel RGB layout per
  radio line. Shared with V2.4 Martin.
- **`SyncPosition::Scottie`** — sync between B and R, the V2.1
  forcing-function variant cashed in.
- Three new `ModeSpec` consts: `SCOTTIE1`, `SCOTTIE2`, `SCOTTIE_DX`.
  Values transcribed from slowrx C `modespec.c:91-128`.
- New module `crate::mode_scottie` with `decode_line`. Mid-line-sync
  handling lives entirely inside this module; the substantive
  changes outside it are a `find_sync` branch (next item) and a
  Scottie DX–only Hann-window-index bump in
  `mode_pd::decode_one_channel_into`.
- New module `crate::scottie_test_encoder` (gated behind
  `cfg(any(test, feature = "test-support"))`). Synthetic encoder for
  round-trip testing.

### Changed

- **`crate::sync::find_sync`** gains a `SyncPosition::Scottie`
  branch. PD/Robot/Martin land at line-start sync; Scottie's sync is
  mid-line, so after the existing `s = (xmax/700) · LineTime −
  SyncTime` formula we apply `s = s − chan_len/2 + 2·porch` to bring
  `skip_samples` back to the start of line 0's content. This is
  exactly the slowrx C `sync.c:123-125` correction the V2.1
  `SyncPosition` carve-out anticipated. PD/Robot/Martin behavior is
  unchanged (`SyncPosition::LineStart` keeps the existing formula).
- **`crate::mode_pd::decode_one_channel_into`** post-adjusts the
  Hann window index by `+1` when `spec.mode == ScottieDx && idx <
  6`, matching slowrx C `video.c:367` (longer integration for SDX's
  1.08 ms pixel time). Applied after the hysteresis selector tracks
  the un-bumped SNR-derived index, so the bump doesn't compound
  across pixels. No-op for non-SDX modes.
- **`decoder.rs`** dispatch grows an `RgbSequential` arm; the
  `target_audio_samples` match arm gains `RgbSequential =>
  spec.image_lines` (one radio line per image row, like Robot).

### Validation

- Three new synthetic round-trips (`scottie1_roundtrip`,
  `scottie2_roundtrip`, `scottie_dx_roundtrip`) pass at unchanged
  `mean < 5.0` per-pixel-RGB-diff threshold.
- All 6 existing round-trips (PD120/180/240, Robot24/36/72) continue
  to pass at the same threshold — regression net intact.
- Coverage ≥ 92% per-file maintained.

### Notes

- Mid-line sync was V2.1's forcing-function carve-out;
  `SyncPosition::Scottie` makes it explicit at dispatch time so
  future modes can't accidentally inherit a line-start assumption.
- Real-radio Scottie capture validation is async (no reference WAVs
  available yet). The pixel-diff comparator earmarked in [#70] is
  still pending. Squiggle work ([#71]) remains parked.

[#70]: https://github.com/jasonherald/slowrx.rs/issues/70
[#71]: https://github.com/jasonherald/slowrx.rs/issues/71

## [0.3.3] - 2026-05-03

Patch release bumping `crate::snr::FFT_LEN` from 256 to 1024 to give
4× finer Hz/bin (10.77 vs 43.07) for the per-pixel demod and SNR
estimator. Validated visually on the 12 ARISS Fram2 R36 reference
WAVs as noticeably clearer pixel content vs. the 0.3.2 baseline. The
squiggle artifacts ([#71]) are unaffected by this change — they're a
separate concern, tracked through 0.3.5+.

The pixel-diff comparator earmarked for 0.3.3 in [#70] moves to a
later patch.

[#70]: https://github.com/jasonherald/slowrx.rs/issues/70
[#71]: https://github.com/jasonherald/slowrx.rs/issues/71

### Changed

- **`crate::snr::FFT_LEN`** bumped from 256 to 1024. The bump produces
  two coupled DSP changes:
  - **Per-pixel demod** gets 4× finer bin density only.
    `crate::snr::HANN_LENS` is unchanged at slowrx-C-divided-by-4
    (`[12, 16, 24, 32, 64, 128, 256]`), so the Hann is applied to the
    first `HANN_LENS[idx]` samples of the FFT input and the rest is
    zero-padded — time-domain support identical to slowrx C, only the
    FFT bin density changes.
  - **SNR estimator** gets a 4× longer Hann window. `hann_long =
    build_hann(FFT_LEN)` scales with `FFT_LEN`, so the SNR estimator's
    integration window grows from ~23 ms (= slowrx C) to ~93 ms. This
    gives a cleaner SNR estimate and likely reduces window-selector
    flip-flop beyond what the 0.3.2 hysteresis already delivers.
- **`mode_pd::FFT_LEN`** is a re-export of `snr::FFT_LEN`, so it picks
  up the new value automatically.
- **`snr_bandwidth_correction_bins_match_slowrx`** test renamed to
  `snr_bandwidth_correction_bins_at_finer_resolution` and re-asserted
  with the post-bump bin counts (75 / 104 / 278). The test still
  guards the `get_bin` floor-truncation math; it no longer asserts
  slowrx-C-parity in `usize` terms.

### Documentation

- New `docs/intentional-deviations.md` entry: "FFT frequency
  resolution exceeds slowrx C by 4×". Documents both coupled
  deviations (per-pixel bin density and SNR-estimator window length),
  the rationale (visibly clearer real-radio output), and three
  triggers for revisiting.
- In-source rustdoc comments referencing `FFT_LEN=256` updated to
  `FFT_LEN=1024` (in `src/snr.rs` and `src/mode_pd.rs`). Stale claims
  about `HANN_LENS[6]` matching `FFT_LEN` and `hann_long` being
  shared with the bank were dropped — they're now different lengths
  (256 and 1024 respectively).

### Validation

- All 6 synthetic round-trips (PD120/180/240, R24/36/72) pass at the
  unchanged `mean < 5.0` per-pixel-RGB-diff threshold.
- Real-audio Fram2 visual validation: all 12 slides reproduce the
  experiment-branch "WAY clearer but still squiggles" finding versus
  the 0.3.2 baseline.
- Wall-clock decode of one Fram2 R36 slide: 0.614 s on v0.3.2 (avg
  of 3 runs) → 1.223 s on 0.3.3 (~2× slower; negligible since R36
  transmits over ~36 s).

### Notes

- The squiggle artifacts in real-radio Fram2 output ([#71]) are
  reduced (per 0.3.2 hysteresis) but still present. The two new
  diagnostic patterns observed during 0.3.2 validation
  (black-background dependence, top/bottom asymmetry) motivate the
  next investigation pass; they are 0.3.4+ work.

## [0.3.2] - 2026-05-02

Patch release adding 1 dB SNR hysteresis to the adaptive Hann window
selector. Targets the threshold flip-flop hypothesized in [#71]'s
code-only audit as a contributor to V2.2 real-radio Robot 36 squiggle
artifacts. Plus two stale-doc cleanups the audit surfaced.

[#71]: https://github.com/jasonherald/slowrx.rs/issues/71

### Added

- **`crate::snr::window_idx_for_snr_with_hysteresis(snr_db, prev_idx)`**
  — `pub(crate)` wrapper around the existing pure-threshold
  `window_idx_for_snr`. Applies a 1 dB hysteresis band at each
  threshold (±0.5 dB on each side) by re-evaluating the lookup at a
  pessimistically-shifted SNR and only accepting changes that survive
  both lookups. Six new unit tests in `snr.rs::tests` cover the band
  edges and the symmetric in-band/robust transitions.

### Changed

- **`mode_pd::decode_one_channel_into`** now threads a local
  `prev_win_idx` through the per-FFT lookup and calls
  `window_idx_for_snr_with_hysteresis` instead of the bare
  `window_idx_for_snr`. State is local to one channel decode — no
  `DecodingState` plumbing.
- **Deliberate divergence from slowrx C** (`video.c:354-367`), which
  uses pure-threshold logic with no hysteresis. Documented in
  `docs/intentional-deviations.md` under "SNR hysteresis on adaptive
  Hann window selection."

### Documentation

- **`mode_pd::decode_pd_line_pair` doc block** refreshed: the V1
  deferral #44 (hardcoded `HANN_LENS[6]`) was lifted in PR #60 / V2.1
  Phase 3 but the doc block at `mode_pd.rs:259-266` still claimed the
  deferral was in effect. Updated to describe the current
  SNR-adaptive + hysteresis behavior.
- **`mode_pd::decode_one_channel_into` doc** had a stale cross-
  reference to the (now-renamed) `#18 deferral note`. Refreshed to
  point at the new `#44 lifted with hysteresis (0.3.2)` note and the
  hysteresis function.
- **`docs/intentional-deviations.md`** gains a new entry for the
  hysteresis: rationale (squiggle period matches SNR re-estimation
  cadence in [#71]'s audit), the algorithmic divergence from slowrx
  C, and three triggers for revisiting.

### Validation

- All 6 synthetic round-trips (PD120/180/240 + R24/36/72) continue
  passing at the same `mean < 5.0` per-pixel-RGB-diff threshold —
  hysteresis is a no-op for synthetic audio (synthetic SNR doesn't
  fluctuate cadence-to-cadence).
- **Real-audio Fram2 visual validation: partial improvement.**
  Numerical diff vs the 0.3.1 baseline shows 0.58–1.41% of pixels
  changed per slide, with diffs clustering at image edges
  (consistent with hysteresis fixing the flip-flop component of the
  artifact). Visually, the squiggles are noticeably reduced but not
  eliminated, and exhibit two residual patterns the visual review
  surfaced — they appear more strongly on black/dark backgrounds, and
  vary in intensity between the top, middle, and bottom thirds of the
  image. Both observations point at root causes other than SNR-flip-
  flop (peak-interp boundary-clip behavior at low signal-frequency,
  and find-sync rate-correction precision at image-time-extremes
  respectively). [#71] stays OPEN with these findings as the next
  iteration's evidence base. The hysteresis itself is a real
  improvement (no regression risk) and ships in 0.3.2.

## [0.3.1] - 2026-05-02

Patch release bundling three small follow-up items from V2.2 review
cycles (no functional changes; pure cleanup).

### Fixed

- **`chroma_planes` over-allocation for R72.** `DecodingState` was
  allocating ~150 KiB of cross-radio-line chroma side buffer for any
  `ChannelLayout::RobotYuv` mode, but R72 composes RGB in-place and
  never reads the planes. Now allocated only for R24/R36 (where chroma
  duplication actually requires the side buffer). Saves ~150 KiB per
  R72 decode.

### Changed

- **`pd_modes_have_zero_septr_seconds` test extended to cover Pd240.**
  Pre-existing gap from V2.1 — the test was never extended after Pd240
  was added. Robot has non-zero `septr_seconds` so the PD-family
  invariant doesn't generalize to it; the test stays PD-specific.

### Documentation

- **`SstvEvent::LineDecoded` rustdoc** now documents the R36/R24
  partial-chroma emission semantics: row 0's Cb is at zero-init when
  LineDecoded fires (no previous radio line to duplicate from);
  faithful to slowrx C's `calloc`'d image buffer behavior. Final
  `ImageComplete` carries the populated buffer.

## [0.3.0] - 2026-05-02

V2.2 — Robot family mode coverage. Adds Robot 24 (`SstvMode::Robot24`,
VIS `0x04`), Robot 36 (`SstvMode::Robot36`, VIS `0x08`), and Robot 72
(`SstvMode::Robot72`, VIS `0x0C`). First V2 release that introduces a
non-PD decoder; introduces the cross-mode-family dispatch refactor in
`decoder.rs`.

Closes V2.2 epic ([#64]). Tracks the V2 umbrella ([#9]).

[#9]: https://github.com/jasonherald/slowrx.rs/issues/9
[#64]: https://github.com/jasonherald/slowrx.rs/issues/64
[#70]: https://github.com/jasonherald/slowrx.rs/issues/70
[#71]: https://github.com/jasonherald/slowrx.rs/issues/71

### Added

- **Robot 24, Robot 36, Robot 72 modes.** All three at 320×240 image
  resolution. Timing constants from slowrx `modespec.c:130-167`. R36/R24
  share decoder code (chroma alternation + neighbor-row duplication per
  slowrx `video.c:182-191`, `:421-425`); R72 uses the simpler 3-channel
  Y/U/V sequential layout per `video.c:60-101` default case.
- **`ChannelLayout::RobotYuv` enum variant** covering all three Robot
  modes. Per-mode chroma topology lives inside `mode_robot.rs` (the
  internal mode-match mirrors slowrx's `switch(Mode)` cases).
- **`src/mode_robot.rs`** — new Robot-family decoder. Reuses
  `mode_pd::decode_one_channel_into` (visibility bumped from private to
  `pub(crate)`; the parameter `pair_seconds` was renamed to mode-
  agnostic `time_offset_seconds`) and `mode_pd::ycbcr_to_rgb`.
- **`src/robot_test_encoder.rs`** — synthetic encoder for round-trip
  testing. Mirrors `pd_test_encoder.rs` shape.
- **Per-mode chroma planes side buffer** on `DecodingState`
  (`chroma_planes: Option<[Vec<u8>; 2]>`). Allocated only for
  `ChannelLayout::RobotYuv`; lets R36/R24 compose RGB after both chroma
  channels (own + duplicated-from-neighbor) are present.
- **`tests/ariss_fram2_validation.md`** — committed procedure for the
  V2.2 real-audio merge gate (decode the 12 ARISS Fram2 reference WAVs
  and visually compare against the 12 reference JPGs).

### Changed

- **`decoder.rs::run_findsync_and_decode`** now dispatches on
  `ChannelLayout`. PD path is byte-identical to V2.1 (zero PD120/180/240
  regression risk). Robot path loops per image line and calls
  `mode_robot::decode_line`.
- **`target_audio_samples` computation** now branches on
  `spec.channel_layout`: PD (line pairing) keeps `image_lines / 2 ×
  line_seconds`; Robot (no pairing) uses `image_lines × line_seconds`.
  Mirrors slowrx C `video.c:251-254`. Surfaced during Phase 5 Fram2
  validation — the prior unconditional PD-style formula had been
  cutting Robot decode in half on real audio.
- **`pd_modes_have_line_start_sync_position` test renamed to
  `all_v2_modes_have_line_start_sync_position`** and extended to cover
  all six current modes (PD + Robot).
- **`bin/slowrx_cli.rs::mode_tag`** — added `Robot24`/`Robot36`/`Robot72`
  arms (same trap V2.1 PD240 fell into; caught pre-merge this time).

### Tests

- `tests/roundtrip.rs::robot72_roundtrip`,
  `tests/roundtrip.rs::robot36_roundtrip`,
  `tests/roundtrip.rs::robot24_roundtrip` — synthetic round-trip with
  mean per-pixel-diff threshold < 5.0 (same threshold as PD).
- `src/decoder.rs::tests::process_emits_vis_detected_for_robot{24,36,72}_burst`
  — VIS-detection unit tests.

### Validation

- Validated against 12 ARISS Fram2 Robot 36 captures
  (<https://ariss-usa.org/ARISS_SSTV/Fram2Test/>) on 2026-05-02 — all 12
  produced visually-matching PNGs after the Phase 5 `target_audio_samples`
  fix landed. Procedure documented at `tests/ariss_fram2_validation.md`.
- Faint vertical squiggle artifacts in real-radio output (not present in
  the reference JPGs) are tracked as a parity gap for a future quality
  pass at [#71].

### Known caveats

- **Robot 24 ships without R24-specific real-radio evidence.** Inherits
  R36 validation by structural identity (R24 and R36 share decoder code;
  only the mode tag and VIS code differ — timing constants are
  bit-identical).
- **Robot 72 ships with synthetic-only coverage.** No public R72 capture
  was sourced during V2.2 brainstorm. Real-radio fixture pending; tracked
  in [#70].
- **Row 0 chroma artifact in R36/R24.** Image row 0 has Y own + Cr own
  + Cb at zero-init (no previous radio line to duplicate Cb forward).
  Faithful to slowrx C, which writes `Image[][][]` with `calloc` and
  never updates row 0's Cb. Visible as a faint color cast on the very
  top row of the decoded image. Documented in
  `mode_robot::decode_r36_or_r24_line`.

---

For releases **0.2.x and earlier** (V1 launch through V2.1 PD240
post-merge cleanup), see [`docs/history.md`](docs/history.md).
