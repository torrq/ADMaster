import numpy as np
import wave
import sys
from scipy.signal import fftconvolve

def read(p):
    with wave.open(p, 'rb') as wf:
        return np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32)

print("Loading waves...", flush=True)
ref = read(r'F:\uploads\MovieProjectDir\_temp_sync\Life_of_Brian_REF_mono8k.wav')
ad = read(r'F:\uploads\MovieProjectDir\_temp_sync\Life_of_Brian_AD_mono8k.wav')
print(f"Ref length: {len(ref)/8000:.1f}s, AD length: {len(ad)/8000:.1f}s", flush=True)

sr = 8000
for t in [60, 120, 180, 240, 300]:
    b = ref[int(t*sr):int((t+20)*sr)]
    # Search first 300s of AD
    a = ad[0:int(300*sr)]
    # FFT correlation
    corr = fftconvolve(a, b[::-1], mode='valid')
    idx = np.argmax(corr)
    peak = corr[idx] / (np.linalg.norm(b) * np.linalg.norm(a[idx:idx+len(b)]) + 1e-9)
    off = (idx / sr) - t
    print(f"Anchor {t:3d}s: off={off:+.3f}s, peak={peak:.4f}", flush=True)
