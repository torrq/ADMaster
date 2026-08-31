---
name: adsync-alignment
description: High-speed acoustic landmark fingerprinting, Viterbi lattice DP, continuous PCHIP warping, and automatic speed detection via ADSync.
---

# ADSync Audio Warping & Alignment Engine

## 1. Core Architecture & Overview
**ADSync** (by JohnnyTheCoder1) is a high-speed, non-linear audio synchronization and time-warping engine. It aligns secondary audio tracks (Audio Description, commentaries, foreign dubs) to reference video soundtracks in **15 to 30 seconds**.

Unlike legacy piecewise DTW tools (which chop audio into discrete segments and crossfade cut boundaries), ADSync computes a **continuous monotone warping spline** and retimes the audio sample-by-sample using WSOLA (Waveform Similarity Overlap-Add). This eliminates mid-speech cutoffs, clipped syllables, and splice transients.

---

## 2. The 12-Step Pipeline Architecture

1. **Probe (`probe`)**: Uses `ffprobe` to inspect duration, channel layout, and sample rate.
2. **Audio Extraction (`extract_audio`)**: Extracts 16kHz mono analysis WAVs and standard 48kHz stereo HQ audio. *(Uses `-rf64 auto` to prevent >4GB truncations).*
3. **Speed & PAL Probing (`detect_speed`)**: Probes 25fps vs 23.976fps speed ratios (+4.28%) across 2-second landmark constellation hashes without requiring filename hints.
4. **Feature Extraction (`extract_basic_features`)**: Computes spectral flux onsets and RMS energy curves.
5. **Acoustic Fingerprinting (`fingerprint_align`)**: Shazam-style peak-pair constellation hashing for robust global landmark alignment.
6. **Global Offset Estimation (`estimate_global_offset`)**: Resolves dominant delay cluster across timeline windows.
7. **Anchor Generation (`find_anchors`)**: Distributes high-density analysis anchors across the timeline.
8. **Drift Estimation (`estimate_drift`)**: Evaluates linear clock drift (PPM).
9. **Confidence Scoring (`compute_confidence`)**: Computes weighted verification score (median/p95 error in ms vs fingerprints).
10. **Candidate Lattice & Viterbi Warp (`warp_decode` / `warp_fit`)**: Constructs candidate offset nodes, runs Viterbi dynamic programming with slope constraints, and fits a **Monotone PCHIP Cubic Spline**.
11. **Waveform Rebuild (`warp_render`)**: Renders synced audio at 48kHz via pitch-preserved WSOLA retiming and spline interpolation.
12. **Export (`export_wav`)**: Writes standard 48kHz WAV deliverable.

---

## 3. Algorithmic Innovations

### A. 3-Band Narration Speech Suppression
Human voice frequencies concentrate between 250 Hz and 1200 Hz. ADSync filters signals into 3 Butterworth bands:
* **Low (< 250 Hz)**: Weight `1.0` (Bass, impacts, explosions)
* **Mid (250 Hz - 1200 Hz)**: Weight **`0.2`** (Dialogue/Narration band — suppressed to prevent false matches)
* **High (> 1200 Hz)**: Weight `0.8` (Foley, high-frequency transients, soundtrack detail)

### B. Monotone PCHIP Spline Warping
Piecewise Cubic Hermite Interpolating Polynomials (PCHIP) guarantee that the warp mapping function is **strictly monotonic** (time never moves backward). This prevents temporal reversal and ensures smooth C1 velocity continuity across structural transitions.

### C. 2-Second Landmark Speed Probing
Probes standard broadcast/theatrical speed ratios (25.0 / 23.976024 = 1.042709x) against landmark constellation hashes. Automatically detects un-tagged PAL releases and applies sample-rate clock restoration (x1.042780, pitch preserved).

---

## 4. Critical Bug Fix: The 4GB WAV File Limit (>3h06m)

### Problem
* Standard 32-bit RIFF WAV files store file length using a 32-bit unsigned integer (2^32 - 1 = 4,294,967,295 bytes / 4.0 GB).
* At 48,000 Hz 32-bit float stereo (384,000 bytes/second), a file hits 4.0 GB at **`11,184.81 seconds` (03:06:24.81)**.
* On movies longer than 3h06m (e.g. *Kill Bill: The Whole Bloody Affair* @ 4h13m), standard WAV extraction caused the header length to cap at 3h06m. Audio readers (`librosa.load`, `soundfile.read`) stopped reading at `03:06:24`, leaving dead/repeating audio for the remainder of the film.

### Patch Applied
In `adsync.media.extract:extract_audio`, added `"-rf64", "auto"` to the FFmpeg extraction arguments:
```python
args = [
    "-i", info.path,
    "-map", f"0:{stream_index}",
    *filter_args,
    "-ar", str(sr),
    *channels_args,
    "-c:a", codecs[sample_fmt],
    "-rf64", "auto",  # Enables 64-bit RF64 headers for files > 4GB
    "-f", "wav",
    str(output_path),
]
```
This allows full 64-bit RF64 decoding of multi-hour features (e.g., 728,691,230 frames / 4h13m) without truncation.

### B. Windows `cp1252` Subprocess `UnicodeDecodeError` Fix
* **Problem**: In Python on Windows, `subprocess.run(..., text=True)` defaults to `locale.getencoding()` (`cp1252`). When `ffprobe` probes media files containing UTF-8 characters (chapter names, metadata tags, unicode track titles), the background reader thread (`_readerthread`) crashes with:
  `UnicodeDecodeError: 'charmap' codec can't decode byte 0x8d: character maps to <undefined>`.
  This causes `subprocess.run` to return `stdout=None`, crashing `json.loads(result.stdout)` with `TypeError: the JSON object must be str, bytes or bytearray, not NoneType`.
* **Patch Applied**: In `adsync.utils.subprocesses:run_ffmpeg` and `run_ffprobe`, explicitly pass `encoding="utf-8", errors="replace"` to all `subprocess.run` calls.

---

## 5. Integration in `ADMaster`

In `ADMaster`, `sync_ad_track.py` invokes ADSync as the default warp engine:
```powershell
# Default non-linear warp:
python scripts/sync_ad_track.py "F:\uploads\queue\<MovieFolder>" --overwrite

# Force non-linear warp explicitly:
python scripts/sync_ad_track.py "F:\uploads\queue\<MovieFolder>" --force-dtw --overwrite
```

### Python Pipeline Execution Snippet
```python
import adsync._pipeline as ad_pipe
import adsync.config as ad_conf
from pathlib import Path

cfg = ad_conf.SyncConfig(
    mode="warp",
    speed_detect=True,
    fingerprint=True,
    crossfade_ms=50,
    output_sr=48000,
    analysis_sr=16000,
    multiband=True,
)

report = ad_pipe.run_pipeline(
    video_path=Path("reference.mkv"),
    ad_path=Path("ad_track.mp3"),
    output_path=None,
    config=cfg,
    debug_dir=Path("scratch/adsync/track_name"),
    mux=False,
)
# Rendered audio deliverable: scratch/adsync/track_name/synced_ad.wav
```
