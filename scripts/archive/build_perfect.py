# SPDX-License-Identifier: MIT
import numpy as np
import subprocess
import os

AD_FILE = r"F:\uploads\Series1\tosync\Series_1x01_AD.mka"
MASTER_FILE = r"F:\uploads\Series1\source\Series_s01e01_Main.ac3"
OUT_FILE = r"F:\uploads\Series1\output\attempt2\Series_Perfect_AD.wav"
FFMPEG = r"bin\ffmpeg.exe"
SR = 48000

def decode_audio(file):
    print(f"Decoding {file}...")
    cmd = [FFMPEG, '-y', '-i', file, '-f', 'f32le', '-ac', '2', '-ar', str(SR), '-']
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    raw, _ = proc.communicate()
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2)

ad_audio = decode_audio(AD_FILE)
master_audio = decode_audio(MASTER_FILE)

# Offsets calculated from cross-correlation
# Offset > 0 means AD is behind Master -> Master = AD - offset -> AD = Master + offset
# Offset < 0 means AD is ahead of Master -> Master = AD - offset -> AD = Master + offset

# Cut 1: Master 0 to 513.280s (8:33.280)
# Offset: +17.788s
# AD source: 17.788s to (513.280 + 17.788) = 531.068s
cut1_master_end = 513.280
offset1 = 17.788
ad_start1 = int(offset1 * SR)
ad_end1 = int((cut1_master_end + offset1) * SR)
seg1 = ad_audio[ad_start1:ad_end1]

# Cut 2: Master 523.057s (8:43.057) to 940.875s (15:40.875)
# Offset: +8.012s
cut2_master_start = 523.057
cut2_master_end = 940.875
offset2 = 8.012
ad_start2 = int((cut2_master_start + offset2) * SR)
ad_end2 = int((cut2_master_end + offset2) * SR)
seg2 = ad_audio[ad_start2:ad_end2]

# Cut 3: Master 950.549s (15:50.549) to End
# Offset: -1.665s
cut3_master_start = 950.549
offset3 = -1.665
ad_start3 = int((cut3_master_start + offset3) * SR)
seg3 = ad_audio[ad_start3:]

# Build the final output track mapped exactly to the Master timeline!
# Since we are mapping to the master timeline, any gaps between cut1_master_end and cut2_master_start 
# (which is 513.280 to 523.057 = 9.777s) must be filled with silence!
gap1_samples = int(cut2_master_start * SR) - int(cut1_master_end * SR)
gap1 = np.zeros((gap1_samples, 2), dtype=np.float32)

gap2_samples = int(cut3_master_start * SR) - int(cut2_master_end * SR)
gap2 = np.zeros((gap2_samples, 2), dtype=np.float32)

print("Assembling final audio...")
out_audio = np.concatenate([seg1, gap1, seg2, gap2, seg3])

# Ensure it matches master length exactly if it overshoots
if len(out_audio) > len(master_audio):
    out_audio = out_audio[:len(master_audio)]
elif len(out_audio) < len(master_audio):
    padding = np.zeros((len(master_audio) - len(out_audio), 2), dtype=np.float32)
    out_audio = np.concatenate([out_audio, padding])

print(f"Writing to {OUT_FILE}...")
proc = subprocess.Popen([FFMPEG, '-y', '-f', 'f32le', '-ac', '2', '-ar', str(SR), '-i', '-', OUT_FILE], stdin=subprocess.PIPE, stderr=subprocess.DEVNULL)
proc.communicate(out_audio.tobytes())
print("Done!")
