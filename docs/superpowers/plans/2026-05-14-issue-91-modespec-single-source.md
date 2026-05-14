# Issue #91 — `ModeSpec` as single source of truth — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate scattered mode metadata behind a single `ALL_SPECS` table in `src/modespec.rs`; add `short_name` + `name` fields to `ModeSpec` so the CLI's hand-maintained `mode_tag` map (4× stale-bug history) becomes one line; normalize doc comments; add an F8 round-trip property test.

**Architecture:** Five sequential tasks. T1 lands the load-bearing structural change (two new fields on `ModeSpec`, populated in all 11 const blocks). T2 simplifies the CLI consumer (delete `mode_tag` + its lockstep test, replace the one call site). T3 introduces `ALL_SPECS`, derives `lookup` from it, and adds the F8 round-trip property test. T4 is pure doc work (E1 + E11). T5 closes with the CHANGELOG and final gate run. Each task preserves the green test suite.

**Tech Stack:** Rust 2021, MSRV 1.85. CI gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-14-issue-91-modespec-single-source-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs B7, B13, E1, E11, F8)

---

## File Structure

| File | Status | Role |
|------|--------|------|
| `src/modespec.rs` | modify | All structural changes — new fields, `ALL_SPECS`, `lookup` rewrite, `for_mode` doc, `SstvMode` variant docs, F8 test. |
| `src/bin/slowrx_cli.rs` | modify | Delete `mode_tag` fn + lockstep test; update one call site to `for_mode(m).short_name`. |
| `src/demod.rs` | modify | One-line cross-reference comment near the existing Scottie DX Hann-bump explanation. |
| `CHANGELOG.md` | modify | One bullet under `[Unreleased] ### Internal`. |

Task order: **T1** (add fields, populate consts) → **T2** (CLI consumer cleanup) → **T3** (`ALL_SPECS` + `lookup` refactor + F8 test) → **T4** (E1 + E11 doc work) → **T5** (CHANGELOG + final gate).

**Verification after each task:**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Lib test counts at each checkpoint:
- **Pre-#91 baseline:** 135 lib tests.
- **Post-T1:** 135 (additive; no new tests, no behavior change).
- **Post-T2:** 135 lib (the deleted `mode_tag_covers_all_known_variants` is in the `slowrx_cli` binary target, not the lib).
- **Post-T3:** 136 (+1 F8 `all_specs_roundtrip`).
- **Post-T4:** 136 (doc-only).
- **Post-T5:** 136.

---

## Task 1: Add `short_name` + `name` fields to `ModeSpec`; populate 11 const blocks

The load-bearing structural change. Two new public fields on `ModeSpec`; each of the 11 standalone `const`s gets two new lines. No new tests; no behavior change.

**Files:**
- Modify: `src/modespec.rs`

- [ ] **Step 1: Add the two fields to the `ModeSpec` struct**

In `src/modespec.rs`, find the `pub struct ModeSpec { ... }` block (around lines 45-78). Find the line `pub mode: SstvMode,` (the first field, line 50 in current source). Immediately after that line, insert:

```rust
    /// CLI/filename slug. Stable across releases (filenames like
    /// `img-NNN-{short_name}.png` depend on this). lowercase, no
    /// separators: "pd120", "robot24", "scottiedx", "scottie1",
    /// "martin1", etc. (audit #91 B13)
    pub short_name: &'static str,
    /// Human-readable mode name. For log lines and any future
    /// user-facing display. "PD-120", "Robot 24", "Scottie DX", etc.
    /// (audit #91 B13)
    pub name: &'static str,
```

The order of fields after this edit: `mode`, `short_name`, `name`, `vis_code`, `line_pixels`, `image_lines`, `line_seconds`, `sync_seconds`, `porch_seconds`, `pixel_seconds`, `septr_seconds`, `channel_layout`, `sync_position`.

- [ ] **Step 2: Populate `short_name` + `name` in the 11 const blocks**

Apply 11 edits (one per const). For each `const PD120/PD180/.../MARTIN2: ModeSpec = ModeSpec { mode: SstvMode::..., vis_code: 0x.., ... };`, insert the two new fields immediately after the `mode:` line.

Use this mapping table — both columns are non-empty, all 11 short_name values are unique, all match the strings the existing `mode_tag` fn returns:

| const | short_name | name |
|---|---|---|
| `PD120` | `"pd120"` | `"PD-120"` |
| `PD180` | `"pd180"` | `"PD-180"` |
| `PD240` | `"pd240"` | `"PD-240"` |
| `ROBOT24` | `"robot24"` | `"Robot 24"` |
| `ROBOT36` | `"robot36"` | `"Robot 36"` |
| `ROBOT72` | `"robot72"` | `"Robot 72"` |
| `SCOTTIE1` | `"scottie1"` | `"Scottie 1"` |
| `SCOTTIE2` | `"scottie2"` | `"Scottie 2"` |
| `SCOTTIE_DX` | `"scottiedx"` | `"Scottie DX"` |
| `MARTIN1` | `"martin1"` | `"Martin 1"` |
| `MARTIN2` | `"martin2"` | `"Martin 2"` |

Example — for `const PD120: ModeSpec = ModeSpec {` (around line 198), insert immediately after `mode: SstvMode::Pd120,`:

```rust
    short_name: "pd120",
    name: "PD-120",
```

The complete updated `PD120` const should look like:

```rust
const PD120: ModeSpec = ModeSpec {
    mode: SstvMode::Pd120,
    short_name: "pd120",
    name: "PD-120",
    vis_code: 0x5F,
    line_pixels: 640,
    image_lines: 496,
    line_seconds: 0.508_48,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_19,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
};
```

Repeat for the other 10 consts with the appropriate values from the mapping table above. Watch for: the const for Scottie DX is named `SCOTTIE_DX` (with underscore), and its `SstvMode` variant is `SstvMode::ScottieDx` (no underscore).

- [ ] **Step 3: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **135** (unchanged — additive change, no new tests).

**Critical:** the existing per-mode `vis_code_resolves` tests check specific fields (`line_pixels`, `sync_seconds`, etc.) but none of them touches `short_name` or `name`, so they should all still pass without modification. If any fail, double-check that you only added the two new fields and didn't accidentally modify existing values.

If clippy fires `clippy::missing_fields_in_debug` (since `ModeSpec` derives `Debug` and the new fields will appear in Debug output), no action needed — `derive(Debug)` covers all fields automatically.

If the new fields trigger any `clippy::missing_docs_in_private_items` or similar pedantic lints, the doc comments included above should silence them.

- [ ] **Step 4: Commit**

```bash
git add src/modespec.rs
git commit -m "refactor(modespec): add short_name + name fields to ModeSpec (#91 B13)

Adds two new public string fields on ModeSpec:
- short_name: CLI/filename slug (\"pd120\", \"robot24\", \"scottiedx\", ...)
- name: human-readable display name (\"PD-120\", \"Robot 24\", \"Scottie DX\", ...)

Populated in all 11 const blocks. Additive change — no existing
tests modified, no behavior change. T2 (CLI consumer) and T3
(ALL_SPECS table + F8 test) follow.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: CLI consumer cleanup — delete `mode_tag`, use `for_mode(m).short_name`

Replace the 30-line `mode_tag` function in `src/bin/slowrx_cli.rs` with a direct call to `for_mode(m).short_name`. Delete the lockstep test. The 4-incident comment block goes away with the function.

**Files:**
- Modify: `src/bin/slowrx_cli.rs`

- [ ] **Step 1: Replace the call site**

In `src/bin/slowrx_cli.rs`, find the single call to `mode_tag` (around line 132 — the `format!("img-{image_count:03}-{}.png", mode_tag(image.mode))` line). The current imports (line 21) are `use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode};` — `modespec` is not directly imported, so use the fully-qualified path:

```rust
// Before:
.join(format!("img-{image_count:03}-{}.png", mode_tag(image.mode)));
// After:
.join(format!("img-{image_count:03}-{}.png", slowrx::modespec::for_mode(image.mode).short_name));
```

(If you prefer, you can instead add `modespec` to the existing `use slowrx::{...}` import and call `modespec::for_mode(image.mode).short_name`. Either form is fine — pick the one that fits the file's existing import style.)

- [ ] **Step 2: Delete the `mode_tag` function**

In `src/bin/slowrx_cli.rs`, delete the entire `fn mode_tag(mode: SstvMode) -> &'static str { ... }` block (lines 282-312). This includes:
- The function signature.
- The 11 explicit match arms.
- The 4-incident comment block (V2.1 PD240, V2.2 Robot24/36/72, V2.3 Scottie 1/2/DX, V2.4 Martin 1/2).
- The `_ => "unknown"` wildcard arm.

The whole function block goes — nothing replaces it.

- [ ] **Step 3: Delete the lockstep test**

In `src/bin/slowrx_cli.rs`, find the `#[cfg(test)] mod tests { ... }` block (around line 328-360). Delete the entire `mode_tag_covers_all_known_variants` test fn (lines 332-359), including its doc comment.

If the `tests` module becomes empty after this deletion, leave the empty `mod tests { use super::*; }` skeleton in place — don't delete the module itself (the binary may have other tests added later, and leaving the `use super::*;` import means future test additions don't need to rebuild it). If the module had other tests besides `mode_tag_covers_all_known_variants`, leave them untouched.

Actually verify this with a quick grep first:

```bash
grep -n "#\[test\]\|^fn\b" /data/source/slowrx.rs/src/bin/slowrx_cli.rs | head -20
```

If `mode_tag_covers_all_known_variants` was the only test in the module, leave the empty module skeleton; if there are other tests, just delete that one fn.

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **135** (unchanged). The `slowrx_cli` binary test count drops by 1 (the deleted `mode_tag_covers_all_known_variants` test).

**Critical:** the CLI's behavior must be unchanged. The `mode_tag` strings (`"pd120"`, `"robot24"`, etc.) and the `short_name` values populated in T1 are character-for-character identical. If any existing CLI integration test fails on filename format, double-check the mapping table in T1 Step 2.

If `clippy::wildcard_imports` or similar fires on the CLI's `use ...` statements after the cleanup (e.g., if `mode_tag` was the only consumer of `SstvMode` and now `SstvMode` import is unused), remove the unused imports. Use `cargo clippy --fix --allow-dirty --bin slowrx_cli` for a mechanical first pass, then review the changes before committing.

- [ ] **Step 5: Commit**

```bash
git add src/bin/slowrx_cli.rs
git commit -m "refactor(cli): replace mode_tag with for_mode(m).short_name (#91 B13)

Deletes the 30-line mode_tag fn (with its 4-incident comment block
documenting the V2.1/V2.2/V2.3/V2.4 stale-tag bugs) and the
lockstep mode_tag_covers_all_known_variants test. The CLI now
sources its filename slugs from ModeSpec::short_name (added in T1),
so adding a new SstvMode variant without its short_name is caught
by the compile-time exhaustive match in modespec::for_mode rather
than by a runtime test.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Add `ALL_SPECS`; derive `lookup`; add F8 round-trip test

Introduce `ALL_SPECS: [ModeSpec; 11]` as the single source of truth. Rewrite `lookup` as a one-liner over the table. Add the F8 property test asserting the table's structural health.

**Files:**
- Modify: `src/modespec.rs`

- [ ] **Step 1: Add the `ALL_SPECS` constant**

In `src/modespec.rs`, find the end of the const blocks — immediately after `const MARTIN2: ModeSpec = ModeSpec { ... };` (the last const, around line 378), and immediately before the `#[cfg(test)]` line. Insert:

```rust

/// All implemented mode specs. Single source of truth — [`lookup`] is
/// derived from this; [`for_mode`] keeps its exhaustive match so
/// adding a `SstvMode` variant without a `const ModeSpec` (and a
/// matching arm in `for_mode`) is a compile error, by design.
///
/// The F8 round-trip test (`all_specs_roundtrip`) verifies every
/// entry's `(mode, vis_code, short_name, name)` quadruple is unique
/// and that `lookup` and `for_mode` agree with the table.
pub(crate) const ALL_SPECS: [ModeSpec; 11] = [
    PD120, PD180, PD240,
    ROBOT24, ROBOT36, ROBOT72,
    SCOTTIE1, SCOTTIE2, SCOTTIE_DX,
    MARTIN1, MARTIN2,
];
```

- [ ] **Step 2: Rewrite `lookup` to derive from `ALL_SPECS`**

In `src/modespec.rs`, find the existing `pub fn lookup(vis_code: u8) -> Option<ModeSpec>` (around lines 154-170 — the function with a 12-arm match on `vis_code`). Replace the entire fn body (everything between `{ ... }`) with a one-liner:

```rust
#[must_use]
pub fn lookup(vis_code: u8) -> Option<ModeSpec> {
    ALL_SPECS.iter().find(|s| s.vis_code == vis_code).copied()
}
```

The fn's existing doc comment (the long block starting `Look up the [`ModeSpec`] ...`, which includes the parity-audit note about `0x00` being intentionally unmapped) stays exactly as it is. Just the body changes.

If the existing doc starts with "Look up the [`ModeSpec`] for a given 7-bit VIS code...", append one line to its first paragraph: " Derived from [`ALL_SPECS`]." So the first paragraph becomes:

```rust
/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns `None`
/// if the code is reserved, undefined, or maps to a mode not yet
/// implemented in this release. Derived from [`ALL_SPECS`].
```

Leave the rest of the doc (the "VIS codes are taken from Dave Jones..." citation and the "Parity-audit note (#27)..." block) untouched.

- [ ] **Step 3: Run the gate to confirm `lookup` refactor is behavior-preserving**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **135** (no new tests yet). The existing per-mode `vis_code_resolves` tests verify `lookup(0x5F)` returns `Some(PD120)` etc. — those must still pass post-refactor. If any fail, the most likely cause is a typo in `ALL_SPECS` (wrong order, missing variant). Cross-check against the 11-element list above.

If clippy fires `clippy::needless_pass_by_value` on the `.find(|s| ...)` closure (it shouldn't — `s` is `&&ModeSpec`), use `|s: &&ModeSpec|` with an explicit type, or `|s| s.vis_code == vis_code` (the closure form above should be lint-clean).

- [ ] **Step 4: Add the F8 round-trip test**

In `src/modespec.rs`, find the `#[cfg(test)] mod tests { ... }` block (around line 380). Append the F8 test at the end of the block (right before the closing `}` of the module):

```rust
    /// F8 (#91). Every entry in `ALL_SPECS` round-trips cleanly
    /// through `lookup` (VIS code → spec) and `for_mode` (mode →
    /// spec); the table has exactly 11 unique modes, 11 unique VIS
    /// codes, 11 unique short_names; every `name` and `short_name`
    /// is non-empty.
    ///
    /// Subsumes the per-mode `vis_code_resolves` tests as a
    /// structural invariant. The individual per-mode tests stay as
    /// fast-failing regression guards with descriptive names.
    #[test]
    fn all_specs_roundtrip() {
        use std::collections::HashSet;

        let modes: HashSet<_> = ALL_SPECS.iter().map(|s| s.mode).collect();
        assert_eq!(
            modes.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate modes"
        );

        let vis: HashSet<_> = ALL_SPECS.iter().map(|s| s.vis_code).collect();
        assert_eq!(
            vis.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate VIS codes"
        );

        let short_names: HashSet<_> = ALL_SPECS.iter().map(|s| s.short_name).collect();
        assert_eq!(
            short_names.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate short_names"
        );

        for spec in ALL_SPECS.iter().copied() {
            assert_eq!(
                lookup(spec.vis_code),
                Some(spec),
                "lookup({:#04x}) did not return ALL_SPECS entry for {:?}",
                spec.vis_code,
                spec.mode
            );
            assert_eq!(
                for_mode(spec.mode),
                spec,
                "for_mode({:?}) did not match ALL_SPECS entry",
                spec.mode
            );
            assert!(
                !spec.short_name.is_empty(),
                "{:?}: short_name empty",
                spec.mode
            );
            assert!(!spec.name.is_empty(), "{:?}: name empty", spec.mode);
        }
    }
```

- [ ] **Step 5: Run the F8 test in isolation**

```bash
cargo test --all-features --locked --release --lib modespec -- all_specs_roundtrip
```

Expected: 1 passed.

If it fails on the `duplicate modes` / `duplicate VIS codes` / `duplicate short_names` assertion, the `ALL_SPECS` array has a typo (probably a duplicated entry). If it fails on the per-spec `lookup(...)` assertion, `ALL_SPECS` is missing a variant or has a wrong `vis_code` value. If it fails on `for_mode(...)`, the `for_mode` match arm and the `const` definitions disagree on field values (most likely a typo in one of the T1 inserts).

- [ ] **Step 6: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (135 + 1 new F8). The existing per-mode tests (24 in `modespec::tests`) all stay green.

- [ ] **Step 7: Commit**

```bash
git add src/modespec.rs
git commit -m "refactor(modespec): ALL_SPECS table + derive lookup + F8 roundtrip (#91 B7/F8)

Adds ALL_SPECS: [ModeSpec; 11] as the single source of truth.
Rewrites lookup() as a one-liner over the table (.iter().find()).
for_mode keeps its exhaustive match — the compile-time check that
every SstvMode variant has a const entry is the load-bearing
invariant; the F8 test catches table-level inconsistencies at
test-run time (duplicate modes/VIS/short_names, mismatched lookup
or for_mode).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Doc cleanup — `for_mode` doc (E1) + `SstvMode` variant docs (E11)

Pure documentation. Rewrite `for_mode`'s doc to describe what it actually guarantees. Normalize the 11 `SstvMode` variant docs to terse one-liners. Add a one-line cross-reference comment near the existing Scottie DX Hann-bump explanation in `src/demod.rs`.

**Files:**
- Modify: `src/modespec.rs`
- Modify: `src/demod.rs`

- [ ] **Step 1: Rewrite `for_mode`'s doc comment (E1)**

In `src/modespec.rs`, find the existing doc comment immediately preceding `pub fn for_mode(mode: SstvMode) -> ModeSpec` (around lines 172-177). It currently reads:

```rust
/// Look up the [`ModeSpec`] for a known [`SstvMode`].
///
/// Always returns `Some` for V1 modes. Reserved for symmetry with
/// [`lookup`] when V2 modes whose decoders are not yet implemented
/// land in the enum.
```

(Note: the existing doc incorrectly says "Always returns `Some`" — the function actually returns `ModeSpec`, not `Option<ModeSpec>`. That's the audit's E1 finding.)

Replace with:

```rust
/// Look up the [`ModeSpec`] for an [`SstvMode`].
///
/// Total over [`SstvMode`] — every implemented variant has a `const`
/// entry. Adding a new variant without adding its `const ModeSpec`
/// (and an arm here) is a compile error, by design. Pair with
/// [`lookup`] when starting from a VIS code on the wire.
```

The function body (the `match mode { ... }` block) stays exactly as-is. Only the doc comment changes.

- [ ] **Step 2: Normalize `SstvMode` variant docs (E11)**

In `src/modespec.rs`, find the `pub enum SstvMode { ... }` block (around lines 16-43). Replace the entire enum body (every variant doc + variant declaration) with the normalized version:

```rust
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SstvMode {
    /// PD-120. VIS `0x5F`. See [`for_mode`] for full timing.
    Pd120,
    /// PD-180. VIS `0x60`.
    Pd180,
    /// PD-240. VIS `0x61`.
    Pd240,
    /// Robot 24 (conventional name — decode buffer is ~36 s). VIS `0x04`.
    Robot24,
    /// Robot 36. VIS `0x08`.
    Robot36,
    /// Robot 72. VIS `0x0C`.
    Robot72,
    /// Scottie 1. VIS `0x3C`.
    Scottie1,
    /// Scottie 2. VIS `0x38`.
    Scottie2,
    /// Scottie DX. VIS `0x4C`.
    ScottieDx,
    /// Martin 1. VIS `0x2C`.
    Martin1,
    /// Martin 2. VIS `0x28`.
    Martin2,
}
```

The `#[non_exhaustive]` and `#[derive(...)]` attributes stay. The enum-level doc comment (the line ` /// SSTV operating mode. Implemented: ...` immediately above the `#[non_exhaustive]`) also stays as-is.

The variant docs lose their dimensional details (320×256, ms/pixel) and the Scottie DX Hann-bump impl note — those details either already live in the `const ModeSpec` blocks (dimensions) or migrate to `src/demod.rs` (Hann-bump cross-ref, next step).

- [ ] **Step 3: Add cross-reference comment in `src/demod.rs`**

In `src/demod.rs`, find the existing Scottie DX Hann-bump comment block (lines 567-576). It currently reads:

```rust
            // slowrx C video.c:367 — Scottie DX bumps WinIdx up by one when not
            // already at saturation, giving SDX's 1.08 ms/pixel a longer
            // integration window. Applied AFTER the hysteresis selector so
            // `prev_win_idx` continues tracking the un-bumped SNR-derived index
            // (the bump shouldn't compound across pixels).
            if ctx.spec.mode == crate::modespec::SstvMode::ScottieDx
```

Append one line to the comment block (just before the `if ctx.spec.mode == ...` line):

```rust
            // slowrx C video.c:367 — Scottie DX bumps WinIdx up by one when not
            // already at saturation, giving SDX's 1.08 ms/pixel a longer
            // integration window. Applied AFTER the hysteresis selector so
            // `prev_win_idx` continues tracking the un-bumped SNR-derived index
            // (the bump shouldn't compound across pixels).
            //
            // (Audit #91 E11: the prior doc-comment on `SstvMode::ScottieDx`
            // mentioned this bump; moved here to live next to the actual code.)
            if ctx.spec.mode == crate::modespec::SstvMode::ScottieDx
```

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (no new tests; doc-only). The rustdoc step is the load-bearing check here — if any intra-doc link breaks (`[`for_mode`]`, `[`lookup`]`, `[`ALL_SPECS`]`, etc.), rustdoc fails under `-D warnings`. If that fires, double-check the link target syntax (rustdoc accepts `[ItemName]` for items in scope; `[`backticks`]` is also fine and renders as code).

- [ ] **Step 5: Commit**

```bash
git add src/modespec.rs src/demod.rs
git commit -m "docs(modespec): rewrite for_mode doc + normalize SstvMode variants (#91 E1/E11)

- for_mode doc no longer claims it returns Option (E1).
- SstvMode variant docs normalized to terse one-liners; dimensional
  facts live on the const ModeSpec blocks (E11).
- Scottie DX Hann-bump impl note moves from the variant doc into the
  existing comment block at src/demod.rs:567-576 (next to the code
  that does the bump).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

Open `CHANGELOG.md`. Under `## [Unreleased]` `### Internal`, prepend (so the newest change is first) a new bullet **before** the existing find_sync cleanup bullet:

```markdown
### Internal

- **`ModeSpec` as single source of truth** — adds two new fields
  (`short_name: &'static str` like `"pd120"`/`"robot24"`/`"scottiedx"`,
  and `name: &'static str` like `"PD-120"`/`"Scottie DX"`) to
  `ModeSpec`. Introduces `ALL_SPECS: [ModeSpec; 11]` as the canonical
  table; `lookup(vis_code)` is now a one-liner over it (audit B7).
  CLI's hand-maintained `mode_tag` function (with its 4-incident
  stale-tag history) is deleted — `slowrx_cli` now calls
  `for_mode(m).short_name` directly (audit B13); the lockstep
  `mode_tag_covers_all_known_variants` test goes away too. `for_mode`
  doc rewritten to accurately describe its return type (audit E1).
  `SstvMode` variant docs normalized to terse one-liners; dimensional
  facts live on the `const ModeSpec` blocks; the Scottie DX
  Hann-window-index bump explanation moves from the variant doc into
  the existing comment near the code at `src/demod.rs:567` (audit
  E11). New F8 round-trip test (`all_specs_roundtrip`) verifies every
  table entry's `(mode, vis_code, short_name, name)` quadruple is
  unique and that `lookup` and `for_mode` agree with the table.
  Behavior-preserving: existing CLI filenames are bit-for-bit
  identical (the `short_name` values match the prior `mode_tag`
  output). (#91; audit B7/B13/E1/E11/F8.)

- **`find_sync` cleanup** — ... [existing #88 bullet stays as-is below]
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
- lib: **136** (135 pre-#91 + 1 new F8 `all_specs_roundtrip`).
- `tests/roundtrip.rs`: 11/11 (unchanged).
- `slowrx_cli` binary: drops by 1 from the pre-#91 count (deleted `mode_tag_covers_all_known_variants`).
- All other integration tests (cli, multi_image, no_vis, unknown_vis): unchanged.
- Doc clean.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the ModeSpec single-source refactor (#91)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - B7 (`ALL_SPECS` + derive `lookup`) → T3 Steps 1-2.
  - B13 (`short_name` + `name` on `ModeSpec`; CLI consumer) → T1 (fields) + T2 (CLI).
  - E1 (`for_mode` doc rewrite) → T4 Step 1.
  - E11 (`SstvMode` variant docs + Scottie DX Hann-bump migration) → T4 Steps 2-3.
  - F8 (round-trip property test) → T3 Step 4.
  - CHANGELOG → T5.

- **No TDD-red moment.** All five audit findings are preventability improvements (single-source consolidation + doc accuracy), not bug fixes. The F8 test in T3 is a structural invariant guard, not a regression target.

- **Behavior preservation guarantees:**
  - CLI filenames: `for_mode(m).short_name` returns the same string as the deleted `mode_tag(m)` for every variant (verified by the mapping table in T1 Step 2 matching the CLI's `mode_tag` arms character-for-character).
  - `lookup(vis_code)`: the `.iter().find()` over `ALL_SPECS` returns the same `Option<ModeSpec>` as the deleted 12-arm match (the F8 test asserts every existing entry round-trips).
  - `for_mode(mode)`: unchanged (body untouched in T4 Step 1).
  - All existing per-mode tests in `modespec::tests` pass without modification (24 tests, none of them touch `short_name` or `name`).

- **Compile-time vs run-time gating of "new variant added without table entry":**
  - `SstvMode` variant added without `for_mode` arm → **compile error** (load-bearing).
  - `SstvMode` variant + `for_mode` arm added without `const` definition → **compile error** (the `for_mode` arm has nowhere to map to).
  - `const ModeSpec` defined without adding to `ALL_SPECS` → **test failure** (F8's `all_specs_roundtrip` would fail at the `lookup(vis_code).is_some()` assertion for the unmapped variant).
  - Duplicate `vis_code` between two entries → **test failure** (F8's HashSet uniqueness check).
  - Duplicate `short_name` → **test failure** (F8's HashSet uniqueness check).

- **Out of scope** (tracked elsewhere):
  - `Display` / `FromStr` for `SstvMode` (#92 or later).
  - Generalizing `ALL_SPECS` to a `pub` API for external consumers (#92 hygiene sweep may revisit visibility).
  - Renaming the 11 standalone `const`s (e.g., `PD120` → `MODESPEC_PD120`) for namespace clarity (out of scope; would add churn for marginal benefit).
