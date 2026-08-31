#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""
dtw_smooth_stitch.py - Smooth DTW Audio Stitcher with VAD-Aware Transitions

Replaces the old hard-cut `build_dtw_stitch.py` with a Multilayer transition
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
import subprocess
import argparse
import shutil
import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.join(os.path.dirname(SCRIPT_DIR), "bin")


# --- Utilities ----------------------------------------------------------

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
                "rate_pct": float(m.group(1)),
                "master_start": time_to_sec(m.group(2)),
                "master_end": time_to_sec(m.group(3)),
                "audio_start": time_to_sec(m.group(4)),
                "audio_end": time_to_sec(m.group(5)),
            })
        else:
            m2 = audio_re.search(line)
            if m2:
                segments.append({
                    "rate_pct": 0.0,
                    "master_start": 0.0,
                    "master_end": 0.0,
                    "audio_start": time_to_sec(m2.group(1)),
                    "audio_end": time_to_sec(m2.group(2)),
                })

    # Merge through micro-glitches: when a tiny hallucinated segment sits
    # between two real segments, removing it creates a hard cut at a bad spot.
    # Instead, extend the previous real segment's master_end/audio_end to bridge
    # the gap, eliminating the phantom boundary entirely.
    cleaned = []
    for s in segments:
        dur_master = s["master_end"] - s["master_start"]
        is_glitch = dur_master < 0.5 and abs(s["rate_pct"]) > 10.0

        if is_glitch and len(cleaned) > 0:
            # Bridge: extend previous segment to cover this glitch
            prev = cleaned[-1]
            prev["master_end"] = s["master_end"]
            prev["audio_end"] = s["audio_end"]
        else:
            cleaned.append(s)

    if not cleaned:
        cleaned = segments

    # --- Plateau Consolidation ---
    # Group jittery DTW segments that share the same baseline offset (within 120ms)
    # into unified structural reels/scenes. This prevents DescribeAlign's 100ms
    # quantization bins from chopping unbroken dialogue into dozens of micro-cuts.
    plateaus = []
    if cleaned:
        current_plateau = [cleaned[0]]
        current_plateau_offset = cleaned[0]["master_start"] - cleaned[0]["audio_start"]

        for i in range(1, len(cleaned)):
            s = cleaned[i]
            seg_offset = s["master_start"] - s["audio_start"]
            delta = abs(seg_offset - current_plateau_offset)

            if delta < 0.120:
                current_plateau.append(s)
            else:
                # Real structural edit or reel change (>120ms shift)
                p_m_start = current_plateau[0]["master_start"]
                p_m_end = current_plateau[-1]["master_end"]
                p_a_start = current_plateau[0]["audio_start"]
                p_a_end = current_plateau[-1]["audio_end"]
                m_dur = p_m_end - p_m_start
                a_dur = p_a_end - p_a_start
                rate = a_dur / m_dur if m_dur > 0 else 1.0
                plateaus.append({
                    "master_start": p_m_start,
                    "master_end": p_m_end,
                    "audio_start": p_a_start,
                    "audio_end": p_a_end,
                    "rate_pct": (rate - 1.0) * 100.0,
                })
                current_plateau = [s]
                current_plateau_offset = seg_offset

        if current_plateau:
            p_m_start = current_plateau[0]["master_start"]
            p_m_end = current_plateau[-1]["master_end"]
            p_a_start = current_plateau[0]["audio_start"]
            p_a_end = current_plateau[-1]["audio_end"]
            m_dur = p_m_end - p_m_start
            a_dur = p_a_end - p_a_start
            rate = a_dur / m_dur if m_dur > 0 else 1.0
            plateaus.append({
                "master_start": p_m_start,
                "master_end": p_m_end,
                "audio_start": p_a_start,
                "audio_end": p_a_end,
                "rate_pct": (rate - 1.0) * 100.0,
            })

    return plateaus if plateaus else cleaned


# --- Audio I/O ----------------------------------------------------------

def predecode_to_pcm(input_audio, output_wav, ffmpeg, sample_rate=48000):
    """
    Decode any audio format to 16-bit WAV (keeping original channels).
    Uses atomic temp-file pattern to prevent corruption on interrupt.
    """
    tmp = output_wav.replace(".wav", "_tmp.wav")
    cmd = [
        ffmpeg, "-y", "-nostdin", "-hide_banner", "-loglevel", "fatal", "-fflags", "+genpts", "-i", input_audio,
        "-vn", "-af", "asetpts=PTS-STARTPTS", "-ar", str(sample_rate),
        "-c:a", "pcm_s16le", "-f", "wav", tmp,
    ]
    result = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE)
    if result.returncode != 0:
        raise RuntimeError(
            f"FFmpeg decode failed:\n{
                result.stderr.decode(
                    'utf-8',
                    errors='ignore')}")
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
            b = raw_bytes[i * 3: i * 3 + 3]
            val = b[0] | (b[1] << 8) | (b[2] << 16)
            if val >= 0x800000:
                val -= 0x1000000
            arr[i] = val / 8388608.0
    elif width == 4:
        arr = np.frombuffer(
            raw, dtype=np.int32).astype(
            np.float32) / 2147483648.0
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

        chunk = samples[idx: idx + win_samples]
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
                        f"  [VAD] Seg {i + 1} end: extended +{delta_ms:.0f}ms "
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
                        f"  [VAD] Seg {i +
                                       1} start: pulled back -{delta_ms:.0f}ms "
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
    fade_in = np.sin(t * np.pi / 2)   # 0 -> 1

    if segments_audio[0].ndim == 2:
        fade_out = fade_out[:, np.newaxis]
        fade_in = fade_in[:, np.newaxis]

    # Calculate total output length
    total = sum(len(s) for s in segments_audio) - \
        xf * (len(segments_audio) - 1)

    if segments_audio[0].ndim == 2:
        output = np.zeros(
            (max(
                total,
                0),
                segments_audio[0].shape[1]),
            dtype=np.float32)
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
        output[pos: pos + write_len] += seg[:write_len]

        # Advance position (minus overlap for next segment)
        if i < len(segments_audio) - 1:
            pos += len(seg) - xf
        else:
            pos += len(seg)

    return np.clip(output, -1.0, 1.0)


# --- Main Pipeline -----------------------------------------------------------

def match_volume(master, ad, sr, block_sec=1.0):
    """
    Computes the median RMS volume of 1-second blocks for both tracks (ignoring absolute silence)
    and scales the master audio to match the AD track's ambient background volume.
    """
    block_samps = int(sr * block_sec)
    
    def get_rms(arr):
        # Convert to mono for calculation
        if arr.ndim > 1:
            arr = np.mean(arr, axis=1)
        
        n_blocks = len(arr) // block_samps
        if n_blocks == 0:
            return np.array([])
        
        blocks = arr[:n_blocks * block_samps].reshape(n_blocks, block_samps)
        return np.sqrt(np.mean(blocks**2, axis=1))

    m_rms = get_rms(master)
    a_rms = get_rms(ad)
    
    # Ignore absolute silence blocks
    m_rms = m_rms[m_rms > 0.0001]
    a_rms = a_rms[a_rms > 0.0001]
    
    if len(m_rms) == 0 or len(a_rms) == 0:
        return master
        
    m_med = np.median(m_rms)
    a_med = np.median(a_rms)
    
    if m_med > 0:
        scale = a_med / m_med
        print(f"  [VOLUME MATCH] Scaling Master Audio by {scale:.3f}x to match AD track ambient RMS", flush=True)
        return master * scale
    return master


def smooth_stitch_audio(segments, samples, sr,
                        crossfade_ms=50, vad_search_ms=500,
                        vad_threshold=0.02, silence_ms=200,
                        debug=False, master_samples=None):
    """
    Full Multilayer smooth stitching pipeline.

    Args:
        segments:  List of dicts from parse_alignment_plot() with audio_start/audio_end
        samples:   Float32 numpy array of the full source AD audio (mono or stereo)
        sr:        Sample rate

    Returns:
        Stitched float32 numpy array.
    """
    # -------------------------------------------------------------------------
    print(
        "\n  Layer 0: Verifying Structural Segment Rates & Gap Continuity...",
        flush=True)
    filtered_segments = []

    for i, seg in enumerate(segments):
        audio_dur = seg["audio_end"] - seg["audio_start"]
        master_dur = seg["master_end"] - seg["master_start"]

        if master_dur > 0:
            implied_rate = audio_dur / master_dur
        else:
            implied_rate = 1.0
        seg["implied_rate"] = implied_rate
        filtered_segments.append(seg)



    # Pass 2: Prevent Audio Replay (Backward Jumps)
    # If the AD track is missing a scene, DescribeAlign might force a continuous master timeline
    # by jumping backwards in the audio (replaying audio). We fix this by truncating the previous
    # segment, creating a natural silence gap in the master timeline.
    for i in range(len(filtered_segments) - 1):
        curr_seg = filtered_segments[i]
        next_seg = filtered_segments[i+1]
        
        if next_seg["audio_start"] < curr_seg["audio_end"]:
            overlap = curr_seg["audio_end"] - next_seg["audio_start"]
            if overlap > 0.01:
                print(f"  [GAP FIX] Seg {i+1} overlaps Seg {i+2} by {overlap:.3f}s. Truncating Seg {i+1} to insert silence.", flush=True)
                if next_seg["audio_start"] <= curr_seg["audio_start"]:
                    curr_seg["audio_end"] = curr_seg["audio_start"]
                else:
                    curr_seg["audio_end"] = next_seg["audio_start"]
                
                new_audio_dur = curr_seg["audio_end"] - curr_seg["audio_start"]
                impl_rate = curr_seg.get("implied_rate", 1.0)
                if impl_rate > 0:
                    curr_seg["master_end"] = curr_seg["master_start"] + (new_audio_dur / impl_rate)
                else:
                    curr_seg["master_end"] = curr_seg["master_start"]

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
        new_s = snap_zero_crossing(
            samples, s, radius_ms=2, sr=sr) if i > 0 else s
        new_e = snap_zero_crossing(
            samples, e, radius_ms=2, sr=sr) if i < len(nudged) - 1 else e
        if new_s != s or new_e != e:
            zc_count += 1
        snapped.append((new_s, new_e))
    print(
        f"  [ZC] Snapped {zc_count}/{len(snapped)} boundary pairs to zero crossings")

    # -- Layer 3: Absolute Timeline Placement & Equal-Power Crossfade --
    print(
        f"\n  Layer 3: Absolute Timeline Placement & Crossfade ({crossfade_ms}ms, {xf_samples} samples)",
        flush=True)
    
    in_gap = True

    duration_s = max(seg["master_end"] for seg in segments)
    # 10s extra padding just in case
    canvas_len = int(duration_s * sr) + int(sr * 10)
    channels = samples.shape[1] if samples.ndim > 1 else 1
    
    if master_samples is not None:
        master_samples = match_volume(master_samples, samples, sr, block_sec=1.0)
        if master_samples.ndim == 1 and channels > 1:
            master_samples = np.column_stack([master_samples] * channels)
        elif master_samples.ndim > 1 and channels == 1:
            master_samples = master_samples[:, 0]
            
        if len(master_samples) >= canvas_len:
            canvas = master_samples[:canvas_len].copy()
        else:
            # Pad with silence if master_samples is shorter than canvas_len
            pad_len = canvas_len - len(master_samples)
            if channels > 1:
                padding = np.zeros((pad_len, channels), dtype=np.float32)
                canvas = np.vstack([master_samples, padding])
            else:
                padding = np.zeros(pad_len, dtype=np.float32)
                canvas = np.concatenate([master_samples, padding])
        print("  [GAP FILL] Initialized canvas with master audio to naturally fill missing AD scenes.", flush=True)
    else:
        canvas = np.zeros((canvas_len, channels), dtype=np.float32)

    t = np.linspace(0, 1, xf_samples, dtype=np.float32)
    fade_out = (np.cos(t * np.pi / 2) ** 2)
    fade_in = (np.sin(t * np.pi / 2) ** 2)

    for i, (s, e) in enumerate(snapped):
        seg = segments[i]
        
        implied_rate = seg.get("implied_rate", 1.0)
        
        # Audio was nudged by Layer 1 (VAD) and Layer 2 (Zero-Crossing).
        # We MUST shift the master_s by the exact same duration to prevent sync drift and overlapping doubles!
        audio_start_orig = int(seg["audio_start"] * sr)
        audio_shift = s - audio_start_orig
        
        # Apply the implied rate to the shift
        master_shift = int(audio_shift / implied_rate) if implied_rate > 0 else audio_shift
        master_s = int(seg["master_start"] * sr) + master_shift
        if master_s < 0:
            master_s = 0

        # Extend e by xf_samples if not the last segment, so it overlaps with
        # the next
        if i < len(snapped) - 1:
            e = min(len(samples), e + xf_samples)

        audio = samples[s:e].copy()

        # Layer 3.5 requires the original implied_rate from Layer 0 before VAD nudging/Wobble correction altered it
        implied_rate = seg.get("implied_rate", 1.0)

        # Layer 3.5: If the rate drift is genuine but tiny (< 5.0%), seamlessly
        # stretch the audio segment using linear interpolation to perfectly fill
        # the master duration. This completely prevents continuous timeline drift
        # without inserting gaps, overlaps, or WSOLA artifacts. Supports PAL mismatches (4.27%).
        if 0.000001 < abs(1.0 - implied_rate) <= 0.05 and len(audio) > 0:
            target_len = int(len(audio) / implied_rate)
            print(f"  [STRETCH] Resampling Seg {i+1} by {1.0/implied_rate:.5f}x to perfectly correct {abs(1.0 - implied_rate)*100:.3f}% timeline drift", flush=True)
            
            # Use float64 for indices to prevent precision loss (float32 maxes out at 16M unique values)
            indices = np.linspace(0, len(audio) - 1, target_len, dtype=np.float64)
            idx0 = np.floor(indices).astype(np.int32)
            idx1 = np.minimum(idx0 + 1, len(audio) - 1)
            weight1 = (indices - idx0).astype(np.float32)
            weight0 = 1.0 - weight1
            
            if audio.ndim == 1:
                audio = (audio[idx0] * weight0 + audio[idx1] * weight1).astype(np.float32)
            else:
                weight0 = weight0[:, np.newaxis]
                weight1 = weight1[:, np.newaxis]
                audio = (audio[idx0] * weight0 + audio[idx1] * weight1).astype(np.float32)

        # AIRTIGHT CLAMP: Prevent any segment from bleeding into the next segment's timeline.
        # This handles extreme DescribeAlign cuts (e.g. 10s of deleted AD audio squeezed into 1ms of Master video)
        if i < len(snapped) - 1:
            next_seg = segments[i+1]
            next_s = snapped[i+1][0]
            next_audio_start_orig = int(next_seg["audio_start"] * sr)
            next_audio_shift = next_s - next_audio_start_orig
            next_implied_rate = next_seg.get("implied_rate", 1.0)
            next_master_shift = int(next_audio_shift / next_implied_rate) if next_implied_rate > 0 else next_audio_shift
            next_master_s = int(next_seg["master_start"] * sr) + next_master_shift
            
            allowed_len = (next_master_s - master_s) + xf_samples
            if allowed_len < 0:
                allowed_len = 0
            if len(audio) > allowed_len:
                audio = audio[:allowed_len]

        if canvas.ndim > 1 and audio.ndim == 1:
            audio = audio[:, np.newaxis]
        elif canvas.ndim == 1 and audio.ndim > 1:
            audio = audio.squeeze()

        f_in = fade_in[:, np.newaxis] if audio.ndim == 2 else fade_in.ravel()
        f_out = fade_out[:,
                         np.newaxis] if audio.ndim == 2 else fade_out.ravel()

        # ---------------------------------------------------------------------
        # Apply equal-power crossfades and manage Master Audio Gap Filling
        # ---------------------------------------------------------------------
        write_e = min(master_s + len(audio), canvas_len)
        read_e = write_e - master_s
        if read_e <= 0:
            continue

        if (e - s) < 100:
            # Deleted scene / timeline void (no AD audio for this master duration).
            # Skip writing AD audio so Master Audio flows continuously through the gap.
            continue

        # Find the next real segment (with non-zero audio) to check for gaps
        next_real_master_s = None
        for j in range(i + 1, len(snapped)):
            s_j, e_j = snapped[j]
            if (e_j - s_j) >= 100:
                seg_j = segments[j]
                rate_j = seg_j.get("implied_rate", 1.0)
                shift_j = s_j - int(seg_j["audio_start"] * sr)
                m_shift_j = int(shift_j / rate_j) if rate_j > 0 else shift_j
                next_real_master_s = int(seg_j["master_start"] * sr) + m_shift_j
                break

        gap_after = True
        if next_real_master_s is not None and next_real_master_s <= (master_s + len(audio) + 100):
            gap_after = False

        if len(audio) >= xf_samples:
            # 1. Start of Segment
            if in_gap:
                # Fade OUT the Master Audio underneath the new AD segment
                canvas[master_s : master_s + xf_samples] *= f_out
            audio[:xf_samples] = audio[:xf_samples] * f_in

            # 2. Body of Segment
            # Zero out Master Audio where AD plays solo
            body_s = master_s + xf_samples
            body_e = write_e - xf_samples
            if body_e > body_s:
                canvas[body_s : body_e] = 0.0

            # 3. End of Segment
            audio[-xf_samples:] = audio[-xf_samples:] * f_out
            if gap_after:
                # Fade IN the Master Audio after the AD segment ends
                canvas[write_e - xf_samples : write_e] *= f_in
            else:
                # The next AD segment will overlap here, so zero out the Master Audio
                canvas[write_e - xf_samples : write_e] = 0.0
        else:
            # Short segment (<50ms): zero out Master Audio underneath it
            canvas[master_s : write_e] = 0.0

        if debug:
            print(f"[DEBUG] Seg {i+1}: master_s={master_s}, write_e={write_e}, audio_len={len(audio)}, gap_after={gap_after}", flush=True)

        canvas[master_s:write_e] += audio[:read_e]
        in_gap = gap_after

    exact_samples = int(duration_s * sr)
    return canvas[:exact_samples]


def run_smooth_stitch(plot_file, input_audio, output_audio,
                      crossfade_ms=50, vad_search_ms=500,
                      vad_threshold=0.02, silence_ms=200,
                      scratch_dir=None, debug=False):
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
        print(f"    Seg {i + 1}: AD [{fmt_time(seg['audio_start'])} -> "
              f"{fmt_time(seg['audio_end'])}] ({dur:.3f}s){rate_str}")

    # -- Scratch directory --
    if scratch_dir is None:
        scratch_dir = os.path.join(os.path.dirname(output_audio), "scratch")
    os.makedirs(scratch_dir, exist_ok=True)

    # -- Pre-decode to PCM WAV --
    basename = os.path.splitext(os.path.basename(input_audio))[0]
    analysis_wav = os.path.join(scratch_dir, f"{basename}_pcm48k.wav")

    if os.path.exists(analysis_wav) and os.path.getsize(analysis_wav) > 1000:
        print(
            f"\n  Using cached analysis WAV: {
                os.path.basename(analysis_wav)}")
    else:
        print(f"\n  Pre-decoding to PCM 48kHz WAV...")
        predecode_to_pcm(input_audio, analysis_wav, ffmpeg, sample_rate=48000)

    # -- Load samples --
    samples, sr = load_wav(analysis_wav)
    duration = len(samples) / sr
    print(f"  Loaded {len(samples):,} samples ({duration:.1f}s) at {sr}Hz")

    # -- Smooth stitch --
    print(f"\n{'-' * 50}")
    print("  APPLYING MULTILAYER TRANSITION SMOOTHING")
    print(f"{'-' * 50}")

    stitched = smooth_stitch_audio(
        segments, samples, sr,
        crossfade_ms=crossfade_ms,
        vad_search_ms=vad_search_ms,
        vad_threshold=vad_threshold,
        silence_ms=silence_ms,
        debug=debug,
    )

    out_duration = len(stitched) / sr
    print(
        f"\n  Stitched result: {
            out_duration:.1f}s ({
            len(stitched):,}  samples)")

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
        res = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        if res.returncode != 0:
            raise RuntimeError(
                f"FFmpeg encode failed for {name}:\n{
                    res.stderr.decode(
                        'utf-8', errors='ignore')}")

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
    parser.add_argument(
        "plot_file",
        help="describealign alignment plot .txt file")
    parser.add_argument(
        "input_audio",
        help="Source AD audio file (any format)")
    parser.add_argument(
        "output_audio",
        help="Output stitched audio (.m4a, .ac3, .flac)")
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
    parser.add_argument("--debug", action="store_true",
                        help="Enable detailed debug logging (cut coordinates and lengths)")
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
            debug=args.debug,
        )
    except Exception as e:
        print(f"\n  [ERROR] {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
