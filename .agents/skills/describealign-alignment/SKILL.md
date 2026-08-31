---
name: describealign-alignment
description: Dynamic Time Warping (DTW) audio alignment using describealign.py for non-linear cuts, alternate versions, and commercial removal.
---

# DescribeAlign DTW Audio Alignment Guide

## Core Purpose
Aligns secondary audio (Audio Description, commentary, dubs) to a master video/audio timeline when tracks have **differing structural edits** (Theatrical vs Extended cuts, commercial breaks, censorship trims). Formulates piecewise DTW as a linear programming optimization problem (`scipy.optimize.linprog`).

---

## Command Syntax
```powershell
python describealign.py --yes --stretch_audio <master_wav> <ad_wav> --alignment_dir <plot_dir> --output_dir <out_dir>
```

### Key Parameters:
* `--yes`: **Mandatory for automation.** Skips interactive user confirmation prompts.
* `--stretch_audio`: Warps the AD track to match the reference master timeline.
* `--alignment_dir`: Target directory for timestamp mapping files (`<filename>.txt`).
* `--output_dir`: Target directory for muxed preview outputs.

---

## Critical Rules for LLMs

1. **Pre-Decode to WAV First**:
   * **NEVER pass large video MKVs directly to DescribeAlign.** It causes FFmpeg stdin/stdout pipe buffer deadlocks.
   * Pre-decode both master and AD audio to 48kHz 16-bit mono WAVs in a `scratch/` directory before calling `describealign.py`.
2. **Reverse Track Order Gotcha**:
   * DescribeAlign outputs the AD track as `a:0` and Master Audio as `a:1` (reverse of broadcast standard). Always swap them when muxing.
3. **Mandatory Post-Processing (`dtw_smooth_stitch.py`)**:
   * DescribeAlign's raw output contains mid-speech cuts, 100ms quantization jitter, and silent voids on deleted scenes.
   * Always post-process the generated `.txt` alignment plot using `dtw_smooth_stitch.py`:
     ```powershell
     python scripts/dtw_smooth_stitch.py <plot.txt> <source_ad.wav> <output.m4a> --master <ref_wav>
     ```
   * This applies **Plateau Consolidation** (drops 85% of phantom cuts), **Master Audio Gap Filling** (fills deleted scenes with movie soundtrack), **VAD Speech Nudging**, **Zero-Crossing Snapping**, and **50ms Cosine Crossfades**.
