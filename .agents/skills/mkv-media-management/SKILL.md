---
name: mkv-media-management
description: Comprehensive guide and automation workflows for Matroska (MKV) metadata, track flagging (commentary, visual impaired), chapter extraction/injection, and stream multiplexing using MKVToolNix and FFmpeg.
---

# Matroska (MKV) Media Management

## Communication Guidelines
* Keep responses concise and factual.
* Provide commands and clear technical explanations only.

---

## 1. Fast In-Place Track Modification (`mkvpropedit`)

`mkvpropedit` modifies Matroska headers in-place in milliseconds without remuxing or rewriting the multi-gigabyte video payload.

### Track Selector Syntax Rules

| Selector Syntax | Target | Description | Example |
| :--- | :--- | :--- | :--- |
| `--edit track:=<UID>` | Track UID | **Primary Standard (Safest & Unambiguous)**: Targets the exact 64-bit unsigned integer `track_uid`. | `--edit track:=1426943748200331858` |
| `--edit track:a<N>` | Audio Stream Index | Targets the N-th audio track (1-based relative index). | `--edit track:a2` |
| `--edit track:v<N>` | Video Stream Index | Targets the N-th video track (1-based relative index). | `--edit track:v1` |
| `--edit track:s<N>` | Subtitle Stream Index | Targets the N-th subtitle track (1-based relative index). | `--edit track:s1` |
| `--edit track:@<NUM>` | Track Number | Targets the sequential track index in the MKV header. | `--edit track:@3` |

> [!CAUTION]
> **Track Selector Prefix Bug:** Never pass `--edit track:@<UID>` when targeting a Track UID. `@` designates the relative Track Number; `=` designates the unique Track UID. Passing `@` with a UID fails with `Error: No track corresponding to the edit specifications was found.`

### Common Track Flags & Metadata

```powershell
# Set Commentary Flag:
mkvpropedit "Movie.mkv" --edit track:=<UID> --set flag-commentary=1

# Set Audio Description / Visual Impaired Flag:
mkvpropedit "Movie.mkv" --edit track:=<UID> --set flag-visual-impaired=1

# Set Default and Forced Flags:
mkvpropedit "Movie.mkv" --edit track:=<UID> --set flag-default=0 --set flag-forced=0

# Set Track Title and Language Tag:
mkvpropedit "Movie.mkv" --edit track:=<UID> --set name="Director's Commentary" --set language=eng
```

---

## 2. Chapter Extraction, Editing & Injection

### 2.1 Extraction
```powershell
# Extract chapters in XML format (preserves UIDs and language attributes):
mkvextract "Movie.mkv" chapters "chapters.xml"

# Extract chapters in simple OGM text format:
mkvextract "Movie.mkv" chapters --simple "chapters.txt"
```

### 2.2 In-Place Chapter Injection
```powershell
# Replace / inject chapters directly into MKV without remuxing:
mkvpropedit "Movie.mkv" --chapters "chapters.xml"
```

---

## 3. Automated Multiplexing in ADMaster

`admaster` and `admaster-cli --mux` automatically invoke `mkvmerge` to combine the reference video, converted stereo main audio, synced Audio Description tracks (with `flag-visual-impaired` set), and English subtitles in a single pass:

```powershell
admaster-cli --ref "Movie.mkv" --ad "Track_AD.m4a" --convert-main --mux --mkv-dir "OutputDir"
```

---

## 4. Lossless Video Sample Multiplexing

To test audio sync with video without re-encoding the video stream:

```powershell
ffmpeg -y -nostdin -ss 0 -t <duration_sec> -i "reference.mkv" -i "synced_ad.m4a" `
  -map 0:v:0 -map 1:a:0 -c:v copy -c:a copy `
  -metadata:s:a:0 title="Audio Description (Synced)" -disposition:a:0 default `
  "sample_preview.mkv"
```
