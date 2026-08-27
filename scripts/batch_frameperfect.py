# SPDX-License-Identifier: MIT
import os
import glob
import re
import subprocess

# Paths
SOURCE_DIR = r"F:\uploads\Series1\source"
SYNC_DIR = r"F:\uploads\Series1\tosync"
OUT_DIR = r"F:\uploads\Series1\output"
FFMPEG = r"bin\ffmpeg.exe"
SYNC_SCRIPT = r"sync_ad_track.py"

os.makedirs(OUT_DIR, exist_ok=True)


def detect_black_screens(mkv_file):
    print(f"Detecting black screens in {os.path.basename(mkv_file)}...")
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
    print(
        f"Running acoustic cross-correlation on {os.path.basename(mkv_file)}...")
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
        print("  [sync_ad_track] " + line)
        m = re.search(
            r"Anchor\s+\d+s:\s*Measured Offset = ([+-]?\d+\.\d+)s",
            line)
        if m:
            offsets.append(float(m.group(1)))

    if not offsets:
        return []

    # Cluster offsets (diff > 1.0s means new segment)
    clustered = [offsets[0]]
    for off in offsets[1:]:
        if abs(off - clustered[-1]) > 1.0:
            clustered.append(off)

    return clustered


def build_filter_graph(regions, gaps, ad_offset):
    # trim_points: exact chunks of black screen to remove
    trim_points = []
    for i in range(len(regions)):
        b_start, b_end = regions[i]
        gap = gaps[i]
        black_dur = b_end - b_start
        padding = (black_dur - gap) / 2.0
        # Wait, if gap is LARGER than black_dur, padding is negative. This means the TV network cut out MORE than just the black screen!
        # If the TV network cut out show footage, we MUST remove the show
        # footage from the Master to match!
        trim_points.append((b_start + padding, b_end - padding))

    # Build graph
    graph = ""
    v_segments = []
    a_segments = []

    # First segment (0 to trim_points[0][start])
    graph += f"[0:v]trim=start=0:end={
        trim_points[0][0]},setpts=PTS-STARTPTS[v1]; "
    graph += f"[0:a]atrim=start=0:end={
        trim_points[0][0]},asetpts=PTS-STARTPTS[a1]; "
    v_segments.append("[v1]")
    a_segments.append("[a1]")

    # Middle segments
    for i in range(1, len(trim_points)):
        graph += f"[0:v]trim=start={trim_points[i -
                                                1][1]}:end={trim_points[i][0]},setpts=PTS-STARTPTS[v{i +
                                                                                                     1}]; "
        graph += f"[0:a]atrim=start={trim_points[i -
                                                 1][1]}:end={trim_points[i][0]},asetpts=PTS-STARTPTS[a{i +
                                                                                                       1}]; "
        v_segments.append(f"[v{i + 1}]")
        a_segments.append(f"[a{i + 1}]")

    # Last segment
    last_i = len(trim_points)
    graph += f"[0:v]trim=start={trim_points[-1][1]},setpts=PTS-STARTPTS[v{last_i + 1}]; "
    graph += f"[0:a]atrim=start={trim_points[-1][1]},asetpts=PTS-STARTPTS[a{last_i + 1}]; "
    v_segments.append(f"[v{last_i + 1}]")
    a_segments.append(f"[a{last_i + 1}]")

    # Concat
    concat_str = ""
    for i in range(len(v_segments)):
        concat_str += f"{v_segments[i]}{a_segments[i]}"
    graph += f"{concat_str}concat=n={
        len(v_segments)}:v=1:a=1[v_out][a_out_master]; "

    # AD Track processing
    if ad_offset >= 0:
        graph += f"[1:a]atrim=start={ad_offset},asetpts=PTS-STARTPTS[a_out_ad]"
    else:
        pad_ms = int(abs(ad_offset) * 1000)
        graph += f"[1:a]adelay={pad_ms}|{pad_ms}[a_out_ad]"

    return graph


def main():
    mkv_files = glob.glob(os.path.join(SOURCE_DIR, "*.mkv"))
    for mkv in mkv_files:
        basename = os.path.basename(mkv)
        m = re.search(r"s01e0([2-6])", basename.lower())
        if not m:
            continue

        ep_num = m.group(1)

        # Find matching AD file
        ad_pattern = os.path.join(SYNC_DIR, f"*1x0{ep_num}*.mka")
        ad_matches = glob.glob(ad_pattern)
        if not ad_matches:
            print(f"Could not find AD track for {basename}!")
            continue

        ad_file = ad_matches[0]
        out_file = os.path.join(
            OUT_DIR, f"Series1 - S01E0{ep_num} (Frame Perfect).mkv")

        if os.path.exists(out_file):
            print(f"Skipping {basename}, already processed!")
            continue

        print(f"\n{'=' * 80}\nPROCESSING EPISODE {ep_num}\n{'=' * 80}")

        # 1. Detect Black Screens
        regions = detect_black_screens(mkv)
        print(f"Found {len(regions)} commercial breaks: {regions}")

        # 2. Extract Offsets
        offsets = get_audio_offsets(mkv, ad_file)
        print(f"Clustered AD Offsets: {offsets}")

        if not offsets:
            print("ERROR: Failed to calculate AD offsets!")
            continue

        gaps = [offsets[i] - offsets[i + 1] for i in range(len(offsets) - 1)]
        print(f"Calculated Gaps: {gaps}")

        if len(gaps) != len(regions):
            print(
                f"ERROR: Found {
                    len(regions)} black screens but {
                    len(gaps)} offset shifts! Cannot perfectly align.")
            continue

        # 3. Build Filter Graph
        graph = build_filter_graph(regions, gaps, offsets[0])
        print(f"Generated Filter Graph: {graph}")

        # 4. Execute FFmpeg
        print(f"Executing Frame-Perfect Re-encode for Episode {ep_num}...")
        cmd = [
            FFMPEG, "-y",
            "-i", mkv,
            "-i", ad_file,
            "-filter_complex", graph,
            "-map", "[v_out]",
            "-map", "[a_out_master]",
            "-map", "[a_out_ad]",
            "-c:v", "libx264", "-preset", "fast", "-crf", "18",
            "-c:a:0", "ac3", "-b:a:0", "640k",
            "-metadata:s:a:0", "language=eng", "-metadata:s:a:0", "title=Main, Stereo", "-disposition:a:0", "default",
            "-c:a:1", "aac", "-b:a:1", "256k", "-ar:a:1", "48000",
            "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=AD, Stereo", "-disposition:a:1", "visual_impaired",
            out_file
        ]

        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8")

        for line in proc.stderr:
            if "time=" in line:
                print(f"  [ffmpeg] {line.strip()}", end="\r")
        print("\nFinished encode!")


if __name__ == "__main__":
    main()
