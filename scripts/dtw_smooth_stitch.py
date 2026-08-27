#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""
dtw_smooth_stitch.py - Smooth DTW Audio Stitcher with VAD-Aware Transitions

Replaces the old hard-cut `build_dtw_stitch.py` with a 3-layer transition
smoothing pipeline that prevents narrator speech from being split at
segment boundaries:

  Layer 1: VAD-Aware Cut Nudging
    - At each segment boundary, checks for active speech using RMS energy
      on 50ms analysis windows. If speech is detected, the outgoing cut
      is extended forward and/or the incoming cut is pulled backward to
      the nearest silence gap (configurable +/-500ms search, 200ms hangover).

  Layer 2: Zero-Crossing Snapping
    - After VAD nudging, each cut point is snapped to the nearest waveform
      zero crossing within +/-2ms to eliminate DC-offset pops and clicks.

  Layer 3: Equal-Power Crossfading
    - At each segment junction, a cosine-shaped equal-power crossfade
      (default 50ms) smoothly blends the outgoing and incoming audio
      using overlap-add mixing.

Usage (standalone):
  python dtw_smooth_stitch.py <alignment_plot.txt> <ad_audio> <output.m4a>

Options:
  --crossfade-ms    Crossfade duration in ms (default: 50)
  --vad-search-ms   VAD search window in ms (default: 500)
  --vad-threshold   RMS threshold for speech detection, 0-1 scale (default: 0.02)
  --silence-ms      Required consecutive silence for clean boundary (default: 200)

Can also be imported as a module by batch_orchestrator.py.
"""

import sys
import os
import re
import wave
import struct
import subprocess
import argparse
import shutil
import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.join(os.path.dirname(SCRIPT_DIR), "bin")


# --- Utilities ----------------------------------------------------------------

def find_ffmpeg():
    """Find ffmpeg binary from bin/ or system PATH."""
    local = os.path.join(BIN_DIR, "ffmpeg.exe")
    if os.path.exists(local):
        return local
    found = shutil.which("ffmpeg")
    if found:
        return found
    raise FileNotFoundError("ffmpeg not found in bin/ or PATH")


def time_to_sec(time_str):
    """Parse 'HH:MM:SS.mmm' to float seconds."""
    h, m, s = time_str.split(":")
    return int(h) * 3600 + int(m) * 60 + float(s)


def fmt_time(seconds):
    """Format seconds as 'MM:SS.sss' for display."""
    m, s = divmod(abs(seconds), 60)
    return f"{int(m):02d}:{s:06.3f}"


# --- Alignment Plot Parser ---------------------------------------------------

def parse_alignment_plot(plot_file):
    """
    Parse a describealign alignment_plots/*.txt file.

    Each line looks like:
      Rate change of  0.0% from  0:00:00.000 to  0:08:44.200
        aligning with audio from  0:00:17.271 to  0:09:01.471

    Returns a list of segment dicts:
      {rate_pct, master_start, master_end, audio_start, audio_end}
    """
    with open(plot_file, "r", encoding="utf-8") as f:
        content = f.read()

    # Full-line regex capturing rate, master times, and audio times
    full_re = re.compile(
        r"Rate change of\s+([-0-9.]+)%\s+from\s+([0-9:\.]+)\s+to\s+([0-9:\.]+)"
        r"\s+aligning with audio from\s+([0-9:\.]+)\s+to\s+([0-9:\.]+)"
    )
    # Fallback: just audio from/to (older plot formats)
    audio_re = re.compile(
        r"aligning with audio from\s+([0-9:\.]+)\s+to\s+([0-9:\.]+)"
    )

    segments = []
    for line in content.splitlines():
        m = full_re.search(line)
        if m:
            segments.append({
                "rate_pct":     float(m.group(1)),
                "master_start": time_to_sec(m.group(2)),
                "master_end":   time_to_sec(m.group(3)),
                "audio_start":  time_to_sec(m.group(4)),
                "audio_end":    time_to_sec(m.group(5)),
            })
        else:
            m2 = audio_re.search(line)
            if m2:
                segments.append({
                    "rate_pct":     0.0,
                    "master_start": 0.0,
                    "master_end":   0.0,
                    "audio_start":  time_to_sec(m2.group(1)),
                    "audio_end":    time_to_sec(m2.group(2)),
                })

    cleaned = []
    for s in segments:
        dur_master = s["master_end"] - s["master_start"]
        # Filter out extreme micro-glitches (e.g. 7407% rate jump over 0.07 seconds)
        if dur_master < 0.25 and abs(s["rate_pct"]) > 50.0:
            continue
        cleaned.append(s)

    return cleaned if cleaned else segments


# --- Audio I/O ----------------------------------------------------------------

def predecode_to_pcm(input_audio, output_wav, ffmpeg, sample_rate=48000):
    """
    Decode any audio format to 16-bit WAV (keeping original channels).
    Uses atomic temp-file pattern to prevent corruption on interrupt.
    """
    tmp = output_wav.replace(".wav", "_tmp.wav")
    cmd = [
        ffmpeg, "-y", "-nostdin", "-hide_banner", "-loglevel", "warning", "-fflags", "+genpts", "-i", input_audio,
        "-vn", "-af", "asetpts=PTS-STARTPTS", "-ar", str(sample_rate),
        "-c:a", "pcm_s16le", "-f", "wav", tmp,
    ]
    result = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode != 0:
        raise RuntimeError(f"FFmpeg decode failed:\n{result.stderr.decode('utf-8', errors='ignore')}")
    os.replace(tmp, output_wav)


def load_wav(wav_path):
    """Load WAV -> numpy float32 array in [-1, 1]. Returns (samples, sample_rate)."""
    with wave.open(wav_path, "rb") as wf:
        sr = wf.getframerate()
        n = wf.getnframes()
        channels = wf.getnchannels()
        raw = wf.readframes(n)
        width = wf.getsampwidth()

    if width == 2:
        arr = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    elif width == 3:
        # 24-bit PCM: unpack manually
        raw_bytes = np.frombuffer(raw, dtype=np.uint8)
        arr = np.zeros(len(raw_bytes) // 3, dtype=np.float32)
        for i in range(len(arr)):
            b = raw_bytes[i * 3 : i * 3 + 3]
            val = b[0] | (b[1] << 8) | (b[2] << 16)
            if val >= 0x800000:
                val -= 0x1000000
            arr[i] = val / 8388608.0
    elif width == 4:
        arr = np.frombuffer(raw, dtype=np.int32).astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"Unsupported sample width: {width}")

    if channels >= 1:
        arr = arr.reshape(-1, channels)

    return arr, sr


def write_wav(samples, sr, output_path):
    """Write float32 [-1,1] samples to 16-bit 48kHz WAV."""
    clipped = np.clip(samples, -1.0, 1.0)
    pcm = (clipped * 32767).astype(np.int16)
    channels = samples.shape[1] if samples.ndim > 1 else 1
    
    if samples.ndim > 1:
        pcm = pcm.flatten()
        
    with wave.open(output_path, "wb") as wf:
        wf.setnchannels(channels)
        wf.setsampwidth(2)
        wf.setframerate(sr)
        wf.writeframes(pcm.tobytes())


def wsola_time_stretch(audio_samples, sr, target_samples, ffmpeg_bin=None):
    """
    Pitch-corrected time-stretching using WSOLA (FFmpeg atempo filter).
    Preserves natural voice pitch and timbre while stretching or shrinking duration.
    """
    if len(audio_samples) == 0 or target_samples <= 0 or len(audio_samples) == target_samples:
        return audio_samples
    
    tempo = float(len(audio_samples)) / float(target_samples)
    if abs(tempo - 1.0) < 0.002:
        return audio_samples
        
    if ffmpeg_bin is None:
        ffmpeg_bin = find_ffmpeg()
        
    # Build atempo filter chain (atempo accepts values in [0.5, 2.0])
    filters = []
    curr = tempo
    while curr < 0.5:
        filters.append("atempo=0.5")
        curr /= 0.5
    while curr > 2.0:
        filters.append("atempo=2.0")
        curr /= 2.0
    filters.append(f"atempo={curr:.6f}")
    filter_str = ",".join(filters)
    
    raw_in = (np.clip(audio_samples, -1.0, 1.0) * 32767).astype(np.int16)
    channels = audio_samples.shape[1] if audio_samples.ndim > 1 else 1
    
    cmd = [
        ffmpeg_bin, "-y", "-nostdin", "-hide_banner", "-loglevel", "error",
        "-f", "s16le", "-ar", str(sr), "-ac", str(channels), "-i", "pipe:0",
        "-af", filter_str,
        "-f", "s16le", "-ar", str(sr), "-ac", str(channels), "pipe:1"
    ]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    out_bytes, _ = proc.communicate(input=raw_in.tobytes())
    
    if len(out_bytes) == 0:
        return audio_samples
        
    out_arr = np.frombuffer(out_bytes, dtype=np.int16).astype(np.float32) / 32768.0
    if channels > 1:
        out_arr = out_arr.reshape(-1, channels)
        
    # Align to target samples
    if len(out_arr) > target_samples:
        out_arr = out_arr[:target_samples]
    elif len(out_arr) < target_samples:
        diff = target_samples - len(out_arr)
        pad = np.zeros((diff, channels) if channels > 1 else (diff,), dtype=np.float32)
        out_arr = np.vstack([out_arr, pad]) if channels > 1 else np.concatenate([out_arr, pad])
        
    return out_arr


# --- Layer 1: VAD-Aware Cut Nudging ------------------------------------------

def rms_energy(samples):
    """Root Mean Square energy of a sample buffer."""
    if len(samples) == 0:
        return 0.0
    if samples.ndim == 2:
        samples = np.mean(samples, axis=1)
    return float(np.sqrt(np.mean(samples.astype(np.float64) ** 2)))


def has_speech(samples, sr, position, window_ms=50, threshold=0.02):
    """Check if there's active speech at a given sample position."""
    half_win = int(sr * window_ms / 2000.0)
    start = max(0, position - half_win)
    end = min(len(samples), position + half_win)
    if end - start < half_win // 2:
        return False
    return rms_energy(samples[start:end]) > threshold


def find_silence(samples, sr, origin, direction,
                 search_ms=500, window_ms=50,
                 threshold=0.02, silence_ms=200):
    """
    Search for the onset of a silence gap near a cut point.

    Args:
        origin:     Starting sample index (the original cut)
        direction:  +1 to search forward, -1 to search backward
        search_ms:  Maximum search radius in ms
        silence_ms: Required consecutive silence to declare "clean"

    Returns:
        Adjusted sample index if silence found, else original.
    """
    win_samples = int(sr * window_ms / 1000.0)
    max_steps = int(sr * search_ms / 1000.0) // win_samples
    need_silent = int(silence_ms / window_ms)

    silent_run = 0
    first_silent_pos = origin

    for step in range(max_steps):
        if direction > 0:
            idx = origin + step * win_samples
        else:
            idx = origin - (step + 1) * win_samples

        if idx < 0 or idx + win_samples > len(samples):
            break

        chunk = samples[idx : idx + win_samples]
        if rms_energy(chunk) <= threshold:
            if silent_run == 0:
                first_silent_pos = idx if direction > 0 else idx + win_samples
            silent_run += 1
            if silent_run >= need_silent:
                return first_silent_pos
        else:
            silent_run = 0
            first_silent_pos = origin

    return origin  # No silence found -- keep original cut


def vad_nudge_boundaries(segments_samples, samples, sr,
                         search_ms=500, threshold=0.02, silence_ms=200):
    """
    Apply VAD-aware nudging to a list of (start_sample, end_sample) pairs.
    Returns adjusted pairs and a log of nudges applied.
    """
    adjusted = []
    nudge_log = []

    for i, (seg_start, seg_end) in enumerate(segments_samples):
        new_start = seg_start
        new_end = seg_end

        # Nudge outgoing boundary (end of segment) -- extend into silence
        if i < len(segments_samples) - 1:
            if has_speech(samples, sr, seg_end, threshold=threshold):
                nudged = find_silence(samples, sr, seg_end, direction=+1,
                                      search_ms=search_ms, threshold=threshold,
                                      silence_ms=silence_ms)
                if nudged != seg_end:
                    delta_ms = (nudged - seg_end) / sr * 1000
                    nudge_log.append(
                        f"  [VAD] Seg {i+1} end: extended +{delta_ms:.0f}ms "
                        f"to protect speech tail"
                    )
                    new_end = nudged

        # Nudge incoming boundary (start of segment) -- pull back to catch word
        if i > 0:
            if has_speech(samples, sr, seg_start, threshold=threshold):
                nudged = find_silence(samples, sr, seg_start, direction=-1,
                                      search_ms=search_ms, threshold=threshold,
                                      silence_ms=silence_ms)
                if nudged != seg_start:
                    delta_ms = (seg_start - nudged) / sr * 1000
                    nudge_log.append(
                        f"  [VAD] Seg {i+1} start: pulled back -{delta_ms:.0f}ms "
                        f"to protect speech head"
                    )
                    new_start = nudged

        adjusted.append((new_start, new_end))

    return adjusted, nudge_log


# --- Layer 2: Zero-Crossing Snapping -----------------------------------------

def snap_zero_crossing(samples, sample_idx, radius_ms=2, sr=48000):
    """
    Snap a cut point to the nearest waveform zero crossing
    within +/-radius_ms to eliminate DC-offset pops.
    """
    radius = int(sr * radius_ms / 1000.0)
    lo = max(0, sample_idx - radius)
    hi = min(len(samples) - 1, sample_idx + radius)

    region = samples[lo:hi]
    if len(region) < 2:
        return sample_idx

    mono_region = region[:, 0] if region.ndim > 1 else region
    crossings = np.where(np.diff(np.signbit(mono_region)))[0]
    
    if len(crossings) == 0:
        return sample_idx

    rel = sample_idx - lo
    best = crossings[np.argmin(np.abs(crossings - rel))]
    return lo + int(best)


# --- Layer 3: Equal-Power Crossfade Stitching --------------------------------

def crossfade_stitch(segments_audio, crossfade_samples):
    """
    Concatenate audio segments with equal-power (cosine) crossfades.

    Uses overlap-add: the last `crossfade_samples` of segment N overlap
    with the first `crossfade_samples` of segment N+1. Gains are:
      outgoing = cos(t * pi/2)     (1->0)
      incoming = sin(t * pi/2)     (0->1)
    which satisfies the constant-power identity cos^2 + sin^2 = 1.
    """
    if len(segments_audio) == 0:
        return np.array([], dtype=np.float32)
    if len(segments_audio) == 1:
        return segments_audio[0]

    xf = crossfade_samples
    t = np.linspace(0, 1, xf, dtype=np.float32)
    fade_out = np.cos(t * np.pi / 2)   # 1 -> 0
    fade_in  = np.sin(t * np.pi / 2)   # 0 -> 1
    
    if segments_audio[0].ndim == 2:
        fade_out = fade_out[:, np.newaxis]
        fade_in = fade_in[:, np.newaxis]

    # Calculate total output length
    total = sum(len(s) for s in segments_audio) - xf * (len(segments_audio) - 1)
    
    if segments_audio[0].ndim == 2:
        output = np.zeros((max(total, 0), segments_audio[0].shape[1]), dtype=np.float32)
    else:
        output = np.zeros(max(total, 0), dtype=np.float32)

    pos = 0
    for i, seg in enumerate(segments_audio):
        seg = seg.copy()

        # Apply fade-out to end (except last segment)
        if i < len(segments_audio) - 1 and len(seg) >= xf:
            seg[-xf:] *= fade_out

        # Apply fade-in to start (except first segment)
        if i > 0 and len(seg) >= xf:
            seg[:xf] *= fade_in

        # Overlap-add into output
        end_pos = min(pos + len(seg), len(output))
        write_len = end_pos - pos
        output[pos : pos + write_len] += seg[:write_len]

        # Advance position (minus overlap for next segment)
        if i < len(segments_audio) - 1:
            pos += len(seg) - xf
        else:
            pos += len(seg)

    return np.clip(output, -1.0, 1.0)


# --- Main Pipeline -----------------------------------------------------------

def smooth_stitch_audio(segments, samples, sr,
                        crossfade_ms=50, vad_search_ms=500,
                        vad_threshold=0.02, silence_ms=200):
    """
    Full 3-layer smooth stitching pipeline.

    Args:
        segments:  List of dicts from parse_alignment_plot() with audio_start/audio_end
        samples:   Float32 numpy array of the full source AD audio (mono or stereo)
        sr:        Sample rate

    Returns:
        Stitched float32 numpy array.
    """
    # -------------------------------------------------------------------------
    # Layer 0: DescribeAlign Wobble Filtering
    # -------------------------------------------------------------------------
    print("\n  Layer 0: Filtering DTW Hallucinations (Wobble Correction)...", flush=True)
    filtered_segments = []
    current_offset = segments[0]["master_start"] - segments[0]["audio_start"]
    
    for i, seg in enumerate(segments):
        audio_dur = seg["audio_end"] - seg["audio_start"]
        master_dur = seg["master_end"] - seg["master_start"]
        target_offset = seg["master_end"] - seg["audio_end"]
        
        if abs(target_offset - current_offset) > 0.05:
            if audio_dur > 2.0 and master_dur > 2.0:
                print(f"  [WOBBLE DETECTED] Seg {i+1}: Ignoring offset shift to {target_offset:+.3f}s, maintaining {current_offset:+.3f}s", flush=True)
            else:
                current_offset = target_offset
                
        # Force the segment to conform to the current true offset
        seg["audio_start"] = seg["master_start"] - current_offset
        seg["audio_end"] = seg["master_end"] - current_offset
        seg["rate_pct"] = 0.0
        filtered_segments.append(seg)
        
    segments = filtered_segments

    xf_samples = int(sr * crossfade_ms / 1000.0)

    # Convert time boundaries -> sample indices
    seg_samples = []
    for seg in segments:
        s = max(0, int(seg["audio_start"] * sr))
        e = min(len(samples), int(seg["audio_end"] * sr))
        seg_samples.append((s, e))

    # -- Layer 1: VAD-Aware Nudging --
    print(f"\n  Layer 1: VAD-Aware Cut Nudging (+/-{vad_search_ms}ms search, "
          f"{silence_ms}ms silence req, threshold={vad_threshold})")
    nudged, nudge_log = vad_nudge_boundaries(
        seg_samples, samples, sr,
        search_ms=vad_search_ms, threshold=vad_threshold, silence_ms=silence_ms
    )
    if nudge_log:
        for msg in nudge_log:
            print(msg)
    else:
        print("  [VAD] No speech detected at boundaries -- all cuts are clean")

    # -- Layer 2: Zero-Crossing Snapping --
    print(f"\n  Layer 2: Zero-Crossing Snapping (+/-2ms radius)")
    snapped = []
    zc_count = 0
    for i, (s, e) in enumerate(nudged):
        new_s = snap_zero_crossing(samples, s, radius_ms=2, sr=sr) if i > 0 else s
        new_e = snap_zero_crossing(samples, e, radius_ms=2, sr=sr) if i < len(nudged) - 1 else e
        if new_s != s or new_e != e:
            zc_count += 1
        snapped.append((new_s, new_e))
    print(f"  [ZC] Snapped {zc_count}/{len(snapped)} boundary pairs to zero crossings")

    # -- Layer 3: Absolute Timeline Placement & Equal-Power Crossfade --
    print(f"\n  Layer 3: Absolute Timeline Placement & Crossfade ({crossfade_ms}ms, "
          f"{xf_samples} samples)")
          
    duration_s = max(seg["master_end"] for seg in segments)
    canvas_len = int(duration_s * sr) + int(sr * 10)  # 10s extra padding just in case
    channels = samples.shape[1] if samples.ndim > 1 else 1
    canvas = np.zeros((canvas_len, channels), dtype=np.float32)
    
    t = np.linspace(0, 1, xf_samples, dtype=np.float32)
    fade_out = (np.cos(t * np.pi / 2) ** 2)
    fade_in = (np.sin(t * np.pi / 2) ** 2)
        
    for i, (s, e) in enumerate(snapped):
        seg = segments[i]
        master_s = int(seg["master_start"] * sr)
        
        # Extend e by xf_samples if not the last segment, so it overlaps with the next
        if i < len(snapped) - 1:
            e = min(len(samples), e + xf_samples)
            
        audio = samples[s:e].copy()
            
        if canvas.ndim > 1 and audio.ndim == 1:
            audio = audio[:, np.newaxis]
        elif canvas.ndim == 1 and audio.ndim > 1:
            audio = audio.squeeze()
            
        f_in = fade_in[:, np.newaxis] if audio.ndim == 2 else fade_in.ravel()
        f_out = fade_out[:, np.newaxis] if audio.ndim == 2 else fade_out.ravel()
        
        # Apply equal-power crossfades
        if i > 0 and len(audio) >= xf_samples:
            audio[:xf_samples] = audio[:xf_samples] * f_in
        if i < len(snapped) - 1 and len(audio) >= xf_samples:
            audio[-xf_samples:] = audio[-xf_samples:] * f_out
            
        write_e = min(master_s + len(audio), canvas_len)
        read_e = write_e - master_s
        print(f"[DEBUG] Seg {i}: master_s={master_s}, write_e={write_e}, audio_len={len(audio)}, read_e={read_e}", flush=True)
        if read_e > 0:
            canvas[master_s:write_e] += audio[:read_e]

    exact_samples = int(duration_s * sr)
    return canvas[:exact_samples]


def run_smooth_stitch(plot_file, input_audio, output_audio,
                      crossfade_ms=50, vad_search_ms=500,
                      vad_threshold=0.02, silence_ms=200,
                      scratch_dir=None):
    """
    End-to-end smooth stitch: parse plot -> decode -> stitch -> encode.

    This is the main entry point for both CLI and batch_orchestrator import.

    Args:
        plot_file:     Path to describealign alignment plot .txt
        input_audio:   Path to source AD audio file (any format)
        output_audio:  Path for final encoded output (.m4a, .ac3, .flac)
        scratch_dir:   Working directory for intermediate files (auto-created)

    Returns:
        Path to the output audio file.
    """
    ffmpeg = find_ffmpeg()

    print("=" * 64)
    print("  SMOOTH DTW AUDIO STITCHER")
    print("  VAD-Aware Nudging * Zero-Crossing Snap * Equal-Power Crossfade")
    print("=" * 64)

    # -- Parse alignment plot --
    segments = parse_alignment_plot(plot_file)
    if not segments:
        raise ValueError(f"No segments found in alignment plot: {plot_file}")

    print(f"\n  Parsed {len(segments)} segments from alignment plot:")
    for i, seg in enumerate(segments):
        dur = seg["audio_end"] - seg["audio_start"]
        rate_str = f" (rate: +{seg['rate_pct']:.1f}%)" if seg["rate_pct"] > 0 else ""
        print(f"    Seg {i+1}: AD [{fmt_time(seg['audio_start'])} -> "
              f"{fmt_time(seg['audio_end'])}] ({dur:.3f}s){rate_str}")

    # -- Scratch directory --
    if scratch_dir is None:
        scratch_dir = os.path.join(os.path.dirname(output_audio), "scratch")
    os.makedirs(scratch_dir, exist_ok=True)

    # -- Pre-decode to PCM WAV --
    basename = os.path.splitext(os.path.basename(input_audio))[0]
    analysis_wav = os.path.join(scratch_dir, f"{basename}_pcm48k.wav")

    if os.path.exists(analysis_wav) and os.path.getsize(analysis_wav) > 1000:
        print(f"\n  Using cached analysis WAV: {os.path.basename(analysis_wav)}")
    else:
        print(f"\n  Pre-decoding to PCM 48kHz WAV...")
        predecode_to_pcm(input_audio, analysis_wav, ffmpeg, sample_rate=48000)

    # -- Load samples --
    samples, sr = load_wav(analysis_wav)
    duration = len(samples) / sr
    print(f"  Loaded {len(samples):,} samples ({duration:.1f}s) at {sr}Hz")

    # -- Smooth stitch --
    print(f"\n{'-' * 50}")
    print("  APPLYING 3-LAYER TRANSITION SMOOTHING")
    print(f"{'-' * 50}")

    stitched = smooth_stitch_audio(
        segments, samples, sr,
        crossfade_ms=crossfade_ms,
        vad_search_ms=vad_search_ms,
        vad_threshold=vad_threshold,
        silence_ms=silence_ms,
    )

    out_duration = len(stitched) / sr
    print(f"\n  Stitched result: {out_duration:.1f}s ({len(stitched):,} samples)")

    # -- Write intermediate WAV --
    raw_wav = os.path.join(scratch_dir, f"{basename}_stitched_raw.wav")
    write_wav(stitched, sr, raw_wav)

    # -- Encode to final format --
    print(f"\n  Encoding final output tracks...")
    
    ext = os.path.splitext(output_audio)[1].lower()
    base_out = os.path.splitext(output_audio)[0]
    
    output_standard = f"{base_out}{ext}"
    
    if ext in (".m4a", ".aac"):
        codec = ["-c:a", "aac", "-b:a", "256k"]
    elif ext == ".ac3":
        codec = ["-c:a", "ac3", "-b:a", "384k"]
    elif ext == ".flac":
        codec = ["-c:a", "flac"]
    else:
        codec = ["-c:a", "aac", "-b:a", "256k"]

    # Encode Standard (Unnormalized, just limited to prevent clipping)
    encode_std_cmd = [
        ffmpeg, "-y", "-nostdin", "-i", raw_wav,
        "-af", "alimiter=level_in=1:level_out=0.98:limit=0.98",
        "-ar", "48000",
        *codec,
        "-vn",
        output_standard,
    ]
    
    # Encode Night Mode (Normalized EBU R128)
        
    for cmd, name in [(encode_std_cmd, "Standard")]:
        print(f"    -> Encoding {name} track...")
        print(f"       COMMAND: {' '.join(cmd)}")
        res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if res.returncode != 0:
            raise RuntimeError(f"FFmpeg encode failed for {name}:\n{res.stderr.decode('utf-8', errors='ignore')}")

    print(f"\n  [SUCCESS] Smooth-stitched outputs:")
    print(f"    - {output_standard}")
    print("=" * 64)
    return output_standard


# --- CLI Entry Point ---------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Smooth DTW Audio Stitcher -- "
                    "VAD-aware transitions for describealign alignment plots",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Basic stitch with default settings (50ms crossfade, +/-500ms VAD search):
  python dtw_smooth_stitch.py alignment.txt AD_track.mka output.m4a

  # Wider VAD search for episodes with very long narration tails:
  python dtw_smooth_stitch.py alignment.txt AD.mka output.m4a --vad-search-ms 800

  # Minimal crossfade (25ms) for tightly-edited content:
  python dtw_smooth_stitch.py alignment.txt AD.mka output.m4a --crossfade-ms 25
""",
    )
    parser.add_argument("plot_file", help="describealign alignment plot .txt file")
    parser.add_argument("input_audio", help="Source AD audio file (any format)")
    parser.add_argument("output_audio", help="Output stitched audio (.m4a, .ac3, .flac)")
    parser.add_argument("--crossfade-ms", type=int, default=50,
                        help="Crossfade duration in ms (default: 50)")
    parser.add_argument("--vad-search-ms", type=int, default=500,
                        help="VAD boundary search window in ms (default: 500)")
    parser.add_argument("--vad-threshold", type=float, default=0.02,
                        help="RMS threshold for speech detection, 0-1 scale (default: 0.02)")
    parser.add_argument("--silence-ms", type=int, default=200,
                        help="Required consecutive silence for clean boundary (default: 200)")
    parser.add_argument("--scratch-dir", default=None,
                        help="Working directory for intermediate files")
    args = parser.parse_args()

    try:
        run_smooth_stitch(
            args.plot_file,
            args.input_audio,
            args.output_audio,
            crossfade_ms=args.crossfade_ms,
            vad_search_ms=args.vad_search_ms,
            vad_threshold=args.vad_threshold,
            silence_ms=args.silence_ms,
            scratch_dir=args.scratch_dir,
        )
    except Exception as e:
        print(f"\n  [ERROR] {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
