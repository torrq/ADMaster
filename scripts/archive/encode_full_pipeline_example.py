# SPDX-License-Identifier: MIT
import os
import sys
import subprocess
import numpy as np
import wave
import re
from scipy.signal import fftconvolve

DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"

def predecode_to_mono_8k(ffmpeg_bin, input_path, scratch_dir, label):
    os.makedirs(scratch_dir, exist_ok=True)
    base_stem = os.path.splitext(os.path.basename(input_path))[0]
    safe_name = re.sub(r'[^a-zA-Z0-9_-]', '_', f"{label}_{base_stem}")[:64]
    out_wav = os.path.join(scratch_dir, f"{safe_name}_mono8k.wav")
    if os.path.exists(out_wav):
        return out_wav
    print(f"  [{label}] Pre-decoding to 8kHz mono analysis WAV in scratch: {os.path.basename(input_path)}...", flush=True)
    cmd = [
        ffmpeg_bin, "-y", "-i", input_path, "-vn",
        "-af", "aresample=8000,pan=mono|c0=c0",
        "-c:a", "pcm_s16le", out_wav
    ]
    res = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if res.returncode != 0 or not os.path.exists(out_wav):
        raise RuntimeError(f"Failed to pre-decode audio '{input_path}' to 8kHz WAV")
    return out_wav

def read_wav_samples(wav_path):
    with wave.open(wav_path, 'rb') as wf:
        nframes = wf.getnframes()
        raw_data = wf.readframes(nframes)
        samples = np.frombuffer(raw_data, dtype=np.int16).astype(np.float32)
    return samples

def get_audio_duration(ffmpeg_bin, input_path):
    ffprobe_bin = os.path.join(os.path.dirname(ffmpeg_bin), "ffprobe.exe" if os.name == 'nt' else "ffprobe")
    if not os.path.exists(ffprobe_bin):
        ffprobe_bin = "ffprobe"
    cmd = [
        ffprobe_bin, "-v", "error", "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1", input_path
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return float(res.stdout.strip())

def main():
    working_dir = r"F:\uploads\MovieProjectDir"
    scratch_dir = os.path.join(working_dir, "scratch")
    out_dir = os.path.join(working_dir, "output")
    
    os.makedirs(scratch_dir, exist_ok=True)
    os.makedirs(out_dir, exist_ok=True)
    
    ref_path = os.path.join(working_dir, "source", "Movie.Title.1992.BluRay.DTS.x264")
    ad_path = os.path.join(working_dir, "tosync", "Movie Title (1992)")
    
    print("================================================================================")
    print("ALIENT 3 SPECIAL ASSEMBLY CUT - ACOUSTIC SYNC & ENCODE PIPELINE")
    print(f"Working Dir: {working_dir}")
    print(f"Scratch Dir: {scratch_dir}")
    print(f"Output Dir:  {out_dir}")
    print("================================================================================")
    
    target_duration = get_audio_duration(DEFAULT_FFMPEG, ref_path)
    print(f"Reference Duration: {target_duration:.3f}s ({target_duration/3600.0:.2f} hours)\n")
    
    sr = 8000
    dur_minutes = target_duration / 60.0
    num_anchors = int(np.clip(dur_minutes / 3.5, 20, 40))
    anchor_times = np.linspace(target_duration * 0.05, target_duration * 0.88, num_anchors)
    print(f"Generated {num_anchors} analysis anchors across the timeline (~every {target_duration*0.83/(num_anchors*60):.1f} mins).", flush=True)
    
    ref_mono_wav = predecode_to_mono_8k(DEFAULT_FFMPEG, ref_path, scratch_dir, "REF")
    ref_samples = read_wav_samples(ref_mono_wav)
    
    ad_mono_wav = predecode_to_mono_8k(DEFAULT_FFMPEG, ad_path, scratch_dir, "AD")
    ad_samples = read_wav_samples(ad_mono_wav)
    
    print("\nComputing multi-anchor FFT cross-correlation in memory...", flush=True)
    measurements = []
    for anchor in anchor_times:
        if anchor + 30.0 > target_duration:
            continue
        base_start = int(anchor * sr)
        base_end = int((anchor + 30.0) * sr)
        if base_start >= len(ref_samples):
            continue
        base_chunk = ref_samples[base_start : min(len(ref_samples), base_end)]
        
        search_start_s = max(0.0, anchor - 30.0)
        ad_start = int(search_start_s * sr)
        ad_end = int((anchor + 60.0) * sr)
        if ad_start >= len(ad_samples):
            continue
        ad_chunk = ad_samples[ad_start : min(len(ad_samples), ad_end)]
        
        if len(ad_chunk) < len(base_chunk) or len(base_chunk) == 0:
            continue
            
        corr = fftconvolve(ad_chunk, base_chunk[::-1], mode='valid')
        best_idx = np.argmax(corr)
        norm_peak = corr[best_idx] / (np.linalg.norm(base_chunk) * np.linalg.norm(ad_chunk[best_idx:best_idx+len(base_chunk)]) + 1e-9)
        offset = search_start_s + (best_idx / sr) - anchor
        measurements.append((offset, norm_peak))
        print(f"  Anchor {anchor:4.0f}s: Offset = {offset:+.3f}s | Confidence = {norm_peak:.4f}", flush=True)
        
    high_conf = [off for off, peak in measurements if peak >= 0.20]
    if high_conf:
        final_offset = float(np.median(high_conf))
    else:
        final_offset = max(measurements, key=lambda x: x[1])[0]
        
    best_conf = max([p for _, p in measurements])
    print(f"\n=> SELECTED CONSENSUS: Offset = {final_offset:+.3f}s (Max Confidence: {best_conf:.4f}, Median of {len(high_conf)} points)", flush=True)
    
    # Render Main Tracks
    print("\n========================================================")
    print("1. Rendering Main DTS 5.1 -> AC3 5.1, Stereo AAC, Night Mode AAC...")
    print("========================================================")
    main_ac3 = os.path.join(out_dir, "Movie Title (1992)")
    main_stereo = os.path.join(out_dir, "Movie Title (1992)")
    main_night = os.path.join(out_dir, "Movie Title (1992)")
    
    pan_filter = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"
    cmd_main = [
        DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning", "-i", ref_path,
        "-c:a", "ac3", "-b:a", "384k", main_ac3,
        "-af", f"aresample=48000,{pan_filter},alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", main_stereo,
        "-af", f"aresample=48000,{pan_filter},loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", main_night
    ]
    res_main = subprocess.run(cmd_main, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_main.returncode == 0:
        print("  [SUCCESS] Main audio tracks rendered successfully!", flush=True)
    else:
        print(f"  [ERROR] Main encode failed: {res_main.stderr}", flush=True)
        
    # Render AD Tracks
    print("\n========================================================")
    print("2. Rendering Synced AD Tracks -> Stereo AAC, Night Mode AAC...")
    print("========================================================")
    ad_stereo = os.path.join(out_dir, "Movie Title (1992)")
    ad_night = os.path.join(out_dir, "Movie Title (1992)")
    
    cmd_ad = [DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning", "-i", ad_path]
    if final_offset > 0:
        cmd_ad.extend(["-ss", f"{final_offset:.3f}"])
    cmd_ad.extend([
        "-vn", "-af", "aresample=48000,alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", ad_stereo
    ])
    if final_offset > 0:
        cmd_ad.extend(["-ss", f"{final_offset:.3f}"])
    cmd_ad.extend([
        "-vn", "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", ad_night
    ])
    
    res_ad = subprocess.run(cmd_ad, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_ad.returncode == 0:
        print("  [SUCCESS] Synced AD tracks rendered successfully!", flush=True)
    else:
        print(f"  [ERROR] AD encode failed: {res_ad.stderr}", flush=True)
        
    print("\n================================================================================")
    print("ALL ALIEN 3 ASSEMBLY CUT AUDIO PROCESSING COMPLETED SUCCESSFULLY!")
    print("================================================================================")

if __name__ == "__main__":
    main()
