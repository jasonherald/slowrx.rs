# Issue #89 — VIS detector fidelity — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the five VIS-detection fixes from audit bundle #89 — A1 (reseed the VIS detector after an unknown code), A3 (keep searching alignments for a *known* code, slowrx-style), C1 (surface unknown VIS bursts via a new `SstvEvent::UnknownVis`, drop the dead `Error::UnknownVisCode`), C5 (`R12BW_VIS_CODE` named constant), C15 (faithful parity-check shape in `match_vis_pattern`).

**Architecture:** `match_vis_pattern` (in `src/vis.rs`) gains an `is_known: impl Fn(u8) -> bool` predicate and, instead of returning the first parity-passing alignment, returns the first *known* code (falling back to the first parity-passing *unknown* code so the caller can still report it). `VisDetector` stores that predicate as a `fn(u8) -> bool` field set at construction. `SstvDecoder` holds the one predicate value in a `const IS_KNOWN_VIS`, gains a `restart_vis_detection` helper that reseeds `self.vis` per the documented `#40` re-anchor contract, and emits `SstvEvent::UnknownVis { code, hedr_shift_hz, sample_offset }` for unrecognized bursts. `Error::UnknownVisCode` (never constructed) is removed.

**Tech Stack:** Rust 2021, MSRV 1.85. `rustfft`. Crate clippy config: `clippy::all`/`pedantic` = warn, `unwrap_used`/`panic`/`expect_used` = warn (no panics in lib code). CI gate: `cargo test --all-features --locked --release`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`. No GPG signing (`git commit` / `git tag -a` plain).

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-11-issue-89-vis-detector-fidelity-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs A1, A3, C1, C5, C15)
- `docs/intentional-deviations.md` (existing VIS entries the new one slots near)

---

## File Structure

| File | Change |
|------|--------|
| `src/vis.rs` | Add `pub(crate) const R12BW_VIS_CODE`. `match_vis_pattern`: faithful parity-check shape (C15) + `is_known` predicate + keep-searching/`first_unknown` (A3) + doc update. `VisDetector`: `is_known_vis: fn(u8) -> bool` field + `new` signature; `process` reads `self.is_known_vis`. Replace `0x06` literals in the test encoder + test comments. New `match_vis_pattern` unit tests + a test-local tone-history builder. Update `VisDetector::new()` call site in the `run` test helper. |
| `src/decoder.rs` | `const IS_KNOWN_VIS: fn(u8) -> bool`. `VisDetector::new(IS_KNOWN_VIS)` at the two construction sites. New `SstvEvent::UnknownVis` variant. `restart_vis_detection` helper. Rewrite the unknown-VIS branch (emit event → `restart_vis_detection` → `continue`). Refactor the post-image reseed to call `restart_vis_detection`. |
| `src/error.rs` | Delete `Error::UnknownVisCode` variant + its test `unknown_vis_code_renders_in_hex`. |
| `src/modespec.rs` | Tweak the `lookup` `#27` doc note to mention `SstvEvent::UnknownVis`. |
| `tests/unknown_vis.rs` | **New.** Integration test: `SstvDecoder::process` emits `UnknownVis` then recovers a following valid VIS. Gated `#![cfg(feature = "test-support")]` (uses `synth_vis`). |
| `docs/intentional-deviations.md` | New section: *"VIS: keep-searching for a known code; surface clean unknown bursts."* |
| `CHANGELOG.md` | `[Unreleased]` entries (Fixed / Added / Removed / internal). |

Task order: **T1** (C5 + C15, `vis.rs` cleanups, no behavior change) → **T2** (A3 predicate + `VisDetector` field + `IS_KNOWN_VIS` + unit tests) → **T3** (`SstvEvent::UnknownVis` + `restart_vis_detection` + A1 fix + post-image refactor + integration test) → **T4** (delete `Error::UnknownVisCode`) → **T5** (docs) → **T6** (full CI gate).

---

## Task 1: `vis.rs` cleanups — `R12BW_VIS_CODE` constant + faithful parity-check shape (C5, C15)

Pure refactor — functionally identical. No new test (existing `vis.rs::tests` cover it: `r12bw_uses_inverted_parity`, `r12bw_rejects_standard_parity`, `parity_failure_is_rejected`, the proptests).

**Files:**
- Modify: `src/vis.rs` (constants block ~lines 22-34; `match_vis_pattern` k==7 branch ~lines 377-388; test encoder `synth_vis_with_offset` ~lines 493-497; test comments ~lines 609-676)

- [ ] **Step 1: Add the `R12BW_VIS_CODE` constant**

In `src/vis.rs`, in the constants block just after `pub(crate) const HISTORY_LEN: usize = 45; // slowrx HedrBuf size` (currently line 34), add:

```rust
/// VIS code for the (unimplemented) Robot 12 B/W mode. R12BW inverts the
/// parity bit (slowrx `vis.c:116`). No `ModeSpec` exists for it yet, so
/// `crate::modespec::lookup` returns `None` for it; `match_vis_pattern`
/// still classifies it so a future R12BW implementation that adds it to
/// `lookup` works without re-touching the parity logic.
//
// TODO: when R12BW gains a `ModeSpec`, derive this from `modespec`.
pub(crate) const R12BW_VIS_CODE: u8 = 0x06;
```

- [ ] **Step 2: Rewrite the `k == 7` parity-check branch in `match_vis_pattern` (C15)**

Replace the `else` arm of the `if k < 7 { ... } else { ... }` inside the `for k in 0..8` loop. Currently:

```rust
                } else {
                    // R12BW (`0x06`) inverts parity per slowrx `vis.c:116`:
                    // `if (VISmap[VIS] == R12BW) Parity = !Parity;`. V1
                    // doesn't decode R12BW (lookup returns None for 0x06),
                    // but the parity check must still pass so a future V2
                    // implementation that adds R12BW to the lookup table
                    // doesn't silently reject every R12BW burst.
                    let expected = if code == 0x06 { bit ^ 1 } else { bit };
                    if parity != expected {
                        bit_ok = false;
                    }
                }
```

becomes:

```rust
                } else {
                    // `bit` is the received parity bit. R12BW flips the
                    // *accumulated* parity per slowrx `vis.c:116`:
                    // `if (VISmap[VIS] == R12BW) Parity = !Parity;`. No
                    // `ModeSpec` exists for R12BW yet (`lookup` returns
                    // `None` for `R12BW_VIS_CODE`), but the parity check
                    // must still pass so a future R12BW implementation that
                    // adds it to `lookup` is not silently broken.
                    if code == R12BW_VIS_CODE {
                        parity ^= 1;
                    }
                    if parity != bit {
                        bit_ok = false;
                    }
                    break; // k == 7 is the last iteration; explicit break mirrors the classify-fail arm
                }
```

- [ ] **Step 3: Replace `0x06` literals in the test encoder**

In `synth_vis_with_offset` (`#[cfg(any(test, feature = "test-support"))] pub mod tests`), the lines currently:

```rust
        // R12BW (code 0x06) inverts the parity bit per slowrx `vis.c:116`.
        // The detector's `match_vis_pattern` does the same inversion when
        // checking, so synthetic bursts must follow the same convention or
        // they'd fail parity at the receiver.
        let parity_bit = if code == 0x06 { parity ^ 1 } else { parity };
```

become:

```rust
        // R12BW (`R12BW_VIS_CODE`) inverts the parity bit per slowrx `vis.c:116`.
        // The detector's `match_vis_pattern` does the same inversion when
        // checking, so synthetic bursts must follow the same convention or
        // they'd fail parity at the receiver.
        let parity_bit = if code == R12BW_VIS_CODE { parity ^ 1 } else { parity };
```

(`R12BW_VIS_CODE` is `pub(crate)` and the test module does `use super::*`, so it is in scope.)

In `r12bw_rejects_standard_parity`, replace `let bit = (0x06_u8 >> b) & 1;` with `let bit = (R12BW_VIS_CODE >> b) & 1;`.

Other `0x06` occurrences in that module are inside doc comments / string literals / `vis_padded(0x06, ...)` / `assert_eq!(detected.code, 0x06)` — leave the *argument* and *assertion* literals (`vis_padded(0x06, ..)`, `assert_eq!(.., 0x06)`) as-is (they read fine as a concrete code), but in the prose comments above those tests change "code 0x06" → "code `R12BW_VIS_CODE` (`0x06`)" where it improves clarity. Minimal touch — do not churn comments unnecessarily.

- [ ] **Step 4: Run the existing tests**

Run: `cargo test --lib vis -- --nocapture`
Expected: PASS — all of `vis::tests` still green (`r12bw_uses_inverted_parity`, `r12bw_rejects_standard_parity`, `parity_failure_is_rejected`, `detects_clean_pd120_and_pd180`, the proptests, etc.).

- [ ] **Step 5: Commit**

```bash
git add src/vis.rs
git commit -m "refactor(vis): R12BW_VIS_CODE const + faithful parity-check shape (#89 C5/C15)"
```

---

## Task 2: A3 — keep-searching `match_vis_pattern`, `is_known_vis` field, `IS_KNOWN_VIS` const

`match_vis_pattern` tries all 9 `(i, j)` alignments and returns the first *known* code (like slowrx, which breaks on the first known code); if none is known but ≥1 parity-passing *unknown* code exists, it returns the first such (the caller surfaces it via `SstvEvent::UnknownVis` in T3). `VisDetector` carries the predicate as a `fn(u8) -> bool` field.

**Files:**
- Modify: `src/vis.rs` (`match_vis_pattern` signature + body + doc; `VisDetector` field + `new` + `process`; `run` test helper; new tests)
- Modify: `src/decoder.rs` (add `const IS_KNOWN_VIS`; update the two `VisDetector::new()` call sites)
- Test: `src/vis.rs` `pub mod tests` — three new `match_vis_pattern` unit tests + a test-local `synth_tone_history` builder

- [ ] **Step 1: Write the failing unit tests + the tone-history builder**

In `src/vis.rs`, inside `pub mod tests { ... }`, add (placed after the existing `synth_tone_n` helper, before the `#[test]` functions — or anywhere in the module):

```rust
    /// Build a 45-entry tone history (`HedrBuf` order: `[0]` oldest) that the
    /// matcher reads as `phase0_code` at phase `i = 0` and, if `phase1_code`
    /// is `Some(c)`, as `c` at phase `i = 1`. Phase `i = 2`'s bit slots are
    /// left as leader tones, so it fails bit classification (never matches).
    /// All leader / break slots are positioned so phases 0 and 1 pass those
    /// checks. Tones are at the un-shifted (`hedr_shift = 0`) frequencies.
    fn synth_tone_history(phase0_code: u8, phase1_code: Option<u8>) -> [f64; HISTORY_LEN] {
        let leader = LEADER_HZ;
        let break_f = leader + BREAK_HZ_OFFSET;
        let zero_f = leader + BIT_ZERO_OFFSET;
        let one_f = leader + BIT_ONE_OFFSET;
        let bit_freq = |b: u8| if b == 1 { one_f } else { zero_f };
        let mut t = [leader; HISTORY_LEN];
        // Break tones — checked at indices 15+i and 42+i for i in 0..3.
        t[15] = break_f;
        t[16] = break_f;
        t[42] = break_f;
        t[43] = break_f;
        // Phase i=0: data bits at tones[18 + 3k] (k=0..6), parity at tones[39].
        let mut p0 = 0u8;
        for k in 0..7 {
            let b = (phase0_code >> k) & 1;
            p0 ^= b;
            t[18 + 3 * k] = bit_freq(b);
        }
        let p0 = if phase0_code == R12BW_VIS_CODE { p0 ^ 1 } else { p0 };
        t[39] = bit_freq(p0);
        // Phase i=1: data bits at tones[19 + 3k], parity at tones[40].
        if let Some(c) = phase1_code {
            let mut p1 = 0u8;
            for k in 0..7 {
                let b = (c >> k) & 1;
                p1 ^= b;
                t[19 + 3 * k] = bit_freq(b);
            }
            let p1 = if c == R12BW_VIS_CODE { p1 ^ 1 } else { p1 };
            t[40] = bit_freq(p1);
        }
        t
    }

    /// `is_known` predicate as used in production: a 7-bit VIS code is "known"
    /// iff it maps to a `ModeSpec`.
    fn vis_known(code: u8) -> bool {
        crate::modespec::lookup(code).is_some()
    }

    #[test]
    fn match_vis_pattern_clean_known_code() {
        // 0x5F == PD120 is in the lookup table.
        let tones = synth_tone_history(0x5F, None);
        let m = match_vis_pattern(&tones, vis_known).expect("known code matches");
        assert_eq!(m.0, 0x5F);
        assert!(m.1.abs() < 1e-9, "hedr_shift should be 0, got {}", m.1);
        assert_eq!(m.2, 0, "matched at phase i = 0");
    }

    #[test]
    fn match_vis_pattern_clean_unknown_code_falls_back() {
        // 0x01 is a valid 7-bit code with parity 1, but maps to no mode.
        assert!(crate::modespec::lookup(0x01).is_none());
        let tones = synth_tone_history(0x01, None);
        let m = match_vis_pattern(&tones, vis_known)
            .expect("an unknown-but-parity-passing code is still returned (fallback)");
        assert_eq!(m.0, 0x01);
        assert_eq!(m.2, 0);
    }

    #[test]
    fn match_vis_pattern_prefers_known_over_earlier_unknown() {
        // Phase i=0 spells unknown 0x01; phase i=1 spells known 0x5F.
        // The (i,j) loop hits i=0 first — the old code returned 0x01 there;
        // the fix must skip it and return 0x5F at i=1.
        let tones = synth_tone_history(0x01, Some(0x5F));
        let m = match_vis_pattern(&tones, vis_known).expect("known code at a later alignment");
        assert_eq!(m.0, 0x5F, "should skip the earlier unknown 0x01 alignment");
        assert_eq!(m.2, 1, "matched at phase i = 1");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib vis::tests::match_vis_pattern`
Expected: FAIL to compile — `match_vis_pattern` takes one argument, not two (`is_known` not yet added). (Once you add the parameter in Step 3 the `prefers_known_over_earlier_unknown` test would still fail against the *old* body because it returns the first parity-passing alignment.)

- [ ] **Step 3: Change `match_vis_pattern` — add `is_known`, keep-search, `first_unknown` fallback**

Signature (`src/vis.rs`): change

```rust
fn match_vis_pattern(tones: &[f64; HISTORY_LEN]) -> Option<(u8, f64, usize)> {
```

to

```rust
fn match_vis_pattern(
    tones: &[f64; HISTORY_LEN],
    is_known: impl Fn(u8) -> bool,
) -> Option<(u8, f64, usize)> {
```

Body: add `let mut first_unknown: Option<(u8, f64, usize)> = None;` immediately before `for i in 0..3 {`. Replace the trailing `if bit_ok { return Some((code, leader - LEADER_HZ, i)); }` with:

```rust
            if bit_ok {
                let m = (code, leader - LEADER_HZ, i);
                if is_known(code) {
                    return Some(m); // slowrx breaks on the first known code
                }
                first_unknown.get_or_insert(m);
            }
```

Replace the function's final `None` with `first_unknown`.

Update the doc comment on `match_vis_pattern`. The intro currently says
*"Returns `(vis_code, hedr_shift_hz, i)` on detection ..."* — change that to
*"Returns the first **known** parity-passing alignment as `(vis_code, hedr_shift_hz, i)` (or the first parity-passing **unknown** code as a fallback) ..."*. Keep the existing `## Deliberate divergence ... Finding 5` paragraph. Append a new paragraph:

```rust
/// **Keep-searching for a known code (issue #89, A3):** like slowrx (`vis.c`),
/// this tries all 9 alignments and returns the *first known* code (`is_known`).
/// A clean burst whose only parity-passing alignment maps to an *unknown* code
/// (a real R12BW transmission, or an unimplemented mode) is still returned — as
/// the `first_unknown` fallback — so the caller can surface it via
/// `SstvEvent::UnknownVis` (added in #89 — note: a plain code span, not an
/// intra-doc link, since `vis` is below `decoder` in the module graph). See
/// `docs/intentional-deviations.md`
/// ("VIS: keep-searching for a known code; surface clean unknown bursts").
```

- [ ] **Step 4: Add the `is_known_vis` field to `VisDetector`**

In the `VisDetector` struct (`src/vis.rs`), add a field (place it last, after `detected: Option<DetectedVis>,`):

```rust
    /// `|c| crate::modespec::lookup(c).is_some()` — passed to
    /// [`match_vis_pattern`] so the matcher keeps searching alignments for a
    /// *known* VIS code (issue #89 A3). An `fn` pointer: the closure captures
    /// nothing, so it coerces.
    is_known_vis: fn(u8) -> bool,
```

Change `VisDetector::new`:

```rust
    pub fn new() -> Self {
```

to

```rust
    pub fn new(is_known_vis: fn(u8) -> bool) -> Self {
```

and add `is_known_vis,` to the returned struct literal (alongside the other fields).

In `VisDetector::process`, change the `match_vis_pattern` call from

```rust
                if let Some((code, hedr_shift_hz, i_match)) =
                    match_vis_pattern(&self.rotated_history())
                {
```

to

```rust
                if let Some((code, hedr_shift_hz, i_match)) =
                    match_vis_pattern(&self.rotated_history(), self.is_known_vis)
                {
```

(`self.is_known_vis` is `Copy` — copying out the `fn` pointer does not conflict with `&mut self`.)

- [ ] **Step 5: Update the `run` test helper in `vis.rs`**

In `pub mod tests`, the `run` helper currently:

```rust
    fn run(audio: &[f32]) -> Option<DetectedVis> {
        let mut det = VisDetector::new();
        det.process(audio, audio.len() as u64);
        det.take_detected()
    }
```

becomes:

```rust
    fn run(audio: &[f32]) -> Option<DetectedVis> {
        let mut det = VisDetector::new(vis_known);
        det.process(audio, audio.len() as u64);
        det.take_detected()
    }
```

(`vis_known` is the helper added in Step 1.)

- [ ] **Step 6: Add the `IS_KNOWN_VIS` const + update `decoder.rs` call sites**

In `src/decoder.rs`, near the other crate-private items (immediately after `const FINDSYNC_AUDIO_HEADROOM: f64 = 1.00;`, currently line 139), add:

```rust
/// `|c| crate::modespec::lookup(c).is_some()` as an `fn` pointer — the
/// "is this VIS code one we can decode?" predicate handed to every
/// [`crate::vis::VisDetector`] (issue #89 A3). The closure captures nothing,
/// so it coerces to `fn(u8) -> bool` in const context.
const IS_KNOWN_VIS: fn(u8) -> bool = |c| crate::modespec::lookup(c).is_some();
```

Update the two construction sites:
- `SstvDecoder::new` (currently `vis: crate::vis::VisDetector::new(),` ~line 193) → `vis: crate::vis::VisDetector::new(IS_KNOWN_VIS),`
- The post-image reseed in `process` (currently `self.vis = crate::vis::VisDetector::new();` ~line 345) → `self.vis = crate::vis::VisDetector::new(IS_KNOWN_VIS);` (T3 refactors this whole block; this keeps it compiling in the meantime).

- [ ] **Step 7: Run the tests**

Run: `cargo test --lib`
Expected: PASS — the three new `match_vis_pattern` tests green, all existing `vis::tests` + `decoder::tests` still green.

- [ ] **Step 8: Commit**

```bash
git add src/vis.rs src/decoder.rs
git commit -m "feat(vis): keep-searching VIS alignments for a known code (#89 A3)"
```

---

## Task 3: `SstvEvent::UnknownVis` + `restart_vis_detection` + A1 fix + post-image refactor

The unknown-VIS branch in `SstvDecoder::process` currently drops the residual buffer and `break`s without reseeding `self.vis` — violating the `#40` re-anchor contract documented on `VisDetector::take_residual_buffer` (the successful-decode path and the post-image path both reseed). Fix it via a `restart_vis_detection` helper, emit a `SstvEvent::UnknownVis` for the dropped burst, and `continue` so a follow-up VIS already in the residual surfaces in the same `process()` call.

**Files:**
- Modify: `src/decoder.rs` (`SstvEvent` enum; `restart_vis_detection` helper; unknown-VIS branch; post-image reseed)
- Create: `tests/unknown_vis.rs`

- [ ] **Step 1: Write the failing integration test**

Create `tests/unknown_vis.rs`:

```rust
//! `SstvDecoder` must surface an unrecognized-but-well-formed VIS burst as
//! `SstvEvent::UnknownVis` (rather than dropping it silently) and then keep
//! detecting subsequent valid VIS bursts — i.e. the unknown-code path reseeds
//! the VIS detector per the `#40` re-anchor contract (audit #89: A1 + C1).

#![cfg(feature = "test-support")]
#![allow(clippy::expect_used, clippy::cast_possible_truncation)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, WORKING_SAMPLE_RATE_HZ};

/// 0x01 is a valid 7-bit VIS code (parity 1) that maps to no SSTV mode.
const UNKNOWN_CODE: u8 = 0x01;
/// 0x5F == PD120.
const PD120_CODE: u8 = 0x5F;

#[test]
fn decoder_emits_unknown_vis_then_recovers() {
    // burst 1: a well-formed VIS for an unknown code.
    // burst 2: a well-formed VIS for PD120, immediately following.
    // trailing zeros so the resampler's FIR group delay still yields a full
    // set of stop-bit windows for burst 2.
    let mut audio = slowrx::__test_support::vis::synth_vis(UNKNOWN_CODE, 0.0);
    audio.extend(slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0));
    audio.extend(std::iter::repeat_n(0.0_f32, 512));

    let mut decoder = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder construct");
    let events = decoder.process(&audio);

    // Expect: UnknownVis { code: 0x01, .. } then VisDetected { mode: Pd120, .. }.
    let unknown_at = events
        .iter()
        .position(|e| matches!(e, SstvEvent::UnknownVis { code, .. } if *code == UNKNOWN_CODE))
        .unwrap_or_else(|| panic!("no UnknownVis event for 0x{UNKNOWN_CODE:02x}; got {events:?}"));
    let detected_at = events
        .iter()
        .position(|e| matches!(e, SstvEvent::VisDetected { mode: SstvMode::Pd120, .. }))
        .unwrap_or_else(|| panic!("no VisDetected for PD120 after the unknown burst; got {events:?}"));
    assert!(
        unknown_at < detected_at,
        "UnknownVis should precede the recovered VisDetected; got {events:?}"
    );

    // The recovered detection's sample_offset should be a sane working-rate
    // index (non-zero, and not wildly past the end of the fed audio). Exact
    // semantics post-restart (relative-to-restart vs absolute) are tracked
    // separately; this is only a gross-corruption guard.
    if let SstvEvent::VisDetected { sample_offset, .. } = &events[detected_at] {
        assert!(
            *sample_offset > 0 && (*sample_offset as usize) <= audio.len(),
            "VisDetected.sample_offset = {sample_offset} out of [1, {}]",
            audio.len()
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features test-support --test unknown_vis`
Expected: FAIL to compile — `SstvEvent::UnknownVis` does not exist yet. (After Step 3 adds the variant but before the branch is rewritten, it would fail at runtime: the old code drops burst 2 via the early `break`, so no `VisDetected` is emitted.)

- [ ] **Step 3: Add the `SstvEvent::UnknownVis` variant**

In `src/decoder.rs`, in the `SstvEvent` enum, add a variant after `VisDetected { ... },` (and before `LineDecoded`):

```rust
    /// A VIS header parsed and passed parity, but its 7-bit code maps to no
    /// SSTV mode this build can decode (reserved / undefined, or a mode not
    /// yet implemented). The decoder discards the burst and resumes scanning
    /// for the next VIS — equivalent to slowrx's `printf("Unknown VIS %d")`
    /// plus its retry-`GetVIS()` loop.
    UnknownVis {
        /// The 7-bit VIS code that did not resolve.
        code: u8,
        /// Radio mistuning offset in Hz: `observed_leader_hz - 1900` (the
        /// same quantity as [`SstvEvent::VisDetected`]'s `hedr_shift_hz`).
        /// Surfaced for diagnostics; the burst is dropped, so it does not
        /// feed any decode.
        hedr_shift_hz: f64,
        /// Working-rate (11025 Hz) sample offset where the VIS stop bit ended.
        sample_offset: u64,
    },
```

- [ ] **Step 4: Add the `restart_vis_detection` helper**

In `src/decoder.rs`, inside `impl SstvDecoder`, add (place it next to `process` — e.g. immediately after `process`). It is an **associated** function taking `&mut self.vis` (not `&mut self`), mirroring the existing `Self::run_findsync_and_decode(...)` pattern — so it can be called from inside a `match &mut self.state` arm without a whole-`self` borrow conflict:

```rust
    /// Discard `vis` and start a fresh detector on `leftover_audio`
    /// (post-stop-bit residue, or trailing image audio). Honors the `#40`
    /// re-anchor contract documented on
    /// [`crate::vis::VisDetector::take_residual_buffer`] — a spent detector's
    /// `hops_completed` / `history` state is never reset, so it must be
    /// replaced rather than re-used. `working_samples_emitted` is the
    /// decoder's running working-rate output count (used to anchor the fresh
    /// detector).
    fn restart_vis_detection(
        vis: &mut crate::vis::VisDetector,
        working_samples_emitted: u64,
        leftover_audio: Vec<f32>,
    ) {
        *vis = crate::vis::VisDetector::new(IS_KNOWN_VIS);
        vis.process(&leftover_audio, working_samples_emitted);
    }
```

- [ ] **Step 5: Rewrite the unknown-VIS branch in `process`**

In `SstvDecoder::process`, the `State::AwaitingVis` arm currently ends (after the `if let Some(spec) = ... { ... continue; }` block) with:

```rust
                        // Unknown VIS codes silently drop. Reset the
                        // detector's buffer so it does not accumulate
                        // forever on uninterpretable bursts.
                        let _ = self.vis.take_residual_buffer();
                    }
                    break;
```

Replace the `let _ = self.vis.take_residual_buffer();` line with:

```rust
                        // Unknown VIS code: surface it so stream-monitoring
                        // callers know a burst arrived, then reseed the
                        // detector (the `#40` re-anchor contract) on the
                        // post-stop-bit residue and re-enter the loop — a
                        // back-to-back VIS in the residue then surfaces in
                        // this same `process` call. Mirrors the known-code
                        // branch's `continue`.
                        out.push(SstvEvent::UnknownVis {
                            code: detected.code,
                            hedr_shift_hz: detected.hedr_shift_hz,
                            sample_offset: detected.end_sample,
                        });
                        let residual = self.vis.take_residual_buffer();
                        Self::restart_vis_detection(
                            &mut self.vis,
                            self.working_samples_emitted,
                            residual,
                        );
                        continue;
```

The `}` closing the `if let Some(detected) = self.vis.take_detected()` block and the trailing `break;` stay.

- [ ] **Step 6: Refactor the post-image reseed to use the helper**

In the `State::Decoding(d)` arm of `process`, the block currently (the comment block plus):

```rust
                    let trailing = std::mem::take(&mut d.audio);
                    self.state = State::AwaitingVis;
                    self.vis = crate::vis::VisDetector::new(IS_KNOWN_VIS);
                    self.vis.process(&trailing, self.working_samples_emitted);
                    break;
```

becomes:

```rust
                    let trailing = std::mem::take(&mut d.audio);
                    self.state = State::AwaitingVis;
                    Self::restart_vis_detection(
                        &mut self.vis,
                        self.working_samples_emitted,
                        trailing,
                    );
                    break;
```

(Keep the surrounding `// Image complete. Preserve trailing audio ... Closes #31.` comment block. This is a pure DRY change — no behavior difference. The `mem::take`-vs-`split_off` question on that line is issue #90's, untouched here.)

- [ ] **Step 7: Run the tests**

Run: `cargo test --features test-support`
Expected: PASS — `tests/unknown_vis.rs::decoder_emits_unknown_vis_then_recovers` green; all existing tests still green.

- [ ] **Step 8: Commit**

```bash
git add src/decoder.rs tests/unknown_vis.rs
git commit -m "feat(decoder): SstvEvent::UnknownVis + reseed VIS detector after unknown code (#89 A1/C1)"
```

---

## Task 4: Delete the dead `Error::UnknownVisCode`

`Error::UnknownVisCode(u8)` is never constructed — `SstvDecoder::process` returns `Vec<SstvEvent>`, never `Result`, and `SstvDecoder::new` only fails on a bad sample rate. The unknown-code path now emits `SstvEvent::UnknownVis` instead. Remove the variant and its test.

**Files:**
- Modify: `src/error.rs`

- [ ] **Step 1: Delete the variant**

In `src/error.rs`, remove the `UnknownVisCode` variant from `pub enum Error` (the doc comment, the `#[error(...)]` attribute, and `UnknownVisCode(u8),` — currently lines 17-21):

```rust
    /// VIS code does not map to a known SSTV mode.
    ///
    /// The `u8` value is the raw 7-bit VIS byte read from the audio stream.
    #[error("VIS code {0:#04x} does not map to a known SSTV mode")]
    UnknownVisCode(u8),
```

Leave `InvalidSampleRate { got: u32 }`. The enum stays `#[non_exhaustive]`.

- [ ] **Step 2: Delete the variant's test**

In `src/error.rs`'s `#[cfg(test)] mod tests`, remove the whole `unknown_vis_code_renders_in_hex` test:

```rust
    #[test]
    fn unknown_vis_code_renders_in_hex() {
        let e = Error::UnknownVisCode(0x42);
        assert_eq!(
            e.to_string(),
            "VIS code 0x42 does not map to a known SSTV mode"
        );
    }
```

Leave `invalid_sample_rate_renders_with_value`.

- [ ] **Step 3: Confirm nothing else references it**

Run: `grep -rn "UnknownVisCode" src/ tests/ examples/`
Expected: no output.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib error`
Expected: PASS — `invalid_sample_rate_renders_with_value` green; nothing references the deleted variant.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs
git commit -m "refactor(error): remove never-constructed Error::UnknownVisCode (#89 C1)"
```

---

## Task 5: Docs — `modespec::lookup` note, `intentional-deviations.md`, `CHANGELOG.md`

**Files:**
- Modify: `src/modespec.rs` (the `lookup` `#27` doc note)
- Modify: `docs/intentional-deviations.md` (new section)
- Modify: `CHANGELOG.md` (`[Unreleased]`)

- [ ] **Step 1: Tweak the `lookup` doc note**

In `src/modespec.rs`, the `lookup` doc comment's `#27` paragraph currently reads (the relevant sentences):

```rust
/// **Parity-audit note (#27):** `0x00` is intentionally unmapped and
/// returns `None`. In slowrx (`vis.c:172-174`), an unknown VIS code causes
/// `GetVIS()` to return 0 and `Listen()` loops back to re-detect
/// (`do { ... } while (Mode == 0)`). Rust's equivalent is `None` from
/// this function: the caller in `SstvDecoder::process` drains the VIS
/// detector's buffer and stays in `AwaitingVis`, which has the same
/// effect as slowrx's re-detect loop. Both treat an unknown code as a
/// silent "try again" rather than an error.
```

Replace the last two sentences with:

```rust
/// this function: the caller in `SstvDecoder::process` emits
/// `SstvEvent::UnknownVis`, reseeds the VIS detector on the post-stop-bit
/// residue, and stays in `AwaitingVis` — the same "try again" effect as
/// slowrx's re-detect loop (slowrx's `printf("Unknown VIS")` becomes the
/// `UnknownVis` event). An unknown code is never an `Error`.
```

- [ ] **Step 2: Add the `intentional-deviations.md` section**

In `docs/intentional-deviations.md`, add a new top-level section. Place it right after the existing `## VIS retry behavior on parity failure` section (i.e. before `## Synthetic round-trip max_diff tolerance`) so the VIS entries are grouped:

```markdown
## VIS: keep-searching for a known code; surface clean unknown bursts

### What slowrx does

`vis.c`'s pattern matcher tries all 9 `(i, j)` alignments. For each alignment
that passes parity it does `VISmap[VIS]` — and if the code is unknown it
`printf`s `"Unknown VIS %d"`, leaves `gotvis = false`, and keeps trying the
remaining alignments. Only a *known* code stops the search (`gotvis = true`).
If no alignment yields a known code, `GetVIS()` returns `0` and `Listen()`'s
`do { ... } while (Mode == 0)` loop re-detects from the audio stream.

### What we do

`match_vis_pattern` takes an `is_known` predicate (`|c| modespec::lookup(c).is_some()`)
and mirrors slowrx: it tries all 9 alignments and returns the **first known**
code. If no alignment is known but at least one parity-passing alignment maps
to an **unknown** code, it returns the first such code as a fallback. The
decoder then emits `SstvEvent::UnknownVis { code, hedr_shift_hz, sample_offset }`
(our equivalent of slowrx's `printf`), reseeds the VIS detector on the
post-stop-bit residue (the `#40` re-anchor contract), and stays in
`AwaitingVis` — the same "try again" loop as slowrx.

### Why we deviated

slowrx's `printf` is a console side effect with no programmatic surface; a
library decoder should let callers observe "a burst arrived but I can't decode
it" (stream monitors, diagnostics). Returning the unknown code as a fallback
from `match_vis_pattern` (rather than `None`, which slowrx effectively does) is
what makes that event possible. The keep-searching-for-a-known-code behavior is
otherwise byte-for-byte slowrx's.

### When to revisit

If R12BW (or any other currently-unimplemented mode) gains a `ModeSpec`, it
becomes "known" automatically (the predicate is `lookup(...).is_some()`), and
bursts for it stop surfacing as `UnknownVis` and start decoding. Nothing else
to change.
```

- [ ] **Step 3: Add the `CHANGELOG.md` `[Unreleased]` entries**

In `CHANGELOG.md`, under the `## [Unreleased]` header, add:

```markdown
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

### Removed

- **`Error::UnknownVisCode`** — never constructed (`SstvDecoder::process` does
  not return `Result`; unknown codes are now reported via `SstvEvent::UnknownVis`).

### Internal

- `R12BW_VIS_CODE` named constant replacing bare `0x06` literals; faithful
  parity-check shape in `match_vis_pattern` (flip the accumulated parity for
  R12BW, matching slowrx `vis.c:116`) (#89 C5/C15).
```

(If `[Unreleased]` already has `### Added` / `### Fixed` / etc. subsections, merge into them in Keep-a-Changelog order rather than duplicating headers.)

- [ ] **Step 4: Verify the docs build**

Run: `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features` and `cargo test --doc --all-features`
Expected: PASS — no broken intra-doc links (the new doc comments reference `SstvEvent::UnknownVis`, `crate::vis::VisDetector::take_residual_buffer`, `match_vis_pattern` — all resolvable).

- [ ] **Step 5: Commit**

```bash
git add src/modespec.rs docs/intentional-deviations.md CHANGELOG.md
git commit -m "docs: VIS keep-searching deviation + SstvEvent::UnknownVis changelog (#89)"
```

---

## Task 6: Full CI gate

**Files:** none (verification + any lint fixes)

- [ ] **Step 1: `cargo fmt`**

Run: `cargo fmt --all`
Then: `cargo fmt --all -- --check`
Expected: PASS (no diff).

- [ ] **Step 2: `cargo clippy`**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS. If a warning fires (e.g. `clippy::doc_markdown` on `SstvEvent` in a doc comment, or `clippy::missing_panics_doc`), fix it minimally — backtick-wrap identifiers in docs, etc. Do **not** add blanket `#[allow]`s.

- [ ] **Step 3: `cargo test` (release, all features, locked)**

Run: `cargo test --all-features --locked --release`
Expected: PASS — full suite green, including `tests/unknown_vis.rs`, `tests/roundtrip.rs`, `tests/no_vis.rs`.

- [ ] **Step 4: `cargo doc` (deny warnings)**

Run: `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
Expected: PASS.

- [ ] **Step 5: Commit any fixes**

If Steps 1-4 produced changes:

```bash
git add -A
git commit -m "chore: satisfy fmt/clippy/doc gate (#89)"
```

If nothing changed, skip the commit. The branch is now ready for the final review and a PR against `main`.

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:** A1 → T3 Steps 4-6; A3 → T2 Steps 3-4 (+ unit tests T2 Step 1); C1 → T3 Step 3 (`UnknownVis`) + T4 (delete `Error::UnknownVisCode`); C5 → T1 Step 1, 3; C15 → T1 Step 2. `intentional-deviations.md` entry → T5 Step 2. `CHANGELOG` → T5 Step 3. `modespec::lookup` doc → T5 Step 1.
- **`fn(u8) -> bool` in `const`:** `const IS_KNOWN_VIS: fn(u8) -> bool = |c| crate::modespec::lookup(c).is_some();` is valid — the closure captures nothing, so it coerces to a function pointer; only the pointer (a link-time constant) is stored, the body is not const-evaluated.
- **`SstvEvent` is `#[non_exhaustive]`** — adding `UnknownVis` is a non-breaking addition; all existing internal matches are `matches!`/`if let`, never exhaustive, so nothing else needs touching.
- **`process` signature is unchanged** — only `VisDetector::new` gains a parameter; the two `new` call sites are in `decoder.rs` (covered) and the `run` test helper in `vis.rs` (covered T2 Step 5). `decoder.rs::tests` does not construct a `VisDetector` directly.
- **Out of scope (do not touch):** the post-image `break`-vs-`continue` and `mem::take`-vs-`split_off` of `d.audio` (issue #90); `sample_offset`-absolute-vs-relative-after-restart semantics (tracked with #90 / the `working_samples_emitted` note); any wider `vis.rs` decomposition (#88).
