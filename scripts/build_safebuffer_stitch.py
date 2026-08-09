import numpy as np
import subprocess
import os

AD_FILE = r"F:\uploads\TPB\tosync\Trailer Park Boys 1x01 Take Your Little Gun and Get Out of My Trailer Park 70143633 2.0 English - Audio Description.mka"
MASTER_FILE = r"F:\uploads\TPB\output\clean_master\Trailer Park Boys - S01E01 (Safe Buffer Master).mkv"
OUT_FILE = r"F:\uploads\TPB\output\clean_master\TPB_Safe_Buffer_AD.wav"
FFMPEG = r"C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
SR = 48000

def decode_audio(file):
    print(f"Decoding {file}...")
    cmd = [FFMPEG, '-y', '-i', file, '-f', 'f32le', '-ac', '2', '-ar', str(SR), '-']
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    raw, _ = proc.communicate()
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2)

ad_audio = decode_audio(AD_FILE)
master_audio = decode_audio(MASTER_FILE)

# Master Video Cuts
cut1_master_end = 514.948
cut2_master_start = 932.665  # 514.948 + (939.071 - 521.354)

# Offsets from sync_ad_track
offset1 = 17.760
offset2 = 14.408
offset3 = 10.837

ad_start1 = int(17.760 * SR)
ad_end1 = int((cut1_master_end + offset1) * SR)
seg1 = ad_audio[ad_start1:ad_end1]

ad_start2 = int((cut1_master_end + offset2) * SR)
ad_end2 = int((cut2_master_start + offset2) * SR)
seg2 = ad_audio[ad_start2:ad_end2]

ad_start3 = int((cut2_master_start + offset3) * SR)
seg3 = ad_audio[ad_start3:]

print("Assembling final audio...")
out_audio = np.concatenate([seg1, seg2, seg3])

# Ensure it matches master length exactly
if len(out_audio) > len(master_audio):
    out_audio = out_audio[:len(master_audio)]
elif len(out_audio) < len(master_audio):
    padding = np.zeros((len(master_audio) - len(out_audio), 2), dtype=np.float32)
    out_audio = np.concatenate([out_audio, padding])

print(f"Writing to {OUT_FILE}...")
proc = subprocess.Popen([FFMPEG, '-y', '-f', 'f32le', '-ac', '2', '-ar', str(SR), '-i', '-', OUT_FILE], stdin=subprocess.PIPE, stderr=subprocess.DEVNULL)
proc.communicate(out_audio.tobytes())
print("Done!")
