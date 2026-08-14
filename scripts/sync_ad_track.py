#!/usr/bin/env python3
r"""
================================================================================
Universal Audio Description (AD) & Main Audio Sync & Encoding Pipeline
================================================================================
Author: Antigravity AI Pair Programmer
Repository: AudioDescriptionStuff

Overview:
  High-performance acoustic cross-correlation alignment, 2-pass EBU R128
  loudness normalization, multi-channel surround conversion, downmixing,
  and automatic MKV video sample multiplexing.

Usage Examples:
  1. Full movie sync & encode (AD + Main Audio):
     python sync_ad_track.py \
       -r "F:\uploads\Aliens3\source\Alien.3.1992.Special.Assembly.Cut.dts" \
       -a "F:\uploads\Aliens3\tosync\Alien 3 (1992) [Assembly Cut AudioVault Original] AD.mp3" \
       --convert-main

  2. Fast 3-minute video test MKV sample multiplexed with video:
     python sync_ad_track.py \
       -r "reference.eac3" \
       -a "ad_track.mp3" \
       -m "movie.mkv" \
       --preview 180
"""

import argparse
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import wave
import numpy as np

try:
    from scipy.signal import fftconvolve
except ImportError:
    def fftconvolve(a, b, mode='valid'):
        return np.correlate(a, b, mode=mode)

import shutil

# Portable local FFmpeg fallback
DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = shutil.which("ffmpeg") or "ffmpeg"


def locate_binaries(ffmpeg_path):
    """Ensure ffmpeg and ffprobe binaries are located and executable."""
    if ffmpeg_path and os.path.exists(ffmpeg_path):
        ffprobe_path = os.path.join(os.path.dirname(ffmpeg_path), "ffprobe.exe" if os.name == 'nt' else "ffprobe")
        if not os.path.exists(ffprobe_path):
            ffprobe_path = shutil.which("ffprobe") or "ffprobe"
    else:
        ffmpeg_path = shutil.which("ffmpeg") or "ffmpeg"
        ffprobe_path = shutil.which("ffprobe") or "ffprobe"
    return ffmpeg_path, ffprobe_path


def get_audio_info(ffprobe_bin, input_path):
    """Retrieve audio duration and channel count using ffprobe."""
    cmd = [
        ffprobe_bin, "-v", "error", "-select_streams", "a:0",
        "-show_entries", "format=duration:stream=channels,channel_layout",
        "-of", "json", input_path
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"Failed to probe audio for '{input_path}': {res.stderr}")
    data = json.loads(res.stdout)
    duration = float(data.get("format", {}).get("duration", 0.0))
    channels = 2
    if "streams" in data and len(data["streams"]) > 0:
        channels = int(data["streams"][0].get("channels", 2))
    return duration, channels


def predecode_to_mono_8k(ffmpeg_bin, input_path, scratch_dir, label):
    """Pre-decode any audio/video stream to an 8kHz mono PCM WAV for instantaneous in-memory correlation."""
    os.makedirs(scratch_dir, exist_ok=True)
    base_stem = os.path.splitext(os.path.basename(input_path))[0]
    safe_name = re.sub(r'[^a-zA-Z0-9_-]', '_', f"{label}_{base_stem}")[:64]
    out_wav = os.path.join(scratch_dir, f"{safe_name}_mono8k.wav")
    tmp_wav = os.path.join(scratch_dir, f"{safe_name}_mono8k_tmp.wav")
    
    if os.path.exists(out_wav):
        if os.path.getsize(out_wav) > 1024:
            return out_wav
        else:
            try:
                os.remove(out_wav)
            except Exception:
                pass
                
    print(f"  [{label}] Pre-decoding stream to 8kHz mono analysis WAV: {os.path.basename(input_path)}...", flush=True)
    cmd = [
        ffmpeg_bin, "-y", "-i", input_path, "-vn",
        "-af", "aresample=8000,pan=mono|c0=c0",
        "-c:a", "pcm_s16le", "-f", "wav", tmp_wav
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0 or not os.path.exists(tmp_wav):
        if os.path.exists(tmp_wav):
            try:
                os.remove(tmp_wav)
            except Exception:
                pass
        raise RuntimeError(f"Failed to pre-decode audio '{input_path}' to 8kHz WAV: {res.stderr.strip()}")
    os.replace(tmp_wav, out_wav)
    return out_wav


def read_wav_samples(wav_path):
    """Read float32 PCM samples from a WAV file."""
    with wave.open(wav_path, 'rb') as wf:
        nframes = wf.getnframes()
        raw_data = wf.readframes(nframes)
        samples = np.frombuffer(raw_data, dtype=np.int16).astype(np.float32)
    return samples


def find_offset_candidates(measurements, min_conf=0.15, cluster_tol=0.03):
    """Clusters anchor measurements to find the consensus winner and runner-up candidate offsets."""
    valid = [(off, peak) for off, peak in measurements if peak >= min_conf]
    if not valid:
        valid = measurements
    if not valid:
        return [(0.0, 0.0, 0)]
        
    clusters = []
    for off, peak in sorted(valid, key=lambda x: x[1], reverse=True):
        matched = False
        for cl in clusters:
            if abs(cl["offset"] - off) <= cluster_tol:
                cl["points"].append((off, peak))
                cl["count"] += 1
                if peak > cl["max_peak"]:
                    cl["max_peak"] = peak
                matched = True
                break
        if not matched:
            clusters.append({
                "offset": off,
                "max_peak": peak,
                "count": 1,
                "points": [(off, peak)]
            })
            
    for cl in clusters:
        cl_offsets = [p[0] for p in cl["points"]]
        cl["median_offset"] = float(np.median(cl_offsets))
        
    # Sort by anchor count first (consensus), then by maximum peak confidence
    clusters.sort(key=lambda x: (x["count"], x["max_peak"]), reverse=True)
    return [(cl["median_offset"], cl["max_peak"], cl["count"]) for cl in clusters]


def compute_sync_parameters(ffmpeg_bin, base_path, ad_tracks, target_duration, scratch_dir, num_anchors=100, force_stretch=False, no_stretch=False):
    """Computes delay offset via high-confidence multi-window FFT cross-correlation in memory."""
    print("\n=== COMPUTING ACOUSTIC CROSS-CORRELATION SYNC PARAMETERS ===", flush=True)
    
    anchor_times = np.linspace(target_duration * 0.05, target_duration * 0.88, num_anchors)
    print(f"  Generated {num_anchors} analysis anchors across the timeline (~every {target_duration*0.83/(num_anchors*60):.1f} mins).", flush=True)
    sync_results = {}
    
    # Pre-decode reference audio ONCE
    base_mono_wav = predecode_to_mono_8k(ffmpeg_bin, base_path, scratch_dir, "REF")
    base_all_samples = read_wav_samples(base_mono_wav)
    sr = 8000
    
    for name, path in ad_tracks.items():
        print(f"\nAnalyzing acoustic correlation for [{name}]...", flush=True)
        ad_mono_wav = predecode_to_mono_8k(ffmpeg_bin, path, scratch_dir, name)
        ad_all_samples = read_wav_samples(ad_mono_wav)
        
        measurements = []
        for anchor in anchor_times:
            if anchor + 30.0 > target_duration:
                continue
                
            base_start_idx = int(anchor * sr)
            base_end_idx = int((anchor + 30.0) * sr)
            if base_start_idx >= len(base_all_samples):
                continue
            base_samples = base_all_samples[base_start_idx : min(len(base_all_samples), base_end_idx)]
            
            # Search window from AD track: anchor-30s to anchor+60s (90s duration)
            search_start_s = max(0.0, anchor - 30.0)
            search_end_s = anchor + 60.0
            ad_start_idx = int(search_start_s * sr)
            ad_end_idx = int(search_end_s * sr)
            if ad_start_idx >= len(ad_all_samples):
                continue
            ad_samples = ad_all_samples[ad_start_idx : min(len(ad_all_samples), ad_end_idx)]
            
            if len(ad_samples) < len(base_samples) or len(base_samples) == 0:
                continue
                
            corr = fftconvolve(ad_samples, base_samples[::-1], mode='valid')
            best_idx = np.argmax(corr)
            norm_peak = corr[best_idx] / (np.linalg.norm(base_samples) * np.linalg.norm(ad_samples[best_idx:best_idx+len(base_samples)]) + 1e-9)
            
            offset = search_start_s + (best_idx / sr) - anchor
            measurements.append((offset, norm_peak))
            print(f"  Anchor {anchor:4.0f}s: Measured Offset = {offset:+.3f}s | Confidence = {norm_peak*100:5.1f}% ({norm_peak:.4f})", flush=True)
            
        candidates = find_offset_candidates(measurements)
        winner_offset, winner_peak, winner_count = candidates[0]
        init_offset = winner_offset
        best_overall_confidence = winner_peak
        atempo = 1.0
        
        is_pal_track = ("uk" in name.lower() or "pal" in name.lower() or "25fps" in name.lower())
        
        if (is_pal_track or force_stretch) and not no_stretch:
            cl_seek = max(0.0, target_duration - 180.0)
            base_cl_start = int(cl_seek * sr)
            base_cl_end = int((cl_seek + 30.0) * sr)
            base_cl_samples = base_all_samples[base_cl_start : min(len(base_all_samples), base_cl_end)]
            
            est_cl_start_s = max(0.0, (cl_seek / 1.042709) + init_offset - 30.0 if is_pal_track else cl_seek + init_offset - 30.0)
            ad_cl_start = int(est_cl_start_s * sr)
            ad_cl_end = int((est_cl_start_s + 90.0) * sr)
            ad_cl_samples = ad_all_samples[ad_cl_start : min(len(ad_all_samples), ad_cl_end)]
            
            if len(ad_cl_samples) >= len(base_cl_samples) and len(base_cl_samples) > 0:
                corr_cl = fftconvolve(ad_cl_samples, base_cl_samples[::-1], mode='valid')
                best_cl_idx = np.argmax(corr_cl)
                actual_cl_t = est_cl_start_s + (best_cl_idx / sr)
                
                t_ad_opening = 300.0 + init_offset
                t_ad_delta = actual_cl_t - t_ad_opening
                speed_ratio = t_ad_delta / (cl_seek - 300.0) if cl_seek > 300.0 else 1.0
                computed_atempo = speed_ratio
                
                if 1.030 <= computed_atempo <= 1.050 or force_stretch:
                    atempo = computed_atempo
                    print(f"  [{name}] Speed Stretch Matched: {atempo:.6f}x", flush=True)
                else:
                    print(f"  [{name}] Speed ratio {computed_atempo:.6f}x outside standard range. Reverting to 1.000000x.", flush=True)
        else:
            print(f"  [{name}] Native rate (1.000000x) enforced.", flush=True)
            
        sync_results[name] = {
            "offset": init_offset,
            "atempo": atempo,
            "confidence": best_overall_confidence
        }
        print(f"\n  => SELECTED CONSENSUS OFFSET for [{name}]: {init_offset:+.3f}s (Max Confidence: {best_overall_confidence*100:.1f}% / {best_overall_confidence:.4f} | {winner_count}/{len(measurements)} anchors)", flush=True)
        
        # Display up to 2 runner-up offsets if distinct
        runner_up_idx = 1
        for r_off, r_peak, r_count in candidates[1:]:
            if abs(r_off - init_offset) > 0.01:
                print(f"     Runner-up #{runner_up_idx:<15}: {r_off:+.3f}s (Max Confidence: {r_peak*100:.1f}% / {r_peak:.4f} | {r_count}/{len(measurements)} anchors)", flush=True)
                runner_up_idx += 1
                if runner_up_idx > 2:
                    break
        print("", flush=True)
        
    return sync_results


def render_main_tracks(ffmpeg_bin, ref_path, base_title, out_dir, channels, preview_sec=None):
    """Renders standard converted formats for Main Audio (Surround AC3, Stereo AAC, Night Mode AAC)."""
    print("\n=== RENDERING MAIN AUDIO TRACKS ===", flush=True)
    os.makedirs(out_dir, exist_ok=True)
    
    suffix = f" (Preview {int(preview_sec)}s)" if preview_sec else ""
    pan_stereo = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"
    
    if channels >= 6:
        out_surround = os.path.join(out_dir, f"{base_title} (Surround){suffix}.ac3")
        out_stereo = os.path.join(out_dir, f"{base_title} (Stereo){suffix}.m4a")
        out_night = os.path.join(out_dir, f"{base_title} (Night Mode){suffix}.m4a")
        
        cmd = [ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "warning", "-i", ref_path]
        if preview_sec:
            cmd.extend(["-t", f"{preview_sec:.3f}"])
        cmd.extend([
            "-c:a", "ac3", "-b:a", "384k", out_surround,
            "-af", f"aresample=48000,{pan_stereo},alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_stereo,
            "-af", f"aresample=48000,{pan_stereo},loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_night
        ])
    else:
        out_stereo = os.path.join(out_dir, f"{base_title} (Stereo){suffix}.m4a")
        out_night = os.path.join(out_dir, f"{base_title} (Night Mode){suffix}.m4a")
        
        cmd = [ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "warning", "-i", ref_path]
        if preview_sec:
            cmd.extend(["-t", f"{preview_sec:.3f}"])
        cmd.extend([
            "-af", "aresample=48000,alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_stereo,
            "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_night
        ])
        
    print(f"  Executing FFmpeg main audio render...", flush=True)
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        print(f"  Error rendering main audio: {res.stderr}", flush=True)
    else:
        print(f"  [SUCCESS] Main audio tracks rendered to {out_dir}!", flush=True)


def mux_video_sample(ffmpeg_bin, video_src, audio_src, out_mkv, render_duration):
    """Multiplexes generated AD track with video stream using zero-re-encode stream copy (-c:v copy)."""
    print(f"  Multiplexing Video Sample MKV ({render_duration:.0f}s)...", flush=True)
    cmd_mux = [
        ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "warning",
        "-ss", "0", "-t", f"{render_duration:.3f}", "-i", video_src,
        "-i", audio_src,
        "-map", "0:v:0", "-map", "1:a:0",
        "-c:v", "copy", "-c:a", "copy",
        "-metadata:s:a:0", "title=Audio Description (Synced)",
        "-disposition:a:0", "default",
        out_mkv
    ]
    res = subprocess.run(cmd_mux, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode == 0 and os.path.exists(out_mkv):
        size_mb = os.path.getsize(out_mkv) / (1024 * 1024)
        print(f"  [SUCCESS] Saved MKV Video Sample: {out_mkv} ({size_mb:.1f} MB)", flush=True)
    else:
        print(f"  Warning: MKV Video Muxing failed: {res.stderr}", flush=True)


def render_ad_tracks(ffmpeg_bin, base_title, ad_tracks, sync_params, target_duration, out_dir, target_i=-16.0, target_tp=-1.5, preview_sec=None, video_mux_path=None):
    """Renders synced AD tracks: Stereo AAC (alimiter) and Night Mode AAC (loudnorm)."""
    print("\n=== RENDERING AUDIO DESCRIPTION TRACKS ===", flush=True)
    os.makedirs(out_dir, exist_ok=True)
    
    render_duration = target_duration if not preview_sec else min(preview_sec, target_duration)
    suffix = f" (Preview {int(render_duration)}s)" if preview_sec else ""
    
    for name, path in ad_tracks.items():
        params = sync_params[name]
        offset = params["offset"]
        atempo = params["atempo"]
        
        label_prefix = f"{base_title}-AD" if len(ad_tracks) == 1 else f"{base_title}-AD [{name}]"
        out_stereo = os.path.join(out_dir, f"{label_prefix} (Stereo){suffix}.m4a")
        out_night = os.path.join(out_dir, f"{label_prefix} (Night Mode){suffix}.m4a")
        
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
        filters_night.append(f"loudnorm=I={target_i}:TP={target_tp}:LRA=11")
        
        cmd = [ffmpeg_bin, "-y", "-hide_banner", "-loglevel", "warning", "-i", path]
        if offset > 0:
            cmd.extend(["-ss", f"{offset:.3f}"])
        if preview_sec:
            cmd.extend(["-t", f"{preview_sec:.3f}"])
        cmd.extend([
            "-vn", "-af", ",".join(filters_stereo), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_stereo
        ])
        if offset > 0:
            cmd.extend(["-ss", f"{offset:.3f}"])
        if preview_sec:
            cmd.extend(["-t", f"{preview_sec:.3f}"])
        cmd.extend([
            "-vn", "-af", ",".join(filters_night), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", out_night
        ])
        
        print(f"  Executing FFmpeg AD render for [{name}]...", flush=True)
        res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        if res.returncode != 0:
            print(f"  Error rendering AD: {res.stderr}", flush=True)
        else:
            print(f"  [SUCCESS] AD tracks rendered for [{name}]!", flush=True)
            if video_mux_path and os.path.exists(video_mux_path):
                out_mkv = os.path.join(out_dir, f"{label_prefix} (Synced){suffix}.mkv")
                mux_video_sample(ffmpeg_bin, video_mux_path, out_stereo, out_mkv, render_duration)


def main():
    parser = argparse.ArgumentParser(
        description="Universal Audio Description (AD) & Main Audio Sync & Encoding Tool",
        formatter_class=argparse.RawTextHelpFormatter
    )
    parser.add_argument("-r", "--ref", required=True, help="Path to reference main audio/video file (AC3, EAC3, DTS, MKV, etc.)")
    parser.add_argument("-a", "--ad", required=True, nargs="+", help="Audio Description track(s). Format: 'Path' or 'TrackLabel=Path'")
    parser.add_argument("-m", "--mux-video", default=None, help="Path to reference MKV/MP4 video file for multiplexing sample video MKV")
    parser.add_argument("-o", "--outdir", default=None, help="Output directory (Defaults to <movie_dir>/output)")
    parser.add_argument("-t", "--title", default=None, help="Movie title prefix for output filenames")
    parser.add_argument("-p", "--preview", type=float, nargs="?", const=300.0, default=None, help="Fast sync test: Render a short preview sample (Default: 300s)")
    parser.add_argument("--convert-main", action="store_true", help="Also convert and downmix reference main audio (Surround AC3, Stereo AAC, Night Mode AAC)")
    parser.add_argument("--no-render", action="store_true", help="Calculate sync parameters only, do not render output files")
    parser.add_argument("--anchors", type=int, default=100, help="Number of correlation analysis anchors across timeline (Default: 100)")
    parser.add_argument("--no-stretch", action="store_true", help="Force native 1.0x speed rate without speed stretch")
    parser.add_argument("--force-stretch", action="store_true", help="Force 2-point closing cross-correlation speed stretch search")
    parser.add_argument("--target-i", type=float, default=-16.0, help="Target EBU R128 integrated loudness in LUFS (Default: -16.0)")
    parser.add_argument("--target-tp", type=float, default=-1.5, help="Target true-peak ceiling in dB (Default: -1.5)")
    parser.add_argument("--scratch-dir", default=None, help="Directory for temporary analysis files (Defaults to <movie_dir>/scratch)")
    parser.add_argument("--keep-scratch", action="store_true", help="Do not delete temporary analysis WAVs on completion")
    parser.add_argument("--ffmpeg", default=DEFAULT_FFMPEG, help="Path to ffmpeg executable")
    
    args = parser.parse_args()
    ffmpeg_bin, ffprobe_bin = locate_binaries(args.ffmpeg)
    
    if not os.path.exists(args.ref):
        print(f"Error: Reference file not found: '{args.ref}'")
        sys.exit(1)
        
    ref_abs = os.path.abspath(args.ref)
    ref_parent = os.path.dirname(ref_abs)
    if os.path.basename(ref_parent).lower() in ["source", "tosync", "output", "scratch"]:
        project_dir = os.path.dirname(ref_parent)
    else:
        project_dir = ref_parent
        
    out_dir = args.outdir if args.outdir else os.path.join(project_dir, "output")
    scratch_dir = args.scratch_dir if args.scratch_dir else os.path.join(project_dir, "scratch")
    
    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(scratch_dir, exist_ok=True)
    
    base_title = args.title
    if not base_title:
        base_title = os.path.splitext(os.path.basename(args.ref))[0]
        # Clean common release scene tags from default title
        base_title = re.sub(r'_(?:track\d+|DELAY.*)$', '', base_title, flags=re.IGNORECASE)
        
    ad_tracks = {}
    for idx, item in enumerate(args.ad):
        if "=" in item:
            label, path = item.split("=", 1)
        else:
            path = item
            label = os.path.splitext(os.path.basename(path))[0]
            
        if not os.path.exists(path):
            print(f"Error: AD track file not found: '{path}'")
            sys.exit(1)
        ad_tracks[label.strip()] = path.strip()
        
    print("================================================================================")
    print(f"Reference Audio : {args.ref}")
    print(f"Movie Title     : {base_title}")
    print(f"Project Dir     : {project_dir}")
    print(f"Scratch Dir     : {scratch_dir}")
    print(f"Output Dir      : {out_dir}")
    print(f"AD Tracks Count : {len(ad_tracks)}")
    if args.convert_main:
        print(f"Convert Main    : YES (Surround AC3 + Stereo AAC + Night Mode AAC)")
    if args.preview:
        print(f"Preview Mode    : ON ({args.preview:.0f} seconds)")
    print("================================================================================")
    
    target_duration, channels = get_audio_info(ffprobe_bin, args.ref)
    print(f"Reference Info  : Duration {target_duration:.3f}s ({target_duration/3600.0:.2f}h) | {channels} Channels")
    
    sync_params = compute_sync_parameters(
        ffmpeg_bin, args.ref, ad_tracks, target_duration, scratch_dir,
        num_anchors=args.anchors, force_stretch=args.force_stretch, no_stretch=args.no_stretch
    )
    
    if not args.no_render:
        if args.convert_main:
            render_main_tracks(ffmpeg_bin, args.ref, base_title, out_dir, channels, preview_sec=args.preview)
            
        render_ad_tracks(
            ffmpeg_bin, base_title, ad_tracks, sync_params, target_duration,
            out_dir, target_i=args.target_i, target_tp=args.target_tp,
            preview_sec=args.preview, video_mux_path=args.mux_video
        )
        
    if not args.keep_scratch:
        try:
            for f in os.listdir(scratch_dir):
                if f.endswith("_mono8k.wav"):
                    os.remove(os.path.join(scratch_dir, f))
        except Exception:
            pass
            
    print("\n================================================================================")
    print("SUCCESS: ALL AUDIO PROCESSING COMPLETED!")
    print("================================================================================")


if __name__ == "__main__":
    main()
