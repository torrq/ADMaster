# SPDX-License-Identifier: MIT
import os
import sys
import glob
import json
import subprocess

import re
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


MKVMERGE = locate_tool("mkvmerge")
MKVPROPEDIT = locate_tool("mkvpropedit")


def scan_and_set_commentary(target_dir):
    print("=" * 68)
    print("  SCANNING FOR COMMENTARY TRACKS TO FLAG")
    print(f"  Directory: {target_dir}")
    print("=" * 68)

    mkv_files = sorted(glob.glob(os.path.join(target_dir, "*.mkv")))
    if not mkv_files:
        print("No MKV files found.")
        return

    found_count = 0
    for mkv in mkv_files:
        fname = os.path.basename(mkv)

        # Probe tracks with mkvmerge -J
        res = subprocess.run([MKVMERGE, "-J", mkv],
                             capture_output=True, text=True, encoding="utf-8")
        if res.returncode != 0:
            print(f"[ERROR] Failed probing {fname}")
            continue

        data = json.loads(res.stdout)
        tracks = data.get("tracks", [])

        commentary_tracks = []
        for t in tracks:
            if t.get("type") != "audio":
                continue
            props = t.get("properties", {})
            name = props.get("track_name", "").lower()
            track_id = t.get("id")
            flag_comm = props.get("flag_commentary", False)

            # Check if name contains 'commentary'
            if "commentary" in name:
                commentary_tracks.append(
                    (track_id, props.get("track_name"), flag_comm))

        if not commentary_tracks:
            print(f"\n[-] {fname}")
            print("    No commentary tracks detected.")
            continue

        print(f"\n[+] {fname}")
        for tid, tname, flag_comm in commentary_tracks:
            # mkvpropedit track selector uses 1-based track number for audio (e.g. track:a2 or track:3)
            # using track:id+1 or track:@uid or --edit track:n where n is the 1-based stream number
            # But mkvpropedit supports --edit track:<track_id + 1> or --edit track:uid
            # The most reliable selector in mkvpropedit is `--edit track:id`
            # where id is mkvmerge id + 1 or track uid.
            print(
                f"    Track ID {tid}: \"{tname}\" (Current flag_commentary={flag_comm})")

            # We construct mkvpropedit command targeting the track by track id (+1 in mkvpropedit)
            # mkvpropedit uses 1-based track numbers: track:1, track:2, etc. (which corresponds to mkvmerge track id + 1)
            # OR track:d where d is track ID.
            # In mkvpropedit: `--edit track:d` where d is track ID (if using track:number or track:id)
            # Best is `--edit track:@<uid>` which is 100% unambiguous.

            uid = props.get("uid")
            if uid:
                edit_target = f"track:={uid}"
            else:
                edit_target = f"track:{tid + 1}"

            cmd = [
                MKVPROPEDIT, mkv,
                "--edit", edit_target,
                "--set", "flag-commentary=1"
            ]

            print(
                f"    Executing: mkvpropedit \"{fname}\" --edit {edit_target} --set flag-commentary=1")
            edit_res = subprocess.run(
                cmd, capture_output=True, text=True, encoding="utf-8")
            if edit_res.returncode == 0:
                print("    -> Successfully set flag-commentary=1")
                found_count += 1
            else:
                print(
                    f"    -> [ERROR] mkvpropedit failed: {edit_res.stderr.strip()}")

    print("\n" + "=" * 68)
    print(f"  FINISHED: Updated {found_count} commentary track(s)")
    print("=" * 68)


if __name__ == "__main__":
    folder = sys.argv[1] if len(
        sys.argv) > 1 else r"X:\video\TV\Show_Title\Show.S01.720p.BluRay.DD5.1.x264-Release"
    scan_and_set_commentary(folder)
