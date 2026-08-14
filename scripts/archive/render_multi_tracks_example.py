import os
import subprocess

DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"

jobs = [
    {
        "title": "Movie Title 1 (Year)",
        "ad": r"F:\uploads\MovieProjectDir\Movie Title 1\tosync\Movie Title 1 (Year) [AudioVault Original].mp3",
        "out_dir": r"F:\uploads\MovieProjectDir\Movie Title 1\output",
        "offset": 8.979,
        "atempo": 1.00000000
    },
    {
        "title": "Movie Title 2 (Year)",
        "ad": r"F:\uploads\MovieProjectDir\Movie Title 2\tosync\Movie Title 2 (Year) [AU].mp3",
        "out_dir": r"F:\uploads\MovieProjectDir\Movie Title 2\output",
        "offset": -12.880,
        "atempo": 0.95830000
    },
    {
        "title": "Movie Title 3 (Year)",
        "ad": r"F:\uploads\MovieProjectDir\Movie Title 3\tosync\Monty Python's The Meaning of life.mp3",
        "out_dir": r"F:\uploads\MovieProjectDir\Movie Title 3\output",
        "offset": -3.300,
        "atempo": 0.95412500
    }
]

for job in jobs:
    title = job["title"]
    ad_input = job["ad"]
    out_dir = job["out_dir"]
    offset = job["offset"]
    atempo = job["atempo"]
    
    os.makedirs(out_dir, exist_ok=True)
    out_stereo = os.path.join(out_dir, f"{title}-AD (Stereo).m4a")
    out_night = os.path.join(out_dir, f"{title}-AD (Night Mode).m4a")
    
    print(f"\n========================================================")
    print(f"Rendering: {title}")
    print(f"  Offset: {offset:+.3f}s | atempo: {atempo:.8f}x")
    print(f"  Stereo: {out_stereo}")
    print(f"  Night:  {out_night}")
    print(f"========================================================")
    
    filters_stereo = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_stereo.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_stereo.append(f"adelay={delay_ms}:all=1")
    filters_stereo.append("alimiter=level_in=1:level_out=0.98:limit=0.98")
    
    filters_night = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_night.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_night.append(f"adelay={delay_ms}:all=1")
    filters_night.append("loudnorm=I=-16:TP=-1.5:LRA=11")
    
    cmd = [DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning", "-i", ad_input]
    if offset > 0:
        cmd.extend(["-ss", f"{offset:.3f}"])
    cmd.extend([
        "-vn", "-af", ",".join(filters_stereo), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_stereo
    ])
    if offset > 0:
        cmd.extend(["-ss", f"{offset:.3f}"])
    cmd.extend([
        "-vn", "-af", ",".join(filters_night), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_night
    ])
    
    print("  Executing FFmpeg encoding...", flush=True)
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        print(f"  Error: {res.stderr}")
    else:
        print(f"  [SUCCESS] Finished {title}!")

print("\n========================================================")
print("ALL 3 MONTY PYTHON AD TRACKS SUCCESSFULLY COMPLETED!")
print("========================================================")
