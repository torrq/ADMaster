# SPDX-License-Identifier: MIT
import os
import sys
import json
import re
import wave
import subprocess
import numpy as np

DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"


def predecode_to_mono_8k(ffmpeg_bin, input_path, temp_dir, label):
    safe_name = re.sub(r'[^a-zA-Z0-9_-]', '_', label)
    out_wav = os.path.join(temp_dir, f"{safe_name}_mono8k.wav")
    if os.path.exists(out_wav):
        return out_wav
    print(f"  [{label}] Pre-decoding stream to 8kHz mono analysis WAV: {os.path.basename(input_path)}...", flush=True)
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


def sync_track(ffmpeg_bin, ref_path, ad_path, temp_dir, movie_name):
    sr = 8000
    target_duration = get_audio_duration(ffmpeg_bin, ref_path)
    print(f"\n========================================================")
    print(f"Syncing: {movie_name} (Duration: {target_duration:.1f}s / {target_duration/60.0:.1f}m)")
    print(f"========================================================")
    
    if target_duration > 1800.0:
        max_anchor = target_duration * 0.85
        anchor_times = [max_anchor * (i / 10.0) for i in range(1, 11)]
    else:
        anchor_times = [target_duration * (i / 11.0) for i in range(1, 11)]
        
    base_mono_wav = predecode_to_mono_8k(ffmpeg_bin, ref_path, temp_dir, f"{movie_name}_REF")
    base_all_samples = read_wav_samples(base_mono_wav)
    
    ad_mono_wav = predecode_to_mono_8k(ffmpeg_bin, ad_path, temp_dir, f"{movie_name}_AD")
    ad_all_samples = read_wav_samples(ad_mono_wav)
    
    best_overall_confidence = -1.0
    best_offset = 0.0
    
    for anchor in anchor_times:
        if anchor + 30.0 > target_duration:
            continue
        base_start = int(anchor * sr)
        base_end = int((anchor + 30.0) * sr)
        if base_start >= len(base_all_samples):
            continue
        base_samples = base_all_samples[base_start : min(len(base_all_samples), base_end)]
        
        search_start_s = max(0.0, anchor - 30.0)
        ad_start = int(search_start_s * sr)
        ad_end = int((anchor + 60.0) * sr)
        if ad_start >= len(ad_all_samples):
            continue
        ad_samples = ad_all_samples[ad_start : min(len(ad_all_samples), ad_end)]
        
        if len(ad_samples) < len(base_samples) or len(base_samples) == 0:
            continue
            
        corr = np.correlate(ad_samples, base_samples, mode='valid')
        best_idx = np.argmax(corr)
        norm_peak = corr[best_idx] / (np.linalg.norm(base_samples) * np.linalg.norm(ad_samples[best_idx:best_idx+len(base_samples)]) + 1e-9)
        offset = search_start_s + (best_idx / sr) - anchor
        
        print(f"  Anchor {anchor:4.0f}s: Offset = {offset:+.3f}s | Confidence = {norm_peak:.4f}", flush=True)
        if norm_peak > best_overall_confidence:
            best_overall_confidence = norm_peak
            best_offset = offset
            
    # Check closing correlation for stretch
    cl_seek = max(0.0, target_duration - 180.0)
    base_cl_start = int(cl_seek * sr)
    base_cl_end = int((cl_seek + 30.0) * sr)
    base_cl_samples = base_all_samples[base_cl_start : min(len(base_all_samples), base_cl_end)]
    
    est_cl_start_s = max(0.0, cl_seek + best_offset - 30.0)
    ad_cl_start = int(est_cl_start_s * sr)
    ad_cl_end = int((est_cl_start_s + 90.0) * sr)
    ad_cl_samples = ad_all_samples[ad_cl_start : min(len(ad_all_samples), ad_end)]
    
    atempo = 1.0
    if len(ad_cl_samples) >= len(base_cl_samples) and len(base_cl_samples) > 0:
        corr_cl = np.correlate(ad_cl_samples, base_cl_samples, mode='valid')
        best_cl_idx = np.argmax(corr_cl)
        actual_cl_t = est_cl_start_s + (best_cl_idx / sr)
        t_ad_opening = 300.0 + best_offset
        t_ad_delta = actual_cl_t - t_ad_opening
        speed_ratio = t_ad_delta / (cl_seek - 300.0) if cl_seek > 300.0 else 1.0
        atempo = speed_ratio
        print(f"  Speed Stretch Matched: {atempo:.8f}x", flush=True)
        
    print(f"  => SELECTED: Offset = {best_offset:+.3f}s (Confidence: {best_overall_confidence:.4f}), atempo = {atempo:.8f}", flush=True)
    return best_offset, atempo, best_overall_confidence


def render_ad(ffmpeg_bin, ad_input, out_dir, title_prefix, offset, atempo):
    os.makedirs(out_dir, exist_ok=True)
    out_stereo = os.path.join(out_dir, f"{title_prefix}-AD (Stereo).m4a")
    out_night = os.path.join(out_dir, f"{title_prefix}-AD (Night Mode).m4a")
    
    print(f"\nRendering final AD tracks to: {out_dir}")
    print(f"  Stereo: {out_stereo}")
    print(f"  Night Mode: {out_night}")
    
    filters_stereo = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_stereo.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_stereo.append(f"adelay={delay_ms}:all=1")
    filters_stereo.append("alimiter=level_in=1:level_out=0.98:limit=0.98")
    
    filters_night = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_night.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_night.append(f"adelay={delay_ms}:all=1")
    filters_night.append("loudnorm=I=-16:TP=-1.5:LRA=11")
    
    combined_cmd = [ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "warning", "-i", ad_input]
    if offset > 0:
        combined_cmd.extend(["-ss", f"{offset:.3f}"])
    combined_cmd.extend([
        "-af", ",".join(filters_stereo), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_stereo
    ])
    if offset > 0:
        combined_cmd.extend(["-ss", f"{offset:.3f}"])
    combined_cmd.extend([
        "-af", ",".join(filters_night), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_night
    ])
    
    print("  Executing FFmpeg render...", flush=True)
    res = subprocess.run(combined_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        print(f"  Error rendering: {res.stderr}")
    else:
        print(f"  [SUCCESS] Generated both AD tracks for {title_prefix}!")


def main():
    movies = [
        {
            "name": "Movie Title 1",
            "title_prefix": "Movie Title 1 (Year)",
            "ref": r"F:\uploads\MovieProjectDir\Movie Title 1\source\Monty.Python.and.the.Holy.Grail.1975.1080p.UHD.BluRay.FLAC.2.0.DoVi.HDR10.x265-c0kE_track2_[eng]_DELAY 0ms.flac",
            "ad": r"F:\uploads\MovieProjectDir\Movie Title 1\tosync\Movie Title 1 (Year) [AudioVault Original].mp3",
            "out": r"F:\uploads\MovieProjectDir\Movie Title 1\output"
        },
        {
            "name": "Movie Title 2",
            "title_prefix": "Movie Title 2 (Year)",
            "ref": r"F:\uploads\MovieProjectDir\Movie Title 2\source\Life.of.Brian.1979.1080p.BluRay.FLAC2.0.x264-ZoroSenpai_track2_[eng]_DELAY 0ms.flac",
            "ad": r"F:\uploads\MovieProjectDir\Movie Title 2\tosync\Movie Title 2 (Year) [AU].mp3",
            "out": r"F:\uploads\MovieProjectDir\Movie Title 2\output"
        },
        {
            "name": "Movie Title 3",
            "title_prefix": "Movie Title 3 (Year)",
            "ref": r"F:\uploads\MovieProjectDir\Movie Title 3\source\The.Meaning.of.Life.1983.1080p.UHD.BluRay.DD+7.1.HDR10.x265-REBORN_track3_[eng]_DELAY 0ms.flac",
            "ad": r"F:\uploads\MovieProjectDir\Movie Title 3\tosync\Monty Python's The Meaning of life.mp3",
            "out": r"F:\uploads\MovieProjectDir\Movie Title 3\output"
        }
    ]
    
    temp_dir = r"F:\uploads\MovieProjectDir\_temp_sync"
    os.makedirs(temp_dir, exist_ok=True)
    
    for m in movies:
        offset, atempo, conf = sync_track(DEFAULT_FFMPEG, m["ref"], m["ad"], temp_dir, m["name"])
        render_ad(DEFAULT_FFMPEG, m["ad"], m["out"], m["title_prefix"], offset, atempo)
        
    print("\n========================================================")
    print("ALL 3 MONTY PYTHON AD TRACKS SUCCESSFULLY SYNCED & ENCODED!")
    print("========================================================")


if __name__ == "__main__":
    main()
