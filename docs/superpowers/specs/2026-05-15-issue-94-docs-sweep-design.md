# Issue #94 — Docs sweep — Design

**Issue:** [#94](https://github.com/jasonherald/slowrx.rs/issues/94) (audit bundle 10 of 12 — IDs E3, E4, E5, E6, E8, E9, E10, E12, E13, B15).

**Scope:** pure documentation cleanup across 10 source files + `docs/intentional-deviations.md`. No code changes; no API surface changes; no test changes. Catches up stale module docs (E3, E4), fixes contradictory or imprecise inline comments (E5, E9, E13), corrects a misleading rustdoc example table (E6), aligns slowrx C line references against the gitignored local reference clone in `original/slowrx/` (E10), restructures issue-archaeology in one rustdoc (E12), adds two new sections to `intentional-deviations.md` separating "faithful-to-slowrx artifacts" from "fidelity improvements over slowrx" (E8), and records the deferred `mode_scottie → mode_rgb_sequential` rename (B15).

This should be the simplest PR in the epic. The CI gate's `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` step is the load-bearing check (catches broken intra-doc links + malformed markdown in doc-comments).

---

## Background — audit findings

(All references below are from the audit doc; the implementer verifies against current post-#85/#93 source via grep before editing.)

- **E3 — `src/decoder.rs:1-8` module doc** still calls this "the V1 skeleton… VIS detection lands in PR-1; pixel decoding lands in PR-2." Pre-merge framing from the V1 build-out era. Replace with the actual two-pass pipeline.
- **E4 — version-pinned prose** in three sites: `src/modespec.rs:6-10` ("Implemented as of V2.4 (0.5.0)"), `src/lib.rs:8-19` `## Status` (`0.5.x — V2.4 published…`), `src/lib.rs:154-158` `__test_support` ("until V1 publishes"). Generalize so patch bumps don't churn these.
- **E5 — `decode_pd_line_pair`** has an inner comment about `chan_bounds_abs` that contradicts the `#32-lifted` note above it. The `chan_bounds_abs` parameter was removed in #85 (B5); the inner comment is dead. Verify + delete.
- **E6 — `get_bin` rustdoc example table** in `src/dsp.rs` (post-#85 location) shows bin values for the 256/11025 sync FFT (and the equivalent 1024/44100 slowrx-native case — same Hz/bin ratio). The table is misleading for the 1024/11025 SNR + per-pixel FFTs (since 0.3.3) where bins are 4× larger (e.g., 1200 Hz → bin 111, not 27). Add a scaling caveat.
- **E8 — `docs/intentional-deviations.md`** currently mixes "we deliberately deviate" entries with no explicit separation between "things we match slowrx on for parity even when buggy" and "things we deliberately improved." Add two new top-level sections: "Faithful-to-slowrx artifacts" (matched-buggy) and "Fidelity improvements over slowrx" (deliberate fixes). Protects future audits from "correcting" Rust back toward broken C.
- **E9 — five module-doc/comment imprecisions:**
  - `src/snr.rs:22-25` ("256 samples = slowrx C" framing is stale — both slowrx and ours use FFT_LEN=1024).
  - `src/snr.rs:257` ("zero-pad" — no pad step in the SNR FFT path).
  - `src/snr.rs:155-160` (hysteresis comment says baseline; actually converges to within-one-band of baseline).
  - `src/mode_robot.rs:178-206` (`chan_start_chroma` site — explain the slowrx 3-entry `ChanStart` collapse to a single Rust local).
  - `src/mode_pd.rs:228-237` (`PIXEL_FFT_STRIDE` is fixed at 1 — the `%`-guard is vestigial; explain or remove).
- **E10 — slowrx C line refs drift.** Inline `// slowrx <file>.c:NNN` comments throughout the codebase are off by 1-3 lines vs the gitignored local reference clone in `original/slowrx/`. Approach: **one alignment pass + per-file disclaimer.** Audit every ref via grep, verify against `original/slowrx/<file>.c`, fix any drift, add a one-line disclaimer at the top of each affected file's module doc anchoring the refs to that local clone.
- **E12 — `decode_pd_line_pair`'s rustdoc** has bare `#NN` GitHub issue refs (`#32`, `#34`, `#40`, `#42`). They don't render as links and clutter the function's user-facing doc. **Move the issue archaeology to a `// HISTORY:` block** immediately below the rustdoc — clean rustdoc keeps the "what this function does" framing; HISTORY block carries the per-issue notes for future archaeologists.
- **E13 — four comment-vs-code mismatches:**
  - `src/vis.rs:212-215` (`HedrBuf[-1]` UB-fix isn't flagged as a deliberate improvement).
  - `src/sync.rs:160` (`Praw /= (hi-lo).max(1)` replicates slowrx's own off-by-one — not commented).
  - `src/sync.rs:103-105` ("1200 Hz bin is 27" — only true at `hedr_shift==0`).
  - `src/mode_robot.rs:1-15` ("2-channel layout" — needs clarification of the time-layout reuse for R36/R24 vs R72).
- **B15 — `mode_scottie.rs → mode_rgb_sequential.rs` rename deferral.** The file handles both Scottie family (`SyncPosition::Scottie`) and Martin family (`SyncPosition::LineStart`) via the shared `ChannelLayout::RgbSequential` decode path. A rename is tracked but deferred. Record durably with a `// NOTE:` block at the top of the file.

---

## Architecture

### Section A — Module doc updates + line-ref alignment

**E3 — `src/decoder.rs:1-8` module doc.** Replace with the actual two-pass pipeline framing:

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

**E4 — generalize version-pinned prose** in 3 sites:

1. `src/modespec.rs:6-10` — drop "Implemented as of V2.4 (0.5.0):" prefix. Rewrite:
   ```rust
   //! Implemented modes: PD120, PD180, PD240, Robot 24, Robot 36, Robot 72,
   //! Scottie 1, Scottie 2, Scottie DX, Martin 1, Martin 2. All RGB-sequential
   //! modes (Scottie + Martin) share a single decode path; the per-line
   //! offsets branch on [`SyncPosition`].
   ```

2. `src/lib.rs:8-19` `## Status` paragraph — current text version-pins on `0.5.x — V2.4 published`. Rewrite to describe coverage + validation status WITHOUT version anchors:
   ```rust
   //! ## Status
   //!
   //! PD120 / PD180 / PD240, Robot 24 / 36 / 72, Scottie 1 / 2 / DX, and
   //! Martin 1 / 2 decode from raw audio. PD120 and PD180 are validated
   //! against the ARISS Dec-2017 capture set; Robot 36 is validated against
   //! the ARISS Fram2 corpus (see `tests/ariss_fram2_validation.md`).
   //! Scottie and Martin families are synthetic round-trip-validated only
   //! — no Scottie or Martin reference WAVs are available. The public API is
   //! `#[non_exhaustive]`-protected for additive growth. See
   //! <https://github.com/jasonherald/slowrx.rs/issues/9> for the V2 roadmap.
   ```

3. `src/lib.rs:154-158` `__test_support` doc — replace "until V1 publishes" with the actual protection mechanism:
   ```rust
   /// Test-support — exposed under the `test-support` feature for integration
   /// tests in this crate (e.g., `tests/roundtrip.rs`). NOT part of the
   /// stable public API; the module is `#[doc(hidden)]` and the items
   /// inside are thin wrappers around `pub(crate)` internals — the API is
   /// `#[non_exhaustive]`-protected for additive growth.
   ```

**E10 — slowrx C line-ref alignment + per-file disclaimer.** Two-part:

1. **Alignment sweep.** For each file with `// slowrx ... \.c:\d+` comments:
   ```bash
   grep -nE "//.*slowrx.*\.c:\d+" /data/source/slowrx.rs/src/<file>.rs
   ```
   For each match, open the referenced `original/slowrx/<file>.c` and verify the line range still describes the cited construct. Fix any off-by-N drift. The implementer logs each correction in the commit body (`<file>.rs:NN-MM: video.c:142 → video.c:145`).

2. **Per-file disclaimer.** Add a one-line note to each affected file's module doc (the disclaimer is already in E3 above for `decoder.rs`; replicate the same line to other files). Standard form:
   ```rust
   //! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
   //! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
   //! verified at audit #94 (2026-05-15).
   ```
   
   Files getting the disclaimer (based on which carry slowrx C line refs): `decoder.rs`, `vis.rs`, `sync.rs`, `mode_pd.rs`, `mode_robot.rs`, `mode_scottie.rs`, `demod.rs`, `snr.rs`. The implementer greps to confirm coverage.

**B15 — `mode_scottie.rs` rename-deferral `// NOTE:`.** Append after the existing module doc:

```rust
// NOTE (audit #94 B15): the file is named `mode_scottie.rs` but its
// `decode_line` is shared by Martin 1 / Martin 2 (both
// `ChannelLayout::RgbSequential` per ModeSpec). A rename to
// `mode_rgb_sequential.rs` is tracked but deferred — the existing name
// is grep-able and the per-mode behavior branches on
// `spec.sync_position` (mid-line for Scottie, line-start for Martin)
// rather than file boundaries.
```

### Section B — Inline doc cleanups (E5 + E6 + E9 + E12 + E13)

Eleven point-fixes. Each is small (1-5 lines edited).

**`src/dsp.rs::get_bin` (E6).** Existing rustdoc has a "Numerical verification" table prefixed with "both at slowrx-native 1024/44100 and our 256/11025 — same Hz/bin ratio, so bins are identical." This is true but misleading: the SNR + per-pixel FFTs run at 1024/11025 (since 0.3.3) where bins are 4× larger. Append a scaling caveat block after the existing table:

```rust
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

**`src/snr.rs` (E9, 3 sites):**

1. Module doc lines ~22-25 ("256 samples = slowrx C" framing): rewrite to clarify that slowrx C and ours both use FFT_LEN=1024, but ours runs at 11025 Hz vs slowrx's 44100 Hz, yielding a 4× longer time-domain window (~93 ms vs ~23 ms). The current text already gets this right in the next paragraph; the issue is the "256 samples" mention specifically — that's the slowrx-C-at-our-sample-rate equivalent, not what we ship. Replace with: "Note: a naive 'use slowrx's window length at our sample rate' would give 256 samples (~23 ms); we instead keep FFT_LEN=1024 for a cleaner SNR estimate, with the deliberate 4× time-domain tradeoff documented in `intentional-deviations.md`."

2. Line ~257 ("zero-pad"): there's no zero-pad in the SNR estimator FFT path (the Hann window has same length as FFT_LEN). Delete the comment.

3. Lines ~155-160 (hysteresis comment): rewrite "converges to baseline" → "converges to within one Hann-window band of the SNR-optimal selection" (matches the implementation; the hysteresis prevents oscillation but doesn't pin to baseline exactly).

**`src/mode_robot.rs` (E9 + E13, 2 sites):**

1. Lines 1-15 (module doc): currently describes R24/R36 as "2-channel layout per radio line." This is accurate for time-allocation but obscures that R24/R36 also do cross-row chroma duplication (each radio line writes to TWO image rows). Add: "**Time vs image-row layout:** R36/R24 are 2-channel per *radio line* in time-allocation terms (Y at 2× pixel-time, then alternating Cr/Cb), but each radio line populates TWO image rows because the chroma sample is duplicated to the neighbor image row (`video.c:421-425`). R72 is 3-channel per radio line, one image row per radio line, no chroma duplication."

2. Lines ~178-206 (`chan_start_chroma` site): add an inline comment explaining the 3-entry collapse: "slowrx C carries 3 `ChanStart[]` entries (Y, Cr, Cb); Rust collapses Cr and Cb into a single `chan_start_chroma` because for R36/R24 the per-line parity determines which channel actually lives in that time slot, and R72 uses all three at distinct offsets. (The 3-entry layout maps cleanly to Robot 72; for R36/R24 the second-and-third entries become a single time-slot decoded per row parity.)"

**`src/mode_pd.rs` (E5 + E9 + E12, 3 sites):**

1. Lines ~228-237 (`PIXEL_FFT_STRIDE` `%`-guard): the modulo always yields 0 because `PIXEL_FFT_STRIDE = 1`. Either:
   - **Recommended:** keep the guard defensive, add a clarifying comment: "`PIXEL_FFT_STRIDE` is fixed at 1 in V1, so this modulo always evaluates to 0 and the branch is always taken. Kept as a structural placeholder for a future per-pixel stride > 1 optimization that would interpolate or downsample within the channel."
   - Or delete the `if` guard entirely. Pick option 1 (defensive-keep) — the cost is one always-taken branch the compiler eliminates, and the structural intent is clearer.

2. Lines ~244-284 (`decode_pd_line_pair` rustdoc): move issue archaeology to a `// HISTORY:` block. The current rustdoc has scattered `#32`/`#34`/`#40`/`#42` references that don't render as links. Restructure:
   - **Clean rustdoc** keeps only the "what this function does" framing (pixel-time formula, channel sweep, ChanStart computation).
   - **`// HISTORY:` block** immediately below the rustdoc, before the function signature, carries the issue-archaeology with brief notes per issue:
     ```rust
     // HISTORY (audit #94 E12):
     //   #32 — chan_bounds_abs zero-pad parameter removed (B5 fold in #85);
     //         the `chan_bounds_abs` parameter was deleted with that PR.
     //   #34 — sample-counter inflation observation (closed; informational
     //         only, no fix needed).
     //   #40 — VisDetector::take_residual_buffer re-anchor contract;
     //         spent detectors must be replaced, not reused.
     //   #42 — find_sync 90° slant deadband (intentional deviation
     //         documented in `docs/intentional-deviations.md`).
     ```

3. Lines ~330-343 (`chan_bounds_abs` inner comment): the inline comment about "used to zero-pad outside the active channel" contradicts the `#32-lifted` note above. The `chan_bounds_abs` parameter was deleted in #85's B5 fold. **Verify the comment still exists** (it may have been deleted with the parameter; if so, E5 is already closed). If it exists, delete it.

**`src/vis.rs` line ~212-215 (E13):** Flag the `HedrBuf[-1]` UB-fix as a deliberate fidelity improvement. Replace the existing terse comment with:

```rust
// slowrx C falls back to `HedrBuf[(HedrPtr - 1) % 45]` here, which
// wraps to `HedrBuf[44]` when only the first sample exists — reading
// garbage from a slot last touched by a prior detection (or zero-init
// on first use). We explicitly handle the `HedrPtr == 1` case to
// avoid the read-garbage UB. **Deliberate fidelity improvement**;
// see `docs/intentional-deviations.md::"Fidelity improvements over
// slowrx" → "HedrBuf[-1] wraparound read in VIS detector"`.
```

**`src/sync.rs` (E13, 2 sites):**

1. Lines ~103-105 ("1200 Hz bin is 27"): clarify the hedr_shift assumption:
   ```rust
   // sync_target_bin for 1200 Hz is 27 (slowrx-correct) not 28
   // (what `.round()` would give) — at zero `hedr_shift_hz`. The
   // actual computed bin is `get_bin(1200.0 + hedr_shift_hz, ...)`,
   // so a non-zero radio mistuning shifts the target bin
   // accordingly.
   ```

2. Line ~160 (`Praw /= (hi - lo).max(1)`): add a slowrx-faithful off-by-one note:
   ```rust
   // slowrx-faithful off-by-one: slowrx C uses `hi - lo` as the
   // divisor for an inclusive `[lo, hi]` range, undercounting by 1.
   // We match for bit-parity of the `p_sync > 2 × p_raw` decision.
   // See `docs/intentional-deviations.md::"Faithful-to-slowrx
   // artifacts"`.
   ```

### Section C — `docs/intentional-deviations.md` additions (E8)

Append two new top-level sections to `docs/intentional-deviations.md`. Format matches the existing entry style (header + Files + What slowrx does + What we do + Why + When to revisit).

**Section: "Faithful-to-slowrx artifacts"** (matched-buggy):

```markdown
---

# Faithful-to-slowrx artifacts (deliberate bit-parity)

These behaviors are technically suboptimal in slowrx C but Rust
reproduces them for bit-exact decode parity with reference output.
Future audits should NOT "fix" these without an intentional
parity-break.

## `Praw /= (hi - lo).max(1)` off-by-one in `SyncTracker::has_sync_at`

**Files:** `src/sync.rs::SyncTracker::has_sync_at` ~line 160 ↔ slowrx `video.c:282-288`.
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

**Files:** `src/mode_robot.rs::decode_line`, `src/decoder.rs::SstvEvent::LineDecoded`
docstring ↔ slowrx `video.c::GetVideo` `calloc`-then-write pattern (lines 421-425).
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
**Tracking issue:** [audit A6, closed in #88](https://github.com/jasonherald/slowrx.rs/issues/88).

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
Same as above — non-parity improvements over slowrx are out of
scope without an explicit deviation entry.

---

# Fidelity improvements over slowrx (UB / div-by-zero / clamp fixes)

These are deliberate fixes for actual bugs in slowrx C — undefined
behavior, divide-by-zero, missing clamps — that Rust avoids. The
deviation is intentional; reverting would re-introduce the slowrx
C bug.

## `HedrBuf[-1]` wraparound read in VIS detector

**Files:** `src/vis.rs` ~line 212-215 ↔ slowrx `vis.c:67`.
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
be wildly off. `-20 dB` → longest Hann gives the cleanest
fallback.

### When to revisit
If a slowrx reference WAV is decoded with reference output and the
`-20` vs `0` choice produces a per-pixel difference. Currently
verified equivalent in the ARISS Dec-2017 + Fram2 sets.
```

The exact line numbers and verbatim slowrx behavior should be verified by the implementer at-edit-time — the audit gives the high-level shape; the implementer fills in any small corrections to match the actual code.

---

## Out of scope

- Code changes (this is a docs-only PR; all 10 audit findings are docs/comments).
- Renaming `mode_scottie.rs` to `mode_rgb_sequential.rs` (B15 records the deferral; the actual rename is a future PR if/when it's worth the churn).
- `lookup_vis` re-export rename or similar API tweaks (out of scope).

## Release implication

Pure docs PR — no behavior change, no API change. Non-breaking. CHANGELOG entry goes under `[Unreleased]` `### Internal`. Next release stays in 0.5.x patch line.

## Success criteria

- All 10 audit findings addressed (E3, E4, E5, E6, E8, E9, E10, E12, E13, B15).
- `cargo fmt --all -- --check` — clean (no formatting changes; only comment text).
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --all-features --locked --release` — **136 lib tests unchanged** (no test changes).
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` — **the load-bearing check.** Any malformed markdown or broken intra-doc link in the new doc-comments fails CI here.
- All slowrx C line refs verified against `original/slowrx/` (E10 alignment sweep).
- `docs/intentional-deviations.md` gains the two new sections (E8) in the documented format.
- `mode_scottie.rs` gains the `// NOTE:` rename-deferral block (B15).
- The commit message for each task itemizes which findings the task closed.
