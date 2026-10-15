# Audio Description Processing & BatchEncoder Specs

## 1. BatchEncoder Configuration

### Tool & Format Setup
* **FFmpeg binary**: Configured in `tools/ffmpeg_x64.xml` -> `path="...ffmpeg.exe"`.
* **Format XMLs** (`formats/`): Define command templates, extensions, and presets. Key format: `FFMPEG_BLURAY_AAC` for 5.1/7.1->stereo AAC downmix with 1-pass `loudnorm`.
* **Progress parsing**: `progress/GetProgress_FFmpegEnc.lua` (FFmpeg), `progress/GetProgress_eac3to.lua` (eac3to).
* **Output naming**: `Outputs.xml` patterns (e.g. `$InputPath$\$Name$.$Ext$`). Format-level `extension=` controls `$Ext$`.

### Critical Quirk
`BatchEncoder.exe` caches formats in RAM and overwrites `formats/*.xml` on exit. **Kill the process before editing format XMLs**, or drag-drop updated XMLs into the GUI.

---

## 2. Audio Alignment Pipeline

For encoding specs, FFmpeg rules, and detailed workflow instructions, see the **audio-description-frame-perfect-sync** skill (`SKILL.md`).

> [!CAUTION]
> **FFmpeg Raw Bitstream Stall Bug:** Raw audio files (`.dts`, `.ac3`, `.eac3`, `.thd`, `.dtshd`) have NO container seek index. `-ss` before `-i` hangs indefinitely. `-ss` after `-i` forces linear decode from byte 0 every call -- making repeated seeking (like 10-anchor correlation) take 10+ minutes on a 2-hour file. **NEVER repeatedly seek into raw bitstreams.** The fix in `sync_ad_track.py` is `predecode_to_mono_8k()`: decode the entire file to a tiny 8kHz mono WAV once, then do all math via in-memory numpy slicing. If you ever write new code that needs to seek into raw bitstreams, use this same pattern.

### sync_ad_track.py (Unified Sync & Encoding Tool)
* **Primary Mandate**: Always use `sync_ad_track.py` for queue and movie processing rather than creating custom one-off scripts.
* **Directory Auto-Discovery**: Passing a folder path (e.g. `python scripts/sync_ad_track.py "F:\uploads\queue\<Movie>"`) automatically pairs `source/*` with all AD tracks in `tosync/*`.
* **100-Anchor Multiband Cross-Correlation**: Samples 100 anchors evenly across 5%-88% of the timeline via fast FFT in memory with 3-band narration suppression (downweights speech band to 0.2).
* **Automatic PAL / Speed Prober**: Uses landmark constellation hashing to automatically detect PAL 25fps vs 23.976fps transfers in ~2 seconds without needing `[UK]` filename clues.
* **Dual Warp Engine Architecture (`--warp-engine adsync|describealign`)**:
  * `adsync` (Default): High-speed Viterbi candidate lattice DP + Monotone PCHIP warp + WSOLA rendering (~15-30s).
  * `describealign` (Legacy): Linear programming DTW + Plateau Consolidation Multilayer stitcher.
* **Direct Warp Fast-Path (`--force-dtw`)**: Bypasses the 100-anchor search entirely and executes non-linear warp alignment directly.
* **Stream Selection (`--ref-stream`)**: Choose specific audio tracks from multi-track MKVs (e.g. `--ref-stream 0:a:1`).
* **Dynamic Sample-Rate PAL Handling (`--pal` / `--no-pal`)**: Probes input sample rates (44.1k vs 48k) to compute exact dynamic clock values for PAL 25fps slowdowns:
  $$\text{asetrate} = \text{input\_sr} \times \frac{23.976024}{25.0}$$
  Use `--no-pal` to enforce native 1.0x speed on shorter Theatrical Cuts.
* **Strict 2.0 Downmix Matrix**: Enforces ITU-R BS.775 matrix (`pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5`), `-ac 2`, and `-ar 48000` for all stereo and night mode deliverables.
* **Master Audio Gap Filling**: Detects deleted scenes / voids in the AD track and fills gaps seamlessly with volume-matched Blu-ray soundtrack audio.
* **Automatic Directory Routing**: Automatically sets `<movie_dir>/output/` for final encodes and `<movie_dir>/scratch/` for temporary analysis files.
* **Intermediate PCM WAV Rendering**: Decompresses and downmixes surround master audio and synchronized AD audio into clean 48kHz WAVs in `scratch/` once before rendering stereo variants. Eliminates redundant DTS/TrueHD decodes, accelerates `loudnorm` by 2-3x, and eliminates FFmpeg filter buffer stalls.
* **Local Machine Configuration**: Automatically reads custom paths (`ffmpeg_path`, `ffprobe_path`, `mkvmerge_path`, `mkvpropedit_path`, `mkvextract_path`, `adsync_path`, `describealign_path`) and default preferences (`default_warp_engine`, `default_speed_detect`, `default_multiband`, `default_threads`, `default_target_i`, `default_anchors`) from `config.local.json` or `config.json` (falling back to `config.default.json`).
* **Thread Throttling (`--threads`)**: Enforces FFmpeg concurrency limit (default: `2` threads) to keep host systems responsive during heavy multi-track processing.
* **Atomic Pre-Decoding**: Uses temporary files (`_tmp.wav`) and atomic `os.replace` to prevent corrupted/half-written cache files from cancelled tasks.
* **Standard Audio Variants**:
  * **AD Tracks**: Generates `-AD (Stereo).m4a` (`alimiter`) and `-AD (Night Mode).m4a` (`loudnorm` -16 LUFS).
  * **Main Audio (`--convert-main`)**: Probes channels and outputs `(Surround).ac3` (5.1 640k), `(Stereo).m4a` (true 2.0 downmix + `alimiter`), and `(Night Mode).m4a` (true 2.0 downmix + `loudnorm`).
* **Fast Test Iterations (`--wav` & `--no-night`)**: Generates 16-bit 48kHz uncompressed PCM WAV stereo deliverables (`.wav`) in seconds without waiting for AAC compression, and bypasses the slow `loudnorm` nighttime filter.
* **Scratch Caching & Clean Runs (`--clean-scratch` / `--wipe-scratch`)**: Automatically caches acoustic correlation results in `scratch/sync_params.json` and reuses warped audio in `scratch/adsync/` across reruns. Use `--clean-scratch` to wipe scratch for a 100% clean run.
* **Preview Mode (`--preview [sec]`)**: Fast test-listening with optional MKV video muxing (`-m`).

### ADSync (Primary -- Non-Linear Time Warping)
* **When**: Cross-correlation detects alternate cuts, structural trims, or drifting frame rates.
* **How**: Acoustic landmark fingerprinting + 3-band speech suppression + Viterbi candidate lattice DP + Monotone PCHIP Cubic Spline Warping + WSOLA rendering.
* **Key Benefits**: Preserves continuous audio playback without slicing into discrete segments, eliminating splice pops, stuttering, and mangled syllables. Completes alignment in ~15-30s.
* **Automatic Speed Probing**: Constellation hashes probe 25/24 speed differences in ~2 seconds.

### describealign (Legacy Fallback -- Dynamic Time Warping)
* **When**: Used when `--warp-engine describealign` is specified or for secondary DTW plot generation.
* **How**: DTW + linear programming (`scipy.optimize.linprog`) generates piecewise alignment plot + `dtw_smooth_stitch.py` applies Plateau Consolidation, Master Audio Gap Fill, VAD cut nudging, zero-crossing snapping, and 50ms crossfading.
* **Command**: `python describealign.py --yes --stretch_audio <master> <ad_track>`
* **Outputs**: `videos_with_ad/ad_<filename>` (aligned MKV), `alignment_plots/<filename>.txt` (timestamp map).
* **Track order gotcha**: Outputs AD as `a:0`, Main as `a:1` -- reverse of our standard. Always remux to swap.
* **Automation gotcha**: `--yes` required (skips interactive prompt that stalls batch scripts).
* **Cleanup**: Run from `scratch/` directory to contain generated `videos_with_ad/` and `alignment_plots/` junk.

### 35mm Film Scans & Piecewise Reel Sync (Rule of Thumb)
* **The 35mm Film Challenge**: 35mm theatrical prints are divided into physical 20-minute reels (~5-6 reels per feature). Splice wear creates discrete missing frame jumps ($\Delta t \approx 0.2\text{s} - 2.0\text{s}$) at reel changes (~20m, ~40m, ~60m, ~75m, ~92m), but **audio runs at exact 1.000000x native speed within every reel**.
* **Why DescribeAlign DTW Fails on 35mm**: DescribeAlign is quantized to 100ms time steps and continuously time-stretches/compresses audio (`-0.6%`, `-4.1%`, `+17.3%`), causing audible pitch distortion and continuous lead/lag sync drift.
* **The Frame-Perfect Fix**:
  1. Detect high-confidence offset clusters across anchors.
  2. Locate silence / scene transitions at reel boundaries.
  3. Extract each reel at **1.000000x native speed** with its exact sub-millisecond offset.
  4. Stitch with **50ms equal-power cosine crossfades** during silence transitions.

### dtw_smooth_stitch.py (Post-DescribeAlign Transition Smoother)
Replaces the old hard-cut `build_dtw_stitch.py`. Parses describealign alignment plots and re-stitches AD audio through a **Multilayer transition smoothing pipeline**:

1. **Layer 0 -- Plateau Consolidation, Wobble Filtering & Gap Filling**: Automatically groups jittery DTW micro-segments sharing the same baseline offset ($\le 120\text{ms}$) into continuous structural blocks/reels, eliminating over 85% of phantom cuts. Protects against DescribeAlign hallucinations by forcing 1.0x native playback across structural film splices, while safely permitting legitimate theatrical (24.000 vs 23.976 fps) and PAL (4.27%) framerate drifts. Automatically detects deleted scenes and perfectly truncates audio repetition to insert clean silence gaps, crossfading them seamlessly with the original Master Audio soundtrack.
2. **Layer 1 -- VAD-Aware Cut Nudging**: At each segment boundary, checks RMS energy in 50ms analysis windows. If speech is active at a cut point, the outgoing boundary is extended forward (or incoming pulled backward) to the nearest silence gap. Configurable: `+/-500ms` search, `200ms` consecutive silence hangover, `0.02` RMS threshold.
3. **Layer 2 -- Zero-Crossing Snapping**: After VAD nudging, each cut point is snapped to the nearest waveform zero crossing within `+/-2ms` to eliminate DC-offset pops and sub-sample clicks.
4. **Layer 3 -- Float64 Resampling & Equal-Power Crossfade**: Uses high-precision `float64` linear interpolation to eliminate continuous timeline drifts (up to 5.0%) without WSOLA stuttering artifacts. Blends segment junctions using 50ms constant-power cosine crossfades ($\cos^2\theta + \sin^2\theta = 1$). Includes an airtight sequence clamp to prevent overlapping audio on tight DescribeAlign cuts.

**Standalone usage**: `python dtw_smooth_stitch.py <alignment_plot.txt> <ad_audio> <output.m4a>`
**Integrated**: `batch_orchestrator.py` automatically imports and calls `run_smooth_stitch()` after describealign produces an alignment plot. Falls back to describealign's raw output if smooth stitching fails.

### batch_orchestrator.py Strategy Decision Tree
Automatically selects per episode:
1. **Direct Mux** -- single consistent offset, no black screens.
2. **Frame-Perfect** -- black screens detected and offset count matches gap count.
3. **DTW (describealign + smooth stitch)** -- no black screens but multiple drifting offsets. Runs describealign for alignment, then re-stitches with `dtw_smooth_stitch.py` for clean transitions.
4. **Piecewise Reel Sync** -- 35mm scans with discrete reel jumps; keeps 1.0x native playback speed and splices at silence.

### Other Core Scripts
* `convert_main_tracks.py`: Extract & downmix 5.1/7.1 surround masters to true 2.0 Stereo AAC and Night Mode AAC.
* `convert_ad_2pass.py`: Standalone 2-pass EBU R128 loudness normalization on existing audio files.
* `set_commentary_flags.py`: Batch flag audio tracks matching commentary keywords using `mkvpropedit`.
* `plex_ad_commentary_collections.py`: Maintain static Plex collections for AD and Commentary tracks.
* `batch_frameperfect.py`: Remove black screen commercial gaps with exact cut points.
* `remove_commercial_breaks.py`: Black screen and silence detector for video containers.
* `parse_mpls.py`: Extracts 45kHz PTS timestamps from Blu-ray `.mpls` playlists.

See `scripts/README.md` for the full scripts index.

---

## 3. Critical Alignment Insights & Best Practices

### 1. Sample-Accurate PAL $\to$ Film Conversion (`asetrate` vs `atempo`)
* **The Problem with `atempo`**: `atempo` uses WSOLA time-domain grain overlap. Over a 2.5-3.5 hour theatrical film, small micro-errors accumulate, causing perceptible pitch/phase smearing and subtle synchronization drift.
* **The Solution**: Use sample-rate playback clock manipulation:
  $$\text{target\_rate} = \text{input\_sr} \times \frac{23.976023976}{25.0}$$
  Followed by `aresample=48000`.
* **Dynamic Sample-Rate Rule**: Always probe the input file's native sample rate first:
  - For **48,000 Hz** input: `asetrate=46033.966,aresample=48000`
  - For **44,100 Hz** input: `asetrate=42293.706,aresample=48000`
  *Never hardcode 46033.966 Hz*, as applying that to a 44.1 kHz input speeds up the audio by $+4.3\%$ instead of slowing it down by $-4.1\%$.

### 2. DTW Segment Resampling Fidelity (Sinc vs. Linear Interpolation)
* **The Pitfall of `np.interp`**: In Digital Signal Processing, linear interpolation (`np.interp` / first-order hold) acts as a triangular low-pass filter in the frequency domain with response:
  $$H(f) = \left( \frac{\sin(\pi f / f_s)}{\pi f / f_s} \right)^2$$
  Applying linear interpolation across millions of audio samples causes $>46\%$ energy attenuation above 8-10 kHz, creating a severely muffled "AM radio brickwall" artifact.
* **The Rules**:
  1. **Bit-Perfect Native Audio**: For all native-speed segments ($\text{rate change} < 1.0\%$), never apply time interpolation. Preserve decoded PCM samples bit-for-bit directly from `samples[s:e]`.
  2. **Bandlimited Sinc Resampling**: When genuine rate changes are required (e.g. PAL 4.27% stretch), always use bandlimited sinc/Fourier resampling (`scipy.signal.resample` or `soxr`) to maintain 100% full 20Hz-20kHz frequency response with 0.0 dB attenuation.
  3. **Memory Protections**: Avoid allocating full-timeline `float64` interpolation arrays via `np.linspace` across 3-hour files ($>480\text{M}$ samples), which triggers Python `Unable to allocate 3.64 GiB` out-of-memory errors. Use `float32` chunking or FFmpeg streaming resamplers.

### 3. TV Broadcast vs. Home Video Master Discrepancies
* Region-tagged tracks (e.g. `[UK]`, `[AU]`) are frequently TV broadcast captures with commercial excisions, pacing trims, and censorship cuts.
* **Diagnostic Signature**: Multi-anchor correlation exhibits wildly oscillating positive and negative offsets (e.g. $-12\text{s} \to +28\text{s} \to -39\text{s}$) with confidence $< 10\%$. Single-offset or simple linear stretch cannot sync these tracks.

### 4. ChapterDB Cut Verification
* Community submissions on ChapterDB frequently paste Extended Edition chapter titles over Theatrical disc timestamps (or vice versa).
* Always verify chapter counts against official DVD/Blu-ray inserts (e.g. Movie 1 Theatrical = 40 chapters, Movie 2 Theatrical = 53 chapters, Movie 3 Theatrical = 60 chapters).

### 5. Strict 2.0 Stereo Downmixing for Surround Sources
* **Pitfall**: When extracting a 5.1/7.1 master track (`channels >= 6`), passing `-af "alimiter=..."` or `-af "loudnorm=..."` without an explicit downmix pan filter and `-ac 2` will cause FFmpeg to retain all 6 channels, producing a 5.1-channel AAC stream instead of 2.0 stereo.
* **The Rule**: Whenever creating a "Stereo" or "Night Mode" deliverable from a surround source ($\ge 6$ channels), ALWAYS:
  1. Apply the ITU-R BS.775 stereo downmix matrix before limiters/loudnorm:
     `pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5`
  2. Explicitly specify `-ac 2` in the FFmpeg command line.
  3. Example Stereo AAC:
     `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,alimiter=level_in=1:level_out=0.98:limit=0.98" -ac 2 -c:a aac -b:a 256k -ar 48000`
  4. Example Night Mode AAC:
     `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ac 2 -c:a aac -b:a 256k -ar 48000`

### 6. PAL Speedup vs. Shorter Alternate Cuts
* **Pitfall**: An AD track with shorter total duration (e.g. 97.6m vs 102.8m) does not automatically mean a PAL 25.0 fps speedup.
* **Diagnostic**:
  - Genuine PAL speedup runs at a constant 0.95904 ratio across all individual scenes.
  - An alternate **Theatrical Cut** runs at **100% native 1.0x film speed (23.976 fps)** within each scene and is shorter only because of scene excisions.
* **Rule**: Applying PAL slowdown (`asetrate=42293.706`) to a native 1.0x track introduces artificial 4.27% drift. Always test local scene correlation. For alternate cuts, decode at native 1.0x and use DescribeAlign DTW to bridge scene gaps.

### 7. MKVPropEdit Track Selector Syntax
* `--edit track:=<uid>` selects track by Matroska Track UID (equals sign `=`).
* `--edit track:@<n>` selects track by 1-based Track Number element (`@`).
* Never use `@` with 64-bit Track UIDs, as `mkvpropedit` treats it as a track number and fails.

---

## 4. MKV Chapters

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

## 5. Standard Paths
* **Project scripts**: `scripts\`
* **ADSync engine**: `adsync_path` / sibling `..\ADSync`
* **DescribeAlign engine**: `describealign_path` / `describealign.py`
* **MKVToolNix binaries**: `mkvmerge_path`, `mkvpropedit_path`, `mkvextract_path`
* **FFmpeg binaries**: `ffmpeg_path`, `ffprobe_path` / `ADMaster\bin\` (auto-resolved by Python scripts)

---

## 6. Pure ASCII Enforcement Rule
* **All Python scripts, JSON configuration files, markdown documentation, comments, and console print outputs MUST be 100% pure ASCII.**
* Never use Unicode checkmarks (use `[OK]`), Unicode cross marks (use `[X]`), em-dashes (use `--`), en-dashes (use `-`), Unicode bullets (use `*`), Unicode arrows (use `->` or `<-`), curly quotes (use standard `'` or `"`), mathematical symbols (use `+/-`, `*`, `pi`, `^2`), or box-drawing characters.
