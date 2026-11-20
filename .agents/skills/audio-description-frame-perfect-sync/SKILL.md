---
name: audio-description-frame-perfect-sync
description: Audio description syncing, encoding standards, PAL speed detection, and mastering workflows in ADMaster.
---

# Audio Description Syncing & Mastering Standards

## Communication Guidelines
* Keep responses concise and factual.
* Focus on verified commands, code, and exact diagnostics.

## Standard Encoding Specs

| Format | Codec | Bitrate | Notes |
|--------|-------|---------|-------|
| Stereo AAC | `aac` | `256k` | Always `-ar 48000`. Use `alimiter=level_in=1:level_out=0.98:limit=0.98` for standard Stereo. |
| Night Mode AAC | `aac` | `256k` | Always `-ar 48000`. Apply `loudnorm=I=-16:TP=-1.5:LRA=11` after stereo downmix. |
| 5.1 AC3 | `ac3` | `640k` | Preserves spatial clarity for surround sources (`channels >= 6`). |
| 5.1 -> Stereo Downmix | `pan=stereo\|c0=c0+0.7071*c2+0.7071*c4\|c1=c1+0.7071*c2+0.7071*c5` | | ITU-R BS.775 matrix; always pair with explicit `-ac 2 -ar 48000`. |

All output files go in an `output/` subdirectory within the project folder (or `--outdir`).
All temporary analysis WAVs and intermediate scratch files go inside `scratch/` (cleaned automatically unless `--keep-scratch` is set).
Session logs default to the source MKV directory (customizable via `--log-dir` / `--log-pattern` or GUI **Settings -> Appearance & Logs**).

**Standard deliverables**:
* **AD Tracks**: Generates both `<Title>-AD (Stereo).m4a` (`alimiter` clipping protection) and `<Title>-AD (Night Mode).m4a` (EBU R128 `loudnorm` -16 LUFS) with `-ac 2 -ar 48000`. If an AD track is natively 5.1/7.1 surround (`>= 6` channels), also preserve a 5.1 AC3 (640k) variant.
* **Main Audio (`--convert-main`)**: Converts surround masters to `(Surround).ac3` (5.1 640k), `(Stereo).m4a` (ITU-R BS.775 2.0 downmix + `alimiter`), and `(Night Mode).m4a` (ITU-R BS.775 2.0 downmix + `loudnorm`). Native stereo masters bypass the surround pan matrix.

### Manual FFmpeg Encode Reference
```powershell
# Stereo AAC (standard) from surround source:
ffmpeg -y -nostdin -i "source.mkv" -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,alimiter=level_in=1:level_out=0.98:limit=0.98" -ac 2 -ar 48000 -c:a aac -b:a 256k "output_stereo.m4a"

# Night Mode AAC (normalized) from surround source:
ffmpeg -y -nostdin -i "source.mkv" -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ac 2 -ar 48000 -c:a aac -b:a 256k "output_night.m4a"

# 5.1 AC3 from surround source:
ffmpeg -y -nostdin -i "source.dts" -c:a ac3 -b:a 640k "output.ac3"
```

## Critical Rules

* **Interactive stdin bug**: Always pass `-nostdin` to automated FFmpeg invocations so terminal input never stalls an encode.
* **loudnorm 96 kHz upsampling bug**: `loudnorm` operates internally at 192 kHz and upsamples output to 96 kHz unless `-ar 48000` is appended after the filter chain.
* **Lossless surround + loudnorm stall**: Extract lossless multi-channel tracks to an intermediate 48 kHz WAV (`-rf64 auto`) before running `loudnorm`.
* **Raw bitstream seeking (DTS/AC3/EAC3 stall bug)**: Raw bitstreams have no container seek index. Never repeatedly seek into raw bitstreams with `-ss`. Pre-decode once to an 8 kHz mono analysis WAV and slice in memory.
* **Offset sign convention**:
  * `offset > 0` (AD starts earlier / has extra intro) -> trim start with `-ss <offset>` placed AFTER `-i`.
  * `offset < 0` (AD starts later) -> prepend silence with `adelay=<ms>|<ms>`. Never use `-ss` for negative offsets.
  * When passing correlation `winner_offset` as `offset_hint` to `warp.rs`, negate it (`-winner_offset`) because the Viterbi lattice uses `t_vid - t_ad`.
* **Track ordering**: Main audio = `a:0` (default), AD = `a:1` (`visual_impaired` disposition / `flag-visual-impaired=1`).

---

## Workflow 1: Standard Sync & Encoding (`admaster-cli` / `admaster`)

```powershell
# 1. Directory auto-discovery (source/ + tosync/) with Main Audio conversion:
admaster-cli "F:\Movies\ExampleMovie" --convert-main

# 2. Specific reference audio stream with forced Time-Warp alignment:
admaster-cli "F:\Movies\ExampleMovie" --ref-stream 0:a:1 --force-warp --no-pal --convert-main

# 3. Fast uncompressed 48 kHz WAV test pass (skip Night Mode):
admaster-cli "F:\Movies\ExampleMovie" --wav --no-night

# 4. Fast 180-second test preview multiplexed with video:
admaster-cli "F:\Movies\ExampleMovie" --preview 180 -m

# 5. Batch process an entire queue folder of movie directories:
admaster-cli "F:\Movies\Queue" --batch --convert-main
```

---

## Workflow 2: Non-Linear Alignment & Alternate Cuts (Time-Warp)

When the AD track and reference movie have **different edits** (commercial breaks removed, excised or added scenes, alternate theatrical vs. extended cuts, or drifting frame rates), single-offset cross-correlation cannot align the entire timeline.

ADMaster's native Rust **Time-Warp** engine (`src/dsp/warp.rs`) activates automatically when global anchor consensus confidence falls below `--min-confidence` (default `0.15`), or immediately when `--force-warp` is passed:
* Builds a 4 kHz 3-band candidate lattice using packed 5-smooth `f32` SIMD FFTs.
* Solves globally optimal alignment via second-order Viterbi dynamic programming with micro-blip spur filtering.
* Nudges structural cut boundaries into narrator silence windows and fits monotone Fritsch-Carlson PCHIP cubic splines.
* Renders in 30-second streaming Catmull-Rom chunks (`< 35 MB` RAM) and fills deleted scenes (`>= 0.25 s`) using level-matched reference audio with 50 ms cosine crossfades (`src/dsp/gap_fill.rs`).

---

## Pitfalls & Troubleshooting Reference

### 1. PAL Speedup vs. Native 1.0x Alternate Cuts / UK Releases
* **Pitfall**: Assuming a `[UK]` filename tag or a shorter total runtime implies a 25.0 fps PAL speedup corrupts native 1.0x tracks by slowing them down 4.27%.
* **Rule**: In `AutoDetect` mode, always decode at native `1.0x` and let the STFT acoustic landmark prober (`detect_speed_pal`) verify whether a true `1.042709x` (`25.0 / 23.976024`) speed transfer exists. Use `--pal` or `--no-pal` only to explicitly override auto-detection.

### 2. Dynamic Sample-Rate Handling for `asetrate` PAL Restoration
* Sample-accurate PAL -> Film conversion adjusts the sample clock rather than using WSOLA `atempo`:
  `asetrate = input_sr * (23.976023976 / 25.0)`, followed by `aresample=48000`.
* Always probe the input sample rate first:
  * `48,000 Hz` input -> `asetrate=46033.966`
  * `44,100 Hz` input -> `asetrate=42293.706`
* Never hardcode `46033.966 Hz` on a `44.1 kHz` file (that speeds it up by +4.3% instead of slowing it down).

### 3. Strict 2.0 Stereo Downmixing for Surround Sources
* Passing `-af "alimiter=..."` or `-af "loudnorm=..."` on a 5.1/7.1 source without the ITU-R BS.775 pan matrix and `-ac 2` retains all 6 channels. Always include both the pan matrix and `-ac 2 -ar 48000`.

### 4. Multi-Track Reference Stream Selection
* Classic films frequently include multiple audio mixes (e.g. original 1.0/2.0 optical audio alongside a modern 5.1/7.1 remix). If alignment confidence is low against stream `0:a:0`, inspect streams in the GUI **Match Against** dropdown or test alternative streams via `--ref-stream 0:a:1`.

### 5. MKVPropEdit Track Selector Syntax (`=` for UID vs `@` for Track Number)
* `--edit track:=<uid>` targets a 64-bit Matroska Track UID (`=`).
* `--edit track:@<number>` targets a 1-based Track Number (`@`).
* `--edit track:a<n>` targets the N-th audio track (e.g. `track:a2`).
