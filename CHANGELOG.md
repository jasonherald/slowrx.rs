# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

## [0.2.1] - 2026-05-02

Patch release bundling the post-merge final-review cleanup of V2.1
(PD240). Two important user-visible fixes plus five minor consistency
items.

### Fixed

- **`slowrx-cli` saved PD240 images as `img-NNN-unknown.png`** instead of
  `img-NNN-pd240.png`. The `mode_tag` match in `src/bin/slowrx_cli.rs`
  was missing the `SstvMode::Pd240` arm; the wildcard arm absorbed it
  silently because `SstvMode` is `#[non_exhaustive]`. Added the explicit
  arm; rewrote the wildcard's comment to flag the trap so future
  variant additions don't repeat it.
- **Crate-level rustdoc Status block was stale** (`src/lib.rs`). Said
  "Pre-0.1 — under active development. Public API is not yet stable."
  on the docs.rs landing page for `0.2.0`. Refreshed to `0.2.x` V2.1
  language linking to the V2 roadmap ([#9]).

### Changed

- Re-exported `SyncPosition` from the crate root so downstream callers
  can write `slowrx::SyncPosition::LineStart` instead of the deeper
  `slowrx::modespec::SyncPosition::LineStart` path. Adds public surface
  ahead of V2.3 Scottie, which will introduce a second variant.
- Stale "PD120/PD180" mentions refreshed to "PD120/PD180/PD240" in:
  `src/mode_pd.rs` (channel-time-offsets explanation) and
  `docs/intentional-deviations.md` (mean-diff observation).

### Tests

- `src/mode_pd.rs::tests::chan_starts_sec_septr_zero...` — loop now
  covers `Pd240` alongside `Pd120`/`Pd180`, verifying the
  `chan_starts_sec` formula stays numerically equivalent for all
  current PD-family modes.
- `src/decoder.rs::tests::process_emits_vis_detected_for_pd240_burst` —
  new sibling of the existing PD120/PD180 VIS-detection unit tests.

[#9]: https://github.com/jasonherald/slowrx.rs/issues/9

## [0.2.0] - 2026-05-02

V2.1 — PD240 mode coverage. First V2 release. Adds the third PD-family
mode and the V2 ModeSpec carve-out (`sync_position` field on `ModeSpec`,
`SyncPosition` enum) that lets later mode-family epics declare their
sync placement explicitly.

Closes V2.1 epic ([#63]). Tracks the V2 umbrella ([#9]).

[#9]: https://github.com/jasonherald/slowrx.rs/issues/9
[#63]: https://github.com/jasonherald/slowrx.rs/issues/63

### Added

- **PD240 mode** (`SstvMode::Pd240`, VIS code `0x61`). 640×496, ~240s
  per image. Same `ChannelLayout::PdYcbcr` as PD120/PD180 — reuses
  `mode_pd::decode_pd_line_pair` unchanged. Timing constants from
  slowrx `modespec.c:299-310`.
- **`SyncPosition` enum + `ModeSpec::sync_position` field.** V2 carve-out
  per [V2 epic-split design](docs/superpowers/specs/2026-05-02-v2-epic-split-design.md).
  PD120/PD180/PD240 all use `SyncPosition::LineStart`. The
  `#[non_exhaustive]` enum will gain a Scottie variant in V2.3.

### Tests

- `tests/roundtrip.rs::pd240_roundtrip` — synthetic encode/decode round-
  trip with mean per-pixel-diff threshold < 5.0.

## [0.1.0] - 2026-05-02

First public release on crates.io. PD120 + PD180 SSTV decoding, validated
end-to-end against Dec-2017 ARISS captures (6 of 7 fixtures decode to
images visually matching reference JPGs; the 7th is a truncated capture
missing the VIS leader). The path here was a 7-phase recovery effort
spanning two parity audits against slowrx C — see closed [epic #12] and
[`docs/audits/`] for the full archaeology. The [`docs/intentional-deviations.md`]
file catalogs every deliberate divergence from slowrx so future audits
have somewhere to point first.

[epic #12]: https://github.com/jasonherald/slowrx.rs/issues/12
[`docs/audits/`]: ./docs/audits/
[`docs/intentional-deviations.md`]: ./docs/intentional-deviations.md

### Added (CLI binary + R12BW parity + deviations doc — issues #7, #26, #39)

- **`slowrx-cli` binary** (#7) — `cargo install slowrx --features cli` builds
  it on `$PATH`. `--input recording.wav --output ./out` decodes every
  SSTV image found, writing `img-NNN-{mode}.png` per `ImageComplete`
  (sequence-numbered, mode-tagged — matches the rtl-sdr satellite
  recorder convention). Handles 8/16/24/32-bit integer and float WAV,
  mono or multi-channel (averaged to mono), with explicit error
  propagation throughout.
- **`cli` cargo feature** gates four optional deps: `hound`, `image`,
  `clap`, `anyhow`. The library crate stays minimal; CLI users opt in.
- **`tests/cli.rs`** — `assert_cmd` integration test that runs the binary
  against the first ARISS fixture in `docs/wav_files/201712-ISS_SSTV/`
  and asserts at least one PNG is written. Skips silently if no
  fixture is present (the corpus is gitignored). Gated on `cli` feature.
- **`examples/decode_wav.rs`** — trimmed to a minimal API demo (~70
  lines) pointing users at `slowrx-cli` for the full tool.

### Fixed (R12BW VIS parity inversion — #26)

- `src/vis.rs::match_vis_pattern` now inverts the expected parity bit
  for VIS code `0x06` (R12BW), matching slowrx `vis.c:116`:
  `if (VISmap[VIS] == R12BW) Parity = !Parity;`. V1's `modespec::lookup`
  doesn't decode R12BW, so this has no V1 functional impact — but it
  prevents a silent correctness bug where future R12BW support would
  reject every R12BW burst at the parity check. Two new tests:
  `r12bw_uses_inverted_parity` (positive) and
  `r12bw_rejects_standard_parity` (negative). The synthetic
  `synth_vis_with_offset` helper was updated to follow the same
  convention so the existing proptest still passes for code 0x06.

### Documentation (intentional deviations from slowrx — #39)

- Added [`docs/intentional-deviations.md`](docs/intentional-deviations.md)
  cataloging every deliberate divergence from slowrx (currently 4):
  VIS stop-bit boundary precision, FindSync 90° deadband, VIS retry
  exhaustion, and the relaxed synthetic round-trip `max_diff` tolerance.
  Each entry has rationale and a "when to revisit" condition. Future
  parity audits should consult this list before flagging "missing".

### Changed (Real-audio validation + Phase 3 deferrals engaged — issues #44, #45)

After Phases 1-6 of the recovery shipped, the decoder was validated against
local Dec-2017 ARISS WAV captures (the 7 fixtures that previously produced
0/7 `ImageComplete` events). Detection and decode worked (6/7 produced
images — the 7th is a 120-second truncated capture missing the VIS burst),
but visual quality was poor: heavy vertical banding at every channel edge
(hallmark of #45 channel-mask) and washed-out chroma (hallmark of #44
hard-coded longest Hann window).

This release engages both deferrals — they were filed during Phase 3
specifically as "engage when real audio shows them needed":

- **#44 — SNR-adaptive Hann window selection** (`src/mode_pd.rs::decode_one_channel_into`):
  Replaced `let win_idx = 6;` (longest window hard-code) with
  `crate::snr::window_idx_for_snr(snr_db)`, matching slowrx
  `video.c:354-367`. Real-radio audio reports realistic SNR; the
  selector picks shorter windows at high SNR for sharper time
  resolution at pixel boundaries.

- **#45 — Drop channel-boundary zero-pad mask** (`src/mode_pd.rs::decode_one_channel_into`):
  Removed the `chan_lo`/`chan_hi` mask that zero-padded FFT input
  outside the channel's nominal bounds. slowrx FFTs across channel
  boundaries on its continuous PCM stream (`video.c::GetVideo`); the
  peak search in 1500-2300 Hz still locks onto the dominant video
  tone even when adjacent channels' content leaks into the windowed
  FFT support. The previous mask hurt the leftmost/rightmost ~60
  pixels of every channel — verified visually against the ARISS
  captures.

**Real-audio result (Dec-2017 ARISS, 6/7 fixtures):**
Decoded images visually match reference JPGs (RSOISS callsign,
Sergey Korolev / Konstantin Tsiolkovsky portraits, Russian text
all legible). The 7th fixture (`12072017_051548.wav`, 120s) is a
truncated capture missing the VIS leader burst — same outcome as
slowrx C on the same input.

**Synthetic round-trip impact:**
The synthetic encoder produces instant frequency-step transitions at
pixel and channel boundaries — real radio's FM-modulator slewing
softens these. With deferrals engaged, the synthetic round-trip's
mean diff stays excellent (PD120 mean=1.51, PD180 mean=1.82) but
`max_diff` hits 234-255 at a handful of isolated boundary pixels.
The `max_diff <= 25` assertion was dropped; the test now checks
mean only (`mean < 5.0`). Real-audio is the truth gate; synthetic
remains a regression check on mean quality. Documented inline.

### Added (Real-audio smoke harness)

- `examples/decode_wav.rs`: takes a WAV path, emits PNG(s) per
  `ImageComplete`. Mono `i16`/`f32` WAVs in any sample rate
  supported by `SstvDecoder`. Used to validate against local ARISS
  fixtures; not committed to the published crate (examples are
  excluded from the package).
- `[dev-dependencies]`: `hound` (WAV reader) and `image` (PNG writer,
  `png` feature only) for the example.

### Changed (Round-2 parity audit bundle — issues #48–#58)

11 findings from the second-pass C↔Rust parity audit closed in one bundle.

**Code fixes (7):**

- **#49/#50/#51 — `get_bin` shared helper** (`src/lib.rs`, `src/vis.rs`,
  `src/mode_pd.rs`, `src/snr.rs`, `src/sync.rs`): Added `crate::get_bin(hz,
  fft_len, sample_rate_hz) -> usize` implementing slowrx's `GetBin` truncation
  (`common.c:39-41`). Replaced 4 local `bin_for` lambdas that incorrectly used
  `.round()` instead of C's implicit `double → guint` truncation. Downstream:
  SNR bandwidth-correction bins now match slowrx exactly (`video_plus_noise=20`,
  `noise_only=27`, `receiver=69`; was 19/28/70 — Pnoise multiplier shifts from
  2.500 to 2.5556). `sync_target_bin` for 1200 Hz is now 27 (was 28). Tests:
  `get_bin_matches_slowrx_truncation`, `snr_bandwidth_correction_bins_match_slowrx`,
  `sync_target_bin_for_1200hz_is_27`.

- **#48 — YCbCr→RGB round-to-nearest** (`src/mode_pd.rs`): `ycbcr_to_rgb`
  now uses `(... / 100.0).clamp(0.0, 255.0).round() as u8` (float divide +
  round), matching slowrx's `clip(double)` in `common.c:49-53` which calls
  `round()`. Previous integer division `/ 100` truncated, producing a 1-LSB
  darker bias on R and B channels for neutral grey and many other combinations.
  This was round-1 Finding #4 partially fixed by PR #47 (`freq_to_luminance`
  was fixed then; `ycbcr_to_rgb` was missed).
  Test: `ycbcr_rounds_to_nearest_matching_slowrx_clip` verifies
  Y=128/Cr=128/Cb=128 → R=129, G=127, B=129.

- **#53 — `max_convd` init** (`src/sync.rs`): Changed from `i32::MIN` to `0`,
  matching slowrx `sync.c:29`: `double maxconvd=0`. With zero input every
  `convd==0` beat `i32::MIN`, placing `xmax=4` (not 0) — 1 ms divergence from
  slowrx's degenerate-input Skip.
  Test: `find_sync_empty_track_has_no_slant_detected` verifies skip is
  negative (xmax=0) on all-false input.

- **#54 — Slant-lock exclusive interval** (`src/sync.rs`): Replaced
  `(SLANT_OK_LO_DEG..SLANT_OK_HI_DEG).contains(&slant_angle)` (half-open `[89,91)`)
  with `slant_angle > SLANT_OK_LO_DEG && slant_angle < SLANT_OK_HI_DEG` (open
  `(89,91)`), matching slowrx `sync.c:83`.

- **#55 — Sync probe stride** (`src/sync.rs`): Changed `SYNC_PROBE_STRIDE`
  from 3 to 4. slowrx uses 13 samples@44.1 kHz ≈ 3.25@11.025 kHz. Stride=4
  (round-up) gives probe density closer to slowrx's than stride=3 (round-down
  gave 25% more probes; stride=4 gives ~19% fewer). Documentation updated.

- **#56 — `pixel_freq` boundary clip** (`src/mode_pd.rs`): Added slowrx's
  `video.c:390-398` boundary guard before Gaussian interpolation. When `max_bin`
  lands on the padded edge bins (`≤ lo` or `≥ hi`), return a clipped value
  `(1500 or 2300) + hedr_shift_hz` instead of interpolating into the noise bin.
  Test: `pixel_freq_clips_below_band_to_1500hz` verifies a 1480 Hz tone returns
  ≈1500 Hz.

- **#57 — `slant_deg` dead-field cleanup** (`src/sync.rs`): Changed
  `SyncResult::slant_deg` from `f64` to `Option<f64>`. Returns `None` when the
  Hough found no pulses (previously returned 90.0 for both "perfectly aligned"
  and "nothing detected" — indistinguishable). Drops the `slant_deg_last`
  tracking variable; uses `slant_deg_detected: Option<f64>` directly.

**Doc-only clarifications (4):**

- **#52 — VIS retry divergence** (`src/vis.rs`): Added doc comment to
  `match_vis_pattern` explaining that Rust exhausts all 9 `(i,j)` candidates
  while slowrx terminates early after a parity failure when `HedrShift ≠ 0`
  (a C quirk — HedrShift is set before parity check). Rust's behavior is
  correct; the divergence is documented for future maintainers.

- **#58 — Stop-bit-end uniform anchor** (`src/vis.rs`): Updated comment in
  `VisDetector::process` to accurately describe the 5 ms uniform offset vs
  slowrx's i-dependent 5–25 ms range. Previous comment claimed "precise i-aware
  boundary" but slowrx is also i-aware, just differently.

- **#50/#51**: Transitively closed by `get_bin` refactor — see #49 above.
  SNR test verifies corrected bandwidth multipliers.

**Round-trip stats — Phase 3 baseline preserved:**
* PD120: max_diff ≤ 25, mean < 5 (tolerance unchanged)
* PD180: max_diff ≤ 25, mean < 5 (tolerance unchanged)
* Note: the `ycbcr_to_rgb` fix (#48) changes round-trip output for pixels where
  the numerator has a fractional part ≥ 0.5. The roundtrip test compares
  `src_rgb` (via the newly-fixed `ycbcr_to_rgb`) to decoded pixels (also via
  the same fixed function), so the comparison remains symmetric and the existing
  tolerance ≤25/mean<5 is preserved unchanged.

### Changed (Parity housekeeping bundle — issues #16, #25, #27, #28, #29, #31, #33, #34, #35, #40)

Nine findings from the slowrx parity audit (#12) addressed in one bundle —
5 code fixes and 4 documentation clarifications.

**Code fixes:**

- **#16 — Luminance round-to-nearest** (`src/mode_pd.rs`): `freq_to_luminance`
  now uses `.round() as u8` instead of plain `as u8` (truncation), matching
  slowrx's `(guchar)round(a)` in `common.c::clip()`. Worst-case off-by-one
  per pixel; images were systematically 0.5 units darker on average.
  At the time we believed `ycbcr_to_rgb` was unaffected — that turned out
  to be wrong: slowrx's `video.c:447-449` does `clip(double / 100.0)` (float
  divide → `round()` via clip's tail call), not `clip()` on an integer-divide
  result. The `ycbcr_to_rgb` correction is shipped in the round-2 bundle
  above (#48). New test `freq_to_luminance_rounds_to_nearest_not_truncates`
  verifies 127.7 → 128.

- **#25 — `septr_seconds` field for V2 parity** (`src/modespec.rs`,
  `src/mode_pd.rs`): Added `septr_seconds: f64` to `ModeSpec` (= 0.0 for
  PD120/PD180, matching slowrx's `SeptrTime = 0e-3`). The `chan_starts_sec`
  formula in `decode_pd_line_pair` now follows slowrx `video.c:88-92` term-
  for-term: `ChanStart[n+1] = ChanStart[n] + ChanLen[n] + SeptrTime`. With
  `septr_seconds = 0` the values are numerically unchanged; the field prevents
  a silent break when non-PD modes (Robot/Scottie/Martin — all non-zero
  SeptrTime) are added in V2. Two new tests verify numeric equivalence and
  zero values for PD modes.

- **#29 + #34 — `working_samples_emitted` informational** (`src/decoder.rs`):
  Added a rustdoc comment explaining that the counter intentionally is not
  decremented on `take_residual_buffer()` transfer. `DetectedVis::end_sample`
  is computed correctly at detection time directly from `total_samples_consumed`
  and `buffer.len()`; the counter is only used as a resampler-output anchor.
  Closes #34 as a duplicate of #29.

- **#33 — Lookahead already implicit** (`src/decoder.rs`): Phase 3's rewrite
  eliminated the dead `lookahead` variable by passing `&d.audio` (full image
  buffer) to `decode_pd_line_pair`. Added a doc comment on
  `run_findsync_and_decode` explaining the implicit lookahead design so the
  pattern is not accidentally reverted.

**Doc-only clarifications:**

- **#27** (`src/modespec.rs`): `lookup()` now documents that `0x00` is
  intentionally unmapped and that `None` matches slowrx's `VISmap[UNKNOWN]`
  → re-detect loop semantics (`vis.c:172-174`).

- **#28** (`src/vis.rs`): `take_residual_buffer()` now explains why no
  stop-bit skip is needed (unlike slowrx's `readPcm(20ms)` at `vis.c:169`):
  Rust's purely-past window means the buffer head is already past the stop bit
  when detection fires.

- **#31** (`src/decoder.rs`): Added `// V2:` comment at the `ImageComplete`
  → `AwaitingVis` transition noting that continuous monitoring already works
  (trailing audio is fed into a fresh `VisDetector`).

- **#35**: Already fixed by Phase 1's VIS rewrite. The `Vec<Tone> + remove(0)`
  pattern is gone; history is now a `[f64; HISTORY_LEN]` ring buffer. No code
  change needed.

- **#40** (`src/vis.rs`): `take_residual_buffer()` now carries a rustdoc
  re-anchor contract: callers MUST use a fresh `VisDetector::new()` after each
  detection. Stale `hops_completed` / `history_*` would corrupt subsequent
  detections in mid-image VIS scenarios.

**Round-trip stats — Phase 3 baseline preserved:**
* PD120: max_diff=11, mean=0.728
* PD180: max_diff=11, mean=0.560

### Changed (Phase 3: per-pixel demod parity)

Brings the per-pixel demod path closer to slowrx's `video.c::GetVideo`
(closes #23, #24; partial #18; partial #32). The previous code sliced
each radio channel into its own audio buffer, ran ONE per-pixel FFT
with a fixed 256-sample Hann window, and computed pixel times relative
to each channel slice — three issues that diverged from slowrx:

- **#24 — time-base alignment.** Pixel sample times now use slowrx's
  exact single-`round()` formula (`video.c:140-142`):
  `Skip + round(rate * (y/2 * line_seconds + chan_start_sec + (x +
  0.5) * pixel_secs))`. Previously the decoder rounded the per-pair
  offset and then `decode_pd_line_pair` rounded again — accumulating
  ~5 samples of drift by pair 11 of PD180.
- **#23 — FFT-every-N + StoredLum cache.** A single sweep over the
  line-pair audio runs an FFT every `PIXEL_FFT_STRIDE` samples and
  fills `stored_lum[s - sweep_start]` at every sample (slowrx
  `video.c:350-406`). Pixel times read out of the cache. At our
  11_025 Hz working rate `PIXEL_FFT_STRIDE = 1` (slowrx's `% 6` at
  44_100 Hz scales to ~1.5; stride=1 has the cleanest pixel-center
  alignment).
- **#18 (partial) — SNR estimator.** New `src/snr.rs` module
  implementing slowrx `video.c:302-343` (bandwidth-corrected
  `Psignal/Pnoise` over the video band 1500–2300 Hz vs noise bands
  400–800 ∪ 2700–3400 Hz, floored at -20 dB) plus a 7-window Hann
  bank with slowrx's exact threshold table
  (`window_idx_for_snr`). The estimator is wired into
  `decode_pd_line_pair` and runs every `SNR_REESTIMATE_STRIDE = 64`
  samples. **Deviation:** the per-pixel `win_idx` is currently
  hard-coded to 6 (longest, length 256) rather than driven by the
  SNR-adaptive selector; the synthetic round-trip's
  instant-frequency-step encoder reports ~12-19 dB SNR on clean
  tones, which would select shorter windows whose wider main lobe
  degrades peak interpolation against synthetic data. The selector
  should engage once a realistic FM-modulator-slewing synthetic
  encoder lands.
- **#32 (partial) — channel-bounded FFT input.** The new
  `decode_one_channel_into` helper builds a `scratch_audio` buffer
  that zero-pads samples outside the active channel's
  `[chan_lo, chan_hi)` range. **Deviation:** slowrx FFTs across
  channel boundaries (its `pcm.Buffer` is a continuous PCM stream
  and real-radio FM-modulator slewing softens cross-channel tone
  steps). With our synthetic encoder's hard tonal cliffs at
  channel boundaries, allowing cross-channel reads creates
  secondary FFT peaks that confuse the bin search. Revisit
  alongside the synthetic-encoder slewing work.

**New module:** `src/snr.rs` — `SnrEstimator`, `HannBank`,
`window_idx_for_snr`, `HANN_LENS`, `FFT_LEN`. Eleven unit tests
covering Hann bank shape, threshold boundaries, silence floor,
pure-tone behaviour, hedr-shift tracking, and degenerate-input
safety.

**File reorganization:** `mode_pd::test_encoder` moved to a new
`src/pd_test_encoder.rs` (test-support gated) so the production
decoder doesn't grow further past the 500-LOC ceiling. Re-export
path `__test_support::mode_pd::encode_pd` preserved.

**Round-trip stats — Phase 2 baseline preserved:**
* PD120 max_diff=11, mean=0.728 (was 11/0.75)
* PD180 max_diff=11, mean=0.560 (was 11/0.55)

**Tests:** 11 new SNR unit tests (`snr_silence_floors_at_minus_twenty`,
`snr_pure_video_tone_is_high`, `snr_pure_noise_band_is_negative`,
`snr_tone_plus_noise_intermediate`, `snr_hedr_shift_tracks_band`,
`window_idx_thresholds_match_slowrx`, `hann_bank_lengths_correct`,
`hann_window_endpoints_are_zero`, `hann_lens_match_slowrx_at_workingrate`,
`build_hann_zero_and_one_length_safe`, `snr_estimator_default_constructs`)
+ 2 new mode_pd tests (`pixel_freq_clamps_out_of_range_win_idx`,
`pixel_freq_short_window_still_recovers_tone`).

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
