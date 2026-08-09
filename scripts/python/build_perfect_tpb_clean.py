import numpy as np
import subprocess
import os

AD_FILE = r"F:\uploads\TPB\tosync\Trailer Park Boys 1x01 Take Your Little Gun and Get Out of My Trailer Park 70143633 2.0 English - Audio Description.mka"
MASTER_FILE = r"F:\uploads\TPB\source\s01e01 - Take Your Little Gun and Get Out of My Trailer Park (Clean).mkv"
OUT_FILE = r"F:\uploads\TPB\output\clean_master\TPB_Perfect_Clean_AD.wav"
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

# Offsets based on the describealign output for the Clean MKV:

# Cut 1: Master 0 to 8:31.040 (511.040s)
# Aligning with AD 17.762s to 528.804s (8:48.804)
ad_start1 = int(17.762 * SR)
ad_end1 = int(528.804 * SR)
seg1 = ad_audio[ad_start1:ad_end1]

# Gap 1: AD track skips from 528.804 to 530.872 (2.068s of extra AD audio discarded)

# Cut 2: Master 8:31.044 to 15:23.636 (511.044s to 923.636s)
# Aligning with AD 530.872s to 943.461s (15:43.461)
ad_start2 = int(530.872 * SR)
ad_end2 = int(943.461 * SR)
seg2 = ad_audio[ad_start2:ad_end2]

# Gap 2: AD track skips from 943.461 to 946.143 (2.682s of extra AD audio discarded)

# Cut 3: Master 15:23.640 to End (923.640s to End)
# Aligning with AD 946.143s to End
ad_start3 = int(946.143 * SR)
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
