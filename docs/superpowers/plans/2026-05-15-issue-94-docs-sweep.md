# Issue #94 — Docs sweep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pure documentation cleanup across 10 source files + `docs/intentional-deviations.md` — refresh stale module docs (E3, E4), fix contradictory or imprecise inline comments (E5, E9, E13), correct a misleading rustdoc table (E6), align slowrx C line refs against the gitignored local reference clone in `original/slowrx/` (E10), restructure issue-archaeology in `decode_pd_line_pair` (E12), add two new sections to `intentional-deviations.md` separating "faithful-to-slowrx artifacts" from "fidelity improvements" (E8), and record the deferred `mode_scottie → mode_rgb_sequential` rename (B15).

**Architecture:** Seven sequential tasks. T1 handles top-of-file module-doc rewrites + version-pinned prose + the rename NOTE (E3 + E4 + B15). T2 is the standalone `get_bin` bin-scaling caveat (E6). T3 is all `mode_pd.rs` inline cleanups in one place (E5 + E9 + E12 — closely coupled). T4 is the remaining inline cleanups across 4 files (E9 + E13). T5 is the big mechanical line-ref alignment sweep across 8 files + per-file disclaimer (E10). T6 adds the two new sections to `intentional-deviations.md` (E8). T7 closes with the CHANGELOG and final gate.

**Tech Stack:** No code; pure docs. CI gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. The last command is the load-bearing check (catches broken intra-doc links + malformed markdown in doc-comments). No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-15-issue-94-docs-sweep-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs E3, E4, E5, E6, E8, E9, E10, E12, E13, B15)
- Vendored slowrx snapshot: `original/slowrx/` (the reference for E10 line-ref verification)

---

## File Structure

| File | Status | Task |
|------|--------|------|
| `src/decoder.rs` | modify | T1 (E3 module doc) + T5 (E10 line-refs in body) |
| `src/modespec.rs` | modify | T1 (E4 version-pinned prose) |
| `src/lib.rs` | modify | T1 (E4 status + `__test_support`) |
| `src/dsp.rs` | modify | T2 (E6 `get_bin` caveat) |
| `src/mode_pd.rs` | modify | T3 (E5 + E9 `PIXEL_FFT_STRIDE` + E12 HISTORY block) + T5 (E10) |
| `src/snr.rs` | modify | T4 (E9 — 3 sites) + T5 (E10) |
| `src/mode_robot.rs` | modify | T4 (E9 + E13 — 2 sites) + T5 (E10) |
| `src/mode_scottie.rs` | modify | T1 (B15 NOTE) + T5 (E10) |
| `src/vis.rs` | modify | T4 (E13 HedrBuf) + T5 (E10) |
| `src/sync.rs` | modify | T4 (E13 — 2 sites) + T5 (E10) |
| `src/demod.rs` | modify | T5 (E10 — likely has slowrx C refs post-#85) |
| `docs/intentional-deviations.md` | modify | T6 (E8 — 2 new top-level sections) |
| `CHANGELOG.md` | modify | T7 (one bullet under `[Unreleased] ### Internal`) |

Task order: **T1** (module docs + B15) → **T2** (E6 get_bin) → **T3** (mode_pd.rs inline) → **T4** (cross-file inline E9 + E13) → **T5** (E10 line-ref alignment) → **T6** (E8 deviations doc) → **T7** (CHANGELOG + gate).

**Verification after each task** (the CI gate):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Lib test count baseline: **136** (post-#93). Every task leaves the count at **136** (pure docs).

---

## Task 1: E3 + E4 + B15 — Module doc rewrites + version-prose + B15 NOTE

Four top-of-file edits.

**Files:**
- Modify: `src/decoder.rs`, `src/modespec.rs`, `src/lib.rs`, `src/mode_scottie.rs`

- [ ] **Step 1: Rewrite `src/decoder.rs:1-8` module doc (E3)**

Locate the existing module doc at the top of `src/decoder.rs`. Currently:

```rust
//! `SstvDecoder` — public state machine driving the decode pipeline.
//!
//! This is the V1 skeleton: state machine shell + public API surface.
//! VIS detection lands in PR-1; per-mode pixel decoding lands in PR-2.
//!
//! Translated in spirit from slowrx's `slowrx.c` `Listen()` loop +
//! `vis.c` `GetVIS()` + `video.c` `GetVideo()`. ISC License — see
//! `NOTICE.md`.
```

Replace with:

```rust
//! [`SstvDecoder`] — public state machine driving the decode pipeline.
//!
//! Two-pass per-image flow: VIS detection (via [`crate::vis`]) →
//! buffer audio in `Decoding` state until ~one image's worth → run
//! [`crate::sync::find_sync`] once to recover the slant-corrected
//! rate + line-zero `Skip` → burst-decode every row (via
//! [`crate::demod::decode_one_channel_into`] in the per-mode glue
//! for PD/Robot/Scottie/Martin) → emit `LineDecoded` events and a
//! final `ImageComplete`. Multi-image streaming is supported in one
//! `process()` call (issue #90).
//!
//! Translated in spirit from slowrx's `slowrx.c::Listen()` loop +
//! `vis.c::GetVIS()` + `video.c::GetVideo()`. ISC License — see
//! `NOTICE.md`. Inline `// slowrx <file>.c:NNN` references throughout
//! point at the gitignored local reference clone under `original/slowrx/`
//! (see `clone-slowrx.sh`); verified at audit #94 (2026-05-15).
```

The `// slowrx … verified at audit #94` line is the E10 per-file disclaimer, landed here as part of E3's rewrite.

- [ ] **Step 2: Verify the intra-doc links resolve**

The new doc references `[`crate::vis`]`, `[`crate::sync::find_sync`]`, `[`crate::demod::decode_one_channel_into`]`. These should be valid — verify quickly:

```bash
grep -n "pub mod vis\|pub mod sync\|pub mod demod" /data/source/slowrx.rs/src/lib.rs
```

Expected: all three modules are declared in `src/lib.rs` (likely as `pub(crate) mod vis;` etc., but rustdoc resolves intra-doc links across module visibility).

If `crate::demod::decode_one_channel_into` doesn't resolve (e.g., the function is `pub(crate)` and not reachable from `crate::demod` doc-link path), use the bare code-span form `\`decode_one_channel_into\`` for that one and leave the others as links.

- [ ] **Step 3: Generalize `src/modespec.rs:6-10` module doc (E4 #1)**

Locate the existing module doc near the top of `src/modespec.rs`. The version-pinned line currently reads:

```rust
//! Implemented as of V2.4 (0.5.0): PD120, PD180, PD240, Robot 24,
//! Robot 36, Robot 72, Scottie 1, Scottie 2, Scottie DX, Martin 1,
//! Martin 2. All RGB-sequential modes (Scottie + Martin) share a
//! single decode path; the per-line offsets branch on
//! [`SyncPosition`].
```

Replace with:

```rust
//! Implemented modes: PD120, PD180, PD240, Robot 24, Robot 36, Robot 72,
//! Scottie 1, Scottie 2, Scottie DX, Martin 1, Martin 2. All RGB-sequential
//! modes (Scottie + Martin) share a single decode path; the per-line
//! offsets branch on [`SyncPosition`].
```

The change is small: drop `as of V2.4 (0.5.0):` and the awkward line wrap.

- [ ] **Step 4: Generalize `src/lib.rs:8-19` `## Status` (E4 #2)**

Locate the `## Status` paragraph in `src/lib.rs`. Currently:

```rust
//! ## Status
//!
//! `0.5.x` — V2.4 published. PD120/PD180/PD240 + Robot 24/36/72 +
//! Scottie 1 / Scottie 2 / Scottie DX + Martin 1 / Martin 2 decoding
//! from raw audio. PD120/PD180 validated against ARISS Dec-2017;
//! Robot 36 validated against the ARISS Fram2 corpus (see
//! `tests/ariss_fram2_validation.md`). Scottie and Martin families
//! are synthetic round-trip-validated only — no Scottie or Martin
//! reference WAVs available. The public API is
//! `#[non_exhaustive]`-protected for additive growth as future
//! mode-family epics land. See
//! <https://github.com/jasonherald/slowrx.rs/issues/9> for the V2 roadmap.
```

Replace with:

```rust
//! ## Status
//!
//! PD120 / PD180 / PD240, Robot 24 / 36 / 72, Scottie 1 / 2 / DX, and
//! Martin 1 / 2 decode from raw audio. PD120 and PD180 are validated
//! against the ARISS Dec-2017 capture set; Robot 36 is validated
//! against the ARISS Fram2 corpus (see
//! `tests/ariss_fram2_validation.md`). Scottie and Martin families
//! are synthetic round-trip-validated only — no Scottie or Martin
//! reference WAVs are available. The public API is
//! `#[non_exhaustive]`-protected for additive growth. See
//! <https://github.com/jasonherald/slowrx.rs/issues/9> for the
//! roadmap.
```

The change drops the `\`0.5.x\` — V2.4 published.` version-anchor and the "as future mode-family epics land" + "V2 roadmap" phrasing in favor of a version-neutral framing.

- [ ] **Step 5: Generalize `src/lib.rs:154-158` `__test_support` doc (E4 #3)**

Locate the `__test_support` doc in `src/lib.rs`. The current line reads:

```rust
/// Test-support — exposed under the `test-support` feature for integration
/// tests in this crate (e.g., `tests/roundtrip.rs`). NOT part of the stable
/// public API; will be hidden behind `#[doc(hidden)]` until V1 publishes.
```

Replace with:

```rust
/// Test-support — exposed under the `test-support` feature for integration
/// tests in this crate (e.g., `tests/roundtrip.rs`). NOT part of the stable
/// public API; the module is `#[doc(hidden)]` and the items inside are thin
/// wrappers around `pub(crate)` internals — the API is
/// `#[non_exhaustive]`-protected for additive growth.
```

Drops "until V1 publishes." Reframes the protection mechanism (already in place — `#[doc(hidden)]` on the module is on line ~157 below the doc; verify).

- [ ] **Step 6: Add B15 `// NOTE:` block to `src/mode_scottie.rs`**

In `src/mode_scottie.rs`, locate the module doc at the top. Append a `// NOTE:` block immediately after the closing `//!` of the module doc and before any `use` statements:

```rust
// NOTE (audit #94 B15): the file is named `mode_scottie.rs` but its
// `decode_line` is shared by Martin 1 / Martin 2 (both
// `ChannelLayout::RgbSequential` per ModeSpec). A rename to
// `mode_rgb_sequential.rs` is tracked but deferred — the existing
// name is grep-able and the per-mode behavior branches on
// `spec.sync_position` (mid-line for Scottie, line-start for Martin)
// rather than file boundaries.
```

The `// NOTE:` block uses regular `//` comments (not `//!`) because it's documentation-for-developers, not user-facing rustdoc.

- [ ] **Step 7: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

**Critical check:** `RUSTDOCFLAGS="-D warnings" cargo doc` is the load-bearing step. If any intra-doc link in the new decoder.rs doc fails to resolve (e.g., `[`crate::demod::decode_one_channel_into`]`), rustdoc will fail. Fallback: use plain code-span backticks for any link that doesn't resolve.

- [ ] **Step 8: Commit**

```bash
git add src/decoder.rs src/modespec.rs src/lib.rs src/mode_scottie.rs
git commit -m "docs: E3 + E4 + B15 — module doc rewrites + version-prose + Scottie rename NOTE (#94)

- decoder.rs: rewrite the V1-skeleton module doc as the actual
  two-pass pipeline (VIS → buffer → find_sync → burst decode →
  ImageComplete). Plus the E10 per-file disclaimer anchoring inline
  slowrx C line refs to original/slowrx/.
- modespec.rs: drop \"Implemented as of V2.4 (0.5.0):\" version
  anchor.
- lib.rs: rewrite ## Status paragraph (drop \`0.5.x — V2.4 published\`
  anchor); generalize __test_support \"until V1 publishes\" to the
  actual protection mechanism (#[doc(hidden)] + pub(crate) wrappers).
- mode_scottie.rs: add B15 NOTE block documenting the
  mode_scottie → mode_rgb_sequential rename deferral.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: E6 — `get_bin` rustdoc bin-scaling caveat

Single small edit in `src/dsp.rs`.

**Files:**
- Modify: `src/dsp.rs`

- [ ] **Step 1: Add the bin-scaling caveat after the existing `get_bin` rustdoc table**

In `src/dsp.rs`, locate the `get_bin` function (around line 83, with its rustdoc above it). The existing rustdoc has a "Numerical verification" table headed:

```rust
/// # Numerical verification (both at slowrx-native 1024/44100 and our 256/11025
/// — same Hz/bin ratio, so bins are identical)
```

After the table (after the line `/// | 3400 Hz   | 78          |`), append a new doc paragraph:

```rust
///
/// **Bin-scaling caveat:** the table above is for FFT setups with
/// `fft_len/sample_rate ≈ 1024/44100 ≈ 256/11025` (the sync-band FFT;
/// see [`crate::sync::SYNC_FFT_LEN`]). The SNR estimator and per-pixel
/// FFTs run at `1024/11025` since 0.3.3, so their bin indices are ~4×
/// larger (e.g., 1200 Hz lands at bin 111, not 27). The function itself
/// is general over `fft_len` and `sample_rate_hz`; this caveat only
/// affects readers who mentally apply the table's values to a 1024/11025
/// context. See `docs/intentional-deviations.md::"FFT frequency
/// resolution exceeds slowrx C by 4×"`.
```

The caveat block goes immediately after the table closing line (`/// | 3400 Hz   | 78          |`) and before the `#[allow(...)]` attributes that precede the function.

- [ ] **Step 2: Verify the intra-doc link**

The new caveat references `[`crate::sync::SYNC_FFT_LEN`]`. Verify it resolves:

```bash
grep -n "pub(crate) const SYNC_FFT_LEN" /data/source/slowrx.rs/src/sync.rs
```

Expected: `SYNC_FFT_LEN` is a `pub(crate) const` in `src/sync.rs` (~line 54). Rustdoc should resolve the intra-doc link.

If rustdoc fails to resolve (because `pub(crate)` items aren't reachable from the public-doc graph), use plain code-span: `\`SYNC_FFT_LEN\``.

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

- [ ] **Step 4: Commit**

```bash
git add src/dsp.rs
git commit -m "docs(dsp): E6 — get_bin bin-scaling caveat for 1024/11025 case (#94)

The existing table is for the 256/11025 sync FFT (and equivalent
1024/44100 slowrx-native case — same Hz/bin ratio). The SNR estimator
and per-pixel FFTs run at 1024/11025 since 0.3.3, so their bin
indices are ~4× larger (1200 Hz lands at bin 111, not 27). Adds a
caveat block after the table.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: E5 + E9 + E12 — `mode_pd.rs` inline cleanups

Three coupled edits in `src/mode_pd.rs`: verify the dead `chan_bounds_abs` comment (E5), clarify the `PIXEL_FFT_STRIDE` `%`-guard comment (E9), restructure issue archaeology into a `// HISTORY:` block (E12).

**Files:**
- Modify: `src/mode_pd.rs`

- [ ] **Step 1: Verify E5 — `chan_bounds_abs` comment status**

The audit references `mode_pd.rs:330-343` for an inner comment about `chan_bounds_abs` "used to zero-pad outside the active channel" that contradicts the `#32-lifted` note above. The `chan_bounds_abs` parameter was removed in #85's B5 fold.

Run:

```bash
grep -nE "chan_bounds_abs|zero-pad outside" /data/source/slowrx.rs/src/mode_pd.rs
```

**If grep returns no matches** for `chan_bounds_abs`: the dead comment was already deleted with the parameter in #85. Mark E5 as "verified absent in current code" in the commit message. Skip this step's edits.

**If grep returns matches**: locate the inner comment that references `chan_bounds_abs` and "zero-pad outside the active channel" — likely a `//` comment inside a function body, not a doc comment. Delete the comment. The implementer reads the surrounding code (1-2 lines context) and confirms the comment is genuinely orphaned (no `chan_bounds_abs` references remain in mode_pd.rs).

- [ ] **Step 2: E9 — clarify the `PIXEL_FFT_STRIDE` `%`-guard**

The audit references `mode_pd.rs:228-237`. Run:

```bash
grep -nB2 -A5 "PIXEL_FFT_STRIDE" /data/source/slowrx.rs/src/mode_pd.rs
```

Expected: a `const PIXEL_FFT_STRIDE: usize = 1;` or similar declaration, plus one or more sites where `something % PIXEL_FFT_STRIDE` is computed (which always equals 0 since `PIXEL_FFT_STRIDE = 1`).

At each `% PIXEL_FFT_STRIDE` site, add an inline comment immediately above the guard:

```rust
// `PIXEL_FFT_STRIDE` is fixed at 1 in V1, so this modulo always
// evaluates to 0 and the branch is always taken. Kept as a
// structural placeholder for a future per-pixel stride > 1
// optimization that would interpolate or downsample within the
// channel. (Audit #94 E9.)
```

Don't delete the guard — the structural intent is "this is the place where per-pixel-stride logic would live if we ever add it." The compiler eliminates the always-true branch in release.

If `PIXEL_FFT_STRIDE` has been removed in a prior PR (post-#85): the spec's reference is stale. Verify via grep and mark E9 sub-finding as absent in the commit message.

- [ ] **Step 3: E12 — restructure `decode_pd_line_pair` rustdoc into rustdoc + `// HISTORY:` block**

Locate the `decode_pd_line_pair` function in `src/mode_pd.rs`:

```bash
grep -nE "^pub.*fn decode_pd_line_pair" /data/source/slowrx.rs/src/mode_pd.rs
```

The current rustdoc above the function has scattered `#32`/`#34`/`#40`/`#42` issue references mixed in with the "what this function does" framing. Read the entire rustdoc block (likely 30-50 lines) to understand its structure.

The restructure:

1. **Clean rustdoc** — keep only the "what this function does" content. Pixel-time formula. Channel sweep. ChanStart computation. Anything that explains the function's behavior to a reader.
2. **`// HISTORY:` block** — immediately after the closing `///` of the rustdoc, before the function signature (and before any `#[allow(...)]` attributes — those go between the HISTORY block and the `fn` line). Carries the issue archaeology in plain `//` comments.

Template for the HISTORY block:

```rust
// HISTORY (audit #94 E12):
//   #32 — chan_bounds_abs zero-pad parameter removed (B5 fold in #85);
//         the parameter was deleted; the inner zero-pad path is gone.
//   #34 — sample-counter inflation observation (closed; informational
//         only, no fix needed).
//   #40 — VisDetector::take_residual_buffer re-anchor contract;
//         spent detectors must be replaced, not reused.
//   #42 — find_sync 90° slant deadband (intentional deviation
//         documented in `docs/intentional-deviations.md`).
```

The exact `#NN` issue list depends on what the current rustdoc actually references. The implementer reads the current rustdoc, extracts the `#NN` mentions, and constructs HISTORY entries with a brief one-line note per issue. If the rustdoc references issues not in the template above, add them; if some template issues aren't actually referenced in the current rustdoc, drop them.

After the restructure, the clean rustdoc should contain NO `#NN` references — the HISTORY block carries all of them.

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

**Critical check:** if `decode_pd_line_pair` was called from any doctests (unlikely — it's a `pub(crate)` function), the rustdoc restructure could break them. Check via grep for `decode_pd_line_pair` in any `///` comment elsewhere in the codebase.

- [ ] **Step 5: Commit**

```bash
git add src/mode_pd.rs
git commit -m "docs(mode_pd): E5 + E9 + E12 — chan_bounds_abs / PIXEL_FFT_STRIDE / HISTORY block (#94)

- E5: deleted the inner comment about chan_bounds_abs zero-pad
  (or verified already removed with the #85 B5 parameter delete).
- E9: clarified the PIXEL_FFT_STRIDE %-guard as a structural
  placeholder for future per-pixel-stride > 1 optimization. The
  guard is always-true today; intent is preserved for forward-compat.
- E12: restructured decode_pd_line_pair's rustdoc. Clean rustdoc
  keeps only the \"what this function does\" framing; issue
  archaeology (#NN refs) moves to a // HISTORY: block immediately
  below the rustdoc.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(Adjust the commit message body based on what Step 1's grep found — drop the E5 line if the comment was already gone, drop the E9 line if `PIXEL_FFT_STRIDE` was already removed.)

---

## Task 4: E9 + E13 — Inline cleanups across snr.rs + mode_robot.rs + vis.rs + sync.rs

Seven point-fixes across four files. Each is small (1-5 lines edited per site).

**Files:**
- Modify: `src/snr.rs`, `src/mode_robot.rs`, `src/vis.rs`, `src/sync.rs`

- [ ] **Step 1: `src/snr.rs` — E9 site 1 (module doc "256 samples = slowrx C" framing)**

Locate the module doc near the top of `src/snr.rs`. The audit references lines 22-25 — the "Working-rate scaling" section talks about slowrx's 1024 @ 44.1k vs ours at 1024 @ 11.025k. The "256 samples" framing is stale; it refers to what slowrx's window length would be at our sample rate (256 = 1024 × 11025/44100), not what we ship.

Find a line near the start of the working-rate-scaling discussion that mentions "256 samples" in the context of "slowrx C equivalent at our rate." If present, replace with a clearer note. Example replacement (adjust to surrounding context):

```rust
//! Note: a naive "use slowrx's window length at our sample rate"
//! would give 256 samples (~23 ms = slowrx C's time-domain support
//! at 44100 Hz). We instead keep `FFT_LEN = 1024` for a cleaner SNR
//! estimate, with the deliberate 4× time-domain tradeoff documented
//! in `docs/intentional-deviations.md::"FFT frequency resolution
//! exceeds slowrx C by 4×"`.
```

If the current text already handles this clearly (without the misleading "256 samples = slowrx C" framing), mark this sub-finding as "verified clear in current code" and skip the edit.

- [ ] **Step 2: `src/snr.rs` — E9 site 2 ("zero-pad" comment at ~line 257)**

The audit references `snr.rs:257` with "zero-pad" — claims there's no pad step in the SNR FFT path. Verify:

```bash
grep -nE "zero.?pad|zero-padded" /data/source/slowrx.rs/src/snr.rs
```

For each `zero-pad` or `zero pad` mention in `src/snr.rs`, read 5 lines of context to determine whether it's:
- (a) A legitimate reference to zero-padding (e.g., in the documentation of the per-pixel FFT where the Hann window is shorter than FFT_LEN — there IS zero-padding there).
- (b) A stale reference to a nonexistent step in the SNR estimator path.

For case (b): delete the comment. For case (a): leave alone.

The implementer notes in the commit message which (if any) were deleted.

- [ ] **Step 3: `src/snr.rs` — E9 site 3 (hysteresis "baseline" comment at ~line 155-160)**

The audit references the hysteresis comment claiming it "converges to baseline." Actually converges to *within one band* of baseline. Locate:

```bash
grep -nE "baseline|hysteresis.*converges|converges to.*selection" /data/source/slowrx.rs/src/snr.rs
```

Find the comment block about hysteresis convergence behavior (likely in the rustdoc of `window_idx_for_snr_with_hysteresis` or a nearby comment). The current text overstates the convergence guarantee.

Rewrite to: "Once SNR settles, the hysteresis selector locks within one Hann-window band of the SNR-optimal selection; baseline is approached but not exactly reached." (Adjust phrasing to match the function's actual semantics — the implementer reads the function body to confirm the "within one band" claim.)

- [ ] **Step 4: `src/mode_robot.rs` — E13 site 1 (module doc "2-channel layout")**

The audit references lines 1-15 of the module doc. Currently describes R24/R36 as "2-channel layout per radio line." This is the time-allocation view; the image-row view is that R24/R36 populate TWO image rows per radio line via chroma duplication. Add a paragraph after the existing description:

```rust
//! **Time vs image-row layout:** R36/R24 are 2-channel per *radio line*
//! in time-allocation terms (Y at 2× pixel-time, then alternating
//! Cr/Cb), but each radio line populates TWO image rows because the
//! chroma sample is duplicated to the neighbor image row (`video.c:421-425`).
//! R72 is 3-channel per radio line, one image row per radio line, no
//! chroma duplication.
```

Insert this paragraph immediately after the existing "Robot 24 / Robot 36: 2-channel layout per radio line — Y …" sentence (preserving the surrounding doc structure).

- [ ] **Step 5: `src/mode_robot.rs` — E9 site (chan_start_chroma explanation at ~line 178-206)**

The audit references the `chan_start_chroma` site. slowrx C carries 3 `ChanStart[]` entries (Y, Cr, Cb); Rust collapses Cr and Cb into a single `chan_start_chroma` because for R36/R24 the per-line parity determines which channel actually lives there. Locate:

```bash
grep -nB2 -A4 "chan_start_chroma" /data/source/slowrx.rs/src/mode_robot.rs
```

Find the site where `chan_start_chroma` is computed (likely a local binding `let chan_start_chroma = ...`). Add an inline comment immediately above:

```rust
// slowrx C carries 3 ChanStart[] entries (Y, Cr, Cb at distinct offsets).
// Rust collapses Cr and Cb into a single `chan_start_chroma` because for
// R36/R24 the per-line parity determines which channel actually lives in
// that time slot (the per-line decode reads `chan_start_chroma`, knowing
// from `line % 2` whether it's Cr or Cb). For R72 all three offsets are
// used distinctly — the per-mode dispatch handles that path separately.
// (Audit #94 E9.)
```

- [ ] **Step 6: `src/vis.rs` — E13 site (HedrBuf UB-fix at ~line 212-215)**

The audit references the `HedrBuf[-1]` UB-fix comment. Flag explicitly as a deliberate fidelity improvement. Locate:

```bash
grep -nB2 -A4 "HedrBuf\|HedrPtr.*-.*1" /data/source/slowrx.rs/src/vis.rs
```

Find the site where the `HedrPtr == 1` edge case is handled (the audit references it around line 212-215; post-#89 the lines may have shifted). Read the existing comment and replace with:

```rust
// slowrx C falls back to `HedrBuf[(HedrPtr - 1) % 45]` here, which
// wraps to `HedrBuf[44]` when only the first sample exists — reading
// garbage from a slot last touched by a prior detection (or zero-init
// on first use). We explicitly handle the `HedrPtr == 1` case to
// avoid the read-garbage UB. **Deliberate fidelity improvement**;
// see `docs/intentional-deviations.md::"Fidelity improvements over
// slowrx" → "HedrBuf[-1] wraparound read in VIS detector"`.
```

The `intentional-deviations.md` cross-reference points at the new section added in T6. If T6 isn't done yet, that's fine — the reference is forward-looking and the section will exist in the final PR.

- [ ] **Step 7: `src/sync.rs` — E13 site 1 (1200 Hz bin comment at ~line 103-105)**

Locate:

```bash
grep -nB2 -A4 "1200 Hz.*bin.*27\|sync_target_bin.*27" /data/source/slowrx.rs/src/sync.rs
```

The current comment says "1200 Hz bin is 27 (slowrx-correct) not 28 (what `.round()` would give)." True at zero hedr_shift; the actual computed bin shifts with hedr_shift. Update:

```rust
// sync_target_bin for 1200 Hz is 27 (slowrx-correct) not 28
// (what `.round()` would give) — at zero `hedr_shift_hz`. The
// actual computed bin is `get_bin(1200.0 + hedr_shift_hz, ...)`,
// so a non-zero radio mistuning shifts the target bin accordingly.
// (Audit #94 E13.)
```

- [ ] **Step 8: `src/sync.rs` — E13 site 2 (Praw off-by-one at ~line 160)**

Locate:

```bash
grep -nB2 -A4 "Praw /= (hi - lo)\|Praw /= (hi-lo)\|p_raw /=" /data/source/slowrx.rs/src/sync.rs
```

Find the `Praw` / `p_raw` divisor site. Add a slowrx-faithful note:

```rust
// slowrx-faithful off-by-one: slowrx C uses `hi - lo` as the
// divisor for an inclusive `[lo, hi]` range, undercounting by 1.
// We match for bit-parity of the `p_sync > 2 × p_raw` decision.
// See `docs/intentional-deviations.md::"Faithful-to-slowrx
// artifacts"`.
// (Audit #94 E13.)
```

- [ ] **Step 9: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

- [ ] **Step 10: Commit**

```bash
git add src/snr.rs src/mode_robot.rs src/vis.rs src/sync.rs
git commit -m "docs: E9 + E13 — inline cleanups across snr/robot/vis/sync (#94)

E9:
- snr.rs: clarify the \"256 samples = slowrx C\" framing in the
  working-rate scaling discussion (we ship 1024, not 256).
- snr.rs: drop the stale \"zero-pad\" comment in the SNR FFT path.
- snr.rs: rewrite the hysteresis-convergence comment from
  \"converges to baseline\" to \"within one band of baseline.\"
- mode_robot.rs: explain the 3-entry ChanStart[] → single
  chan_start_chroma collapse for R36/R24.

E13:
- vis.rs: flag the HedrBuf[-1] UB-fix as a deliberate fidelity
  improvement, cross-reference intentional-deviations.md.
- sync.rs: clarify the 1200 Hz bin comment is for zero hedr_shift
  only.
- sync.rs: comment the Praw /= (hi - lo) off-by-one as
  slowrx-faithful, cross-reference intentional-deviations.md.
- mode_robot.rs: clarify the time vs image-row layout for R36/R24.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(Adjust the commit body if any sub-findings were verified absent — e.g., "zero-pad" comment may not exist in current code.)

---

## Task 5: E10 — slowrx C line-ref alignment + per-file disclaimer

The big mechanical task. Verify every `// slowrx <file>.c:NNN` line reference against the gitignored local reference clone in `original/slowrx/` (see `clone-slowrx.sh`), fix any drift, add a per-file disclaimer to each affected file's module doc.

**Files:**
- Modify: `src/decoder.rs` (disclaimer already added in T1 Step 1)
- Modify: `src/vis.rs`, `src/sync.rs`, `src/mode_pd.rs`, `src/mode_robot.rs`, `src/mode_scottie.rs`, `src/demod.rs`, `src/snr.rs`

- [ ] **Step 1: Inventory all slowrx C line references**

Run:

```bash
grep -rnE "slowrx.*\.c:\d+" /data/source/slowrx.rs/src/ | grep -v "^/data/source/slowrx.rs/src/bin/" > /tmp/slowrx-refs-before.txt
wc -l /tmp/slowrx-refs-before.txt
cat /tmp/slowrx-refs-before.txt
```

Expected: 30-50 references across 8 files. Save this inventory for the commit message.

- [ ] **Step 2: Verify each reference against `original/slowrx/`**

For each reference, open the cited `original/slowrx/<file>.c` and read the line range to confirm it still describes the cited construct (e.g., a `// slowrx video.c:140-142` ref should still point at the relevant lines in `original/slowrx/video.c`).

**Workflow:**

```bash
# Example for a single reference:
# In src/decoder.rs:42 — `// slowrx video.c:140-142 computes pixel time as...`
sed -n '138,145p' /data/source/slowrx.rs/original/slowrx/video.c
# Verify the snippet at video.c:140-142 actually matches the cited construct.
```

For each ref:
- If line numbers match the cited construct: no edit needed.
- If line numbers are off by N (the construct is at video.c:142-144 not video.c:140-142): update the ref in the Rust source.
- If the cited construct no longer exists in the local reference clone (slowrx upstream rewrote that function): note in the commit message; either remove the line ref entirely or update it to the new location of the equivalent logic.

Log every fix in a working file:

```
src/decoder.rs:42 | video.c:140-142 → video.c:142-144 (off-by-2)
src/sync.rs:117  | sync.c:117 (verified, no drift)
...
```

- [ ] **Step 3: Apply the line-ref fixes**

For each ref that needs an update, edit the corresponding `.rs` file to use the corrected line number/range.

**Important:** do NOT change the surrounding context of the comment, just the line number. Preserve any function/symbol names mentioned in the comment.

- [ ] **Step 4: Add per-file disclaimer to each affected file**

For each `.rs` file that has at least one slowrx C line reference, ensure the file's module doc ends with the standard disclaimer line:

```rust
//! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
//! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
//! verified at audit #94 (2026-05-15).
```

`src/decoder.rs` already has this disclaimer (added in T1 Step 1's E3 rewrite). For the other 7 files, append the disclaimer as a new line at the end of the existing module doc.

If a file's module doc is short and the disclaimer would be the only "metadata" line, place it as the last `//!` line, separated by a blank `//!` line from the previous content.

- [ ] **Step 5: Inventory after fixes (sanity check)**

```bash
grep -rnE "slowrx.*\.c:\d+" /data/source/slowrx.rs/src/ | grep -v "^/data/source/slowrx.rs/src/bin/" > /tmp/slowrx-refs-after.txt
diff /tmp/slowrx-refs-before.txt /tmp/slowrx-refs-after.txt
```

The diff shows the line-number changes. Should be a small set of edits (only the drifted refs change).

```bash
# Confirm the disclaimer landed in all 8 files:
grep -lE "Inline.*slowrx.*verified at audit #94" /data/source/slowrx.rs/src/*.rs
```

Should list `decoder.rs`, `vis.rs`, `sync.rs`, `mode_pd.rs`, `mode_robot.rs`, `mode_scottie.rs`, `demod.rs`, `snr.rs` (8 files).

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

- [ ] **Step 7: Commit**

```bash
git add src/
git commit -m "docs: E10 — slowrx C line-ref alignment + per-file disclaimers (#94)

Audited all inline \`// slowrx <file>.c:NNN\` references across 8 files
against the gitignored local reference clone in \`original/slowrx/\`.
Fixed N drifted refs (off-by-1 to off-by-3 in most cases); added a
per-file disclaimer to each module doc anchoring the refs to that
local clone and verifying at this audit pass.

[Implementer: list the specific files + line-number changes here, from
the working file built in Step 2.]

The disclaimer template:

    //! Inline \`// slowrx <file>.c:NNN\` line refs are against the
    //! gitignored local reference clone in \`original/slowrx/\` (see
    //! \`clone-slowrx.sh\`); verified at audit #94 (2026-05-15).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(The implementer replaces the `[Implementer: ...]` block with the actual list of corrections from Step 2's working file.)

---

## Task 6: E8 — `intentional-deviations.md` additions

Two new top-level sections appended to `docs/intentional-deviations.md`.

**Files:**
- Modify: `docs/intentional-deviations.md`

- [ ] **Step 1: Read the current `intentional-deviations.md` structure**

```bash
grep -nE "^#|^##" /data/source/slowrx.rs/docs/intentional-deviations.md
```

The current structure (post-#88) has 4 entries under no top-level grouping. The audit asks to add two new top-level sections; the existing entries stay where they are (separated by `---` horizontal rules per the established pattern).

- [ ] **Step 2: Append the two new sections at the end of the file**

After the last existing entry's closing horizontal rule, append:

```markdown
---

# Faithful-to-slowrx artifacts (deliberate bit-parity)

These behaviors are technically suboptimal in slowrx C but Rust
reproduces them for bit-exact decode parity with reference output.
Future audits should NOT "fix" these without an intentional
parity-break.

## `Praw /= (hi - lo).max(1)` off-by-one in `SyncTracker::has_sync_at`

**Files:** `src/sync.rs::SyncTracker::has_sync_at` ↔ slowrx `video.c:282-288`.
**Tracking issue:** (none — slowrx-faithful).

### What slowrx does
slowrx divides the video-band power sum by `(hi - lo)` for an
inclusive `[lo, hi]` range — undercounting by 1.

### What we do
We match exactly (`Praw /= (hi - lo).max(1)`).

### Why
Bit-parity of the `p_sync > 2 × p_raw` sync-pulse decision boundary.

### When to revisit
If a non-parity decision is taken to optimize sync detection
independently of slowrx C reference output.

---

## Row-0 `Cb` zero-init for R36/R24

**Files:** `src/mode_robot.rs::decode_line`, `src/decoder.rs::SstvEvent::LineDecoded` docstring ↔ slowrx `video.c::GetVideo` `calloc`-then-write pattern (lines 421-425).
**Tracking issue:** (none — slowrx-faithful).

### What slowrx does
slowrx allocates the image buffer via `calloc` (zero-init) and never
writes row 0's `Cb` channel — the chroma-duplication writes are
prev-row → current-row, so row 0's `Cb` slot has no source. The
result is a transient color cast on the top row of every R36/R24
decode.

### What we do
We reproduce the artifact: emit row 0 with `Cb = 0` after
`LineDecoded(0)`.

### Why
Slowrx-faithful bit-parity on the R36/R24 top row. The artifact is
visible on the ARISS Fram2 capture set and matches slowrx's output
exactly.

### When to revisit
If a "fix row 0 to use row 1's Cb" cosmetic improvement is
acceptable as an intentional deviation.

---

## `xAcc[8]` window-bound truncation in `find_falling_edge`

**Files:** `src/sync.rs::falling_edge_from_x_acc` ↔ slowrx `sync.c:108`.
**Tracking issue:** [audit A6, closed in #88](https://github.com/jasonherald/slowrx.rs/pull/106).

### What slowrx does
slowrx's falling-edge convolution loop iterates `n` from 0 to
`X_ACC_BINS - 8 = 692` exclusive, missing position `n=692` — a
faithful off-by-one in the loop bound.

### What we do
We match slowrx via `.take(X_ACC_BINS - 8)` (closed in #88's A6 fix —
audit caught Rust's native `.windows(8)` was giving 693 windows
where slowrx C gives 692).

### Why
Bit-parity on the falling-edge `xmax` position.

### When to revisit
Non-parity improvements over slowrx are out of scope without an
explicit deviation entry.

---

# Fidelity improvements over slowrx (UB / div-by-zero / clamp fixes)

These are deliberate fixes for actual bugs in slowrx C — undefined
behavior, divide-by-zero, missing clamps — that Rust avoids. The
deviation is intentional; reverting would re-introduce the slowrx
C bug.

## `HedrBuf[-1]` wraparound read in VIS detector

**Files:** `src/vis.rs` (early-`HedrPtr` guard) ↔ slowrx `vis.c:67`.
**Tracking issue:** (none — slowrx UB, no upstream issue).

### What slowrx does
slowrx's VIS detector reads `HedrBuf[(HedrPtr - 1) % 45]` when
only the first sample has been written — this wraps to
`HedrBuf[44]`, which was last touched on the previous detection (or
is `calloc`-zero on first use). The read returns garbage.

### What we do
We explicitly handle the `HedrPtr == 1` case and return early
without reading the would-be-`-1` slot.

### Why
Avoid the read-garbage UB. The behavior in slowrx isn't reproducible
across runs (depends on previous detection state), so bit-parity
isn't a goal here.

### When to revisit
If the read-garbage value ever proves to be load-bearing in a real
slowrx reference output (extremely unlikely — slowrx's own retry
loop should swamp any single-detection garbage).

---

## Gaussian-interpolation div-by-zero guard

**Files:** `src/demod.rs::pixel_freq` peak-interpolation site ↔ slowrx `video.c::GetVideo` peak refinement.
**Tracking issue:** (none — slowrx NaN, no upstream issue).

### What slowrx does
slowrx's peak-bin refinement uses the parabolic interpolation
formula `(prev - next) / (2 × (prev - 2 × peak + next))`. When the
three-bin centroid is flat (silence / DC bin), the denominator is
zero and the result is NaN.

### What we do
We check the denominator and return the integer bin (no refinement)
when it's near-zero.

### Why
Avoid NaN propagating through the per-pixel frequency-to-luminance
mapping (which would clamp to black via `f64::is_nan` later, but
the propagation path is fragile).

### When to revisit
If profiling shows the guard is a hot-path overhead (it's a single
fp compare; unlikely).

---

## `-20 dB` SNR return paths

**Files:** `src/snr.rs::SnrEstimator::estimate` ↔ slowrx `video.c:302-343`.
**Tracking issue:** (none — slowrx-faithful behavior with a sensible-fallback improvement).

### What slowrx does
slowrx returns SNR = 0 when the FFT bin sums underflow or the band
integration is empty (no signal in the video band).

### What we do
We return `-20 dB` — slowrx's minimum meaningful value — so the
hysteresis selector lands at the longest Hann window
(`window_idx_for_snr(-20) == 6`). A smoother fallback than slowrx's
"treat zero-SNR as 0 dB high-confidence" behavior, which would lock
the shortest window on a silent input.

### Why
On real-radio captures with intermittent dropouts (lost samples,
multipath fades), slowrx's behavior caused brief glitches where the
SNR-0 fallback would pick the shortest Hann (= sharpest time
resolution, but no noise rejection) and the next pixel decode would
be wildly off. `-20 dB` → longest Hann gives the cleanest fallback.

### When to revisit
If a slowrx reference WAV is decoded with reference output and the
`-20` vs `0` choice produces a per-pixel difference. Currently
verified equivalent in the ARISS Dec-2017 + Fram2 sets.
```

The implementer verifies the exact line numbers / behavior claims in each entry against the actual code at edit-time. If any sub-claim is wrong (e.g., the "Gaussian-interpolation div-by-zero guard" doesn't actually exist in `pixel_freq` because slowrx's formula is different), the implementer adjusts the entry text or marks the sub-finding for follow-up.

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

The `intentional-deviations.md` is plain markdown (not rustdoc), so `cargo doc` won't validate it. The gate just checks that the file still parses as part of the workspace (which it does — it's not in any compile path).

- [ ] **Step 4: Commit**

```bash
git add docs/intentional-deviations.md
git commit -m "docs(deviations): E8 — Faithful-to-slowrx + Fidelity-improvements sections (#94)

Adds two new top-level sections to docs/intentional-deviations.md:

# Faithful-to-slowrx artifacts (deliberate bit-parity)
  - Praw /= (hi - lo) off-by-one in SyncTracker::has_sync_at
  - Row-0 Cb zero-init for R36/R24
  - xAcc[8] window-bound truncation in find_falling_edge

# Fidelity improvements over slowrx (UB / div-by-zero / clamp fixes)
  - HedrBuf[-1] wraparound read in VIS detector
  - Gaussian-interpolation div-by-zero guard
  - -20 dB SNR return paths

These document things future audits should NOT \"correct\" Rust back
toward broken C — the bit-parity behaviors are deliberate, and the
UB/NaN/clamp fixes are deliberate improvements over slowrx C.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

Open `CHANGELOG.md`. Under `## [Unreleased]` `### Internal`, prepend a new bullet:

```markdown
### Internal

- **Docs sweep** — ten audit findings, all docs-only (audit bundle 10
  of 12). `decoder.rs` module doc rewritten as the actual two-pass
  pipeline (E3); version-pinned prose generalized in `modespec.rs`
  and `lib.rs` (E4); `get_bin` rustdoc gains a bin-scaling caveat
  noting that the example table is for the 256/11025 sync FFT, not
  the 1024/11025 SNR + per-pixel FFTs (E6); contradictory or stale
  inline comments fixed in `mode_pd.rs` (E5), `snr.rs` (3 sites),
  `mode_robot.rs` (2 sites), `vis.rs`, and `sync.rs` (2 sites)
  (E9 + E13); slowrx C line refs aligned against the gitignored
  local reference clone in `original/slowrx/`, with per-file disclaimers
  anchoring the refs (E10); `decode_pd_line_pair` issue archaeology moved from
  inline `#NN` references in the rustdoc to a `// HISTORY:` block
  below the rustdoc (E12); two new sections in
  `docs/intentional-deviations.md` — "Faithful-to-slowrx artifacts"
  (deliberate bit-parity behaviors) and "Fidelity improvements over
  slowrx" (UB / div-by-zero / clamp fixes) — protect future audits
  from "correcting" Rust back toward broken C (E8); `mode_scottie.rs`
  gains a `// NOTE:` block recording the deferred
  `mode_scottie → mode_rgb_sequential` rename (B15). Non-breaking;
  no code changes; lib test count unchanged at 136. (#94; audit
  E3/E4/E5/E6/E8/E9/E10/E12/E13/B15.)

- **Performance: hoist per-channel/per-line allocations into reusable scratch**
  — ... [existing #93 bullet stays as-is below]
```

- [ ] **Step 2: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected test counts:
- lib: **136** (unchanged from baseline).
- `tests/roundtrip.rs`: 11/11 (unchanged).
- All other integration tests: unchanged.
- Doc clean.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the docs sweep (#94)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - E3 (decoder.rs module doc) → T1 Step 1
  - E4 (3 version-pinned prose sites) → T1 Steps 3-5
  - E5 (chan_bounds_abs contradictory comment) → T3 Step 1
  - E6 (get_bin bin-scaling caveat) → T2
  - E8 (intentional-deviations.md additions) → T6
  - E9 (5 imprecisions) → T3 Step 2 (PIXEL_FFT_STRIDE), T4 Steps 1-3 (snr.rs ×3), T4 Step 5 (mode_robot.rs chan_start_chroma)
  - E10 (line-ref alignment + disclaimer) → T5
  - E12 (decode_pd_line_pair HISTORY block) → T3 Step 3
  - E13 (4 comment-vs-code mismatches) → T4 Step 6 (vis.rs), T4 Step 7 (sync.rs 1200 Hz), T4 Step 8 (sync.rs Praw), T4 Step 4 (mode_robot.rs module doc)
  - B15 (rename NOTE) → T1 Step 6
  - CHANGELOG → T7

- **Behavior preservation:** zero code changes. Lib test count stays at 136 throughout. The CI gate's `cargo test` step is a no-op proof (tests pass because no behavior changed); the load-bearing checks are `cargo clippy -D warnings` (for any new `must_use` or other lints triggered by added attributes) and `RUSTDOCFLAGS="-D warnings" cargo doc` (for malformed markdown or broken intra-doc links in the new doc text).

- **TDD red moments:** none — this is a docs PR, no tests change.

- **Out of scope:**
  - Renaming `mode_scottie.rs` to `mode_rgb_sequential.rs` (B15 records the deferral; the rename itself is future work).
  - Updating the audit doc itself (`docs/audits/2026-05-11-deep-code-review-audit.md`) — that's a historical snapshot and stays as-is.
  - Adding new tests (this is docs-only).
  - Any code-level cleanup that touched the same lines as a comment change — keep diffs minimal.

- **CodeRabbit risk:** docs PRs often draw CodeRabbit MD040 (fenced-code-block language) and similar markdown-style nits, especially in the new `intentional-deviations.md` sections. The implementer pre-emptively adds language tags to fenced code blocks where applicable (`rust`, `text`, etc.).
