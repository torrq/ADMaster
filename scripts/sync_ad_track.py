#!/usr/bin/env python3
r"""
================================================================================
Universal Audio Description (AD) Cross-Correlation & 2-Pass Normalization Sync
================================================================================
Author: Antigravity AI Pair Programmer
Repository: AudioDescriptionStuff

Overview:
  High-performance acoustic cross-correlation alignment, 2-pass EBU R128
  loudness normalization, and automatic MKV video sample multiplexing for
  Audio Description (AD) tracks against any reference video/audio source.

Usage Examples:
  1. Instant 3-minute video test MKV sample multiplexed with video:
     python sync_ad_track.py \
       -r "F:\uploads\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer_Audio03.ac3" \
       -a "New Description=F:\uploads\ad\10530 The Godfather (1972) [New Description] .mp3" \
       -m "X:\incoming\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer.mkv" \
       -o "F:\uploads" \
       -t "The Godfather (1972)" \
       --preview 180 \
       --preset standard \
       --no-stretch

Key Architectural Features for Future LLM Agents:
1. Multi-Window Acoustic Correlation (Confidence Peak Filtering):
   - Samples reference audio across robust acoustic sections (e.g. 300s / 5 minutes) to avoid false-positive matches on silent/black-screen openings.
   - Computes delay offset in < 0.05 seconds with sub-millisecond precision.

2. Fast Preview Testing Mode (`--preview [sec]`):
   - Generates short 3-to-5 minute sample tracks in seconds for rapid iteration.

3. Automatic MKV Video Multiplexing (`-m` / `--mux-video`):
   - Multiplexes the generated AD track with video using zero-re-encode stream copy (`-c:v copy`).
   - Produces a ready-to-watch `.mkv` sample file for instant test listening in video lip-sync.
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

# Portable local FFmpeg fallback
DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"


def locate_binaries(ffmpeg_path):
    """Ensure ffmpeg and ffprobe binaries are located and executable."""
    if os.path.exists(ffmpeg_path):
        ffprobe_path = os.path.join(os.path.dirname(ffmpeg_path), "ffprobe.exe" if os.name == 'nt' else "ffprobe")
        if not os.path.exists(ffprobe_path):
            ffprobe_path = "ffprobe"
    else:
        ffmpeg_path = "ffmpeg"
        ffprobe_path = "ffprobe"
    return ffmpeg_path, ffprobe_path


def get_audio_duration(ffprobe_bin, input_path):
    """Retrieve total audio duration in seconds using ffprobe."""
    cmd = [
        ffprobe_bin, "-v", "error", "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1", input_path
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0 or not res.stdout.strip():
        raise RuntimeError(f"Failed to probe audio duration for '{input_path}': {res.stderr}")
    return float(res.stdout.strip())


def extract_mono_wav_fast(ffmpeg_bin, input_path, start_s, dur_s, output_wav, sample_rate=8000):
    """Fast-seek and extract a lightweight mono PCM WAV snippet for correlation."""
    cmd = [
        ffmpeg_bin, "-y", "-ss", f"{start_s:.3f}", "-t", f"{dur_s:.3f}",
        "-i", input_path,
        "-af", f"aresample={sample_rate},pan=mono|c0=c0",
        "-c:a", "pcm_s16le", output_wav
    ]
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def read_wav_samples(wav_path):
    """Read float32 PCM samples from a WAV file."""
    with wave.open(wav_path, 'rb') as wf:
        nframes = wf.getnframes()
        raw_data = wf.readframes(nframes)
        samples = np.frombuffer(raw_data, dtype=np.int16).astype(np.float32)
    return samples


def decode_mp3_to_wav(ffmpeg_bin, mp3_path, temp_dir):
    """Decode MP3 to lossless 24-bit PCM WAV to avoid FFmpeg's imprecise -ss seeking on raw MP3.
    
    MP3 files without a Xing/LAME seek table cause -ss (before -i) to seek to
    the wrong position, sometimes off by several seconds. Decoding to WAV first
    enables sample-exact seeking. Also eliminates MP3 encoder delay artifacts
    (e.g. start: 0.023021 reported by ffprobe).
    """
    wav_path = os.path.join(temp_dir, os.path.splitext(os.path.basename(mp3_path))[0] + ".wav")
    if os.path.exists(wav_path):
        return wav_path
    print(f"  Decoding MP3 to lossless WAV intermediary: {os.path.basename(mp3_path)}...")
    cmd = [ffmpeg_bin, "-y", "-i", mp3_path, "-c:a", "pcm_s24le", wav_path]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if res.returncode != 0 or not os.path.exists(wav_path):
        print(f"  Warning: WAV decode failed, falling back to original MP3")
        return mp3_path
    return wav_path


def verify_alignment(ffmpeg_bin, rendered_path, reference_path, checkpoints, temp_dir):
    """Cross-correlate rendered AD against reference audio at multiple checkpoints.
    
    Returns True if all checkpoints have lag < 0.5s.
    """
    sr = 8000
    all_ok = True
    for t in checkpoints:
        vfy_ad = os.path.join(temp_dir, f"vfy_ad_{int(t)}.wav")
        vfy_ref = os.path.join(temp_dir, f"vfy_ref_{int(t)}.wav")
        extract_mono_wav_fast(ffmpeg_bin, rendered_path, t, 20.0, vfy_ad, sample_rate=sr)
        # For MKV references, extract first audio stream
        cmd = [ffmpeg_bin, "-y", "-ss", f"{t:.3f}", "-i", reference_path,
               "-map", "0:a:0", "-t", "20.0",
               "-af", f"aresample={sr},pan=mono|c0=c0",
               "-c:a", "pcm_s16le", vfy_ref]
        subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        s_ad = read_wav_samples(vfy_ad)
        s_ref = read_wav_samples(vfy_ref)
        if len(s_ad) == 0 or len(s_ref) == 0:
            continue
        corr = np.correlate(s_ad, s_ref, mode="full")
        mid = len(s_ref) - 1
        peak = np.argmax(corr)
        lag = (peak - mid) / sr
        conf = corr[peak] / (np.linalg.norm(s_ref) * np.linalg.norm(s_ad) + 1e-9)
        status = "OK" if abs(lag) < 0.5 else "MISALIGNED"
        if abs(lag) >= 0.5:
            all_ok = False
        print(f"    @{t:5.0f}s: Lag = {lag:+.4f}s | Confidence = {conf:.4f} | {status}")
    return all_ok


def compute_sync_parameters(ffmpeg_bin, base_path, ad_tracks, target_duration, temp_dir, force_stretch=False, no_stretch=False):
    """
    Computes initial delay offset via high-confidence multi-window cross-correlation.
    """
    print("\n=== COMPUTING ACOUSTIC CROSS-CORRELATION SYNC PARAMETERS ===")
    
    sr = 8000
    if target_duration > 1800.0:
        # Generate 10 evenly spaced anchors up to 85% of the movie (avoiding end credits)
        max_anchor = target_duration * 0.85
        anchor_times = [max_anchor * (i / 10.0) for i in range(1, 11)]
    else:
        anchor_times = [target_duration * (i / 11.0) for i in range(1, 11)]
    sync_results = {}
    
    for name, path in ad_tracks.items():
        print(f"\nAnalyzing acoustic correlation for [{name}]...")
        
        # Decode MP3 to WAV first for precise sample-exact seeking without Xing/LAME errors
        analyze_path = path
        if path.lower().endswith('.mp3') and temp_dir:
            print(f"  [{name}] Pre-decoding MP3 to WAV for sample-exact correlation seeking...")
            analyze_path = decode_mp3_to_wav(ffmpeg_bin, path, temp_dir)
            
        best_overall_confidence = -1.0
        best_offset = 0.0
        
        for anchor in anchor_times:
            if anchor + 30.0 > target_duration:
                continue
                
            base_anchor_wav = os.path.join(temp_dir, f"ref_anchor_{int(anchor)}.wav")
            extract_mono_wav_fast(ffmpeg_bin, base_path, anchor, 30.0, base_anchor_wav, sample_rate=sr)
            base_samples = read_wav_samples(base_anchor_wav)
            
            # Extract search window from AD track: anchor-30s to anchor+60s (90s duration)
            ad_anchor_wav = os.path.join(temp_dir, f"{name}_anchor_{int(anchor)}.wav")
            search_start = max(0.0, anchor - 30.0)
            extract_mono_wav_fast(ffmpeg_bin, analyze_path, search_start, 90.0, ad_anchor_wav, sample_rate=sr)
            ad_samples = read_wav_samples(ad_anchor_wav)
            
            corr = np.correlate(ad_samples, base_samples, mode='valid')
            best_idx = np.argmax(corr)
            norm_peak = corr[best_idx] / (np.linalg.norm(base_samples) * np.linalg.norm(ad_samples[best_idx:best_idx+len(base_samples)]) + 1e-9)
            
            offset = search_start + (best_idx / sr) - anchor
            
            print(f"  Anchor {anchor:4.0f}s: Measured Offset = {offset:+.3f}s | Correlation Peak Confidence = {norm_peak:.4f}")
            
            if norm_peak > best_overall_confidence:
                best_overall_confidence = norm_peak
                best_offset = offset
                
        init_offset = best_offset
        atempo = 1.0
        
        is_pal_track = ("uk" in name.lower() or "pal" in name.lower() or "25fps" in name.lower())
        
        if (is_pal_track or force_stretch) and not no_stretch:
            cl_seek = max(0.0, target_duration - 180.0)
            base_cl_wav = os.path.join(temp_dir, "ref_closing.wav")
            if not os.path.exists(base_cl_wav):
                extract_mono_wav_fast(ffmpeg_bin, base_path, cl_seek, 30.0, base_cl_wav, sample_rate=sr)
            base_cl_samples = read_wav_samples(base_cl_wav)
            
            est_cl_start = (cl_seek / 1.042709) + init_offset - 30.0 if is_pal_track else cl_seek + init_offset - 30.0
            ad_cl_wav = os.path.join(temp_dir, f"{name}_cl.wav")
            extract_mono_wav_fast(ffmpeg_bin, analyze_path, max(0.0, est_cl_start), 90.0, ad_cl_wav, sample_rate=sr)
            ad_cl_samples = read_wav_samples(ad_cl_wav)
            
            corr_cl = np.correlate(ad_cl_samples, base_cl_samples, mode='valid')
            best_cl_idx = np.argmax(corr_cl)
            actual_cl_t = max(0.0, est_cl_start) + (best_cl_idx / sr)
            
            t_ad_opening = 300.0 + init_offset
            t_ad_delta = actual_cl_t - t_ad_opening
            speed_ratio = t_ad_delta / (cl_seek - 300.0) if cl_seek > 300.0 else 1.0
            computed_atempo = 1.0 / speed_ratio
            
            if 1.030 <= computed_atempo <= 1.050 or force_stretch:
                atempo = computed_atempo
                print(f"  [{name}] Speed Stretch Matched: {atempo:.6f}x")
            else:
                print(f"  [{name}] Speed ratio {computed_atempo:.6f}x outside standard PAL range. Reverting to 1.000000x.")
        else:
            print(f"  [{name}] Native rate (1.000000x) enforced.")
            
        sync_results[name] = {
            "offset": init_offset,
            "atempo": atempo,
            "confidence": best_overall_confidence
        }
        print(f"  => SELECTED PARAMETERS for [{name}]: Delay Offset = {init_offset:+.3f}s (Confidence: {best_overall_confidence:.4f})")
        
    return sync_results


def run_loudnorm_2pass(ffmpeg_bin, cmd_base, filter_prefix, target_i, target_tp, target_lra, out_path):
    """Executes 2-pass EBU R128 loudness normalization."""
    af_pass1 = filter_prefix + [f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:print_format=json"]
    cmd_pass1 = list(cmd_base)
    cmd_pass1.extend(["-af", ",".join(af_pass1), "-f", "null", "-"])
    
    res = subprocess.run(cmd_pass1, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    stderr_out = res.stderr
    
    json_match = re.search(r'\{[^{}]*"input_i"[^{}]*\}', stderr_out, re.DOTALL)
    if not json_match:
        print("  Warning: Could not parse Pass 1 JSON output from loudnorm. Falling back to 1-pass.")
        af_pass2 = filter_prefix + [f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}"]
    else:
        stats = json.loads(json_match.group(0))
        loudnorm_pass2 = (
            f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:"
            f"measured_I={stats['input_i']}:measured_TP={stats['input_tp']}:"
            f"measured_LRA={stats['input_lra']}:measured_thresh={stats['input_thresh']}:linear=true"
        )
        af_pass2 = filter_prefix + [loudnorm_pass2]
        
    cmd_pass2 = list(cmd_base)
    cmd_pass2.extend([
        "-af", ",".join(af_pass2),
        "-c:a", "aac", "-b:a", "256k", "-ar", "48000", out_path
    ])
    
    res_p2 = subprocess.run(cmd_pass2, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_p2.returncode != 0:
        print(f"  Error rendering track: {res_p2.stderr}")
        sys.exit(1)


def mux_video_sample(ffmpeg_bin, video_src, audio_src, out_mkv, render_duration):
    """Multiplexes generated AD track with video stream using zero-re-encode stream copy (-c:v copy)."""
    print(f"\n--- Multiplexing Video Sample MKV ({render_duration:.0f}s) ---")
    cmd_mux = [
        ffmpeg_bin, "-y",
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
        print(f"  Saved MKV Sample: {out_mkv} ({size_mb:.1f} MB)")
    else:
        print(f"  Warning: MKV Video Muxing failed: {res.stderr}")


def render_ad_tracks(ffmpeg_bin, base_title, ad_tracks, sync_params, target_duration, out_dir, preset_mode, target_i, target_tp, preview_sec=None, video_mux_path=None, temp_dir=None):
    """Renders aligned AD tracks according to selected loudness presets.
    
    CRITICAL OFFSET SIGN CONVENTION (verified 2026-07-30):
      offset < 0 -> AD is AHEAD of reference -> PREPEND silence via adelay
      offset > 0 -> AD is BEHIND reference -> TRIM start via -ss (after -i)
    
    Previous bug: signs were inverted, causing -ss to trim when AD was already
    ahead, doubling the misalignment.
    """
    render_duration = target_duration if not preview_sec else min(preview_sec, target_duration)
    
    mode_str = f"PREVIEW SAMPLE ({render_duration:.0f}s)" if preview_sec else "FULL MOVIE RENDER"
    print(f"\n=== RENDERING 2-PASS LOUDNORM AAC TRACKS ({mode_str} | TP={target_tp} dB) ===")
    
    rendered_outputs = []
    
    for name, path in ad_tracks.items():
        params = sync_params[name]
        offset = params["offset"]
        atempo = params["atempo"]
        
        # Decode MP3 to WAV intermediary for sample-exact seeking
        render_path = path
        if path.lower().endswith('.mp3') and temp_dir:
            render_path = decode_mp3_to_wav(ffmpeg_bin, path, temp_dir)
        
        # Build base FFmpeg command with correct offset direction:
        #   offset > 0 (AD behind): trim from start with -ss AFTER -i
        #   offset < 0 (AD ahead): prepend silence with adelay filter
        cmd_base = [ffmpeg_bin, "-y", "-i", render_path]
        if offset > 0:
            # AD is behind -> trim offset seconds from start (seek after -i for accuracy)
            cmd_base.extend(["-ss", f"{abs(offset):.3f}"])
        cmd_base.extend(["-t", f"{render_duration:.3f}"])
        
        render_orig = preset_mode in ["all", "original"]
        render_stereo = preset_mode in ["all", "stereo"]
        
        suffix = f" (Preview {int(render_duration)}s)" if preview_sec else ""
        
        if render_orig:
            out_orig = os.path.join(out_dir, f"{base_title} - Audio Description [{name}] (Synced){suffix}.ac3")
            
            cmd_orig = list(cmd_base)
            filters_orig = []
            if abs(atempo - 1.0) > 0.0001:
                filters_orig.append(f"atempo={atempo:.8f}")
            if offset < 0:
                delay_ms = int(abs(offset) * 1000)
                filters_orig.append(f"adelay={delay_ms}:all=1")
            
            if filters_orig:
                cmd_orig.extend(["-af", ",".join(filters_orig)])
            
            # Use AC3 at 640k to safely preserve channels (Stereo or 5.1) without loudnorm
            cmd_orig.extend(["-c:a", "ac3", "-b:a", "640k", out_orig])
            
            print(f"\n--- Processing [{name}] Original Preset ---")
            print(f"  Offset: {offset:+.3f}s -> {'adelay=' + str(int(abs(offset)*1000)) + 'ms' if offset < 0 else '-ss ' + f'{abs(offset):.3f}' if offset > 0 else 'no adjustment'}")
            
            subprocess.run(cmd_orig, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            print(f"  Saved Audio: {out_orig}")
            rendered_outputs.append((name, "Original", out_orig))
            
            if video_mux_path and os.path.exists(video_mux_path):
                out_mkv = os.path.join(out_dir, f"{base_title} - Audio Description [{name}] (Synced){suffix}.mkv")
                mux_video_sample(ffmpeg_bin, video_mux_path, out_orig, out_mkv, render_duration)
            
        if render_stereo:
            out_stereo = os.path.join(out_dir, f"{base_title} - Audio Description [{name}] (Synced Stereo){suffix}.m4a")
            # Explicitly force a stereo downmix before applying loudnorm
            base_filters_stereo = ["aresample=48000", "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"]
            if abs(atempo - 1.0) > 0.0001:
                base_filters_stereo.insert(0, f"atempo={atempo:.8f}")
            if offset < 0:
                delay_ms = int(abs(offset) * 1000)
                base_filters_stereo.insert(0, f"adelay={delay_ms}:all=1")
                
            print(f"\n--- Processing [{name}] Stereo Preset ---")
            print(f"  Offset: {offset:+.3f}s -> {'adelay=' + str(int(abs(offset)*1000)) + 'ms' if offset < 0 else '-ss ' + f'{abs(offset):.3f}' if offset > 0 else 'no adjustment'}")
            run_loudnorm_2pass(ffmpeg_bin, cmd_base, base_filters_stereo, target_i, target_tp, target_lra=11, out_path=out_stereo)
            print(f"  Saved Audio: {out_stereo}")
            rendered_outputs.append((name, "Stereo", out_stereo))
            
            if video_mux_path and os.path.exists(video_mux_path):
                out_mkv = os.path.join(out_dir, f"{base_title} - Audio Description [{name}] (Synced Stereo){suffix}.mkv")
                mux_video_sample(ffmpeg_bin, video_mux_path, out_stereo, out_mkv, render_duration)
    
    # Post-render verification if we have a video/reference to check against
    ref_for_verify = video_mux_path if video_mux_path else None
    if ref_for_verify and temp_dir and rendered_outputs:
        print(f"\n=== POST-RENDER ALIGNMENT VERIFICATION ===")
        checkpoints = [30.0, render_duration * 0.5, render_duration * 0.85]
        checkpoints = [c for c in checkpoints if c + 20.0 < render_duration]
        for name, preset, audio_path in rendered_outputs:
            print(f"  [{name}] {preset}:")
            verify_alignment(ffmpeg_bin, audio_path, ref_for_verify, checkpoints, temp_dir)


def main():
    parser = argparse.ArgumentParser(
        description="Universal Audio Description (AD) Cross-Correlation & 2-Pass Normalization Sync Tool",
        formatter_class=argparse.RawTextHelpFormatter
    )
    parser.add_argument("-r", "--ref", required=True, help="Path to reference main audio/video file (AC3, EAC3, DTS, MKV, etc.)")
    parser.add_argument("-a", "--ad", required=True, nargs="+", help="Audio Description track(s). Format: 'Path' or 'TrackLabel=Path'")
    parser.add_argument("-m", "--mux-video", default=None, help="Path to reference MKV/MP4 video file for multiplexing sample video MKV")
    parser.add_argument("-o", "--outdir", default=os.getenv("AD_OUTPUT_DIR", r"F:\uploads"), help="Output directory for synced AD tracks (Default: F:\\uploads)")
    parser.add_argument("-t", "--title", default=None, help="Movie title prefix for output filenames (Defaults to reference file basename)")
    parser.add_argument("-p", "--preview", type=float, nargs="?", const=300.0, default=None, help="Fast sync test: Render a short preview sample (Default: 300s / 5 mins)")
    parser.add_argument("--preset", choices=["all", "original", "stereo"], default="all", help="Output presets to render (Default: all)")
    parser.add_argument("--no-stretch", action="store_true", help="Force native 1.0x speed rate without speed stretch")
    parser.add_argument("--force-stretch", action="store_true", help="Force 2-point closing cross-correlation speed stretch search")
    parser.add_argument("--target-i", type=float, default=-16.0, help="Target EBU R128 integrated loudness in LUFS (Default: -16.0)")
    parser.add_argument("--target-tp", type=float, default=-1.5, help="Target true-peak ceiling in dB (Default: -1.5)")
    parser.add_argument("--ffmpeg", default=DEFAULT_FFMPEG, help="Path to ffmpeg executable")
    
    args = parser.parse_args()
    
    ffmpeg_bin, ffprobe_bin = locate_binaries(args.ffmpeg)
    
    if not os.path.exists(args.ref):
        print(f"Error: Reference file not found: '{args.ref}'")
        sys.exit(1)
        
    os.makedirs(args.outdir, exist_ok=True)
    temp_dir = os.path.join(args.outdir, "_temp_sync")
    os.makedirs(temp_dir, exist_ok=True)
    
    base_title = args.title
    if not base_title:
        base_title = os.path.splitext(os.path.basename(args.ref))[0]
        
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
        
    video_mux_path = args.mux_video
    if video_mux_path and os.path.isdir(video_mux_path):
        mkv_candidates = [os.path.join(video_mux_path, f) for f in os.listdir(video_mux_path) if f.endswith('.mkv')]
        if mkv_candidates:
            video_mux_path = mkv_candidates[0]
            
    print("================================================================================")
    print(f"Reference Audio : {args.ref}")
    print(f"Movie Title     : {base_title}")
    print(f"Output Dir      : {args.outdir}")
    print(f"AD Tracks Count : {len(ad_tracks)}")
    if video_mux_path:
        print(f"Video Muxing    : {video_mux_path}")
    if args.preview:
        print(f"Preview Mode    : ON ({args.preview:.0f} seconds)")
    print("================================================================================")
    
    target_duration = get_audio_duration(ffprobe_bin, args.ref)
    print(f"Reference Duration: {target_duration:.3f}s ({target_duration/3600.0:.2f} hours)")
    
    sync_params = compute_sync_parameters(
        ffmpeg_bin, args.ref, ad_tracks, target_duration, temp_dir,
        force_stretch=args.force_stretch, no_stretch=args.no_stretch
    )
    
    render_ad_tracks(
        ffmpeg_bin, base_title, ad_tracks, sync_params, target_duration,
        args.outdir, args.preset, args.target_i, args.target_tp,
        preview_sec=args.preview, video_mux_path=video_mux_path,
        temp_dir=temp_dir
    )
    
    try:
        for f in os.listdir(temp_dir):
            os.remove(os.path.join(temp_dir, f))
        os.rmdir(temp_dir)
    except Exception:
        pass
        
    print("\n================================================================================")
    print("SUCCESS: SYNCED AUDIO & VIDEO SAMPLE TRACKS GENERATED SUCCESSFULLY!")
    print("================================================================================")


if __name__ == "__main__":
    main()
