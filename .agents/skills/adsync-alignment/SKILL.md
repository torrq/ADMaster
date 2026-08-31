---
name: adsync-alignment
description: High-speed acoustic landmark fingerprinting, Viterbi lattice DP, continuous PCHIP warping, and automatic speed detection via ADSync.
---

# ADSync Audio Warping & Alignment Guide

## Core Purpose
High-performance non-linear time warping and acoustic alignment engine developed by JohnnyTheCoder1. Uses landmark constellation hashing (Shazam-style), 3-band speech suppression, Viterbi dynamic programming on candidate lattices, continuous monotone PCHIP cubic spline warping, and WSOLA pitch-preserved rendering in **15-30 seconds**.

---

## Python API Usage
```python
import adsync._pipeline as ad_pipe
import adsync.config as ad_conf
from pathlib import Path

cfg = ad_conf.SyncConfig(
    mode="warp",            # 'warp' (non-linear PCHIP spline) or 'fingerprint' / 'cross_corr'
    speed_detect=True,      # Probes 25fps PAL transfers automatically in ~2s
    fingerprint=True,       # Landmark constellation hashing
    crossfade_ms=50,        # Transition envelope
    output_sr=48000,        # Standard audio sample rate
    analysis_sr=16000,      # Feature analysis sample rate
    multiband=True,         # 3-band filtering (downweights 250-1200Hz human speech to 0.2)
)

report = ad_pipe.run_pipeline(
    video_path=Path('reference_video_or_audio.mkv'),
    ad_path=Path('ad_track.mp3'),
    output_path=None,       # None outputs synced_ad.wav in debug_dir
    config=cfg,
    debug_dir=Path('scratch/adsync/track_name'),
    mux=False,
)
# Output rendered to: scratch/adsync/track_name/synced_ad.wav
```

---

## Standalone CLI Usage
```powershell
adsync align --mode warp <reference_video_or_audio> <ad_track> -o <output_synced.m4a>
```

---

## Critical Rules & Capabilities for LLMs

1. **Continuous Spline Warping (No Mid-Speech Slices)**:
   * ADSync maps time continuously via PCHIP cubic splines rather than slicing audio into piecewise blocks.
   * Eliminates click transients, syllable clipping, and transition glitches across structural cuts.
2. **Multiband Speech Suppression**:
   * Low (<250Hz) and High (>1200Hz) frequencies carry 1.0/0.8 weights. Dialogue (250-1200Hz) is suppressed to 0.2 weight so cross-correlation locks onto background music and sound effects instead of narrator vocals.
3. **Automatic PAL / Speed Probing**:
   * Automatically detects 25fps vs 23.976fps speed differences across 2-second landmark hashes and applies exact rate/pitch restoration without requiring `[UK]` filename clues.
4. **RF64 >4GB WAV Handling**:
   * For long features (>3h06m / 4.2GB 32-bit WAVs), ensure FFmpeg uses `-rf64 auto` during extraction to prevent 32-bit RIFF WAV truncation.
