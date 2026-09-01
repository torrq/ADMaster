#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
r"""
================================================================================
Universal Audio Description (AD) & Main Audio Sync & Encoding Pipeline
================================================================================
Repository: ADMaster
License: MIT

Overview:
  High-performance acoustic cross-correlation alignment, DescribeAlign Dynamic
  Time Warping (DTW) piecewise synchronization with Multilayer transition smoothing
  (VAD + Zero-Crossing + 50ms Crossfading), true ITU-R BS.775 2.0 downmixing,
  2-pass EBU R128 loudness normalization, and MKV sample multiplexing.

Usage Examples:
  1. Full Directory Auto-Discovery (Source MKV in source/, ADs in tosync/):
     python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --convert-main

  2. Batch Process All Subdirectories in a Queue:
     python scripts/sync_ad_track.py "F:\uploads\queue" --batch --convert-main

  3. Explicit Files with 5.1 Main Audio Conversion & AD Sync:
     python scripts/sync_ad_track.py \
       -r "F:\uploads\Movie\source\Movie.mkv" --ref-stream 0:a:1 \
       -a "F:\uploads\Movie\tosync\Movie_AD.mp3" \
       --convert-main

  4. Forced DescribeAlign DTW Piecewise Alignment (for alternate cuts):
     python scripts/sync_ad_track.py "F:\uploads\queue\Movie2" --force-dtw --no-pal

  5. Fast 3-minute Video Test Preview:
     python scripts/sync_ad_track.py "F:\uploads\queue\Movie1" --preview 180
"""

import argparse
from datetime import datetime
import glob
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import wave
import numpy as np

# Mute noisy internal ADSync fingerprint/drift tracing unless --debug is passed
logging.getLogger("adsync").setLevel(logging.ERROR)

try:
    from scipy.signal import fftconvolve
except ImportError:
    def fftconvolve(a, b, mode='valid'):
        return np.correlate(a, b, mode=mode)


class Color:
    """ANSI color and terminal formatting palette for ADMaster."""
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"

    # Foregrounds
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
        if os.name == 'nt':
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
    def success(cls, text="[SUCCESS]"):
        return f"{cls.GREEN}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def warn(cls, text="[WARNING]"):
        return f"{cls.YELLOW}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def err(cls, text="[ERROR]"):
        return f"{cls.RED}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def info(cls, text="[INFO]"):
        return f"{cls.CYAN}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def gap(cls, text="[GAP FILL]"):
        return f"{cls.BLUE}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def track(cls, name):
        return f"{cls.MAGENTA}{cls.BOLD}[{name}]{cls.RESET}"

    @classmethod
    def cyan(cls, text):
        return f"{cls.CYAN}{cls.BOLD}{text}{cls.RESET}"

    @classmethod
    def bold(cls, text):
        return f"{cls.WHITE}{cls.BOLD}{text}{cls.RESET}"


Color.init()


def load_config():
    """Loads configuration from config.local.json, config.json, or config.default.json, stripping comments."""
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
                    # Strip // single-line comments and /* */ multi-line
                    # comments
                    content_clean = re.sub(r'//.*', '', content)
                    content_clean = re.sub(
                        r'/\*.*?\*/', '', content_clean, flags=re.DOTALL)
                    return json.loads(content_clean)
            except Exception:
                pass
    return {}


CONFIG = load_config()


class TeeLogger:
    """Duplicates stdout/stderr to a clean log file, stripping ANSI color escape sequences."""
    def __init__(self, filepath):
        self.terminal = sys.stdout
        self.encoding = getattr(sys.stdout, "encoding", "utf-8") or "utf-8"
        self.log_file = open(filepath, "w", encoding="utf-8", buffering=1)
        self._ansi_re = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]')
        self._last_was_cr = False

    def write(self, message):
        try:
            self.terminal.write(message)
        except UnicodeEncodeError:
            try:
                term_enc = getattr(self.terminal, 'encoding', 'utf-8') or 'utf-8'
                self.terminal.write(message.encode(term_enc, errors='replace').decode(term_enc))
            except Exception:
                pass
        except Exception:
            pass

        clean_msg = self._ansi_re.sub('', message)

        if '\r' in message and '\n' not in message:
            self._last_was_cr = True
            return
        elif self._last_was_cr:
            if '\n' in message:
                self._last_was_cr = False
            clean_msg = clean_msg.lstrip('\r')

        if clean_msg:
            try:
                self.log_file.write(clean_msg)
                self.log_file.flush()
            except Exception:
                pass

    def flush(self):
        self.terminal.flush()
        self.log_file.flush()

    def close(self):
        if not self.log_file.closed:
            self.log_file.flush()
            self.log_file.close()


def format_time_code(seconds: float) -> str:
    """Format duration in seconds to standard HH:MM:SS.mmm timecode string."""
    m, s = divmod(seconds, 60)
    h, m = divmod(int(m), 60)
    return f"{h:02d}:{m:02d}:{s:06.3f}" if h else f"00:{m:02d}:{s:06.3f}"


def extract_warp_cuts(params, scratch_dir=None, name=None, jump_threshold=0.20):
    """
    Extracts cut transition timestamps and offset shifts from ADSync report or DescribeAlign segments.
    Returns list of dicts:
      [{'index': 1, 'vid_start': float, 'vid_end': float, 'offset': float, 'jump': float, 'is_initial': bool}, ...]
    """
    engine = params.get("engine_used", "adsync").lower()
    cuts = []

    # 1. ADSync Engine
    report = params.get("report")
    pts = None
    if report and getattr(report, "warp_path", None) and getattr(report.warp_path, "points", None):
        pts = report.warp_path.points
    elif scratch_dir and name:
        clean_name = re.sub(r'[^a-zA-Z0-9_-]', '_', name)
        report_json_path = os.path.join(scratch_dir, "adsync", clean_name, "report.json")
        if os.path.exists(report_json_path):
            try:
                with open(report_json_path, "r", encoding="utf-8") as f:
                    data = json.load(f)
                pts_data = data.get("warp_path", {}).get("points", [])
                if pts_data:
                    from adsync.models import WarpPoint
                    pts = [WarpPoint(**p) for p in pts_data]
            except Exception:
                pass

    if pts and len(pts) >= 2:
        offsets = [p.target_time - p.source_time for p in pts]
        runs = []
        start = 0
        for i in range(1, len(pts) + 1):
            if i == len(pts) or abs(offsets[i] - offsets[i - 1]) > jump_threshold:
                seg_offsets = offsets[start:i]
                mid_off = float(np.median(seg_offsets))
                runs.append({
                    "vid_start": pts[start].target_time,
                    "vid_end": pts[i - 1].target_time,
                    "offset": mid_off,
                })
                start = i
        for idx, r in enumerate(runs):
            if idx == 0:
                cuts.append({
                    "index": 1,
                    "vid_start": r["vid_start"],
                    "vid_end": r["vid_end"],
                    "offset": r["offset"],
                    "jump": 0.0,
                    "is_initial": True
                })
            else:
                prev = runs[idx - 1]
                cuts.append({
                    "index": idx + 1,
                    "vid_start": r["vid_start"],
                    "vid_end": r["vid_end"],
                    "offset": r["offset"],
                    "jump": r["offset"] - prev["offset"],
                    "is_initial": False
                })
        return cuts

    # 2. DescribeAlign Engine (or fallback plot file)
    segs = params.get("describealign_segments")
    if not segs and scratch_dir and name:
        clean_name = re.sub(r'[^a-zA-Z0-9_-]', '_', name)
        dtw_plots_dir = os.path.join(scratch_dir, "describealign", clean_name, "alignment_plots")
        if os.path.exists(dtw_plots_dir):
            found_plots = sorted(glob.glob(os.path.join(dtw_plots_dir, "*.txt")), key=os.path.getmtime, reverse=True)
            if found_plots:
                try:
                    import dtw_smooth_stitch
                    segs = dtw_smooth_stitch.parse_alignment_plot(found_plots[0])
                except Exception:
                    pass

    if segs and len(segs) > 0:
        for idx, s in enumerate(segs):
            cur_off = s["master_start"] - s["audio_start"]
            if idx == 0:
                cuts.append({
                    "index": 1,
                    "vid_start": s["master_start"],
                    "vid_end": s["master_end"],
                    "offset": cur_off,
                    "jump": 0.0,
                    "is_initial": True
                })
            else:
                prev_off = segs[idx - 1]["master_start"] - segs[idx - 1]["audio_start"]
                cuts.append({
                    "index": idx + 1,
                    "vid_start": s["master_start"],
                    "vid_end": s["master_end"],
                    "offset": cur_off,
                    "jump": cur_off - prev_off,
                    "is_initial": False
                })

    return cuts

# Portable local FFmpeg fallback
DEFAULT_FFMPEG = CONFIG.get("ffmpeg_path") or os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = os.path.join(
        os.path.dirname(
            os.path.dirname(
                os.path.abspath(__file__))),
        "bin",
        "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = shutil.which("ffmpeg") or "ffmpeg"


def find_describealign(cfg=None):
    """Find describealign.py from config, project directories, siblings, or PATH."""
    cfg = cfg or CONFIG
    custom_path = cfg.get("describealign_path")
    if custom_path and os.path.exists(custom_path):
        return os.path.abspath(custom_path)

    candidates = [
        os.path.join(
            os.path.dirname(
                os.path.abspath(__file__)),
            "describealign.py"),
        os.path.join(
            os.path.dirname(
                os.path.abspath(__file__)),
            "..",
            "describealign.py"),
        os.path.join(
            os.path.dirname(
                os.path.abspath(__file__)),
            "..",
            "..",
            "describealign",
            "describealign.py"),
        os.path.join(
            os.path.dirname(
                os.path.dirname(
                    os.path.dirname(
                        os.path.abspath(__file__)))),
            "describealign",
            "describealign.py"),
        "describealign.py"
    ]
    for c in candidates:
        if os.path.exists(c):
            return os.path.abspath(c)
    which = shutil.which("describealign")
    if which:
        return which
    return None


def resolve_adsync(cfg=None):
    """Resolve and import adsync package from config path, sibling repos, or environment."""
    cfg = cfg or CONFIG
    custom_path = cfg.get("adsync_path")
    candidates = []
    if custom_path:
        candidates.extend([custom_path, os.path.join(custom_path, "src")])

    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates.extend([
        os.path.join(base_dir, "..", "ADSync", "src"),
        os.path.join(base_dir, "..", "ADSync"),
        r"C:\Users\nathan\Projects\git\ADSync\src",
        r"C:\Users\nathan\Projects\git\ADSync",
        r"F:\uploads\ADSync\src",
        r"F:\uploads\ADSync",
    ])
    for c in candidates:
        if c and os.path.exists(c) and os.path.isdir(c):
            abs_c = os.path.abspath(c)
            if abs_c not in sys.path:
                sys.path.insert(0, abs_c)
            try:
                import adsync
                return abs_c
            except ImportError:
                continue
    return None


def locate_mkvtoolnix(tool_name="mkvmerge", cfg=None):
    """Locate MKVToolNix binary (mkvmerge, mkvpropedit, mkvextract) from config, local bin, or PATH."""
    cfg = cfg or CONFIG
    custom = cfg.get(f"{tool_name}_path")
    if custom and os.path.exists(custom):
        return os.path.abspath(custom)

    exe_name = f"{tool_name}.exe" if os.name == 'nt' else tool_name
    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(base_dir, "bin", exe_name),
        os.path.join(r"C:\Program Files\MKVToolNix", exe_name),
        os.path.join(r"C:\Program Files (x86)\MKVToolNix", exe_name),
    ]
    for c in candidates:
        if os.path.exists(c):
            return os.path.abspath(c)

    which = shutil.which(tool_name) or shutil.which(exe_name)
    if which:
        return which
    return exe_name


def locate_binaries(ffmpeg_path):
    """Ensure ffmpeg and ffprobe binaries are located and executable."""
    custom_ffprobe = CONFIG.get("ffprobe_path")
    if custom_ffprobe and os.path.exists(custom_ffprobe):
        ffprobe_path = custom_ffprobe
    elif ffmpeg_path and os.path.exists(ffmpeg_path):
        ffprobe_path = os.path.join(
            os.path.dirname(ffmpeg_path),
            "ffprobe.exe" if os.name == 'nt' else "ffprobe")
        if not os.path.exists(ffprobe_path):
            ffprobe_path = shutil.which("ffprobe") or "ffprobe"
    else:
        ffmpeg_path = shutil.which("ffmpeg") or "ffmpeg"
        ffprobe_path = shutil.which("ffprobe") or "ffprobe"
    return ffmpeg_path, ffprobe_path


def format_stream_selector(stream_str):
    """Formats stream selector string into standard ffprobe and ffmpeg notation."""
    if not stream_str:
        return "a:0", "0:a:0"
    s = stream_str.strip().lower()
    if s.startswith("0:"):
        s = s[2:]
    if s.startswith("a:"):
        return s, f"0:{s}"
    if s.isdigit():
        return f"a:{s}", f"0:a:{s}"
    return s, f"0:{s}"


def get_audio_info(ffprobe_bin, input_path, stream_selector="a:0"):
    """Retrieve audio duration, sample rate, and channel count using ffprobe."""
    select_stream, _ = format_stream_selector(stream_selector)
    cmd = [
        ffprobe_bin, "-v", "error", "-select_streams", select_stream,
        "-show_entries", "format=duration:stream=channels,channel_layout,duration,sample_rate",
        "-of", "json", input_path
    ]
    res = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    if res.returncode != 0:
        raise RuntimeError(
            f"Failed to probe audio for '{input_path}': {
                res.stderr}")
    data = json.loads(res.stdout)

    channels = 2
    sample_rate = 48000
    duration = 0.0
    if "streams" in data and len(data["streams"]) > 0:
        st = data["streams"][0]
        channels = int(st.get("channels", 2))
        sample_rate = int(st.get("sample_rate", 48000))
        try:
            duration = float(st.get("duration", 0.0))
        except (ValueError, TypeError):
            pass

    if duration <= 0.0:
        try:
            duration = float(data.get("format", {}).get("duration", 0.0))
        except (ValueError, TypeError):
            pass

    return duration, channels, sample_rate


def predecode_to_mono_8k(ffmpeg_bin, input_path, scratch_dir,
                         label, stream_map=None, is_pal=False, input_sr=48000):
    """Pre-decode any audio/video stream to an 8kHz mono PCM WAV for instantaneous in-memory correlation."""
    os.makedirs(scratch_dir, exist_ok=True)
    base_stem = os.path.splitext(os.path.basename(input_path))[0]
    safe_name = re.sub(r'[^a-zA-Z0-9_-]', '_', f"{label}_{base_stem}")[:64]
    if is_pal:
        safe_name += "_pal"
    out_wav = os.path.join(scratch_dir, f"{safe_name}_mono8k.wav")
    tmp_wav = os.path.join(scratch_dir, f"{safe_name}_mono8k_tmp.wav")

    if os.path.exists(out_wav):
        if os.path.getsize(out_wav) > 1024:
            return out_wav
        else:
            try:
                os.remove(out_wav)
            except Exception:
                pass

    print(
        f"  [{label}] Pre-decoding stream to 8kHz mono analysis WAV: {
            os.path.basename(input_path)}...",
        flush=True)
    cmd = [
        ffmpeg_bin,
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-fflags",
        "+genpts",
        "-i",
        input_path]
    if stream_map:
        cmd.extend(["-map", stream_map])

    af_chain = ["asetpts=PTS-STARTPTS"]
    if is_pal:
        pal_rate = float(input_sr) * (23.976023976 / 25.0)
        af_chain.append(f"asetrate={pal_rate:.3f}")
    af_chain.append("aresample=8000")
    af_chain.append("pan=mono|c0=c0")

    cmd.extend([
        "-vn", "-af", ",".join(af_chain),
        "-c:a", "pcm_s16le", "-f", "wav", tmp_wav
    ])
    res = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    if res.returncode != 0 or not os.path.exists(tmp_wav):
        if os.path.exists(tmp_wav):
            try:
                os.remove(tmp_wav)
            except Exception:
                pass
        raise RuntimeError(
            f"Failed to pre-decode audio '{input_path}' to 8kHz WAV: {res.stderr.strip()}")
    os.replace(tmp_wav, out_wav)
    return out_wav


def read_wav_samples(wav_path):
    """Read float32 PCM samples from a WAV file."""
    with wave.open(wav_path, 'rb') as wf:
        nframes = wf.getnframes()
        raw_data = wf.readframes(nframes)
        samples = np.frombuffer(raw_data, dtype=np.int16).astype(np.float32)
    return samples


def find_offset_candidates(measurements, min_conf=0.15, cluster_tol=0.03):
    """Clusters anchor measurements to find the consensus winner and runner-up candidate offsets."""
    valid = [(off, peak) for off, peak in measurements if peak >= min_conf]
    if not valid:
        valid = measurements
    if not valid:
        return [(0.0, 0.0, 0)]

    clusters = []
    for off, peak in sorted(valid, key=lambda x: x[1], reverse=True):
        matched = False
        for cl in clusters:
            if abs(cl["offset"] - off) <= cluster_tol:
                cl["points"].append((off, peak))
                cl["count"] += 1
                if peak > cl["max_peak"]:
                    cl["max_peak"] = peak
                matched = True
                break
        if not matched:
            clusters.append({
                "offset": off,
                "max_peak": peak,
                "count": 1,
                "points": [(off, peak)]
            })

    for cl in clusters:
        cl_offsets = [p[0] for p in cl["points"]]
        cl["median_offset"] = float(np.median(cl_offsets))

    # Sort by anchor count first (consensus), then by maximum peak confidence
    clusters.sort(key=lambda x: (x["count"], x["max_peak"]), reverse=True)
    return [(cl["median_offset"], cl["max_peak"], cl["count"])
            for cl in clusters]


def compute_sync_parameters(ffmpeg_bin, ffprobe_bin, base_path, ad_tracks, target_duration, scratch_dir, ref_stream_map=None, num_anchors=100,
                            force_stretch=False, no_stretch=False, min_confidence=0.15, force_dtw=False, no_dtw_fallback=False, force_pal=False, no_pal=False,
                            no_multiband=False, no_speed_detect=False, debug=False):
    """Computes delay offset via high-confidence multi-window FFT cross-correlation in memory."""
    print(
        "\n=== COMPUTING ACOUSTIC CROSS-CORRELATION SYNC PARAMETERS ===",
        flush=True)

    anchor_times = np.linspace(
        target_duration * 0.05,
        target_duration * 0.88,
        num_anchors)
    print(
        f"  Generated {num_anchors} analysis anchors across the timeline (~every {
            target_duration *
            0.83 /
            (
                num_anchors *
                60):.1f} mins).",
        flush=True)
    sync_results = {}

    # Pre-decode reference audio ONCE
    base_mono_wav = predecode_to_mono_8k(
        ffmpeg_bin,
        base_path,
        scratch_dir,
        "REF",
        stream_map=ref_stream_map)
    base_all_samples = read_wav_samples(base_mono_wav)
    sr = 8000

    for name, path in ad_tracks.items():
        _, _, ad_sr = get_audio_info(ffprobe_bin, path)
        is_pal_track = (force_pal or ("uk" in name.lower(
        ) or "pal" in name.lower() or "25fps" in name.lower())) and not no_pal

        # Automatic Acoustic Landmark Speed / PAL Probing
        if not is_pal_track and not no_pal and not no_speed_detect:
            try:
                from align_features import detect_speed_pal
            except ImportError:
                try:
                    from scripts.align_features import detect_speed_pal
                except ImportError:
                    detect_speed_pal = None

            if detect_speed_pal is not None:
                probe_ad_wav = predecode_to_mono_8k(
                    ffmpeg_bin, path, scratch_dir, f"{name}_rawprobe", is_pal=False, input_sr=ad_sr
                )
                probe_ad_samples = read_wav_samples(probe_ad_wav)
                if len(probe_ad_samples) > 0 and len(base_all_samples) > 0:
                    is_detected_pal, speed_ratio, speed_conf = detect_speed_pal(
                        base_all_samples, probe_ad_samples, sr=8000
                    )
                    if is_detected_pal:
                        print(
                            f"  [SPEED PROBE] Acoustic landmark probe detected PAL speed transfer (stretch ×{speed_ratio:.4f}) for [{name}]!",
                            flush=True)
                        is_pal_track = True

        if force_dtw:
            print(
                f"\n[{name}] Forced DescribeAlign DTW mode enabled (bypassing cross-correlation search).",
                flush=True)
            sync_results[name] = {
                "offset": 0.0,
                "atempo": 1.0,
                "confidence": 1.0,
                "method": "dtw",
                "is_pal": is_pal_track
            }
            continue

        print(
            f"\nAnalyzing acoustic correlation for [{name}] (PAL={is_pal_track} @ {ad_sr} Hz)...",
            flush=True)
        ad_mono_wav = predecode_to_mono_8k(
            ffmpeg_bin,
            path,
            scratch_dir,
            name,
            is_pal=is_pal_track,
            input_sr=ad_sr)
        ad_all_samples = read_wav_samples(ad_mono_wav)

        try:
            from align_features import multiband_normalized_correlation
        except ImportError:
            try:
                from scripts.align_features import multiband_normalized_correlation
            except ImportError:
                multiband_normalized_correlation = None

        measurements = []
        # Pre-filter anchors within both reference and AD duration limits
        eligible_anchors = [
            a for a in anchor_times
            if (a + 30.0 <= target_duration) and (int((max(0.0, a - 30.0)) * sr) < len(ad_all_samples))
        ]
        total_valid = len(eligible_anchors)
        processed_count = 0

        for anchor in anchor_times:
            if anchor + 30.0 > target_duration:
                continue

            base_start_idx = int(anchor * sr)
            base_end_idx = int((anchor + 30.0) * sr)
            if base_start_idx >= len(base_all_samples):
                continue
            base_samples = base_all_samples[base_start_idx: min(
                len(base_all_samples), base_end_idx)]

            search_start_s = max(0.0, anchor - 30.0)
            search_end_s = anchor + 60.0
            ad_start_idx = int(search_start_s * sr)
            ad_end_idx = int(search_end_s * sr)
            if ad_start_idx >= len(ad_all_samples):
                continue
            ad_samples = ad_all_samples[ad_start_idx: min(
                len(ad_all_samples), ad_end_idx)]

            if len(ad_samples) < len(base_samples) or len(base_samples) == 0:
                continue

            if multiband_normalized_correlation is not None and not no_multiband:
                norm_corr = multiband_normalized_correlation(ad_samples, base_samples, sr, multiband=True)
                if norm_corr is not None and len(norm_corr) > 0:
                    best_idx = int(np.argmax(norm_corr))
                    norm_peak = float(norm_corr[best_idx])
                else:
                    corr = fftconvolve(ad_samples, base_samples[::-1], mode='valid')
                    best_idx = int(np.argmax(corr))
                    norm_peak = float(corr[best_idx] / (np.linalg.norm(base_samples) * np.linalg.norm(
                        ad_samples[best_idx:best_idx + len(base_samples)]) + 1e-9))
            else:
                corr = fftconvolve(ad_samples, base_samples[::-1], mode='valid')
                best_idx = int(np.argmax(corr))
                norm_peak = float(corr[best_idx] / (np.linalg.norm(base_samples) * np.linalg.norm(
                    ad_samples[best_idx:best_idx + len(base_samples)]) + 1e-9))

            offset = search_start_s + (best_idx / sr) - anchor
            measurements.append((offset, norm_peak))
            processed_count += 1

            if debug:
                print(
                    f"  Anchor {anchor:4.0f}s: Measured Offset = {offset:+.3f}s | Confidence = {norm_peak * 100:5.1f}% ({norm_peak:.4f})",
                    flush=True)
            else:
                pct = int(100 * processed_count / total_valid) if total_valid else 100
                bar_len = 25
                filled = int(bar_len * processed_count / total_valid) if total_valid else bar_len
                bar = "=" * filled + "-" * (bar_len - filled)
                cur_cands = find_offset_candidates(measurements, min_conf=min_confidence)
                winner_off, winner_conf, winner_cnt = cur_cands[0]
                conf_c = Color.GREEN if winner_conf >= 0.20 else (Color.YELLOW if winner_conf >= 0.10 else Color.RED)
                anchor_str = f"{winner_cnt} anchor" if winner_cnt == 1 else f"{winner_cnt} anchors"
                sys.stdout.write(f"\r  Correlation Progress: [{Color.CYAN}{bar}{Color.RESET}] {pct:3d}% ({processed_count}/{total_valid}) | Best: {Color.BOLD}{winner_off:+.3f}s{Color.RESET} ({conf_c}{winner_conf*100:.1f}%{Color.RESET} | {anchor_str})")
                sys.stdout.flush()

        if not debug:
            sys.stdout.write("\n")
            sys.stdout.flush()

        candidates = find_offset_candidates(
            measurements, min_conf=min_confidence)
        winner_offset, winner_peak, winner_count = candidates[0]
        if abs(winner_offset) < 0.008:
            winner_offset = 0.0
        init_offset = winner_offset
        best_overall_confidence = winner_peak
        atempo = 1.0

        if force_stretch and not no_stretch:
            cl_seek = max(0.0, target_duration - 180.0)
            base_cl_start = int(cl_seek * sr)
            base_cl_end = int((cl_seek + 30.0) * sr)
            base_cl_samples = base_all_samples[base_cl_start: min(
                len(base_all_samples), base_cl_end)]

            est_cl_start_s = max(0.0, cl_seek + init_offset - 30.0)
            ad_cl_start = int(est_cl_start_s * sr)
            ad_cl_end = int((est_cl_start_s + 90.0) * sr)
            ad_cl_samples = ad_all_samples[ad_cl_start: min(
                len(ad_all_samples), ad_cl_end)]

            if len(ad_cl_samples) >= len(
                    base_cl_samples) and len(base_cl_samples) > 0:
                corr_cl = fftconvolve(
                    ad_cl_samples, base_cl_samples[::-1], mode='valid')
                best_cl_idx = np.argmax(corr_cl)
                actual_cl_t = est_cl_start_s + (best_cl_idx / sr)

                t_ad_opening = 300.0 + init_offset
                t_ad_delta = actual_cl_t - t_ad_opening
                speed_ratio = t_ad_delta / \
                    (cl_seek - 300.0) if cl_seek > 300.0 else 1.0
                computed_atempo = speed_ratio

                if (1.030 <= computed_atempo <= 1.050) or (
                        0.950 <= computed_atempo <= 0.970):
                    atempo = computed_atempo
                    print(
                        f"  [{name}] Speed Stretch Matched: {
                            atempo:.6f}x", flush=True)
                else:
                    print(
                        f"  [{name}] Speed ratio {
                            computed_atempo:.6f}x outside standard range. Reverting to 1.000000x.",
                        flush=True)
        else:
            print(f"  [{name}] Native rate (1.000000x) enforced.", flush=True)

        # 1. Consensus Quorum Check: A single isolated anchor (e.g. 1/100 or < 3% of timeline)
        # without overwhelming peak confidence (>= 0.40) is acoustic
        # noise/false-positive, not consensus.
        min_quorum = max(3, int(len(measurements) * 0.03))
        has_insufficient_quorum = (
            winner_count < min_quorum and best_overall_confidence < 0.40)

        # 2. Drifting Multi-Cluster Check: Distinct offset clusters with
        # significant delta (> 0.20s)
        has_drifting_clusters = False
        for r_off, r_peak, r_count in candidates[1:5]:
            if r_peak >= min_confidence and abs(
                    r_off - init_offset) > 0.20 and (r_count >= 2 or r_peak >= 0.25):
                has_drifting_clusters = True
                break

        if (best_overall_confidence <
                min_confidence or has_insufficient_quorum or has_drifting_clusters) and not no_dtw_fallback:
            if has_drifting_clusters:
                print(
                    f"  {Color.warn('[WARNING]')} Multiple distinct offset clusters detected across timeline (drifting offsets / reel changes / structural cuts).",
                    flush=True)
            elif has_insufficient_quorum:
                print(
                    f"  {Color.warn('[WARNING]')} Insufficient anchor consensus: Only {winner_count}/{
                        len(measurements)} anchors agreed on winner offset with {
                        best_overall_confidence * 100:.1f}% peak confidence (< {min_quorum} anchors required for quorum).",
                    flush=True)
            else:
                print(
                    f"  {Color.warn('[WARNING]')} Max correlation confidence is only {
                        best_overall_confidence *
                        100:.1f}% (< {
                        min_confidence *
                        100:.1f}%) with erratic offsets.",
                    flush=True)
            print(
                f"  {Color.info('[DIAGNOSTIC]')} Cut/version mismatch or structural edits detected for {Color.track(name)}.",
                flush=True)
            warp_eng = CONFIG.get("default_warp_engine", "adsync").upper()
            print(
                f"  {Color.warn('[FALLBACK]')} Engaging {Color.cyan(warp_eng)} Dynamic Time Warping (DTW) + Smooth Stitching...",
                flush=True)
            method = "dtw"
        else:
            method = "cross_correlation"

        sync_results[name] = {
            "offset": init_offset,
            "atempo": atempo,
            "confidence": best_overall_confidence,
            "method": method,
            "is_pal": is_pal_track
        }
        
        conf_color = Color.GREEN if best_overall_confidence >= 0.20 else (Color.YELLOW if best_overall_confidence >= 0.10 else Color.RED)
        method_str = Color.cyan("CROSS_CORRELATION") if method == "cross_correlation" else Color.warn("DTW")
        print(
            f"  => {Color.ok('CONSENSUS OFFSET')} for {Color.track(name)}: {Color.bold(f'{init_offset:+.3f}s')} (Max Confidence: {conf_color}{best_overall_confidence * 100:.1f}%{Color.RESET} | {winner_count}/{len(measurements)} anchors | Method: {method_str})",
            flush=True)

        runner_up_idx = 1
        for r_off, r_peak, r_count in candidates[1:]:
            if abs(r_off - init_offset) > 0.01:
                print(
                    f"     Runner-up #{runner_up_idx:<6}: {Color.bold(f'{r_off:+.3f}s')} (Max Confidence: {r_peak * 100:.1f}% | {r_count}/{len(measurements)} anchors)",
                    flush=True)
                runner_up_idx += 1
                if runner_up_idx > 2:
                    break
        print("", flush=True)

    return sync_results


def render_main_tracks(ffmpeg_bin, ref_path, base_title, out_dir, scratch_dir, channels, stream_map=None,
                       preview_sec=None, threads=2, keep_scratch=False, overwrite=False, no_night=False):
    """Renders standard converted formats for Main Audio (Surround AC3, Stereo AAC 2.0, Night Mode AAC 2.0).
    Enforces strict ITU-R BS.775 2.0 downmixing and -ac 2."""
    print("\n=== RENDERING MAIN AUDIO TRACKS ===", flush=True)
    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(scratch_dir, exist_ok=True)

    suffix = f" (Preview {int(preview_sec)}s)" if preview_sec else ""
    pan_stereo = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"

    # 1. Surround 5.1 AC3 (if 6+ channels)
    if channels >= 6:
        out_surround = os.path.join(
            out_dir, f"{base_title} (Surround){suffix}.ac3")
        if os.path.exists(out_surround) and os.path.getsize(
                out_surround) > 1000 and not overwrite:
            print(
                f"  [SKIP] 5.1 Surround AC3 already exists: {
                    os.path.basename(out_surround)} (use --overwrite to force)",
                flush=True)
        else:
            print(f"  [1/3] Encoding 5.1 Surround AC3 (640k)...", flush=True)
            cmd_surround = [
                ffmpeg_bin,
                "-y",
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-threads",
                str(threads),
                "-i",
                ref_path]
            if stream_map:
                cmd_surround.extend(["-map", stream_map])
            if preview_sec:
                cmd_surround.extend(["-t", f"{preview_sec:.3f}"])
            cmd_surround.extend(
                ["-c:a", "ac3", "-b:a", "640k", "-ar", "48000", out_surround])
            res = subprocess.run(
                cmd_surround,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True)
            if res.returncode != 0:
                print(
                    f"  Error rendering surround audio: {
                        res.stderr}", flush=True)
            else:
                print(f"  [SUCCESS] Saved: {out_surround}", flush=True)

    out_stereo = os.path.join(out_dir, f"{base_title} (Stereo){suffix}.m4a")
    out_night = os.path.join(out_dir, f"{base_title} (Night Mode){suffix}.m4a")

    stereo_exists = os.path.exists(
        out_stereo) and os.path.getsize(out_stereo) > 1000
    night_exists = True if no_night else (os.path.exists(
        out_night) and os.path.getsize(out_night) > 1000)

    if stereo_exists and night_exists and not overwrite:
        print(
            f"  [SKIP] Main Stereo & Night Mode AAC already exist in output/ (use --overwrite to force re-render):",
            flush=True)
        print(
            f"    [OK] {
                os.path.basename(out_stereo)} ({
                os.path.getsize(out_stereo) /
                (
                    1024 *
                    1024):.1f} MB)",
            flush=True)
        if not no_night:
            print(
                f"    [OK] {
                    os.path.basename(out_night)} ({
                    os.path.getsize(out_night) / (
                        1024 * 1024):.1f} MB)",
                flush=True)
        return

    # 2. Extract intermediate stereo WAV in scratch (BS.775 downmix)
    stereo_wav = os.path.join(scratch_dir, "main_stereo_temp.wav")
    print(
        f"  Extracting intermediate True 2.0 Stereo WAV (ITU-R BS.775 downmix)...",
        flush=True)
    cmd_wav = [
        ffmpeg_bin,
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "fatal",
        "-threads",
        str(threads),
        "-fflags",
        "+genpts",
        "-i",
        ref_path]
    if stream_map:
        cmd_wav.extend(["-map", stream_map])
    if preview_sec:
        cmd_wav.extend(["-t", f"{preview_sec:.3f}"])
    if channels >= 6:
        cmd_wav.extend(["-af",
                        f"asetpts=PTS-STARTPTS,{pan_stereo},aresample=48000",
                        "-ac",
                        "2",
                        "-c:a",
                        "pcm_s16le",
                        stereo_wav])
    else:
        cmd_wav.extend(["-af",
                        "asetpts=PTS-STARTPTS,aresample=48000",
                        "-ac",
                        "2",
                        "-c:a",
                        "pcm_s16le",
                        stereo_wav])

    res_wav = subprocess.run(
        cmd_wav,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    if res_wav.returncode != 0:
        print(
            f"  Error extracting intermediate stereo WAV: {
                res_wav.stderr}",
            flush=True)
        return

    # 3. Standard Stereo AAC 2.0 from stereo WAV
    if stereo_exists and not overwrite:
        print(
            f"  [SKIP] Main Stereo AAC already exists: {
                os.path.basename(out_stereo)}",
            flush=True)
    else:
        print(
            f"  Encoding Main Stereo AAC 2.0 (alimiter, full dynamic range)...",
            flush=True)
        cmd_stereo = [
            ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                threads),
            "-i", stereo_wav,
            "-af", "alimiter=level_in=1:level_out=0.98:limit=0.98",
            "-ac", "2", "-ar", "48000",
            "-c:a", "aac", "-b:a", "256k", out_stereo
        ]
        res_stereo = subprocess.run(
            cmd_stereo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_stereo.returncode == 0:
            print(f"  [SUCCESS] Saved: {out_stereo}", flush=True)
        else:
            print(f"  Error rendering stereo: {res_stereo.stderr}", flush=True)

    # 4. Night Mode AAC 2.0 from stereo WAV
    if no_night:
        pass
    elif night_exists and not overwrite:
        print(
            f"  [SKIP] Main Night Mode AAC already exists: {
                os.path.basename(out_night)}",
            flush=True)
    else:
        print(
            f"  Encoding Main Night Mode AAC 2.0 (loudnorm -16 LUFS)...",
            flush=True)
        cmd_night = [
            ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                threads),
            "-i", stereo_wav,
            "-af", "loudnorm=I=-16:TP=-1.5:LRA=11",
            "-ac", "2", "-ar", "48000",
            "-c:a", "aac", "-b:a", "256k", out_night
        ]
        res_night = subprocess.run(
            cmd_night,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_night.returncode == 0:
            print(f"  [SUCCESS] Saved: {out_night}", flush=True)
        else:
            print(
                f"  Error rendering night mode: {
                    res_night.stderr}",
                flush=True)

    # 5. Clean up intermediate WAV
    if not keep_scratch and os.path.exists(stereo_wav):
        try:
            os.remove(stereo_wav)
        except Exception:
            pass


def mux_video_sample(ffmpeg_bin, video_src, audio_src,
                     out_mkv, render_duration):
    """Multiplexes generated AD track with video stream using zero-re-encode stream copy (-c:v copy)."""
    print(
        f"  Multiplexing Video Sample MKV ({
            render_duration:.0f}s)...",
        flush=True)
    cmd_mux = [
        ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal",
        "-ss", "0", "-t", f"{render_duration:.3f}", "-i", video_src,
        "-i", audio_src,
        "-map", "0:v:0", "-map", "1:a:0",
        "-c:v", "copy", "-c:a", "copy",
        "-metadata:s:a:0", "title=Audio Description (Synced)",
        "-disposition:a:0", "default",
        out_mkv
    ]
    res = subprocess.run(
        cmd_mux,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    if res.returncode == 0 and os.path.exists(out_mkv):
        size_mb = os.path.getsize(out_mkv) / (1024 * 1024)
        print(
            f"  [SUCCESS] Saved MKV Video Sample: {out_mkv} ({
                size_mb:.1f} MB)",
            flush=True)
    else:
        print(f"  Warning: MKV Video Muxing failed: {res.stderr}", flush=True)


def extract_master_stereo_wav(
    ffmpeg_bin: str,
    ref_path: str,
    ref_stream_map: str,
    channels: int,
    scratch_dir: str,
    threads: int = 2
) -> str:
    """Extract or retrieve a clean 48kHz 2.0 stereo downmix of the master reference audio."""
    ref_stereo_wav = os.path.join(scratch_dir, "ref_master_stereo_48k.wav")
    if os.path.exists(ref_stereo_wav) and os.path.getsize(ref_stereo_wav) > 1000:
        return ref_stereo_wav

    print("  [GAP FILL] Pre-decoding Reference Master Audio 2.0 stereo downmix...", flush=True)
    tmp_wav = os.path.join(scratch_dir, "ref_master_stereo_48k_tmp.wav")
    cmd = [
        ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal",
        "-threads", str(threads), "-fflags", "+genpts", "-i", ref_path
    ]
    if ref_stream_map:
        cmd.extend(["-map", ref_stream_map])

    if channels >= 6:
        filter_str = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,asetpts=PTS-STARTPTS,aresample=48000"
    else:
        filter_str = "asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo"

    cmd.extend(["-vn", "-af", filter_str, "-ac", "2", "-c:a", "pcm_s16le", tmp_wav])
    subprocess.run(cmd, check=True)
    os.replace(tmp_wav, ref_stereo_wav)
    return ref_stereo_wav


def fill_master_audio_gaps(
    ffmpeg_bin: str,
    ad_wav_path: str,
    ref_path: str,
    ref_stream_map: str,
    channels: int,
    scratch_dir: str,
    threads: int = 2,
    explicit_gaps: list[tuple[float, float, float]] = None,
    crossfade_ms: int = 50,
) -> int:
    """
    Splices volume-matched Reference Master Audio with equal-power cosine crossfades
    into explicit structural gap intervals (caused by excised scenes in alternate cuts).
    """
    try:
        import soundfile as sf
        import numpy as np
    except ImportError:
        return 0

    if not explicit_gaps or not os.path.exists(ad_wav_path) or os.path.getsize(ad_wav_path) < 1000:
        return 0

    gaps = explicit_gaps

    # Ensure ad_wav_path is 48000 Hz 16-bit stereo WAV
    ad_info = sf.info(ad_wav_path)
    if ad_info.samplerate != 48000 or ad_info.channels != 2:
        tmp_48k = ad_wav_path.replace(".wav", "_48k_tmp.wav")
        cmd_resample = [
            ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal",
            "-threads", str(threads), "-i", ad_wav_path,
            "-af", "asetpts=PTS-STARTPTS,aresample=48000",
            "-ac", "2", "-c:a", "pcm_s16le", tmp_48k
        ]
        subprocess.run(cmd_resample, check=True)
        os.replace(tmp_48k, ad_wav_path)
        ad_info = sf.info(ad_wav_path)

    sr = 48000

    print(f"\n  [GAP FILL] Splicing Reference Master Audio into {len(gaps)} explicit missing scene gap(s):", flush=True)
    for s, e, d in gaps:
        m_s, sec_s = divmod(int(s), 60)
        h_s, m_s = divmod(m_s, 60)
        m_e, sec_e = divmod(int(e), 60)
        h_e, m_e = divmod(m_e, 60)
        print(f"    - Excised Scene Void: {h_s:02d}:{m_s:02d}:{sec_s:02d} -> {h_e:02d}:{m_e:02d}:{sec_e:02d} ({d:.1f}s)", flush=True)

    # Pre-decode / get Reference Master Audio stereo downmix
    ref_stereo_wav = extract_master_stereo_wav(
        ffmpeg_bin, ref_path, ref_stream_map, channels, scratch_dir, threads
    )

    ad_audio, _ = sf.read(ad_wav_path, dtype="float32")
    master_audio, _ = sf.read(ref_stereo_wav, dtype="float32")

    if ad_audio.ndim == 1:
        ad_audio = np.column_stack((ad_audio, ad_audio))
    if master_audio.ndim == 1:
        master_audio = np.column_stack((master_audio, master_audio))

    target_len = min(len(ad_audio), len(master_audio))
    ad_audio = ad_audio[:target_len]
    master_audio = master_audio[:target_len]

    fade_len = int(sr * (crossfade_ms / 1000.0))
    t_fade = np.linspace(0, np.pi / 2, fade_len)
    fade_in = np.sin(t_fade)[:, np.newaxis]
    fade_out = np.cos(t_fade)[:, np.newaxis]

    for s, e, d in gaps:
        start_idx = int(s * sr)
        end_idx = min(int(e * sr), target_len)

        if end_idx <= start_idx + 2 * fade_len:
            ad_audio[start_idx:end_idx] = master_audio[start_idx:end_idx]
            continue

        ad_audio[start_idx:start_idx + fade_len] = (
            ad_audio[start_idx:start_idx + fade_len] * fade_out +
            master_audio[start_idx:start_idx + fade_len] * fade_in
        )
        ad_audio[start_idx + fade_len:end_idx - fade_len] = master_audio[start_idx + fade_len:end_idx - fade_len]
        ad_audio[end_idx - fade_len:end_idx] = (
            master_audio[end_idx - fade_len:end_idx] * fade_out +
            ad_audio[end_idx - fade_len:end_idx] * fade_in
        )

    tmp_out = ad_wav_path.replace(".wav", "_gapfilled_tmp.wav")
    sf.write(tmp_out, ad_audio, sr)
    os.replace(tmp_out, ad_wav_path)
    print(f"  [GAP FILL] Spliced Master Reference Audio into {len(gaps)} gap(s) with {crossfade_ms}ms crossfades.\n", flush=True)
    return len(gaps)


def render_ad_tracks(ffmpeg_bin, ffprobe_bin, ref_path, base_title, ad_tracks, sync_params, target_duration, out_dir, scratch_dir, ref_stream_map=None,
                     target_i=-16.0, target_tp=-1.5, preview_sec=None, video_mux_path=None, threads=2, keep_scratch=False, overwrite=False, no_night=False, debug=False,
                     warp_engine="adsync", no_speed_detect=False, no_multiband=False, no_gap_fill=False):
    """Renders synced AD tracks: Stereo AAC (alimiter) and Night Mode AAC (loudnorm).
    Supports single-offset intermediate WAV rendering and dual-engine warp fallback (ADSync Viterbi DP / DescribeAlign DTW)."""
    print(f"\n{Color.CYAN}=== RENDERING AUDIO DESCRIPTION TRACKS ==={Color.RESET}", flush=True)
    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(scratch_dir, exist_ok=True)

    render_duration = target_duration if not preview_sec else min(
        preview_sec, target_duration)
    suffix = f" (Preview {int(render_duration)}s)" if preview_sec else ""

    for name, path in ad_tracks.items():
        params = sync_params[name]
        offset = params["offset"]
        atempo = params["atempo"]
        method = params.get("method", "cross_correlation")
        is_pal = params.get("is_pal", False)

        label_prefix = f"{base_title}-AD" if len(
            ad_tracks) == 1 else f"{base_title}-AD [{name}]"
        out_stereo = os.path.join(
            out_dir, f"{label_prefix} (Stereo){suffix}.m4a")
        out_night = os.path.join(
            out_dir, f"{label_prefix} (Night Mode){suffix}.m4a")
        out_mkv = os.path.join(
            out_dir, f"{base_title} (Sample Sync Preview {
                int(render_duration)}s).mkv") if video_mux_path else None

        stereo_exists = os.path.exists(
            out_stereo) and os.path.getsize(out_stereo) > 1000
        night_exists = True if no_night else (os.path.exists(
            out_night) and os.path.getsize(out_night) > 1000)
        mkv_exists = (
            not video_mux_path) or (
            os.path.exists(out_mkv) and os.path.getsize(out_mkv) > 1000)

        if stereo_exists and night_exists and mkv_exists and not overwrite:
            print(
                f"\n  {Color.info('[SKIP]')} All AD deliverables for {Color.track(name)} already exist in output/ (use --overwrite to force re-render):",
                flush=True)
            print(
                f"    {Color.ok()} {os.path.basename(out_stereo)} ({os.path.getsize(out_stereo) / (1024 * 1024):.1f} MB)",
                flush=True)
            if not no_night:
                print(
                    f"    {Color.ok()} {os.path.basename(out_night)} ({os.path.getsize(out_night) / (1024 * 1024):.1f} MB)",
                    flush=True)
            if out_mkv and os.path.exists(out_mkv):
                print(
                    f"    {Color.ok()} {os.path.basename(out_mkv)} ({os.path.getsize(out_mkv) / (1024 * 1024):.1f} MB)",
                    flush=True)
            continue

        # Probe input AD sample rate
        _, _, ad_sr = get_audio_info(ffprobe_bin, path)

        if method == "dtw":
            engine_used = warp_engine
            raw_stitched_wav = os.path.join(scratch_dir, f"{name}_stitched_raw.wav")

            if engine_used == "adsync":
                print(
                    f"\n--- [{Color.CYAN}{Color.BOLD}WARP MODE: ADSync Viterbi+PCHIP{Color.RESET}] Processing {Color.track(name)} ---",
                    flush=True)
                clean_name = re.sub(r'[^a-zA-Z0-9_-]', '_', name)
                adsync_scratch = os.path.join(scratch_dir, "adsync", clean_name)
                os.makedirs(adsync_scratch, exist_ok=True)

                try:
                    import adsync._pipeline as ad_pipe
                    import adsync.config as ad_conf
                    from pathlib import Path
                    import shutil

                    cfg = ad_conf.SyncConfig(
                        mode="warp",
                        speed_detect=not no_speed_detect,
                        fingerprint=True,
                        crossfade_ms=50,
                        output_sr=48000,
                        analysis_sr=16000,
                        multiband=not no_multiband,
                    )

                    report = ad_pipe.run_pipeline(
                        video_path=Path(ref_path),
                        ad_path=Path(path),
                        output_path=None,
                        config=cfg,
                        debug_dir=Path(adsync_scratch),
                        mux=False,
                    )

                    rendered_debug_wav = os.path.join(adsync_scratch, "synced_ad.wav")
                    if os.path.exists(rendered_debug_wav):
                        shutil.copyfile(rendered_debug_wav, raw_stitched_wav)
                        sync_params[name]["report"] = report
                        sync_params[name]["engine_used"] = "adsync"
                        try:
                            with open(os.path.join(adsync_scratch, "report.json"), "w", encoding="utf-8") as f:
                                f.write(report.model_dump_json(indent=2))
                        except Exception:
                            pass
                        try:
                            from adsync.report.json_report import print_summary as adsync_print_summary
                            adsync_print_summary(report)
                        except Exception:
                            pass
                        print(f"  {Color.success('[SUCCESS]')} ADSync Viterbi Warp rendered successfully! (Confidence: {Color.bold(f'{report.confidence*100:.1f}%')})", flush=True)
                    else:
                        raise RuntimeError("ADSync did not output synced_ad.wav")
                except Exception as e:
                    print(f"  {Color.warn('[WARN]')} ADSync native engine encountered issue ({e}). Falling back to DescribeAlign DTW...", flush=True)
                    engine_used = "describealign"

            if engine_used == "describealign":
                print(
                    f"\n--- [{Color.YELLOW}{Color.BOLD}DTW MODE: DescribeAlign{Color.RESET}] Processing {Color.track(name)} via DescribeAlign & Smooth Stitching ---",
                    flush=True)
                da_script = find_describealign()
                if not da_script:
                    print(
                        f"  [ERROR] describealign.py not found on system! Falling back to single-offset render.",
                        flush=True)
                    method = "cross_correlation"
                else:
                    clean_name = re.sub(r'[^a-zA-Z0-9_-]', '_', name)
                dtw_scratch = os.path.join(scratch_dir, "describealign", clean_name)
                dtw_plots_dir = os.path.join(dtw_scratch, "alignment_plots")
                dtw_out_dir = os.path.join(dtw_scratch, "videos_with_ad")
                os.makedirs(dtw_plots_dir, exist_ok=True)
                os.makedirs(dtw_out_dir, exist_ok=True)

                # Pre-decode reference mono WAV for DescribeAlign
                ref_da_wav = os.path.join(
                    scratch_dir, "ref_analysis_48k_mono.wav")
                if not (os.path.exists(ref_da_wav) and os.path.getsize(ref_da_wav) > 1000):
                    print(
                        f"  Pre-decoding Reference Audio to 48kHz mono WAV for DescribeAlign...",
                        flush=True)
                    cmd_ref_wav = [
                        ffmpeg_bin,
                        "-y",
                        "-nostdin",
                        "-hide_banner",
                        "-loglevel",
                        "fatal",
                        "-threads",
                        str(threads),
                        "-fflags",
                        "+genpts",
                        "-i",
                        ref_path]
                    if ref_stream_map:
                        cmd_ref_wav.extend(["-map", ref_stream_map])
                    cmd_ref_wav.extend(["-vn",
                                        "-af",
                                        "asetpts=PTS-STARTPTS,aresample=48000",
                                        "-ac",
                                        "1",
                                        "-c:a",
                                        "pcm_s16le",
                                        ref_da_wav])
                    subprocess.run(cmd_ref_wav)

                # Pre-decode AD mono WAV for DescribeAlign (applying dynamic
                # sample rate PAL slowdown if PAL)
                ad_da_wav = os.path.join(
                    scratch_dir, f"{name}_analysis_48k_mono.wav")
                if not (os.path.exists(ad_da_wav) and os.path.getsize(ad_da_wav) > 1000):
                    print(
                        f"  Pre-decoding AD Audio to 48kHz mono WAV for DescribeAlign (PAL={is_pal})...",
                        flush=True)
                    cmd_ad_wav = [
                        ffmpeg_bin,
                        "-y",
                        "-nostdin",
                        "-hide_banner",
                        "-loglevel",
                        "fatal",
                        "-threads",
                        str(threads),
                        "-fflags",
                        "+genpts",
                        "-i",
                        path,
                        "-vn"]
                    if is_pal:
                        pal_rate = float(ad_sr) * (23.976023976 / 25.0)
                        cmd_ad_wav.extend(
                            ["-af", f"asetpts=PTS-STARTPTS,asetrate={pal_rate:.3f},aresample=48000:resampler=soxr:precision=28"])
                    else:
                        cmd_ad_wav.extend(
                            ["-af", "asetpts=PTS-STARTPTS,aresample=48000:resampler=soxr:precision=28"])
                    cmd_ad_wav.extend(
                        ["-ac", "1", "-c:a", "pcm_s16le", ad_da_wav])
                    subprocess.run(cmd_ad_wav)

                # Check for existing alignment plot first
                existing_plots = sorted(
                    glob.glob(
                        os.path.join(
                            dtw_plots_dir,
                            "*.txt")),
                    key=os.path.getmtime,
                    reverse=True)
                plot_file = None
                if existing_plots and os.path.getsize(existing_plots[0]) > 50:
                    plot_file = existing_plots[0]
                    print(
                        f"  Using existing alignment plot: {plot_file}",
                        flush=True)
                else:
                    cmd_da = [
                        sys.executable, da_script,
                        "--yes", "--stretch_audio",
                        ref_da_wav, ad_da_wav,
                        "--alignment_dir", dtw_plots_dir,
                        "--output_dir", dtw_out_dir
                    ]
                    print(
                        f"  Running DescribeAlign DTW (via uncompressed mono WAVs)...",
                        flush=True)
                    res_da = subprocess.run(
                        cmd_da, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    if res_da.returncode != 0:
                        print(
                            f"  DescribeAlign output:\n{
                                res_da.stderr.strip()}",
                            flush=True)

                    found_plots = sorted(
                        glob.glob(
                            os.path.join(
                                dtw_plots_dir,
                                "*.txt")),
                        key=os.path.getmtime,
                        reverse=True)
                    if found_plots:
                        plot_file = found_plots[0]

                if not plot_file or not os.path.exists(plot_file):
                    print(
                        f"  [ERROR] No alignment plot generated by DescribeAlign! Falling back to single-offset render.",
                        flush=True)
                    method = "cross_correlation"
                else:
                    print(f"  Alignment Plot Found: {plot_file}", flush=True)
                    try:
                        import dtw_smooth_stitch
                    except ImportError:
                        sys.path.insert(
                            0, os.path.dirname(
                                os.path.abspath(__file__)))
                        import dtw_smooth_stitch

                    print(
                        f"  Executing Multilayer Transition Smoothing (VAD + Zero-Crossing + 50ms Crossfade)...",
                        flush=True)
                    segments = dtw_smooth_stitch.parse_alignment_plot(
                        plot_file)

                    samples, sr = dtw_smooth_stitch.load_wav(ad_da_wav)
                    master_samples, _ = dtw_smooth_stitch.load_wav(ref_da_wav)
                    stitched = dtw_smooth_stitch.smooth_stitch_audio(
                        segments, samples, sr,
                        crossfade_ms=50, vad_search_ms=500, vad_threshold=0.02, silence_ms=200,
                        debug=debug, master_samples=master_samples
                    )

                    dtw_smooth_stitch.write_wav(stitched, sr, raw_stitched_wav)
                    sync_params[name]["describealign_segments"] = segments
                    sync_params[name]["engine_used"] = "describealign"
                    print(
                        f"  Stitched AD Audio Duration: {
                            len(stitched) / sr:.2f}s",
                        flush=True)

            # Master Audio Gap Filling: Splice in volume-matched Reference Master Audio into explicit excised scene voids
            explicit_gaps = []
            if engine_used == "adsync":
                cuts = extract_warp_cuts(sync_params[name], scratch_dir=scratch_dir, name=name, jump_threshold=3.0)
                if cuts:
                    # Check intro gap (if movie starts before first AD warp point)
                    if cuts[0]["vid_start"] >= 3.0 and cuts[0]["offset"] >= 3.0:
                        explicit_gaps.append((0.0, cuts[0]["vid_start"], cuts[0]["vid_start"]))

                    for i in range(len(cuts) - 1):
                        c1 = cuts[i]
                        c2 = cuts[i + 1]
                        g_start = c1["vid_end"]
                        g_end = c2["vid_start"]
                        dst_gap = g_end - g_start
                        offset_shift = c2["offset"] - c1["offset"]
                        # An excised scene void occurs when the video continues across a structural cut (offset jump >= 3.0s)
                        if dst_gap >= 3.0 and offset_shift >= 3.0:
                            explicit_gaps.append((g_start, g_end, dst_gap))

                    # Check outro gap (if movie ends after last AD warp point)
                    if (target_duration - cuts[-1]["vid_end"]) >= 3.0:
                        explicit_gaps.append((cuts[-1]["vid_end"], target_duration, target_duration - cuts[-1]["vid_end"]))
            elif engine_used == "describealign" and 'segments' in locals() and segments and len(segments) > 1:
                for i in range(len(segments) - 1):
                    g_start = segments[i].get("master_end", 0.0)
                    g_end = segments[i + 1].get("master_start", 0.0)
                    if g_end - g_start >= 3.0:
                        explicit_gaps.append((g_start, g_end, g_end - g_start))

            if not no_gap_fill and explicit_gaps and os.path.exists(raw_stitched_wav) and os.path.getsize(raw_stitched_wav) > 1000:
                _, ref_channels, _ = get_audio_info(ffprobe_bin, ref_path, ref_stream_map)
                fill_master_audio_gaps(
                    ffmpeg_bin=ffmpeg_bin,
                    ad_wav_path=raw_stitched_wav,
                    ref_path=ref_path,
                    ref_stream_map=ref_stream_map,
                    channels=ref_channels or 2,
                    scratch_dir=scratch_dir,
                    threads=threads,
                    explicit_gaps=explicit_gaps,
                    crossfade_ms=50,
                )
                sync_params[name]["explicit_gaps"] = explicit_gaps

            # Encode Standard Stereo and Night Mode deliverables from raw_stitched_wav
            if os.path.exists(raw_stitched_wav) and os.path.getsize(raw_stitched_wav) > 1000:
                print(
                    f"  Encoding Synced AD Stereo AAC 2.0 (alimiter)...",
                    flush=True)
                cmd_stereo = [
                    ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                        threads),
                    "-i", raw_stitched_wav,
                    "-af", "alimiter=level_in=1:level_out=0.98:limit=0.98",
                    "-ac", "2", "-ar", "48000",
                    "-c:a", "aac", "-b:a", "256k", out_stereo
                ]
                res_stereo = subprocess.run(
                    cmd_stereo, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                if res_stereo.returncode == 0:
                    print(f"  {Color.success('[SUCCESS]')} Saved: {Color.bold(out_stereo)}", flush=True)
                else:
                    print(
                        f"  {Color.err('[ERROR]')} Error rendering AD Stereo: {
                            res_stereo.stderr}", flush=True)

                if not no_night:
                    print(
                        f"  Encoding Synced AD Night Mode AAC 2.0 (loudnorm {target_i} LUFS)...",
                        flush=True)
                    cmd_night = [
                        ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                            threads),
                        "-i", raw_stitched_wav,
                        "-af", f"loudnorm=I={target_i}:TP={target_tp}:LRA=11",
                        "-ac", "2", "-ar", "48000",
                        "-c:a", "aac", "-b:a", "256k", out_night
                    ]
                    res_night = subprocess.run(
                        cmd_night, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    if res_night.returncode == 0:
                        print(
                            f"  {Color.success('[SUCCESS]')} Saved: {Color.bold(out_night)}", flush=True)
                    else:
                        print(
                            f"  {Color.err('[ERROR]')} Error rendering AD Night Mode: {
                                res_night.stderr}", flush=True)

                if not keep_scratch:
                    for sw in [raw_stitched_wav]:
                        if os.path.exists(sw):
                            try:
                                os.remove(sw)
                            except Exception:
                                pass

                    if video_mux_path and os.path.exists(video_mux_path):
                        out_mkv = os.path.join(
                            out_dir, f"{label_prefix} (Synced){suffix}.mkv")
                        mux_video_sample(
                            ffmpeg_bin,
                            video_mux_path,
                            out_stereo,
                            out_mkv,
                            render_duration)
                    continue

        # Standard Cross-Correlation Single Offset Path
        temp_wav = os.path.join(scratch_dir, f"{name}_synced_temp.wav")
        print(f"  Pre-rendering synced PCM WAV for [{name}]...", flush=True)

        wav_filters = ["asetpts=PTS-STARTPTS"]
        if is_pal:
            pal_rate = float(ad_sr) * (23.976023976 / 25.0)
            wav_filters.append(f"asetrate={pal_rate:.8f}")
            wav_filters.append("aresample=48000")
        else:
            if abs(atempo - 1.0) > 0.00001:
                wav_filters.append(f"atempo={atempo:.8f}")

        if offset > 0:
            wav_filters.append(f"atrim=start={offset:.3f}")
            wav_filters.append("asetpts=PTS-STARTPTS")
        elif offset < 0:
            delay_ms = int(abs(offset) * 1000)
            wav_filters.append(f"adelay={delay_ms}:all=1")

        cmd_wav = [
            ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal",
            "-threads", str(threads), "-fflags", "+genpts", "-i", path
        ]
        if preview_sec:
            cmd_wav.extend(["-t", f"{preview_sec:.3f}"])
        cmd_wav.extend([
            "-vn", "-af", ",".join(wav_filters),
            "-ac", "2", "-c:a", "pcm_s16le", temp_wav
        ])
        res_wav = subprocess.run(
            cmd_wav,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_wav.returncode != 0:
            print(
                f"  Error pre-rendering AD WAV: {res_wav.stderr}", flush=True)
            continue

        # Standard Stereo AAC from clean WAV
        print(f"  Encoding AD Stereo AAC 2.0 (alimiter)...", flush=True)
        cmd_stereo = [
            ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                threads),
            "-i", temp_wav,
            "-af", "alimiter=level_in=1:level_out=0.98:limit=0.98",
            "-ac", "2", "-ar", "48000",
            "-c:a", "aac", "-b:a", "256k", out_stereo
        ]
        res_stereo = subprocess.run(
            cmd_stereo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_stereo.returncode == 0:
            print(f"  [SUCCESS] Saved: {out_stereo}", flush=True)
        else:
            print(
                f"  Error rendering AD Stereo: {
                    res_stereo.stderr}",
                flush=True)

        # Night Mode AAC from clean WAV
        if not no_night:
            print(
                f"  Encoding AD Night Mode AAC 2.0 (loudnorm {target_i} LUFS)...",
                flush=True)
            cmd_night = [
                ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                    threads),
                "-i", temp_wav,
                "-af", f"loudnorm=I={target_i}:TP={target_tp}:LRA=11",
                "-ac", "2", "-ar", "48000",
                "-c:a", "aac", "-b:a", "256k", out_night
            ]
            res_night = subprocess.run(
                cmd_night,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True)
            if res_night.returncode == 0:
                print(f"  [SUCCESS] Saved: {out_night}", flush=True)
            else:
                print(
                    f"  Error rendering AD Night Mode: {
                        res_night.stderr}", flush=True)

        # Cleanup intermediate WAV
        if not keep_scratch and os.path.exists(temp_wav):
            try:
                os.remove(temp_wav)
            except Exception:
                pass

        if video_mux_path and os.path.exists(video_mux_path):
            out_mkv = os.path.join(
                out_dir, f"{label_prefix} (Synced){suffix}.mkv")
            mux_video_sample(
                ffmpeg_bin,
                video_mux_path,
                out_stereo,
                out_mkv,
                render_duration)


def auto_discover_inputs(directory):
    """Auto-discovers source reference MKV/audio and AD tracks in a movie directory."""
    ref_file = None
    ad_files = []

    # 1. Search for reference in source/ or top-level
    source_dir = os.path.join(directory, "source")
    search_dirs = [source_dir, directory] if os.path.exists(source_dir) else [
        directory]

    ref_exts = [
        "*.mkv",
        "*.dts",
        "*.dtshd",
        "*.ac3",
        "*.eac3",
        "*.thd",
        "*.mka",
        "*.flac",
        "*.wav"]
    for s_dir in search_dirs:
        for ext in ref_exts:
            matches = glob.glob(os.path.join(s_dir, ext))
            if matches:
                ref_file = matches[0]
                break
        if ref_file:
            break

    # 2. Search for AD tracks in tosync/ or top-level
    tosync_dir = os.path.join(directory, "tosync")
    ad_search_dirs = [tosync_dir, directory] if os.path.exists(tosync_dir) else [
        directory]

    ad_exts = [
        "*.mp3",
        "*.m4a",
        "*.aac",
        "*.ogg",
        "*.flac",
        "*.wav",
        "*.ac3",
        "*.eac3",
        "*.thd",
        "*.dts",
        "*.dtshd"]
    for a_dir in ad_search_dirs:
        for ext in ad_exts:
            for f in sorted(glob.glob(os.path.join(a_dir, ext))):
                if f != ref_file and not f.endswith(
                        "_mono8k.wav") and not f.endswith("_temp.wav"):
                    if f not in ad_files:
                        ad_files.append(f)

    return ref_file, ad_files


def process_single_movie(target_dir, args, ffmpeg_bin, ffprobe_bin):
    """Processes a single movie directory or specified reference and AD files."""
    ref_path = args.ref
    ad_input = args.ad

    if target_dir and os.path.isdir(target_dir):
        auto_ref, auto_ads = auto_discover_inputs(target_dir)
        if not ref_path:
            ref_path = auto_ref
        if not ad_input:
            ad_input = auto_ads

    if not ref_path or not os.path.exists(ref_path):
        print(f"Error: Reference file not found in '{target_dir or ref_path}'")
        return False

    if not ad_input:
        print(
            f"Error: No Audio Description tracks provided or found in '{target_dir}'.")
        return False

    ref_abs = os.path.abspath(ref_path)
    ref_parent = os.path.dirname(ref_abs)
    if os.path.basename(ref_parent).lower() in [
            "source", "tosync", "output", "scratch"]:
        project_dir = os.path.dirname(ref_parent)
    else:
        project_dir = target_dir if (
            target_dir and os.path.isdir(target_dir)) else ref_parent

    out_dir = args.outdir if args.outdir else os.path.join(
        project_dir, "output")
    scratch_dir = args.scratch_dir if args.scratch_dir else os.path.join(
        project_dir, "scratch")

    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(scratch_dir, exist_ok=True)

    base_title = args.title
    if not base_title:
        base_title = os.path.splitext(os.path.basename(ref_path))[0]
        base_title = re.sub(
            r'_(?:track\d+|DELAY.*)$',
            '',
            base_title,
            flags=re.IGNORECASE)
        if project_dir and len(os.path.basename(project_dir)) > 2:
            base_title = os.path.basename(project_dir)

    ad_tracks = {}
    for idx, item in enumerate(ad_input):
        if "=" in item:
            label, path = item.split("=", 1)
        else:
            path = item
            label = os.path.splitext(os.path.basename(path))[0]
        if not os.path.exists(path):
            print(f"Error: AD track file not found: '{path}'")
            return False
        ad_tracks[label.strip()] = path.strip()
    _, ref_stream_map = format_stream_selector(args.ref_stream)
    select_stream_probe, _ = format_stream_selector(args.ref_stream)
    target_duration, channels, ref_sr = get_audio_info(
        ffprobe_bin, ref_path, stream_selector=select_stream_probe)

    # Determine session file logging
    should_log = False
    if getattr(args, "no_log_to_file", False):
        should_log = False
    elif getattr(args, "log_to_file", False):
        should_log = True
    else:
        should_log = bool(CONFIG.get("log_to_file", False))

    tee_logger = None
    log_filepath = None
    if should_log:
        log_dir = getattr(args, "log_dir", None) or CONFIG.get("log_dir") or ""
        if not log_dir or not str(log_dir).strip():
            log_dir = project_dir or target_dir or os.getcwd()
        os.makedirs(log_dir, exist_ok=True)

        clean_title = re.sub(r'[^a-zA-Z0-9_-]', '_', base_title or "ADMaster").strip('_')[:35]
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        log_filename = f"{timestamp}_{clean_title}.log"
        log_filepath = os.path.join(log_dir, log_filename)

        tee_logger = TeeLogger(log_filepath)
        sys.stdout = tee_logger
        sys.stderr = tee_logger

    try:
        print(f"{Color.CYAN}================================================================================{Color.RESET}")
        print(f"{Color.bold('Reference Audio')} : {Color.GREEN}{ref_path}{Color.RESET}")
        print(
            f"{Color.bold('Stream Selection')}: {ref_stream_map} ({channels} Channels @ {ref_sr} Hz | Duration: {Color.bold(f'{target_duration:.2f}s')})")
        print(f"{Color.bold('Movie Title')}     : {Color.CYAN}{base_title}{Color.RESET}")
        print(f"{Color.bold('Project Dir')}     : {project_dir}")
        print(f"{Color.bold('Scratch Dir')}     : {scratch_dir}")
        print(f"{Color.bold('Output Dir')}      : {out_dir}")
        if tee_logger and log_filepath:
            print(f"{Color.bold('Session Log File')}: {Color.CYAN}{log_filepath}{Color.RESET}")
        print(f"{Color.bold('AD Tracks Count')} : {Color.MAGENTA}{len(ad_tracks)}{Color.RESET}")
        for name, p in ad_tracks.items():
            print(f"  - {Color.track(name)}: {os.path.basename(p)}")
        if args.convert_main:
            print(
                f"{Color.bold('Convert Main')}    : {Color.ok('YES')} (Surround AC3 + True 2.0 Stereo AAC + Night Mode AAC)")
        if args.force_dtw:
            print(f"{Color.bold('DTW Mode')}        : {Color.warn('FORCED')} ({CONFIG.get('default_warp_engine', 'adsync').upper()} + Multilayer smooth stitch)")
        if args.no_pal:
            print(f"{Color.bold('PAL Mode')}        : {Color.info('DISABLED')} (1.0x native speed enforced)")
        elif args.pal:
            print(
                f"{Color.bold('PAL Mode')}        : {Color.warn('FORCED')} (Dynamic sample-rate PAL 25fps -> 23.976fps slowdown)")
        if args.preview:
            print(f"{Color.bold('Preview Mode')}    : {Color.YELLOW}ON ({args.preview:.0f} seconds){Color.RESET}")
        print(
            f"{Color.bold('Overwrite Mode')}  : {
                Color.warn('ENABLED (--overwrite)') if args.overwrite else Color.ok('DISABLED (preserve existing files)')}")
        print(f"{Color.bold('FFmpeg Threads')}  : {args.threads}")
        print(f"{Color.CYAN}================================================================================{Color.RESET}")

        # Pre-check: If all output deliverables already exist and overwrite is
        # False, skip everything!
        render_duration = target_duration if not args.preview else min(
            args.preview, target_duration)
        suffix = f" (Preview {int(render_duration)}s)" if args.preview else ""
        expected_outputs = []

        if args.convert_main:
            if channels >= 6:
                expected_outputs.append(
                    os.path.join(
                        out_dir,
                        f"{base_title} (Surround){suffix}.ac3"))
            expected_outputs.append(
                os.path.join(
                    out_dir,
                    f"{base_title} (Stereo){suffix}.m4a"))
            if not getattr(args, "no_night", False):
                expected_outputs.append(
                    os.path.join(
                        out_dir,
                        f"{base_title} (Night Mode){suffix}.m4a"))

        for name in ad_tracks.keys():
            label_prefix = f"{base_title}-AD" if len(
                ad_tracks) == 1 else f"{base_title}-AD [{name}]"
            expected_outputs.append(
                os.path.join(
                    out_dir,
                    f"{label_prefix} (Stereo){suffix}.m4a"))
            if not getattr(args, "no_night", False):
                expected_outputs.append(
                    os.path.join(
                        out_dir,
                        f"{label_prefix} (Night Mode){suffix}.m4a"))
            if args.mux_video:
                expected_outputs.append(
                    os.path.join(
                        out_dir,
                        f"{base_title} (Sample Sync Preview {int(render_duration)}s).mkv"))

        all_exist = len(expected_outputs) > 0 and all(os.path.exists(
            p) and os.path.getsize(p) > 1000 for p in expected_outputs)

        if all_exist and not args.overwrite and not args.no_render:
            print(
                f"\n  [SKIP] All {len(expected_outputs)} requested deliverables already exist in output/:")
            for exp in expected_outputs:
                size_mb = os.path.getsize(exp) / (1024 * 1024)
                print(f"    [OK] {os.path.basename(exp)} ({size_mb:.1f} MB)")
            print(f"\n  Skipping correlation analysis and rendering.")
            print(f"  To force re-analyzing and re-rendering, pass --overwrite (or -f / --force-overwrite).")
            print(
                "================================================================================\n",
                flush=True)
            return True

        # Filter AD tracks that actually need to be aligned and rendered
        needed_ad_tracks = {}
        for name, p in ad_tracks.items():
            label_prefix = f"{base_title}-AD" if len(
                ad_tracks) == 1 else f"{base_title}-AD [{name}]"
            out_stereo = os.path.join(
                out_dir, f"{label_prefix} (Stereo){suffix}.m4a")
            out_night = os.path.join(
                out_dir, f"{label_prefix} (Night Mode){suffix}.m4a")
            out_mkv = os.path.join(
                out_dir, f"{base_title} (Sample Sync Preview {int(render_duration)}s).mkv") if args.mux_video else None

            stereo_done = os.path.exists(out_stereo) and os.path.getsize(
                out_stereo) > 1000 and not args.overwrite
            night_done = True if getattr(args, "no_night", False) else (os.path.exists(
                out_night) and os.path.getsize(out_night) > 1000 and not args.overwrite)
            mkv_done = (not args.mux_video) or (os.path.exists(out_mkv)
                                                and os.path.getsize(out_mkv) > 1000 and not args.overwrite)

            if not (stereo_done and night_done and mkv_done):
                needed_ad_tracks[name] = p
            else:
                print(
                    f"  [SKIP] All AD deliverables for [{name}] already exist. Skipping sync & render.",
                    flush=True)

        sync_params = {}
        if needed_ad_tracks:
            sync_params = compute_sync_parameters(
                ffmpeg_bin, ffprobe_bin, ref_path, needed_ad_tracks, target_duration, scratch_dir,
                ref_stream_map=ref_stream_map,
                num_anchors=args.anchors, force_stretch=args.force_stretch, no_stretch=args.no_stretch,
                min_confidence=args.min_confidence, force_dtw=args.force_dtw, no_dtw_fallback=args.no_dtw_fallback,
                force_pal=args.pal, no_pal=args.no_pal,
                no_multiband=getattr(args, "no_multiband", False),
                no_speed_detect=getattr(args, "no_speed_detect", False),
                debug=getattr(args, "debug", False)
            )
        else:
            print(
                f"\n  [INFO] All AD deliverables already exist. Skipping acoustic cross-correlation.",
                flush=True)

        if not args.no_render:
            if args.convert_main:
                render_main_tracks(
                    ffmpeg_bin, ref_path, base_title, out_dir, scratch_dir, channels,
                    stream_map=ref_stream_map, preview_sec=args.preview, threads=args.threads,
                    keep_scratch=args.keep_scratch, overwrite=args.overwrite, no_night=getattr(
                        args, "no_night", False)
                )

            if needed_ad_tracks:
                render_ad_tracks(
                    ffmpeg_bin, ffprobe_bin, ref_path, base_title, needed_ad_tracks, sync_params, target_duration,
                    out_dir, scratch_dir, ref_stream_map=ref_stream_map, target_i=args.target_i, target_tp=args.target_tp,
                    preview_sec=args.preview, video_mux_path=args.mux_video,
                    threads=args.threads, keep_scratch=args.keep_scratch, overwrite=args.overwrite, no_night=getattr(
                        args, "no_night", False), debug=getattr(args, "debug", False),
                    warp_engine=getattr(args, "warp_engine", "adsync"),
                    no_speed_detect=getattr(args, "no_speed_detect", False),
                    no_multiband=getattr(args, "no_multiband", False),
                    no_gap_fill=getattr(args, "no_gap_fill", False)
                )

        if not args.keep_scratch:
            try:
                for f in os.listdir(scratch_dir):
                    if f.endswith("_mono8k.wav") or f.endswith("_temp.wav"):
                        os.remove(os.path.join(scratch_dir, f))
            except Exception:
                pass

        print(f"\n{Color.CYAN}================================================================================{Color.RESET}")
        print(f"{Color.success('SUCCESS')}: Processing completed for: {Color.bold(base_title)}")
        print(f"Deliverables located in: {Color.cyan(out_dir)}")
        print(f"{Color.CYAN}--------------------------------------------------------------------------------{Color.RESET}")
        print(f"{Color.bold('FINAL SYNC SUMMARY:')}")

        if args.convert_main:
            print(f"  Main Audio     : {Color.ok('Processed')} (Surround / Stereo / Night Mode)")

        for name, params in sync_params.items():
            method = params.get("method", "cross_correlation")
            print(f"\n  AD Track {Color.track(name)}")
            method_str = Color.cyan("CROSS_CORRELATION") if method == "cross_correlation" else Color.warn("WARP / DTW")
            print(f"    Sync Method  : {method_str}")

            if method == "cross_correlation":
                conf_val = params.get('confidence', 0)
                conf_c = Color.GREEN if conf_val >= 0.20 else (Color.YELLOW if conf_val >= 0.10 else Color.RED)
                offset_val = params.get('offset', 0.0)
                print(f"    Global Offset: {Color.bold(f'{offset_val:+.3f}s')}")
                print(f"    Time Stretch : {params['atempo']:.5f}x")
                print(f"    PAL Slowdown : {'Yes' if params.get('is_pal') else 'No'}")
                print(f"    Confidence   : {conf_c}{conf_val * 100:.1f}%{Color.RESET}")
            elif method == "dtw":
                engine_str = params.get("engine_used", getattr(args, "warp_engine", "adsync")).upper()
                print(f"    Engine       : {Color.bold(engine_str)}")

                # Check for ADSync report metrics
                report = params.get("report")
                if not report and scratch_dir and name:
                    clean_name = re.sub(r'[^a-zA-Z0-9_-]', '_', name)
                    report_json_path = os.path.join(scratch_dir, "adsync", clean_name, "report.json")
                    if os.path.exists(report_json_path):
                        try:
                            with open(report_json_path, "r", encoding="utf-8") as f:
                                data = json.load(f)
                            from adsync.models import SyncReport
                            report = SyncReport(**data)
                        except Exception:
                            pass

                if report:
                    conf_val = getattr(report, "confidence", 0.0)
                    conf_c = Color.GREEN if conf_val >= 0.85 else (Color.YELLOW if conf_val >= 0.60 else Color.RED)
                    print(f"    Confidence   : {conf_c}{conf_val * 100:.1f}%{Color.RESET}")
                    if getattr(report, "speed_stretch", None) and abs(report.speed_stretch - 1.0) > 0.001:
                        print(f"    Speed Stretch: {report.speed_stretch:.6f}x ({(report.speed_stretch - 1) * 100:+.2f}%)")
                    if getattr(report, "global_offset", None) is not None:
                        print(f"    Global Offset: {Color.bold(f'{report.global_offset:+.3f}s')}")

                gaps = params.get("explicit_gaps", [])
                if gaps:
                    total_gap_sec = sum(g[2] for g in gaps)
                    m_g, s_g = divmod(int(total_gap_sec), 60)
                    h_g, m_g = divmod(m_g, 60)
                    dur_str = f"{h_g:02d}:{m_g:02d}:{s_g:02d}" if h_g else f"{m_g:02d}:{s_g:02d}"
                    print(f"    {Color.warn('[NOTICE]')} AD Track has {len(gaps)} excised scene void(s) ({dur_str} missing from AD) -> Spliced with Reference Master Audio.")

                cuts = extract_warp_cuts(params, scratch_dir=scratch_dir, name=name)
                if cuts:
                    print(f"    Warp Segments: {Color.bold(str(len(cuts)))} continuous blocks")
                    significant_cuts = [c for c in cuts if c["is_initial"] or abs(c["jump"]) > 0.03]
                    if len(significant_cuts) > 1:
                        print(f"    {Color.YELLOW}Double-Check Cut Times (where AD track shifts):{Color.RESET}")
                        max_display = 25
                        for c in significant_cuts[:max_display]:
                            t_str = format_time_code(c["vid_start"])
                            off_str = f"{c['offset']:+.3f}s"
                            jump_str = f"{c['jump']:+.3f}s"
                            if c["is_initial"]:
                                print(f"      - Seg 1 starts at {Color.CYAN}{t_str}{Color.RESET} (initial offset: {Color.bold(off_str)})")
                            else:
                                print(f"      - Seg {c['index']} jumps at {Color.CYAN}{t_str}{Color.RESET} (offset: {Color.bold(off_str)}, shift: {Color.warn(jump_str)})")
                        if len(significant_cuts) > max_display:
                            print(f"      ... and {len(significant_cuts) - max_display} more cut transition(s)")

        print(f"{Color.CYAN}================================================================================{Color.RESET}\n")
        return True
    finally:
        if tee_logger:
            sys.stdout = tee_logger.terminal
            sys.stderr = tee_logger.terminal
            tee_logger.close()


def main():
    parser = argparse.ArgumentParser(
        description="""
================================================================================
ADMaster -- Universal Audio Description & Secondary Audio Sync & Encoding Tool
================================================================================
Acoustic cross-correlation, Multiband narration suppression, Automatic PAL speed probing,
Native ADSync Viterbi DP + PCHIP Warp / DescribeAlign DTW with Multilayer smoothing
(VAD + Zero-Crossing + 50ms Crossfade), ITU-R BS.775 2.0 downmixing, EBU R128 loudness
normalization, and MKV sample multiplexing.
""",
        formatter_class=argparse.RawTextHelpFormatter
    )
    parser.add_argument(
        "movie_dir",
        nargs="?",
        default=None,
        help="Directory containing 'source/' and 'tosync/' subfolders for auto-discovery (or parent queue folder with --batch)")
    parser.add_argument(
        "-d",
        "--dir",
        dest="dir_opt",
        default=None,
        help="Alias for movie_dir")
    parser.add_argument(
        "-b",
        "--batch",
        action="store_true",
        help="Batch mode: Process all subdirectories inside movie_dir sequentially")
    parser.add_argument(
        "-r",
        "--ref",
        default=None,
        help="Path to reference audio/video file (MKV, DTS, AC3, etc.)")
    parser.add_argument(
        "--ad",
        nargs="+",
        default=None,
        help="Path to one or more AD audio files")
    parser.add_argument(
        "--ref-stream",
        default="0:a:0",
        help="Reference audio stream index or map (e.g. '0:a:0', '0:a:1', '1'). Default: 0:a:0")
    parser.add_argument(
        "-m",
        "--mux-video",
        default=None,
        help="Path to reference MKV/MP4 for multiplexing preview video sample")
    parser.add_argument(
        "-o",
        "--out",
        "--outdir",
        dest="outdir",
        default=None,
        help="Output directory (Defaults to <movie_dir>/output)")
    parser.add_argument(
        "-t",
        "--title",
        default=None,
        help="Movie title prefix for output filenames")
    parser.add_argument(
        "--preview",
        type=float,
        default=None,
        help="Render only the first N seconds for rapid sync verification")
    parser.add_argument(
        "-f",
        "--force-overwrite",
        "--overwrite",
        dest="overwrite",
        action="store_true",
        default=CONFIG.get(
            "default_overwrite",
            False),
        help="Overwrite existing output deliverables in output/ (Default: False, skips existing files)")
    parser.add_argument(
        "--convert-main",
        action="store_true",
        help="Also convert and downmix reference main audio (Surround AC3, True 2.0 Stereo AAC, Night Mode AAC)")
    parser.add_argument(
        "--no-night",
        action="store_true",
        help="Disable generating Night Mode (loudnorm) AAC variants")
    parser.add_argument(
        "--no-render",
        action="store_true",
        help="Calculate sync parameters only, do not render output files")
    parser.add_argument(
        "--anchors",
        type=int,
        default=CONFIG.get(
            "default_anchors",
            100),
        help="Number of correlation analysis anchors across timeline (Default: 100)")
    parser.add_argument(
        "--no-stretch",
        action="store_true",
        help="Force native 1.0x speed rate without speed stretch")
    parser.add_argument(
        "--force-stretch",
        action="store_true",
        help="Force 2-point closing cross-correlation speed stretch search")
    parser.add_argument(
        "--pal",
        action="store_true",
        help="Explicitly apply dynamic sample-rate PAL 25fps -> 23.976fps slowdown")
    parser.add_argument(
        "--no-pal",
        action="store_true",
        help="Force native 1.0x film speed (disables automatic PAL slowdown even if 'uk' is in filename)")
    parser.add_argument(
        "--min-confidence",
        type=float,
        default=0.15,
        help="Minimum peak confidence threshold before triggering DTW fallback (Default: 0.15 / 15%%)")
    parser.add_argument(
        "--force-dtw",
        action="store_true",
        help="Force warp/DTW alignment instead of cross-correlation")
    parser.add_argument(
        "--no-dtw-fallback",
        action="store_true",
        help="Disable automatic warp/DTW fallback on low confidence")
    parser.add_argument(
        "--warp-engine",
        choices=["adsync", "describealign"],
        default=CONFIG.get("default_warp_engine", "adsync"),
        help="Warp engine for non-linear / structural edits (Default: adsync - Viterbi DP + PCHIP; fallback: describealign)")
    parser.add_argument(
        "--no-speed-detect",
        action="store_true",
        help="Disable automatic acoustic PAL speed detection probe")
    parser.add_argument(
        "--no-multiband",
        action="store_true",
        help="Disable multiband narration-suppression in cross-correlation (use full-band)")
    parser.add_argument(
        "--no-gap-fill",
        action="store_true",
        help="Disable automatic Master Audio gap filling during deleted scenes / silence voids")
    parser.add_argument(
        "--target-i",
        type=float,
        default=CONFIG.get(
            "default_target_i",
            -16.0),
        help="Target EBU R128 integrated loudness in LUFS (Default: -16.0)")
    parser.add_argument(
        "--target-tp",
        type=float,
        default=CONFIG.get(
            "default_target_tp",
            -1.5),
        help="Target true-peak ceiling in dB (Default: -1.5)")
    parser.add_argument(
        "--threads",
        type=int,
        default=CONFIG.get(
            "default_threads",
            2),
        help="Number of CPU threads for FFmpeg encoding (Default: 2)")
    parser.add_argument(
        "--scratch-dir",
        default=None,
        help="Directory for temporary analysis files (Defaults to <movie_dir>/scratch)")
    parser.add_argument(
        "--keep-scratch",
        action="store_true",
        help="Keep temporary WAV files in scratch folder for debugging"
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Enable verbose debug logging (useful for troubleshooting DTW mapping and cut points)"
    )
    parser.add_argument(
        "--ffmpeg",
        default=DEFAULT_FFMPEG,
        help="Path to ffmpeg executable")
    parser.add_argument(
        "--log-to-file",
        action="store_true",
        default=None,
        help="Enable writing session output log to a timestamped file")
    parser.add_argument(
        "--no-log-to-file",
        action="store_true",
        default=False,
        help="Disable writing session output log to file")
    parser.add_argument(
        "--log-dir",
        default=None,
        help="Directory path where run logs are stored (Defaults to config or target movie directory)")

    args = parser.parse_args()
    args.debug = args.debug or CONFIG.get("debug_mode", False)
    
    if args.debug:
        logging.getLogger("adsync").setLevel(logging.DEBUG)
    else:
        logging.getLogger("adsync").setLevel(logging.ERROR)

    ffmpeg_bin, ffprobe_bin = locate_binaries(args.ffmpeg)

    target_dir = args.dir_opt or args.movie_dir

    # Check for batch queue mode
    if target_dir and os.path.isdir(target_dir) and (
            args.batch or not os.path.exists(os.path.join(target_dir, "source"))):
        subdirs = [
            os.path.join(target_dir, d) for d in os.listdir(target_dir)
            if os.path.isdir(os.path.join(target_dir, d)) and not d.lower() in ["scratch", "output", "bin"]
        ]
        # Filter subdirectories that have source or tosync
        movie_dirs = [
            d for d in subdirs if os.path.exists(
                os.path.join(
                    d, "source")) or os.path.exists(
                os.path.join(
                    d, "tosync"))]

        if movie_dirs:
            print(
                f"=== BATCH QUEUE MODE: Found {
                    len(movie_dirs)} movie folders in '{target_dir}' ===")
            for i, m_dir in enumerate(movie_dirs, 1):
                print(f"\n[{i}/{len(movie_dirs)}] Processing folder: {m_dir}")
                process_single_movie(m_dir, args, ffmpeg_bin, ffprobe_bin)
            print(
                "\n================================================================================")
            print("ALL BATCH QUEUE JOBS COMPLETED SUCCESSFULLY!")
            print(
                "================================================================================")
            return

    # Single movie process
    process_single_movie(target_dir, args, ffmpeg_bin, ffprobe_bin)


if __name__ == "__main__":
    main()
