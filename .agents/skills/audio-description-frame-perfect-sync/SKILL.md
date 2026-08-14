---
name: audio-description-frame-perfect-sync
description: Audio description syncing, encoding standards, and commercial removal workflows.
---

# Audio Description Syncing & Processing

## Standard Encoding Specs

| Format | Codec | Bitrate | Notes |
|--------|-------|---------|-------|
| Stereo AAC | `aac` | `256k` | Always `-ar 48000`. Use `loudnorm=I=-16:TP=-1.5:LRA=11` for normalized variant. |
| 5.1 AC3 | `ac3` | `384k` | |
| Stereo FLAC | `flac` | lossless | For native stereo tracks only (no downmix). |
| 5.1→Stereo Downmix | `pan=stereo\|c0=c0+0.7071*c2+0.7071*c4\|c1=c1+0.7071*c2+0.7071*c5` | | Apply before `loudnorm`. |

All output files go in an `output/` subdirectory within the project folder.

**Standard stereo mix convention**: When processing surround main audio alongside AD, always include a normalized stereo AAC downmix of the main track unless the user says otherwise.

### Manual FFmpeg Encode Reference
```powershell
# Stereo AAC (normalized) from surround source:
ffmpeg -y -ss <offset> -i "source.dts" -af "aresample=48000,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ar 48000 -c:a aac -b:a 256k "output.m4a"

# 5.1 AC3 from surround source:
ffmpeg -y -ss <offset> -i "source.dts" -c:a ac3 -b:a 384k "output.ac3"

# Stereo AAC (NOT normalized, e.g. AD from external source):
ffmpeg -y -ss <offset> -i "source.mp3" -c:a aac -b:a 256k "output.m4a"
```

## Critical Rules

* **loudnorm 96kHz bug**: `loudnorm` operates internally at 192kHz and will upsample output to 96kHz. ALWAYS append `-ar 48000` after the filter chain.
* **DTS-HD MA + loudnorm stall**: Feeding lossless DTS-HD MA directly into `loudnorm` causes severe FFmpeg buffer stalls (~0.6x speed). Extract to intermediate WAV/W64 first, then apply `loudnorm` on the intermediate. Use `-f w64` (Wave64) for 5.1+ tracks that exceed the 4GB WAV limit.
* **MP3 seeking**: Never use `-ss` before `-i` on MP3 files. Decode to WAV first (`-c:a pcm_s24le`), then seek on WAV for sample-exact accuracy.
* **Raw bitstream seeking**: Placing `-ss` before `-i` on raw `.ac3`/`.thd` bitstreams causes frame boundary misalignment. Place `-ss` after `-i`.
* **Offset sign convention**:
  * `offset > 0` (AD behind) → trim start with `-ss <offset>` placed AFTER `-i`.
  * `offset < 0` (AD ahead) → prepend silence with `adelay=<ms>|<ms>`. Never use `-ss` for negative offsets.
* **Track ordering**: Main audio = `a:0` (default), AD = `a:1` (visual_impaired disposition).

---

## Workflow 1: Simple Sync (`sync_ad_track.py`)

**When to use**: AD and reference are from the same or very similar release. A single global offset (and optional speed stretch) aligns them across the entire timeline.

```powershell
# Preview (fast test-listen):
python sync_ad_track.py -r "reference.eac3" -a "ad_track.mp3" --preview 180

# Full render:
python sync_ad_track.py -r "reference.eac3" -a "ad_track.mp3" -o "output/" --force-stretch
```

**How it works**: Computes 10-anchor acoustic cross-correlation at 8kHz mono. Detects sub-millisecond offset and optional PAL/NTSC speed stretch. Renders 2-pass EBU R128 loudnorm AAC.

**Confidence check**: Peaks below `0.1` mean the tracks don't match (different mix, edits, or source). Peaks above `0.3` are reliable.

For tracks from the **same bluray/source** as a previously synced track, reuse the calculated offset — no need to re-correlate.

---

## Workflow 2: DescribeAlign DTW (Complex Edits)

**When to use**: AD and reference have **different edits** — commercial breaks removed, scenes cut/added, or network vs bluray cuts. Cross-correlation fails because no single offset works across the timeline.

**Location**: `describealign.py`

### Command
```powershell
python "describealign.py" --yes --stretch_audio <master_video_or_audio> <ad_track>
```

* `--stretch_audio`: Stretches/compresses the AD to match the master timeline. Keeps original video audio tracks; adds AD as first audio track.
* `--yes`: **Required for automation.** Skips interactive confirmation prompt that stalls unattended scripts.

### Output
* `videos_with_ad/ad_<filename>` — MKV/audio with aligned AD track.
* `alignment_plots/<filename>.txt` — Piecewise alignment map showing offsets, rate changes, and timestamp mappings per segment.
* `alignment_plots/<filename>.png` — Visual alignment curve plot.

### Track Order Gotcha
DescribeAlign outputs AD as `a:0` and Main as `a:1` — the **reverse** of our standard. Always remux to swap:
```powershell
ffmpeg -y -i "describealign_output.mkv" -map 0:v:0 -map 0:a:1 -map 0:a:0 -c copy ^
  -metadata:s:a:0 title="Main, Stereo" -disposition:a:0 default ^
  -metadata:s:a:1 title="AD, Stereo" -disposition:a:1 visual_impaired ^
  "final_output.mkv"
```

### Limitations
* Audio segments stretched >10% introduce audible pitch distortion.
* Cuts between segments may slice through narrator speech mid-sentence. Use VAD center-channel overlay at cut boundaries if needed.
* Slower than simple cross-correlation (full DTW feature computation).
* Creates `videos_with_ad/` and `alignment_plots/` directories — run from `scratch/` or clean up after.

### Integration with batch_orchestrator.py
`batch_orchestrator.py` automatically selects the right strategy per episode:
1. **Direct Mux** — single consistent offset, no black screens → simple `-ss` trim + mux.
2. **Frame-Perfect** — black screens detected and offset count matches → FFmpeg `trim`/`concat` re-encode.
3. **DTW (describealign)** — no black screens but multiple drifting offsets → dynamic time warp fallback.

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
