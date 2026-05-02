# slowrx ↔ slowrx.rs Side-by-Side Parity Audit

## Section 1 — Coverage table (mandatory enumeration)

### `common.h` / `common.c` (utilities & constants)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `common.h::MINSLANT 30`, `MAXSLANT 150` | NOT TRANSLATED | NOT-PORTED | Hough transform constants for sync.c slant correction. Rust port has no slant correction. |
| `common.h::BUFLEN 4096` | NOT TRANSLATED | NOT-PORTED | C's hard-coded PCM ring-buffer length. Rust uses streaming `Vec<f32>` — no fixed buffer. |
| `common.h::SYNCPIXLEN 1.5e-3` | NOT TRANSLATED | NOT-PORTED | Sync pixel duration; only referenced by sync.c. |
| `common.h::CurrentPic.HedrShift` (gshort) | NOT TRANSLATED | NOT-PORTED | **Critical: radio mistuning offset**. C carries this through entire pipeline. Rust has no equivalent. |
| `common.h::CurrentPic.Mode/Rate/Skip` | `decoder::State::Decoding{mode, spec, line_pair_index, ...}` | DIVERGE | Rust does not track `Rate` or `Skip` (no slant/skip refinement). |
| `common.h::HasSync` array | NOT TRANSLATED | NOT-PORTED | Sync band power tracking buffer. |
| `common.h::StoredLum` array | NOT TRANSLATED | NOT-PORTED | Cached luminance per sample for redraw. Rust does not redraw — single-pass. |
| `common.h::Adaptive` flag | NOT TRANSLATED | NOT-PORTED | **Adaptive FFT-window flag.** Rust uses fixed FFT_LEN=256. |
| `common.h::ModeSpec[]` array indexed by enum | `modespec.rs::for_mode/lookup` | DIVERGE-IN-COVERAGE | Rust ports only PD120 and PD180. C has all 25+ modes. |
| `common.c::GetBin(Freq, FFTLen)` | inlined `bin_for(hz)` closure in `mode_pd::pixel_freq`, also inlined in `vis::goertzel_power` | MATCH (semantically) | Rust uses `WORKING_SAMPLE_RATE_HZ=11025` rather than C's hard-coded `44100`. |
| `common.c::power(coeff)` | inlined `power(c)` closure in `mode_pd::pixel_freq` | MATCH | `r*r + i*i`. |
| `common.c::clip(a)` | `mode_pd::freq_to_luminance` clamp + `ycbcr_to_rgb` clamp | DIVERGE-IN-SEMANTICS | C does `(guchar)round(a)`; Rust does `as u8` (truncation). See Finding 4. |
| `common.c::deg2rad/rad2deg` | NOT TRANSLATED | NOT-PORTED | sync.c-only helpers. |
| `common.c::ensure_dir_exists/saveCurrentPic` | NOT TRANSLATED | NOT-PORTED | I/O — out of crate scope. |
| `common.c::evt_*` GTK handlers | NOT TRANSLATED | NOT-PORTED | UI layer. |

### `vis.c` (VIS detection)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `vis.c::Hann[882]` window (20 ms) | `vis.rs::goertzel_power` (no Hann window!) | DIVERGE | **Critical: no windowing.** See Finding 1. |
| `vis.c::FFTLen=2048`, `Plan2048` | `vis.rs::goertzel_power` (no FFT) | DIVERGE | C uses 2048-pt FFT looking for max bin in 500-3300 Hz; Rust uses 4 fixed Goertzels at 1900/1200/1100/1300 Hz. |
| `vis.c::readPcm(441)` (10 ms hop, sliding) | `vis.rs::process` 30 ms boundary windows | DIVERGE | **Critical: no overlap.** See Finding 2. |
| `vis.c::HedrBuf[100]` (45-window history) | `vis.rs::tones[14]` (14-window history) | DIVERGE | C remembers ~450 ms (45 × 10 ms hops) and pattern-matches with offset `i` and reference `j`; Rust matches on rigid 14 × 30 ms slots. |
| `vis.c::Gaussian-interpolated peak` (lines 63-66) | NOT TRANSLATED | NOT-PORTED | C extracts continuous frequency; Rust doesn't (no need with Goertzel). |
| `vis.c::tone[i]` storage in Hz | `vis.rs::Tone` enum (categorical) | DIVERGE | C stores continuous frequency in Hz; Rust stores 5-way enum classification. |
| `vis.c::±25 Hz tolerance` (lines 85-91) | `vis.rs::classify` 5× power dominance | DIVERGE | **Critical: relative-vs-absolute tolerance.** See Finding 3. |
| `vis.c::HedrShift = tone[0+j] - 1900` (line 106) | NOT TRANSLATED | NOT-PORTED | **Critical: radio mistuning measurement.** See Finding 5. |
| `vis.c::3-position phase scan (i=0..2)` | NOT TRANSLATED | NOT-PORTED | C scans i=0,1,2 to handle window misalignment by ±10 ms. Rust has no such scan. |
| `vis.c::3-leader scan (j=0..2)` | NOT TRANSLATED | NOT-PORTED | C uses any of 3 leader windows as the reference for tolerance; Rust requires absolute match. |
| `vis.c::Parity check + R12BW special-case` | `vis.rs::match_vis_pattern` parity check | MATCH (partial) | Rust does parity, but R12BW negation is not present (mode not implemented). |
| `vis.c::VISmap[VIS] lookup` | `modespec.rs::lookup` | MATCH (partial) | Only PD120/PD180 — see modespec.rs section. |
| `vis.c::"Skip the rest of the stop bit" 20 ms read` (line 169) | `decoder::process` consumes residual buffer | DIVERGE | C explicitly skips 20 ms after stop bit; Rust hands trailing window contents to PD decoder as residual. |
| `vis.c::ManualActivated/ManualResync` | NOT TRANSLATED | NOT-PORTED | UI flags. |

### `modespec.c` (mode timing tables)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `ModeSpec[PD120]` (lines 260-271) | `modespec.rs::PD120` const | **MISSING-FIELD** | Rust omits `septr_seconds` (C: 0e-3, used in PD ChanStart calculations). |
| `ModeSpec[PD180]` (lines 286-297) | `modespec.rs::PD180` const | **MISSING-FIELD** | Same omission. |
| `ModeSpec.SyncTime/PorchTime/PixelTime/LineTime` | matching fields | MATCH | Numeric values verified row-by-row. |
| `ModeSpec.SeptrTime` for PD = 0e-3 | NOT TRANSLATED | NOT-PORTED | PD ChanStart uses `+ SeptrTime` between channels; for PD it's 0 so omission is functionally OK, but spec field is missing (would break if non-PD mode added). |
| `ModeSpec.LineHeight` | NOT TRANSLATED | NOT-PORTED | Used by C's PNG saver and click-to-set-edge handler. Not needed in Rust pixel buffer. |
| `ModeSpec.ColorEnc=YUV` for PD | `modespec.rs::ChannelLayout::PdYcbcr` | MATCH | |
| `VISmap[]` 0x5F → PD120 (line 381) | `modespec.rs::lookup` | MATCH | |
| `VISmap[]` 0x60 → PD180 (line 382) | `modespec.rs::lookup` | MATCH | |
| `VISmap[]` for all other modes | `modespec.rs::lookup` returns None | NOT-PORTED-BY-DESIGN | V1 scope. |

### `sync.c` (slant correction)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `FindSync` (whole function) | NOT TRANSLATED | NOT-PORTED | **Important.** No Hough-transform slant correction. See Finding 7. |
| Linear Hough transform (lines 50-69) | NOT TRANSLATED | NOT-PORTED | |
| Sample rate adjustment (line 81) | NOT TRANSLATED | NOT-PORTED | C corrects sample rate by inverse-tangent of slant angle. |
| 8-point convolution sync edge finder (lines 105-113) | NOT TRANSLATED | NOT-PORTED | **Important.** Determines `Skip` (where the line actually starts). See Finding 8. |
| `Skip` calculation (lines 119-127) | `decoder::process` uses pair_index × line_seconds, no skip | DIVERGE | Rust assumes line 0 starts immediately at end of stop bit + residual. |
| Scottie-mode skip adjustment | NOT TRANSLATED | NOT-PORTED | Scottie modes not implemented in V1. |

### `video.c` (per-pixel decode)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `GetVideo` setup (lines 19-50) | `decoder::process` State::Decoding | MATCH | Rust state machine vs C single function. |
| Hann windows of 7 lengths (lines 53-57) | `mode_pd::hann_window` (single 256 length) | DIVERGE | **Critical: no SNR adaptation.** See Finding 6. |
| HannLens[7] = {48, 64, 96, 128, 256, 512, 1024} | NOT TRANSLATED | NOT-PORTED | C adapts FFT window 48..1024 by SNR. |
| Channel time offsets `ChanStart[]` (lines 81-93 PD case) | `mode_pd::chan_starts_sec` | MATCH | Both use sync+porch + n×width×pixel offsets. SeptrTime=0 for PD. |
| `NumChans=4` for PD (lines 117-125) | implicit in `mode_pd::decode_pd_line_pair` (4 channels) | MATCH | |
| PixelGrid construction PD (lines 135-176) | `mode_pd::decode_pd_line_pair` | DIVERGE | Different structure: C pre-plans per-pixel sample times globally; Rust loops within each line pair. See Finding 12. |
| PixelGrid `Time` formula (line 140-142) | Rust per-channel `start_sec * work_rate.round()` + `(x + 0.5) * pixel_secs` | MATCH (semantically) | C: `Rate * (y/2 * LineTime + ChanStart + PixelTime * (x + 0.5))`; Rust: same formula in two parts. |
| `PixelIdx` → first non-negative `Time` skip (lines 211-216) | NOT TRANSLATED | NOT-PORTED | C handles negative skip times (when Skip is negative, PD odd starts before t=0). Rust doesn't have skip. |
| Length calculation (lines 251-254) | `decoder::process` cumulative `cur_off`/`next_off` | DIVERGE | C: total frames × line_time × 44100. Rust: per-pair samples_per_pair from cumulative rounding. |
| `SyncTargetBin = GetBin(1200 + HedrShift)` (line 255) | NOT TRANSLATED | NOT-PORTED | Sync band tracking uses HedrShift; Rust has no sync band tracking. |
| Sync band Praw/Psync power tracking (lines 271-298) | NOT TRANSLATED | NOT-PORTED | **Important.** Used by sync.c for slant detection. |
| `HasSync[SyncSampleNum]` array fill | NOT TRANSLATED | NOT-PORTED | |
| SNR estimation (lines 304-344) | NOT TRANSLATED | NOT-PORTED | **Critical.** Drives `WinIdx` adaptive window selection. See Finding 6. |
| FM demod every 6 samples (line 350) | per-pixel FFT (one per pixel) | DIVERGE | **Important.** C runs sliding FFT every 6 samples (= 7350 Hz analysis rate at 44100). Rust runs FFT only at pixel centers. See Finding 11. |
| Window-size selection by SNR (lines 354-367) | fixed 256 | DIVERGE | See Finding 6. |
| Window centered on `WindowPtr - WinLength/2` (line 375) | `mode_pd::pixel_freq` centered on `center_sample - FFT_LEN/2` | MATCH | Both centered. |
| Peak search 1500-2300 Hz with HedrShift offset (line 382) | `mode_pd::pixel_freq` peak search 1500-2300 Hz, NO HedrShift | DIVERGE | **Critical: missing HedrShift.** See Finding 5. |
| Out-of-band freq fallback to 1500 or 2300 (line 397) | `mode_pd::pixel_freq` returns `max_bin` (no HedrShift, no fallback to 1500/2300) | DIVERGE | C clips to band edges (with HedrShift); Rust returns interpolated bin. |
| Gaussian peak interpolation (lines 391-394) | `mode_pd::pixel_freq` Gaussian interp | MATCH | Same formula. |
| `StoredLum[SampleNum] = clip((Freq - 1500 - HedrShift) / 3.1372549)` (line 406) | `mode_pd::freq_to_luminance(freq)` | DIVERGE | **Critical: missing HedrShift in luminance scale.** See Finding 5 + Finding 4. |
| `Image[x][y][Channel] = StoredLum[SampleNum]` at PixelGrid time (line 421) | `decode_pd_line_pair` per-pixel `pixel_freq` then `freq_to_luminance` | DIVERGE | **Important.** Slowrx uses cached `StoredLum` indexed by sample number; Rust re-runs FFT centered at each pixel. See Finding 11. |
| YUV→RGB matrix (lines 446-451) | `mode_pd::ycbcr_to_rgb` | MATCH | Coefficients verified identical: 100/140/-17850, 100/-71/-33/13260, 100/178/-22695. |
| `Image[x][y+1][Channel]` for R36/R24 (line 425) | NOT TRANSLATED | NOT-PORTED | Robot modes not implemented. |
| `setVU` calls | NOT TRANSLATED | NOT-PORTED | UI. |
| Abort handling | NOT TRANSLATED | NOT-PORTED | Caller's responsibility. |

### `slowrx.c` (`Listen` orchestrator)

| C source | Rust source | Match? | Notes |
|---|---|---|---|
| `Listen()` outer loop | `SstvDecoder::process` driver | MATCH (in spirit) | |
| `GetVIS()` → `GetVideo()` sequencing | `State::AwaitingVis` → `State::Decoding` | MATCH | |
| `CurrentPic.Rate = 44100` (initial) | `WORKING_SAMPLE_RATE_HZ = 11025` | DIVERGE | Different working rate (intentional). |
| First-pass `GetVideo(Mode, 44100, 0, FALSE)` then `FindSync` then second-pass `GetVideo(..., TRUE)` (lines 115-149) | single-pass `decode_pd_line_pair` | DIVERGE | **Important.** Rust skips two-pass slant refinement entirely. See Finding 7. |
| `StoredLum` calloc'd to whole-image size (line 86) | NOT TRANSLATED | NOT-PORTED | Rust streams pixels as decoded. |
| `HasSync` calloc'd (line 93) | NOT TRANSLATED | NOT-PORTED | |
| FSK ID reception (line 127) | NOT TRANSLATED | NOT-PORTED | Out of V1 scope. |
| FFT plan allocation (lines 218-232) | per-`PdDemod` rustfft planner | MATCH | |
| `ManualResync` redraw path (lines 57-67) | NOT TRANSLATED | NOT-PORTED | UI. |

---

## Section 2 — Per-file walk-through

### `common.h` — Read.
A 151-line header defining `_ModeSpec`, `_PicMeta`, the SSTV mode enum, the `FFTStuff`/`PcmData`/`GuiObjs` structs, and externs. Maps to `modespec.rs` (mode enum + spec table — partial, only PD120/PD180), `decoder.rs::State` (subsumes `_PicMeta` partially: Mode is tracked, but `Rate` and `HedrShift` and `Skip` are NOT tracked). `_PicMeta.HedrShift` is the most consequential omission — it threads through `vis.c → video.c` as a frequency offset for SSTV mistuning. The enum covers 25+ modes; only PD120 and PD180 are in the Rust enum.

### `common.c` — Read.
A 215-line file mostly of GTK glue plus three pure helpers: `GetBin`, `power`, `clip`, and `deg2rad/rad2deg`. `GetBin` and `power` are inlined in Rust at their callsites (with the bin formula updated for the 11025 Hz working rate). `clip` is split into `freq_to_luminance` (for grayscale) and `ycbcr_to_rgb`'s clamps. **One semantic divergence**: `clip` calls `round(a)` then casts to `guchar` — Rust uses `as u8` (truncation). `deg2rad`/`rad2deg` are unused in Rust (slant correction not ported). `saveCurrentPic` is out of crate scope.

### `vis.c` — Read.
The 175-line file is **architecturally different from the Rust port**, not just translated. C uses a 2048-point FFT with a 20 ms Hann window, slid every 10 ms (= 882 sample window with 441 sample hops, overlapping 50%), maintaining a 450 ms history buffer (45 frequency samples). Pattern matching scans 3 sub-window offsets × 3 reference-leader positions, with ±25 Hz tolerance relative to the detected leader frequency. Rust replaces all of this with 4 fixed-frequency Goertzel filters at exact 1900/1200/1100/1300 Hz, scanned on rigid 30 ms boundaries (no overlap, no Hann window). Rust loses: HedrShift detection, frequency-tolerance to off-frequency captures, sub-window time alignment, and Hann windowing for spectral leakage suppression.

### `modespec.c` — Read.
A 385-line table-driven file. Only PD120 and PD180 are ported. Field-by-field comparison of those two entries shows numeric values match (SyncTime=20e-3, PorchTime=2.08e-3, PixelTime=0.19e-3 (PD120) / 0.286e-3 (PD180), LineTime=0.50848 / 0.75424, ImgWidth=640, NumLines=496). The Rust `ModeSpec` struct **omits `SeptrTime`**: in C this is 0e-3 for PD modes so it's a no-op in `ChanStart` arithmetic, but the field is structurally absent — adding any non-PD mode would expose it. `LineHeight` and `Name`/`ShortName` are also absent (cosmetic / used only by C's PNG path).

### `sync.c` — Read.
The 133-line file is **completely unported** in Rust. It does two things: (a) Hough-transform-based slant angle detection, with up to 4 retries refining the sample rate, and (b) 8-point convolution sync-edge detection driving the `Skip` value (where line 0 truly starts within the audio). Without these, the Rust port has no defense against sample-rate mismatches (sound card clock drift) or sync-phase misalignment between the VIS stop bit and the first scan line. For real off-air recordings these matter — the C code's design centers `GetVideo` first, then re-runs it after `FindSync` recalculates Rate and Skip.

### `video.c` — Read.
The 491-line core. Three subsystems live here that Rust doesn't have: (1) **HedrShift application**: every `GetBin(... + HedrShift, ...)` call shifts the analysis bands by the radio mistuning offset detected during VIS. (2) **SNR-adaptive Hann window** chooses one of 7 window lengths (48..1024) based on continuously updated SNR. (3) **Sync-band power tracking** writes the `HasSync` array consumed by sync.c. The luminance pipeline itself (`StoredLum[SampleNum] = clip((Freq - 1500 - HedrShift) / 3.1372549)`) is structurally similar to Rust's `freq_to_luminance`, but Rust runs FFT only at pixel-center sample times whereas C runs FFT every 6 samples and indexes `StoredLum` at the pixel times — meaning C effectively low-pass filters luminance over ~136 µs while Rust takes a single instantaneous reading. The PD channel-layout / YUV→RGB code is a clean match.

### `slowrx.c` — Read.
The 256-line `Listen()` thread. Two-pass decode flow: (1) GetVIS, (2) GetVideo with default 44100 Rate and 0 Skip (cached luminance via StoredLum), (3) FindSync to refine Rate/Skip, (4) GetVideo again in `Redraw=TRUE` mode that re-uses the cached luminance with new pixel-time grid. Rust collapses this to a single-pass `decode_pd_line_pair` with no Rate/Skip refinement step. The `Adaptive` checkbox state is read here (passed to GetVideo via global) — Rust has no equivalent toggle (it's effectively `Adaptive=FALSE, WinIdx=0` always, which means C's largest 1024-tap window).

---

## Section 3 — Detailed divergence findings

### Finding 1: VIS detection has no Hann window

**Severity:** Important
**Likely impact on real-audio decoder failure:** Yes
**Files:** `vis.c:30, 47-48` ↔ `src/vis.rs:23-42`

#### C source

```c
// Create 20ms Hann window
for (i = 0; i < 882; i++) Hann[i] = 0.5 * (1 - cos( (2 * M_PI * (double)i) / 881 ) );

// ...

// Apply Hann window
for (i = 0; i < 882; i++) fft.in[i] = pcm.Buffer[pcm.WindowPtr + i - 441] / 32768.0 * Hann[i];

// FFT of last 20 ms
fftw_execute(fft.Plan2048);
```

#### Rust source

```rust
pub(crate) fn goertzel_power(samples: &[f32], target_hz: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = samples.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let k = (0.5 + n * target_hz / f64::from(WORKING_SAMPLE_RATE_HZ)).floor();
    let omega = 2.0 * std::f64::consts::PI * k / n;
    let coeff = 2.0 * omega.cos();

    let mut s_prev = 0.0_f64;
    let mut s_prev2 = 0.0_f64;
    for &sample in samples {
        let s = f64::from(sample) + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2.mul_add(s_prev2, s_prev.mul_add(s_prev, -coeff * s_prev * s_prev2))
}
```

#### Divergence

Slowrx applies a 882-sample Hann window to the time-domain PCM before FFT. This suppresses spectral leakage from the rectangular boxcar implicit in `samples`. Rust's Goertzel filter operates on the raw rectangular-windowed sequence — equivalent to using a `sinc`-shaped (boxcar) frequency-domain response with ~4% peak sidelobes vs. Hann's ~0.4% sidelobes (a 10× difference in nearby-tone rejection). The Rust comment claims "mathematically equivalent for VIS purposes" — that's true for a pure single tone in clean audio but false when nearby spectral energy from speech, hum (60/120 Hz), noise, or adjacent SSTV tones leaks into the bin.

Quantitatively: a real-radio capture has noise + occasional speech in the audio band. The boxcar window's first sidelobe at 13 dB below peak means a noise tone 110 Hz away from 1900 Hz (near `WINDOW_SAMPLES = 330` × frequency-resolution ≈ 33 Hz, so ~3 bins) leaks at -13 dB; with Hann that drops to -32 dB.

#### Why this might cause the real-audio failure (or not)

This is plausible but not the strongest candidate. ARISS captures are known for noise + adjacent transmissions. Without Hann windowing the 5×-dominance threshold could fail to classify any tone (returning `Tone::Other`) when noise leaks into the bin. However, the real failure mode is "0 ImageComplete events on 6/7 captures" — if VIS detection itself were the bottleneck the 1 success would be unexplained too. **More likely contributor than primary cause.**

---

### Finding 2: VIS windows do not overlap (no 10 ms hop)

**Severity:** Critical
**Likely impact on real-audio decoder failure:** Yes
**Files:** `vis.c:40-48, 73, 165` ↔ `src/vis.rs:94-118`

#### C source

```c
while ( TRUE ) {

    if (Abort || ManualResync) return(0);

    // Read 10 ms from sound card
    readPcm(441);

    // Apply Hann window
    for (i = 0; i < 882; i++) fft.in[i] = pcm.Buffer[pcm.WindowPtr + i - 441] / 32768.0 * Hann[i];

    // FFT of last 20 ms
    fftw_execute(fft.Plan2048);
    ...
    // Header buffer holds 45 * 10 msec = 450 msec
    HedrPtr = (HedrPtr + 1) % 45;
    ...
    pcm.WindowPtr += 441;
}
```

#### Rust source

```rust
pub fn process(&mut self, samples: &[f32], total_samples_consumed: u64) {
    self.buffer.extend_from_slice(samples);

    while self.buffer.len() >= WINDOW_SAMPLES && self.detected.is_none() {
        let window: Vec<f32> = self.buffer.drain(..WINDOW_SAMPLES).collect();
        let tone = classify(&window);
        self.tones.push(tone);
        if self.tones.len() > 14 {
            self.tones.remove(0);
        }
        if self.tones.len() == 14 {
            if let Some(code) = match_vis_pattern(&self.tones) {
                ...
            }
        }
    }
}
```

(Where `WINDOW_SAMPLES = 330` = 30 ms at 11025 Hz.)

#### Divergence

This is the single most important finding. Slowrx classifies on a **20 ms window slid 10 ms at a time** (50% overlap). Rust classifies on **30 ms boundary-aligned windows with 0 ms overlap** and uses the most recent 14 windows.

Consequences:
1. **VIS bit boundary alignment.** A VIS bit is 30 ms long. If the burst arrives at a sub-window phase offset, the C decoder has 3 candidate alignments per 30 ms (the `i = 0..2` loop, lines 82-84) corresponding to 0/10/20 ms phase offsets. Rust has only one alignment — whichever the boundary lands at. If the VIS burst starts at a buffer offset that's not a multiple of 330 samples, **every bit window straddles two adjacent bits**, mixing the two tones and yielding `Tone::Other` (5×-dominance failure).
2. **Tone history depth.** C remembers 450 ms, allowing it to find the VIS pattern anywhere within that window. Rust remembers exactly 14 × 30 ms = 420 ms — basically just the VIS pattern length itself, so a single-bit misclassification kills detection until 14 more good windows arrive.

The Rust test `detects_vis_after_pre_silence` explicitly notes "Pre-silence is a whole multiple of WINDOW_SAMPLES so the burst aligns with detector window boundaries" — which calls out this bug. The synthetic tests align by construction; real audio does not.

#### Why this might cause the real-audio failure

VERY likely. If 6 of 7 ARISS captures fail to produce ImageComplete and the failure mode is "0 events" (not "1 event with corrupt image"), VIS detection failure is the most common cause: no VIS detected → never enter Decoding state → no LineDecoded events → no ImageComplete event. Real audio captures will not be aligned to 30 ms boundaries; the boundary alignment of the 1 successful capture is essentially a lucky coincidence.

---

### Finding 3: VIS classification uses absolute frequencies, not relative tolerance

**Severity:** Critical
**Likely impact on real-audio decoder failure:** Yes
**Files:** `vis.c:80-110` ↔ `src/vis.rs:140-198`

#### C source

```c
// Tolerance ±25 Hz
CurrentPic.HedrShift = 0;
gotvis    = FALSE;
for (i = 0; i < 3; i++) {
  if (CurrentPic.HedrShift != 0) break;
  for (j = 0; j < 3; j++) {
    if ( (tone[1*3+i]  > tone[0+j] - 25  && tone[1*3+i]  < tone[0+j] + 25)  && // 1900 Hz leader
         (tone[2*3+i]  > tone[0+j] - 25  && tone[2*3+i]  < tone[0+j] + 25)  && // 1900 Hz leader
         ...
         (tone[5*3+i]  > tone[0+j] - 725 && tone[5*3+i]  < tone[0+j] - 675) && // 1200 Hz start bit
         (tone[14*3+i] > tone[0+j] - 725 && tone[14*3+i] < tone[0+j] - 675)    // 1200 Hz stop bit
       ) {
      gotvis = TRUE;
      for (k = 0; k < 8; k++) {
        if      (tone[6*3+i+3*k] > tone[0+j] - 625 && tone[6*3+i+3*k] < tone[0+j] - 575) Bit[k] = 0;
        else if (tone[6*3+i+3*k] > tone[0+j] - 825 && tone[6*3+i+3*k] < tone[0+j] - 775) Bit[k] = 1;
        ...
      }
      if (gotvis) {
        CurrentPic.HedrShift = tone[0+j] - 1900;
```

#### Rust source

```rust
fn classify(window: &[f32]) -> Tone {
    let p_leader = goertzel_power(window, LEADER_HZ);  // 1900.0
    let p_break = goertzel_power(window, BREAK_HZ);    // 1200.0
    let p0 = goertzel_power(window, BIT_ZERO_HZ);      // 1300.0
    let p1 = goertzel_power(window, BIT_ONE_HZ);       // 1100.0
    let mut ranked = [...];
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if ranked[0].1 > 5.0 * ranked[1].1 {
        ranked[0].0
    } else {
        Tone::Other
    }
}
```

#### Divergence

Slowrx's tolerance is **relative to the actually-observed leader frequency**: it scans `j = 0..2` candidate leader windows, takes that window's measured frequency as the reference (`tone[0+j]`), and checks all subsequent windows are within ±25 Hz of the offsets that VIS specifies (leader = 0, break = -700, bit 0 = -600, bit 1 = -800). The break bit thus is matched at `[reference - 725, reference - 675]` — *not* at exactly 1200 Hz. This means a radio mistuned by, say, +60 Hz (so 1900 Hz leader appears as 1960 Hz, break as 1260 Hz) is still classified correctly: the reference is 1960, the break window appears at 1260 ≈ 1960 - 700 (within ±25).

Rust's classifier compares against fixed absolute bins at 1900/1200/1100/1300 Hz. A VIS burst from a +60 Hz mistuned radio:
- Leader at 1960 Hz → Goertzel at 1900 Hz sees a tone ~60 Hz off, well outside `WINDOW_SAMPLES=330`'s frequency resolution (33 Hz/bin). Power at 1900 bin drops to noise level.
- Break at 1260 Hz → Goertzel at 1200 Hz sees a tone 60 Hz off → similar power drop.
- The 5× dominance test fails for **every window** → all classify as `Tone::Other` → no VIS detected.

Real radios drift. The 25 Hz tolerance in slowrx is there precisely because typical SSTV radios mistune by tens of Hz. The Rust port lost both the tolerance AND the detection of the offset (Finding 5).

#### Why this might cause the real-audio failure

VERY likely contributor. ARISS is well-known for drift due to Doppler shift on the ISS uplink (~10 kHz/s closing rate at 144-145 MHz = ~3 Hz/s in audio band, but accumulated over a pass is significant — and the ground station's tuner correction may overshoot). Combined with Finding 2 (alignment) this is the most plausible explanation for the 6/7 zero-detection failure pattern.

---

### Finding 4: Luminance clip uses truncation, not rounding

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No
**Files:** `common.c:49-53` ↔ `src/mode_pd.rs:23-30`

#### C source

```c
// Clip to [0..255]
guchar clip (double a) {
  if      (a < 0)   return 0;
  else if (a > 255) return 255;
  return  (guchar)round(a);
}
```

#### Rust source

```rust
pub(crate) fn freq_to_luminance(freq_hz: f64) -> u8 {
    let v = (freq_hz - 1500.0) / 3.137_254_9;
    // Truncation-via-`as` matches slowrx's clip() semantics
    // (slowrx uses `(unsigned char)a` which truncates the fractional part).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lum = v.clamp(0.0, 255.0) as u8;
    lum
}
```

#### Divergence

The Rust comment is **factually wrong**: slowrx uses `(guchar)round(a)`, which rounds-to-nearest. Rust uses `as u8`, which truncates toward zero. For a value like 127.7, slowrx outputs 128, Rust outputs 127. This applies to luminance and to all three RGB channels in `ycbcr_to_rgb` (where the integer divisions also truncate toward zero — though that's pre-divide truncation, slightly different math).

Worst-case error per pixel is ±1 in luminance, mostly bias-toward-darker. Cumulative effect: image is consistently 0.5 units dimmer on average. Visible if you compare side-by-side; not likely the cause of decode failure.

#### Why this might cause the real-audio failure

No. This is a rounding-error finding, off by one bit per channel. It would not cause a complete-image failure.

---

### Finding 5: HedrShift (radio mistuning offset) is not detected or applied

**Severity:** Critical
**Likely impact on real-audio decoder failure:** Yes
**Files:** `vis.c:106` + `video.c:255, 282-326, 397, 406` ↔ `src/vis.rs` (none) + `src/mode_pd.rs:127, 24` (none)

#### C source

```c
// vis.c:106
CurrentPic.HedrShift = tone[0+j] - 1900;

// video.c:255 (in GetVideo setup)
SyncTargetBin = GetBin(1200 + CurrentPic.HedrShift, FFTLen);

// video.c:282 (Praw range)
for (i=GetBin(1500+CurrentPic.HedrShift,FFTLen); i<=GetBin(2300+CurrentPic.HedrShift, FFTLen); i++)

// video.c:382 (peak search)
for (n = GetBin(1500 + CurrentPic.HedrShift, FFTLen) - 1; n <= GetBin(2300 + CurrentPic.HedrShift, FFTLen) + 1; n++) {

// video.c:397 (out-of-band fallback)
Freq = ( (MaxBin > GetBin(1900 + CurrentPic.HedrShift, FFTLen)) ? 2300 : 1500 ) + CurrentPic.HedrShift;

// video.c:406 (luminance scale)
StoredLum[SampleNum] = clip((Freq - (1500 + CurrentPic.HedrShift)) / 3.1372549);
```

#### Rust source

```rust
// mode_pd.rs:127 (peak search hardcoded at 1500-2300 Hz)
let bin_for = |hz: f64| -> usize {
    (hz * (FFT_LEN as f64) / f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)).round()
        as usize
};
let lo = bin_for(1500.0).saturating_sub(1).max(1);
let hi = bin_for(2300.0).saturating_add(1).min(FFT_LEN / 2 - 1);

// mode_pd.rs:24 (luminance: hardcoded 1500.0 / 3.1372549)
let v = (freq_hz - 1500.0) / 3.137_254_9;
```

#### Divergence

Slowrx detects the radio mistuning offset during VIS (the leader was supposed to be 1900 Hz; you measured it at 1900+δ; therefore radio is offset by δ Hz) and applies that offset throughout the entire decode pipeline. Every place that says "1500 Hz" or "2300 Hz" or "1200 Hz" actually means "the corresponding tone, offset by HedrShift".

Rust does not detect HedrShift (it's not a field on any struct, nor returned by `VisDetector`) and uses fixed unshifted bands. Consequence for a radio mistuned by, say, +40 Hz (small for ARISS):
- Black tone (1500 Hz) becomes 1540 Hz on the air → freq_to_luminance returns `(1540-1500)/3.137 ≈ 12.7` → 12 (instead of 0).
- White tone (2300 Hz) becomes 2340 Hz → freq_to_luminance returns `(2340-1500)/3.137 ≈ 267.7` → clamped to 255 (correct, but with no headroom).

But more critically: the peak-search range `[1500, 2300]` shifts to actual `[1540, 2340]` on the air. The Rust search is hardcoded to bins for `[1500, 2300]`, so the 2340 Hz tone may fall **just past** the search region (FFT_LEN=256, bin 53.4 = 2300 Hz, bin 54.5 = 2340 Hz; the search includes `bin_for(2300).saturating_add(1)` = bin 54, so 2340 Hz at bin 54.5 is right at the edge). For mistuning > ~50 Hz, white tones drop entirely out of the peak search and get classified as their nearest in-band neighbor.

For HedrShift > ~80 Hz the entire image is unreadable in the Rust port even though slowrx would still recover it perfectly.

Combined with Finding 3, this is the most plausible explanation: ARISS captures with measurable Doppler / radio offset would have HedrShift in the 30-100 Hz range → VIS detection fails (Finding 3) → no decoding path even attempted.

#### Why this might cause the real-audio failure

VERY likely. Even if Findings 2 and 3 are fixed, real captures with non-zero HedrShift will produce dim/dark/wrong-color images because the luminance scale is wrong. This is the kind of bug where 1 of 7 captures with near-zero offset works perfectly and the others fail (or produce garbled output). Pattern matches the observed behavior.

---

### Finding 6: No SNR-adaptive FFT window length

**Severity:** Important
**Likely impact on real-audio decoder failure:** Yes (low SNR ARISS captures)
**Files:** `video.c:30, 53-57, 354-367, 374-377` ↔ `src/mode_pd.rs:59, 63-70, 122`

#### C source

```c
// video.c:30, 53-57
double Hann[7][1024] = {{0}};
gushort HannLens[7] = { 48, 64, 96, 128, 256, 512, 1024 };
for (j = 0; j < 7; j++)
  for (i = 0; i < HannLens[j]; i++)
    Hann[j][i] = 0.5 * (1 - cos( (2 * M_PI * i) / (HannLens[j] - 1)) );

// video.c:354-367
// Adapt window size to SNR
if      (!Adaptive)  WinIdx = 0;
else if (SNR >=  20) WinIdx = 0;
else if (SNR >=  10) WinIdx = 1;
else if (SNR >=   9) WinIdx = 2;
else if (SNR >=   3) WinIdx = 3;
else if (SNR >=  -5) WinIdx = 4;
else if (SNR >= -10) WinIdx = 5;
else                 WinIdx = 6;

// Minimum winlength can be doubled for Scottie DX
if (Mode == SDX && WinIdx < 6) WinIdx++;
...
WinLength = HannLens[WinIdx];
for (i = 0; i < WinLength; i++) fft.in[i] = pcm.Buffer[pcm.WindowPtr + i - WinLength/2] / 32768.0 * Hann[WinIdx][i];
```

#### Rust source

```rust
/// FFT length used for per-pixel demod. Matches slowrx's bin spacing:
/// 256/11025 Hz = 43.07 Hz/bin, equal to slowrx's 1024/44100 Hz.
pub(crate) const FFT_LEN: usize = 256;

#[allow(clippy::cast_precision_loss)]
fn hann_window() -> Vec<f32> {
    (0..FFT_LEN)
        .map(|i| {
            let m = (FFT_LEN - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / m).cos())
        })
        .collect()
}
```

#### Divergence

Slowrx adapts FFT window length to SNR: short windows (48 samples = ~1.1 ms at 44.1 kHz) for high SNR (better time resolution = sharper edges between pixels), long windows (1024 samples = ~23 ms) for low SNR (better frequency resolution = recover from noise). At -10 dB SNR slowrx uses a 1024-sample window for noise rejection.

Rust uses fixed 256 samples (= ~23 ms at 11.025 kHz, matching slowrx's 1024@44.1kHz **at the floor of slowrx's adaptive range**). The Rust comment "matches slowrx's bin spacing: 256/11025 Hz = 43.07 Hz/bin, equal to slowrx's 1024/44100 Hz" is true but misleading — that equivalence is only with slowrx's *worst-case* (lowest SNR) window choice. In high-SNR conditions slowrx uses a 48-sample window (= 918 Hz/bin at 44.1 kHz) which is much sharper temporally — by a factor of ~21×.

For real radio with widely varying SNR, two issues:
1. **High-SNR pixels are blurred.** The fixed 23 ms window blurs across multiple PD pixels (PD120 PixelTime = 0.19 ms → 23 ms covers 121 pixels worth of time). Even with the hann window the frequency estimate is the average over those 121 pixels.
2. **Low-SNR pixels are usable but only because the fixed window happens to be at the slowrx low-SNR setting.** This is actually a (perhaps unintended) good default for real radio — high SNR is rare.

The bigger problem is that **with FFT_LEN=256 every per-pixel FFT covers 23 ms** while pixels are only 0.19 ms apart. Each FFT includes its left and right ~60 neighbors. The Rust code then averages over 121 pixels of frequency content per pixel sample. This is essentially a low-pass filter in image space — fine details are gone.

Wait, let me re-read. The FFT *centers* on the pixel sample. For PD120 (pixel time 0.19 ms = 2.1 samples at 11025 Hz), an FFT_LEN=256 window covers 256 samples = 23.2 ms ≈ 122 PD120 pixels. So adjacent pixels' FFTs overlap by 99.2%. The frequency estimate at pixel x is dominated by content from pixel x-60 to x+60.

#### Why this might cause the real-audio failure

Possibly contributor. A 121-pixel-wide spatial blur means you'd see a *smeared* image, but you'd still see *something* and ImageComplete would still fire. So this isn't the "0 ImageComplete" mechanism — it's a degraded-quality mechanism. **More likely contributes to image quality issues than to total failure**.

---

### Finding 7: No slant correction (sync.c::FindSync not ported)

**Severity:** Important
**Likely impact on real-audio decoder failure:** Yes (degrades image, maybe mode-detection-fail)
**Files:** `sync.c:18-133` ↔ none in Rust

#### C source

```c
double FindSync (guchar Mode, double Rate, int *Skip) {
  ...
  // Linear Hough transform (lines 50-69)
  ...
  // Adjust sample rate (line 81)
  Rate += tan(deg2rad(90 - slantAngle)) / LineWidth * Rate;
  ...
  // Skip until the start of the line (lines 119-127)
  s = xmax / 700.0 * ModeSpec[Mode].LineTime - ModeSpec[Mode].SyncTime;
  if (Mode == S1 || Mode == S2 || Mode == SDX)
    s = s - ModeSpec[Mode].PixelTime * ModeSpec[Mode].ImgWidth / 2.0
          + ModeSpec[Mode].PorchTime * 2;
  *Skip = s * Rate;
  return (Rate);
}
```

```c
// slowrx.c:144-149
printf("  FindSync @ %.1f Hz\n",CurrentPic.Rate);
CurrentPic.Rate = FindSync(CurrentPic.Mode, CurrentPic.Rate, &CurrentPic.Skip);
// Final image
printf("  getvideo @ %.1f Hz, Skip %d, HedrShift %+d Hz\n", CurrentPic.Rate, CurrentPic.Skip, CurrentPic.HedrShift);
GetVideo(CurrentPic.Mode, CurrentPic.Rate, CurrentPic.Skip, TRUE);
```

#### Rust source

```rust
// decoder.rs:180-220 (no FindSync; line pair indexing assumes constant cadence)
loop {
    let cur_pair = u64::from(*line_pair_index);
    let cur_off = (cur_pair as f64 * spec.line_seconds * work_rate).round();
    let next_off =
        ((cur_pair + 1) as f64 * spec.line_seconds * work_rate).round();
    let samples_per_pair = (next_off - cur_off) as usize;
    ...
    crate::mode_pd::decode_pd_line_pair(*spec, *line_pair_index, &buffer[..needed], image, &mut self.pd_demod);
    ...
    buffer.drain(..samples_per_pair);
    *line_pair_index += 1;
}
```

#### Divergence

Slowrx's two-pass decode flow: GetVIS → first GetVideo (sample-rate=44100, skip=0) → FindSync analyzes the captured `HasSync[]` array → adjusts sample rate to cancel the slant + finds the true line-start offset → second GetVideo (rate=adjusted, skip=found, redraw=TRUE) re-reads the cached `StoredLum` at the corrected pixel times.

Rust does only the first pass. If the audio source's sample rate is off by even 0.1% from the assumed 11025 (which is **always true** — sound cards drift, recordings are nominally 16 kHz mono but actually 16001.3 Hz, etc), each line drifts by 0.1% × line_seconds. For PD180 (line_seconds = 0.75424), that's 754 µs/line × 248 lines = 187 ms drift. By line 248 the decoder is sampling 248 lines later than audio actually is — pixels come from where the sync pulse is, not the image data.

Two consequences:
1. **Slant in the image** — visible diagonal stripes if you look at the output.
2. **End-of-image runs out of samples**: if drift accumulates past the captured audio length, the decoder's `if buffer.len() < needed { break; }` exits early — meaning **ImageComplete is never emitted**.

This is highly relevant to the failure mode. Real ARISS captures are typically 2-5 minutes long (PD120 image runs ~120s; if the decoder drifts and runs out of audio before line 248, you get 0 ImageComplete events).

#### Why this might cause the real-audio failure

YES — likely contributor. Sample rate mismatch is the canonical cause of "decoder hangs at end of image". For the 1 capture that succeeded, either the recording happened to be sample-perfect, or it had enough trailing audio that even with drift the decoder reached line 496.

---

### Finding 8: No Skip computation (line 0 phase alignment)

**Severity:** Important
**Likely impact on real-audio decoder failure:** Yes
**Files:** `sync.c:96-127` + `slowrx.c:115-149` ↔ `src/decoder.rs:127-134`

#### C source

```c
// sync.c:96-127
// accumulate a 1-dim array of the position of the sync pulse
memset(xAcc, 0, sizeof(xAcc[0]) * 700);
for (y=0; y<ModeSpec[Mode].NumLines; y++) {
  for (x=0; x<700; x++) { 
    t = y * ModeSpec[Mode].LineTime + x/700.0 * ModeSpec[Mode].LineTime;
    xAcc[x] += HasSync[ (int)(t / (13.0/44100) * Rate/44100) ];
  }
}

// find falling edge of the sync pulse by 8-point convolution
for (x=0;x<700-8;x++) {
  convd = 0;
  for (int i=0;i<8;i++) convd += xAcc[x+i] * ConvoFilter[i];
  if (convd > maxconvd) {
    maxconvd = convd;
    xmax = x+4;
  }
}

if (xmax > 350) xmax -= 350;
s = xmax / 700.0 * ModeSpec[Mode].LineTime - ModeSpec[Mode].SyncTime;
*Skip = s * Rate;
```

#### Rust source

```rust
// vis.rs::take_residual_buffer + decoder.rs:127-134
let residual = self.vis.take_residual_buffer();
self.state = State::Decoding {
    mode: spec.mode,
    spec,
    line_pair_index: 0,
    image,
    buffer: residual,
};
```

#### Divergence

Slowrx computes a `Skip` in samples = where line 0 actually starts within the captured audio, by 8-point convolutional edge detection on the accumulated sync-pulse position across every line. This handles the case where there's audio between the VIS stop bit and the first sync pulse (radio settling, gap).

Rust assumes the first sample of the residual buffer (post-VIS-stop-bit) is the start of line 0's sync pulse. In `vis.c:169` slowrx explicitly skips 20 ms after the stop bit (`readPcm(20e-3 * 44100); pcm.WindowPtr += 20e-3 * 44100;`). Rust does *not* — it hands raw post-stop residual to the PD decoder.

Combined with PD's expectation that line 0 starts with a 20 ms sync pulse: the Rust decoder reads **VIS stop bit residue + post-stop silence** as the first 20 ms of "line 0 sync." If the residual happened to be 5 ms of stop bit + 15 ms of silence + actual sync starts at 20 ms from buffer head, then PD's chan_starts[0] = sync+porch = 22.08 ms is the `Y(odd)` start — but the actual `Y(odd)` data lives at `~37 ms` from buffer head. The decoder reads 22 ms of audio that's actually part of the radio's pre-sync transient or even silence.

For low-SNR signals where the sync pulse onset is ambiguous, even slowrx's approach has trouble — but the convolution-based localization handles up to ±175 ms of misalignment (`if (xmax > 350) xmax -= 350`).

#### Why this might cause the real-audio failure

YES, this is the second-most-likely smoking gun after Findings 3 and 5. With no Skip computation, every Rust-decoded image starts mid-pixel and the line-pair cadence is offset from real audio by a constant amount — likely tens to hundreds of milliseconds. After PD180's 248 line pairs at ~754 ms each, accumulated misalignment can exceed the captured audio length, returning early without ImageComplete.

---

### Finding 9: VIS post-stop-bit gap not skipped

**Severity:** Important (related to Finding 8)
**Likely impact on real-audio decoder failure:** Yes
**Files:** `vis.c:168-170` ↔ `src/vis.rs:135-137` + `src/decoder.rs:127`

#### C source

```c
// Skip the rest of the stop bit
readPcm(20e-3 * 44100);
pcm.WindowPtr += 20e-3 * 44100;

if (VISmap[VIS] != UNKNOWN) return VISmap[VIS];
```

#### Rust source

```rust
// vis.rs:135-137
pub fn take_residual_buffer(&mut self) -> Vec<f32> {
    std::mem::take(&mut self.buffer)
}

// decoder.rs:127 (handed straight to State::Decoding)
let residual = self.vis.take_residual_buffer();
```

#### Divergence

Slowrx skips an additional 20 ms (= 882 samples at 44.1 kHz, or ~221 samples at 11.025 kHz) after detecting the VIS stop bit. The C comment is "Skip the rest of the stop bit" — meaning whatever fraction of the 30 ms stop bit overlaps the detection window's end is consumed. (In C, the detection window straddles 20 ms of past + 10 ms of future relative to `WindowPtr`, so when the stop bit is detected the read pointer is at the *middle* of the stop bit; an extra 20 ms read positions it past the stop bit's end.)

In Rust, when VIS is detected the boundary-aligned 30 ms stop bit window has just been consumed (drained from buffer), so `take_residual_buffer` returns only the post-stop-bit audio — `vis.rs::process` already drained the stop window. **However**, because Rust uses 30 ms boundary windows, the stop bit detection actually completes at the END of the 30 ms stop window, meaning the residual *is* post-stop-bit audio — no extra skip needed. So far so good.

The catch: slowrx's 20 ms extra-skip handles the boundary-misaligned case where the stop bit didn't end exactly at the window boundary. In real audio there can be 5-50 ms of dead air (radio AGC settling, transmitter timing) between the VIS stop bit and the start of line 0's sync pulse — slowrx absorbs this by relying on FindSync to compute Skip later.

**The actual issue:** Rust hands the residual *as if it starts at line 0's sync pulse*. Without Skip computation (Finding 8), if any audio between stop bit and line 0 exists, it ends up consumed as part of "line 0's sync pulse + porch + Y(odd)." This pushes every pixel reading earlier than the actual audio content, and the misalignment compounds across all 248 line pairs.

#### Why this might cause the real-audio failure

Yes — directly related to Finding 8. Without the 20 ms skip + Skip recovery, line 0 alignment is sample-accurate only for synthetic signals where no inter-burst gap exists.

---

### Finding 10: Sample rate fixed at 11025 — no `Rate` adjustment from FindSync

**Severity:** Important
**Likely impact on real-audio decoder failure:** Yes
**Files:** `slowrx.c:73, 145` ↔ `src/decoder.rs:166, 187-189`

#### C source

```c
// slowrx.c:73
CurrentPic.Rate = 44100;

// slowrx.c:145
CurrentPic.Rate = FindSync(CurrentPic.Mode, CurrentPic.Rate, &CurrentPic.Skip);
```

#### Rust source

```rust
// decoder.rs:166
let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);

// decoder.rs:187-189
let cur_off = (cur_pair as f64 * spec.line_seconds * work_rate).round();
let next_off =
    ((cur_pair + 1) as f64 * spec.line_seconds * work_rate).round();
```

#### Divergence

Slowrx caches an explicit `Rate` field on `CurrentPic` and FindSync mutates it. The second GetVideo pass uses the corrected rate, recomputing `PixelGrid[].Time = (int)round(Rate * (...))` for every pixel.

Rust uses `WORKING_SAMPLE_RATE_HZ = 11025` as a constant. If the recording's effective rate is different (sound card clock drift, file actually 16000 Hz resampled "to 11025 Hz" with a slight error in the resampler, etc.), there is no way to compensate. The `cur_off`/`next_off` rounded-cumulative-time approach prevents per-pair drift from accumulating, but it cannot correct for an actual rate mismatch.

#### Why this might cause the real-audio failure

Yes — same root as Finding 7 (slant correction). Sample rate mismatches are normal for real audio. C handles them via FindSync; Rust does not.

---

### Finding 11: FFT once per pixel (Rust) vs every 6 samples (C)

**Severity:** Important
**Likely impact on real-audio decoder failure:** No (image quality, not detection)
**Files:** `video.c:350-407, 421` ↔ `src/mode_pd.rs:241-247`

#### C source

```c
// video.c:350
if (SampleNum % 6 == 0) { // Take FFT every 6 samples
  PrevFreq = Freq;
  // ...adapt window, FFT, find peak, interpolate, set Freq...
} /* endif */
// Calculate luminency & store for later use
StoredLum[SampleNum] = clip((Freq - (1500 + CurrentPic.HedrShift)) / 3.1372549);

// video.c:411-421
if (SampleNum == PixelGrid[PixelIdx].Time) {
  while (SampleNum == PixelGrid[PixelIdx].Time) {
    x = PixelGrid[PixelIdx].X;
    ...
    Image[x][y][Channel] = StoredLum[SampleNum];
```

#### Rust source

```rust
// mode_pd.rs:241-247
for x in 0..width as usize {
    let center_sec_rel = (x as f64 + 0.5) * pixel_secs;
    let center_sample_rel = (center_sec_rel * work_rate).round() as i64;
    let freq = demod.pixel_freq(&chan_samples, center_sample_rel);
    channel_buf[x] = freq_to_luminance(freq);
}
```

#### Divergence

Slowrx computes FFT every 6 samples (= 7350 Hz analysis rate at 44.1 kHz, or 6 samples = 136 µs). The pipeline is: FFT → instantaneous Freq estimate → `StoredLum[SampleNum] = clip(...)`. The `Freq` variable stays stale between FFTs (only refreshed every 6 samples), so adjacent samples within the 6-sample interval all get the *same* `StoredLum` value. When the `PixelGrid[].Time`s are sampled, they read from this densely-populated `StoredLum` array.

Effect: the per-pixel luminance is the freq estimate from the FFT closest in time to the pixel center, using whatever WinIdx (window length) was active when that FFT ran. The window centers shift with each FFT, so the pixel reads represent a continuous time-domain envelope.

Rust does FFT *only* at pixel centers (one FFT per pixel × width × 4 channels = 2560 FFTs per line pair for PD180). The window CENTERED on each pixel time means `[pixel_center - 128, pixel_center + 128]` samples are analyzed.

For most pixels, the two approaches are equivalent. The difference shows up at:
- **Channel boundaries**: Rust *isolates* each channel into its own zero-padded slice (line 233-239) so the FFT doesn't see adjacent channel content. Slowrx allows the FFT to leak across channel boundaries — but only for the rightmost ~12 pixels of one channel and leftmost ~12 pixels of the next (PD pixel time 0.19 ms, FFT window 23 ms = 121 PD pixels, but only the half within the FFT window's other side leaks).

Actually, let me re-think. The Rust isolation strategy means at the right edge of a channel, the FFT window has ~half its samples zero-padded. **This destroys the FFT estimate for the rightmost pixels of every channel.** Specifically: the rightmost 60 pixels of every channel have FFT windows that are progressively more zero-padded, biasing the frequency estimate toward DC.

Compare to slowrx: the rightmost pixels of, e.g., the Y(odd) channel see a window that includes the Cr channel's left side. That's ALSO bad — but at least the Cr tone is in the analyzable band (1500-2300 Hz range), so the peak search finds *something*. With zero-padding, the peak search just finds whichever bin has the most spectral leakage from the truncated tone — typically biased toward DC.

#### Why this might cause the real-audio failure

Probably not the primary cause of "no ImageComplete." But this IS a quality issue: rightmost pixels of every channel have systematically wrong values. A 60-pixel-wide stripe of garbage at every channel boundary on every line pair would be visually obvious. Probably contributes to "the decoded image looks weird" but doesn't prevent decoding.

---

### Finding 12: PD line-pair `pixel_seconds` time-base misaligned with cumulative line offsets

**Severity:** Important
**Likely impact on real-audio decoder failure:** Possibly (subtle drift)
**Files:** `video.c:140-142` ↔ `src/decoder.rs:181-220, src/mode_pd.rs:225-228`

#### C source

```c
// video.c:140-142
PixelGrid[PixelIdx].Time = (int)round(Rate * ( y/2 * ModeSpec[Mode].LineTime + ChanStart[Channel] +
                                      ModeSpec[Mode].PixelTime * 1.0 * (x + 0.5))) +
                                      Skip;
```

#### Rust source

```rust
// decoder.rs:181-194 (caller cuts the buffer at cumulative-rounded line-pair boundaries)
let cur_off = (cur_pair as f64 * spec.line_seconds * work_rate).round();
let next_off =
    ((cur_pair + 1) as f64 * spec.line_seconds * work_rate).round();
let samples_per_pair = (next_off - cur_off) as usize;
let needed = samples_per_pair + lookahead;
if buffer.len() < needed {
    break;
}
crate::mode_pd::decode_pd_line_pair(
    *spec,
    *line_pair_index,
    &buffer[..needed],
    image,
    &mut self.pd_demod,
);
buffer.drain(..samples_per_pair);

// mode_pd.rs:225-228 (inside decode_pd_line_pair: each pair starts at relative time 0)
let chan_start = (start_sec * work_rate).round() as i64;
let chan_end = (end_sec * work_rate).round() as i64;
```

#### Divergence

Both compute pixel times the same way globally, but Rust does it in *two stages*: the decoder cuts the audio buffer at cumulative-rounded line-pair boundaries, then the per-line-pair decoder treats time relative to that pair's start as `0`.

Stage-1 rounding: `next_off = ((cur_pair + 1) * line_seconds * sr).round()` and the consumed length is `next_off - cur_off`. If `line_seconds = 0.50848`, `sr = 11025`, then `line_seconds * sr = 5606.09`. So pair 0 is 5606 samples (= round(5606.09)), pair 1 is round(11212.18) - round(5606.09) = 11212 - 5606 = 5606, pair 2 is round(16818.27) - round(11212.18) = 16818 - 11212 = 5606, ... The difference accumulates a 1-sample deviation per ~10 pairs (the comment in decoder.rs:178 says "PD180 drifts ~0.05 samples/pair"). So far OK.

Stage-2 within-pair: each pair's `chan_start = (start_sec * work_rate).round() as i64` where `start_sec = sync + porch + n*width*pixel_secs` — but **anchored at relative-zero**, not at the actual pair start (which differs from relative-zero by a fraction of a sample due to stage-1 rounding).

Effect: each pair has a sub-sample misalignment of up to 0.5 samples between the audio's actual content and the channel boundary. For PD180 with 248 line pairs, this is visually invisible — the channel-isolation slicing dominates (Finding 11) so most pixel samples are correct.

But: the *first pair* (pair 0) starts at buffer index 0 in Rust; in slowrx, line 0 of GetVideo is at SampleNum = 0 too — but slowrx's `SampleNum = 0` corresponds to the audio sample where the post-VIS skip ended (i.e., line 0's sync pulse start). Rust assumes the same — but without Skip computation (Finding 8), line 0's sync pulse may not be at buffer[0] at all.

#### Why this might cause the real-audio failure

This finding alone is minor (sub-sample stage-2 rounding). But combined with Finding 8 (no Skip), the "buffer[0] = line 0 sync start" assumption breaks for any real recording.

---

### Finding 13: `PD` channel start formula uses `ChanStart[1] = ChanStart[0] + ChanLen[0] + SeptrTime` (with SeptrTime=0); Rust hardcodes `+ width*pixel_secs` (no SeptrTime field)

**Severity:** Cosmetic for PD; would break for non-PD-zero-Septr modes
**Likely impact on real-audio decoder failure:** No
**Files:** `video.c:88-92` ↔ `src/mode_pd.rs:206-211`

#### C source

```c
// video.c:88-92
case PD50:
case PD90:
case PD120:
...
case PD290:
  ChanLen[0]   = ChanLen[1] = ChanLen[2] = ChanLen[3] = ModeSpec[Mode].PixelTime * ModeSpec[Mode].ImgWidth;
  ChanStart[0] = ModeSpec[Mode].SyncTime + ModeSpec[Mode].PorchTime;
  ChanStart[1] = ChanStart[0] + ChanLen[0] + ModeSpec[Mode].SeptrTime;
  ChanStart[2] = ChanStart[1] + ChanLen[1] + ModeSpec[Mode].SeptrTime;
  ChanStart[3] = ChanStart[2] + ChanLen[2] + ModeSpec[Mode].SeptrTime;
  break;
```

#### Rust source

```rust
// mode_pd.rs:206-211
let chan_starts_sec = [
    sync_secs + porch_secs,                                       // Y(odd)
    sync_secs + porch_secs + f64::from(width) * pixel_secs,       // Cr
    sync_secs + porch_secs + 2.0 * f64::from(width) * pixel_secs, // Cb
    sync_secs + porch_secs + 3.0 * f64::from(width) * pixel_secs, // Y(even)
];
```

#### Divergence

For PD120/PD180, `SeptrTime = 0e-3` per modespec.c, so the channel start formulas are mathematically identical. Rust simply omitted the `SeptrTime` term entirely — there's no `septr_seconds` field on the Rust ModeSpec struct.

For pure correctness this is fine for PD; for V2 modes (Robot, Scottie, Martin) the SeptrTime is non-zero and this approach would break. Right now, no risk.

#### Why this might cause the real-audio failure

No — for PD modes the term is 0.

---

### Finding 14: VIS `R12BW` parity inversion not applied (mode not implemented but the C code special-cases it)

**Severity:** Cosmetic (V1 doesn't implement R12BW)
**Likely impact on real-audio decoder failure:** No
**Files:** `vis.c:116` ↔ `src/vis.rs:165-198`

#### C source

```c
Parity = Bit[0] ^ Bit[1] ^ Bit[2] ^ Bit[3] ^ Bit[4] ^ Bit[5] ^ Bit[6];

if (VISmap[VIS] == R12BW) Parity = !Parity;

if (Parity != ParityBit) {
  printf("  Parity fail\n");
  gotvis = FALSE;
}
```

#### Rust source

```rust
fn match_vis_pattern(tones: &[Tone]) -> Option<u8> {
    ...
    let mut code = 0u8;
    let mut parity = 0u8;
    for (i, tone) in tones[5..12].iter().enumerate() {
        ...
        code |= bit << i;
        parity ^= bit;
    }
    ...
    if parity != parity_bit {
        return None;
    }
    Some(code)
}
```

#### Divergence

The C code inverts parity for R12BW (VIS code 0x06). Rust does not. Since R12BW is not implemented in `lookup`, this would never produce a successful decode anyway — but if a future PR adds R12BW without fixing the parity check, this will silently reject all R12BW VIS bursts.

#### Why this might cause the real-audio failure

No — V1 doesn't implement R12BW.

---

### Finding 15: `lookup` returns `None` for `0x00` instead of treating "VIS = 0" as silent-noise (compatibility)

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `vis.c:172-174` ↔ `src/modespec.rs:62-68`

#### C source

```c
if (VISmap[VIS] != UNKNOWN) return VISmap[VIS];
else                        printf("  No VIS found\n");
return 0;
```

#### Rust source

```rust
pub fn lookup(vis_code: u8) -> Option<ModeSpec> {
    match vis_code {
        0x5F => Some(PD120),
        0x60 => Some(PD180),
        _ => None,
    }
}
```

In the decoder:
```rust
} else {
    // Unknown VIS codes silently drop. Reset the
    // detector's buffer so it does not accumulate
    // forever on uninterpretable bursts.
    let _ = self.vis.take_residual_buffer();
}
```

#### Divergence

Slowrx's GetVIS returns 0 for unknown codes, and Listen() loops back to `do { ... } while (Mode == 0);`. Rust does the same in spirit.

#### Why this might cause the real-audio failure

No.

---

### Finding 16: VIS bit timing — Rust's per-window-end time vs C's center-of-window time for "WindowPtr"

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No
**Files:** `vis.c:48` ↔ `src/vis.rs:97-118`

#### C source

```c
// fft.in[i] = pcm.Buffer[pcm.WindowPtr + i - 441] / 32768.0 * Hann[i];
// WindowPtr is at the END of the most recent 10 ms read.
// The window is [WindowPtr - 441, WindowPtr - 441 + 882) = [WindowPtr - 441, WindowPtr + 441).
// Effectively: the FFT analyzes 10 ms of past + 10 ms of future relative to WindowPtr.
```

#### Rust source

```rust
let window: Vec<f32> = self.buffer.drain(..WINDOW_SAMPLES).collect();
```

(Window is [drain_start, drain_start + 330) — purely 30 ms of past relative to where the next window starts.)

#### Divergence

C's analysis window is centered (10 ms past + 10 ms future); Rust's is purely past. This means the VIS pattern check uses different timing. C effectively detects the VIS burst after consuming ~half a window's worth more of audio than Rust would (since it can "look ahead" 10 ms).

This is mostly invisible because both analyze a contiguous audio stream. Where it matters: the residual buffer left after detection. C's `pcm.WindowPtr` is at the *middle* of the stop-bit window when detection fires; Rust's `buffer` head is at the *end* of the stop-bit window. So C needs an additional `readPcm(20e-3 * 44100)` to skip past the stop bit; Rust is already past it. This is actually fine.

#### Why this might cause the real-audio failure

No.

---

### Finding 17: The `working_samples_emitted` is computed from resampler output but `vis::DetectedVis::end_sample` reports the input-buffer length, not the working-rate sample where the stop bit ended

**Severity:** Cosmetic (informational field, not used by decoder logic)
**Likely impact on real-audio decoder failure:** No
**Files:** `src/vis.rs:113` (referenced from `src/decoder.rs:117-121`)

#### Rust source

```rust
// vis.rs:113 (using `self.buffer.len()` to subtract back from the running counter)
let end_sample =
    total_samples_consumed.saturating_sub(self.buffer.len() as u64);
self.detected = Some(DetectedVis { code, end_sample });
```

This is an informational field. The decoder emits it as `VisDetected.sample_offset`. Not used by downstream decoding logic.

#### Why this might cause the real-audio failure

No.

---

### Finding 18: Rust resampler — group delay and FIR transient

**Severity:** Important
**Likely impact on real-audio decoder failure:** Possibly
**Files:** `src/resample.rs:91-132` ↔ slowrx (no resampler — fixed 44100)

#### Rust source

```rust
pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
    let mut buf = std::mem::take(&mut self.tail);
    buf.extend_from_slice(input);

    let mut out = Vec::new();
    let half = (FIR_TAPS as f64) / 2.0;
    loop {
        let center = self.phase + half;
        let needed_end = (center + half).ceil() as usize;
        if needed_end > buf.len() {
            break;
        }
        ...
    }
    ...
}
```

#### Divergence

The 64-tap FIR introduces a 32-sample group delay at the working rate. The first 32 working-rate samples emitted from `Resampler::process` are convolutions involving phantom zero samples at the start of the input — effectively a transient. The Rust decoder test `process_emits_vis_detected_for_pd120_burst` adds 512 samples of trailing silence to compensate, which acknowledges this.

Effect on VIS detection: the first ~32 working-rate samples (= ~3 ms) are biased toward zero. For a VIS burst that starts at audio[0], the first 3 ms of the leader is attenuated. This is short relative to the 30 ms VIS window, but combined with Finding 2 (boundary alignment), if the burst-start happens to land on a boundary, that 3 ms attenuation falls inside the first leader window. The Goertzel power for that window drops, possibly below the 5×-dominance threshold.

#### Why this might cause the real-audio failure

Possible. Real captures rarely have the VIS burst start at audio[0] — usually there's seconds of pre-burst audio, so the FIR transient is long absorbed before VIS-relevant content begins. **Probably not the primary cause** but contributes in edge cases.

---

### Finding 19: VIS detector skips real-time analysis after detection (no continuation for next-burst)

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No (one-image-per-decoder use case)
**Files:** `vis.c:40-167` (loops forever) ↔ `src/vis.rs:97-117` + `src/decoder.rs:127-141`

#### Rust source

```rust
// vis.rs:97 — process exits as soon as detected.is_some()
while self.buffer.len() >= WINDOW_SAMPLES && self.detected.is_none() {
```

#### Divergence

C's GetVIS loop returns immediately upon detection; the next call re-enters a fresh loop. Rust's structure is the same. The Rust comment about mid-image VIS detection (lines 153-164 of decoder.rs) acknowledges this is deferred. For single-image decode, this matches.

#### Why this might cause the real-audio failure

No.

---

### Finding 20: `decode_pd_line_pair` per-channel zero-padding of the FFT input loses information at channel boundaries

**Severity:** Important
**Likely impact on real-audio decoder failure:** No (image quality)
**Files:** `src/mode_pd.rs:230-247`

#### Rust source

```rust
let chan_len = (chan_end - chan_start).max(0) as usize;
let mut chan_samples = vec![0.0_f32; chan_len];
for (i, dst) in chan_samples.iter_mut().enumerate() {
    let src_idx = chan_start + i as i64;
    if src_idx >= 0 && (src_idx as usize) < window.len() {
        *dst = window[src_idx as usize];
    }
}

for x in 0..width as usize {
    let center_sec_rel = (x as f64 + 0.5) * pixel_secs;
    let center_sample_rel = (center_sec_rel * work_rate).round() as i64;
    let freq = demod.pixel_freq(&chan_samples, center_sample_rel);
    channel_buf[x] = freq_to_luminance(freq);
}
```

#### Divergence

For PD120, `chan_len = round(640 * 0.19e-3 * 11025) = round(1340.4) = 1340 samples`. Each channel's audio slice is 1340 samples. The FFT_LEN is 256, centered on `(x + 0.5) * 0.19e-3 * 11025 = 2.094 * (x + 0.5)` — so for x=0, center is sample ~1; for x=639, center is sample ~1339.

For x=0 (center=~1), the FFT window is `[1 - 128, 1 + 128) = [-127, 129)`. **127 of those samples are zero** (left of buffer start). Rust's FFT sees 127 zero samples + 129 actual samples — a massively zero-padded signal. The FFT power spectrum is dominated by the boxcar shape of the zero-padded gap, biasing the peak toward DC.

Same effect at x=639 (center=~1339): window is `[1339-128, 1339+128) = [1211, 1467)`. Samples 1340..1467 are out-of-bounds → 127 zero samples on the right.

So the leftmost ~60 and rightmost ~60 pixels of every channel have unreliable frequency estimates due to zero-padding. That's 120 of every 640 pixels per channel × 4 channels per pair = bad pixel count = 480 / 2560 ≈ 19% of every line pair has unreliable values.

Slowrx avoids this by NOT isolating channels — each FFT sees ~128 samples to each side of the pixel center, including content from adjacent channels. This is technically wrong (cross-channel leakage), but the result is "use the dominant tone in the band" which still finds the correct video tone; with isolation Rust gets "the dominant tone of a heavily-zero-padded signal" which biases toward DC.

#### Why this might cause the real-audio failure

No, but a major image quality contributor. Visible as ~10% wide stripes of garbage at left/right of every channel = 4 vertical noise stripes per line pair. Would not prevent ImageComplete from firing.

---

### Finding 21: PD `decode_pd_line_pair` window passed as `&buffer[..needed]` may not extend far enough for last-pixel FFT

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No
**Files:** `src/decoder.rs:170, 192, 199-205`

#### Rust source

```rust
let lookahead = crate::mode_pd::FFT_LEN / 2;
...
let needed = samples_per_pair + lookahead;
if buffer.len() < needed {
    break;
}
...
crate::mode_pd::decode_pd_line_pair(
    *spec,
    *line_pair_index,
    &buffer[..needed],
    image,
    &mut self.pd_demod,
);
```

#### Divergence

The decoder requests `samples_per_pair + FFT_LEN/2` samples to allow the last-pixel FFT to extend right of the line-pair end. But `decode_pd_line_pair` then slices each channel's content — for the Y(even) channel (last channel of the pair), `chan_end = (sync + porch + 4*width*pixel) * sr`. For PD180: `(0.020 + 0.00208 + 4*640*0.000286) * 11025 = (0.022 + 0.7322) * 11025 = 0.75424 * 11025 = 8316` samples. samples_per_pair = round(0.75424 * 11025) = 8316 too. So chan_end = samples_per_pair, meaning the FFT for the last pixel of Y(even) would want samples up to chan_end + FFT_LEN/2 = 8444, but the window is only `[..8316 + 128] = [..8444]` ... so just barely in bounds.

But **mode_pd does not pass a window with that lookahead to the per-channel slice** — it just slices `[chan_start..chan_end]` (lines 232-239), throwing away any lookahead samples that the decoder bothered to pass in. So the last pixel of Y(even) gets zero-padded on the right (Finding 20's mechanism). The `lookahead` value computed in decoder.rs is **dead code** in `decode_pd_line_pair`'s actual analysis.

#### Why this might cause the real-audio failure

No — but it's dead code that suggests the original author intended to provide lookahead and forgot to wire it through.

---

### Finding 22: The `working_samples_emitted` counter is never decremented on `take_residual_buffer`

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `src/decoder.rs:104-107, 127`

#### Rust source

```rust
self.samples_processed = self.samples_processed.saturating_add(audio.len() as u64);
self.working_samples_emitted = self
    .working_samples_emitted
    .saturating_add(working.len() as u64);
...
let residual = self.vis.take_residual_buffer();
```

The counter does not subtract residual length when transferred to State::Decoding. Cosmetic — counter is informational only.

---

### Finding 23: `tones.remove(0)` is O(n) per window — minor but not material

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `src/vis.rs:101-103`

```rust
self.tones.push(tone);
if self.tones.len() > 14 {
    self.tones.remove(0);
}
```

`Vec::remove(0)` shifts all 13 remaining elements. Trivial for 14-element vec. Use `VecDeque` for clarity. Cosmetic.

---

## Section 4 — Severity-ranked summary

1. **[Critical]** Finding 5 — HedrShift (radio mistuning offset) is not detected or applied. Real radios drift by tens of Hz; without HedrShift, both VIS detection and per-pixel luminance scaling are broken.
2. **[Critical]** Finding 3 — VIS classification uses absolute (not relative) frequency tolerance. A radio off by 50 Hz fails to match VIS bits at all → no detection.
3. **[Critical]** Finding 2 — VIS windows do not overlap (no 10 ms hop). Real audio is not 30 ms boundary-aligned with the burst → bit windows straddle two adjacent bits → all classify as `Tone::Other` → no detection.
4. **[Important]** Finding 7 — No slant correction. Sample rate drift accumulates; PD180's 248 line pairs may run out of audio before ImageComplete fires.
5. **[Important]** Finding 8 — No Skip computation (line 0 phase alignment). The residual buffer's first sample is assumed to be the start of line 0's sync pulse, which is never true for real audio with any post-VIS settling time.
6. **[Important]** Finding 10 — Sample rate fixed at 11025; no `Rate` adjustment from FindSync. Same root cause as Finding 7.
7. **[Important]** Finding 9 — VIS post-stop-bit gap not skipped. Related to Finding 8.
8. **[Important]** Finding 6 — No SNR-adaptive FFT window. Fixed 256 = 23 ms covers ~120 PD120 pixels of audio per FFT, blurring image detail.
9. **[Important]** Finding 1 — VIS detection has no Hann window. Spectral leakage at boxcar -13 dB sidelobes vs Hann -32 dB. Lowers detection robustness in noisy audio.
10. **[Important]** Finding 11 — FFT once per pixel (Rust) vs every 6 samples (C). Combined with Finding 20 (channel isolation) creates wide stripes of garbage at channel edges.
11. **[Important]** Finding 20 — Per-channel zero-padding of FFT input loses information at channel boundaries. ~19% of pixels per line pair have unreliable estimates.
12. **[Important]** Finding 18 — Rust resampler group delay / FIR transient. Edge case for VIS bursts at start of audio.
13. **[Important]** Finding 12 — Stage-2 within-pair sub-sample misalignment. Minor on its own; compounds with Finding 8.
14. **[Minor]** Finding 4 — `clip` uses truncation instead of round-to-nearest. ±1 LSB error per pixel.
15. **[Minor]** Finding 16 — VIS WindowPtr semantics differ (center vs end). Functional difference is absorbed by the residual-buffer handling.
16. **[Minor]** Finding 13 — `chan_starts_sec` lacks `septr_seconds` term (zero for PD; would break Robot/Scottie).
17. **[Minor]** Finding 19 — VIS detector exits on first detection (matches C; fine).
18. **[Minor]** Finding 21 — `decode_pd_line_pair` ignores the lookahead samples the decoder provides.
19. **[Cosmetic]** Finding 14 — R12BW parity inversion not present (mode not implemented anyway).
20. **[Cosmetic]** Finding 15 — `lookup` returns None for unknown codes (matches C semantics).
21. **[Cosmetic]** Finding 17 — `working_samples_emitted` not decremented on residual transfer.
22. **[Cosmetic]** Finding 22 — `working_samples_emitted` counter not decremented on take_residual.
23. **[Cosmetic]** Finding 23 — `tones.remove(0)` is O(n) per window (n=14).

---

## Section 5 — What you DIDN'T find

Things I expected to be divergences but were actually clean:

- **YCbCr→RGB matrix coefficients** (`100/140/-17850`, `100/-71/-33/13260`, `100/178/-22695`) match slowrx's `video.c:447-450` exactly. The unit tests `ycbcr_neutral_grey_is_grey` / `ycbcr_pure_red` validate the matrix.
- **Luminance scale slope** (`3.137_254_9 = (2300 - 1500) / 255`) matches slowrx's hardcoded `3.1372549`.
- **PD channel order** (Y(odd) → Cr → Cb → Y(even)) matches slowrx exactly.
- **PD120 / PD180 timing constants** (`SyncTime`, `PorchTime`, `PixelTime`, `LineTime`) match slowrx row-for-row.
- **Gaussian peak interpolation formula** (`MaxBin + log(P[k+1]/P[k-1]) / (2*log(P[k]^2 / (P[k+1]*P[k-1])))`) matches slowrx's `video.c:391-394`.
- **VIS LSB-first bit ordering** (Rust: `code |= bit << i`; C: `Bit[0] + (Bit[1]<<1) + ...`) matches.
- **Even-parity convention** (XOR of all 7 data bits = parity bit) matches.
- **PD120 / PD180 VIS codes** (0x5F / 0x60) match.
- **Hann window formula** (`0.5 * (1 - cos(2π*i/(N-1)))`) matches slowrx in `mode_pd::hann_window` (though VIS lacks Hann entirely — Finding 1).
- **PD even-line/odd-line image row mapping** (pair_index*2, pair_index*2+1) matches the C `(y, y+1)` convention.

What I noticed but didn't dig into deeper:

- The `match_vis_pattern` accepts any window where `tones[5..12]` are all `BitZero` or `BitOne`, but does not enforce a tone *change* between bits. A 7-bit pattern of all `BitZero` (which represents VIS code 0x00) would be accepted. C accepts 0x00 too (VISmap[0]=0=UNKNOWN, which Listen() loops on). So this is identical behavior — but a noisy-decoder-stuck-at-1300-Hz scenario could falsely "succeed" at parity check, matching C's behavior. Probably fine.
- I did not verify the sign of slowrx's HedrShift application beyond the literal `(1500 + HedrShift)` in `video.c:406`. I'm assuming positive HedrShift means "radio is tuned high", which matches the leader observation `tone[0+j] - 1900` where tone > 1900 → HedrShift > 0.
- I did not deeply audit the resample.rs FIR coefficient correctness — the tests verify tones survive at expected frequencies, which is a sufficient sanity check for the audit.
- I did not verify the `take_residual_buffer` interaction with state-machine resets in detail.

---

**Conclusion (informational, not a fix proposal):** the leading candidates for the 6/7 real-audio failure are Findings 2, 3, and 5 — all VIS-detection-side. The pattern of "0 ImageComplete events" (rather than "1 partial decode") strongly implies the decoder is not entering the Decoding state at all, which is consistent with VIS detection failing due to non-boundary-aligned bursts (Finding 2) on radios with non-zero HedrShift (Findings 3 and 5). Fixing those should be the first investigation. Findings 7 / 8 / 10 (slant + Skip + rate) become relevant only after VIS detection is fixed, but they would explain why even *clean* captures fail to reach ImageComplete on long modes (PD180).

Total findings: 23 across all 7 source files. Critical: 3. Important: 10. Minor: 5. Cosmetic: 5.