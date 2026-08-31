# SPDX-License-Identifier: MIT
import subprocess
import re
import argparse
from datetime import timedelta

import json
import os
import shutil


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


FFMPEG_PATH = locate_tool("ffmpeg")
MKVMERGE_PATH = locate_tool("mkvmerge")


def format_time(seconds):
    td = timedelta(seconds=float(seconds))
    # mkvmerge format: HH:MM:SS.nnnnnn
    hours, remainder = divmod(td.seconds, 3600)
    minutes, seconds_int = divmod(remainder, 60)
    microseconds = td.microseconds
    return f"{hours:02d}:{minutes:02d}:{seconds_int:02d}.{microseconds:06d}"


def detect_commercial_breaks(mkv_file, black_duration=2.0, silence_duration=2.0,
                             black_threshold=0.1, silence_threshold="-40dB", safe_buffer=2.0):
    print(f"Scanning {mkv_file} for commercial breaks...")
    cmd = [
        FFMPEG_PATH,
        "-i", mkv_file,
        "-vf", f"blackdetect=d={black_duration}:pix_th={black_threshold}",
        "-af", f"silencedetect=noise={silence_threshold}:d={silence_duration}",
        "-f", "null", "-"
    ]

    proc = subprocess.Popen(
        cmd,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8")

    black_regions = []
    silence_regions = []

    for line in proc.stderr:
        line = line.strip()
        # Parse blackdetect
        if "black_start:" in line:
            m = re.search(
                r"black_start:([0-9\.]+) black_end:([0-9\.]+) black_duration:", line)
            if m:
                black_regions.append((float(m.group(1)), float(m.group(2))))
        # Parse silencedetect
        elif "silence_start:" in line:
            m = re.search(r"silence_start:\s*([0-9\.]+)", line)
            if m:
                start = float(m.group(1))
                # Will wait for silence_end
                silence_regions.append([start, None])
        elif "silence_end:" in line:
            m = re.search(r"silence_end:\s*([0-9\.]+)", line)
            if m and silence_regions and silence_regions[-1][1] is None:
                silence_regions[-1][1] = float(m.group(1))

    # Find intersections where BOTH black and silence occur
    commercial_breaks = []
    for b_start, b_end in black_regions:
        for s_start, s_end in silence_regions:
            if s_end is None:
                continue

            # Check overlap
            overlap_start = max(b_start, s_start)
            overlap_end = min(b_end, s_end)

            if overlap_end > overlap_start:
                duration = overlap_end - overlap_start
                print(
                    f"Detected raw commercial break: {
                        overlap_start:.3f}s to {
                        overlap_end:.3f}s (Duration: {
                        duration:.3f}s)")

                # Apply the safe buffer logic!
                if duration > (safe_buffer * 2):
                    safe_start = overlap_start + safe_buffer
                    safe_end = overlap_end - safe_buffer
                    print(
                        f" -> Applying {safe_buffer}s safe buffer. New cut window: {
                            safe_start:.3f}s to {
                            safe_end:.3f}s")
                    commercial_breaks.append((safe_start, safe_end))
                else:
                    print(
                        f" -> Break too short for safe buffer! Keeping original window.")
                    commercial_breaks.append((overlap_start, overlap_end))

    return commercial_breaks


def remove_breaks(mkv_file, out_file, breaks):
    if not breaks:
        print("No commercial breaks found to remove.")
        return

    # Sort breaks by start time
    breaks.sort(key=lambda x: x[0])

    parts = []
    current_time = 0.0

    for b_start, b_end in breaks:
        parts.append(f"{format_time(current_time)}-{format_time(b_start)}")
        current_time = b_end

    # Add final part
    parts.append(f"{format_time(current_time)}-")

    parts_arg = "parts:" + ",+".join(parts)
    print(f"\nmkvmerge split instruction: {parts_arg}")

    cmd = [
        MKVMERGE_PATH,
        "-o", out_file,
        "--split", parts_arg,
        mkv_file
    ]

    print("\nExecuting MKVMerge...")
    subprocess.run(cmd)
    print(f"\nDone! Cleaned MKV saved to: {out_file}")


def main():
    parser = argparse.ArgumentParser(
        description="Remove black/silent commercial breaks from MKV losslessly.")
    parser.add_argument("input_mkv", help="Path to the input MKV file")
    parser.add_argument("output_mkv", help="Path to the output MKV file")
    parser.add_argument(
        "--safe-buffer",
        type=float,
        default=2.0,
        help="Number of seconds to buffer inside the commercial break to prevent keyframe deletion of the show.")
    args = parser.parse_args()

    breaks = detect_commercial_breaks(
        args.input_mkv, safe_buffer=args.safe_buffer)
    remove_breaks(args.input_mkv, args.output_mkv, breaks)


if __name__ == "__main__":
    main()
