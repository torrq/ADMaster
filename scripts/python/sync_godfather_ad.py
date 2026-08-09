import subprocess
import os
import wave
import struct
import json
import re
import numpy as np

temp_dir = r"C:\Users\nathan\.gemini\antigravity\scratch\godfather_sync2"
os.makedirs(temp_dir, exist_ok=True)

ffmpeg_bin = r"C:\Users\nathan\AppData\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"

out_dir = r"F:\uploads\ad_synced2"
os.makedirs(out_dir, exist_ok=True)

base_eac3 = r"F:\uploads\The.Godfather.1972.1080p.BluRay.DDP5.1.x264-PTer_Audio02.eac3"
ad_tracks = {
    "New Description": r"F:\uploads\ad\10530 The Godfather (1972) [New Description] .mp3",
    "UK": r"F:\uploads\ad\12208 The Godfather (UK) (1972) .mp3",
    "US": r"F:\uploads\ad\7565 The Godfather (1972) [US] .mp3"
}

def get_duration(input_path):
    ffprobe_bin = os.path.join(os.path.dirname(ffmpeg_bin), "ffprobe.exe")
    cmd = [
        ffprobe_bin, "-v", "error", "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1", input_path
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return float(res.stdout.strip())

target_duration = get_duration(base_eac3)
print(f"Base Audio Reference Duration: {target_duration:.3f}s ({target_duration/3600.0:.2f} hours)")

def extract_mono_wav(input_path, start_s, dur_s, output_wav):
    cmd = [
        ffmpeg_bin, "-y", "-ss", f"{start_s:.3f}", "-t", f"{dur_s:.3f}",
        "-i", input_path,
        "-af", "aresample=16000,pan=mono|c0=c0",
        "-c:a", "pcm_s16le", output_wav
    ]
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def read_wav_samples(wav_path):
    with wave.open(wav_path, 'rb') as wf:
        nframes = wf.getnframes()
        raw_data = wf.readframes(nframes)
        samples = np.frombuffer(raw_data, dtype=np.int16).astype(np.float32)
    return samples

def compute_sync_parameters():
    print("\n=== COMPUTING TWO-POINT CROSS-CORRELATION SYNC PARAMETERS ===")
    
    # Extract Base opening & closing reference snippets
    base_op_wav = os.path.join(temp_dir, "base_op.wav")
    base_cl_wav = os.path.join(temp_dir, "base_cl.wav")
    
    extract_mono_wav(base_eac3, 0.0, 180.0, base_op_wav)
    # Closing snippet around 10300s relative to base start
    cl_seek = max(0.0, target_duration - 330.0)
    extract_mono_wav(base_eac3, cl_seek, 180.0, base_cl_wav)
    
    base_op_samples = read_wav_samples(base_op_wav)
    base_cl_samples = read_wav_samples(base_cl_wav)
    
    sr = 16000
    ref_op_signal = base_op_samples[int(10.0*sr):int(40.0*sr)]
    ref_cl_signal = base_cl_samples[int(10.0*sr):int(40.0*sr)]
    
    results = {}
    
    for name, path in ad_tracks.items():
        print(f"\nAnalyzing cross-correlation for [{name}]...")
        # Opening search
        ad_op_wav = os.path.join(temp_dir, f"{name}_op.wav")
        extract_mono_wav(path, 0.0, 180.0, ad_op_wav)
        ad_op_samples = read_wav_samples(ad_op_wav)
        
        corr_op = np.correlate(ad_op_samples, ref_op_signal, mode='valid')
        best_op_idx = np.argmax(corr_op)
        init_offset = (best_op_idx / sr) - 10.0
        
        # Closing search
        est_cl_start = cl_seek + init_offset - 60.0
        if name == "UK":
            est_cl_start = (cl_seek / 1.042709) + init_offset - 60.0
            
        ad_cl_wav = os.path.join(temp_dir, f"{name}_cl.wav")
        extract_mono_wav(path, max(0, est_cl_start), 300.0, ad_cl_wav)
        ad_cl_samples = read_wav_samples(ad_cl_wav)
        
        corr_cl = np.correlate(ad_cl_samples, ref_cl_signal, mode='valid')
        best_cl_idx = np.argmax(corr_cl)
        actual_cl_t = max(0, est_cl_start) + (best_cl_idx / sr)
        
        t_ad_opening = 10.0 + init_offset
        t_ad_delta = actual_cl_t - t_ad_opening
        speed_ratio = t_ad_delta / (cl_seek + 10.0 - 10.0)
        
        results[name] = {
            "offset": init_offset,
            "speed_ratio": speed_ratio,
            "atempo": 1.0 / speed_ratio if abs(speed_ratio - 1.0) > 0.0001 else 1.0
        }
        print(f"  {name}: Offset = {init_offset:+.3f}s | Speed Stretch = {results[name]['atempo']:.6f}x")
        
    return results

def run_loudnorm_2pass(cmd_base, filter_prefix, target_i, target_tp, target_lra, out_path):
    # Pass 1: Measure loudness stats
    af_pass1 = filter_prefix + [f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:print_format=json"]
    cmd_pass1 = list(cmd_base)
    cmd_pass1.extend(["-af", ",".join(af_pass1), "-f", "null", "-"])
    
    res = subprocess.run(cmd_pass1, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    stderr_out = res.stderr
    
    json_match = re.search(r'\{[^{}]*"input_i"[^{}]*\}', stderr_out, re.DOTALL)
    if not json_match:
        print("  Error: Could not parse Pass 1 JSON output from loudnorm! Falling back to 1-pass.")
        af_pass2 = filter_prefix + [f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}"]
    else:
        stats = json.loads(json_match.group(0))
        loudnorm_pass2 = (
            f"loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:"
            f"measured_I={stats['input_i']}:measured_TP={stats['input_tp']}:"
            f"measured_LRA={stats['input_lra']}:measured_thresh={stats['input_thresh']}:linear=true"
        )
        af_pass2 = filter_prefix + [loudnorm_pass2]
        
    # Pass 2: Render audio with linear gain scaling & true peak protection
    cmd_pass2 = list(cmd_base)
    cmd_pass2.extend([
        "-af", ",".join(af_pass2),
        "-c:a", "aac", "-b:a", "256k", out_path
    ])
    
    subprocess.run(cmd_pass2, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def render_tracks(sync_params):
    print("\n=== RENDERING 2-PASS LOUDNORM (TP=-1.5 dB) AAC TRACKS ===")
    
    for name, path in ad_tracks.items():
        params = sync_params[name]
        offset = params["offset"]
        atempo = params["atempo"]
        
        out_std = os.path.join(out_dir, f"The Godfather (1972) - Audio Description [{name}] (Synced Standard).m4a")
        out_night = os.path.join(out_dir, f"The Godfather (1972) - Audio Description [{name}] (Synced NightMode).m4a")
        
        # Build base input command with exact trimming & timing alignment
        cmd_base = [ffmpeg_bin, "-y"]
        if offset > 0:
            cmd_base.extend(["-ss", f"{offset:.3f}"])
        cmd_base.extend(["-i", path, "-t", f"{target_duration:.3f}"])
        
        # Base audio filters before loudnorm
        base_filters_std = ["aresample=48000"]
        base_filters_night = ["aresample=48000", "acompressor=threshold=-21dB:ratio=3.5:attack=10:release=100"]
        
        if abs(atempo - 1.0) > 0.0001:
            base_filters_std.insert(0, f"atempo={atempo:.8f}")
            base_filters_night.insert(0, f"atempo={atempo:.8f}")
            
        if offset < 0:
            delay_ms = int(abs(offset) * 1000)
            base_filters_std.insert(0, f"adelay={delay_ms}|{delay_ms}")
            base_filters_night.insert(0, f"adelay={delay_ms}|{delay_ms}")
            
        print(f"\n--- Processing [{name}] Standard ---")
        run_loudnorm_2pass(cmd_base, base_filters_std, target_i=-16, target_tp=-1.5, target_lra=11, out_path=out_std)
        print(f"  Saved: {os.path.basename(out_std)}")
        
        print(f"\n--- Processing [{name}] NightMode ---")
        run_loudnorm_2pass(cmd_base, base_filters_night, target_i=-16, target_tp=-1.5, target_lra=7, out_path=out_night)
        print(f"  Saved: {os.path.basename(out_night)}")

if __name__ == "__main__":
    params = compute_sync_parameters()
    render_tracks(params)
    print("\nALL 6 AUDIO DESCRIPTION TRACKS SYNCED AND RENDERED WITH 2-PASS LOUDNORM SUCCESSFULLY!")
