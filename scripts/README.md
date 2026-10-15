# ADMaster Pipeline Scripts

This directory contains the production-ready tools for acoustic audio description synchronization, video stream commercial break excision, multi-channel surround downmixing, MKV metadata manipulation, and media server automation.

---

## Tool Summary & Capability Matrix

| Script | Primary Responsibility | Modifies Video? | Input Types | Output Deliverables |
| :--- | :--- | :---: | :--- | :--- |
| **[`sync_ad_track.py`](#1-sync_ad_trackpy-master-sync--encoder)** | Universal acoustic alignment & audio encoding | No | MKV, MP4, DTS, AC3, FLAC, MP3, WAV | Aligned `-AD (Stereo).m4a`, `-AD (Night Mode).m4a`, `(Surround).ac3`, `(Stereo).m4a` |
| **[`align_features.py`](#2-align_featurespy-advanced-alignment-features)** | Multiband correlation, PAL prober, & ADSync warp engine | No | Audio numpy arrays / files | Filtered band correlations, speed ratios, warp time maps |
| **[`batch_orchestrator.py`](#3-batch_orchestratorpy-tv-season-orchestrator)** | Automated heuristic TV series strategy router | Conditional | Directory of MKV episodes + AD tracks | Synced MKVs via Direct Mux, Frame-Perfect Cut, or DTW |
| **[`dtw_smooth_stitch.py`](#4-dtw_smooth_stitchpy-dtw-transition-smoother)** | Multilayer DTW transition smoother | No | Alignment map + Source AD audio | Stitched, smooth AD audio with VAD + Crossfades |
| **[`batch_frameperfect.py`](#5-batch_frameperfectpy-commercial-break-cutter)** | Surgical video/audio commercial gap excision | Yes | MKVs with black commercial gaps | Clean MKV without black gaps, frame-accurate |
| **[`remove_commercial_breaks.py`](#6-remove_commercial_breakspy-break-detector)** | Dual black & silence gap scanner | No | MKV video files | JSON / formatted slice timestamps |
| **[`convert_main_tracks.py`](#7-convert_main_trackspy-master-audio-converter)** | Standalone 5.1/7.1 downmixing converter | No | Multi-channel audio / video | Surround AC3, True 2.0 Stereo AAC, Night Mode AAC |
| **[`convert_ad_2pass.py`](#8-convert_ad_2passpy-2-pass-loudness-normalizer)** | Standalone 2-pass EBU R128 normalizer | No | Standalone audio files | Broadcast normalized 2.0 AAC |
| **[`set_commentary_flags.py`](#9-set_commentary_flagspy-in-place-mkv-tagger)** | In-place Matroska commentary flagger | No | MKV files / directory | In-place tagged MKV (`flag-commentary=1`) |
| **[`plex_ad_commentary_collections.py`](#10-plex_ad_commentary_collectionspy-plex-sync)** | Plex server collection maintainer | No | Plex media library | Smart static collections in Plex |
| **[`parse_mpls.py`](#11-parse_mplspy-blu-ray-playlist-parser)** | Blu-ray 45 kHz PTS playlist parser | No | Retail `.mpls` playlist files | Clip list with exact in/out timestamps |
| **[`chunk_ad_video.py`](#11-chunk_ad_videopy-gemini-ad-video-chunker)** | Automated 1 FPS video chunking for Gemini AD pipeline | Yes | MKV, MP4, video files | Balanced 1 FPS MP4 chunks + `{movie}_ad_manifest.json` |

---

## 1. `sync_ad_track.py` (Master Sync & Encoder)

Master tool for synchronizing external Audio Description tracks to reference media and rendering broadcast-quality deliverables.

### Key Capabilities:
* **Directory Auto-Discovery**: Passing a folder automatically pairs `source/*` with all AD tracks in `tosync/*`.
* **Batch Queue Processing (`--batch`)**: Iterates through all movie folders in a top-level queue directory sequentially.
* **100-Anchor Multiband Cross-Correlation**: Samples 100 timeline positions using 3-band narration suppression (downweights $300\text{Hz}-1.2\text{kHz}$ vocal frequencies) for peak sharpness against soundtrack effects.
* **Automatic Acoustic PAL / Speed Prober**: Employs landmark constellation hashing to detect PAL 25fps vs 23.976fps transfers in ~2 seconds without needing `[UK]` filename tags.
* **Dual Warp Engines (`--warp-engine adsync|describealign`)**:
  * `adsync` (Default): High-speed Viterbi candidate lattice DP + Monotone PCHIP warp + WSOLA rendering (~15-30s).
  * `describealign` (Legacy): Linear programming DTW + Plateau Consolidation Multilayer stitcher.
* **Master Audio Gap Filling**: Automatically detects missing AD scenes/edits and seamlessly fills timeline voids with volume-matched Blu-ray soundtrack audio.
* **Rapid Test Iterations (`--wav` & `--no-night`)**: Generates 16-bit 48kHz uncompressed PCM WAV deliverables directly without waiting for AAC compression, and disables the slow `loudnorm` nighttime filter.
* **Scratch Caching & Clean Runs (`--clean-scratch` / `--wipe-scratch`)**: Automatically caches acoustic correlation results in `scratch/sync_params.json` and reuses warped audio in `scratch/adsync/` across reruns. Use `--clean-scratch` to wipe scratch for a 100% clean run.
* **Strict ITU-R BS.775 2.0 Downmixing**: Enforces standard downmix matrix (`pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5`), `-ac 2`, and `-ar 48000`.
* **Overwrite Protection (`--overwrite` / `-f` / `--force-overwrite`)**: Prevents re-rendering deliverables that already exist in `output/` by default, saving CPU and time during queue re-runs.
* **Detailed Final Sync Summary**: Outputs a comprehensive report detailing global offset, sample-rate stretch, and structural cut timestamps for easy manual verification.

### Common Commands:
```powershell
# 1. Directory Auto-Discovery (AD sync + Main 5.1/2.0 downmix conversion):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --convert-main

# 2. Batch process all movie folders in a queue:
python scripts/sync_ad_track.py "F:\uploads\queue" --batch --convert-main

# 3. Fast sync testing (WAV output, no night mode, reuse scratch):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --wav --no-night --overwrite --keep-scratch

# 4. Clean run from scratch (wipe scratch folder and recompute):
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --clean-scratch --convert-main

# 5. Explicit files with stream selection and forced DTW:
python scripts/sync_ad_track.py -r "F:\media\Movie2.mkv" --ref-stream 0:a:1 -a "F:\media\Movie2_AD.mp3" --force-dtw --no-pal

# 6. Fast 3-minute video sync test preview:
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

Post-processor for DescribeAlign timestamp maps. Re-stitches AD audio using a transparent **Multilayer transition smoothing pipeline**:

1. **Layer 0 -- Plateau Consolidation, Wobble Filtering & Gap Filling**: Automatically consolidates jittery DTW micro-segments sharing the same baseline offset ($\le 120\text{ms}$) into continuous structural blocks, eliminating over 85% of phantom cuts. Differentiates between DTW tracking hallucinations (> 0.2% drift) and genuine continuous clock drifts (like 24.000 vs 23.976 fps), rejecting hallucinations to force strict 1.0x native playback across structural splices. Detects deleted scenes and truncates DescribeAlign's fallback audio repetition, inserting clean silence gaps crossfaded with the original Master Audio soundtrack to maintain ambient continuity.
2. **Layer 1 -- VAD-Aware Cut Nudging**: Analyzes center/narrator RMS energy around segment boundaries. If a cut point lands mid-speech, the cut is automatically nudged forward or backward to the nearest natural silence gap (+/-500ms search window, 200ms hangover).
3. **Layer 2 -- Zero-Crossing Snapping**: Micro-adjusts boundaries (+/-2ms) to land on exact waveform zero crossings, eliminating DC offset pop artifacts.
4. **Layer 3 -- Float64 Resampling & Equal-Power Crossfade**: Automatically applies high-precision `float64` linear interpolation to eliminate continuous timeline drifts (up to 5.0%, supporting native 4.27% PAL matches) without WSOLA artifacts. Blends segment junctions using 50ms constant-power cosine crossfades ($\cos^2\theta + \sin^2\theta = 1$).

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
* `(Surround).ac3` -- 5.1 AC3 Surround stream (640 kbps).
* `(Stereo).m4a` -- True 2.0 Stereo downmix using ITU-R BS.775 matrix + peak limiter (`alimiter`).
* `(Night Mode).m4a` -- True 2.0 Stereo downmix using ITU-R BS.775 matrix + broadcast loudness normalization (`loudnorm=I=-16:TP=-1.5:LRA=11`).

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

---

## 11. `chunk_ad_video.py` (Gemini AD Video Chunker)

Specialized video pre-processor for the Automated Long-Form Video Descriptive Audio (AD) pipeline. Slices source movies into balanced, lightweight sections (default <= 45 minutes each) pre-transcoded directly to Gemini's multimodal ingestion profile (1 FPS, 720p/480p, CRF 28, mono AAC 64k). Automatically scores and selects cut boundaries within dialogue pauses (`silencedetect`), visual scene transitions (`select='gt(scene,0.25)'`), and Matroska chapters to preserve speech continuity. Outputs a comprehensive JSON manifest with sample-accurate timestamps for downstream Gemini inference, TTS synthesis, and timeline re-alignment.

### Key Capabilities:
* **Dialogue Preservation**: Scans audio for silence intervals (-30 dB) to ensure cut boundaries never interrupt spoken dialogue.
* **Scene and Chapter Snapping**: Aligns cut boundaries with camera shot changes and retail disc chapters.
* **Gemini 1 FPS Pre-Processing**: Downsamples video to 1 FPS and mono AAC, slashing file sizes by ~90-95% for rapid Gemini File API upload.
* **Structured Manifest Output**: Emits `{basename}_ad_manifest.json` containing exact start/end timecodes and timeline offsets.
* **Dry-Run Analysis**: Supports `--dry-run` to preview chunk boundaries and manifest without transcoding.

### Common Commands:
```powershell
# Standard 45m max chunking of a movie MKV:
python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv"

# Dry-run analysis (inspect cut points and generate manifest only):
python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --dry-run

# Custom target chunk constraints (e.g. max 35m, min 25m):
python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --max-duration 35 --min-duration 25

# 480p resolution profile with 96k audio bitrate:
python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --resolution 480p --audio-bitrate 96k
```

