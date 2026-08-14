# Audio Description Pipeline Scripts

This directory contains the production-ready automation scripts for the Audio Description sync, main audio conversion, and TV episode processing pipeline.

## Core Production Tools

* **`sync_ad_track.py`** - Master universal acoustic sync & encoding tool. Computes 100-anchor in-memory FFT correlation, candidate clustering ($\pm 30\text{ms}$ sub-frame tolerance), dual AD variant rendering (`Stereo` + `Night Mode`), and triple Main audio conversion (`--convert-main`: 5.1 AC3 Surround, 2.0 Stereo AAC, 2.0 Night Mode AAC).
* **`batch_orchestrator.py`** - Master TV batch orchestrator. Automatically determines the optimal sync strategy (Direct Mux vs Frame-Perfect Black Screen Cuts vs DTW `describealign`) per episode.
* **`batch_frameperfect.py`** - Automates Frame-Perfect FFmpeg cut workflow across MKV episodes to remove black screen commercial breaks without keyframe snapping.
* **`remove_commercial_breaks.py`** - Scans MKVs for black screens using FFmpeg `blackdetect` and generates slice timings.
* **`convert_ad_2pass.py`** - Performs standalone 2-pass EBU R128 Loudnorm normalization on 5.1/7.1 audio tracks without alignment.
* **`build_smart_ad_pipeline.py`** - Voice Activity Detection (VAD) on the center channel to preserve narrator speech tails over abrupt scene transitions.
* **`process_extended_ad_option_a.py`** - Reconstructs seamless branching AD tracks for extended cuts with 50ms equal-power crossfades.
* **`parse_mpls.py`** - Parses Blu-ray `.mpls` playlist files to extract 45 kHz PTS timestamps.

## Archived & Reference Examples (`scripts/archive/`)
All historical one-off runners, prototyping scripts, and reference implementations have been sanitized with generic titles/paths and organized in the **`scripts/archive/`** folder.
