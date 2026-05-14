# Issue #91 — `ModeSpec` as single source of truth — Design

**Issue:** [#91](https://github.com/jasonherald/slowrx.rs/issues/91) (audit bundle 7 of 12 — IDs B7, B13, E1, E11, F8).

**Scope:** consolidate the scattered mode-metadata mappings in `src/modespec.rs` behind a single `ALL_SPECS` table; derive `lookup` from it; add `short_name` + `name` fields to `ModeSpec` so the CLI's hand-maintained `mode_tag` map (4× stale-bug history) becomes one line; rewrite stale doc comments (`for_mode`, `SstvMode` variants); add an F8 round-trip property test. The change is pure refactor + new fields — no behavior changes for the decoder.

---

## Background — audit findings

- **B7 — three independently-maintained mappings.** `lookup(vis_code) → Some(spec)`, `for_mode(mode) → spec`, and the 11 standalone `const SPEC: ModeSpec = ...` definitions are three places the same `(mode, vis_code, spec)` association lives. Drift across them has been a recurring tax. Fix: define `const ALL_SPECS: [ModeSpec; 11] = [...]`, derive `lookup` from it via `.iter().find()`, keep `for_mode` as an exhaustive `match` (the compile-time check that every `SstvMode` variant has a `const`).
- **B13 — CLI `mode_tag` is a separate stale map.** `src/bin/slowrx_cli.rs:282-312` has its own 11-arm match producing slug strings (`"pd120"`, `"robot24"`, `"scottiedx"`, …) used in filenames like `img-NNN-{tag}.png`. It has fallen through to its `"unknown"` wildcard arm 4 times when new variants weren't added (V2.1 PD240; V2.2 Robot24/36/72; V2.3 Scottie 1/2/DX; V2.4 Martin 1/2 — documented in the source). A `mode_tag_covers_all_known_variants` test catches the omission, but the underlying `#[non_exhaustive]` enum forces a wildcard arm that hides the gap from the compiler. Fix: add `short_name: &'static str` to `ModeSpec`; CLI calls `for_mode(m).short_name`. Drops the wildcard and the lockstep test.
- **E1 — `for_mode` doc says it returns `Option`.** It returns `ModeSpec`. Rewrite to describe what the function actually guarantees: "total over `SstvMode`; adding a variant without a const arm is a compile error (intentional)."
- **E11 — `SstvMode` variant docs are inconsistent.** Scottie/Martin list VIS + dims + impl notes inline (e.g., Scottie DX has "+1 Hann-window-index bump in per-pixel demod"); PD/Robot are terse. Normalize to terse one-liners; dimensional facts live on the `ModeSpec` const blocks; the ScottieDx Hann-bump fact migrates to a comment near the actual code in `src/demod.rs:567-576` (which already explains the bump).
- **F8 — round-trip property test.** Iterate `ALL_SPECS`; assert `lookup(s.vis_code) == Some(s)`, `for_mode(s.mode) == s`, distinct VIS codes, distinct modes, distinct short_names, non-empty name/short_name.

---

## Architecture

### `ModeSpec` gains two fields

```rust
pub struct ModeSpec {
    pub mode: SstvMode,
    /// CLI/filename slug. Stable across releases (filenames like
    /// `img-NNN-{short_name}.png` depend on this). lowercase, no
    /// separators: "pd120", "robot24", "scottiedx", "scottie1",
    /// "martin1", etc. (B13)
    pub short_name: &'static str,
    /// Human-readable mode name. For log lines and any future
    /// user-facing display. "PD-120", "Robot 24", "Scottie DX", etc.
    /// (B13)
    pub name: &'static str,
    pub vis_code: u8,
    // ... (existing fields unchanged)
}
```

Each of the 11 existing `const PD120/PD180/PD240/ROBOT24/.../MARTIN2: ModeSpec` definitions adds two new lines (`short_name: "..."`, `name: "..."`).

Mapping table (slug → display):

| Mode variant | short_name | name |
|---|---|---|
| `Pd120` | `"pd120"` | `"PD-120"` |
| `Pd180` | `"pd180"` | `"PD-180"` |
| `Pd240` | `"pd240"` | `"PD-240"` |
| `Robot24` | `"robot24"` | `"Robot 24"` |
| `Robot36` | `"robot36"` | `"Robot 36"` |
| `Robot72` | `"robot72"` | `"Robot 72"` |
| `Scottie1` | `"scottie1"` | `"Scottie 1"` |
| `Scottie2` | `"scottie2"` | `"Scottie 2"` |
| `ScottieDx` | `"scottiedx"` | `"Scottie DX"` |
| `Martin1` | `"martin1"` | `"Martin 1"` |
| `Martin2` | `"martin2"` | `"Martin 2"` |

The `short_name` column exactly matches the strings the current `mode_tag` function returns, so existing CLI filenames are preserved bit-for-bit (no script-breakage for users who automate around the filename format).

### `ALL_SPECS` constant

```rust
/// All implemented mode specs. Single source of truth — `lookup` is
/// derived from this; `for_mode` keeps its exhaustive match so
/// adding a `SstvMode` variant without a `const ModeSpec` (and a
/// matching arm in `for_mode`) is a compile error, by design.
///
/// F8 round-trip test (`all_specs_roundtrip`) verifies every entry's
/// `(mode, vis_code, short_name, name)` quadruple is unique and that
/// `lookup` and `for_mode` agree with the table.
pub(crate) const ALL_SPECS: [ModeSpec; 11] = [
    PD120, PD180, PD240,
    ROBOT24, ROBOT36, ROBOT72,
    SCOTTIE1, SCOTTIE2, SCOTTIE_DX,
    MARTIN1, MARTIN2,
];
```

Visibility is `pub(crate)` — external consumers call `lookup` or `for_mode`. Easy to relax later if a use case emerges.

### `lookup` becomes a one-liner

```rust
/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns
/// `None` if the code is reserved, undefined, or maps to a mode not
/// yet implemented in this release.
///
/// Derived from [`ALL_SPECS`] — adding a new mode is one table
/// entry; no separate update here.
///
/// (existing parity-audit note about `0x00 → None` kept verbatim;
/// it explains the decoder's `UnknownVis` event semantics)
#[must_use]
pub fn lookup(vis_code: u8) -> Option<ModeSpec> {
    ALL_SPECS.iter().find(|s| s.vis_code == vis_code).copied()
}
```

### `for_mode` keeps the exhaustive match (E1 doc rewrite)

```rust
/// Look up the [`ModeSpec`] for an [`SstvMode`].
///
/// Total over [`SstvMode`] — every implemented variant has a const
/// entry. Adding a new variant without adding its `const ModeSpec`
/// (and an arm here) is a compile error, by design. Pair with
/// [`lookup`] when starting from a VIS code on the wire.
#[must_use]
pub fn for_mode(mode: SstvMode) -> ModeSpec {
    match mode {
        SstvMode::Pd120 => PD120,
        SstvMode::Pd180 => PD180,
        SstvMode::Pd240 => PD240,
        SstvMode::Robot24 => ROBOT24,
        SstvMode::Robot36 => ROBOT36,
        SstvMode::Robot72 => ROBOT72,
        SstvMode::Scottie1 => SCOTTIE1,
        SstvMode::Scottie2 => SCOTTIE2,
        SstvMode::ScottieDx => SCOTTIE_DX,
        SstvMode::Martin1 => MARTIN1,
        SstvMode::Martin2 => MARTIN2,
    }
}
```

(Body unchanged; only the doc comment changes.)

### `SstvMode` variant docs normalized (E11)

```rust
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

The Robot24 note ("conventional name — decode buffer is ~36 s") is the one fact the audit asked to preserve at the variant level. The `+1 Hann-window-index bump` line currently on `ScottieDx` is removed; the existing comment at `src/demod.rs:567-576` (which already explains the bump in detail) gains a one-line cross-reference noting that `ModeSpec::short_name == "scottiedx"` is the dispatch key.

### CLI cleanup (B13 consumer)

In `src/bin/slowrx_cli.rs`:

1. **Delete** the `mode_tag` fn (lines 282-312, including the 4-incident comment block).
2. **Delete** the `mode_tag_covers_all_known_variants` test (lines 332-359).
3. **Update** the single call site at line ~132:
   ```rust
   // Before:
   .join(format!("img-{image_count:03}-{}.png", mode_tag(image.mode)));
   // After:
   .join(format!("img-{image_count:03}-{}.png", modespec::for_mode(image.mode).short_name));
   ```

The wildcard arm goes away with the function. The `mode_tag_covers_all_known_variants` test is replaced (more directly) by the F8 round-trip test in `modespec.rs`.

### F8 round-trip property test

Append to `src/modespec.rs::tests`:

```rust
/// F8 (#91). Every entry in `ALL_SPECS` round-trips cleanly through
/// `lookup` (VIS code → spec) and `for_mode` (mode → spec); the
/// table has exactly 11 unique modes, 11 unique VIS codes, 11
/// unique short_names; every `name` and `short_name` is non-empty.
///
/// Replaces the per-mode `vis_code_resolves` tests as the structural
/// guarantee. The individual per-mode tests stay as fast-failing
/// regression guards with descriptive names.
#[test]
fn all_specs_roundtrip() {
    use std::collections::HashSet;

    let modes: HashSet<_> = ALL_SPECS.iter().map(|s| s.mode).collect();
    assert_eq!(modes.len(), ALL_SPECS.len(), "ALL_SPECS has duplicate modes");

    let vis: HashSet<_> = ALL_SPECS.iter().map(|s| s.vis_code).collect();
    assert_eq!(vis.len(), ALL_SPECS.len(), "ALL_SPECS has duplicate VIS codes");

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
        assert!(!spec.short_name.is_empty(), "{:?}: short_name empty", spec.mode);
        assert!(!spec.name.is_empty(), "{:?}: name empty", spec.mode);
    }
}
```

---

## File touch list

| File | Status | Role |
|------|--------|------|
| `src/modespec.rs` | modify | Add `short_name` + `name` to `ModeSpec`; populate them in 11 const blocks; add `ALL_SPECS`; rewrite `lookup`; rewrite `for_mode` doc; normalize `SstvMode` variant docs (drop Scottie DX impl note + Scottie/Martin dim inlines); add F8 test. |
| `src/bin/slowrx_cli.rs` | modify | Delete `mode_tag` fn + its lockstep test; update the one call site to use `for_mode(m).short_name`. |
| `src/demod.rs` | modify | One-line cross-reference comment near the Scottie DX Hann-bump (the existing comment is thorough — just point at it from where the variant doc used to live). |
| `CHANGELOG.md` | modify | One bullet under `[Unreleased] ### Internal`. |

---

## Out of scope

- Generalizing `ALL_SPECS` to a `pub` API for external consumers (no concrete use case yet; `lookup`/`for_mode` cover everything).
- Renaming the const items (e.g., `PD120 → MODESPEC_PD120`). The current names are fine; renaming creates churn without a benefit proportional to the diff.
- Reorganizing the 11 const definitions into a single literal inside `ALL_SPECS` (would drop the standalone consts but obscures one-mode-at-a-time editing).
- Adding `Display`/`FromStr` for `SstvMode` based on `short_name` (separate audit item; could be a follow-up if a parser ever needs it).

---

## Success criteria

- All 5 audit findings addressed (B7, B13, E1, E11, F8).
- Full CI gate green: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
- Lib test count: **135 → 136** (current 135 + 1 new F8 `all_specs_roundtrip`). The deleted `mode_tag_covers_all_known_variants` lives in the `slowrx_cli` binary target, so its removal decrements the binary's test count (not the lib's).
- Existing per-mode `vis_code_resolves` tests still pass (they were already covered by the F8 invariants; we retain them as named regression guards).
- `tests/roundtrip.rs` 11/11 unchanged.
- CLI filename format `img-NNN-{slug}.png` produces identical strings to the pre-change `mode_tag` output for every variant (verified by `find target -name "img-*"` from a prior run; the F8 test's `short_names` uniqueness check is the structural guarantee).
- Adding a new `SstvMode` variant without a corresponding `const ModeSpec` + `for_mode` arm + `ALL_SPECS` entry produces a **compile error** (the `for_mode` match is exhaustive; the F8 test catches the `ALL_SPECS` omission at test-run time).
