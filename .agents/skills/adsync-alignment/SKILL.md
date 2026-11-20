---
name: adsync-alignment
description: High-speed acoustic landmark fingerprinting, Viterbi lattice DP, continuous PCHIP warping, and automatic speed detection via ADMaster's native Rust ADSync engine.
---

# ADSync Audio Warping & Alignment Engine (Native Rust)

## 1. Core Architecture & Overview
**ADSync** (by Anilo / JohnnyTheCoder1, MIT License) is the foundational non-linear audio synchronization and time-warping architecture adapted into pure Rust inside `ADMaster` (`src/dsp/`).

Unlike legacy piecewise DTW tools (which chop audio into discrete segments and crossfade cut boundaries), the ADSync architecture computes a **continuous monotone warping spline** and renders the retimed audio smoothly in streaming chunks. This eliminates mid-speech cutoffs, clipped syllables, and splice transients while keeping peak RAM under **55 MB** even on 3.5+ hour feature films.

---

## 2. Native Rust Pipeline Stages (`src/dsp/`)

1. **Media Probing (`src/media.rs`)**: Uses `ffprobe` to inspect duration, channel layout, audio streams, and sample rate.
2. **Atomic Audio Extraction (`src/media.rs`)**: Extracts 8 kHz mono analysis WAVs and 48 kHz stereo HQ WAVs atomically (`_tmp.wav` -> rename) using `-nostdin` and `-rf64 auto` to prevent >4 GB RIFF truncation.
3. **3-Band Speech Suppression (`src/dsp/filters.rs`)**: 4th-order zero-phase Butterworth SOS `filtfilt` cascade splits audio into Low (`< 250 Hz`, weight `1.0`), Mid/Speech (`250 - 1200 Hz`, weight `0.2`), and High (`> 1200 Hz`, weight `0.8`).
4. **Acoustic Landmark PAL Speed Probing (`src/dsp/fingerprint.rs`)**: Extracts STFT spectral peak-pair constellation hashes with 70th-percentile peak flooring in a streaming ring buffer and evaluates single-pass `O(n_matches)` bucket histograms at native `1.0x` (`23.976 fps`) and PAL `1.042709x` (`25.0 fps`) without relying on `[UK]` filename tags.
5. **100-Anchor Packed SIMD FFT Correlation (`src/dsp/correlation.rs`)**: Evaluates 100 anchors across `5%..88%` of the timeline in parallel via `rayon` using thread-local cached 5-smooth `f32` SIMD FFTs with 2-real-in-1 Hermitian packing and parabolic sub-sample peak interpolation.
6. **Second-Order Viterbi Lattice & Monotone PCHIP Warping (`src/dsp/warp.rs`)**: Pre-computes 4 kHz 3-band signals, runs fingerprint-guided tight interior search + second-order Viterbi dynamic programming (penalizing offset jumps and slope curvature), filters out spurious micro-blip spurs, nudges structural cut boundaries into narrator silence windows, and fits **Monotone Fritsch-Carlson PCHIP Cubic Splines**.
7. **Streaming Catmull-Rom Warp Render (`src/dsp/warp.rs`)**: Renders synced 48 kHz stereo audio in 30-second streaming chunks via 4-point Catmull-Rom cubic interpolation (`< 35 MB` peak RAM).
8. **Windowed Reference Gap Filling (`src/dsp/gap_fill.rs`)**: Detects excised scenes / timeline voids (`>= 0.25 s`) using spline-endpoint offset analysis and splices level-matched (`-2.0 dB` RMS capped) reference soundtrack audio with 50 ms equal-power cosine crossfades (`< 15 MB` RAM).

---

## 3. Critical Invariants & Bug Fixes

### A. 4 GB RIFF WAV Limit (>3h06m)
* Standard 32-bit RIFF WAV files hit 4.0 GB at `11,184.81 s` (`03:06:24`) for 48 kHz 32-bit float stereo.
* **Fix**: All 48 kHz WAV extractions pass `-rf64 auto` to FFmpeg so multi-hour features (e.g. 4+ hour epics) decode completely without truncation.

### B. Sign Convention Between Correlation and Warp
* `correlation.rs` outputs `winner_offset = t_ad - t_vid`.
* `warp.rs` lattice offsets are `t_vid - t_ad`.
* `pipeline.rs` MUST pass `-winner_offset` as `offset_hint` into `warp.rs` so the Viterbi search window centers on the true match.

### C. Never Force PAL From Filename Tags
* In `PalMode::AutoDetect`, always decode at native `1.0x` first and run `detect_speed_pal`. Only apply PAL slowdown when landmark probing confirms a true ~1.0427x speedup or `--pal` is explicitly requested.

---

## 4. CLI Usage in `ADMaster`

```powershell
# Auto-select Correlation or Time-Warp fallback:
admaster-cli "F:\Movies\ExampleMovie" --overwrite

# Force non-linear Viterbi + PCHIP Time-Warp explicitly:
admaster-cli "F:\Movies\ExampleMovie" --force-warp --overwrite

# Enforce native 1.0x speed (disable PAL slowdown) with Time-Warp:
admaster-cli "F:\Movies\ExampleMovie" --force-warp --no-pal --overwrite
```
