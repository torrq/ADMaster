# Audio Description Processing & BatchEncoder Specs

## 1. BatchEncoder Configuration

### Tool & Format Setup
* **FFmpeg binary**: Configured in `tools/ffmpeg_x64.xml` → `path="...ffmpeg.exe"`.
* **Format XMLs** (`formats/`): Define command templates, extensions, and presets. Key format: `FFMPEG_BLURAY_AAC` for 5.1/7.1→stereo AAC downmix with 1-pass `loudnorm`.
* **Progress parsing**: `progress/GetProgress_FFmpegEnc.lua` (FFmpeg), `progress/GetProgress_eac3to.lua` (eac3to).
* **Output naming**: `Outputs.xml` patterns (e.g. `$InputPath$\$Name$.$Ext$`). Format-level `extension=` controls `$Ext$`.

### Critical Quirk
`BatchEncoder.exe` caches formats in RAM and overwrites `formats/*.xml` on exit. **Kill the process before editing format XMLs**, or drag-drop updated XMLs into the GUI.

---

## 2. Audio Alignment Pipeline

For encoding specs, FFmpeg rules, and detailed workflow instructions, see the **audio-description-frame-perfect-sync** skill (`SKILL.md`).

> [!CAUTION]
> **FFmpeg Raw Bitstream Stall Bug:** Raw audio files (`.dts`, `.ac3`, `.eac3`, `.thd`, `.dtshd`) have NO container seek index. `-ss` before `-i` hangs indefinitely. `-ss` after `-i` forces linear decode from byte 0 every call — making repeated seeking (like 10-anchor correlation) take 10+ minutes on a 2-hour file. **NEVER repeatedly seek into raw bitstreams.** The fix in `sync_ad_track.py` is `predecode_to_mono_8k()`: decode the entire file to a tiny 8kHz mono WAV once, then do all math via in-memory numpy slicing. If you ever write new code that needs to seek into raw bitstreams, use this same pattern.

### sync_ad_track.py (Primary — Single-Offset Alignment & Encoding)
* **100-Anchor FFT Cross-Correlation**: Samples 100 anchors evenly across 5%–88% of the timeline via fast FFT in memory (pre-decoding streams once to 8kHz mono WAV in `scratch/`).
* **Candidate Clustering & Runner-Ups**: Groups anchor measurements within a $\pm 30\text{ms}$ sub-frame tolerance (`cluster_tol=0.03`) to compute the consensus winner, and displays up to 2 distinct runner-up offsets with confidence ratings and anchor vote counts.
* **Confidence Reporting**: Displays match confidence as both percentage and normalized cosine similarity (e.g. `88.9% (0.8887)`).
* **Automatic Directory Routing**: Automatically sets `<movie_dir>/output/` for final encodes and `<movie_dir>/scratch/` for temporary analysis files.
* **Atomic Pre-Decoding**: Uses temporary files (`_tmp.wav`) and atomic `os.replace` to prevent corrupted/half-written cache files from cancelled tasks.
* **Standard Audio Variants**:
  * **AD Tracks**: Generates `-AD (Stereo).m4a` (`alimiter`) and `-AD (Night Mode).m4a` (`loudnorm` -16 LUFS).
  * **Main Audio (`--convert-main`)**: Probes channels and outputs `(Surround).ac3` (5.1 384k), `(Stereo).m4a` (downmix + `alimiter`), and `(Night Mode).m4a` (downmix + `loudnorm`).
* **Auto-detects PAL/NTSC**: Detects PAL 25fps speedup (`1.04271x`) and NTSC pulldown (`0.99956x`).
* **Preview Mode (`--preview [sec]`)**: Fast test-listening with optional MKV video muxing (`-m`).

### describealign (Fallback — Dynamic Time Warping)
* **When**: Cross-correlation fails due to different edits (commercial cuts, scene additions, network vs bluray).
* **How**: DTW + linear programming (`scipy.optimize.linprog`) builds piecewise alignment map.
* **Command**: `python describealign.py --yes --stretch_audio <master> <ad_track>`
* **Outputs**: `videos_with_ad/ad_<filename>` (aligned MKV), `alignment_plots/<filename>.txt` (timestamp map).
* **Track order gotcha**: Outputs AD as `a:0`, Main as `a:1` — reverse of our standard. Always remux to swap.
* **Automation gotcha**: `--yes` required (skips interactive prompt that stalls batch scripts).
* **Cleanup**: Run from `scratch/` directory to contain generated `videos_with_ad/` and `alignment_plots/` junk.

### dtw_smooth_stitch.py (Post-DescribeAlign Transition Smoother)
Replaces the old hard-cut `build_dtw_stitch.py`. Parses describealign alignment plots and re-stitches AD audio through a **3-layer transition smoothing pipeline**:

1. **Layer 1 — VAD-Aware Cut Nudging**: At each segment boundary, checks RMS energy in 50ms analysis windows. If speech is active at a cut point, the outgoing boundary is extended forward (or incoming pulled backward) to the nearest silence gap. Configurable: `±500ms` search, `200ms` consecutive silence hangover, `0.02` RMS threshold.
2. **Layer 2 — Zero-Crossing Snapping**: After VAD nudging, each cut point is snapped to the nearest waveform zero crossing within `±2ms` to eliminate DC-offset pops and sub-sample clicks.
3. **Layer 3 — Equal-Power Crossfade**: At each segment junction, a cosine-shaped equal-power crossfade (`50ms` default) blends outgoing and incoming audio via overlap-add mixing. Satisfies the constant-power identity $\cos^2 + \sin^2 = 1$.

**Standalone usage**: `python dtw_smooth_stitch.py <alignment_plot.txt> <ad_audio> <output.m4a>`
**Integrated**: `batch_orchestrator.py` automatically imports and calls `run_smooth_stitch()` after describealign produces an alignment plot. Falls back to describealign's raw output if smooth stitching fails.

### batch_orchestrator.py Strategy Decision Tree
Automatically selects per episode:
1. **Direct Mux** — single consistent offset, no black screens.
2. **Frame-Perfect** — black screens detected and offset count matches gap count.
3. **DTW (describealign + smooth stitch)** — no black screens but multiple drifting offsets. Runs describealign for alignment, then re-stitches with `dtw_smooth_stitch.py` for clean transitions.

### Other Scripts
* `convert_ad_2pass.py`: Standalone 2-pass EBU R128 normalization (no alignment).
* `parse_mpls.py`: Extracts 45kHz PTS timestamps from Blu-ray `.mpls` playlists.
* `process_extended_ad_option_a.py`: Seamless branching AD reconstruction with 50ms equal-power crossfades.
* `build_smart_ad_pipeline.py`: VAD center-channel analysis to prevent narrator speech truncation.


See `scripts/README.md` for the full scripts index.

---

## 3. MKV Chapters

```powershell
# Extract XML chapters:
& "C:\Program Files\MKVToolNix\mkvextract.exe" "movie.mkv" chapters "chapters.xml"

# Extract simple text chapters:
& "C:\Program Files\MKVToolNix\mkvextract.exe" "movie.mkv" chapters --simple "chapters.txt"

# Inject chapters in-place (no remux):
& "C:\Program Files\MKVToolNix\mkvpropedit.exe" "movie.mkv" --chapters "chapters.xml"
```

**MKVcleaver quirk**: Set `chapters_s=1` (txt) or `chapters_s=2` (xml) in `custom.ini` to enable the chapters checkbox.

---

## 4. Standard Paths
* **Project scripts**: `scripts\`
* **DescribeAlign**: `describealign.py`
* **FFmpeg binaries**: `AudioDescriptionStuff\bin\` (auto-resolved by Python scripts)
