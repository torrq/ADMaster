# SPDX-License-Identifier: MIT
import os
import shutil
import glob
import re
import subprocess
import argparse
import sys

# Paths
SOURCE_DIR = r"F:\uploads\Series1\source"
SYNC_DIR = r"F:\uploads\Series1\tosync"
OUT_DIR = r"F:\uploads\Series1\output"
FFMPEG = r"bin\ffmpeg.exe"

# Script paths relative to the script's directory
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
SYNC_SCRIPT = os.path.join(SCRIPT_DIR, "sync_ad_track.py")
FRAMEPERFECT_SCRIPT = os.path.join(SCRIPT_DIR, "batch_frameperfect.py")
DTW_SCRIPT = os.path.join(SCRIPT_DIR, "dtw_smooth_stitch.py")

os.makedirs(OUT_DIR, exist_ok=True)


def detect_black_screens(mkv_file):
    cmd = [
        FFMPEG, "-i", mkv_file,
        "-vf", "blackdetect=d=5.0:pix_th=0.1",
        "-f", "null", "-"
    ]
    proc = subprocess.Popen(
        cmd,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8")

    regions = []
    for line in proc.stderr:
        line = line.strip()
        if "black_start:" in line:
            m = re.search(
                r"black_start:([0-9\.]+) black_end:([0-9\.]+) black_duration:", line)
            if m:
                regions.append((float(m.group(1)), float(m.group(2))))

    return regions


def get_audio_offsets(mkv_file, mka_file):
    cmd = [
        "python",
        SYNC_SCRIPT,
        "-r",
        mkv_file,
        "-a",
        mka_file,
        "-o",
        OUT_DIR,
        "-t",
        "Series1"]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8")

    offsets = []
    for line in proc.stdout:
        line = line.strip()
        m = re.search(r"SELECTED CONSENSUS OFFSET.*:\s*([+-]?\d+\.\d+)s", line)
        if m:
            offsets.append(float(m.group(1)))
        m2 = re.search(r"Runner-up #\d+.*:\s*([+-]?\d+\.\d+)s", line)
        if m2:
            offsets.append(float(m2.group(1)))

    if not offsets:
        return []

    clustered = [offsets[0]]
    for off in offsets[1:]:
        if abs(off - clustered[-1]) > 0.05:
            clustered.append(off)

    return clustered


def build_filter_graph(regions, gaps, ad_offset):
    trim_points = []
    for i in range(len(regions)):
        b_start, b_end = regions[i]
        gap = gaps[i]
        black_dur = b_end - b_start
        padding = (black_dur - gap) / 2.0
        trim_points.append((b_start + padding, b_end - padding))

    graph = ""
    v_segments = []
    a_segments = []

    graph += f"[0:v]trim=start=0:end={
        trim_points[0][0]},setpts=PTS-STARTPTS[v1]; "
    graph += f"[0:a]atrim=start=0:end={
        trim_points[0][0]},asetpts=PTS-STARTPTS[a1]; "
    v_segments.append("[v1]")
    a_segments.append("[a1]")

    for i in range(1, len(trim_points)):
        graph += f"[0:v]trim=start={trim_points[i -
                                                1][1]}:end={trim_points[i][0]},setpts=PTS-STARTPTS[v{i +
                                                                                                     1}]; "
        graph += f"[0:a]atrim=start={trim_points[i -
                                                 1][1]}:end={trim_points[i][0]},asetpts=PTS-STARTPTS[a{i +
                                                                                                       1}]; "
        v_segments.append(f"[v{i + 1}]")
        a_segments.append(f"[a{i + 1}]")

    last_i = len(trim_points)
    graph += f"[0:v]trim=start={trim_points[-1][1]},setpts=PTS-STARTPTS[v{last_i + 1}]; "
    graph += f"[0:a]atrim=start={trim_points[-1][1]},asetpts=PTS-STARTPTS[a{last_i + 1}]; "
    v_segments.append(f"[v{last_i + 1}]")
    a_segments.append(f"[a{last_i + 1}]")

    concat_str = ""
    for i in range(len(v_segments)):
        concat_str += f"{v_segments[i]}{a_segments[i]}"
    graph += f"{concat_str}concat=n={
        len(v_segments)}:v=1:a=1[v_out][a_out_master]; "

    if ad_offset >= 0:
        graph += f"[1:a]atrim=start={ad_offset},asetpts=PTS-STARTPTS[a_out_ad]"
    else:
        pad_ms = int(abs(ad_offset) * 1000)
        graph += f"[1:a]adelay={pad_ms}|{pad_ms}[a_out_ad]"

    return graph


def do_direct_mux(ep_num, mkv_file, ad_file, offset, dry_run=False):
    out_file = os.path.join(
        OUT_DIR, f"Series1 - S{ep_num[:2]}E{ep_num[2:]} (Direct Mux).mkv")
    if os.path.exists(out_file):
        print(f"[{ep_num}] Skipping DIRECT MUX (already exists)")
        return

    print(f"[{ep_num}] --> Strategy: DIRECT MUX (Single offset: {offset:+.3f}s)")
    if dry_run:
        return

    if offset >= 0:
        audio_filter = f"[1:a]atrim=start={offset},asetpts=PTS-STARTPTS,loudnorm=I=-16:TP=-1.5:LRA=11[a_out]"
    else:
        pad_ms = int(abs(offset) * 1000)
        audio_filter = f"[1:a]adelay={pad_ms}|{pad_ms},loudnorm=I=-16:TP=-1.5:LRA=11[a_out]"

    cmd = [
        FFMPEG, "-y",
        "-i", mkv_file,
        "-i", ad_file,
        "-filter_complex", audio_filter,
        "-map", "0:v", "-c:v", "copy",
        "-map", "0:a:0", "-c:a:0", "copy",
        "-map", "[a_out]", "-c:a:1", "aac", "-b:a:1", "256k", "-ar:a:1", "48000",
        "-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=Main, Stereo", "-disposition:a:0", "default",
        "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=AD, Stereo", "-disposition:a:1", "visual_impaired",
        out_file
    ]
    print(f"COMMAND: {' '.join(cmd)}")
    subprocess.run(cmd)


def do_frame_perfect(ep_num, mkv_file, ad_file, regions,
                     gaps, first_offset, dry_run=False):
    out_file = os.path.join(
        OUT_DIR, f"Series1 - S{ep_num[:2]}E{ep_num[2:]} (Frame Perfect).mkv")
    if os.path.exists(out_file):
        print(f"[{ep_num}] Skipping FRAME PERFECT (already exists)")
        return

    print(f"[{ep_num}] --> Strategy: FRAME-PERFECT (Black screens match audio shifts)")
    if dry_run:
        return

    graph = build_filter_graph(regions, gaps, first_offset)

    cmd = [
        FFMPEG, "-y",
        "-i", mkv_file,
        "-i", ad_file,
        "-filter_complex", graph,
        "-map", "[v_out]",
        "-map", "[a_out_master]",
        "-map", "[a_out_ad]",
        "-c:v", "libx264", "-preset", "fast", "-crf", "18",
        "-c:a:0", "ac3", "-b:a:0", "640k",
        "-c:a:1", "aac", "-b:a:1", "256k", "-ar:a:1", "48000",
        "-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=Main, Stereo", "-disposition:a:0", "default",
        "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=AD, Stereo", "-disposition:a:1", "visual_impaired",
        out_file
    ]
    print(f"COMMAND: {' '.join(cmd)}")
    subprocess.run(cmd)


def do_dtw(ep_num, mkv_file, ad_file, dry_run=False):
    out_file = os.path.join(
        OUT_DIR, f"Series1 - S{ep_num[:2]}E{ep_num[2:]} (DTW Sync).mkv")
    if os.path.exists(out_file):
        print(f"[{ep_num}] Skipping DTW SYNC (already exists)")
        return

    print(f"[{ep_num}] --> Strategy: DYNAMIC TIME WARPING (0 black screens, but audio drifts)")
    if dry_run:
        return

    # Run describealign to generate alignment plot + aligned MKV
    describealign_script = r"describealign.py"
    cmd = [
        "python",
        describealign_script,
        "--yes",
        "--stretch_audio",
        mkv_file,
        ad_file]

    print(f"[{ep_num}] Running describealign DTW processor (this will take a while)...")
    print(f"COMMAND: {' '.join(cmd)}")
    subprocess.run(cmd, cwd=OUT_DIR)

    # describealign outputs both:
    #   videos_with_ad/ad_<filename>   (aligned MKV -- hard cuts, can be harsh)
    #   alignment_plots/<filename>.txt (piecewise timestamp map)

    # Look for alignment plot to use our smooth stitcher (preferred path)
    import glob
    basename = os.path.basename(mkv_file)
    plot_name = os.path.splitext(basename)[0]
    plot_files = sorted(
        glob.glob(
            os.path.join(
                OUT_DIR,
                "alignment_plots",
                f"*{plot_name}*.txt")))
    aligned_files = glob.glob(
        os.path.join(
            OUT_DIR,
            "videos_with_ad",
            f"*{basename}"))

    if plot_files:
        # -- Smooth Stitch Path (3-layer transitions) --
        print(
            f"[{ep_num}] Found alignment plot -- applying smooth stitch with VAD transitions...")

        # Import the smooth stitcher
        sys.path.insert(0, SCRIPT_DIR)
        from dtw_smooth_stitch import run_smooth_stitch

        scratch_dir = os.path.join(OUT_DIR, "scratch")
        smooth_ad_m4a = os.path.join(scratch_dir, f"ep{ep_num}_smooth_ad.m4a")

        try:
            std_ad, nm_ad = run_smooth_stitch(
                plot_files[-1],    # Most recent alignment plot
                ad_file,           # Original AD audio source
                smooth_ad_m4a,     # Smooth-stitched output base
                scratch_dir=scratch_dir,
            )

            # Mux: original video + original main audio + our smooth AD audio
            # (std + nm)
            temp_file = out_file + ".temp.mkv"
            mux_cmd = [
                FFMPEG, "-y", "-hide_banner", "-loglevel", "error",
                "-i", mkv_file,         # Video + Main audio
                "-i", std_ad,           # Smooth-stitched AD (Standard)
                "-i", nm_ad,            # Smooth-stitched AD (Night Mode)
                "-map", "0:v:0",
                "-map", "0:a:0",        # Main audio
                "-map", "1:a:0",        # Smooth AD audio (Standard)
                "-map", "2:a:0",        # Smooth AD audio (Night Mode)
                "-c", "copy",
                "-disposition:a:0", "default",
                "-disposition:a:1", "0",
                "-disposition:a:2", "0",
                "-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=Main, Stereo",
                "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=AD, Stereo",
                "-metadata:s:a:2", "language=eng", "-metadata:s:a:2", "title=AD, Night Mode",
                temp_file
            ]
            print(f"COMMAND: {' '.join(mux_cmd)}")
            subprocess.run(mux_cmd, check=True)
            os.replace(temp_file, out_file)

            # Clean up describealign's raw output if it exists
            for f in aligned_files:
                try:
                    os.remove(f)
                except OSError:
                    pass

            print(f"[{ep_num}] Finished DTW Sync (smooth transitions)!")
            return

        except Exception as e:
            print(
                f"[{ep_num}] WARNING: Smooth stitch failed ({e}), falling back to describealign output...")

    # -- Fallback: Use describealign's direct output --
    if not aligned_files:
        print(f"[{ep_num}] ERROR: describealign failed to produce output!")
        return

    shutil.move(aligned_files[0], out_file)

    # describealign outputs AD as a:0, Main as a:1 -- swap to our standard
    # order
    temp_file = out_file + ".temp.mkv"
    remux_cmd = [
        FFMPEG, "-y", "-hide_banner", "-loglevel", "error",
        "-i", out_file,
        "-map", "0:v:0",
        "-map", "0:a:1",  # Original Main audio (was track 2) -> now track 1
        "-map", "0:a:0",  # AD audio (was track 1) -> now track 2
        "-c", "copy",
        "-metadata:s:a:0", "title=Main, Stereo", "-metadata:s:a:0", "language=eng",
        "-disposition:a:0", "default",
        "-metadata:s:a:1", "title=AD, Stereo", "-metadata:s:a:1", "language=eng",
        "-disposition:a:1", "visual_impaired",
        temp_file,
    ]
    subprocess.run(remux_cmd, check=True)
    os.replace(temp_file, out_file)

    print(f"[{ep_num}] Finished DTW Sync (describealign fallback)!")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Parse and categorize all episodes without encoding")
    args = parser.parse_args()

    mkv_files = sorted(glob.glob(os.path.join(SOURCE_DIR, "*.mkv")))

    stats = {"Frame-Perfect": 0, "Direct Mux": 0, "DTW": 0, "Error": 0}

    for mkv in mkv_files:
        basename = os.path.basename(mkv)
        m = re.search(r"s(\d+)e(\d+)", basename.lower())
        if not m:
            continue

        season = m.group(1).lstrip("0")
        episode = m.group(2).lstrip("0")
        ep_code = f"{season.zfill(2)}{episode.zfill(2)}"

        out_fp = os.path.join(
            OUT_DIR, f"Series1 - S{ep_code[:2]}E{ep_code[2:]} (Frame Perfect).mkv")
        out_dm = os.path.join(
            OUT_DIR, f"Series1 - S{ep_code[:2]}E{ep_code[2:]} (Direct Mux).mkv")
        out_dtw = os.path.join(
            OUT_DIR, f"Series1 - S{ep_code[:2]}E{ep_code[2:]} (DTW Sync).mkv")
        if os.path.exists(out_fp) or os.path.exists(
                out_dm) or os.path.exists(out_dtw):
            print(f"[{ep_code}] Skipping analysis (already synced)")
            continue

        ad_pattern = os.path.join(SYNC_DIR,
                                  f"*{season}x{episode.zfill(2)}*.mka")
        ad_matches = glob.glob(ad_pattern)
        if not ad_matches:
            print(f"[{ep_code}] ERROR: Could not find AD track!")
            stats["Error"] += 1
            continue

        ad_file = ad_matches[0]

        regions = detect_black_screens(mkv)
        offsets = get_audio_offsets(mkv, ad_file)

        if not offsets:
            print(f"[{ep_code}] ERROR: Failed to calculate AD offsets!")
            stats["Error"] += 1
            continue

        if len(regions) > 0:
            gaps = [offsets[i] - offsets[i + 1]
                    for i in range(len(offsets) - 1)]
            if len(offsets) == len(regions) + 1:
                stats["Frame-Perfect"] += 1
                do_frame_perfect(
                    ep_code,
                    mkv,
                    ad_file,
                    regions,
                    gaps,
                    offsets[0],
                    args.dry_run)
            else:
                print(
                    f"[{ep_code}] ERROR: {len(regions)} black screens but {len(offsets)} shifts. Mismatch!")
                stats["Error"] += 1
        else:
            if len(offsets) == 1:
                stats["Direct Mux"] += 1
                do_direct_mux(ep_code, mkv, ad_file, offsets[0], args.dry_run)
            else:
                stats["DTW"] += 1
                do_dtw(ep_code, mkv, ad_file, args.dry_run)

    print(f"\n{'=' * 40}\nDRY RUN RESULTS\n{'=' * 40}")
    for k, v in stats.items():
        print(f"{k}: {v} episodes")


if __name__ == "__main__":
    main()
