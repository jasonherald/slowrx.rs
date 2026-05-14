# Issue #86 — Shared test-tone module + encoders off the public API — design

**Issue:** [#86](https://github.com/jasonherald/slowrx.rs/issues/86) (audit bundle 2 of 12 — IDs B9, B10, E2, F11, C17 partial)
**Source of record:** `docs/audits/2026-05-11-deep-code-review-audit.md`
**Scope:** the test-encoder-side complement to #85. Pure refactor — identical behavior, no public-API change for stable consumers.

## Background

The audit found:

- **B9** — the continuous-phase FM tone generator (`SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` consts + `lum_to_freq` + `fill_to`) is duplicated nearly byte-for-byte across `pd_test_encoder.rs`, `robot_test_encoder.rs`, `scottie_test_encoder.rs`, and `vis::tests::synth_vis_with_offset` (+ a second inline copy in `vis::tests::r12bw_rejects_standard_parity`). ~100+ LOC of duplication.
- **B10** — the test-only encoders sit on the *published* API surface with two stable-looking paths each: `slowrx::pd_test_encoder::encode_pd` (via the `pub mod pd_test_encoder;` declaration in `lib.rs`) AND `slowrx::__test_support::mode_pd::encode_pd` (via the `pub use` re-export). Both reachable with `test-support` on. The audit wants only `__test_support` as the consumer-facing path.
- **E2** — `scottie_test_encoder::encode_scottie` has TWO `///` doc-comment paragraphs straddling `#[must_use]`; the first is stale Scottie-only, the second is the accurate Scottie-or-Martin one. rustdoc concatenates both.
- **F11** — only `scottie_test_encoder` has unit tests (2 of them). `pd_test_encoder` and `robot_test_encoder` have none — a regression in channel order or septr placement surfaces only as a fuzzy `mean ≥ 5.0` in `tests/roundtrip.rs`, not a pointed failure.
- **C17 partial** — the test encoders have `unreachable!()` / `.expect()` arms after `assert!(matches!(mode, ...))` guards on the same variant set — redundant.

## Design

### Part 1 — Module structure

**New: `src/test_tone.rs`** — `cfg(any(test, feature = "test-support"))`-gated, `pub(crate) mod test_tone;` in `lib.rs`. Contains the consolidated tone generator:

```rust
//! Continuous-phase FM tone generator for the synthetic test encoders.
//! Consolidates the four pre-#86 copies in `pd_test_encoder.rs`,
//! `robot_test_encoder.rs`, `scottie_test_encoder.rs`, and `vis::tests`.

pub(crate) const SYNC_HZ:  f64 = 1200.0;
pub(crate) const PORCH_HZ: f64 = 1500.0;
pub(crate) const SEPTR_HZ: f64 = 1500.0;  // same value as PORCH_HZ/BLACK_HZ; named for SSTV-spec clarity
pub(crate) const BLACK_HZ: f64 = 1500.0;
pub(crate) const WHITE_HZ: f64 = 2300.0;

pub(crate) fn lum_to_freq(lum: u8) -> f64 {
    BLACK_HZ + (WHITE_HZ - BLACK_HZ) * f64::from(lum) / 255.0
}

/// Continuous-phase FM tone writer. Owns the output `Vec<f32>` and a running
/// `phase` accumulator. Both [`fill_to`] (cumulative-target — encoder style,
/// prevents per-tone rounding drift across a 640-pixel line) and [`fill_secs`]
/// (per-tone duration — VIS-burst style) advance the same `phase` so
/// consecutive tones never produce an audible discontinuity.
pub(crate) struct ToneWriter {
    out: Vec<f32>,
    phase: f64,
}

impl ToneWriter {
    pub fn new() -> Self;
    pub fn with_pre_silence_samples(n: usize) -> Self;
    pub fn with_capacity(cap: usize) -> Self;

    /// Emit samples up to absolute output index `target_n` (exclusive) at
    /// `freq_hz`. Cumulative-target — call repeatedly with increasing
    /// `target_n` across a multi-channel line; per-pixel rounding error
    /// never compounds.
    pub fn fill_to(&mut self, freq_hz: f64, target_n: usize);

    /// Emit `secs` seconds at `freq_hz`. Per-tone-duration form. Used by VIS
    /// bursts where each tone has a fixed wall-clock duration.
    pub fn fill_secs(&mut self, freq_hz: f64, secs: f64);

    pub fn len(&self) -> usize;
    pub fn into_vec(self) -> Vec<f32>;
}
```

`fill_to` and `fill_secs` share the same body shape (advance `phase` by `2π·freq/sr` per sample, wrap when `phase > 2π`); they differ only in the stopping condition (`out.len() < target_n` vs an explicit sample count derived from `secs * sr`).

Plus `#[cfg(test)] mod tests` with three unit tests:
- `tone_writer_phase_is_continuous_across_tone_boundaries` — fill two consecutive different-freq tones, assert no discontinuity at the boundary (sample-to-sample delta < expected per-sample step + margin).
- `lum_to_freq_endpoints` — `lum_to_freq(0) == BLACK_HZ`, `lum_to_freq(255) == WHITE_HZ`.
- `fill_to_and_fill_secs_produce_equivalent_output_for_same_duration` — `fill_to(freq, n)` and `fill_secs(freq, n_as_secs)` produce identical samples (modulo `.round()`).

**`pub mod` → `pub(crate) mod` flip** in `src/lib.rs`. The three lines

```rust
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod scottie_test_encoder;
```

become:

```rust
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod scottie_test_encoder;
```

`#[doc(hidden)]` is no longer needed — `pub(crate)` doesn't appear in `cargo doc` at all. The `slowrx::pd_test_encoder::*` / `robot_test_encoder::*` / `scottie_test_encoder::*` external paths disappear entirely.

**`__test_support` switches from `pub use` to thin wrapper fns.** `pub use crate::pd_test_encoder::encode_pd;` of a `pub(crate)`-effective item from inside `pub mod __test_support` would trip rustc's "private item in public interface" check (same issue as `ycbcr_to_rgb` in #85). The fix: wrap each encoder in a thin `pub fn` whose body just delegates to the now-`pub(crate)` implementation.

Today:

```rust
pub mod __test_support {
    pub mod vis { pub use crate::vis::tests::synth_vis; }
    pub mod mode_pd {
        pub use crate::demod::ycbcr_to_rgb;
        pub use crate::pd_test_encoder::encode_pd;
    }
    pub mod mode_robot { pub use crate::robot_test_encoder::encode_robot; }
    pub mod mode_scottie { pub use crate::scottie_test_encoder::encode_scottie; }
}
```

After:

```rust
pub mod __test_support {
    pub mod vis {
        // synth_vis stays a `pub use` — vis::tests is `pub mod tests` with
        // a `pub fn synth_vis`, so re-exporting at a `pub` path is legal.
        pub use crate::vis::tests::synth_vis;
    }
    pub mod mode_pd {
        // ycbcr_to_rgb is pub + #[doc(hidden)] (from #85); the `pub use` works.
        pub use crate::demod::ycbcr_to_rgb;

        /// Thin wrapper around the now-`pub(crate)` `crate::pd_test_encoder::encode_pd`.
        /// `__test_support` is the sole consumer-facing path for the synthetic
        /// PD encoder (#86 B10).
        #[doc(hidden)]
        #[must_use]
        pub fn encode_pd(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
            crate::pd_test_encoder::encode_pd(mode, ycrcb)
        }
    }
    pub mod mode_robot {
        #[doc(hidden)]
        #[must_use]
        pub fn encode_robot(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
            crate::robot_test_encoder::encode_robot(mode, ycrcb)
        }
    }
    pub mod mode_scottie {
        #[doc(hidden)]
        #[must_use]
        pub fn encode_scottie(mode: crate::modespec::SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
            crate::scottie_test_encoder::encode_scottie(mode, rgb)
        }
    }
}
```

The three wrapper signatures match the underlying fns exactly (verify against the current `pub fn encode_*` definitions; preserve the exact parameter types). Consumer paths (`slowrx::__test_support::mode_pd::encode_pd` etc.) are stable; `tests/roundtrip.rs` keeps working without changes.

**Inside each encoder module:** `pub fn encode_*` → `pub(crate) fn encode_*` (outer module is now `pub(crate)`, so `pub` is unnecessary). The `#[doc(hidden)]` attributes on the fns become redundant — drop them. Other `#[allow]`s (`dead_code`, `too_many_lines` on scottie) stay.

### Part 2 — `ToneWriter` consumption + `vis::tests` migration

**Each encoder** loses its local `SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` consts, its local `fn lum_to_freq`, and its local `fn fill_to`. They get an `use crate::test_tone::{...}` import at the top instead.

The encoder body pattern changes from:

```rust
let mut out: Vec<f32> = Vec::with_capacity(estimated_total_samples);
let mut phase = 0.0_f64;
// ... many fill_to(&mut out, freq, target_n, &mut phase) calls ...
out
```

to:

```rust
let mut tone = ToneWriter::with_capacity(estimated_total_samples);
// ... many tone.fill_to(freq, target_n) calls ...
tone.into_vec()
```

`scottie_test_encoder` if it currently pre-pads with silence (`let mut out: Vec<f32> = vec![0.0; pre_silence_samples];`) uses `ToneWriter::with_pre_silence_samples(...)` instead. (Verify during implementation; if pd / robot encoders also do this, same substitution.)

**`vis::tests::synth_vis_with_offset`** currently rolls a per-tone `emit` closure (`fill_secs`-style — wall-clock duration). After the refactor:

```rust
// Before:
let mut out: Vec<f32> = vec![0.0; (pre_silence_secs * sr).round() as usize];
let mut phase = 0.0_f64;
let mut emit = |freq, secs, out: &mut Vec<f32>| { /* ~10-line phase-advance loop */ };
emit(leader, 0.300, &mut out);
emit(break_f, 0.030, &mut out);
// ... 7 bit emits + parity + trailing break ...
out

// After:
let mut tone = crate::test_tone::ToneWriter::with_pre_silence_samples(
    (pre_silence_secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize,
);
tone.fill_secs(leader, 0.300);
tone.fill_secs(break_f, 0.030);
// ... 7 bit fill_secs + parity + trailing break ...
tone.into_vec()
```

The VIS frequency math (`leader + BREAK_HZ_OFFSET`, `bit_freq(bit)`, etc.) stays in `synth_vis_with_offset` — those are VIS-classifier-relative offsets, not encoder consts. `ToneWriter` is freq-in-Hz-only.

**`vis::tests::r12bw_rejects_standard_parity`** has a second inline `emit`-closure copy (audit's note). Same substitution.

`vis::tests::synth_tone` / `synth_tone_n` (the bare-tone helpers, no phase carry) are unrelated to `ToneWriter` — they stay as-is.

### Part 3 — E2, F11, C17, CHANGELOG, verification

**E2 — scottie double-doc fix.** `scottie_test_encoder::encode_scottie` currently has:

```rust
/// (paragraph 1 — stale Scottie-only "Per radio line, emits in this order: ...")
#[must_use]
/// (paragraph 2 — accurate Scottie-or-Martin "Encode an RGB image as continuous-phase FM ...")
#[doc(hidden)]
#[allow(dead_code, clippy::too_many_lines)]
pub fn encode_scottie(...)
```

After: delete paragraph 1 entirely; `#[doc(hidden)]` drops (visibility tightens to `pub(crate)`); keep `#[must_use]`, `#[allow(...)]`:

```rust
/// (paragraph 2 — kept verbatim, the accurate Scottie-OR-Martin doc.)
#[must_use]
#[allow(dead_code, clippy::too_many_lines)]
pub(crate) fn encode_scottie(...)
```

**F11 — encoder smoke tests.** Add to `pd_test_encoder::tests` and `robot_test_encoder::tests`:

```rust
// First-tone-is-SYNC check — a regression in channel order surfaces as a
// pointed failure here instead of a fuzzy roundtrip pixel-diff.
#[test]
fn encode_<mode>_emits_expected_first_tone_at_sync_hz() {
    let spec = crate::modespec::for_mode(SstvMode::Pd120 /* or Robot72 */);
    let img = vec![[128, 128, 128]; (spec.line_pixels * spec.image_lines) as usize];
    let audio = encode_<pd|robot>(SstvMode::Pd120 /* or Robot72 */, &img);
    let sync_samples = (spec.sync_seconds * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
    let p_sync  = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::SYNC_HZ);
    let p_porch = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::PORCH_HZ);
    assert!(p_sync > 10.0 * p_porch, "line starts with SYNC tone");
}

// Length-matches-radio-frames check — catches a structural drift (extra/missing
// septr, wrong channel count) without round-tripping.
#[test]
fn encode_<mode>_length_matches_radio_frames() {
    let spec = crate::modespec::for_mode(SstvMode::Pd120 /* or Robot72 */);
    let img = vec![[0_u8; 3]; (spec.line_pixels * spec.image_lines) as usize];
    let audio = encode_<pd|robot>(SstvMode::Pd120 /* or Robot72 */, &img);
    // PD packs 2 image rows / radio frame, so denominator is 2; Robot is 1.
    let radio_frames = f64::from(spec.image_lines) / /* 2.0 for PD, 1.0 for Robot */;
    let expected = (radio_frames * spec.line_seconds * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
    assert!((audio.len() as i64 - expected as i64).abs() < 64, "audio len {} ≉ {}", audio.len(), expected);
}
```

For Robot, the SYNC-first check targets one of the three Robot variants (pick Robot72 — 3 channels, no chroma alternation, simplest test setup). Existing `scottie_test_encoder` tests stay (update `SYNC_HZ` etc. references to `crate::test_tone::SYNC_HZ`).

**C17 partial — `unreachable!()` / `expect()` after `assert!(matches!(...))`.** Each encoder opens with `assert!(matches!(mode, ...))` to constrain the supported variants, then later has `_ => unreachable!()` / `.expect("...")` arms on the same set. Resolution per site:

- If the `unreachable!()` arm is on a `match` that already covers all supported variants exhaustively (no wildcard needed because the assert already proved exhaustiveness): replace the `match` with a closed form that doesn't need a wildcard.
- If the wildcard is structurally required (e.g. `SstvMode` is `#[non_exhaustive]` — which it is): leave the `unreachable!()` but drop the redundant leading `assert!(matches!(...))` — the `unreachable!()` is sufficient (and will panic with a clearer message at the actual problem site instead of pre-emptively in the assert).

The implementer picks the cleaner shape per site. Don't add new `#[allow]`s. If neither cleanup applies cleanly, leave both AND add an inline comment noting why both stay.

**CHANGELOG `[Unreleased]` `### Internal`** entry:

> Extracted `crate::test_tone` (continuous-phase FM tone generator) — consolidates the four pre-#86 copies in `pd_test_encoder` / `robot_test_encoder` / `scottie_test_encoder` / `vis::tests`. Tightened the test-encoder modules to `pub(crate) mod` so `slowrx::pd_test_encoder::*` (and `robot_test_encoder::*` / `scottie_test_encoder::*`) are no longer reachable externally; `__test_support` switched from `pub use` re-exports to thin `pub fn` wrappers (the sole consumer-facing path). Plus the scottie double-doc fix (E2), smoke tests for `encode_pd` / `encode_robot` (F11), and `unreachable!`/`expect` cleanup in the test encoders (C17 partial). Pure refactor: identical behavior, `slowrx::__test_support::mode_*::encode_*` paths stable for existing consumers. (#86; audit B9/B10/E2/F11/C17.)

**Verification** — full local CI gate per repo convention:
- `cargo test --all-features --locked --release` — `tests/roundtrip.rs` exercises all three encoders end-to-end on every mode; any drift in tone math or channel ordering surfaces here. Plus the new F11 smoke tests.
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
- Sanity grep — empty after the refactor:
  ```bash
  grep -rn "fn lum_to_freq\|fn fill_to\|SYNC_HZ:\s*f64\|PORCH_HZ:\s*f64\|BLACK_HZ:\s*f64\|WHITE_HZ:\s*f64\|SEPTR_HZ:\s*f64\|pub mod pd_test_encoder\|pub mod robot_test_encoder\|pub mod scottie_test_encoder" src/
  ```
  (`crate::test_tone` itself is the one legitimate definition site for the consts and `lum_to_freq`; the encoder modules are now `pub(crate) mod`.)

## Out of scope

- The remaining audit cleanups not in #86's bundle (#87 / #88 / #91-96) — separate.
- `vis::tests::synth_vis` / `synth_tone` / `synth_tone_n` (the bare-tone helpers that don't carry phase) — staying in `vis::tests`; only the `synth_vis_with_offset` continuous-phase emitter migrates to `ToneWriter`.
- The audit's deeper "move encoders to `tests/common/`" option — considered, rejected during brainstorming in favor of the lighter `pub(crate) mod` + wrapper-fn resolution that achieves the same B10 outcome without restructuring integration tests.

## Files touched

`src/test_tone.rs` (new), `src/lib.rs` (module decl flips + `__test_support` wrapper fns), `src/pd_test_encoder.rs`, `src/robot_test_encoder.rs`, `src/scottie_test_encoder.rs`, `src/vis.rs` (the `tests` module's `synth_vis_with_offset` + `r12bw_rejects_standard_parity` emit closures), `CHANGELOG.md`.
