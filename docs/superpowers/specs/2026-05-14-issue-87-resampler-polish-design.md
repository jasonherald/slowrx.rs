# Issue #87 — Resampler polish — design

**Issue:** [#87](https://github.com/jasonherald/slowrx.rs/issues/87) (audit bundle 3 of 12 — IDs D1, D2, D2b, F6)
**Source of record:** `docs/audits/2026-05-11-deep-code-review-audit.md`
**Scope:** all-in-one cleanup of `src/resample.rs`. Behavior change for the polyphase quantization (D2, ≈ −52 dB phase noise on a 2300 Hz tone); doc-only fixes for the cutoff formula and group delay (D2b); off-by-one bug fix in `needed_end` (D2b); five new tests (F6). D1 turned out to be a phantom finding (verified during T1) — kept as a regression-guarding test; no code change needed.

## Background

`src/resample.rs` is the input-rate → 11025 Hz working-rate FIR resampler. The audit flagged four issues:

- **D1 — unit-gain miss** *(phantom finding, see note below)*. The audit claimed the 64 Hann-windowed sinc taps were accumulated raw with sum-of-taps ≈ 0.5 → ~6 dB attenuation. **Empirically verified false** during T1's TDD-red step: the `exact_rate_preserves_amplitude_and_no_attenuation` test passes on the current code with `output_peak / input_peak ≈ 1.0`. The audit author appears to have confused the Hann *window*'s mean (= 0.5 by definition) with the Hann-*windowed-sinc*'s DC gain (= ~1.0 because the sinc's `2·fc · sin(2π·fc·n)/(2π·fc·n)` form already has unity passband gain). **D1 is therefore docs/test-only**: we keep the amplitude test added by F6 as a regression guard, but no normalization is needed in the polyphase build (the raw windowed-sinc taps already sum to ~1.0).
- **D2 — "polyphase" misnomer + per-output-sample transcendentals.** Module/struct docs say "polyphase FIR" but `process` recomputes all 64 taps fresh each output sample (64 × `.sin()` for the sinc plus 64 × `.cos()` for the Hann = ~128 transcendentals per output × 11025 outputs/sec ≈ 1.4 M/sec). An inline comment on line 76 even admits "true fractional-delay FIR rather than the quantized integer-delay version" — contradicting "polyphase."
- **D2b — three doc/logic bugs:**
  - `cutoff_hz` doc says `min(input/2, working/2) × 0.45` (Nyquist form) but the code is `min(input, working) × 0.45 cap 4500` (full-rate form). The code is correct (`0.9 × Nyquist` of the lower rate); the doc is wrong and would imply a cutoff below 2300 Hz, which would break SSTV.
  - The ~31.5-sample group delay (`(FIR_TAPS - 1) / 2`) is undocumented. Inside the decoder `find_sync` absorbs it; standalone consumers deserve to know.
  - `needed_end = (center + half).ceil() as usize` over-reserves by one sample when `phase` is fractional (e.g. `phase = 100.5` → `ceil(164.5) = 165` but the kernel only reads indices `100..164`).
- **F6 — missing tests.** No exact-rate amplitude test (would have caught D1); no upsampling test (`stride < 1`); no 192 kHz max-rate test; no tiny-chunks streaming-buffer test; no empty-input test.

## Design

### Part 1 — Polyphase tap bank (D2)

**Struct shape:**

```rust
const FIR_TAPS: usize = 64;
const NUM_PHASES: usize = 256;

pub struct Resampler {
    input_rate: u32,
    stride: f64,
    phase: f64,
    tail: Vec<f32>,
    /// 256-phase polyphase tap bank, indexed by `frac` quantized to 1/256
    /// sub-sample. Built once in `new()` (~64 KB, static for the
    /// resampler's lifetime). Each row is a Hann-windowed sinc at the
    /// corresponding fractional delay. Raw taps (no normalization
    /// pass) — the windowed-sinc form already sums to ~1.0 at typical
    /// `fc`; verified by the `exact_rate_preserves_amplitude_and_no_attenuation`
    /// test.
    taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>,
}
```

Memory: 256 × 64 × 4 B = **64 KB** per `Resampler`. `SstvDecoder` holds one — negligible.

**Build the bank in `new()`:**

```rust
let cutoff_norm = cutoff_hz(input_rate) / f64::from(input_rate);
let mut taps = Box::new([[0.0_f32; FIR_TAPS]; NUM_PHASES]);
for phase_idx in 0..NUM_PHASES {
    let frac = (phase_idx as f64) / (NUM_PHASES as f64);
    for k in 0..FIR_TAPS {
        taps[phase_idx][k] = fir_tap(k, frac, cutoff_norm);
    }
}
```

The existing `fir_tap(tap_index, frac, fc) -> f32` fn stays — it's still the per-tap formula, now called only during construction. The runtime hot path no longer invokes it. No D1 normalization step (the audit's D1 was a phantom finding; raw taps already sum to ~1.0).

**`process()` hot path** becomes:

```rust
loop {
    // D2b — off-by-one fix: was `(center + half).ceil() as usize` which
    // over-reserves by one sample for fractional phase. `floor(phase) + 64`
    // is the exact requirement.
    let needed_end = (self.phase.floor() as usize) + FIR_TAPS;
    if needed_end > buf.len() { break; }

    let frac = self.phase.fract();
    let phase_idx = ((frac * NUM_PHASES as f64).round() as usize).min(NUM_PHASES - 1);
    let taps = &self.taps[phase_idx];
    let start = self.phase.floor() as isize;

    let mut acc: f32 = 0.0;
    for k in 0..FIR_TAPS {
        let idx = start + k as isize;
        if (0..buf.len() as isize).contains(&idx) {
            acc += taps[k] * buf[idx as usize];
        }
    }
    out.push(acc);
    self.phase += self.stride;
}
```

No transcendentals in the hot path; 64 MACs per output sample. The `center` / `half` locals from the old code go away.

**Quantization noise** at 256 phases: max sub-sample position error = 1/512 sample ≈ 177 ns at 11.025 kHz. For a 2300 Hz tone, phase-noise contribution ≈ −52 dB — well below SSTV's noise floor on real radio. No lerp between adjacent phases for the first cut; the F6 amplitude test will catch anything bigger.

**Module-level doc** updates the opening paragraph from "Hand-rolled 64-tap Hann-windowed-sinc polyphase FIR" to **"Hand-rolled 64-tap Hann-windowed-sinc polyphase FIR with 256 phase positions"** — "polyphase" is now honestly described (the per-output-sample tap recomputation is gone).

### Part 2 — D2b: three doc/logic fixes

**`cutoff_hz` doc/code mismatch.** Replace the existing doc comment with:

```rust
/// Cutoff frequency (Hz) for the resampler, derived from the input rate.
/// `min(input_rate, WORKING_SAMPLE_RATE_HZ) × 0.45`, hard-capped at 4500 Hz —
/// i.e. ~0.9 × Nyquist of the lower of the two rates. The cap keeps the
/// passband from extending past SSTV's 2300 Hz video band at typical input
/// rates (44.1k → 4961 Hz uncapped → 4500 capped).
fn cutoff_hz(input_rate: u32) -> f64 { /* unchanged */ }
```

(The code stays as-is; only the doc changes.)

**Group delay documented.** Append to the `Resampler` struct's doc comment:

```rust
/// **Group delay:** the 64-tap symmetric FIR has linear-phase group delay
/// of `(FIR_TAPS - 1) / 2 = 31.5` input-rate samples (≈ 715 µs at 44.1 kHz,
/// ≈ 2.86 ms at 11.025 kHz). Output is shifted right by this amount
/// relative to input. SSTV's `find_sync` re-anchors the rate against sync
/// pulses, so this is invisible inside the decoder pipeline; standalone
/// consumers should compensate if they need sample-accurate alignment.
```

**`needed_end` off-by-one** — folded into the Part 1 hot-path rewrite (same line; one fix).

**Dead-branch acknowledgment** — the `else { self.tail.clear(); self.phase -= buf.len() as f64; }` at the bottom of `process()` is provably unreachable under `MAX_INPUT_SAMPLE_RATE_HZ`. Keep it as defensive code, add a one-line comment:

```rust
// Unreachable under MAX_INPUT_SAMPLE_RATE_HZ — defensive against a
// future cap relaxation. (#87 D2b acknowledgment.)
} else {
    self.tail.clear();
    self.phase -= buf.len() as f64;
}
```

### Part 3 — F6: five new tests

All added to the existing `#[cfg(test)] mod tests` in `src/resample.rs`. The existing tests stay (they exercise Goertzel-ratio properties that the refactor preserves).

1. **`exact_rate_preserves_amplitude_and_no_attenuation`** — the D1 regression guard. `Resampler::new(WORKING_SAMPLE_RATE_HZ)` (stride = 1.0, every output sample has `frac = 0`). Feed 200 samples of a 1500 Hz tone at amplitude 0.8 (above the 64-tap kernel ramp-up). Assert output peak amplitude is within ±5 % of input peak. **Passes on current pre-#87 code** (the audit's D1 claim turned out to be wrong — see Background note). Kept as a regression guard so a future tap-formula change can't silently introduce attenuation.

2. **`upsampling_8khz_to_11025`** — `Resampler::new(8000)`. `stride ≈ 0.726` — fractional `frac` varies on every output sample, exercising the polyphase lookup path. Feed 8000 samples of a 1500 Hz tone; assert (a) output length ≈ 11025 ± 64, and (b) Goertzel power at 1500 Hz dominates neighboring bins (signal preserved across the rate change).

3. **`max_input_rate_192khz`** — `Resampler::new(192_000)`. `stride ≈ 17.41` — many input samples per output. Feed 96 000 samples of a 2000 Hz tone (0.5 s); assert no panic, output length ≈ 5512 ± 64, Goertzel power at 2000 Hz dominates.

4. **`tiny_chunks_emit_nothing_then_catch_up`** — `Resampler::new(44100)`. Call `process(&buf)` repeatedly with 3-sample chunks. Assert every call returns `Vec::new()` until cumulative input reaches `FIR_TAPS = 64` samples; the next call after that starts emitting. (Verifies the `tail` carry-over correctness across many small calls — the production decoder's streaming path depends on this.)

5. **`empty_input_returns_empty`** — `Resampler::new(44100).process(&[])` → `vec![]`. Plus: an empty call sandwiched between two non-empty calls is a no-op (state unchanged).

## Files touched

`src/resample.rs` (the entire change), `CHANGELOG.md` (`[Unreleased]` `### Internal` bullet).

## CHANGELOG entry

> **Resampler polish** — `Resampler` now uses a true 256-phase polyphase tap bank (precomputed in `new()`, ~64 KB) instead of recomputing all 64 Hann-windowed-sinc taps per output sample; eliminates ~1.4 M `sin()`+`cos()` calls/sec from the hot path. Off-by-one in `needed_end` (was `ceil(phase + 64)` causing one extra sample of buffering; now `floor(phase) + 64`) fixed. Group delay documented (`(FIR_TAPS - 1) / 2 = 31.5` input-rate samples). `cutoff_hz` doc corrected (the Nyquist factor was wrong in the doc; the code was right). Five new tests (`exact_rate_preserves_amplitude_and_no_attenuation`, `upsampling_8khz_to_11025`, `max_input_rate_192khz`, `tiny_chunks_emit_nothing_then_catch_up`, `empty_input_returns_empty`). The audit's D1 ("FIR taps not normalized to unit DC gain → ~6 dB attenuation") turned out to be a phantom finding — verified during T1, taps already sum to ~1.0; the amplitude test stays as a regression guard. Quantization noise at 256 phases is ≈ −52 dB phase noise on a 2300 Hz tone — well below SSTV's noise floor. (#87; audit D2/D2b/F6.)

## Verification

Full local CI gate. The `cargo test --release` step is the load-bearing one — `tests/roundtrip.rs` decodes one image per mode through the resampler (any drift in polyphase quantization surfaces as a pixel-diff regression), the existing `resample::tests` Goertzel suite still passes (it's preservation-of-power-ratio, unaffected by the polyphase quantization), the 5 new F6 tests cover the audit gaps.

- `cargo test --all-features --locked --release`
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`

## Out of scope

- **Lerp between adjacent phases.** Quantization at 256 phases is already ≈ −52 dB phase noise on a 2300 Hz tone — far below audible / SSTV's noise floor. Add later if F6's amplitude test ever reveals a real issue.
- **SIMD-ifying the 64-tap inner loop.** Tracked in #77 (the post-experiment finding: function-level `multiversion` wrap is a no-op for rustfft-style hot loops; would need profile-driven targeting). Separate from #87.
- **Remaining audit cleanups:** #88 (find_sync), #91 (ModeSpec), #92 (API hygiene), #93 (perf), #94 (docs), #95 (CI), #96 (standalone).
