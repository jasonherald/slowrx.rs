# Issue #89 — VIS detector fidelity — design

**Issue:** [#89](https://github.com/jasonherald/slowrx.rs/issues/89) (audit bundle 5 of 12)
**Source of record:** `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs A1, A3, C1, C5, C15)
**Scope:** five cohesive fixes in the VIS-detection path. No behavior change for the 99.9% case (clean, known VIS burst); the changes harden the unknown-code path and bring `match_vis_pattern` closer to slowrx's structure.

## Background

`SstvDecoder` runs a VIS detector (`src/vis.rs::VisDetector`) until a 14-window VIS
pattern matches; the resulting 7-bit code is looked up in `src/modespec.rs::lookup`
to dispatch a mode. The deep code-review audit found five issues clustered here:

- **A1 (High)** — on an *unknown* VIS code, `SstvDecoder::process` drains the detector's
  residual buffer but never reconstructs `self.vis`. `VisDetector::take_residual_buffer`'s
  documented "#40 re-anchor contract" requires a fresh `VisDetector::new()` afterward
  (`hops_completed` / `history` / `history_ptr` / `history_filled` are not reset), or the
  next detection's `end_sample` and pattern window are corrupted. The successful-decode
  path and the post-image path both already reseed; the unknown path was the outlier.
- **A3** — `match_vis_pattern` returns the *first* parity-passing `(i, j)` alignment, known
  or not. slowrx (`vis.c`) tries all 9 alignments, only stopping on a *known* code (and
  `printf`s unknown-but-parity-passing ones as it goes). On a marginal burst that classifies
  to garbage-but-parity-OK at one alignment and the real code at another, slowrx recovers
  and we don't.
- **C1** — `Error::UnknownVisCode(u8)` is never constructed. `process()` returns
  `Vec<SstvEvent>`, never `Result`, and `SstvDecoder::new` only fails on a bad sample rate,
  so the variant is unreachable. Unknown codes are dropped silently.
- **C5** — the R12BW VIS code `0x06` appears as a bare literal in `vis.rs`
  (`match_vis_pattern`, the test encoder, test comments). slowrx writes `VISmap[VIS] == R12BW`
  symbolically.
- **C15** — the parity-bit branch of `match_vis_pattern`'s bit loop flips the *expected bit*
  (`bit ^ 1`) for R12BW rather than the *accumulator* (`Parity = !Parity` in slowrx), and
  has no `break` after a parity failure (harmless — `k == 7` is the last iteration — but
  asymmetric with the classify-fail arm).

## Design

### Part 1 — Decoder side (`src/decoder.rs`, `src/error.rs`, `src/modespec.rs`)

**A1 fix + DRY helper.** Add to `SstvDecoder`:

```rust
/// `|c| crate::modespec::lookup(c).is_some()` as an `fn` pointer (the closure
/// captures nothing, so it coerces). Threaded into every `VisDetector` so it
/// can keep-search past unknown codes (issue #89 A3).
const IS_KNOWN_VIS: fn(u8) -> bool = |c| crate::modespec::lookup(c).is_some();

/// Discard the current VIS detector and start fresh on `leftover_audio`
/// (post-stop-bit residue, or trailing image audio). Honors the #40
/// re-anchor contract documented on `vis::VisDetector::take_residual_buffer`.
fn restart_vis_detection(&mut self, leftover_audio: Vec<f32>) {
    self.vis = crate::vis::VisDetector::new(IS_KNOWN_VIS);
    self.vis.process(&leftover_audio, self.working_samples_emitted);
}
```

(`VisDetector::new` takes `IS_KNOWN_VIS` and stores it; `process`'s signature is unchanged
— see Part 2.)

The unknown-VIS branch (currently `decoder.rs:284-289`) becomes, on `lookup(detected.code)` → `None`:

```rust
out.push(SstvEvent::UnknownVis {
    code: detected.code,
    hedr_shift_hz: detected.hedr_shift_hz,
    sample_offset: detected.end_sample,
});
let residual = self.vis.take_residual_buffer();
self.restart_vis_detection(residual);
continue; // surface a follow-up VIS already in the residual within this call
```

`continue` (not the current `break`) matches the known-VIS branch — a back-to-back VIS
in the residual is detected without waiting for another `process()` call.

The post-image reseed (`decoder.rs:343-346`) is refactored to use the helper — a pure
DRY change, no behavior difference:

```rust
let trailing = std::mem::take(&mut d.audio);
self.state = State::AwaitingVis;
self.restart_vis_detection(trailing);
break;
```

(The `mem::take`-vs-`split_off` question on that line is issue #90's; this PR does not
touch it. `SstvDecoder::new`'s initial `VisDetector::new(IS_KNOWN_VIS)` stays inline —
it isn't a "restart".)

**`SstvEvent::UnknownVis`** — new variant on the `#[non_exhaustive]` `SstvEvent` enum
(additive):

```rust
/// A VIS header parsed and passed parity, but its 7-bit code maps to no
/// SSTV mode this build can decode (reserved / undefined, or a mode not yet
/// implemented). The decoder discards the burst and resumes scanning for the
/// next VIS — equivalent to slowrx's `printf("Unknown VIS %d")` plus its
/// retry-`GetVIS()` loop.
UnknownVis {
    /// The 7-bit VIS code that did not resolve.
    code: u8,
    /// Radio mistuning offset in Hz: `observed_leader_hz - 1900` (same
    /// quantity as [`SstvEvent::VisDetected::hedr_shift_hz`]). Surfaced for
    /// diagnostics only; the burst is dropped, so it does not feed decode.
    hedr_shift_hz: f64,
    /// Working-rate (11025 Hz) sample offset where the VIS stop bit ended.
    sample_offset: u64,
},
```

**`Error::UnknownVisCode`** — deleted, along with its test `unknown_vis_code_renders_in_hex`.
`Error::InvalidSampleRate` stays. (Removing a variant from a `#[non_exhaustive]` enum is
technically breaking, but the variant has never been constructed — noted in the CHANGELOG.)

**`modespec::lookup`** — the `#27` doc note is tweaked to mention `SstvEvent::UnknownVis` is
now emitted (the "silent try again" wording becomes "emits `SstvEvent::UnknownVis` and tries
again").

### Part 2 — `src/vis.rs`: faithful keep-searching, `R12BW_VIS_CODE`, parity-check shape

**`R12BW_VIS_CODE` (C5).** Add near the other VIS constants:

```rust
/// VIS code for the (unimplemented) Robot 12 B/W mode. R12BW inverts the
/// parity bit (slowrx `vis.c:116`). No `ModeSpec` exists for it yet;
/// `match_vis_pattern` still classifies it so a future R12BW implementation
/// that adds it to `modespec::lookup` works without re-touching the parity
/// logic. TODO: when R12BW gains a `ModeSpec`, derive this from it.
pub(crate) const R12BW_VIS_CODE: u8 = 0x06;
```

Use it in `match_vis_pattern`, in the test encoder `synth_vis_with_offset`, and in the
R12BW test comments — replacing the bare `0x06` literals.

**A3 — keep-searching for a known code.** `match_vis_pattern` gains an `is_known` predicate:

```rust
fn match_vis_pattern(
    tones: &[f64; HISTORY_LEN],
    is_known: impl Fn(u8) -> bool,
) -> Option<(u8, f64, usize)> {
    let tol = TONE_TOLERANCE_HZ;
    let mut first_unknown: Option<(u8, f64, usize)> = None;
    for i in 0..3 {
        for j in 0..3 {
            // ... leader / break / 8-bit classification + parity (unchanged) ...
            if bit_ok {
                let m = (code, leader - LEADER_HZ, i);
                if is_known(code) {
                    return Some(m); // slowrx breaks on the first known code
                }
                first_unknown.get_or_insert(m);
            }
        }
    }
    first_unknown
}
```

- A marginal burst that hits an unknown-but-parity-OK alignment first and the real code at a
  later alignment now returns the real code (the A3 motivating case).
- A clean *unknown-but-well-formed* burst (a real R12BW transmission, or an unimplemented
  mode) still surfaces — as the `first_unknown` fallback, with `code` intact — so the decoder
  can emit `SstvEvent::UnknownVis`. The caller already re-runs `crate::modespec::lookup(code)`
  to split known→`VisDetected` / unknown→`UnknownVis`, so `DetectedVis` needs no new field.

**Threading the predicate.** `VisDetector` gains an `is_known_vis: fn(u8) -> bool` field, set
by `VisDetector::new(is_known_vis: fn(u8) -> bool)` and stored on the struct; `process` reads
`self.is_known_vis` when calling `match_vis_pattern` — `process`'s signature is unchanged.
(`fn` pointer, not `Box<dyn Fn>` — `|c| crate::modespec::lookup(c).is_some()` captures
nothing, so it coerces; `decoder.rs` holds the one value in `IS_KNOWN_VIS`. `vis.rs::tests`
pass `|c| crate::modespec::lookup(c).is_some()` to `VisDetector::new` — `modespec` is a
sibling leaf module, no cycle.)

**C15 — parity-check shape.** The `k == 7` branch of the bit loop becomes:

```rust
} else {
    // `bit` is the received parity bit. R12BW flips the *accumulated* parity
    // per slowrx `vis.c:116`: `if (VISmap[VIS] == R12BW) Parity = !Parity;`.
    if code == R12BW_VIS_CODE {
        parity ^= 1;
    }
    if parity != bit {
        bit_ok = false;
    }
    break; // k == 7 is the last iteration; explicit break mirrors the classify-fail arm
}
```

Functionally identical to today, but faithful in structure and the `break` makes the loop's
two exits symmetric. The surrounding comment block (currently `vis.rs:378-384`) is updated to
match.

**`match_vis_pattern` doc comment.** The existing `#5` / round-2 deviation note stays; a new
sentence points at the new `intentional-deviations.md` entry for the keep-searching /
`UnknownVis` behavior.

### Part 3 — Tests, docs, changelog

**`src/vis.rs::tests` — `match_vis_pattern` unit tests** (it's private; the test module can
call it directly). A small test-local helper builds a `[f64; HISTORY_LEN]` array from a
`(phase0_code, phase1_code)` pair (leader / break tones positioned so phases `i = 0` and
`i = 1` both pass the leader/break checks):

- `match_vis_pattern_clean_known_code` — array for known code `0x5F` at `i = 0`; assert
  `Some((0x5F, ~0.0, 0))`.
- `match_vis_pattern_clean_unknown_code_falls_back` — array for code `0x01` (well-formed,
  `lookup` → `None`); assert `Some((0x01, _, 0))` (the `first_unknown` fallback).
- `match_vis_pattern_prefers_known_over_earlier_unknown` — `phase0 = 0x01` (unknown),
  `phase1 = 0x5F` (known); assert it returns `(0x5F, _, 1)` — i.e. it skipped the earlier
  unknown alignment. **This is the A3 regression test.**

Existing `vis.rs::tests` keep passing: the `VisDetector::new(...)` and `process(...)` call
sites in the test module gain the predicate arg; `every_valid_vis_code_decodes_correctly`
still holds because an unknown code surfaces via `first_unknown` with `code` intact.

**`tests/unknown_vis.rs` — new integration test** (uses the `test-support` feature, like
`tests/no_vis.rs`):

- `decoder_emits_unknown_vis_then_recovers` —
  `SstvDecoder::process(synth_vis(0x01, 0.0) ++ synth_vis(0x5F, 0.0) ++ short zero pad)`,
  collect events, assert the sequence contains `SstvEvent::UnknownVis { code: 0x01, .. }`
  followed by `SstvEvent::VisDetected { mode: SstvMode::Pd120, .. }`, **and** the
  `VisDetected.sample_offset` is within one VIS-window of the true end of the second burst's
  stop bit (≈ both bursts' combined working-rate length). The `sample_offset` check is the
  A1 regression — with the stale detector, `sample_offset` came out as
  `(cumulative_hops + i) * HOP_SAMPLES`, wildly inflated. No full-image decode is needed —
  `VisDetected` fires the moment the second stop bit is recognized.

**`src/error.rs`** — drop `unknown_vis_code_renders_in_hex`; `invalid_sample_rate_renders_with_value`
stays.

**`docs/intentional-deviations.md`** — one new entry near the existing VIS entries:
*"VIS: keep-searching for a known code; surface clean unknown bursts."* Covers the A3
mechanics (all 9 alignments tried, first known wins), the `SstvEvent::UnknownVis`-vs-slowrx-
`printf` choice (we surface the code as an event; slowrx prints it and returns 0), and the
residual edge case (the only scenario slowrx still beats us is now also handled).

**`CHANGELOG.md`** under `[Unreleased]`:

- *Fixed* — stale `VisDetector` after an unknown VIS code (A1): could mis-anchor the next
  detection's `sample_offset` and pollute its pattern window.
- *Added* — `SstvEvent::UnknownVis { code, hedr_shift_hz, sample_offset }`: unrecognized-but-
  well-formed VIS bursts are surfaced instead of dropped silently; `match_vis_pattern` keeps
  searching past unknown alignments for a known one (A3).
- *Removed* — `Error::UnknownVisCode` (never constructed; `process()` does not return `Result`).
- *(internal)* — `R12BW_VIS_CODE` named constant; faithful parity-check shape in
  `match_vis_pattern` (C5, C15).

## Out of scope

- The post-image `break`-vs-`continue` and the `mem::take`-vs-`split_off` of the decoded
  audio buffer — issue #90 (A2 / D4).
- Any wider `vis.rs` decomposition or the shared-DSP `dsp`/`demod` modules — issues #88, #85.
- `SstvImage` / `Error` further changes beyond deleting the dead variant.

## Files touched

- `src/decoder.rs` — `IS_KNOWN_VIS` const, `restart_vis_detection` helper, `SstvEvent::UnknownVis`
  variant + handling, refactor the post-image reseed to use the helper, `VisDetector::new(IS_KNOWN_VIS)`
  at every construction site (`SstvDecoder::new`, the helper).
- `src/vis.rs` — `R12BW_VIS_CODE` const; `match_vis_pattern` `is_known` parameter + keep-searching
  + parity shape; `VisDetector::is_known_vis` field + `new` signature (`process` unchanged); doc
  updates; new `match_vis_pattern` unit tests; test-encoder/test-comment literal cleanup.
- `src/error.rs` — delete `UnknownVisCode` variant + its test.
- `src/modespec.rs` — tweak the `lookup` `#27` doc note.
- `tests/unknown_vis.rs` — new integration test (`test-support` feature).
- `docs/intentional-deviations.md` — new VIS deviation entry.
- `CHANGELOG.md` — `[Unreleased]` entries.

## Verification

Full local CI gate after the work, per repo convention:

- `cargo test --all-features --locked --release`
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
