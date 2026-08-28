# SPDX-License-Identifier: MIT
import argparse
import json
import os
import re
import subprocess
import sys

DEFAULT_FFMPEG = os.path.join(
    os.path.dirname(
        os.path.abspath(__file__)),
    "bin",
    "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"


def run_loudnorm_2pass(ffmpeg_bin, input_path, base_filters,
                       target_i, target_tp, target_lra, out_path, overwrite=False):
    if os.path.exists(out_path) and os.path.getsize(
            out_path) > 1000 and not overwrite:
        print(
            f"  [SKIP] Output file already exists: {
                os.path.basename(out_path)} (use --overwrite to force re-render)")
        return

    print(
        f"\n--- Pass 1: Measuring Loudness for {os.path.basename(out_path)} ---")
    pass1_filters = f"asetpts=PTS-STARTPTS,{base_filters},loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:print_format=json"

    cmd_pass1 = [
        ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "fatal", "-fflags", "+genpts", "-i", input_path,
        "-af", pass1_filters,
        "-f", "null", "-"
    ]

    res1 = subprocess.run(
        cmd_pass1,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True)
    json_pattern = r'\{[^{}]*"input_i"[^{}]*\}'
    json_match = re.search(json_pattern, res1.stderr, re.DOTALL)

    if not json_match:
        print("  WARNING: Pass 1 failed to output JSON. Falling back to 1-pass loudnorm.")
        cmd_fallback = [
            ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "fatal", "-fflags", "+genpts", "-i", input_path,
            "-af", f"asetpts=PTS-STARTPTS,{base_filters},loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}",
            "-c:a", "aac", "-b:a", "256k", "-ar", "48000", out_path
        ]
        subprocess.run(
            cmd_fallback,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        return

    stats = json.loads(json_match.group(0))
    print(
        f"  Measured: I={
            stats['input_i']} TP={
            stats['input_tp']} LRA={
                stats['input_lra']}")

    print(f"--- Pass 2: Applying Linear Gain ---")
    loudnorm_p2 = (
        f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:"
        f"measured_I={stats['input_i']}:measured_TP={stats['input_tp']}:"
        f"measured_LRA={
            stats['input_lra']}:measured_thresh={
            stats['input_thresh']}:linear=true"
    )
    pass2_filters = f"asetpts=PTS-STARTPTS,{base_filters},{loudnorm_p2}"

    cmd_pass2 = [
        ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "fatal", "-fflags", "+genpts", "-i", input_path,
        "-af", pass2_filters,
        "-c:a", "aac", "-b:a", "256k", "-ar", "48000", out_path
    ]

    subprocess.run(cmd_pass2, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    print(f"  Saved: {out_path}")


def main():
    parser = argparse.ArgumentParser(
        description="Standalone 2-pass AAC converter (Standard & NightMode)")
    parser.add_argument("input", help="Input audio file (e.g., 5.1 AC3)")
    parser.add_argument(
        "-f",
        "--force-overwrite",
        "--overwrite",
        dest="overwrite",
        action="store_true",
        default=False,
        help="Overwrite existing output deliverables (Default: False, skips existing files)")
    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: File not found: {args.input}")
        sys.exit(1)

    input_dir = os.path.dirname(args.input)
    base_name = os.path.splitext(os.path.basename(args.input))[0]

    out_std = os.path.join(input_dir, f"{base_name} (Stereo Standard).m4a")
    out_night = os.path.join(input_dir, f"{base_name} (Stereo NightMode).m4a")

    std_filters = "aresample=48000,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"
    night_filters = "aresample=48000,pan=stereo|c0=0.5*c0+1.0*c2+0.5*c4|c1=0.5*c1+1.0*c2+0.5*c5,acompressor=threshold=-21dB:ratio=3.5:attack=10:release=100"

    print(f"Processing: {args.input}")
    run_loudnorm_2pass(
        DEFAULT_FFMPEG,
        args.input,
        std_filters,
        target_i=-16.0,
        target_tp=-1.5,
        target_lra=11,
        out_path=out_std,
        overwrite=args.overwrite)
    run_loudnorm_2pass(
        DEFAULT_FFMPEG,
        args.input,
        night_filters,
        target_i=-16.0,
        target_tp=-1.5,
        target_lra=7,
        out_path=out_night,
        overwrite=args.overwrite)
    print("Done!")


if __name__ == "__main__":
    main()
