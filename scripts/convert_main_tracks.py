# SPDX-License-Identifier: MIT
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys

DEFAULT_FFMPEG = "ffmpeg"
DEFAULT_FFPROBE = "ffprobe"


def locate_binaries(custom_ffmpeg=None):
    ffmpeg_bin = custom_ffmpeg or DEFAULT_FFMPEG
    ffprobe_bin = DEFAULT_FFPROBE

    if shutil.which(ffmpeg_bin):
        ffmpeg_bin = shutil.which(ffmpeg_bin)
    if shutil.which(ffprobe_bin):
        ffprobe_bin = shutil.which(ffprobe_bin)

    return ffmpeg_bin, ffprobe_bin


def get_audio_info(ffprobe_bin, file_path):
    cmd = [
        ffprobe_bin, "-v", "error", "-select_streams", "a:0",
        "-show_entries", "format=duration:stream=channels,channel_layout,duration",
        "-of", "json",
        file_path
    ]
    channels = 2
    duration = 0.0
    try:
        res = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=True)
        data = json.loads(res.stdout)

        # Channel count from first audio stream
        if "streams" in data and len(data["streams"]) > 0:
            channels = int(data["streams"][0].get("channels", 2))
            try:
                duration = float(data["streams"][0].get("duration", 0.0))
            except (ValueError, TypeError):
                pass

        # Format duration fallback
        if duration <= 0.0:
            try:
                duration = float(data.get("format", {}).get("duration", 0.0))
            except (ValueError, TypeError):
                pass

        return duration, channels
    except Exception as e:
        print(f"  Warning: Could not probe audio info for {file_path}: {e}")
        return 0.0, channels


def sanitize_title(filename):
    base = os.path.splitext(os.path.basename(filename))[0]
    base = re.sub(r'_(?:track\d+|DELAY.*)$', '', base, flags=re.IGNORECASE)
    # Convert dotted scene names to Title (Year) if present
    m = re.match(r'^([A-Za-z0-9\._\s]+?)[\.\s_]((?:19|20)\d{2})[\.\s_]', base)
    if m:
        name_part = m.group(1).replace(".", " ").replace("_", " ").strip()
        year_part = m.group(2)
        return f"{name_part} ({year_part})"
    return base.replace(".", " ").replace("_", " ").strip()


def convert_track(ffmpeg_bin, ffprobe_bin, input_path, base_title,
                  out_dir, scratch_dir, threads=2, preview_sec=None, overwrite=False):
    """Processes a single multi-channel track into Surround AC3, Stereo AAC, and Night Mode AAC."""
    print(f"\n================================================================================")
    print(f"Processing Track : {input_path}")
    print(f"Title Prefix     : {base_title}")
    print(f"Output Dir       : {out_dir}")
    print(f"Scratch Dir      : {scratch_dir}")
    print(f"Threads          : {threads}")
    print(
        f"Overwrite Mode   : {
            'ENABLED (--overwrite)' if overwrite else 'DISABLED (preserve existing files)'}")
    print(f"================================================================================")

    suffix = f" (Preview {int(preview_sec)}s)" if preview_sec else ""
    pan_stereo = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"

    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(scratch_dir, exist_ok=True)

    _, channels = get_audio_info(ffprobe_bin, input_path)

    out_surround = os.path.join(
        out_dir, f"{base_title} (Surround){suffix}.ac3") if channels >= 6 else None
    out_stereo = os.path.join(out_dir, f"{base_title} (Stereo){suffix}.m4a")
    out_night = os.path.join(out_dir, f"{base_title} (Night Mode){suffix}.m4a")

    # 1. 5.1 Surround AC3 (if 6+ channels)
    if channels >= 6:
        if os.path.exists(out_surround) and os.path.getsize(
                out_surround) > 1000 and not overwrite:
            print(
                f"\n  [SKIP] 5.1 Surround AC3 already exists: {
                    os.path.basename(out_surround)} (use --overwrite to force)",
                flush=True)
        else:
            print(f"\n  [1/3] Encoding 5.1 Surround AC3 (640k)...", flush=True)
            cmd_surround = [
                ffmpeg_bin,
                "-y",
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-threads",
                str(threads),
                "-i",
                input_path]
            if preview_sec:
                cmd_surround.extend(["-t", f"{preview_sec:.3f}"])
            cmd_surround.extend(["-c:a", "ac3", "-b:a", "640k", out_surround])
            res_surr = subprocess.run(
                cmd_surround,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True)
            if res_surr.returncode == 0 and os.path.exists(out_surround):
                size_mb = os.path.getsize(out_surround) / (1024 * 1024)
                print(
                    f"  [SUCCESS] Saved Surround AC3: {out_surround} ({
                        size_mb:.1f} MB)", flush=True)
            else:
                print(
                    f"  [ERROR] Surround encode failed: {
                        res_surr.stderr}", flush=True)
    else:
        print(
            f"\n  [INFO] Source has {channels} channels (< 6). Skipping Surround AC3 encode.",
            flush=True)

    stereo_exists = os.path.exists(
        out_stereo) and os.path.getsize(out_stereo) > 1000
    night_exists = os.path.exists(
        out_night) and os.path.getsize(out_night) > 1000

    if stereo_exists and night_exists and not overwrite:
        print(
            f"\n  [SKIP] Main Stereo & Night Mode AAC already exist in output/ (use --overwrite to force re-render):",
            flush=True)
        print(
            f"    [OK] {
                os.path.basename(out_stereo)} ({
                os.path.getsize(out_stereo) /
                (
                    1024 *
                    1024):.1f} MB)",
            flush=True)
        print(
            f"    [OK] {
                os.path.basename(out_night)} ({
                os.path.getsize(out_night) /
                (
                    1024 *
                    1024):.1f} MB)",
            flush=True)
        return

    # 2. Extract intermediate stereo WAV in scratch
    stereo_wav = os.path.join(
        scratch_dir, f"{
            re.sub(
                r'[^a-zA-Z0-9]', '_', base_title)}_stereo_temp.wav")
    print(
        f"\n  [2/3] Extracting intermediate 48kHz Stereo WAV for downmixing...",
        flush=True)
    cmd_wav = [
        ffmpeg_bin,
        "-y",
        "-hide_banner",
        "-loglevel",
        "fatal",
        "-threads",
        str(threads),
        "-fflags",
        "+genpts",
        "-i",
        input_path]
    if preview_sec:
        cmd_wav.extend(["-t", f"{preview_sec:.3f}"])
    if channels >= 6:
        cmd_wav.extend(["-af",
                        f"asetpts=PTS-STARTPTS,aresample=48000,{pan_stereo}",
                        "-c:a",
                        "pcm_s16le",
                        stereo_wav])
    else:
        cmd_wav.extend(["-af",
                        "asetpts=PTS-STARTPTS,aresample=48000",
                        "-c:a",
                        "pcm_s16le",
                        stereo_wav])

    res_wav = subprocess.run(
        cmd_wav,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    if res_wav.returncode != 0 or not os.path.exists(stereo_wav):
        print(
            f"  [ERROR] Intermediate WAV extraction failed: {
                res_wav.stderr}",
            flush=True)
        return
    print(
        f"  [SUCCESS] Extracted Stereo WAV: {stereo_wav} ({
            os.path.getsize(stereo_wav) / (
                1024 * 1024):.1f} MB)",
        flush=True)

    # 3. Standard Stereo AAC from stereo WAV
    if stereo_exists and not overwrite:
        print(
            f"\n  [SKIP] Main Stereo AAC already exists: {
                os.path.basename(out_stereo)}",
            flush=True)
    else:
        print(f"\n  [3/3] Encoding Main Stereo AAC (alimiter)...", flush=True)
        cmd_stereo = [
            ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                threads),
            "-i", stereo_wav,
            "-af", "alimiter=level_in=1:level_out=0.98:limit=0.98",
            "-c:a", "aac", "-b:a", "256k", out_stereo
        ]
        res_stereo = subprocess.run(
            cmd_stereo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_stereo.returncode == 0 and os.path.exists(out_stereo):
            size_mb = os.path.getsize(out_stereo) / (1024 * 1024)
            print(
                f"  [SUCCESS] Saved Stereo AAC: {out_stereo} ({
                    size_mb:.1f} MB)", flush=True)
        else:
            print(
                f"  [ERROR] Stereo encode failed: {
                    res_stereo.stderr}",
                flush=True)

    # 4. Night Mode AAC from stereo WAV
    if night_exists and not overwrite:
        print(
            f"\n  [SKIP] Main Night Mode AAC already exists: {
                os.path.basename(out_night)}",
            flush=True)
    else:
        print(
            f"\n  [4/3] Encoding Main Night Mode AAC (loudnorm -16 LUFS)...",
            flush=True)
        cmd_night = [
            ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "fatal", "-threads", str(
                threads),
            "-i", stereo_wav,
            "-af", "loudnorm=I=-16.0:TP=-1.5:LRA=11", "-ar", "48000",
            "-c:a", "aac", "-b:a", "256k", out_night
        ]
        res_night = subprocess.run(
            cmd_night,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True)
        if res_night.returncode == 0 and os.path.exists(out_night):
            size_mb = os.path.getsize(out_night) / (1024 * 1024)
            print(
                f"  [SUCCESS] Saved Night Mode AAC: {out_night} ({
                    size_mb:.1f} MB)", flush=True)
        else:
            print(
                f"  [ERROR] Night Mode encode failed: {
                    res_night.stderr}",
                flush=True)

    # 5. Clean up intermediate WAV
    if os.path.exists(stereo_wav):
        try:
            os.remove(stereo_wav)
            print(f"  [CLEANUP] Removed temporary stereo WAV.", flush=True)
        except Exception:
            pass


def main():
    parser = argparse.ArgumentParser(
        description="Convert Audio Tracks to Surround AC3, Stereo AAC, and Night Mode AAC")
    parser.add_argument(
        "-i",
        "--inputs",
        nargs="+",
        help="Input file path(s) or glob pattern(s)")
    parser.add_argument(
        "-d",
        "--dir",
        default=None,
        help="Directory containing audio tracks to process")
    parser.add_argument(
        "-t",
        "--titles",
        nargs="+",
        default=None,
        help="Optional movie title prefix(es) corresponding to inputs")
    parser.add_argument(
        "-o",
        "--outdir",
        default=None,
        help="Output directory (Defaults to <parent_dir>/output)")
    parser.add_argument(
        "-f",
        "--force-overwrite",
        "--overwrite",
        dest="overwrite",
        action="store_true",
        default=False,
        help="Overwrite existing output deliverables in output/ (Default: False, skips existing files)")
    parser.add_argument(
        "--threads",
        type=int,
        default=2,
        help="Number of CPU threads for FFmpeg encoding (Default: 2)")
    parser.add_argument(
        "--preview",
        type=float,
        default=None,
        help="Render preview sample duration in seconds")
    parser.add_argument(
        "--ffmpeg",
        default=DEFAULT_FFMPEG,
        help="Path to ffmpeg executable")

    args = parser.parse_args()
    ffmpeg_bin, ffprobe_bin = locate_binaries(args.ffmpeg)

    input_files = []
    if args.inputs:
        for item in args.inputs:
            matches = glob.glob(item)
            if matches:
                input_files.extend(matches)
            elif os.path.exists(item):
                input_files.append(item)
    elif args.dir:
        exts = [
            "*.dts",
            "*.ac3",
            "*.eac3",
            "*.mka",
            "*.flac",
            "*.wav",
            "*.mkv",
            "*.mp4"]
        for ext in exts:
            input_files.extend(glob.glob(os.path.join(args.dir, ext)))

    if not input_files:
        print("Error: No valid input files specified or found.")
        sys.exit(1)

    print(f"Found {len(input_files)} file(s) to process.")
    for idx, fpath in enumerate(input_files):
        parent_dir = os.path.dirname(os.path.abspath(fpath))
        if os.path.basename(parent_dir).lower() in [
                "source", "tosync", "output", "scratch"]:
            proj_dir = os.path.dirname(parent_dir)
        else:
            proj_dir = parent_dir

        out_dir = args.outdir if args.outdir else os.path.join(
            proj_dir, "output")
        scratch_dir = os.path.join(proj_dir, "scratch")

        if args.titles and idx < len(args.titles):
            title = args.titles[idx]
        else:
            title = sanitize_title(fpath)

        convert_track(
            ffmpeg_bin=ffmpeg_bin,
            ffprobe_bin=ffprobe_bin,
            input_path=fpath,
            base_title=title,
            out_dir=out_dir,
            scratch_dir=scratch_dir,
            threads=args.threads,
            preview_sec=args.preview,
            overwrite=args.overwrite
        )

    print("\n================================================================================")
    print("SUCCESS: ALL CONVERSIONS COMPLETED!")
    print("================================================================================")


if __name__ == "__main__":
    main()
