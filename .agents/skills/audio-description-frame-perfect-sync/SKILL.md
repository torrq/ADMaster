---
name: audio-description-frame-perfect-sync
description: Audio description syncing, encoding standards, and commercial removal workflows.
---

# Audio Description Syncing & Processing

## Communication Guidelines
* Keep responses extremely concise.
* Eliminate conversational filler, personality, and excessive exclamations (e.g. no "ah", "classic", "the best part").
* Do not anthropomorphize code (e.g. do not say "the script is smart").
* Provide scripts, commands, and factual explanations only.

## Standard Encoding Specs

| Format | Codec | Bitrate | Notes |
|--------|-------|---------|-------|
| Stereo AAC | `aac` | `256k` | Always `-ar 48000`. Use `loudnorm=I=-16:TP=-1.5:LRA=11` for normalized variant. |
| 5.1 AC3 | `ac3` | `640k` | Use 640k to preserve spatial clarity and high frequencies for lossless/high-quality sources. |
| Stereo FLAC | `flac` | lossless | For native stereo tracks only (no downmix). |
| 5.1->Stereo Downmix | `pan=stereo\|c0=c0+0.7071*c2+0.7071*c4\|c1=c1+0.7071*c2+0.7071*c5` | | Apply before `loudnorm`. |

All output files go in an `output/` subdirectory within the project folder.
All temporary, intermediate analysis WAVs, and DTW scratch files MUST be stored inside a `scratch/` subdirectory within the project folder (e.g. `F:\uploads\<Movie>\scratch\`). Never write temp files to the root or shared directories.

**Standard stereo mix convention**: When processing surround main audio alongside AD, always include a normalized stereo AAC downmix of the main track unless the user says otherwise.
**Night Mode convention**: From now on, the `loudnorm` processed track should be referred to as "Night Mode". When generating an AD track, always create TWO stereo AAC tracks:
1. A regular stereo track that isn't normalized (but uses `alimiter=level_in=1:level_out=0.98:limit=0.98` to prevent clipping).
2. A normalized "Night Mode" track using `loudnorm=I=-16:TP=-1.5:LRA=11`.

**Surround AD/Comm convention**: Whenever an AD or Commentary track natively contains 5.1/7.1 surround sound (6+ channels), ALWAYS generate a synced high-bitrate 5.1 AC3 (640k) in addition to the standard Stereo AAC and Night Mode downmixes. Do not restrict surround preservation strictly to Main tracks.

### Manual FFmpeg Encode Reference
```powershell
# Stereo AAC (normalized) from surround source:
ffmpeg -y -ss <offset> -i "source.dts" -af "aresample=48000,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ar 48000 -c:a aac -b:a 256k "output.m4a"

# 5.1 AC3 from surround source:
ffmpeg -y -ss <offset> -i "source.dts" -c:a ac3 -b:a 640k "output.ac3"

# Stereo AAC (NOT normalized, e.g. AD from external source):
ffmpeg -y -ss <offset> -i "source.mp3" -c:a aac -b:a 256k "output.m4a"
```

## Critical Rules

* **Interactive stdin bug**: FFmpeg actively listens to standard input (`stdin`) by default. If a user clicks the terminal or accidentally pastes text during a long batch encode, FFmpeg can enter interactive command mode and hang indefinitely. ALWAYS include the `-nostdin` flag in all automated FFmpeg commands to prevent terminal lockups.
* **loudnorm 96kHz bug**: `loudnorm` operates internally at 192kHz and will upsample output to 96kHz. ALWAYS append `-ar 48000` after the filter chain.
* **DTS-HD MA + loudnorm stall**: Feeding lossless DTS-HD MA directly into `loudnorm` causes severe FFmpeg buffer stalls (~0.6x speed). Extract to intermediate WAV/W64 first, then apply `loudnorm` on the intermediate. Use `-f w64` (Wave64) for 5.1+ tracks that exceed the 4GB WAV limit.
* **MP3 seeking**: Never use `-ss` before `-i` on MP3 files. Decode to WAV first (`-c:a pcm_s24le`), then seek on WAV for sample-exact accuracy.
* **Raw bitstream seeking (DTS/AC3/EAC3 stall bug)**: Raw audio bitstreams (`.dts`, `.ac3`, `.eac3`, `.thd`, `.dtshd`) have NO container index. FFmpeg cannot fast-seek them. Placing `-ss` before `-i` hangs/stalls indefinitely. Placing `-ss` after `-i` forces a linear decode from byte 0 to the seek point every single call -- for a 2-hour DTS file seeking to 90 minutes, that's 90 minutes of decoding just to extract 30 seconds. The sync script calls `extract_mono_wav_fast()` 20+ times per track, so this compounds into 10+ minute stalls. **FIX**: `sync_ad_track.py` now pre-decodes each input file to a tiny 8kHz mono WAV ONCE via `predecode_to_mono_8k()`, then does all correlation math via in-memory numpy array slicing. Zero repeated FFmpeg calls. Never revert to per-anchor FFmpeg seeking for raw bitstreams.
* **Offset sign convention**:
  * `offset > 0` (AD behind) -> trim start with `-ss <offset>` placed AFTER `-i`.
  * `offset < 0` (AD ahead) -> prepend silence with `adelay=<ms>|<ms>`. Never use `-ss` for negative offsets.
* **Track ordering**: Main audio = `a:0` (default), AD = `a:1` (visual_impaired disposition).

---

## Workflow 1: Universal Sync & Conversion (`sync_ad_track.py`)

**Primary Rule**: Always use `sync_ad_track.py` for all movie processing and queue operations rather than writing custom one-off scripts.

```powershell
# 1. Full Directory Auto-Discovery (AD sync + Main 5.1 & True 2.0 AAC conversion):
python scripts/sync_ad_track.py "F:\uploads\queue\<MovieName>" --convert-main

# 2. Specific audio stream with forced DTW alignment (e.g. Theatrical vs Extended Cut):
python scripts/sync_ad_track.py "F:\uploads\queue\<MovieName>" --ref-stream 0:a:1 --force-dtw --no-pal --convert-main

# 3. Fast preview test-listen multiplexed with video:
python scripts/sync_ad_track.py "F:\uploads\queue\<MovieName>" --preview 180
```

**Key Features**:
* **Directory Auto-Discovery**: Point to any folder containing `source/` and `tosync/`. Automatically resolves reference audio/video, pairs all AD/commentary tracks, and outputs deliverables to `output/`.
* **100-Anchor FFT Cross-Correlation**: Samples 100 anchors across the movie via fast FFT in memory (pre-decoding streams once to 8kHz mono WAV in `scratch/`).
* **Direct DTW Bypass (`--force-dtw`)**: Bypasses the 100-anchor cross-correlation search entirely and jumps straight to DescribeAlign DTW piecewise alignment and Multilayer smooth stitching.
* **Stream Selection (`--ref-stream`)**: Specify exact audio stream in multi-track MKVs (e.g. `--ref-stream 0:a:1`).
* **Dynamic Sample-Rate PAL Handling (`--pal` / `--no-pal`)**: Probes input sample rates (44.1k vs 48k) to compute exact dynamic clock values for PAL 25fps slowdowns:
  $$\text{asetrate} = \text{input\_sr} \times \frac{23.976024}{25.0}$$
  Use `--no-pal` to enforce native 1.0x speed on shorter Theatrical Cuts.
* **Standard Multi-Track Encoding**:
  * **AD Tracks**: Outputs both `<Title>-AD (Stereo).m4a` (with `alimiter` clipping protection) and `<Title>-AD (Night Mode).m4a` (EBU R128 `loudnorm` -16 LUFS) with `-ac 2`.
  * **Main Audio (`--convert-main`)**: Converts 5.1/7.1 sources to `(Surround).ac3` (5.1 640k), `(Stereo).m4a` (true 2.0 downmix via ITU-R BS.775 matrix + `alimiter`), and `(Night Mode).m4a` (true 2.0 downmix + `loudnorm`). Native stereo sources bypass the surround pan matrix.
* **Sub-Frame Rounding**: Disregards imperceptible offsets $< 8\text{ms}$ as exact 0.000s alignment.

---

## Workflow 2: DescribeAlign DTW Piecewise Alignment (Complex Edits)

**When to use**: AD and reference have **different edits** -- commercial breaks removed, scenes cut/added, or network vs bluray cuts. Cross-correlation fails because no single offset works across the timeline.

**Location**: `describealign.py` (external tool, resolved from PATH or project config)

### Command
```powershell
python describealign.py --yes --stretch_audio <master_video_or_audio> <ad_track>
```

* `--stretch_audio`: Stretches/compresses the AD to match the master timeline. Keeps original video audio tracks; adds AD as first audio track.
* `--yes`: **Required for automation.** Skips interactive confirmation prompt that stalls unattended scripts.

### Output
* `videos_with_ad/ad_<filename>` -- MKV/audio with aligned AD track.
* `alignment_plots/<filename>.txt` -- Piecewise alignment map showing offsets, rate changes, and timestamp mappings per segment.
* `alignment_plots/<filename>.png` -- Visual alignment curve plot.

### Track Order Gotcha
DescribeAlign outputs AD as `a:0` and Main as `a:1` -- the **reverse** of our standard. Always remux to swap:
```powershell
ffmpeg -y -i "describealign_output.mkv" -map 0:v:0 -map 0:a:1 -map 0:a:0 -c copy ^
  -metadata:s:a:0 title="Main, Stereo" -disposition:a:0 default ^
  -metadata:s:a:1 title="AD, Stereo" -disposition:a:1 visual_impaired ^
  "final_output.mkv"
```

### Limitations & Micro-Drifts Warning
* **DO NOT use describealign for continuous micro-drift** (e.g. 0.5s drift over 2 hours). DTW is meant for massive structural edits (commercial breaks, missing scenes). If fed a track with continuous framerate drift, DTW will hallucinate jumps due to low acoustic similarity (AD narration vs raw dialogue) and make the sync worse and jerky.
* **Always use `atempo` for micro-drifts.** If `sync_ad_track.py` detects a tiny stretch factor (e.g. `0.9998x` or `1.00008x`), manually apply it via the `atempo` filter. A 0.00008x stretch equals a 0.5s lip-sync drift over a 2-hour movie, which is highly noticeable.
* Audio segments stretched >10% introduce audible pitch distortion.
* Slower than simple cross-correlation (full DTW feature computation).
* Creates `videos_with_ad/` and `alignment_plots/` directories -- run from `scratch/` or clean up after.

### Transition Smoothing (dtw_smooth_stitch.py)
DescribeAlign's raw output can slice through narrator speech mid-sentence at segment boundaries. The `dtw_smooth_stitch.py` script fixes this with a **Multilayer transition pipeline** applied after describealign produces its alignment plot:

1. **Layer 1 -- VAD-Aware Cut Nudging**: At each segment boundary, checks RMS energy in 50ms windows. If speech is active at a cut, extends the outgoing boundary forward (or pulls incoming backward) to the nearest silence gap. Parameters: `+/-500ms` search, `200ms` silence hangover, `0.02` RMS threshold.
2. **Layer 2 -- Zero-Crossing Snapping**: Snaps each cut to the nearest waveform zero crossing within `+/-2ms` to eliminate DC-offset pops.
3. **Layer 3 -- Equal-Power Crossfade**: 50ms cosine-shaped crossfade at every junction using overlap-add mixing.

**Standalone**: `python dtw_smooth_stitch.py alignment.txt AD.mka output.m4a`
**Integrated**: `batch_orchestrator.py` automatically imports and calls it after describealign runs.

### Integration with batch_orchestrator.py
`batch_orchestrator.py` automatically selects the right strategy per episode:
1. **Direct Mux** -- single consistent offset, no black screens -> simple `-ss` trim + mux.
2. **Frame-Perfect** -- black screens detected and offset count matches -> FFmpeg `trim`/`concat` re-encode.
3. **DTW (describealign + smooth stitch)** -- no black screens but multiple drifting offsets -> describealign computes alignment, then `dtw_smooth_stitch.py` re-stitches with VAD-protected transitions.

---

## Workflow 2B: 35mm Film Scans & Piecewise Reel-Aware Sync

**When to use**: Aligning retail / digital Audio Description (or foreign dub) tracks to **35mm film scans** or archival theatrical prints.

### The 35mm Film Print Problem
* **Physical Reels**: 35mm theatrical prints are split across physical 20-minute reels (~5 to 6 reels for a 100-120 minute feature: ~20m, ~40m, ~60m, ~75m, ~92m).
* **Missing Head/Tail Frames**: Archival projection wear and reel splices produce **discrete frame gaps** ($\Delta t \approx 0.2\text{s} - 2.0\text{s}$, or 6-48 missing frames) at each reel change.
* **100% Native Speed Inside Reels**: Within each individual reel, the audio runs at **exact 1.000000x bit-perfect native speed and pitch**. The offset is completely flat.

### Why DescribeAlign DTW Fails on 35mm Scans
* **100ms Quantization**: DescribeAlign hardcodes `TIMESTEPS_PER_SECOND = 10` (100ms bins), producing an intrinsic jitter of $\pm 50 - 100\text{ms}$ (~2-3 video frames).
* **Warping Artifacts**: DescribeAlign does not know they are physical reel splices; it attempts to continuously stretch and warp the audio (`-0.6%`, `-4.1%`, `+17.3%`), corrupting pitch and causing dialogue to drift in and out of sync continuously across scenes.

### Recommended 35mm Workflow: Piecewise Reel Splicing
1. **Anchor Scan**: Sample 100-600 correlation points across the movie.
2. **Reel Offset Clustering**: Group high-confidence ($> 20\%$) points into discrete offset clusters corresponding to the physical reels (e.g. `+9.217s` $\to$ `+9.463s` $\to$ `+9.819s` $\to$ `+10.752s` $\to$ `+12.431s` $\to$ `+14.357s`).
3. **Silence Boundary Detection**: Locate the natural silence / scene transition nearest each offset jump.
4. **1.0x Native Splicing & Crossfade**: Extract each reel segment at **exact 1.000000x native speed**, apply each reel's measured offset, and stitch with **50ms equal-power cosine crossfades** (`\cos^2 + \sin^2 = 1`) during silence gaps.

---

---

## Workflow 3: Commercial Break Removal (Frame-Perfect)

**When to use**: Master MKV has black-screen commercial breaks that need removal while preserving frame-exact AD sync.

**The problem**: `mkvmerge --split` snaps to keyframes, deleting 1-2s of show footage and breaking AD alignment.

### Option A: FFmpeg Re-encode (Seamless)
Use `blackdetect` to find breaks, then a single FFmpeg `trim`/`atrim`/`concat` filter graph to surgically excise them without keyframe snapping. Re-encodes video (`libx264 -preset fast -crf 18`).

### Option B: Safe Buffer (Fast, No Re-encode)
Shrink the cut window by 2s on each side so `mkvmerge` keyframe snaps stay inside the black screen. Result: ~4s black transition, zero footage loss, instant processing.

### Option C: VAD Speech Rescue (Keyframe Recovery)
When keyframe snapping deletes footage, extract the missing AD audio chunk, collapse to mono center-channel, and overlay with 50ms fade onto the next scene boundary.

**Rule**: Never use `mkvmerge` for seamless removal. Never use `describealign` on a keyframe-snapped MKV.

---

## Pitfalls & Troubleshooting Reference

### 1. Bidirectional PAL / NTSC Stretch Detection
* **PAL Broadcast Speedup**: PAL audio runs at 25 fps, which is $1.042709\times$ faster than 23.976 fps film.
* **Directional Ratio Check**:
  * If calculating stretch to slow down the 25 fps AD track: $\text{ratio} = 23.976 / 25.0 = 0.959039\times$.
  * If calculating stretch to speed up reference: $\text{ratio} = 25.0 / 23.976 = 1.042709\times$.
* **Pitfall**: Scripts checking only $1.030 - 1.050\times$ will reject $0.959\times$, falsely declare a structural cut mismatch, and trigger unnecessary DTW fallback. Always support both $0.950 - 0.970\times$ and $1.030 - 1.050\times$.

### 2. Never Pass Large Video MKVs to DescribeAlign
* **Pitfall**: Passing 10-40GB MKV files directly to `describealign.py` causes it to attempt video remuxing via FFmpeg stdin/stdout pipes. This leads to fatal FFmpeg buffer overflows, bitstream parsing crashes (e.g. truncated H.264 SEI data), and wasted disk space.
* **Rule**: ALWAYS extract master reference audio to `.w64` or `.wav` first and pass the extracted audio file to DescribeAlign.

### 3. DescribeAlign File Extension Whitelist
* **Pitfall**: DescribeAlign uses strict extension sets (`VIDEO_EXTENSIONS` and `AUDIO_EXTENSIONS`). If an intermediate file uses an extension not in the set (e.g. `.w64`), it throws `RuntimeError: No files with valid extensions found`.
* **Rule**: Ensure `.w64`, `.wav`, `.flac`, and `.mka` are included in `AUDIO_EXTENSIONS`.

### 4. Wrapper Script Cache Invalidation
* **Pitfall**: Custom batch/wrapper scripts with `if not os.path.exists(output_file): skip` will silently skip re-encoding when re-running a movie after fixing a bug, leaving stale, out-of-sync files in `output/`.
* **Rule**: When fixing or re-running alignment workflows, always overwrite (`-y` in FFmpeg) or purge stale intermediates in `output/` and `scratch/`.

### 5. ChapterDB Extended vs. Theatrical Cut Contamination
* **Pitfall**: ChapterDB entries often contain correct timestamps extracted from retail discs, but community contributors frequently copy-pasted Extended Edition chapter titles over Theatrical Edition timelines (or vice-versa), resulting in phantom scenes and repeating titles at the end.
* **Rule**: Always cross-reference chapter counts and scene names against official release indices (e.g. Theatrical vs Extended releases).

### 6. Identifying Wildly Different Cuts (TV Broadcasts vs. Retail Cuts)
* **The Reality**: Tracks labeled simply with region tags (e.g. `[UK]`, `[AU]`) are frequently **TV broadcast captures** rather than retail DVD/Blu-ray rips.
* **Symptoms of Broadcast / Alternate Cuts**:
  1. **Excised Commercial Breaks**: Abrupt jumps where commercials were removed.
  2. **Broadcast Trims & Watershed Edits**: Minor scene trims made for time slots or broadcast censorship.
  3. **Oscillating Anchor Offsets**: Multi-anchor correlation exhibits wild positive and negative jumps across the film (e.g. `-12s` $\to$ `+28s` $\to$ `-10s` $\to$ `+9s` $\to$ `-39s` $\to$ `-55s`) with very low overall confidence ($< 10\%$).
* **Rule**: When cross-correlation produces erratic, oscillating offset deltas across multiple anchors, never attempt to force a single global offset or linear speed stretch. The track is an entirely different cut--either replace the source with a verified retail rip or use DescribeAlign DTW piecewise reconstruction.

### 7. Dynamic Sample-Rate Handling for `asetrate` Resampling
* **The Math**: Sample-accurate PAL $\to$ Film rate conversion changes the sample clock:
  $$\text{target\_rate} = \text{input\_sr} \times \frac{23.976023976}{25.0}$$
* **Pitfall**: Hardcoding $46033.966\text{ Hz}$ assumes an input rate of $48,000\text{ Hz}$. If the source audio is $44,100\text{ Hz}$ (common in MP3s), applying $46033.966\text{ Hz}$ speeds up the audio by $+4.3\%$ instead of slowing it down by $-4.1\%$.
* **Rule**: Always dynamically probe the input sample rate with `ffprobe` and calculate:
  - For $48,000\text{ Hz}$ input: $\text{asetrate} = 46033.966\text{ Hz}$
  - For $44,100\text{ Hz}$ input: $\text{asetrate} = 42293.706\text{ Hz}$
  Followed by `aresample=48000`.

### 8. Digital Audio Resampling Fidelity (Sinc vs. Linear Interpolation)
* **The DSP Math**: Linear interpolation (`np.interp` / first-order hold) behaves as a triangular low-pass filter in the frequency domain with frequency response:
  $$H(f) = \left( \frac{\sin(\pi f / f_s)}{\pi f / f_s} \right)^2$$
  This causes severe high-frequency attenuation ($>45\%$ energy loss at $15\text{ kHz}$), creating a dull, muffled "AM radio brickwall" effect.
* **The Rules**:
  1. **Bit-Perfect Native Audio**: For all native-speed segments ($\text{rate change} < 1.0\%$), never apply time interpolation. Preserve decoded PCM samples bit-for-bit.
  2. **Bandlimited Sinc Resampling**: When genuine rate changes are required (e.g. PAL 4.27%), always use bandlimited sinc/Fourier resampling (`scipy.signal.resample` or `soxr`) to maintain 100% full 20Hz-20kHz frequency response with 0.0 dB attenuation.

### 9. Strict 2.0 Stereo Downmixing for Surround Sources
* **Pitfall**: When extracting a 5.1 or 7.1 master track (`channels >= 6`), passing `-af "alimiter=..."` or `-af "loudnorm=..."` without an explicit downmix pan filter and `-ac 2` will cause FFmpeg to process all 6 channels, producing a **5.1-channel AAC stream** instead of 2.0 stereo.
* **The Rule**: Whenever creating a "Stereo" or "Night Mode" deliverable from a surround source ($\ge 6$ channels), ALWAYS:
  1. Apply the ITU-R BS.775 stereo downmix matrix before any limiters or normalization:
     `pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5`
  2. Explicitly specify `-ac 2` in the FFmpeg command line.
  3. Example Stereo AAC:
     `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,alimiter=level_in=1:level_out=0.98:limit=0.98" -ac 2 -c:a aac -b:a 256k -ar 48000`
  4. Example Night Mode AAC:
     `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ac 2 -c:a aac -b:a 256k -ar 48000`

### 10. Disambiguating PAL Speedup vs. Shorter Alternate Cuts
* **The Pitfall**: An AD track is shorter than the Blu-ray (e.g. `5860s` vs `6168s`). Assuming shorter total runtime automatically implies a 25.0 fps PAL speedup is a critical mistake.
* **The Diagnostic**:
  - If a movie is a 25.0 fps PAL speedup (e.g. PAL broadcast vs NTSC master), the duration ratio will be almost exactly $\frac{23.976024}{25.0} \approx 0.95904$, and every individual scene will run 4.27% faster.
  - If a movie is from a shorter **Theatrical Cut**, the audio within each continuous scene runs at **100% native 1.000000x film speed (23.976 fps)**.
* **The Rule**: Applying a PAL slowdown (`asetrate=42293.706`) to an AD track that is already running at 1.0x native speed forces the audio to play 4.27% too slow, causing continuous drift across every scene and confusing DTW. Always test local scene correlation before applying tempo/sample-rate conversions. When a track is an alternate cut running at 1.0x speed, decode at native 1.0x rate and use DescribeAlign DTW to bridge the excised/inserted scenes.

### 11. MKVPropEdit Track Selector Syntax (`=` for UID vs `@` for Track Number)
* **The Pitfall**: In `mkvpropedit`, `--edit track:@<val>` interprets `<val>` as a 1-based Track Number (e.g. `@3`). Passing a 19-digit Matroska Track UID with `@` causes `mkvpropedit` command-line parsing errors.
* **The Rule**:
  - For **Track UID**: Use `--edit track:=<uid>` (with an equals sign `=`).
  - For **Track Number**: Use `--edit track:@<number>` or `--edit track:<n>`.
  - For **Audio Track Number**: Use `--edit track:a<n>` (e.g. `--edit track:a2`).

### 12. Legacy Optical Audio vs. Modern Surround Remix Alignment
* **The Situation**: Classic films frequently feature multiple audio tracks with significant mix differences (e.g. original 1.0 Mono optical audio vs. modern 5.1/7.1 remix with new voice dubs, replacement foley, and re-recorded music).
* **The Rule**: If DescribeAlign produces low similarity ($< 25\%$) against one stream, test correlation against the alternative audio streams. An AD track recorded for a home-video remix will align to the 5.1 stream, whereas an older broadcast AD track may align strictly to the original optical mono stream.
