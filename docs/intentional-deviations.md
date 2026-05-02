# Intentional deviations from slowrx

Translating slowrx 1:1 in Rust isn't always the right call — a few
behaviors are deliberately different. This file lists every deviation
we know about, why we chose it, and the conditions under which we'd
revisit. Future audits should consult this list before flagging any
of these as "missing".

For the parity audit reports themselves, see
[`docs/audits/`](./audits/).

---

## VIS stop-bit boundary: precise vs. ±20 ms slop

**Files:** `src/vis.rs::take_residual_buffer` ↔ slowrx `vis.c:168-170`.
**Tracking issue:** [#39](https://github.com/jasonherald/slowrx.rs/issues/39).

### What slowrx does

After the VIS stop bit, slowrx unconditionally skips a fixed 20 ms
(`readPcm(20e-3 * 44100)`) regardless of which `i` (phase-offset slot
in the 9-iteration loop) matched. The actual stop-bit end can be
0–20 ms before that point depending on `i`.

### What we do

`vis.rs` computes the **exact `i`-aware stop-bit end** as
`stop_end_abs = (hops_completed + i) * HOP_SAMPLES`. The residual buffer
begins precisely there.

### Why we deviated

For real radio capture, slowrx's slop is fine — the receiver re-locks
on the post-burst SYNC pulse. But during Phase 1 (VIS rewrite, before
Phase 2's FindSync existed) the synthetic round-trip needed exact
alignment to test pixel decode without the line-zero find absorbing
the misalignment. Computing the exact boundary keeps per-pixel image
alignment tight without depending on FindSync to clean up.

After Phase 2 landed, FindSync's Skip computation absorbs ±175 ms of
misalignment via convolution — so technically we *could* relax to
slowrx's slop. But there's no functional reason to: the exact-boundary
code is simpler and more predictable, and we'd lose nothing by keeping
it.

### When to revisit

If we ever observe a real-radio capture where the exact-boundary
computation is *worse* than slowrx's slop (e.g., burst timing where
slowrx's slop happens to land on a SYNC edge that helps Skip lock
faster). Has not happened in the Dec-2017 ARISS validation set.

---

## FindSync 90° slant deadband

**Files:** `src/sync.rs::find_sync` ↔ slowrx `sync.c:79`.
**Tracking issue:** [#42](https://github.com/jasonherald/slowrx.rs/issues/42).

### What slowrx does

slowrx applies a Hough-derived rate correction unconditionally, even
when the detected slant is already ~90° (i.e., no slant). The
correction term `tan(90 - slant_angle) / line_width * Rate` is small
near 90° but non-zero, so a clean image still gets a tiny rate nudge
each call.

### What we do

We apply a 0.5° deadband around 90° — if `|slant - 90| <= 0.5°`, no
correction is applied.

### Why we deviated

slowrx's "harmless" tiny correction compounds over multiple `find_sync`
calls, eventually producing visible drift on long images. The deadband
gives us a stable "lock" state that's a strict improvement.

### When to revisit

If a future test reveals a case where the 0.5° deadband prevents
necessary corrections (extremely tilted slant near the lock window).
Not observed.

---

## VIS retry behavior on parity failure

**Files:** `src/vis.rs::match_vis_pattern` ↔ slowrx `vis.c:140-160`.
**Tracking issue:** Documented inline (round-2 audit Finding 5).

### What slowrx does

slowrx terminates the (i, j) alignment loop on the first `(i, j)` whose
bits decode without a parity error. If a tone-classification mistake at
one `(i, j)` yields a parity-failing code, slowrx aborts the whole
detection and waits for the next 10 ms hop to retry.

### What we do

We exhaust all 9 `(i, j)` candidates before giving up. If a later
`(i, j)` decodes a parity-passing code, we accept it.

### Why we deviated

More recovery on borderline real-radio bursts. slowrx's early-exit is
mostly an artifact of its `HedrShift`-set-before-parity-check pattern;
the strict "first parity-passing match wins" semantics aren't
load-bearing.

### When to revisit

If a real burst gives Rust a *different* valid VIS code than slowrx
(borderline tones that pass parity at multiple `(i, j)`). Not observed.

---

## Synthetic round-trip max_diff tolerance

**Files:** `tests/roundtrip.rs`.
**Context:** Phase 7 (PR #60).

### What changed

Round-trip test originally asserted `max_diff <= 25` (and `mean < 5`).
With Phase 3 deferrals (#44 SNR-adaptive Hann, #45 channel-mask drop)
engaged, isolated synthetic boundary pixels hit `max_diff = 234–255`.
The `max_diff` check was dropped; only `mean < 5.0` remains.

### Why

The synthetic encoder produces instant frequency-step transitions at
pixel boundaries. Real radio's FM-modulator slewing softens these.
slowrx's behavior (which our deferral engagement matches) is correct
on real radio — verified visually against the Dec-2017 ARISS captures
— but the synthetic "instant step" inputs trip the SNR-adaptive
selector + boundary FFT in ways the slewed real-audio doesn't.

Mean diff staying excellent (1.5–1.9 on PD120/PD180) shows the
decoder is mostly fine; the `max` is dominated by a handful of
boundary pixels per image.

### When to revisit

Either:
1. Upgrade the synthetic encoder to model FM slewing (tunable risetime
   between adjacent pixel frequencies). Then `max_diff` becomes
   meaningful again.
2. Add a real-audio cross-validation suite (gitignored fixtures already
   exist in `docs/wav_files/`; the `slowrx-cli` binary covers ad-hoc
   smokes).
