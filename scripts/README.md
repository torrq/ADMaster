# adsync Pipeline Scripts

This directory contains the production-ready tools for acoustic audio description synchronization, video stream commercial break excision, multi-channel surround downmixing, MKV metadata manipulation, and media server automation.

---

## Tool Summary & Capability Matrix

| Script | Primary Responsibility | Modifies Video? | Input Types | Output Deliverables |
| :--- | :--- | :---: | :--- | :--- |
| **[`sync_ad_track.py`](#1-sync_ad_trackpy-master-sync--encoder)** | Universal acoustic alignment & audio encoding | ❌ No | MKV, MP4, DTS, AC3, FLAC, MP3, WAV | Aligned `-AD (Stereo).m4a`, `-AD (Night Mode).m4a`, `(Surround).ac3`, `(Stereo).m4a` |
| **[`batch_orchestrator.py`](#2-batch_orchestratorpy-tv-season-orchestrator)** | Automated heuristic TV series strategy router | ⚠️ Yes (if needed) | Directory of MKV episodes + AD tracks | Synced MKVs via Direct Mux, Frame-Perfect Cut, or DTW |
| **[`dtw_smooth_stitch.py`](#3-dtw_smooth_stitchpy-dtw-transition-smoother)** | 3-Layer DTW transition smoother | ❌ No | Alignment map + Source AD audio | Stitched, smooth AD audio with VAD + Crossfades |
| **[`batch_frameperfect.py`](#4-batch_frameperfectpy-commercial-break-cutter)** | Surgical video/audio commercial gap excision | ✅ Yes | MKVs with black commercial gaps | Clean MKV without black gaps, frame-accurate |
| **[`remove_commercial_breaks.py`](#5-remove_commercial_breakspy-break-detector)** | Dual black & silence gap scanner | ❌ No | MKV video files | JSON / formatted slice timestamps |
| **[`convert_main_tracks.py`](#6-convert_main_trackspy-master-audio-converter)** | Standalone 5.1/7.1 downmixing converter | ❌ No | Multi-channel audio / video | Surround AC3, True 2.0 Stereo AAC, Night Mode AAC |
| **[`convert_ad_2pass.py`](#7-convert_ad_2passpy-2-pass-loudness-normalizer)** | Standalone 2-pass EBU R128 normalizer | ❌ No | Standalone audio files | Broadcast normalized 2.0 AAC |
| **[`set_commentary_flags.py`](#8-set_commentary_flagspy-in-place-mkv-tagger)** | In-place Matroska commentary flagger | ❌ No | MKV files / directory | In-place tagged MKV (`flag-commentary=1`) |
| **[`plex_ad_commentary_collections.py`](#9-plex_ad_commentary_collectionspy-plex-sync)** | Plex server collection maintainer | ❌ No | Plex media library | Smart static collections in Plex |
| **[`parse_mpls.py`](#10-parse_mplspy-blu-ray-playlist-parser)** | Blu-ray 45 kHz PTS playlist parser | ❌ No | Retail `.mpls` playlist files | Clip list with exact in/out timestamps |

---

## 1. `sync_ad_track.py` (Master Sync & Encoder)

Master tool for synchronizing external Audio Description tracks to reference media and rendering broadcast-quality deliverables.

### Key Capabilities:
* **Directory Auto-Discovery**: Passing a folder automatically pairs `source/*` with all AD tracks in `tosync/*`.
* **Batch Queue Processing (`--batch`)**: Iterates through all movie folders in a top-level queue directory sequentially.
* **100-Anchor FFT Cross-Correlation**: Samples 100 timeline positions via fast FFT in memory (sub-millisecond $\pm 1\text{ms}$ precision, 100% bit-perfect linear passthrough).
* **Forced DTW Fast-Path (`--force-dtw`)**: Bypasses correlation to directly execute DescribeAlign Dynamic Time Warping with 3-layer smoothing for alternate cuts.
* **Dynamic Sample-Rate PAL Clock Math (`--pal` / `--no-pal`)**: Probes input sample rates (44.1k vs 48k) to calculate exact resampling ratios for PAL 25fps slowdowns:
  $$\text{asetrate} = \text{input\_sr} \times \frac{23.976024}{25.0}$$
* **Strict ITU-R BS.775 2.0 Downmixing**: Enforces standard downmix matrix (`pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5`), `-ac 2`, and `-ar 48000`.
* **Local Machine Config Support**: Loads settings and paths from `config.local.json` or `config.json` (falls back to `config.default.json`).

### Common Commands:
```powershell
# 1. Directory Auto-Discovery (AD sync + Main 5.1/2.0 downmix conversion):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --convert-main

# 2. Batch process all movie folders in a queue:
python scripts/sync_ad_track.py "F:\uploads\queue" --batch --convert-main

# 3. Explicit files with stream selection and forced DTW:
python scripts/sync_ad_track.py -r "F:\media\Movie2.mkv" --ref-stream 0:a:1 -a "F:\media\Movie2_AD.mp3" --force-dtw --no-pal

# 4. Fast 3-minute video sync test preview:
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --preview 180 -m "F:\uploads\queue\Movie1\source\Movie1.mkv"
```

---

## 2. `batch_orchestrator.py` (TV Season Orchestrator)

Automated decision engine for full TV seasons where different episodes require different alignment strategies.

### Decision Heuristics:
1. **Direct Mux Strategy**: Selected when multi-anchor offsets are identical ($<30\text{ms}$ variance) and no commercial gaps exist.
2. **Frame-Perfect Strategy**: Selected when black screen counts match offset shift counts. Builds a surgical FFmpeg filter graph to excise commercial breaks without keyframe snapping.
3. **DTW Dynamic Time Warping Strategy**: Selected when audio drifts unpredictably across the episode without black screen markers. Triggers DescribeAlign + `dtw_smooth_stitch.py`.

### Common Commands:
```powershell
# Run automated strategy analysis and processing across TV season:
python scripts/batch_orchestrator.py

# Perform dry-run evaluation without rendering:
python scripts/batch_orchestrator.py --dry-run
```

---

## 3. `dtw_smooth_stitch.py` (DTW Transition Smoother)

Post-processor for DescribeAlign timestamp maps. Re-stitches AD audio using a transparent **3-layer transition smoothing pipeline**:

1. **Layer 1 — VAD-Aware Cut Nudging**: Analyzes center/narrator RMS energy around segment boundaries. If a cut point lands mid-speech, the cut is automatically nudged forward or backward to the nearest natural silence gap (±500ms search window, 200ms hangover).
2. **Layer 2 — Zero-Crossing Snapping**: Micro-adjusts boundaries (±2ms) to land on exact waveform zero crossings, eliminating DC offset pop artifacts.
3. **Layer 3 — Equal-Power Crossfade & Sinc Resampling**: Blends segment junctions using 50ms constant-power cosine crossfades ($\cos^2 + \sin^2 = 1$). Applies bandlimited sinc resampling for genuine rate changes and bit-perfect native PCM passthrough for 1.0x segments.

### Common Commands:
```powershell
# Smooth-stitch an alignment plot output by DescribeAlign:
python scripts/dtw_smooth_stitch.py --plot "scratch/alignment_plots/Movie_map.txt" --ad "tosync/Movie_AD.mp3" --out "output/Movie_Smooth_AD.wav"
```

---

## 4. `batch_frameperfect.py` (Commercial Break Cutter)

Specialized batch processor for web or broadcast TV rips that contain black-screen commercial breaks. Unlike standard container-level splitting (`mkvmerge`), this tool constructs frame-exact FFmpeg `trim`/`atrim`/`concat` filter graphs to remove commercial black frames seamlessly.

### Common Commands:
```powershell
# Process commercial removal across episodes:
python scripts/batch_frameperfect.py
```

---

## 5. `remove_commercial_breaks.py` (Break Detector)

Diagnostic utility that scans video containers for simultaneous black frames (`blackdetect=d=2.0:pix_th=0.1`) and audio silence (`silencedetect=noise=-40dB:d=2.0`). Outputs exact start/end timestamps and duration of commercial gaps.

### Common Commands:
```powershell
# Scan an MKV for commercial break intersections:
python scripts/remove_commercial_breaks.py "F:\media\Episode01.mkv"
```

---

## 6. `convert_main_tracks.py` (Master Audio Converter)

Standalone audio processor for extracting and downmixing 5.1 and 7.1 surround sound master tracks into multiple standardized formats without performing alignment.

### Outputs Generated:
* `(Surround).ac3` — 5.1 AC3 Surround stream (640 kbps).
* `(Stereo).m4a` — True 2.0 Stereo downmix using ITU-R BS.775 matrix + peak limiter (`alimiter`).
* `(Night Mode).m4a` — True 2.0 Stereo downmix using ITU-R BS.775 matrix + broadcast loudness normalization (`loudnorm=I=-16:TP=-1.5:LRA=11`).

### Common Commands:
```powershell
# Convert all multi-channel audio in a directory:
python scripts/convert_main_tracks.py "F:\media\MovieDirectory"
```

---

## 7. `convert_ad_2pass.py` (2-Pass Loudness Normalizer)

Standalone utility for applying broadcast-standard 2-pass EBU R128 loudness normalization directly to standalone audio tracks.

### Common Commands:
```powershell
# Apply 2-pass EBU R128 normalization:
python scripts/convert_ad_2pass.py "F:\uploads\Movie_AD.ac3"
```

---

## 8. `set_commentary_flags.py` (In-Place MKV Tagger)

Ultra-fast in-place Matroska track property manager. Inspects MKV headers via `mkvmerge -J` JSON analysis and uses `mkvpropedit` to set `flag-commentary=1` for tracks matching keyword criteria (`commentary`, `director`, `producer`, `filmmaker`, `writer`). Operates in ~10ms without remuxing.

### Common Commands:
```powershell
# Scan and flag commentary tracks across a directory:
python scripts/set_commentary_flags.py "X:\video\TV\Show_Title"
```

---

## 9. `plex_ad_commentary_collections.py` (Plex Sync)

Plex Media Server integration script that scans media libraries and maintains static collections for items containing Audio Description (`visualImpaired=true`) or Commentary tracks.

### Common Commands:
```powershell
# Dry-run inspection:
python scripts/plex_ad_commentary_collections.py --dry-run

# Update collections for Movies:
python scripts/plex_ad_commentary_collections.py --library "Movies"

# Update collections for TV Shows:
python scripts/plex_ad_commentary_collections.py --library "TV Shows"
```

---

## 10. `parse_mpls.py` (Blu-ray Playlist Parser)

Utility for extracting exact 45 kHz PTS timestamps and PlayItem sequence durations directly from Blu-ray `.mpls` playlist files. Useful for analyzing seamless branching discs, multi-angle releases, and cut differences.

### Common Commands:
```powershell
# Inspect a Blu-ray playlist:
python scripts/parse_mpls.py "D:\BDMV\PLAYLIST\00800.mpls"
```
