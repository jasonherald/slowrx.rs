# Issue #90 — Multi-image streaming — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After `SstvDecoder::process` finishes an image it should re-enter VIS detection *in place* (`continue`, not `break`) so a back-to-back transmission's next VIS — and the image after it — surface in the same `process()` call (#90 A2); and the VIS detector should be re-armed on only the last few scan lines of the decoded image audio plus everything past, not the entire ~1.4–3 M-sample buffer (#90 D4).

**Architecture:** One localized change in the `State::Decoding(d)` arm of `SstvDecoder::process`: after `run_findsync_and_decode`, compute a small carry-back window (`MULTI_IMAGE_CARRYBACK_LINES` scan lines of working-rate audio), re-arm the VIS detector on `&d.audio[carry_from..]` via the existing `restart_vis_detection` helper (which already takes `&[f32]`), set `self.state = State::AwaitingVis`, clear `remaining`, and `continue`. Plus an honest doc fix on the `working_samples_emitted` field (the `sample_offset`-relative-after-restart limitation, tracked separately in #99) and one integration test.

**Tech Stack:** Rust 2021, MSRV 1.85. Crate clippy config: `clippy::all`/`pedantic` = warn, `unwrap_used`/`panic`/`expect_used` = warn (no panics in lib code). CI gate: `cargo test --all-features --locked --release`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`. No GPG signing (`git commit` / `git tag -a` plain).

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-11-issue-90-multi-image-streaming-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs A2, D4)
- Follow-up issue (out of scope here): #99 (`sample_offset` relative-to-restart)

---

## File Structure

| File | Change |
|------|--------|
| `tests/multi_image.rs` | **New.** Integration test: `decoder_decodes_two_back_to_back_images` — feed `[VIS(PD120)][PD120 img1][VIS(PD120)][PD120 img2][pad]` in one `process()` call; assert exactly 2 `VisDetected{Pd120}` + 2 `ImageComplete{partial:false}`, in order. `#![cfg(feature = "test-support")]`. |
| `Cargo.toml` | Add `[[test]] name = "multi_image" required-features = ["test-support"]` (matches `roundtrip` / `unknown_vis`). |
| `src/decoder.rs` | Add `const MULTI_IMAGE_CARRYBACK_LINES: u32 = 4;`. Rewrite the post-image transition in the `State::Decoding` arm (carry-back slice → `restart_vis_detection` → `State::AwaitingVis` → `remaining = &[]` → `continue`). Fix the `working_samples_emitted` field doc (the `sample_offset`-after-restart limitation). |
| `CHANGELOG.md` | `[Unreleased]` Fixed + Performance entries. |

Task order: **T1** (failing integration test + `Cargo.toml`) → **T2** (decoder fix + const + `working_samples_emitted` doc fix + CHANGELOG) → **T3** (full CI gate).

---

## Task 1: Failing integration test for back-to-back images

Write the test first (TDD red). It must compile against the current code but **fail** the count assertions: the current `break`-after-`ImageComplete` returns only image 1's events from a single `process()` call.

**Files:**
- Create: `tests/multi_image.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Create `tests/multi_image.rs`**

```rust
//! Multi-image streaming — back-to-back SSTV transmissions must all decode
//! within a single `SstvDecoder::process` call, not one-per-call (audit #90:
//! A2 + D4). Counterpart to `tests/roundtrip.rs` (single image) and
//! `tests/unknown_vis.rs` (post-unknown-VIS reseed).

#![cfg(feature = "test-support")]
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use slowrx::{SstvDecoder, SstvEvent, SstvMode, WORKING_SAMPLE_RATE_HZ};

/// PD120 == VIS code 0x5F.
const PD120_CODE: u8 = 0x5F;

/// A small synthetic YCrCb image for PD120 (luma gradient + smooth chroma
/// stripes — same shape as `tests/roundtrip.rs`'s `test_image`, so the
/// encoder's adjacent-row chroma averaging has something it can reproduce).
fn pd120_test_image() -> Vec<[u8; 3]> {
    let spec = slowrx::for_mode(SstvMode::Pd120);
    let w = spec.line_pixels;
    let h = spec.image_lines;
    let mut ycrcb = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let lum = ((f64::from(x)) / (f64::from(w)) * 255.0) as u8;
            let cr = if y % 4 < 2 { 200 } else { 56 };
            let cb = if (y / 2) % 2 == 0 { 200 } else { 56 };
            ycrcb.push([lum, cr, cb]);
        }
    }
    ycrcb
}

#[test]
fn decoder_decodes_two_back_to_back_images() {
    let img1 = pd120_test_image();
    let img2 = pd120_test_image();

    // Two complete PD120 transmissions, concatenated, then a pad to absorb
    // the resampler FIR group delay so image 2's last line + VIS 2 produce
    // full analysis windows (2048 matches tests/roundtrip.rs's PD padding).
    let mut audio = slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0);
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(SstvMode::Pd120, &img1));
    audio.extend(slowrx::__test_support::vis::synth_vis(PD120_CODE, 0.0));
    audio.extend(slowrx::__test_support::mode_pd::encode_pd(SstvMode::Pd120, &img2));
    audio.extend(std::iter::repeat_n(0.0_f32, 2048));

    let mut decoder = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder construct");
    let events = decoder.process(&audio); // ONE call

    let vis_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SstvEvent::VisDetected { mode: SstvMode::Pd120, .. }))
        .map(|(i, _)| i)
        .collect();
    let complete_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SstvEvent::ImageComplete { partial: false, .. }))
        .map(|(i, _)| i)
        .collect();

    assert_eq!(
        vis_positions.len(),
        2,
        "expected 2 VisDetected{{Pd120}} in one process() call, got {} (event count {})",
        vis_positions.len(),
        events.len()
    );
    assert_eq!(
        complete_positions.len(),
        2,
        "expected 2 ImageComplete{{partial:false}} in one process() call, got {}",
        complete_positions.len()
    );
    // Order: VisDetected1 < ImageComplete1 < VisDetected2 < ImageComplete2.
    assert!(
        vis_positions[0] < complete_positions[0]
            && complete_positions[0] < vis_positions[1]
            && vis_positions[1] < complete_positions[1],
        "events out of order: vis@{vis_positions:?} complete@{complete_positions:?}"
    );
}
```

- [ ] **Step 2: Register the test in `Cargo.toml`**

After the existing `[[test]] name = "unknown_vis" ...` block (or anywhere among the `[[test]]` entries), add:

```toml
[[test]]
name = "multi_image"
required-features = ["test-support"]
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --features test-support --test multi_image`
Expected: FAIL — `decoder_decodes_two_back_to_back_images` panics at `assert_eq!(vis_positions.len(), 2, ...)` with `1` (the current code `break`s after image 1, so the single `process()` call returns only image 1's `VisDetected` + `LineDecoded` ×N + `ImageComplete`). It must *compile* — it uses only existing public API + `__test_support` helpers (`synth_vis`, `encode_pd`) already used by `tests/roundtrip.rs`.

- [ ] **Step 4: Commit**

```bash
git add tests/multi_image.rs Cargo.toml
git commit -m "test(decoder): failing back-to-back two-image streaming test (#90)"
```

---

## Task 2: Decoder fix — carry-back slice + `continue`, `MULTI_IMAGE_CARRYBACK_LINES`, doc fix, CHANGELOG

Make the test pass: re-arm the VIS detector on only the tail of the decoded image audio, and re-enter the loop instead of bailing.

**Files:**
- Modify: `src/decoder.rs`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `MULTI_IMAGE_CARRYBACK_LINES` constant**

In `src/decoder.rs`, immediately after `const FINDSYNC_AUDIO_HEADROOM: f64 = 1.00;`, add:

```rust
/// How many scan lines of the *just-decoded* image audio to keep when
/// re-arming the VIS detector after `ImageComplete` (issue #90 D4). A
/// back-to-back transmission's VIS leader starts right after the image's
/// last line; carrying back this many lines absorbs a fast transmitter
/// clock (4 lines ≈ 0.8 % of a PD240's airtime, ~1.7 % of Robot24's — far
/// more than any real clock error, <0.1 %) so the leader is always inside
/// the carry-forward window. For a single transmission this is just the
/// image's last few lines plus trailing silence; the fresh detector finds
/// nothing and waits for more audio.
const MULTI_IMAGE_CARRYBACK_LINES: u32 = 4;
```

- [ ] **Step 2: Rewrite the post-image transition**

In `SstvDecoder::process`, the `State::Decoding(d)` arm, the block currently is (right after `Self::run_findsync_and_decode(d, &mut self.pd_demod, &mut self.snr_est, &mut out);`):

```rust
                    // Image complete. Preserve trailing audio not consumed —
                    // it may contain the leading edge of a follow-up VIS
                    // burst (ARISS multi-image case). Feed it into a fresh
                    // VIS detector so the next process() call sees it.
                    //
                    // V2: After ImageComplete, this decoder re-enters
                    // AwaitingVis automatically (continuous monitoring).
                    // For true multi-image streams (back-to-back transmissions
                    // on the same connection) the trailing audio here is fed
                    // into a fresh VisDetector, so the next VIS burst is
                    // detected without any caller intervention. Closes #31.
                    let trailing = std::mem::take(&mut d.audio);
                    self.state = State::AwaitingVis;
                    Self::restart_vis_detection(
                        &mut self.vis,
                        self.working_samples_emitted,
                        &trailing,
                    );
                    break;
```

Replace the whole block with:

```rust
                    // Image complete. Re-arm VIS detection in place — a
                    // back-to-back transmission's VIS leader starts right
                    // after this image's last line, so feed the fresh
                    // detector only the tail of the image audio (a few lines,
                    // to absorb a fast TX clock) plus everything past it; the
                    // rest is decoded video tones a VIS burst can't hide in.
                    // `continue` (not `break`) so that next VIS — and the
                    // image after it — surface in this same `process()` call,
                    // mirroring the known/unknown-code branches above. Closes
                    // #31; #90 (A2 + D4). (`sample_offset` on detections after
                    // the first is relative to the carry-forward start, not
                    // absolute — #99.)
                    let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
                    let carryback = (f64::from(MULTI_IMAGE_CARRYBACK_LINES)
                        * d.spec.line_seconds
                        * work_rate) as usize;
                    let carry_from = d.target_audio_samples.saturating_sub(carryback);
                    Self::restart_vis_detection(
                        &mut self.vis,
                        self.working_samples_emitted,
                        &d.audio[carry_from..],
                    );
                    self.state = State::AwaitingVis;
                    remaining = &[]; // already folded into d.audio → now inside the fresh detector
                    continue;
```

Notes for the implementer:
- `carry_from <= d.target_audio_samples <= d.audio.len()` always (we only reach this code when `d.audio.len() >= d.target_audio_samples`), so `&d.audio[carry_from..]` is always a valid (possibly empty) slice — no bounds-check guard needed.
- Borrow check: `Self::restart_vis_detection(&mut self.vis, self.working_samples_emitted, &d.audio[carry_from..])` — `&mut self.vis` + read of `self.working_samples_emitted` + immutable reborrow of `d.audio` (which borrows `self.state` via `d`) — disjoint fields, and `restart_vis_detection` is an associated fn (not `&mut self`). It compiles; this is the same shape the block already used. After it returns, the `&d.audio[..]` borrow is dead, so `self.state = State::AwaitingVis;` (which drops the `Box<DecodingState>`) is fine.
- If clippy flags the `(... * ... * ...) as usize` chain for precision/sign loss: the enclosing `process` already carries `#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]`, so it should be covered — do not add a new `#[allow]`.

- [ ] **Step 3: Fix the `working_samples_emitted` field doc**

In `src/decoder.rs`, the `working_samples_emitted` field's doc comment currently contains this paragraph:

```rust
    /// This is intentional: `DetectedVis::end_sample` is computed directly
    /// from `total_samples_consumed` and `buffer.len()` inside
    /// `VisDetector::process` at the moment of detection, so `sample_offset`
    /// in `SstvEvent::VisDetected` is always correct. The counter here is
    /// only used to advance the VIS detector's anchor on each chunk; it does
    /// not gate any decode logic.
```

Replace that paragraph with:

```rust
    /// The counter is used to anchor the VIS detector each time a fresh one
    /// is constructed (initial, post-image, post-unknown-VIS). Note: for the
    /// *first* detection on a freshly-built decoder, `DetectedVis::end_sample`
    /// (→ `SstvEvent::VisDetected.sample_offset` / `UnknownVis.sample_offset`)
    /// is an absolute working-rate index from sample 0. After a *restart*
    /// (post-image — see `restart_vis_detection` — or post-unknown-VIS) the
    /// fresh detector counts hops from 0, so a later detection's `sample_offset`
    /// is relative to where the carry-forward audio began, not absolute. That
    /// is acceptable — `sample_offset` is informational only — and tracked in
    /// issue #99 (fixing it needs a `VisDetector` API change).
```

(Leave the rest of the field doc — the "Informational only" paragraph about the counter not being decremented, the "If mid-image VIS detection is ever re-activated" paragraph, and the `Closes #29 and #34` line — unchanged.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --features test-support --test multi_image`
Expected: PASS — `decoder_decodes_two_back_to_back_images` green (2 `VisDetected{Pd120}` + 2 `ImageComplete{partial:false}`, in order).

Then run the fast suites (skip the ~9-min debug `roundtrip` — the full release suite runs in T3):
Run: `cargo test --features test-support --lib --bins --test unknown_vis --test no_vis`
Expected: PASS — `decoder::tests` and the other integration tests are unaffected by the post-image-transition change.

- [ ] **Step 5: Add the `CHANGELOG.md` `[Unreleased]` entries**

In `CHANGELOG.md`, under the `## [Unreleased]` header, add (merging into existing `### Fixed` if one is already present from a prior unreleased change; add `### Performance` if absent):

```markdown
### Fixed

- **Multi-image streams now decode all images within a single `process()` call.**
  After `ImageComplete`, `SstvDecoder::process` re-enters VIS detection in-place
  (`continue`, not `break`), so a back-to-back transmission's next VIS — and the
  image after it — surface in the same call instead of requiring further
  `process()` calls (#90 A2). Note: `sample_offset` on detections *after* the
  first is relative to the carry-forward start, not absolute (#99).

### Performance

- After `ImageComplete`, the VIS detector is re-armed on only the last few scan
  lines of the decoded image audio (plus everything past), not the entire
  ~1.4–3 M-sample buffer — no more FFT-scanning ~10⁴ sliding-window hops of
  decoded video tones for a VIS burst that can't be there (#90 D4).
```

- [ ] **Step 6: Commit**

```bash
git add src/decoder.rs CHANGELOG.md
git commit -m "fix(decoder): decode all images per process() call; re-arm VIS on the tail only (#90 A2/D4)"
```

---

## Task 3: Full CI gate

**Files:** none (verification + any lint fixes)

- [ ] **Step 1: `cargo fmt`**

Run: `cargo fmt --all`
Then: `cargo fmt --all -- --check`
Expected: PASS (no diff). If `cargo fmt` reformatted anything, it's just whitespace from the new code.

- [ ] **Step 2: `cargo clippy`**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS. If a warning fires (e.g. `clippy::doc_markdown` on an identifier in a doc comment), fix it minimally — backtick-wrap, etc. Do **not** add blanket `#[allow]`s.

- [ ] **Step 3: `cargo test` (release, all features, locked)**

Run: `cargo test --all-features --locked --release`
Expected: PASS — full suite green, including `tests/multi_image.rs`, `tests/roundtrip.rs`, `tests/unknown_vis.rs`, `tests/no_vis.rs`.

- [ ] **Step 4: `cargo doc` (deny warnings)**

Run: `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
Expected: PASS — no broken intra-doc links / rustdoc warnings (the edited `working_samples_emitted` doc references `restart_vis_detection`, `SstvEvent::VisDetected`, etc. — `VisDetector::process` / `restart_vis_detection` are private, so their docs aren't link-checked; `SstvEvent::VisDetected` is in `crate::decoder`, reachable).

- [ ] **Step 5: Commit any fixes**

If Steps 1-4 produced changes:

```bash
git add -A
git commit -m "chore: satisfy fmt/clippy/doc gate (#90)"
```

If nothing changed, skip the commit. The branch is now ready for the final review and a PR against `main`.

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:** A2 (`continue` + `remaining = &[]`) → T2 Step 2; D4 (`carry_from` slice + `MULTI_IMAGE_CARRYBACK_LINES`) → T2 Steps 1-2; `working_samples_emitted` doc fix → T2 Step 3; integration test → T1; CHANGELOG → T2 Step 5; CI gate → T3.
- **Why the test fails on the current code:** the current arm `break`s after `ImageComplete`, so a single `process()` call returns only image 1's events; image 2's `VisDetected`/`ImageComplete` only come back on later calls. With `continue`, the fresh detector (fed the carry-forward, which includes VIS 2 + image 2's video) surfaces VIS 2 immediately → image 2 decodes in the same call.
- **`remaining = &[]` is load-bearing:** the caller's `working` slice was folded into `d.audio` (`d.audio.extend_from_slice(remaining)` earlier in the arm) and `&d.audio[carry_from..]` (which includes all of it, since `working` sits past `target` ≥ `carry_from`) is now inside the fresh detector. Without clearing `remaining`, the next `State::AwaitingVis` iteration would re-feed it (double-process).
- **Out of scope (do not touch):** the `sample_offset`-absolute-vs-relative fix (#99); the mid-image-VIS-detection TODO comment in the `State::Decoding` arm (a *new* VIS arriving *during* decoding — unrelated); `FINDSYNC_AUDIO_HEADROOM`; any wider `decoder.rs` decomposition.
- **No public API change** — `SstvEvent`, `SstvDecoder`, `Error` are untouched; the change is internal to `process` plus a private const and a doc comment.
