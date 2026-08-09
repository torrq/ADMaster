import subprocess
import os
import wave
import re
import json
import numpy as np

ffmpeg_bin = r"C:\Users\nathan\AppData\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"
temp_dir = r"C:\Users\nathan\.gemini\antigravity\scratch\sync_test"

ad_wav = r"F:\uploads\ad\10530 The Godfather (1972) [New Description].wav"
mkv = r"X:\incoming\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer.mkv"

# AD is 3.6s AHEAD of MKV → we need to DELAY the AD by 3.6s (prepend silence)
delay_ms = 3600  # 3.6 seconds
render_dur = 180.0

out_m4a = r"F:\uploads\The Godfather (1972) - AD [New Description] (Delay Fix Test).m4a"
out_mkv = r"F:\uploads\The Godfather (1972) - AD [New Description] (Delay Fix Test).mkv"

# ===== STEP 1: 2-pass loudnorm render with adelay (prepend silence) =====
print("Step 1: Pass 1 - Measuring loudness stats...")
print(f"  Applying adelay={delay_ms}ms (prepend {delay_ms/1000:.1f}s silence to align)")
af_base = f"adelay={delay_ms}|{delay_ms},aresample=48000"
cmd_p1 = [
    ffmpeg_bin, "-y",
    "-i", ad_wav,
    "-t", f"{render_dur:.3f}",
    "-af", f"{af_base},loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json",
    "-f", "null", "-"
]
res1 = subprocess.run(cmd_p1, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

json_pattern = r'\{[^{}]*"input_i"[^{}]*\}'
json_match = re.search(json_pattern, res1.stderr, re.DOTALL)

if json_match:
    stats = json.loads(json_match.group(0))
    print(f'  Measured: I={stats["input_i"]} TP={stats["input_tp"]} LRA={stats["input_lra"]}')

    print("Step 2: Pass 2 - Rendering with linear gain...")
    loudnorm_p2 = (
        f'loudnorm=I=-16:TP=-1.5:LRA=11:'
        f'measured_I={stats["input_i"]}:measured_TP={stats["input_tp"]}:'
        f'measured_LRA={stats["input_lra"]}:measured_thresh={stats["input_thresh"]}:linear=true'
    )
    cmd_p2 = [
        ffmpeg_bin, "-y",
        "-i", ad_wav,
        "-t", f"{render_dur:.3f}",
        "-af", f"{af_base},{loudnorm_p2}",
        "-c:a", "aac", "-b:a", "256k", out_m4a
    ]
    subprocess.run(cmd_p2, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    print(f"  Saved: {out_m4a}")
else:
    print("  WARN: Pass 1 JSON parse failed, using 1-pass fallback")
    cmd_fb = [
        ffmpeg_bin, "-y",
        "-i", ad_wav,
        "-t", f"{render_dur:.3f}",
        "-af", f"{af_base},loudnorm=I=-16:TP=-1.5:LRA=11",
        "-c:a", "aac", "-b:a", "256k", out_m4a
    ]
    subprocess.run(cmd_fb, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    print(f"  Saved: {out_m4a}")

# ===== STEP 3: VERIFY rendered output against MKV audio =====
print()
print("Step 3: Verifying rendered AD alignment against MKV audio...")

sr = 8000

def extract(inp, start, d, out, map_arg=None):
    cmd = [ffmpeg_bin, "-y", "-ss", f"{start:.3f}", "-i", inp]
    if map_arg:
        cmd.extend(["-map", map_arg])
    cmd.extend(["-t", f"{d:.3f}", "-af", f"aresample={sr},pan=mono|c0=c0", "-c:a", "pcm_s16le", out])
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def read_wav(p):
    with wave.open(p, "rb") as wf:
        return np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32)

for check_time in [30.0, 90.0, 150.0]:
    verify_ad = os.path.join(temp_dir, f"vfy_ad_{int(check_time)}.wav")
    verify_mkv = os.path.join(temp_dir, f"vfy_mkv_{int(check_time)}.wav")
    extract(out_m4a, check_time, 20.0, verify_ad)
    extract(mkv, check_time, 20.0, verify_mkv, map_arg="0:a:0")

    s_ad = read_wav(verify_ad)
    s_mkv = read_wav(verify_mkv)

    corr = np.correlate(s_ad, s_mkv, mode="full")
    mid = len(s_mkv) - 1
    peak = np.argmax(corr)
    lag = (peak - mid) / sr
    conf = corr[peak] / (np.linalg.norm(s_mkv) * np.linalg.norm(s_ad) + 1e-9)
    status = "OK" if abs(lag) < 0.5 else "MISALIGNED"
    print(f"  @{check_time:5.0f}s: Lag = {lag:+.4f}s | Confidence = {conf:.4f} | {status}")

# ===== STEP 4: Mux with video =====
print()
print("Step 4: Multiplexing with video...")
cmd_mux = [
    ffmpeg_bin, "-y",
    "-ss", "0", "-t", f"{render_dur:.3f}", "-i", mkv,
    "-i", out_m4a,
    "-map", "0:v:0", "-map", "1:a:0",
    "-c:v", "copy", "-c:a", "copy",
    "-metadata:s:a:0", "title=Audio Description (Synced)",
    out_mkv
]
subprocess.run(cmd_mux, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if os.path.exists(out_mkv):
    sz = os.path.getsize(out_mkv) / (1024 * 1024)
    print(f"  Saved: {out_mkv} ({sz:.1f} MB)")
else:
    print("  ERROR: MKV muxing failed")
