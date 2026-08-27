# SPDX-License-Identifier: MIT
import numpy as np
import wave
import sys
from scipy.signal import fftconvolve, decimate

def read(p):
    with wave.open(p, 'rb') as wf:
        return np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32)

ref_full = read(r'F:\uploads\MovieProjectDir\_temp_sync\Life_of_Brian_REF_mono8k.wav')
ad_full = read(r'F:\uploads\MovieProjectDir\_temp_sync\Life_of_Brian_AD_mono8k.wav')

# Downsample to 500 Hz (factor of 16)
sr_coarse = 500
ref_coarse = decimate(ref_full, 16)
ad_coarse = decimate(ad_full, 16)

print(f"Ref coarse: {len(ref_coarse)} samples ({len(ref_coarse)/sr_coarse:.1f}s)", flush=True)
print(f"AD coarse: {len(ad_coarse)} samples ({len(ad_coarse)/sr_coarse:.1f}s)", flush=True)

# Test multiple 60-second chunks from reference against the entire AD track
for t in [300, 600, 900, 1200, 1500, 1800, 2400, 3000]:
    b = ref_coarse[int(t*sr_coarse):int((t+60)*sr_coarse)]
    corr = fftconvolve(ad_coarse, b[::-1], mode='valid')
    idx = np.argmax(corr)
    peak = corr[idx] / (np.linalg.norm(b) * np.linalg.norm(ad_coarse[idx:idx+len(b)]) + 1e-9)
    # AD timestamp where chunk starts:
    ad_t = idx / sr_coarse
    off = ad_t - t
    print(f"Ref @ {t:4d}s -> Best match in AD @ {ad_t:6.1f}s (offset = {off:+7.1f}s) | Peak = {peak:.4f}", flush=True)
