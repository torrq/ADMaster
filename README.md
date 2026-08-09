# Audio Description Processing Pipeline

This repository contains robust, automated Python scripts for processing, synchronizing, and loudness-normalizing Audio Description (AD) tracks for movies and TV shows.

It is designed to be fully portable on Windows. Simply place `ffmpeg.exe` and `ffprobe.exe` in the `bin/` directory, and the scripts will automatically use them without requiring a system-wide installation.

## Core Tools

### 1. Acoustic Cross-Correlation Sync (`sync_ad_track.py`)
Automatically aligns out-of-sync AD tracks (MP3, AC3, etc.) to a reference movie audio track. 
* **Fast & Accurate:** Uses multi-point 8kHz PCM cross-correlation to find sub-millisecond delay offsets in under a second.
* **Smart Corrections:** Automatically detects and reverses PAL (4% speedup) or NTSC pulldown pitch/tempo shifts.
* **Broadcast Standard Loudness:** Applies a rigorous 2-pass EBU R128 loudness normalization (Standard and NightMode presets) to ensure dialogue is perfectly leveled.
* **Test Previews:** Multiplexes short 3-minute MKV samples with video so you can visually verify lip-sync before rendering the full movie.

**Example Usage:**
```powershell
python scripts/sync_ad_track.py -r "Movie.eac3" -a "New Description=AD.mp3" -o "out_dir" --preset all
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

### 4. Full Scripts Index
Please refer to `scripts/README.md` for a comprehensive, up-to-date index of all core pipeline, prototyping, and legacy scripts included in this repository.

## Setup & Portability
This tool is self-contained. 
1. Create a `bin/` folder in this directory.
2. Drop `ffmpeg.exe` and `ffprobe.exe` inside.
3. Run the scripts using Python 3.x with `numpy` installed.

## BatchEncoder Integration
For GUI-based bulk processing, this project pairs excellently with **BatchEncoder**. We maintain a custom `FFMPEG_BLURAY_AAC.xml` format preset that replicates these exact downmix and loudness matrices (via a 1-pass approximation) directly inside the BatchEncoder UI.

*(For detailed architectural rules, LLM-agent instructions, and pipeline limitations, refer to `AGENTS.md`)*
