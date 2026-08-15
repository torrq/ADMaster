# Audio Description Processing Pipeline

This repository contains robust, automated Python scripts for processing, synchronizing, and loudness-normalizing Audio Description (AD) tracks for movies and TV shows.

It is designed to be fully portable on Windows. Simply place `ffmpeg.exe` and `ffprobe.exe` in the `bin/` directory, and the scripts will automatically use them without requiring a system-wide installation.

## Core Tools

### 1. Universal Acoustic Sync & Main Audio Conversion (`sync_ad_track.py`)
Automatically aligns out-of-sync AD tracks (MP3, AC3, M4A, etc.) to a reference movie audio track and optionally converts/downmixes main audio in a single pass.
* **100-Anchor FFT Cross-Correlation:** Uses dynamic multi-anchor 8kHz PCM FFT cross-correlation in memory to find sub-millisecond delay offsets in seconds.
* **Candidate Clustering & Runner-Ups:** Groups measurements within a $\pm 30\text{ms}$ sub-frame tolerance to calculate a robust consensus winner and displays up to 2 distinct runner-up offsets with confidence ratings.
* **Percentage & Raw Confidence:** Reports match confidence both as a clear percentage and normalized peak score (e.g. `88.9% (0.8887)`).
* **Automatic Working Directory Routing:** Places temporary analysis WAVs in `<movie_dir>/scratch/` (with atomic write protections) and renders final encodes to `<movie_dir>/output/`.
* **Standard Multi-Track Encoding:**
  * **AD Tracks:** Outputs both `-AD (Stereo).m4a` (with `alimiter` clipping protection) and `-AD (Night Mode).m4a` (EBU R128 `loudnorm` -16 LUFS).
  * **Main Audio (`--convert-main`):** Converts 5.1/7.1 sources to `(Surround).ac3` (5.1 384k), `(Stereo).m4a` (downmix + `alimiter`), and `(Night Mode).m4a` (downmix + `loudnorm`).
* **Smart Pitch & Speed Corrections:** Automatically detects and reverses PAL (4% speedup) or NTSC pulldown tempo shifts.
* **Test Previews:** Multiplexes short MKV samples with video so you can visually verify lip-sync before rendering the full movie.

**Example Usage:**
```powershell
# Full movie sync & main audio conversion in 1 command:
python scripts/sync_ad_track.py -r "F:\uploads\Movie\source\Movie.dts" -a "F:\uploads\Movie\tosync\Movie_AD.mp3" --convert-main

# Fast 3-minute video lip-sync test:
python scripts/sync_ad_track.py -r "Movie.eac3" -a "AD.mp3" -m "Movie.mkv" --preview 180
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
When `describealign` Dynamic Time Warping is used for complex alignment (episodes with commercial edits, scene insertions), the resulting audio splices can sound harsh — cutting mid-word or creating audible clicks at segment transitions.

The smooth stitcher solves this with a **3-layer transition pipeline** applied to describealign's alignment data:

1. **VAD-Aware Cut Nudging** — Detects active speech at each cut boundary using RMS energy analysis. If a cut falls mid-word, it's automatically nudged to the nearest silence gap (±500ms search window, 200ms hangover).
2. **Zero-Crossing Snapping** — Each cut is micro-adjusted (±2ms) to land on a waveform zero crossing, eliminating DC-offset pops.
3. **Equal-Power Crossfade** — 50ms cosine-shaped crossfades at every transition ensure smooth blending without volume dips.

**Example Usage:**
```powershell
# Standalone (from describealign alignment plot):
python scripts/dtw_smooth_stitch.py alignment.txt AD_track.mka output.m4a

# Automatic (via batch orchestrator — no extra steps needed):
python scripts/batch_orchestrator.py
```

### 5. Full Scripts Index
Please refer to `scripts/README.md` for a comprehensive, up-to-date index of all core pipeline, prototyping, and legacy scripts included in this repository.

## Setup & Portability
This tool is self-contained. 
1. Create a `bin/` folder in this directory.
2. Drop `ffmpeg.exe` and `ffprobe.exe` inside.
3. Run the scripts using Python 3.x with `numpy` installed.

## BatchEncoder Integration
For GUI-based bulk processing, this project pairs excellently with **BatchEncoder**. We maintain a custom `FFMPEG_BLURAY_AAC.xml` format preset that replicates these exact downmix and loudness matrices (via a 1-pass approximation) directly inside the BatchEncoder UI.

*(For detailed architectural rules, LLM-agent instructions, and pipeline limitations, refer to `AGENTS.md`)*
