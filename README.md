# adsync — Audio Description & Secondary Audio Sync Pipeline

This repository contains robust, automated Python scripts for processing, synchronizing, and loudness-normalizing Audio Description (AD) tracks for movies and TV shows.

It is designed to be fully portable on Windows. Simply place `ffmpeg.exe` and `ffprobe.exe` in the `bin/` directory, and the scripts will automatically use them without requiring a system-wide installation.

## Core Tools

### 1. Universal Acoustic Sync & Main Audio Conversion (`sync_ad_track.py`)
Automatically aligns out-of-sync AD tracks (MP3, AC3, M4A, etc.) to a reference movie audio track and optionally converts/downmixes main audio in a single pass.
* **Directory Auto-Discovery:** Point to any folder containing `source/` (reference MKV/audio) and `tosync/` (AD MP3s/M4As), and the tool automatically pairs, analyzes, and renders deliverables to `output/`.
* **100-Anchor FFT Cross-Correlation:** Uses dynamic multi-anchor 8kHz PCM FFT cross-correlation in memory to find sub-millisecond delay offsets in seconds.
* **DescribeAlign DTW Integration:** Automatically triggers or forces (`--force-dtw`) Dynamic Time Warping piecewise alignment for alternate cuts with 3-layer smoothing (VAD + Zero-Crossing + 50ms Crossfade) and bit-perfect PCM passthrough.
* **Stream Selection (`--ref-stream`):** Choose specific audio tracks from multi-track MKVs (e.g. `--ref-stream 0:a:1`).
* **Dynamic Sample-Rate PAL Handling (`--pal` / `--no-pal`):** Computes exact `asetrate` sample clock manipulation based on probed input sample rates (44.1k vs 48k). Use `--no-pal` to enforce native 1.0x speed on shorter Theatrical Cuts.
* **Standard Multi-Track Encoding:**
  * **AD Tracks:** Outputs both `-AD (Stereo).m4a` (with `alimiter` clipping protection) and `-AD (Night Mode).m4a` (EBU R128 `loudnorm` -16 LUFS).
  * **Main Audio (`--convert-main`):** Converts 5.1/7.1 sources to `(Surround).ac3` (5.1 640k), `(Stereo).m4a` (true 2.0 downmix via ITU-R BS.775 matrix + `alimiter`), and `(Night Mode).m4a` (true 2.0 downmix + `loudnorm`).
* **Test Previews:** Multiplexes short MKV samples with video so you can visually verify lip-sync before rendering the full movie.

**Example Usage:**
```powershell
# 1. Directory Auto-Discovery (AD sync + Main 5.1 & True 2.0 AAC conversion):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --convert-main

# 2. Specific audio stream with forced DTW alignment (e.g. Theatrical vs Extended Cut):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie2" --ref-stream 0:a:1 --force-dtw --no-pal

# 3. Fast 3-minute video lip-sync test:
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --preview 180
```

### 2. Standalone 2-Pass Converter (`convert_ad_2pass.py`)
A fast utility to convert standard 5.1/7.1 AD tracks (such as raw Blu-ray AC3/DTS/TrueHD) into stereo AAC using broadcast-standard 2-pass loudness normalization. 
It bypasses the sync logic and simply applies the **Standard** and **NightMode** EBU R128 profiles.

**Example Usage:**
```powershell
python scripts/convert_ad_2pass.py "F:\uploads\Movie\Movie_AD.ac3"
```

### 3. Seamless Branching Reconstruction (`build_smart_ad_pipeline.py` & `process_extended_ad_option_a.py`)
For Blu-rays with multiple cuts (e.g., Theatrical vs Extended), these scripts rebuild AD tracks to match the Extended cut using the Theatrical AD track as a base.
* Parses Blu-ray `.mpls` files (`parse_mpls.py`) to align PlayItem PTS timestamps.
* Uses Voice Activity Detection (VAD) via center-channel RMS energy to preserve narrator speech tails that cross scene boundaries.
* Automatically crossfades in the standard 5.1/7.1 audio for inserted scenes where no AD exists.

### 4. DTW Smooth Stitcher (`dtw_smooth_stitch.py`)
When `describealign` Dynamic Time Warping is used for complex alignment (episodes with commercial edits, scene insertions, or rate-stretched cuts), the smooth stitcher applies a **3-layer transition pipeline** to describealign's alignment data:

1. **VAD-Aware Cut Nudging** — Detects active speech at each cut boundary using RMS energy analysis. If a cut falls mid-word, it's automatically nudged to the nearest silence gap (±500ms search window, 200ms hangover).
2. **Zero-Crossing Snapping** — Each cut is micro-adjusted (±2ms) to land on a waveform zero crossing, eliminating DC-offset pops.
3. **Equal-Power Crossfade & Sinc Rate Resampling** — Preserves 100% bit-perfect native PCM samples without interpolation for standard-speed audio, applies bandlimited sinc resampling (`scipy.signal.resample`) for genuine rate-stretch segments (e.g. PAL $\to$ Film), and blends segment junctions using 50ms cosine-shaped constant-power crossfades ($\cos^2 + \sin^2 = 1$).

### 5. Piecewise Reel-Aware Synchronization (35mm Film Scans)
For archival 35mm film scans where physical reel changes (every ~20 minutes) have missing or damaged frames:
* Identifies discrete offset clusters across the film using sub-millisecond FFT cross-correlation.
* Preserves **100% native speed (1.000000x) and pitch** across every individual reel without artificial time-stretching.
* Finds natural silence gaps at scene transitions near reel changes and applies equal-power cosine crossfades.

---

## Alignment Strategy: Anchor Correlation vs. DescribeAlign DTW

Both alignment methods serve complementary purposes depending on release characteristics:

| Feature / Metric | 100-Anchor Cross-Correlation (Default) | DescribeAlign Dynamic Time Warping (DTW) |
| :--- | :--- | :--- |
| **Best Used For** | Identical cuts (same edit, different audio delay/master). | Alternate cuts (Theatrical vs Extended, TV trims, commercials). |
| **Audio Integrity** | **100% Bit-Perfect Linear Passthrough:** Shifts the track as one continuous stream. Zero splicing, zero pitch/tempo warps. | **Piecewise Reconstruction:** Slices into segments, stretches/compresses intervals, and crossfades boundaries. |
| **Precision** | Sub-millisecond sample accuracy ($\pm 1\text{ms}$). | Quantized time step windows (~100ms resolution). |
| **Execution Time** | **~2–4 seconds** via in-memory 8kHz FFT arrays. | **1–3 minutes** per track (feature extraction + LP optimization). |
| **Diagnostic Role** | Consensus score immediately validates whether cuts match ($\ge 50\%$) or differ ($< 10\%$). | Engages automatically when anchor score confirms cut mismatch. |

### When to Use Which:
* **Use Anchor Method (Default):** Always try first for standard releases. It provides instantaneous sync without altering the acoustic timbre, pitch, or dynamics of the narrator's track.
* **Use DescribeAlign (`--force-dtw`):** Use when syncing an AD track from an alternate edition (e.g. aligning a Theatrical Cut AD track onto an Extended Cut Blu-ray). DescribeAlign navigates scene insertions and trims, while `dtw_smooth_stitch.py` ensures transparent transitions.

---

### 6. Full Scripts Index
Please refer to `scripts/README.md` for a comprehensive, up-to-date index of all core pipeline, prototyping, and legacy scripts included in this repository.

## Setup & Portability
This tool is self-contained and requires zero hardcoded paths. 
1. Create a `bin/` folder in this directory and drop `ffmpeg.exe` and `ffprobe.exe` inside (or ensure they are on your system `PATH`).
2. Run the scripts using Python 3.x with `numpy` and `scipy` installed.

### Optional Local Configuration (`config.local.json`)
You can customize binary paths, default threads, and targets without modifying scripts:
1. Copy `config.default.json` to `config.local.json` (which is `.gitignore`d).
2. Override any paths or defaults:
```json
{
  "ffmpeg_path": "C:\\Tools\\ffmpeg\\ffmpeg.exe",
  "ffprobe_path": "C:\\Tools\\ffmpeg\\ffprobe.exe",
  "describealign_path": "C:\\Tools\\describealign\\describealign.py",
  "default_threads": 4,
  "default_target_i": -16.0,
  "default_target_tp": -1.5
}
```

## BatchEncoder Integration
For GUI-based bulk processing, this project pairs excellently with **BatchEncoder**. We maintain a custom `FFMPEG_BLURAY_AAC.xml` format preset that replicates these exact downmix and loudness matrices (via a 1-pass approximation) directly inside the BatchEncoder UI.

*(For detailed architectural rules, LLM-agent instructions, and pipeline limitations, refer to `AGENTS.md`)*
