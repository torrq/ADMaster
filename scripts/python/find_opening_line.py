import subprocess
import os
import wave
import numpy as np

temp_dir = r"C:\Users\nathan\.gemini\antigravity\scratch\sync_test"
os.makedirs(temp_dir, exist_ok=True)
ffmpeg_bin = r"C:\Users\nathan\AppData\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"

ac3_3 = r"F:\uploads\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer_Audio03.ac3"
ad_mp3 = r"F:\uploads\ad\10530 The Godfather (1972) [New Description] .mp3"

def extract_mono(inp, start, dur, out):
    cmd = [ffmpeg_bin, "-y", "-ss", f"{start:.3f}", "-t", f"{dur:.3f}", "-i", inp, "-af", "aresample=16000,pan=mono|c0=c0", "-c:a", "pcm_s16le", out]
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def read_wav(p):
    with wave.open(p, 'rb') as wf:
        return np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32)

ref_bonasera = os.path.join(temp_dir, "ref_bonasera.wav")
ad_opening = os.path.join(temp_dir, "ad_opening.wav")

extract_mono(ac3_3, 35.0, 10.0, ref_bonasera)
extract_mono(ad_mp3, 0.0, 60.0, ad_opening)

s_ref = read_wav(ref_bonasera)
s_ad = read_wav(ad_opening)

corr = np.correlate(s_ad, s_ref, mode='valid')
best_idx = np.argmax(corr)
peak_val = corr[best_idx] / (np.linalg.norm(s_ref) * np.linalg.norm(s_ad[best_idx:best_idx+len(s_ref)]) + 1e-9)

ad_bonasera_start = (best_idx / 16000.0)
delay_needed = 35.0 - ad_bonasera_start

res_text = (
    f"Bonasera Line in Ref: 35.00s | Bonasera Line in AD Track: {ad_bonasera_start:.2f}s\n"
    f"Peak Correlation Confidence: {peak_val:.4f}\n"
    f"Delay needed for perfect Bonasera match: {delay_needed:+.3f}s\n"
)

out_file = os.path.join(temp_dir, "onset_result.txt")
with open(out_file, "w") as f:
    f.write(res_text)

print(res_text)
