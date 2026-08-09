import subprocess
import os
import re
import json

ffmpeg_bin = r"C:\Users\nathan\AppData\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"
ffprobe_bin = os.path.join(os.path.dirname(ffmpeg_bin), "ffprobe.exe")

ref = r"F:\uploads\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer_Audio03.ac3"
ad = r"F:\uploads\ad\10530 The Godfather (1972) [New Description] .mp3"
out = r"F:\uploads\The Godfather (1972) - Audio Description [New Description] (Synced Standard).m4a"

# Get reference duration
cmd_prob = [ffprobe_bin, "-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", ref]
dur = float(subprocess.run(cmd_prob, stdout=subprocess.PIPE, text=True).stdout.strip())

offset = 3.600 # Exact -3.600s trimming delay verified across 7 timeline points

print(f"Rendering New Description test track with exact offset -{offset:.3f}s at 1.0x native speed...")

# Pass 1: Measure loudness stats
cmd_pass1 = [
    ffmpeg_bin, "-y", "-ss", f"{offset:.3f}",
    "-i", ad, "-t", f"{dur:.3f}",
    "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json",
    "-f", "null", "-"
]
res1 = subprocess.run(cmd_pass1, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

json_match = re.search(r'\{[^{}]*"input_i"[^{}]*\}', res1.stderr, re.DOTALL)
stats = json.loads(json_match.group(0))

loudnorm_p2 = (
    f"loudnorm=I=-16:TP=-1.5:LRA=11:"
    f"measured_I={stats['input_i']}:measured_TP={stats['input_tp']}:"
    f"measured_LRA={stats['input_lra']}:measured_thresh={stats['input_thresh']}:linear=true"
)

# Pass 2: Render 256k AAC M4A
cmd_pass2 = [
    ffmpeg_bin, "-y", "-ss", f"{offset:.3f}",
    "-i", ad, "-t", f"{dur:.3f}",
    "-af", f"aresample=48000,{loudnorm_p2}",
    "-c:a", "aac", "-b:a", "256k", out
]
res2 = subprocess.run(cmd_pass2, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

if res2.returncode == 0 and os.path.exists(out):
    size_mb = os.path.getsize(out) / (1024 * 1024)
    print(f"SUCCESSFULLY RENDERED TEST TRACK!")
    print(f"Path: {out}")
    print(f"Size: {size_mb:.2f} MB")
else:
    print(f"Error rendering: {res2.stderr}")
