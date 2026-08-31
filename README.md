<img width="1672" height="500" alt="ADMaster" src="https://github.com/user-attachments/assets/f2a5f098-cf73-432d-a949-f856c72426cb" />

# ADMaster -- Universal Audio Description & Secondary Audio Sync Toolkit

**ADMaster** is a high-performance, automated Python toolkit for synchronizing, downmixing, loudness-normalizing, and packaging Audio Description (AD) and secondary commentary tracks for movies and television series.

It provides sub-millisecond acoustic cross-correlation, dynamic Dynamic Time Warping (DTW) piecewise alignment with Multilayer transition smoothing, frame-accurate black-screen commercial break excision, broadcast-standard ITU-R BS.775 2.0 stereo downmixing, in-place Matroska flag management, and Plex Media Server collection integration.

---

## Production Scripts Index

All scripts are located in [`scripts/`](scripts/) and can be run standalone or chained in automated batch workflows:

| Script | Purpose | When to Use |
| :--- | :--- | :--- |
| **[`sync_ad_track.py`](scripts/README.md#1-sync_ad_trackpy-master-sync--encoder)** | **Master Movie Sync & Encoder** | Aligning AD tracks to movies, queue batch processing, dynamic PAL conversion, 5.1/2.0 deliverable rendering. |
| **[`batch_orchestrator.py`](scripts/README.md#2-batch_orchestratorpy-tv-season-orchestrator)** | **TV Season Router** | Processing entire TV seasons with automatic strategy selection (Direct Mux, Frame-Perfect Cut, DTW). |
| **[`dtw_smooth_stitch.py`](scripts/README.md#3-dtw_smooth_stitchpy-dtw-transition-smoother)** | **Multilayer DTW Smoother** | Re-stitching DescribeAlign alignment maps with VAD cut nudging, zero-crossing snapping, and 50ms crossfades. |
| **[`batch_frameperfect.py`](scripts/README.md#4-batch_frameperfectpy-commercial-break-cutter)** | **Commercial Break Cutter** | Surgically cutting black screen commercial gaps from video streams without keyframe snapping. |
| **[`remove_commercial_breaks.py`](scripts/README.md#5-remove_commercial_breakspy-break-detector)** | **Break Scanner** | Detecting simultaneous video black and audio silence intervals to output exact cut timestamps. |
| **[`convert_main_tracks.py`](scripts/README.md#6-convert_main_trackspy-master-audio-converter)** | **Master Audio Downmixer** | Extracting and downmixing 5.1/7.1 masters into true 2.0 Stereo (`alimiter`) and Night Mode (`loudnorm`). |
| **[`convert_ad_2pass.py`](scripts/README.md#7-convert_ad_2passpy-2-pass-loudness-normalizer)** | **2-Pass Loudness Normalizer** | Applying broadcast EBU R128 loudness normalization directly to standalone audio tracks. |
| **[`set_commentary_flags.py`](scripts/README.md#8-set_commentary_flagspy-in-place-mkv-tagger)** | **In-Place MKV Tagger** | Tagging commentary and director audio tracks with `flag-commentary=1` in ~10ms via `mkvpropedit`. |
| **[`plex_ad_commentary_collections.py`](scripts/README.md#9-plex_ad_commentary_collectionspy-plex-sync)** | **Plex Server Integration** | Automatically creating and maintaining smart Plex collections for AD and commentary media. |
| **[`parse_mpls.py`](scripts/README.md#10-parse_mplspy-blu-ray-playlist-parser)** | **Blu-ray Playlist Parser** | Extracting 45 kHz PTS PlayItem timestamps directly from retail Blu-ray `.mpls` playlist files. |

*(For detailed CLI flags and parameter options for each script, refer to [`scripts/README.md`](scripts/README.md))*

---

## Quick Start & Common Workflows

### 1. Movie Directory Auto-Discovery (Acoustic Correlation + Deliverables)
Drop the reference video or audio file into `source/` and all candidate AD tracks into `tosync/`:
```powershell
# Automatically discovers files, computes 100-anchor offset, renders dual AD and triple Main audio:
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --convert-main
```

### 2. Batch Processing a Full Queue Directory
Process an entire directory containing dozens of movie folders sequentially:
```powershell
python scripts/sync_ad_track.py "F:\uploads\queue" --batch --convert-main
```

### 3. Forced DTW Alignment for Alternate Cuts
When syncing an AD track from a Theatrical Cut onto an Extended Cut Blu-ray:
```powershell
python scripts/sync_ad_track.py "F:\uploads\queue\Movie2" --force-dtw --no-pal
```

### 4. Fast 3-Minute Video Test Preview
Multiplexes a 3-minute video sample to verify lip-sync before processing a full feature:
```powershell
python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --preview 180
```

### 5. Tagging Commentary Tracks Across a TV Show or Movie Library
Scans MKV headers with `mkvmerge -J` and sets `flag-commentary=1` in-place without remuxing:
```powershell
python scripts/set_commentary_flags.py "X:\video\TV\Show_Title"
```

---

## Alignment Strategies: Anchor Correlation vs. Native Warp (ADSync) vs. DescribeAlign DTW

| Feature / Metric | 100-Anchor Multiband Cross-Correlation (Default) | Native ADSync Warp (Default Non-Linear) | DescribeAlign DTW (Legacy Non-Linear) |
| :--- | :--- | :--- | :--- |
| **Best Used For** | Identical cuts (same edit, different audio delay or drift). | Alternate cuts (Theatrical vs Extended, TV trims, reel splices). | Secondary DTW comparison & legacy plot processing. |
| **Narration Suppression** | **3-Band Filtering:** Low/High prioritized; speech band ($300\text{Hz}-1.2\text{kHz}$) downweighted to 0.2. | **Multiband Lattice Scoring:** Downweights narration speech to lock onto background soundtrack. | Single-band linear programming optimization. |
| **Audio Integrity** | **100% Bit-Perfect Linear Passthrough:** Preserves decoded PCM samples bit-for-bit. Zero pitch/tempo warping. | **Continuous PCHIP Warp + WSOLA:** Sample-by-sample monotone mapping with pitch preservation. | **Piecewise Reconstruction:** Slices into discrete segments and crossfades transitions. |
| **PAL Speed Probing** | **Acoustic Landmark Probing:** Automatically detects 25/24 speed differences in ~2 seconds. | **Acoustic Landmark Probing:** Automatically detects 25/24 speed differences in ~2 seconds. | Filename heuristics (`[UK]`, `[PAL]`) or linear drift slope fitting. |
| **Execution Time** | **~2-4 seconds** via in-memory FFT arrays. | **~15-30 seconds** via Viterbi dynamic programming. | **1-3 minutes** per track (feature extraction + LP optimization). |

---

## Audio Encoding & Downmixing Standards

All stereo deliverables generated by this toolkit adhere strictly to international broadcast specifications:

1. **ITU-R BS.775 True 2.0 Downmix Matrix**:
   Surround audio tracks ($\ge 6$ channels) are folded down using the ITU standard formula with centered dialogue:
   $$\text{Left} = L + 0.7071 \times C + 0.7071 \times L_s$$
   $$\text{Right} = R + 0.7071 \times C + 0.7071 \times R_s$$
2. **True 2.0 Stereo Deliverable**:
   Applies the BS.775 matrix followed by `alimiter` clipping protection (`level_in=1:level_out=0.98:limit=0.98`) to preserve the natural dynamic range of theatrical mixes.
3. **True 2.0 Night Mode Deliverable**:
   Applies the BS.775 matrix followed by broadcast-standard EBU R128 loudness normalization (`loudnorm=I=-16:TP=-1.5:LRA=11`) with centered vocal clarity for nighttime or headphone listening.
4. **Resampling Fidelity**:
   Enforces bandlimited sinc/Fourier resampling (`scipy.signal.resample` / `soxr`) for genuine rate conversions (e.g. PAL 4.27% slowdowns) and bit-perfect passthrough for native 1.0x streams.

---

## Configuration & Local Overrides

This toolkit is fully portable and requires zero hardcoded paths. 

To configure custom binary paths, encoding threads, or loudness targets:
1. Copy [`config.default.json`](config.default.json) to `config.local.json` or `config.json` (which are ignored by `.gitignore`).
2. Customize your settings (comments using `//` or `/* */` are supported):

```jsonc
{
  // Path to custom FFmpeg binary (leave empty to auto-discover in ./bin/ or PATH)
  "ffmpeg_path": "",

  // Path to custom FFprobe binary (leave empty to auto-discover alongside FFmpeg)
  "ffprobe_path": "",

  // Path to MKVToolNix binaries (auto-discovers in ./bin/, Program Files, or PATH)
  "mkvmerge_path": "",
  "mkvpropedit_path": "",
  "mkvextract_path": "",

  // Path to ADSync repository or package (auto-discovers in sibling folders)
  "adsync_path": "",

  // Path to describealign.py script for legacy DTW alignment
  "describealign_path": "",

  // Default non-linear warp engine: "adsync" (default) or "describealign"
  "default_warp_engine": "adsync",

  // CPU threads for FFmpeg encoding
  "default_threads": 2,

  // Default target EBU R128 loudness (LUFS) and True Peak ceiling (dB)
  "default_target_i": -16.0,
  "default_target_tp": -1.5,

  // Correlation anchors sampled across timeline (Default: 100)
  "default_anchors": 100,

  // Automatic acoustic feature detection
  "default_multiband": true,
  "default_speed_detect": true,

  // Overwrite existing output files (default: false)
  "default_overwrite": false,

  // Enable verbose debug logging
  "debug_mode": false
}
```

---

## Setup & Requirements

1. **Python**: Python 3.9+ with `numpy` and `scipy` installed:
   ```powershell
   pip install numpy scipy
   ```
2. **Binaries**: Place `ffmpeg.exe` and `ffprobe.exe` inside a `bin/` folder in the project root, or ensure they are available in your system `PATH`.
3. **MKV Tools**: For commentary flagging and metadata inspection, install [MKVToolNix](https://mkvtoolnix.download/).

---

## Upstream Attribution & Alignment Engines

Non-linear audio alignment in **ADMaster** is powered by two complementary open-source engines:

### 1. Primary Engine: [ADSync](https://github.com/JohnnyTheCoder1/ADSync) (Default)
By **JohnnyTheCoder1**. **ADSync** provides state-of-the-art acoustic landmark fingerprinting, 3-band narration speech suppression, and dynamic programming on candidate lattices:
* **Continuous Monotone PCHIP Cubic Spline Warping**: Maps timestamps smoothly without cutting audio into discrete blocks, eliminating splice glitch transients and clipped syllables.
* **WSOLA Time-Stretching**: Preserves audio pitch and timbre across complex reel speed differences.
* **Automatic Acoustic PAL / Speed Probing**: Identifies 25fps vs 23.976fps transfers in ~2 seconds using constellation hashes.
* **Execution Speed**: High-performance alignment and rendering in **~15–30 seconds**.

### 2. Secondary Engine: [DescribeAlign](https://github.com/julbean/describealign) (Legacy Option)
By **julbean**. **DescribeAlign** formulates acoustic Dynamic Time Warping (DTW) as a linear programming optimization problem (`scipy.optimize.linprog`). 

**ADMaster** feeds DescribeAlign warping vectors into our custom Multilayer post-processing pipeline ([`dtw_smooth_stitch.py`](scripts/README.md#4-dtw_smooth_stitchpy-dtw-transition-smoother)):
1. **Plateau Consolidation**: Groups jittery DTW micro-segments sharing the same baseline offset ($\le 120\text{ms}$) into unified structural blocks, eliminating over 85% of phantom cuts.
2. **Wobble Filtering (DTW Hallucination Correction)**: Rejects erratic offset drifts during musical overtures and silence gaps.
3. **Master Audio Gap Filling**: Detects deleted scenes (timeline voids) in the AD track and smoothly blends volume-matched Blu-ray master audio across the void.
4. **VAD Cut Nudging**: Evaluates narrator RMS energy ($\pm 500\text{ms}$ search window, 200ms hangover) to snap cut points to natural speech pauses.
5. **Zero-Crossing Phase Snapping**: Micro-adjusts cut boundaries ($\pm 2\text{ms}$) to waveform zero crossings, eliminating DC pops.
6. **Equal-Power Cosine Crossfades**: Blends segment junctions via 50ms constant-power crossfades ($\cos^2\theta + \sin^2\theta = 1$).

---

## BatchEncoder Integration

For GUI-based bulk processing, this project pairs excellently with **BatchEncoder**. We maintain a custom `FFMPEG_BLURAY_AAC.xml` format preset that replicates these exact downmix and loudness matrices (via a 1-pass approximation) directly inside the BatchEncoder GUI.

---

## License

This project and all included scripts are open-source and licensed under the [MIT License](LICENSE).

*(For detailed architectural rules, LLM-agent instructions, and pipeline limitations, refer to [`AGENTS.md`](AGENTS.md))*
