# slowrx.rs V2 — Epic Split Design

**Status:** Approved 2026-05-02. Ready for implementation planning via the
[superpowers:writing-plans](https://github.com/anthropic-ai/superpowers) skill.

**Goal:** Decompose the single V2 umbrella ([issue #9]) into focused child
epics that can ship one at a time, each producing an additive minor crate
release. Add a parallel child epic for ISS / Zarya real-capture validation.

**Out of scope for this spec:**

- Decoder algorithm details for each mode-family (those land per-epic in the
  writing-plans skill output).
- Encoding (slowrx.rs is decode-only).
- HF SSTV modes (SC2-180, MMSSTV-specific). V2 targets VHF/UHF amateur SSTV.
- GUI, streaming I/O, or any non-library work.
- Refactoring shared logic across mode families (deferred — see Risks §4).

[issue #9]: https://github.com/jasonherald/slowrx.rs/issues/9

## Context

`slowrx.rs` shipped 0.1.0 on 2026-05-02 with PD120 + PD180 only — sufficient
for ARISS reception. V1's architecture deliberately carved out
`#[non_exhaustive]` enums (`SstvMode`, `ChannelLayout`) and a `septr_seconds`
field so V2 modes could land additively without churning V1 files. V2
realizes that promise.

The user is the only consumer plus the [`jasonherald/rtl-sdr`] application.
There is no external release pressure; the project favors careful delivery
over fast delivery.

[`jasonherald/rtl-sdr`]: https://github.com/jasonherald/rtl-sdr

## Epic structure

Issue #9 stays open as the V2 umbrella. Five child epics file under it.
The umbrella body gets a "Children" section linking each child; the umbrella
closes when V2.1–V2.4 close. V2.5 outlives V2 as an open-ended capture
tracker.

| #    | Epic              | Modes                          | Files touched                                                     | Est. LOC | Release |
|------|-------------------|--------------------------------|-------------------------------------------------------------------|----------|---------|
| V2.1 | PD-extended       | PD240                          | extends `mode_pd.rs`, `modespec.rs` (+1 row), `lib.rs` re-export  | ~150     | 0.2.0   |
| V2.2 | Robot family      | Robot 36, Robot 72             | new `mode_robot.rs`, `modespec.rs` (+2), `decoder.rs` dispatch    | ~400     | 0.3.0   |
| V2.3 | Scottie family    | Scottie 1, Scottie 2, DX       | new `mode_scottie.rs`, `modespec.rs` (+3), dispatch               | ~400     | 0.4.0   |
| V2.4 | Martin family     | Martin 1, Martin 2             | new `mode_martin.rs`, `modespec.rs` (+2), dispatch                | ~300     | 0.5.0   |
| V2.5 | Zarya validation  | (no decoder)                   | `tests/fixtures/iss-zarya/README.md` + regression test once live  | ~50      | patch   |

**Sequential delivery order:** V2.1 → V2.2 → V2.3 → V2.4. PD240 first because
it's the lowest-risk extension of existing code (same chroma layout, only
timing differs) and acts as a confidence-builder that the V1 carve-outs
work as designed. Robot family second because it's the second-most-likely
on-air mode after PD. Scottie before Martin because they share RGB-sequential
layout — Scottie de-risks Martin.

**Closeout per child:** epic closes when the mode-family PR merges, the
semver bump publishes to crates.io, the CHANGELOG entry lands, and the
umbrella #9 checklist is ticked.

**V2.5 (Zarya) runs in parallel** as opportunity allows. It does not gate
any decoder release. Its merge gate is a real-world Zarya capture (RSØISS /
ISS Russian-side SSTV pass). Recent ARISS-SSTV events from the Russian side
have used PD120 almost exclusively — already covered by V1 — so V2.5 is
fixture/observability work, not new decoder work.

## Per-epic shape (mode-family template)

Each mode-family child epic body has these sections:

1. **Mode family** — modes, VIS codes, slowrx C reference (file:line).
2. **DSP delta from V1** — what's new vs. PD-family chroma/sync logic.
3. **File plan** — new file path, ≤ 500 LOC cap, additions to `modespec.rs`
   and `decoder.rs` dispatch.
4. **Test corpus** — synthetic round-trip in `tests/roundtrip.rs` plus a
   per-mode encoder helper at `src/<family>_test_encoder.rs`, mirroring
   `src/pd_test_encoder.rs`.
5. **Coverage gate** — `cargo llvm-cov` ≥ 92 % per-file maintained.
6. **Acceptance checklist** — synthetic round-trip green, per-file coverage
   ≥ 92 %, `CHANGELOG.md` entry, version bump, `cargo publish --dry-run`
   clean.
7. **Out of scope** — explicitly: real-radio fixtures (those land
   asynchronously, not as a merge gate; see Risks §3).

**Intentionally absent from the template:**

- slowrx C cross-validation infrastructure. V1 audits showed this is a large
  undertaking for marginal gain. We rely on synthetic round-trips and
  post-hoc audit (V1's Phase-1→Phase-7 recovery pattern) instead.
- Real-radio capture as a merge gate. Captures arrive on their own schedule.
- Cross-mode shared-helper refactors. Let Scottie and Martin duplicate;
  only extract a shared helper if V2.4 shows clear duplication pain.

## Per-epic shape (Zarya template — V2.5 only)

V2.5 has no decoder work. Its body sections are:

1. **Capture goals** — target an RSØISS / ISS Russian-side SSTV pass;
   document date, frequency (145.800 MHz FM), equipment, audio chain.
2. **Mode identification** — use `slowrx-cli` to detect the VIS code from
   the recording; confirm mode (expected: PD120, possibly PD180).
3. **Fixture acceptance** — drop the WAV under `tests/fixtures/iss-zarya/`,
   add a regression case, follow the V1 ARISS-corpus pattern (gitignored,
   no CI gate).

**Acceptance:** at least one Zarya capture decoded to a recognizable image,
fixture path committed (the WAV itself stays out of git, matching the
Dec-2017 corpus convention).

## Architectural deltas

Three shared files change. Everything else lives in new per-family modules.

### `modespec.rs` — additive only

The V1 enums are already `#[non_exhaustive]`, so all additions are
non-breaking.

- New `SstvMode` variants: `Pd240`, `Robot36`, `Robot72`, `Scottie1`,
  `Scottie2`, `ScottieDx`, `Martin1`, `Martin2`.
- New `ChannelLayout` variants:
  - `RobotYuv` — Y line + alternating Cr/Cb chroma (Robot 36); full
    per-line chroma (Robot 72).
  - `RgbSequential` — Scottie + Martin both. They share layout; they
    differ only in sync placement.
- New `sync_position: SyncPosition` field on `ModeSpec` to capture the
  Scottie quirk (sync between G and B, not at line start). PD / Robot /
  Martin all keep `LineStart`. Adding the field is non-breaking — it's a
  new struct field, not an enum variant change.
- The `septr_seconds` field carved out in V1 finally takes non-zero values
  for Robot, Scottie, and Martin. (V1's foresight pays off — the
  `chan_starts_sec` formula in `mode_pd::decode_pd_line_pair` already uses
  it term-for-term per `slowrx video.c:88-92`.)

### `decoder.rs` — small dispatch addition (~30–50 LOC)

- The state machine currently calls `mode_pd::decode_pd_line_pair` directly
  when the active mode's layout is `PdYcbcr`.
- New dispatch matches on `ChannelLayout` (not `SstvMode`):
  `RobotYuv` → `mode_robot::decode_line`, `RgbSequential` →
  `mode_scottie::decode_line` (Scottie + Martin share an entry point;
  the `sync_position` field disambiguates).
- The `PdYcbcr` arm stays byte-identical to today. Zero behavioral change
  for V1 modes.

### `lib.rs` — re-exports

- `pub mod` declarations for new mode modules per epic.
- New `SstvMode` variants and `ChannelLayout` variants surface through the
  existing re-export of `crate::modespec::*`.
- No churn to existing exports.

### Files unchanged by V2 (expected)

`vis.rs`, `sync.rs`, `snr.rs`, `resample.rs`, `image.rs`, `error.rs`. Sync
correlation is mode-agnostic; per-mode sync placement is metadata on
`ModeSpec`, not new logic in `sync.rs`. **Caveat:** Risk §1 documents the
one path under which `sync.rs` or the line-clock advance in `decoder.rs`
may need to change during V2.3 (Scottie). If that turns out to be required,
the change lands in V2.3 in scope, not as a separate refactor.

## Versioning

One semver-minor bump per mode-family epic (4 minor bumps total). Adding a
variant to `#[non_exhaustive]` enums is API-additive — semver-minor under
the strict rule.

| Epic | Crate version | Trigger                  |
|------|---------------|--------------------------|
| V2.1 | 0.2.0         | PD240 ships              |
| V2.2 | 0.3.0         | Robot 36 + Robot 72 ship |
| V2.3 | 0.4.0         | Scottie 1 / 2 / DX ship  |
| V2.4 | 0.5.0         | Martin 1 / 2 ship        |
| V2.5 | (patch)       | Fixture lands; no API change |

This gives the rtl-sdr application a precise pin to declare against
("requires `slowrx ≥ 0.3` for Robot support") rather than depending on
"whatever 0.2.x has."

## Testing posture

Same as V1, no new infrastructure:

- **Synthetic round-trip** per mode-family in `tests/roundtrip.rs` with a
  per-mode encoder helper (`src/<family>_test_encoder.rs`).
- **Per-file coverage gate** at 92 %, enforced via `cargo llvm-cov`.
- **Real-world fixtures** (when they appear) land under
  `tests/fixtures/<event>/` following the V1 pattern: gitignored corpus,
  local validation only, no CI gate on real-WAV decoding. The V1 CLI
  integration test (`tests/cli.rs`) is the model — it skips silently if
  the fixture isn't present.

## Risks and unknowns

1. **Scottie sync placement** (V2.3) — V1's line-clock advance assumes sync
   at line start. If hidden assumptions exist, V2.3 fixes them in scope.
   Mitigation: the `sync_position` field added in V2.1 forces the assumption
   to be made explicit at dispatch time.
2. **Robot 36 chroma alternation** (V2.2) — Robot 36 alternates Cr/Cb
   between consecutive lines. This is genuinely new per-line state, not
   just timing. Synthetic round-trip must cover both line phases.
3. **Real-radio gap** — same trap V1 hit (synthetic green, real-radio
   broken). Without slowrx-C cross-validation, each mode-family will have a
   window where it passes synthetic tests but its real-radio behavior is
   unverified. Mitigation: V1's parity-audit recovery pattern remains
   available; we accept the risk knowingly.
4. **Fixture availability for V2.5** — Zarya captures are user-dependent.
   V2.5 may stay open indefinitely. Acceptable — it's a capture tracker,
   not a release-gate epic. Promising sources (untried as of writing):
   ARISS-SSTV award gallery, N7CXI mode samples, SSTV-handbook.com sample
   library, SatNOGS network archives.

## Open questions for writing-plans

- **Per-mode `*_test_encoder.rs` shape** — replicate `pd_test_encoder.rs`
  exactly, or factor a shared encoder trait? **Recommend: replicate, don't
  factor. YAGNI.**
- **Dispatch key in `decoder.rs`** — match on `ChannelLayout` or
  `SstvMode`? **Recommend: `ChannelLayout`, since Scottie + Martin share
  an entry point.**

## Acceptance for this spec

This spec is "done" when:

- Issue #9 has been edited to add a "Children" section linking V2.1–V2.5.
- 5 child epics exist on GitHub with the body sections above filled in.
- The first child epic (V2.1) has an accepted implementation plan produced
  by the writing-plans skill.

Spec lifecycle ends here. Each child epic gets its own writing-plans pass
when it comes up in the queue.
