import subprocess
import os
import wave
import numpy as np

temp_dir = r"C:\Users\nathan\.gemini\antigravity\scratch\sync_test"
os.makedirs(temp_dir, exist_ok=True)
ffmpeg_bin = r"C:\Users\nathan\AppData\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"

mkv = r"X:\incoming\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer.mkv"
ac3_3 = r"F:\uploads\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer_Audio03.ac3"
ad_mp3 = r"F:\uploads\ad\10530 The Godfather (1972) [New Description] .mp3"

def extract_mono(inp, start, dur, out, map_arg=None):
    cmd = [ffmpeg_bin, "-y", "-ss", f"{start:.3f}", "-t", f"{dur:.3f}", "-i", inp]
    if map_arg:
        cmd.extend(["-map", map_arg])
    cmd.extend(["-af", "aresample=16000,pan=mono|c0=c0", "-c:a", "pcm_s16le", out])
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def read_wav(p):
    with wave.open(p, 'rb') as wf:
        return np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32)

mkv_a_wav = os.path.join(temp_dir, "mkv_a_300.wav")
ac3_3_wav = os.path.join(temp_dir, "ac3_3_300.wav")
ad_mp3_wav = os.path.join(temp_dir, "ad_mp3_300.wav")

extract_mono(mkv, 300.0, 30.0, mkv_a_wav, map_arg="0:a:0")
extract_mono(ac3_3, 300.0, 30.0, ac3_3_wav)
extract_mono(ad_mp3, 270.0, 90.0, ad_mp3_wav)

s_mkv = read_wav(mkv_a_wav)
s_ac3 = read_wav(ac3_3_wav)
s_ad  = read_wav(ad_mp3_wav)

c_mkv_ac3 = np.correlate(s_ac3, s_mkv, mode='valid')
b_mkv_ac3 = np.argmax(c_mkv_ac3)
p_mkv_ac3 = c_mkv_ac3[b_mkv_ac3] / (np.linalg.norm(s_mkv) * np.linalg.norm(s_ac3) + 1e-9)

c_mkv_ad = np.correlate(s_ad, s_mkv, mode='valid')
b_mkv_ad = np.argmax(c_mkv_ad)
p_mkv_ad = c_mkv_ad[b_mkv_ad] / (np.linalg.norm(s_mkv) * np.linalg.norm(s_ad[b_mkv_ad:b_mkv_ad+len(s_mkv)]) + 1e-9)
mkv_ad_diff = 270.0 + (b_mkv_ad / 16000.0) - 300.0

print(f"MKV Audio01 vs Audio03.ac3 Offset: {b_mkv_ac3 / 16000.0:+.3f}s (Peak: {p_mkv_ac3:.4f})")
print(f"MKV Audio01 vs AD MP3 Offset     : {mkv_ad_diff:+.3f}s (Peak: {p_mkv_ad:.4f})")
