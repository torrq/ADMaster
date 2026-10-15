r"""
run_silo_sequence.py - Batch Sequence Runner for Silo (Seasons 1-3)

Iterates sequentially through all episode folders in F:\uploads\Silo\queue,
synchronizing audio description tracks, downmixing surround master audio to
True 2.0 Stereo AAC, and multiplexing the final -stereoonly.mkv via sync_ad_track.py
into a consolidated MKV output directory (default: F:\uploads\Silo\mkv).

Usage:
    python scripts/run_silo_sequence.py
    python scripts/run_silo_sequence.py --dry-run
    python scripts/run_silo_sequence.py --mkv-dir "F:\uploads\Silo\mkv"
    python scripts/run_silo_sequence.py --start 3
    python scripts/run_silo_sequence.py --start S02E01 --stop S02E10
"""

import argparse
import os
import re
import subprocess
import sys
import time


DEFAULT_QUEUE_DIR = r"F:\uploads\Silo\queue"
DEFAULT_MKV_DIR = r"F:\uploads\Silo\mkv"


def get_sort_key(name):
    """Extract season and episode numbers for natural chronological sorting."""
    m = re.search(r"S0?(\d+)[\.\s_-]*E0?(\d+)", name, re.IGNORECASE)
    if m:
        return (int(m.group(1)), int(m.group(2)))
    return (999, 999)


def parse_ep_spec(spec, available_dirs):
    """Parses episode index (1-based) or name/pattern (e.g. S01E03) to directory index."""
    if spec is None:
        return None
    spec_str = str(spec).strip()
    if spec_str.isdigit():
        idx = int(spec_str) - 1
        return max(0, min(idx, len(available_dirs) - 1))
    
    spec_upper = spec_str.upper()
    for idx, d in enumerate(available_dirs):
        if spec_upper in d.upper():
            return idx
    return None


def main():
    parser = argparse.ArgumentParser(
        description="Run Silo batch alignment and stereo-only MKV remuxing in sequence."
    )
    parser.add_argument(
        "--queue-dir",
        default=DEFAULT_QUEUE_DIR,
        help=f"Directory containing episode folders (Default: {DEFAULT_QUEUE_DIR})"
    )
    parser.add_argument(
        "--mkv-dir",
        default=DEFAULT_MKV_DIR,
        help=f"Directory to output final multiplexed MKVs (Default: {DEFAULT_MKV_DIR})"
    )
    parser.add_argument(
        "--start",
        default=None,
        help="Start at specific 1-based index or episode label (e.g. 3, S01E03)"
    )
    parser.add_argument(
        "--stop",
        default=None,
        help="Stop at specific 1-based index or episode label (e.g. 10, S02E10)"
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the planned sequence and commands without executing them"
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="Force re-analyzing, re-rendering, and re-muxing existing deliverables"
    )
    parser.add_argument(
        "--threads",
        type=int,
        default=2,
        help="Number of CPU threads per job (Default: 2)"
    )

    args = parser.parse_args()

    if not os.path.exists(args.queue_dir):
        print(f"[ERROR] Queue directory not found: {args.queue_dir}")
        sys.exit(1)

    os.makedirs(args.mkv_dir, exist_ok=True)

    all_entries = [
        d for d in os.listdir(args.queue_dir)
        if os.path.isdir(os.path.join(args.queue_dir, d))
    ]
    ep_dirs = sorted(all_entries, key=get_sort_key)

    if not ep_dirs:
        print(f"[ERROR] No episode directories found in {args.queue_dir}")
        sys.exit(1)

    start_idx = parse_ep_spec(args.start, ep_dirs) if args.start else 0
    stop_idx = parse_ep_spec(args.stop, ep_dirs) if args.stop else len(ep_dirs) - 1

    if start_idx is None:
        print(f"[ERROR] Could not resolve --start '{args.start}'")
        sys.exit(1)
    if stop_idx is None:
        print(f"[ERROR] Could not resolve --stop '{args.stop}'")
        sys.exit(1)

    selected_eps = ep_dirs[start_idx:stop_idx + 1]

    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    sync_script = os.path.join(base_dir, "scripts", "sync_ad_track.py")

    print("=" * 80)
    print("SILO BATCH SEQUENCE RUNNER")
    print(f"Queue Directory : {args.queue_dir}")
    print(f"MKV Output Dir  : {args.mkv_dir}")
    print(f"Total Episodes  : {len(ep_dirs)}")
    print(f"Selected Range  : [{start_idx + 1} to {stop_idx + 1}] ({len(selected_eps)} episodes)")
    print(f"Flags           : --stereo-only --no-night --mux --mkv-dir \"{args.mkv_dir}\"")
    if args.overwrite:
        print("Overwrite Mode  : ENABLED")
    if args.dry_run:
        print("Dry Run Mode    : ACTIVE (no jobs will be launched)")
    print("=" * 80)

    for i, ep_name in enumerate(selected_eps, start=start_idx + 1):
        se = get_sort_key(ep_name)
        se_tag = f"S{se[0]:02d}E{se[1]:02d}" if se[0] < 999 else "SPEC"
        print(f"  [{i:2d}/{len(ep_dirs)}] {se_tag}: {ep_name}")

    if args.dry_run:
        print("\n[DRY RUN] Exiting without changes.")
        return

    total_start_time = time.time()
    results = []

    print("\n" + "=" * 80)
    print("STARTING SEQUENCE EXECUTION")
    print("=" * 80)

    for i, ep_name in enumerate(selected_eps, start=start_idx + 1):
        ep_dir = os.path.join(args.queue_dir, ep_name)
        se = get_sort_key(ep_name)
        se_tag = f"S{se[0]:02d}E{se[1]:02d}" if se[0] < 999 else "EP"

        # Check if output MKV already exists (either .mkv or legacy -stereoonly.mkv)
        target_mkv = os.path.join(args.mkv_dir, f"{ep_name}.mkv")
        alt_mkv = os.path.join(args.mkv_dir, f"{ep_name}-stereoonly.mkv")

        if not args.overwrite:
            found_mkv = None
            if os.path.exists(target_mkv) and os.path.getsize(target_mkv) > 1000:
                found_mkv = target_mkv
            elif os.path.exists(alt_mkv) and os.path.getsize(alt_mkv) > 1000:
                found_mkv = alt_mkv

            if found_mkv:
                size_mb = os.path.getsize(found_mkv) / (1024 * 1024)
                print(f"[SKIP] [{i:2d}/{len(ep_dirs)}] {se_tag}: MKV already exists -> {os.path.basename(found_mkv)} ({size_mb:.1f} MB)", flush=True)
                results.append((i, se_tag, ep_name, "SKIPPED", 0.0))
                continue

        print("\n" + "#" * 80)
        print(f"# [{i}/{len(ep_dirs)}] PROCESSING: {se_tag} -> {ep_name}")
        print("#" * 80 + "\n", flush=True)

        cmd = [
            sys.executable,
            sync_script,
            ep_dir,
            "--stereo-only",
            "--no-night",
            "--mux",
            "--mkv-dir", args.mkv_dir,
            "--threads", str(args.threads),
        ]
        if args.overwrite:
            cmd.append("--overwrite")

        job_start = time.time()
        res = subprocess.run(cmd, stdin=subprocess.DEVNULL)
        elapsed = time.time() - job_start

        if res.returncode == 0:
            status = "SUCCESS"
        else:
            status = f"FAILED (code {res.returncode})"

        results.append((i, se_tag, ep_name, status, elapsed))
        print(f"\n>> [{status}] {se_tag} completed in {elapsed:.1f}s <<\n", flush=True)

    total_elapsed = time.time() - total_start_time

    print("\n" + "=" * 80)
    print("BATCH SEQUENCE SUMMARY")
    print("=" * 80)
    for idx, se_tag, ep_name, status, elapsed in results:
        if status == "SKIPPED":
            status_tag = "[SKIP]  "
        elif "SUCCESS" in status:
            status_tag = "[OK]    "
        else:
            status_tag = "[ERROR] "
        print(f"{status_tag} [{idx:2d}/{len(ep_dirs)}] {se_tag}: {status} ({elapsed:.1f}s)")
    print("-" * 80)
    print(f"Total Sequence Runtime: {total_elapsed / 60.0:.2f} minutes")
    print(f"Output MKVs Directory : {args.mkv_dir}")
    print("=" * 80 + "\n")


if __name__ == "__main__":
    main()
