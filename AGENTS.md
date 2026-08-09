# Audio Description Processing & BatchEncoder Master Specs

Project repository for Audio Description (AD) extraction, multi-track alignment, seamless branching reconstruction, BatchEncoder / FFmpeg automation, and MKV chapter manipulation.

---

## 1. BatchEncoder Setup & Configuration Guide (Step-by-Step)

### A. Directory Structure & Environment Setup
* **Installation Root**: `C:\Path\To\BatchEncoder-5.1-x64\` (Portable Mode via `BatchEncoder.portable`).
* **Source Code**: `C:\Path\To\git\BatchEncoder\`

### B. Tool Executable Setup (`tools/`)
Tools define the CLI binary paths linked to format IDs:
1. Edit `tools/ffmpeg_x64.xml` (or `tools/ffmpeg_x86.xml`) to point `path` to the binary:
   ```xml
   path="C:\Path\To\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"
   ```
2. Map supported format IDs in the tool XML: `formats="FFMPEG_FLAC_AAC,FFMPEG_AC3,..."`.
3. Edit `tools/eac3to_x64.xml` to point `path` to `C:\Path\To\eac3to\eac3to_3.63\eac3to.exe`.

### C. Format & Preset Setup (`formats/`)
Formats define command templates, file extensions, and preset options:
1. **Create Format XML** in `formats/` (e.g., `formats/FFMPEG_FLAC_AAC.xml`):
   ```xml
   <Format id="FFMPEG_FLAC_AAC" name="Surround FLAC to Stereo AAC - ffmpeg" template="$EXE -y -i $INFILE $OPTIONS -c:a aac -ar 48000 $OUTFILE" function="progress\GetProgress_FFmpegEnc.lua" formats="flac" extension="stereo.m4a">
       <Presets>
           <Preset name="Default (Stereo 256k + Anti-Clipping Limiter)" options="-af &quot;pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,alimiter=level_in=1:level_out=0.98:limit=0.98&quot; -b:a 256k"/>
           <Preset name="Stereo 256k + Loudness Normalized (EBU R128)" options="-af &quot;pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11&quot; -b:a 256k"/>
           <Preset name="Night Mode / Easy Listening (Dialogue Boost + Compressed Dynamics + R128)" options="-af &quot;pan=stereo|c0=0.5*c0+1.0*c2+0.5*c4|c1=0.5*c1+1.0*c2+0.5*c5,acompressor=threshold=-21dB:ratio=3.5:attack=10:release=100,loudnorm=I=-16:TP=-1.5:LRA=7&quot; -b:a 256k"/>
       </Presets>
   </Format>
   ```
2. **Surround Blu-ray Downmix Format (`FFMPEG_BLURAY_AAC.xml`)**:
   We maintain a specific format `FFMPEG_BLURAY_AAC` (linked to `ffmpeg_x64` in tools) that accepts `ac3,eac3,dts,dtshd,thd,flac,wav` and downmixes 5.1/7.1 to stereo AAC. Because BatchEncoder runs a single command template, it uses the **1-pass** `loudnorm` filter (which is highly effective but less rigorous than the 2-pass Python script).

### D. Progress Parsing Setup (`progress/`)
Link a Lua script in `function="..."` of format XML to report live `0%-100%` UI progress:
* For FFmpeg: `progress/GetProgress_FFmpegEnc.lua` (parses `Duration` and `time=` stderr logs).
* For eac3to: `progress/GetProgress_eac3to.lua` (parses percentage completion).

### E. Output Naming Templates (`Outputs.xml` & `Options.xml`)
* `Outputs.xml` stores pattern replacements (e.g. `$InputPath$\$Name$.$Ext$`, `$InputPath$\$Name$_stereo.$Ext$`).
* Setting `extension="stereo.m4a"` in format XML forces `$Ext$` to evaluate to `stereo.m4a`—yielding `Filename.stereo.m4a`.

### F. Validation Commands
Run PowerShell validation scripts inside `C:\Path\To\BatchEncoder-5.1-x64\` to confirm paths and tools:
```powershell
.\validate-tools-path.ps1
.\validate-formats-path.ps1
.\validate-tools-file.ps1
```

### G. Critical Architectural Quirks for LLM Agents
* **RAM Caching & Overwrite on Exit**: `BatchEncoder.exe` caches formats in RAM and serializes them back to `formats/*.xml` when closing. **ALWAYS kill `BatchEncoder.exe` before editing format XMLs**, or drag-and-drop the updated `.xml` file directly into the BatchEncoder GUI.
* **`CPreset` Limitations**: Presets only store CLI flags (`$OPTIONS`). Output file extensions are format-level (`CFormat::szOutputExtension`).

---

## 2. Audio Description (AD) Alignment & Processing Pipeline

### Universal Acoustic Cross-Correlation Alignment & 2-Pass Loudness Pipeline (`sync_ad_track.py`)
* **Multi-Format Input Support**: Accepts reference main audio from any container or codec (`.ac3`, `.eac3`, `.dts`, `.dtshd`, `.thd`, `.flac`, `.wav`, `.mkv`, `.mp4`) and AD input tracks (`.mp3`, `.m4a`, `.ac3`, `.flac`, `.wav`).
* **Ultra-Fast 8kHz Acoustic Correlation**: Uses 8000Hz mono PCM resampling and compact 30s/60s search windows to compute delay offset in < 0.05 seconds with sub-millisecond precision.
* **Automatic Speed Corrections**: Automatically corrects PAL 25.0 fps speedup (`atempo=1.03907x`) and NTSC pulldown (`atempo=0.99956x`), with `--no-stretch` option to enforce native 1.0x rate.
* **2-Pass EBU R128 Loudness Normalization**: Pass 1 measures exact integrated loudness (`input_i`), True Peak (`input_tp`), and threshold via FFmpeg JSON output. Pass 2 applies linear gain scaling (`linear=true`) with `TP=-1.5 dB` true-peak protection.
* **Fast Preview Testing Mode (`--preview [sec]`)**: Generates short 3-to-5 minute sample tracks in seconds so users can test-listen to sync alignment before running full-length movie encodes.
* **CLI Command Examples**:
  ```powershell
  # Instant 3-minute preview sample for fast sync test-listening:
  python sync_ad_track.py -r "F:\uploads\Movie.eac3" -a "Movie_AD.mp3" --preview 180 --no-stretch

  # Full 2-pass render for multiple labeled AD tracks:
  python sync_ad_track.py `
    -r "F:\uploads\Movie.Title.eac3" `
    -a "UK=F:\uploads\ad\UK_AD.mp3" "US=F:\uploads\ad\US_AD.mp3" `
    -o "F:\uploads\ad_synced" `
    -t "Movie Title"
  ```

### Critical Sync Alignment Rules (Verified 2026-07-30)
* **Correlation Confidence Threshold**: Cross-correlation confidence peaks below `0.1` indicate that the underlying audio tracks do not match the reference (likely a different mix, pitch, or severe edits). Outputs generated from these low-confidence matches will be out of sync and require manual validation or a different reference track. High confidence matches (e.g., `> 0.3`) indicate a reliable alignment.
* **Offset Sign Convention**: Cross-correlation offset = `search_start + (best_idx / sr) - anchor`. A **negative offset** means the AD track is AHEAD of the reference (plays content earlier); a **positive offset** means the AD track is BEHIND.
  * `offset < 0` (AD ahead) → **Prepend silence** via `adelay` filter (`adelay=Nms|Nms`).
  * `offset > 0` (AD behind) → **Trim start** via `-ss` placed AFTER `-i` for sample-exact accuracy.
  * **NEVER** use `-ss` (trim) when offset is negative — this doubles the misalignment instead of correcting it.
* **Post-Render Verification Requirement**: After rendering a synced AD track, ALWAYS cross-correlate the rendered output against the reference audio at 2-3 timeline checkpoints (e.g. 30s, 90s, 150s). All checkpoints must show lag < 0.5s before delivering the file to the user. `sync_ad_track.py` does this automatically via `verify_alignment()`.
* **MP3-to-WAV Intermediary Decoding**: Before applying any offset to an MP3 input, ALWAYS decode to lossless 24-bit PCM WAV first (`-c:a pcm_s24le`). FFmpeg's `-ss` seeking on raw MP3 files without Xing/LAME seek tables is imprecise (can be off by seconds). WAV enables sample-exact seeking. The decoded WAV also eliminates MP3 encoder delay artifacts (`start: 0.023021` reported by ffprobe).

### Standalone 2-Pass Loudness Normalization (`convert_ad_2pass.py`)
* **Purpose**: Converts standard 5.1/7.1 AD tracks (AC3, DTS, TrueHD) to 2-pass loudness normalized AAC (Standard and NightMode presets) without needing cross-correlation alignment.
* **Batch Processing**: Designed to be run sequentially to conserve threads. It automatically parses the Pass-1 JSON output and applies linear gain on Pass-2.
* **Command**: `python convert_ad_2pass.py "F:\uploads\Movie\Movie_AD.ac3"`

### Seamless Branching AD Track Reconstruction (`process_extended_ad_option_a.py` & `build_smart_ad_pipeline.py`)
* **Blu-ray MPLS PTS Timestamps**: Parses 45 kHz PTS values from `.mpls` files (`(OUT_time - IN_time) / 45000.0`) to map theatrical vs extended PlayItems.
* **Option A Seamless Replacement**: Preserves 100% of shared theatrical AD scenes and inserts matching Extended main audio (`7.1 -> 5.1`) for re-edited extended scenes with 50ms equal-power crossfades (`afade`).
* **Speech Tail Preservation (VAD)**: Uses 50ms RMS window analysis on mono center-channel audio (`pan=mono|c0=c2`, `speech_threshold=450`) in `build_smart_ad_pipeline.py` to detect narrator speech extending past PlayItem boundaries, dynamically extending AD slices so voice narration is never truncated mid-sentence.

### Hybrid DTW & Anchor Stitching Workflow (Complex Commercial Cuts)
* **DescribeAlign DTW**: For heavily edited TV rips (e.g. missing commercial breaks), raw cross-correlation fails. Run `describealign` (via `python describealign.py <video> <audio> --stretch_audio --yes`) to generate a dynamic time-warp alignment math plot (`alignment_plots/*.txt`). This maps exactly where all edits and missing scenes occur.
* **Micro-Drift Correction**: Because `describealign` penalizes micro-stretching and uses low-resolution interpolation, the audio may drift out of sync towards the end if the AD framerate varies from the master (e.g. 23.976 vs 24.000).
* **Sample-Exact Reassembly**: To achieve perfect sync, use `sync_ad_track.py --preview 30` to test anchors and find the exact sub-millisecond offsets for each segment between the commercial breaks detected by `describealign`. Then use a custom Python script (with `numpy` and `ffmpeg` `f32le` decoding) to surgically slice the AD track at the commercial breaks, inject exact silence gaps (using `np.zeros`), and concatenate the segments back together.

### Legacy / Prototype Scripts

---

## 3. FFmpeg Bitstream Trimming & Intermediate Audio Rules

* **Raw Bitstream Seeking Danger (`.ac3` / `.thd`)**: Placing `-ss` before `-i` on raw bitstream inputs without a container index causes frame boundary misalignment (`invalid block size: 3` error).
* **MP3 Seeking Danger (`.mp3`)**: Placing `-ss` before `-i` on raw MP3 files without a Xing/LAME seek table causes FFmpeg to seek to the wrong position, sometimes off by **several seconds**. This silently produces out-of-sync audio with no error message. **ALWAYS decode MP3 to WAV first**, then seek on the WAV.
* **Golden Rule for Sample-Exact Trimming**: Always decode raw bitstreams and compressed audio (MP3, AAC) to uncompressed 24-bit PCM WAV / FLAC first, OR place `-ss` AFTER `-i` when trimming uncompressed PCM sources.
* **Intermediate Splicing Format**: When creating intermediate audio chunks for concatenating, ALWAYS use 24-bit PCM WAV (`-c:a pcm_s24le`). WAV eliminates block size constraints, runs at >100x realtime speed, and prevents compression artifacts.
* **Explicit Channel & Format Matching**: When concatenating intermediate WAV chunks, explicitly force matching channel layouts and sample formats (`aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=5.1`) to prevent concat demuxer format mismatch errors. Downmix 7.1 to 5.1 via standard matrix pan (`pan=5.1|c0=c0|c1=c1|c2=c2|c3=c3|c4=c4+0.7071*c6|c5=c5+0.7071*c7`).
* **Loudnorm Sample Rate Bug (96kHz AAC)**: The `loudnorm` filter (EBU R128) operates internally at 192 kHz. If you encode to AAC immediately after `loudnorm` without explicitly specifying an output sample rate, FFmpeg will attempt to encode at 192 kHz, and the AAC encoder will automatically cap out and output at **96 kHz**. ALWAYS append `-ar 48000` AFTER the AAC codec declaration (`-c:a aac -b:a 256k -ar 48000`) when using `loudnorm` to enforce standard 48 kHz output.

---

## 4. MKV Chapter Extraction & Injection

### Command-Line Extraction & Injection (`MKVToolNix`)
* **Extract XML Chapters**:
  ```powershell
  & "C:\Program Files\MKVToolNix\mkvextract.exe" "movie.mkv" chapters "chapters.xml"
  ```
* **Extract Simple Text Chapters**:
  ```powershell
  & "C:\Program Files\MKVToolNix\mkvextract.exe" "movie.mkv" chapters --simple "chapters.txt"
  ```
* **Instant In-Place Injection without Re-muxing (`mkvpropedit`)**:
  ```powershell
  & "C:\Program Files\MKVToolNix\mkvpropedit.exe" "movie.mkv" --chapters "chapters.xml"
  ```

### MKVcleaver Configuration Quirk
* **Disabled Chapters Checkbox Fix**: In MKVcleaver, if `chapters_s=0` in `custom.ini`, the Chapters checkbox remains uncheckable. Set `chapters_s=1` (Simple `.txt`) or `chapters_s=2` (`.xml`) in `custom.ini` or GUI options to enable the checkbox.

---

## 5. Directory Locations

* **Source AD Directory**: `F:\uploads\ad\`
* **Reference Audio / MKV Directory**: `F:\uploads\`
* **Synced Output Directory**: `F:\uploads\ad_synced\`
* **Project Scripts**: `C:\Path\To\AudioDescriptionStuff\scripts\`
* **Portable FFmpeg Binaries**: `C:\Path\To\AudioDescriptionStuff\bin\` (Place `ffmpeg.exe` and `ffprobe.exe` here. Python scripts will automatically resolve this path dynamically.)

---

## 6. Project Scripts Reference
The master documentation for all custom Python algorithms and PowerShell pipeline orchestrators has been migrated to dedicated index files. 

Please refer to `scripts/README.md` for comprehensive documentation on all script behaviors and edge-case capabilities (e.g. VAD speech rescue, Safe Buffering, Frame-Perfect rendering).
