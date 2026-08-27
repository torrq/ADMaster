---
name: mkv-media-management
description: Comprehensive guide and automation workflows for Matroska (MKV) metadata, track flagging (commentary, visual impaired), chapter extraction/injection, and stream multiplexing using MKVToolNix and FFmpeg.
---

# Matroska (MKV) Media Management

## Communication Guidelines
* Keep responses extremely concise.
* Eliminate conversational filler, personality, and excessive exclamations.
* Provide scripts, commands, and factual explanations only.

---

## 1. Fast In-Place Track Modification (`mkvpropedit`)

`mkvpropedit` modifies Matroska headers in-place in milliseconds without remuxing or rewriting the multi-gigabyte video payload.

### Track Selector Syntax Rules

| Selector Syntax | Target | Description | Example |
| :--- | :--- | :--- | :--- |
| `--edit track:=<UID>` | Track UID | **Primary Standard (Safest & Unambiguous)**: Targets the exact 64-bit unsigned integer `track_uid`. | `--edit track:=1426943748200331858` |
| `--edit track:a<N>` | Audio Stream Index | Targets the $N$-th audio track (1-based relative index). | `--edit track:a2` |
| `--edit track:v<N>` | Video Stream Index | Targets the $N$-th video track (1-based relative index). | `--edit track:v1` |
| `--edit track:s<N>` | Subtitle Stream Index | Targets the $N$-th subtitle track (1-based relative index). | `--edit track:s1` |
| `--edit track:@<NUM>` | Track Number | Targets the sequential track index in the MKV header. | `--edit track:@3` |

> [!CAUTION]
> **Track Selector Prefix Bug:** Never pass `--edit track:@<UID>` when targeting a Track UID. `@` designates the relative Track Number; `=` designates the unique Track UID. Passing `@` with a UID results in `The file is analyzed... Error: No track corresponding to the edit specifications was found.`

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

## 3. Automated Batch Flagging Tools

### 3.1 Batch Commentary Flagging (`scripts/set_commentary_flags.py`)
Inspects MKV headers via `mkvmerge -J` JSON inspection and automatically sets `flag-commentary=1` for tracks matching keyword criteria (`commentary`, `director`, `producer`, `filmmaker`, `writer`).

```powershell
# Flag all commentary tracks across an entire TV show or Movie directory:
python scripts/set_commentary_flags.py "X:\video\TV\Show_Title\Show.S01.720p.BluRay.DD5.1.x264-Release"
```

### 3.2 Plex Audio Description & Commentary Collections (`scripts/plex_ad_commentary_collections.py`)
Scans media libraries to build smart collections and reports on files containing `flag-commentary=1` or `flag-visual-impaired=1`.

---

## 4. Lossless Video Sample Multiplexing

To test audio sync with video without re-encoding the video stream:

```powershell
ffmpeg -y -nostdin -ss 0 -t <duration_sec> -i "reference.mkv" -i "synced_ad.m4a" `
  -map 0:v:0 -map 1:a:0 -c:v copy -c:a copy `
  -metadata:s:a:0 title="Audio Description (Synced)" -disposition:a:0 default `
  "sample_preview.mkv"
```
