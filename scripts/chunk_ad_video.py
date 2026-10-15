#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
r"""
================================================================================
Automated Video Chunker for Long-Form Descriptive Audio (AD) Pipeline
================================================================================
Repository: ADMaster
License: MIT

Overview:
  Inputs movie files (MKV, MP4, etc.) and partitions them into lightweight,
  multimodal Gemini-optimized MP4 sections (max 45 minutes each, 1 FPS,
  CRF 28, mono AAC 64k). Automatically analyzes dialogue pauses (silence),
  visual scene breaks, and Matroska chapters to place cut boundaries without
  breaking spoken dialogue. Emits a structured JSON manifest detailing sample-
  accurate offsets for downstream Gemini inference, TTS synthesis, and audio mixing.

Usage Examples:
  1. Standard 45m max chunking of a movie MKV:
     python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv"

  2. Dry run (analyze cut points and manifest without encoding video):
     python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --dry-run

  3. Custom target chunk size (e.g. max 35m, min 25m):
     python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --max-duration 35 --min-duration 25

  4. 480p resolution profile with 96k audio:
     python scripts/chunk_ad_video.py "F:\uploads\Movie\Movie.mkv" --resolution 480p --audio-bitrate 96k
"""

import argparse
from datetime import datetime
import json
import math
import os
import re
import shutil
import subprocess
import sys


class Color:
    """ANSI color and terminal formatting palette (Pure ASCII)."""
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"

    RED = "\033[91m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    BLUE = "\033[94m"
    MAGENTA = "\033[95m"
    CYAN = "\033[96m"
    WHITE = "\033[97m"
    GRAY = "\033[90m"

    @classmethod
    def init(cls):
        """Enable virtual terminal processing on Windows console."""
        if os.name == "nt":
            try:
                import ctypes
                kernel32 = ctypes.windll.kernel32
                h = kernel32.GetStdHandle(-11)
                m = ctypes.c_ulong()
                if kernel32.GetConsoleMode(h, ctypes.byref(m)):
                    kernel32.SetConsoleMode(h, m.value | 0x0004)
            except Exception:
                pass

    @classmethod
    def ok(cls, text="[OK]"):
        return f"{cls.GREEN}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def info(cls, text="[INFO]"):
        return f"{cls.CYAN}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def warn(cls, text="[WARNING]"):
        return f"{cls.YELLOW}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def err(cls, text="[ERROR]"):
        return f"{cls.RED}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def section(cls, text):
        return f"{cls.WHITE}{cls.BOLD}{text}{cls.RESET}"


Color.init()


def load_config():
    """Loads configuration from config.local.json, config.json, or config.default.json."""
    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(base_dir, "config.local.json"),
        os.path.join(base_dir, "config.json"),
        os.path.join(base_dir, "config.default.json")
    ]
    for c in candidates:
        if os.path.exists(c):
            try:
                with open(c, "r", encoding="utf-8") as f:
                    content = f.read()
                    content_clean = re.sub(r'//.*', '', content)
                    content_clean = re.sub(r'/\*.*?\*/', '', content_clean, flags=re.DOTALL)
                    return json.loads(content_clean)
            except Exception:
                pass
    return {}


CONFIG = load_config()


def locate_tool(name):
    """Resolves absolute path to external executables."""
    custom = CONFIG.get(f"{name}_path")
    if custom and os.path.exists(custom):
        return os.path.abspath(custom)
    exe = f"{name}.exe" if os.name == "nt" else name
    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(base_dir, "bin", exe),
        os.path.join(r"C:\Program Files\MKVToolNix", exe),
        os.path.join(r"C:\Program Files (x86)\MKVToolNix", exe),
    ]
    for c in candidates:
        if os.path.exists(c):
            return os.path.abspath(c)
    return shutil.which(name) or shutil.which(exe) or exe


FFMPEG_BIN = locate_tool("ffmpeg")
FFPROBE_BIN = locate_tool("ffprobe")
MKVMERGE_BIN = locate_tool("mkvmerge")


def format_timecode(seconds: float) -> str:
    """Format duration in seconds to standard HH:MM:SS.mmm timecode string."""
    seconds = max(0.0, float(seconds))
    m, s = divmod(seconds, 60.0)
    h, m = divmod(int(m), 60)
    return f"{h:02d}:{m:02d}:{s:06.3f}"


def format_timecode_filename(seconds: float) -> str:
    """Format duration in seconds to clean filename time string (e.g. 01h24m35s)."""
    seconds = max(0.0, float(seconds))
    m, s = divmod(seconds, 60.0)
    h, m = divmod(int(m), 60)
    return f"{h:02d}h{m:02d}m{int(s):02d}s"


def resolve_input_video(raw_input: str):
    """
    Resolves a raw input string (file path or directory) to a concrete video file.
    Handles:
      - Direct video files (.mkv, .mp4, .m4v, .avi, .ts, .mov, etc.)
      - Directory paths (e.g. F:\\uploads\\TMNT\\ or F:\\uploads\\TMNT)
      - Standard source/ subdirectories (e.g. F:\\uploads\\TMNT\\source\\Movie.mkv)
      - Windows trailing quote quirks (e.g. 'path\\"' parsed as 'path"')
    Returns:
      (video_file_path, default_output_dir)
    """
    cleaned = raw_input.strip()
    cleaned = cleaned.strip('"\'')
    if cleaned.endswith('"') or cleaned.endswith("'"):
        cleaned = cleaned[:-1].strip()
    cleaned = os.path.normpath(cleaned)

    # 1. If path is directly a file
    if os.path.isfile(cleaned):
        parent_dir = os.path.dirname(cleaned)
        base_name = os.path.splitext(os.path.basename(cleaned))[0]
        if os.path.basename(parent_dir).lower() == "source":
            movie_root = os.path.dirname(parent_dir)
            default_out = os.path.join(movie_root, "ad_chunks")
        else:
            default_out = os.path.join(parent_dir, f"{base_name}_ad_chunks")
        return cleaned, default_out

    # 2. If path is a directory, auto-discover source video
    if os.path.isdir(cleaned):
        video_exts = {".mkv", ".mp4", ".m4v", ".avi", ".ts", ".mov", ".flv", ".webm"}

        # Check source/ folder first
        source_dir = os.path.join(cleaned, "source")
        candidate_files = []
        if os.path.isdir(source_dir):
            for f in os.listdir(source_dir):
                ext = os.path.splitext(f)[1].lower()
                if ext in video_exts:
                    full_p = os.path.join(source_dir, f)
                    if os.path.isfile(full_p):
                        candidate_files.append(full_p)

        # If not in source/, check directly in directory
        if not candidate_files:
            for f in os.listdir(cleaned):
                ext = os.path.splitext(f)[1].lower()
                if ext in video_exts:
                    full_p = os.path.join(cleaned, f)
                    if os.path.isfile(full_p):
                        candidate_files.append(full_p)

        # If still not found, search one level deeper
        if not candidate_files:
            for root, dirs, files in os.walk(cleaned):
                dirs[:] = [d for d in dirs if d.lower() not in ["output", "scratch", "ad_chunks", "bin", ".git"]]
                for f in files:
                    ext = os.path.splitext(f)[1].lower()
                    if ext in video_exts:
                        candidate_files.append(os.path.join(root, f))

        if not candidate_files:
            raise FileNotFoundError(f"No video files found inside directory: {cleaned}")

        # Filter out sample/preview clips if multiple files exist
        non_samples = [f for f in candidate_files if "sample" not in os.path.basename(f).lower()]
        filtered = non_samples if non_samples else candidate_files

        # Sort by file size descending (main movie feature is largest file)
        filtered.sort(key=lambda x: os.path.getsize(x), reverse=True)
        chosen_video = filtered[0]

        default_out = os.path.join(cleaned, "ad_chunks")
        return chosen_video, default_out

    raise FileNotFoundError(f"Input path not found: {cleaned}")


def probe_media_info(video_path: str):
    """
    Probes video duration, streams, and chapters using ffprobe.
    Returns:
      {
        'duration': float,
        'chapters': [{'title': str, 'start': float, 'end': float}, ...],
        'video_streams': [{'index': int, 'codec': str, 'width': int, 'height': int}, ...],
        'audio_streams': [{'index': int, 'codec': str, 'channels': int, 'lang': str, 'title': str}, ...]
      }
    """
    if not os.path.exists(video_path):
        raise FileNotFoundError(f"Input video file not found: {video_path}")

    cmd = [
        FFPROBE_BIN,
        "-v", "error",
        "-show_entries", "format=duration:stream=index,codec_type,codec_name,channels,width,height,tags:chapter=id,start_time,end_time,tags",
        "-of", "json",
        video_path
    ]

    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=True)
    data = json.loads(res.stdout)

    # Duration parsing
    duration = 0.0
    try:
        duration = float(data.get("format", {}).get("duration", 0.0))
    except (ValueError, TypeError):
        pass

    video_streams = []
    audio_streams = []

    for s in data.get("streams", []):
        ctype = s.get("codec_type")
        cname = s.get("codec_name", "")
        idx = int(s.get("index", 0))
        tags = s.get("tags") or {}
        lang = tags.get("language", "").lower()
        title = tags.get("title", "")

        if ctype == "video":
            w = int(s.get("width") or 0)
            h = int(s.get("height") or 0)
            video_streams.append({"index": idx, "codec": cname, "width": w, "height": h})
        elif ctype == "audio":
            ch = int(s.get("channels") or 2)
            audio_streams.append({"index": idx, "codec": cname, "channels": ch, "lang": lang, "title": title})

    chapters = []
    for c in data.get("chapters", []):
        try:
            c_start = float(c.get("start_time", 0.0))
            c_end = float(c.get("end_time", 0.0))
            c_title = c.get("tags", {}).get("title", f"Chapter {len(chapters) + 1}")
            chapters.append({"title": c_title, "start": c_start, "end": c_end})
        except (ValueError, TypeError):
            continue

    chapters.sort(key=lambda x: x["start"])

    return {
        "duration": duration,
        "chapters": chapters,
        "video_streams": video_streams,
        "audio_streams": audio_streams
    }


def select_best_audio_stream(audio_streams):
    """Selects the best English / default audio stream if not specified."""
    if not audio_streams:
        return "0:a:0"

    # 1. Look for English audio track
    for s in audio_streams:
        if s.get("lang") in ["eng", "en"]:
            return f"0:{s['index']}"

    # 2. Default to first audio stream
    return f"0:{audio_streams[0]['index']}"


def scan_candidate_window(video_path: str, w_start: float, w_end: float,
                           audio_stream: str = "0:a:0",
                           silence_noise: str = "-30dB",
                           silence_min_dur: float = 0.5,
                           scene_threshold: float = 0.25,
                           detect_scenes: bool = True):
    """
    Scans a candidate time window for audio silence and visual scene cuts.
    Returns:
      silence_intervals: [(start_sec, end_sec, dur_sec), ...]
      scene_cuts: [(pts_sec, scene_score), ...]
    All returned timestamps are absolute seconds on the source movie timeline.
    """
    duration = max(0.1, w_end - w_start)

    cmd = [
        FFMPEG_BIN,
        "-y", "-nostdin",
        "-ss", f"{w_start:.3f}",
        "-t", f"{duration:.3f}",
        "-i", video_path,
        "-map", audio_stream,
        "-af", f"silencedetect=noise={silence_noise}:d={silence_min_dur}",
    ]

    if detect_scenes:
        cmd.extend([
            "-map", "0:v:0",
            "-vf", f"scale=320:-2,select='gt(scene,{scene_threshold})',metadata=print:key=lavfi.scene_score",
        ])

    cmd.extend(["-f", "null", "-"])

    proc = subprocess.Popen(
        cmd,
        stderr=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace"
    )

    silence_intervals = []
    scene_cuts = []

    current_silence_start = None
    last_pts_time = None

    for line in proc.stderr:
        line_str = line.strip()

        # Parse silencedetect
        if "silence_start:" in line_str:
            m = re.search(r"silence_start:\s*([0-9\.]+)", line_str)
            if m:
                rel_start = float(m.group(1))
                current_silence_start = w_start + rel_start
        elif "silence_end:" in line_str:
            m = re.search(r"silence_end:\s*([0-9\.]+)\s*\|\s*silence_duration:\s*([0-9\.]+)", line_str)
            if m:
                rel_end = float(m.group(1))
                dur = float(m.group(2))
                abs_end = w_start + rel_end
                abs_start = current_silence_start if current_silence_start is not None else (abs_end - dur)
                silence_intervals.append((abs_start, abs_end, dur))
                current_silence_start = None

        # Parse scene detection metadata
        if "pts_time:" in line_str:
            m = re.search(r"pts_time:\s*([0-9\.]+)", line_str)
            if m:
                last_pts_time = float(m.group(1))
        elif "lavfi.scene_score=" in line_str:
            m = re.search(r"lavfi\.scene_score=\s*([0-9\.]+)", line_str)
            if m and last_pts_time is not None:
                score = float(m.group(1))
                abs_pts = w_start + last_pts_time
                scene_cuts.append((abs_pts, score))
                last_pts_time = None

    proc.wait()
    return silence_intervals, scene_cuts


def find_optimal_cut_point(video_path: str,
                           t_start: float,
                           t_target: float,
                           max_duration_sec: float,
                           min_duration_sec: float,
                           slack_sec: float,
                           chapters: list,
                           audio_stream: str = "0:a:0",
                           silence_noise: str = "-30dB",
                           silence_min_dur: float = 0.5,
                           scene_threshold: float = 0.25,
                           detect_scenes: bool = True,
                           use_chapters: bool = True):
    """
    Finds the optimal cut point within [W_start, W_end] that preserves dialogue
    and snaps to scene / chapter breaks.
    """
    hard_max = t_start + max_duration_sec
    hard_min = t_start + min_duration_sec

    # Candidate search window
    w_start = max(hard_min, t_target - slack_sec)
    w_end = min(hard_max, t_target + slack_sec)

    if w_end <= w_start:
        w_start = hard_min
        w_end = hard_max

    print(f"  {Color.info()} Scanning window: {format_timecode(w_start)} -> {format_timecode(w_end)} "
          f"(Target: {format_timecode(t_target)}, Hard Max: {format_timecode(hard_max)})")

    # Run FFmpeg silence and scene detection
    silence_intervals, scene_cuts = scan_candidate_window(
        video_path=video_path,
        w_start=w_start,
        w_end=w_end,
        audio_stream=audio_stream,
        silence_noise=silence_noise,
        silence_min_dur=silence_min_dur,
        scene_threshold=scene_threshold,
        detect_scenes=detect_scenes
    )

    # Filter chapters inside window
    window_chapters = []
    if use_chapters and chapters:
        for ch in chapters:
            if w_start <= ch["start"] <= w_end:
                window_chapters.append(ch)

    print(f"  {Color.info()} Detected {len(silence_intervals)} dialogue pause(s), "
          f"{len(scene_cuts)} visual scene cut(s), {len(window_chapters)} chapter marker(s).")

    candidates = []

    # 1. Evaluate silence intervals (Dialogue Protection)
    for s_start, s_end, s_dur in silence_intervals:
        # Check if a visual scene cut lands inside this silence
        matching_scene_cuts = [sc for sc in scene_cuts if (s_start - 0.15) <= sc[0] <= (s_end + 0.15)]
        # Check if a chapter lands inside or near this silence
        matching_chapters = [ch for ch in window_chapters if (s_start - 1.0) <= ch["start"] <= (s_end + 1.0)]

        if matching_scene_cuts:
            # Best possible cut: visual scene change happening during dialogue silence
            for sc_time, sc_score in matching_scene_cuts:
                cut_time = sc_time
                candidates.append({
                    "timestamp": cut_time,
                    "type": "scene+silence",
                    "silence_dur": s_dur,
                    "scene_score": sc_score,
                    "base_score": 120.0,
                    "reason": f"Visual scene cut at {format_timecode(cut_time)} (score: {sc_score:.2f}) inside {s_dur:.2f}s dialogue silence"
                })
        elif matching_chapters:
            # Chapter transition happening during dialogue silence
            for ch in matching_chapters:
                cut_time = (s_start + s_end) / 2.0
                candidates.append({
                    "timestamp": cut_time,
                    "type": "chapter+silence",
                    "silence_dur": s_dur,
                    "scene_score": 0.0,
                    "base_score": 110.0,
                    "reason": f"Chapter '{ch['title']}' at {format_timecode(ch['start'])} during {s_dur:.2f}s dialogue silence"
                })
        else:
            # Silence interval without detected scene change: cut in middle of pause
            cut_time = (s_start + s_end) / 2.0
            candidates.append({
                "timestamp": cut_time,
                "type": "silence_only",
                "silence_dur": s_dur,
                "scene_score": 0.0,
                "base_score": 75.0,
                "reason": f"Dialogue pause ({s_dur:.2f}s silence) centered at {format_timecode(cut_time)}"
            })

    # 2. Evaluate chapters without silence match (disc author boundary)
    for ch in window_chapters:
        cut_time = ch["start"]
        candidates.append({
            "timestamp": cut_time,
            "type": "chapter_only",
            "silence_dur": 0.0,
            "scene_score": 0.0,
            "base_score": 40.0,
            "reason": f"Chapter marker '{ch['title']}' at {format_timecode(cut_time)}"
        })

    # 3. Evaluate visual scene cuts without silence match (fallback)
    for sc_time, sc_score in scene_cuts:
        candidates.append({
            "timestamp": sc_time,
            "type": "scene_only",
            "silence_dur": 0.0,
            "scene_score": sc_score,
            "base_score": 30.0,
            "reason": f"Visual scene cut at {format_timecode(sc_time)} (score: {sc_score:.2f})"
        })

    # Filter candidates strictly within [hard_min, hard_max]
    valid_candidates = []
    for c in candidates:
        if hard_min <= c["timestamp"] <= hard_max:
            valid_candidates.append(c)

    # Score candidates
    best_candidate = None
    best_score = -1e9

    for c in valid_candidates:
        dist = abs(c["timestamp"] - t_target)
        # Distance penalty normalized to slack window
        dist_ratio = dist / max(1.0, slack_sec)
        penalty = 25.0 * (dist_ratio ** 2)

        # Bonus components
        silence_bonus = min(c["silence_dur"], 3.0) * 10.0
        scene_bonus = min(c["scene_score"], 1.0) * 20.0

        final_score = c["base_score"] + silence_bonus + scene_bonus - penalty
        c["final_score"] = round(final_score, 2)

        if final_score > best_score:
            best_score = final_score
            best_candidate = c

    if best_candidate:
        return best_candidate

    # Fallback if no candidate points found: pick t_target clamped to hard bounds
    fallback_time = max(hard_min, min(hard_max, t_target))
    return {
        "timestamp": fallback_time,
        "type": "fallback_target",
        "silence_dur": 0.0,
        "scene_score": 0.0,
        "base_score": 10.0,
        "final_score": 10.0,
        "reason": f"Fallback nominal target at {format_timecode(fallback_time)} (no pause markers detected)"
    }


def compute_chunk_plan(video_path: str,
                       total_duration: float,
                       chapters: list,
                       audio_stream: str,
                       max_duration_min: float = 45.0,
                       min_duration_min: float = 30.0,
                       slack_min: float = 5.0,
                       silence_noise: str = "-30dB",
                       silence_min_dur: float = 0.5,
                       scene_threshold: float = 0.25,
                       detect_scenes: bool = True,
                       use_chapters: bool = True):
    """
    Computes a sequence of optimal chunks covering the entire video timeline.
    Returns: list of dicts with chunk metadata.
    """
    max_duration_sec = max_duration_min * 60.0
    min_duration_sec = min_duration_min * 60.0
    slack_sec = slack_min * 60.0

    # If file duration is already within max_duration, output single chunk
    if total_duration <= max_duration_sec:
        print(f"  {Color.ok()} Total duration ({format_timecode(total_duration)}) <= max duration ({format_timecode(max_duration_sec)}). Single chunk plan.")
        return [{
            "index": 1,
            "start_time_seconds": 0.0,
            "start_time_timecode": "00:00:00.000",
            "end_time_seconds": total_duration,
            "end_time_timecode": format_timecode(total_duration),
            "duration_seconds": total_duration,
            "duration_timecode": format_timecode(total_duration),
            "cut_type": "full_length",
            "cut_reason": "Single chunk covers full video duration without splitting",
            "confidence_score": 100.0
        }]

    # Determine chunk count K
    k_chunks = math.ceil(total_duration / max_duration_sec)
    # Ensure min chunk constraint is respected
    if total_duration / k_chunks < min_duration_sec:
        k_chunks = max(1, math.floor(total_duration / min_duration_sec))

    print(f"\n{Color.section('=== Chunk Planning Strategy ===')}")
    print(f"  Source Duration: {format_timecode(total_duration)} ({total_duration:.2f}s)")
    print(f"  Target Chunks  : {k_chunks}")
    print(f"  Max Duration   : {max_duration_min:.1f}m ({max_duration_sec:.0f}s)")
    print(f"  Min Duration   : {min_duration_min:.1f}m ({min_duration_sec:.0f}s)")
    print(f"  Search Slack   : +/-{slack_min:.1f}m (+/-{slack_sec:.0f}s)\n")

    chunks = []
    current_start = 0.0

    for i in range(1, k_chunks + 1):
        remaining_chunks = k_chunks - i + 1

        if remaining_chunks == 1:
            # Final chunk reaches the end of the timeline
            chunk_dur = total_duration - current_start
            chunks.append({
                "index": i,
                "start_time_seconds": round(current_start, 3),
                "start_time_timecode": format_timecode(current_start),
                "end_time_seconds": round(total_duration, 3),
                "end_time_timecode": format_timecode(total_duration),
                "duration_seconds": round(chunk_dur, 3),
                "duration_timecode": format_timecode(chunk_dur),
                "cut_type": "timeline_end",
                "cut_reason": "Final section reaching end of movie timeline",
                "confidence_score": 100.0
            })
            break

        # Calculate nominal target for this cut
        remaining_duration = total_duration - current_start
        target_span = remaining_duration / remaining_chunks
        t_target = current_start + target_span

        print(f"{Color.section(f'--- Determining Boundary for Chunk {i} of {k_chunks} ---')}")
        cut_cand = find_optimal_cut_point(
            video_path=video_path,
            t_start=current_start,
            t_target=t_target,
            max_duration_sec=max_duration_sec,
            min_duration_sec=min_duration_sec,
            slack_sec=slack_sec,
            chapters=chapters,
            audio_stream=audio_stream,
            silence_noise=silence_noise,
            silence_min_dur=silence_min_dur,
            scene_threshold=scene_threshold,
            detect_scenes=detect_scenes,
            use_chapters=use_chapters
        )

        cut_time = cut_cand["timestamp"]
        chunk_dur = cut_time - current_start

        print(f"  {Color.ok()} Selected cut at {format_timecode(cut_time)} "
              f"(Score: {cut_cand.get('final_score', 0):.1f}) -> {cut_cand['reason']}\n")

        chunks.append({
            "index": i,
            "start_time_seconds": round(current_start, 3),
            "start_time_timecode": format_timecode(current_start),
            "end_time_seconds": round(cut_time, 3),
            "end_time_timecode": format_timecode(cut_time),
            "duration_seconds": round(chunk_dur, 3),
            "duration_timecode": format_timecode(chunk_dur),
            "cut_type": cut_cand.get("type", "unknown"),
            "cut_reason": cut_cand.get("reason", ""),
            "confidence_score": cut_cand.get("final_score", 0.0)
        })

        current_start = cut_time

    return chunks


def encode_chunk(video_path: str,
                 output_path: str,
                 start_sec: float,
                 dur_sec: float,
                 audio_stream: str = "0:a:0",
                 scale_str: str = "1280:-2",
                 fps: int = 1,
                 crf: int = 28,
                 preset: str = "fast",
                 audio_bitrate: str = "64k",
                 threads: int = 2):
    """
    Transcodes a video chunk directly into the Gemini-optimized profile:
    1 FPS H.264 AVC, mono AAC 64k, .mp4 container.
    """
    cmd = [
        FFMPEG_BIN,
        "-y", "-nostdin",
        "-ss", f"{start_sec:.3f}",
        "-t", f"{dur_sec:.3f}",
        "-i", video_path,
        "-map", "0:v:0",
        "-map", audio_stream,
        "-vf", f"fps={fps},scale={scale_str}",
        "-c:v", "libx264",
        "-crf", str(crf),
        "-preset", preset,
        "-c:a", "aac",
        "-b:a", audio_bitrate,
        "-ac", "1",
        "-avoid_negative_ts", "make_zero",
        "-map_chapters", "-1",
        "-threads", str(threads),
        output_path
    ]

    subprocess.run(cmd, check=True)


def main():
    parser = argparse.ArgumentParser(
        description="Automated Video Chunker for Long-Form Descriptive Audio (AD) Pipeline (Gemini Multimodal 1 FPS)."
    )
    parser.add_argument("input_video", help="Path to source video file (MKV, MP4, etc.)")
    parser.add_argument("-o", "--output-dir", help="Directory where chunks and manifest will be written.")
    parser.add_argument("--max-duration", type=float, default=45.0,
                        help="Maximum chunk duration in minutes (Default: 45.0).")
    parser.add_argument("--min-duration", type=float, default=30.0,
                        help="Minimum chunk duration in minutes (Default: 30.0).")
    parser.add_argument("--window-slack", type=float, default=5.0,
                        help="Candidate search window slack in minutes around target (Default: 5.0).")
    parser.add_argument("--resolution", choices=["720p", "480p"], default="720p",
                        help="Output video resolution profile (Default: 720p / 1280:-2).")
    parser.add_argument("--fps", type=int, default=1,
                        help="Output video frame rate (Default: 1 FPS per Gemini spec).")
    parser.add_argument("--crf", type=int, default=28,
                        help="H.264 video CRF quality (Default: 28).")
    parser.add_argument("--preset", default="fast",
                        help="H.264 video encoder preset (Default: fast).")
    parser.add_argument("--audio-bitrate", default="64k",
                        help="Mono AAC audio bitrate (Default: 64k).")
    parser.add_argument("--audio-stream", default=None,
                        help="Explicit audio stream selector (e.g. 0:a:0). Auto-detected if omitted.")
    parser.add_argument("--silence-noise", default="-30dB",
                        help="FFmpeg silencedetect noise threshold (Default: -30dB).")
    parser.add_argument("--silence-min-dur", type=float, default=0.5,
                        help="Minimum silence duration in seconds to qualify as a dialogue pause (Default: 0.5s).")
    parser.add_argument("--scene-threshold", type=float, default=0.25,
                        help="FFmpeg scene detection threshold (Default: 0.25).")
    parser.add_argument("--threads", type=int, default=None,
                        help="FFmpeg concurrency thread limit (Default: from config or 2).")
    parser.add_argument("--dry-run", action="store_true",
                        help="Analyze cut boundaries and produce manifest without transcoding video.")
    parser.add_argument("--overwrite", action="store_true",
                        help="Overwrite existing chunk MP4 files.")
    parser.add_argument("--no-scene-detect", action="store_true",
                        help="Disable video visual scene detection (use silence and chapters only).")
    parser.add_argument("--no-chapters", action="store_true",
                        help="Ignore Matroska/container chapter markers.")

    args = parser.parse_args()

    raw_input = args.input_video
    try:
        video_path, default_output_dir = resolve_input_video(raw_input)
    except Exception as e:
        print(f"{Color.err()} {e}")
        sys.exit(1)

    # Resolution mapping
    scale_str = "1280:-2" if args.resolution == "720p" else "854:-2"

    # Thread configuration
    threads = args.threads or CONFIG.get("default_threads", 2)

    # Setup output directory
    base_name = os.path.splitext(os.path.basename(video_path))[0]
    if args.output_dir:
        cleaned_out = args.output_dir.strip().strip('"\'')
        output_dir = os.path.abspath(cleaned_out)
    else:
        output_dir = default_output_dir

    os.makedirs(output_dir, exist_ok=True)

    print(f"\n{Color.section('================================================================================')}")
    print(f"{Color.section('Universal Audio Description (AD) Video Chunker (Gemini 1 FPS Architecture)')}")
    print(f"{Color.section('================================================================================')}")
    print(f"  Input Video : {video_path}")
    print(f"  Output Dir  : {output_dir}")
    print(f"  Resolution  : {args.resolution} ({scale_str})")
    print(f"  Profile     : {args.fps} FPS | CRF {args.crf} | Mono AAC {args.audio_bitrate}")
    print(f"  Max Chunk   : {args.max_duration} min | Min Chunk: {args.min_duration} min")
    print(f"  Dry Run     : {'Enabled' if args.dry_run else 'Disabled'}\n")

    # 1. Probe input video metadata
    print(f"{Color.info()} Probing input media metadata via ffprobe...")
    info = probe_media_info(video_path)
    total_dur = info["duration"]

    if total_dur <= 0.0:
        print(f"{Color.err()} Failed to determine video duration.")
        sys.exit(1)

    print(f"  {Color.ok()} Probed Duration : {format_timecode(total_dur)} ({total_dur:.2f}s)")
    print(f"  {Color.ok()} Video Streams   : {len(info['video_streams'])}")
    print(f"  {Color.ok()} Audio Streams   : {len(info['audio_streams'])}")
    print(f"  {Color.ok()} Chapters Found  : {len(info['chapters'])}")

    # Resolve audio stream
    audio_stream = args.audio_stream or select_best_audio_stream(info["audio_streams"])
    print(f"  {Color.ok()} Selected Audio  : {audio_stream}\n")

    # 2. Compute chunk plan
    chunks = compute_chunk_plan(
        video_path=video_path,
        total_duration=total_dur,
        chapters=info["chapters"],
        audio_stream=audio_stream,
        max_duration_min=args.max_duration,
        min_duration_min=args.min_duration,
        slack_min=args.window_slack,
        silence_noise=args.silence_noise,
        silence_min_dur=args.silence_min_dur,
        scene_threshold=args.scene_threshold,
        detect_scenes=not args.no_scene_detect,
        use_chapters=not args.no_chapters
    )

    # 3. Assign filenames and paths
    for c in chunks:
        idx_str = f"part{c['index']:02d}"
        t_start_fn = format_timecode_filename(c["start_time_seconds"])
        t_end_fn = format_timecode_filename(c["end_time_seconds"])
        filename = f"{base_name}_{idx_str}_{t_start_fn}_to_{t_end_fn}.mp4"
        c["filename"] = filename
        c["path"] = os.path.join(output_dir, filename)

    # 4. Transcode chunks (unless dry-run)
    print(f"\n{Color.section('=== Processing Video Chunks ===')}")
    total_chunks = len(chunks)
    for c in chunks:
        c_idx = c["index"]
        c_fn = c["filename"]
        header_text = f"--- Chunk {c_idx}/{total_chunks}: {c_fn} ---"
        print(f"\n{Color.section(header_text)}")
        print(f"  Timeline : {c['start_time_timecode']} -> {c['end_time_timecode']} ({c['duration_timecode']})")
        print(f"  Cut Type : {c['cut_type']} (Confidence: {c['confidence_score']:.1f})")
        print(f"  Reason   : {c['cut_reason']}")

        if args.dry_run:
            print(f"  {Color.info()} Dry run: Transcoding skipped.")
            continue

        if os.path.exists(c["path"]) and not args.overwrite:
            print(f"  {Color.ok()} Target chunk exists. Skipping (use --overwrite to re-encode).")
            continue

        print(f"  {Color.info()} Encoding 1 FPS chunk with FFmpeg (threads: {threads})...")
        t_start_encode = datetime.now()
        encode_chunk(
            video_path=video_path,
            output_path=c["path"],
            start_sec=c["start_time_seconds"],
            dur_sec=c["duration_seconds"],
            audio_stream=audio_stream,
            scale_str=scale_str,
            fps=args.fps,
            crf=args.crf,
            preset=args.preset,
            audio_bitrate=args.audio_bitrate,
            threads=threads
        )
        elapsed = (datetime.now() - t_start_encode).total_seconds()
        file_size_mb = os.path.getsize(c["path"]) / (1024 * 1024)
        print(f"  {Color.ok()} Chunk encoded in {elapsed:.1f}s (Size: {file_size_mb:.2f} MB)")

    # 5. Emit JSON Manifest
    manifest_data = {
        "source_file": video_path,
        "source_filename": os.path.basename(video_path),
        "total_duration_seconds": round(total_dur, 3),
        "total_duration_timecode": format_timecode(total_dur),
        "max_chunk_duration_seconds": args.max_duration * 60.0,
        "min_chunk_duration_seconds": args.min_duration * 60.0,
        "video_profile": {
            "fps": args.fps,
            "scale": scale_str,
            "resolution": args.resolution,
            "codec": "h264",
            "crf": args.crf,
            "preset": args.preset
        },
        "audio_profile": {
            "codec": "aac",
            "channels": 1,
            "bitrate": args.audio_bitrate,
            "stream": audio_stream
        },
        "chunks_count": len(chunks),
        "chunks": chunks,
        "downstream_guidance": {
            "gemini_system_prompt": (
                "You are an expert Audio Description (AD) writer for visually impaired audiences. "
                "Analyze the synchronized visual and audio tracks. "
                "Identify natural dialogue pauses and describe critical visual elements (actions, scene shifts, "
                "expressions, text) that occur during those pauses. Do NOT talk over spoken dialogue. "
                "Descriptions must be objective, concise, and fit comfortably within the designated time gap."
            ),
            "timeline_offset_instruction": (
                "For each chunk, add start_time_seconds to all relative timestamps produced by Gemini "
                "to yield master timeline positions."
            )
        }
    }

    manifest_path = os.path.join(output_dir, f"{base_name}_ad_manifest.json")
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest_data, f, indent=2)

    print(f"\n{Color.section('================================================================================')}")
    print(f"  {Color.ok()} Complete! Generated {len(chunks)} chunk(s).")
    print(f"  Manifest written to: {manifest_path}")
    print(f"{Color.section('================================================================================')}\n")


if __name__ == "__main__":
    main()
