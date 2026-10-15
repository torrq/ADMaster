# Fast Audio Description Sync Testing & Verification Protocol

## 1. Overview

This document outlines the standard protocol and command-line flags for rapid testing, diagnostic inspection, and synchronization verification in the ADMaster pipeline.

When diagnosing sync issues or tuning alignment parameters, running full AAC encoding and 2-pass EBU R128 loudness normalization (`loudnorm`) across a 2-hour to 3-hour feature adds 4 to 8 minutes of unnecessary encoding latency per iteration.

By combining uncompressed PCM WAV deliverables (`--wav`), disabling night mode loudness filters (`--no-night`), and leveraging scratch caching (`sync_params.json` and ADSync cached warps), sync iterations complete in seconds rather than minutes.

---

## 2. Fast Iteration Command Reference

### Recommended Fast Test Command
```powershell
python scripts/sync_ad_track.py "F:\uploads\<MovieFolder>" --wav --no-night --overwrite --keep-scratch
```

### Key CLI Flags for Testing

| Flag | Purpose | Testing Benefit |
| :--- | :--- | :--- |
| `--wav` | Outputs uncompressed 16-bit 48kHz PCM WAV for stereo deliverables instead of AAC (`.m4a`). | Eliminates FFmpeg AAC encoding overhead; renders stereo deliverables in 10-15 seconds; opens instantly in audio editors (Audacity, Reaper) and Python NumPy without decoding. |
| `--no-night` | Disables the Night Mode (`loudnorm -16 LUFS`) variant entirely. | Bypasses the CPU-intensive 1-pass/2-pass EBU R128 filter, saving 3 to 5 minutes per run on full features. |
| `--clean-scratch` / `--wipe-scratch` | Wipes the entire `scratch/` directory before processing begins. | Ensures a 100% clean run from scratch without using cached correlation parameters, predecoded WAVs, or ADSync reports. |
| `--overwrite` / `-f` | Forces re-rendering of deliverables in `output/` even if files exist. | Re-renders output deliverables while reusing existing cached scratch data (such as `sync_params.json` and `synced_ad.wav`). |
| `--keep-scratch` | Preserves all intermediate WAVs, CSV logs, alignment plots, and `report.json` in `scratch/`. | Allows deep inspection of cut points, waveforms, RMS energy profiles, and gap-fill boundaries. |
| `--preview [sec]` | Limits analysis and audio rendering to the first N seconds (e.g. `--preview 180`). | Provides a lightning-fast 3-minute sample test for quick offset verification. |
| `--force-dtw` | Bypasses the 100-anchor cross-correlation search and forces warp alignment directly. | Accelerates testing on known alternate cuts or broadcast captures with commercial excisions. |

---

## 3. Scratch Caching Architecture

The pipeline implements multi-stage caching in `<movie_dir>/scratch/` to avoid repeated expensive operations across test runs:

### Stage 1: Pre-Decoded Mono 8kHz WAVs
- Files: `REF_..._mono8k.wav`, `<name>_mono8k.wav`.
- Purpose: Eliminates repeated linear decodes of multi-gigabyte raw bitstreams (DTS-HD MA, TrueHD, AC3). Sliced directly in memory for FFT cross-correlation.

### Stage 2: Cross-Correlation Parameters (`sync_params.json`)
- File: `<movie_dir>/scratch/sync_params.json`.
- Contents: Per-track JSON dictionary storing `offset`, `atempo`, `confidence`, `method` (`cross_correlation` vs `dtw`), and `is_pal`.
- Behavior:
  - If `sync_params.json` exists in `scratch/`, `compute_sync_parameters()` loads the parameters instantly, bypassing the 100-anchor FFT convolution loop.
  - Passing `--clean-scratch` wipes this file for a pristine re-run.

### Stage 3: ADSync Warp Cache (`synced_ad.wav` & `report.json`)
- Location: `<movie_dir>/scratch/adsync/<track_name>/`.
- Files: `synced_ad.wav`, `report.json`, `plots/`.
- Behavior:
  - When re-running with `--overwrite` (for example, to switch between `--wav` and AAC deliverables), the existing warped audio is reused directly, avoiding redundant 30-second ADSync pipeline executions.
  - Passing `--clean-scratch` wipes this folder for a fresh warp calculation.

---

## 4. Verification & Diagnostic Protocol

When testing a newly synchronized track, verify the three common defect signatures:

### 1. Head of Speech Truncation (Boundary Nudged Too Close)
- **Symptom**: The first syllable or consonant of a narration sentence is clipped or cut off (e.g. at excised scene boundaries or DTW cuts).
- **Cause**: Boundary nudging snapped to an intra-word plosive closure (p, t, k) rather than true sentence silence.
- **Rule**: Silence detection requires a sustained silence run of >= 200ms with RMS < 0.008 (-42 dBFS), snapping to the center of the silence run with at least a 300ms leading margin before speech onset.

### 2. Audio Doubling / Grain Repetition
- **Symptom**: Syllables or background sounds stutter or repeat at a cut point.
- **Cause**: ADSync warp discontinuity threshold was set too high (e.g. 2.0s), causing the algorithm to compress large jumps through WSOLA time-stretching instead of making a discrete segment cut.
- **Rule**: `warp_discontinuity_threshold` must be set to 0.8s. Within each continuous segment, audio runs at 100% native 1.000000x speed without grain overlap stuttering.

### 3. Missing Audio Attack / False Scene Void
- **Symptom**: Music attacks or quiet ambient audio are suddenly muted or replaced with master soundtrack audio.
- **Cause**: `scan_silent_voids` flagged a quiet pause in the AD track as an "excised scene void".
- **Rule**: `scan_silent_voids` must be set to `False` for ADSync and single-offset tracks. Gap filling is only applied to explicit structural segment cuts where AD narration energy is verified absent (peak < 0.015, windowed RMS < 0.005).

---

## 5. Python Waveform Inspection Snippet

To verify audio around a specific timestamp (e.g. 00:47:24.00) without opening a heavy DAW:

```python
import soundfile as sf
import numpy as np

# Load 10 seconds around target timestamp
target_sec = 2844.0  # 00:47:24
sr = 48000
start_sample = int((target_sec - 5.0) * sr)
end_sample = int((target_sec + 5.0) * sr)

data, fs = sf.read("Clockwork-AD (Stereo).wav", start=start_sample, stop=end_sample)
rms = np.sqrt(np.mean(data**2, axis=1)) if data.ndim == 2 else np.sqrt(np.mean(data**2))
peak = np.max(np.abs(data))

print(f"Sample range: {start_sample} -> {end_sample}")
print(f"Peak amplitude: {peak:.4f}")
print(f"Average RMS   : {np.mean(rms):.6f}")
```

---

## 6. Testing Workflow Summary

1. **Step 1 -- Fast Sync Test**:
   ```powershell
   python scripts/sync_ad_track.py "F:\uploads\<Movie>" --wav --no-night --overwrite --keep-scratch
   ```
2. **Step 2 -- Verify Cut Timestamps**:
   Inspect the double-check cut times printed in the final sync summary table.
3. **Step 3 -- Audition Key Scenes**:
   Load `<Movie>-AD (Stereo).wav` in an audio editor or media player and check:
   - Dialogue onset clarity at segment transitions.
   - Pacing and absence of syllable stuttering.
   - Smooth 50ms crossfading at music/soundtrack splice points.
4. **Step 4 -- Final Production Render**:
   Once alignment is verified perfect, run full production encoding:
   ```powershell
   python scripts/sync_ad_track.py "F:\uploads\<Movie>" --convert-main --overwrite
   ```
