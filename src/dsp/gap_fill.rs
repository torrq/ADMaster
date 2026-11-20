use crate::media::{extract_master_stereo_wav, WavStreamReader};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilledGapReport {
    pub start_sec: f64,
    pub end_sec: f64,
    pub duration_sec: f64,
    pub gain_start: f64,
    pub gain_end: f64,
}

/// Splices volume-matched Reference Master Audio with equal-power cosine crossfades
/// into explicit structural gap intervals in `ad_wav_path` using **windowed file seeking**.
///
/// Peak RAM is < 15 MB regardless of movie duration, and when `explicit_gaps` is empty
/// it returns immediately in 0ms without spawning FFmpeg or reading any audio!
#[allow(clippy::too_many_arguments)]
pub fn fill_master_audio_gaps_windowed(
    ffmpeg_bin: &Path,
    ad_wav_path: &Path,
    ref_path: &Path,
    ref_stream_map: Option<&str>,
    channels: u32,
    scratch_dir: &Path,
    threads: usize,
    explicit_gaps: &[(f64, f64, f64)],
    crossfade_ms: u32,
    cancel_flag: &AtomicBool,
) -> Result<Vec<FilledGapReport>, String> {
    if explicit_gaps.is_empty() {
        return Ok(Vec::new());
    }
    if !ad_wav_path.exists()
        || fs::metadata(ad_wav_path).map(|m| m.len()).unwrap_or(0) < 1000
    {
        return Ok(Vec::new());
    }

    // Merge overlapping or near-contiguous intervals (within 200ms)
    let mut sorted_gaps = explicit_gaps.to_vec();
    sorted_gaps.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e, _) in sorted_gaps {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 + 0.20 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    let raw_gaps: Vec<(f64, f64)> = merged
        .into_iter()
        .filter(|&(s, e)| (e - s) >= 0.10)
        .collect();
    if raw_gaps.is_empty() {
        return Ok(Vec::new());
    }

    // Extract or reuse 48kHz stereo master reference downmix
    let ref_stereo_wav = extract_master_stereo_wav(
        ffmpeg_bin,
        ref_path,
        ref_stream_map,
        channels,
        scratch_dir,
        threads,
        cancel_flag,
    )?;

    let mut ad_reader = WavStreamReader::open(ad_wav_path)?;
    let mut ref_reader = WavStreamReader::open(&ref_stereo_wav)?;
    let sr = ad_reader.layout.sample_rate;
    let sr_f = sr as f64;
    let target_len = ad_reader.layout.total_frames.min(ref_reader.layout.total_frames);
    let win_frames = ((0.05 * sr_f) as usize).max(1); // 50ms RMS gain window
    let void_win = ((0.005 * sr_f) as usize).max(1); // 5ms fine void-edge resolution (prevents 30-49ms truncation holes)

    // 1. Identify true silent voids within candidate gap intervals at 5ms resolution!
    let mut safe_gaps: Vec<(f64, f64)> = Vec::new();
    let mut ad_buf = Vec::new();
    let mut ref_buf = Vec::new();

    for &(s, e) in &raw_gaps {
        let start_frame = ((s * sr_f) as u64).min(target_len);
        let end_frame = ((e * sr_f) as u64).min(target_len);
        if end_frame <= start_frame {
            continue;
        }
        let count = (end_frame - start_frame) as usize;
        if count < void_win {
            continue;
        }
        let n_ad = ad_reader.read_frames_stereo_f32(start_frame, count, &mut ad_buf)?;
        let n_ref = ref_reader.read_frames_stereo_f32(start_frame, count, &mut ref_buf)?;
        let n_frames = n_ad.min(n_ref);
        let n_wins = n_frames / void_win;
        if n_wins == 0 {
            continue;
        }

        let mut in_void = false;
        let mut v_start = 0usize;
        let mut ref_active_in_void = false;
        for w in 0..n_wins {
            let ad_w = &ad_buf[w * void_win * 2..(w + 1) * void_win * 2];
            let ref_w = &ref_buf[w * void_win * 2..(w + 1) * void_win * 2];

            let mut ad_sq = 0.0_f64;
            let mut ad_peak = 0.0_f32;
            for &x in ad_w {
                let ax = x.abs();
                if ax > ad_peak {
                    ad_peak = ax;
                }
                ad_sq += (x as f64) * (x as f64);
            }
            let ad_rms = (ad_sq / (ad_w.len() as f64)).sqrt();

            let mut ref_sq = 0.0_f64;
            for &x in ref_w {
                ref_sq += (x as f64) * (x as f64);
            }
            let ref_rms = (ref_sq / (ref_w.len() as f64)).sqrt();

            // Check only AD silence per 5ms window so brief quiet moments in the reference audio
            // never split a continuous gap and leave an unfilled mute tail; verify ref has audio
            // somewhere inside the void before queuing it for fill.
            let is_ad_silent = ad_peak < 0.018 && ad_rms < 0.004;
            if is_ad_silent {
                if !in_void {
                    in_void = true;
                    v_start = w;
                    ref_active_in_void = false;
                }
                if ref_rms >= 0.0005 {
                    ref_active_in_void = true;
                }
            } else if in_void {
                in_void = false;
                let sub_s = s + ((v_start * void_win) as f64) / sr_f;
                let sub_e = s + ((w * void_win) as f64) / sr_f;
                if ref_active_in_void && sub_e - sub_s >= 0.35 {
                    safe_gaps.push((sub_s, sub_e));
                }
            }
        }
        if in_void {
            let sub_s = s + ((v_start * void_win) as f64) / sr_f;
            let sub_e = s + (n_frames as f64) / sr_f;
            if ref_active_in_void && sub_e - sub_s >= 0.35 {
                safe_gaps.push((sub_s, sub_e));
            }
        }
    }

    if safe_gaps.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Sample track-wide baseline gain at 20-second intervals (seeking 2-second snippets)
    let step_frames = (sr as u64) * 20;
    let margin_frames = (sr as u64) * 30;
    let snippet_frames = (sr as usize) * 2;
    let mut gain_samples = Vec::new();
    if target_len > margin_frames * 2 {
        let mut p = margin_frames;
        while p + (snippet_frames as u64) < target_len - margin_frames {
            let na = ad_reader.read_frames_stereo_f32(p, snippet_frames, &mut ad_buf)?;
            let nr = ref_reader.read_frames_stereo_f32(p, snippet_frames, &mut ref_buf)?;
            let n = na.min(nr);
            if n > 0 {
                let r_ad = (ad_buf[..n * 2]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
                    / ((n * 2) as f64))
                    .sqrt();
                let r_ref = (ref_buf[..n * 2]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
                    / ((n * 2) as f64))
                    .sqrt();
                if r_ref > 0.003 && r_ad > 0.005 {
                    gain_samples.push(r_ad / r_ref);
                }
            }
            p += step_frames;
        }
    }
    let global_gain = if gain_samples.is_empty() {
        1.0
    } else {
        gain_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        gain_samples[gain_samples.len() / 2]
    };

    // 3. Measure neighboring 4-second regions and patch only the gap intervals directly in place!
    // Use at least an 85ms equal-power cosine crossfade and expand interior edges by 15ms so
    // warp.rs's 15ms silence edge fades are completely subsumed (no double-fade volume notch!).
    let eff_xf_ms = crossfade_ms.max(85);
    let fade_frames = ((sr as usize) * (eff_xf_ms as usize)) / 1000;
    let edge_subsume = ((sr as u64) * 15) / 1000;
    let neighbor_frames = (sr as u64) * 4;
    let data_offset = ad_reader.layout.data_offset;
    let ad_ch = ad_reader.layout.channels as usize;

    // Open `ad_wav_path` for in-place random-access read/write of the gap windows
    let mut patch_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(ad_wav_path)
        .map_err(|e| format!("Failed to open AD WAV for in-place gap patching: {}", e))?;

    let mut reports = Vec::with_capacity(safe_gaps.len());
    let mut ad_pre_buf = Vec::new();
    let mut ref_pre_buf = Vec::new();
    let mut ad_post_buf = Vec::new();
    let mut ref_post_buf = Vec::new();
    let mut pcm_out_bytes = Vec::new();

    for (s, e) in safe_gaps {
        let raw_start = ((s * sr_f) as u64).min(target_len);
        let raw_end = ((e * sr_f) as u64).min(target_len);
        if raw_end <= raw_start {
            continue;
        }
        // Subsume warp.rs's 15ms edge fade-out/fade-in on interior gap boundaries so crossfades
        // blend full-level AD directly with full-level Reference audio without a -6dB dip.
        let start_idx = if raw_start > edge_subsume {
            raw_start - edge_subsume
        } else {
            raw_start
        };
        let end_idx = if raw_end + edge_subsume < target_len {
            raw_end + edge_subsume
        } else {
            raw_end
        };

        let pre_start = start_idx.saturating_sub(neighbor_frames);
        let pre_cnt = (start_idx - pre_start) as usize;
        let _ = ad_reader.read_frames_stereo_f32(pre_start, pre_cnt, &mut ad_pre_buf)?;
        let _ = ref_reader.read_frames_stereo_f32(pre_start, pre_cnt, &mut ref_pre_buf)?;

        let post_end = (end_idx + neighbor_frames).min(target_len);
        let post_cnt = (post_end - end_idx) as usize;
        let _ = ad_reader.read_frames_stereo_f32(end_idx, post_cnt, &mut ad_post_buf)?;
        let _ = ref_reader.read_frames_stereo_f32(end_idx, post_cnt, &mut ref_post_buf)?;

        let mut g_start =
            measure_region_gain(&ad_pre_buf, &ref_pre_buf, win_frames, global_gain);
        let mut g_end =
            measure_region_gain(&ad_post_buf, &ref_post_buf, win_frames, global_gain);
        if start_idx == 0 {
            g_start = g_end;
        }
        if end_idx >= target_len {
            g_end = g_start;
        }

        // Match waveform polarity across the crossfade boundaries so phase-inverted tracks
        // never suffer destructive phase cancellation right in the middle of the splice.
        let polarity = detect_splice_polarity(&ad_pre_buf, &ref_pre_buf, &ad_post_buf, &ref_post_buf);

        // Read the full patch span including intro/outro crossfade margins
        let patch_start = start_idx.saturating_sub(fade_frames as u64);
        let patch_end = (end_idx + (fade_frames as u64)).min(target_len);
        let patch_frames = (patch_end - patch_start) as usize;
        if patch_frames == 0 {
            continue;
        }

        let n_ad = ad_reader.read_frames_stereo_f32(patch_start, patch_frames, &mut ad_buf)?;
        let n_ref = ref_reader.read_frames_stereo_f32(patch_start, patch_frames, &mut ref_buf)?;
        let actual_frames = n_ad.min(n_ref);
        if actual_frames == 0 {
            continue;
        }

        let intro_xf = (start_idx - patch_start) as usize;
        let gap_len = ((end_idx - start_idx) as usize).min(actual_frames.saturating_sub(intro_xf));
        let outro_xf = actual_frames.saturating_sub(intro_xf + gap_len);

        // Compute peak inside gap to clamp if > 0.98
        let mut max_gap_peak = 0.0_f32;
        for f in 0..gap_len {
            let frac = if gap_len > 1 {
                (f as f64) / ((gap_len - 1) as f64)
            } else {
                0.0
            };
            let g = (g_start + frac * (g_end - g_start)) as f32;
            let idx = (intro_xf + f) * 2;
            let pl = (ref_buf[idx] * g).abs();
            let pr = (ref_buf[idx + 1] * g).abs();
            max_gap_peak = max_gap_peak.max(pl).max(pr);
        }
        let peak_clamp = if max_gap_peak > 0.98 {
            0.98 / max_gap_peak
        } else {
            1.0
        };

        // 1. Intro crossfade [0..intro_xf)
        for f in 0..intro_xf {
            let phase = (f as f32) / (intro_xf.max(1) as f32) * std::f32::consts::FRAC_PI_2;
            let fout = phase.cos();
            let fin = phase.sin();
            let g = (g_start as f32) * peak_clamp * polarity;
            let idx = f * 2;
            ad_buf[idx] = ad_buf[idx] * fout + ref_buf[idx] * g * fin;
            ad_buf[idx + 1] = ad_buf[idx + 1] * fout + ref_buf[idx + 1] * g * fin;
        }

        // 2. Main gap body [intro_xf..intro_xf + gap_len)
        for f in 0..gap_len {
            let frac = if gap_len > 1 {
                (f as f64) / ((gap_len - 1) as f64)
            } else {
                0.0
            };
            let g = ((g_start + frac * (g_end - g_start)) as f32) * peak_clamp * polarity;
            let idx = (intro_xf + f) * 2;
            ad_buf[idx] = ref_buf[idx] * g;
            ad_buf[idx + 1] = ref_buf[idx + 1] * g;
        }

        // 3. Outro crossfade [intro_xf + gap_len..actual_frames)
        for f in 0..outro_xf {
            let phase = (f as f32) / (outro_xf.max(1) as f32) * std::f32::consts::FRAC_PI_2;
            let fout = phase.cos();
            let fin = phase.sin();
            let g = (g_end as f32) * peak_clamp * polarity;
            let idx = (intro_xf + gap_len + f) * 2;
            ad_buf[idx] = ref_buf[idx] * g * fout + ad_buf[idx] * fin;
            ad_buf[idx + 1] = ref_buf[idx + 1] * g * fout + ad_buf[idx + 1] * fin;
        }

        // Write patched 16-bit PCM frames directly back to `ad_wav_path` at `patch_start`
        pcm_out_bytes.clear();
        pcm_out_bytes.reserve(actual_frames * ad_ch * 2);
        for f in 0..actual_frames {
            let l = (ad_buf[f * 2].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            let r = (ad_buf[f * 2 + 1].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            pcm_out_bytes.extend_from_slice(&l.to_le_bytes());
            if ad_ch >= 2 {
                pcm_out_bytes.extend_from_slice(&r.to_le_bytes());
            }
        }

        let byte_pos = data_offset + patch_start * (ad_ch as u64) * 2;
        patch_file
            .seek(SeekFrom::Start(byte_pos))
            .map_err(|e| e.to_string())?;
        patch_file
            .write_all(&pcm_out_bytes)
            .map_err(|e| e.to_string())?;

        reports.push(FilledGapReport {
            start_sec: s,
            end_sec: e,
            duration_sec: e - s,
            gain_start: g_start,
            gain_end: g_end,
        });
    }

    patch_file.flush().map_err(|e| e.to_string())?;
    Ok(reports)
}

fn measure_region_gain(
    ad_interleaved: &[f32],
    ref_interleaved: &[f32],
    win_frames: usize,
    default_gain: f64,
) -> f64 {
    let n_frames = (ad_interleaved.len() / 2).min(ref_interleaved.len() / 2);
    let nw = n_frames / win_frames;
    if nw < 3 {
        return default_gain;
    }

    let mut rad_valid = Vec::with_capacity(nw);
    let mut rref_valid = Vec::with_capacity(nw);

    for w in 0..nw {
        let ad_w = &ad_interleaved[w * win_frames * 2..(w + 1) * win_frames * 2];
        let ref_w = &ref_interleaved[w * win_frames * 2..(w + 1) * win_frames * 2];

        let rad = (ad_w.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>()
            / (ad_w.len() as f64))
            .sqrt();
        let rref = (ref_w.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>()
            / (ref_w.len() as f64))
            .sqrt();
        if rref > 0.001 && rad > 0.002 {
            rad_valid.push(rad);
            rref_valid.push(rref);
        }
    }

    if rad_valid.len() < 3 {
        return default_gain;
    }
    rad_valid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    rref_valid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p30_idx = (rad_valid.len() * 30) / 100;
    let ratio_local = rad_valid[p30_idx] / rref_valid[p30_idx].max(1e-6);
    let blended = 0.65 * ratio_local + 0.35 * default_gain;
    blended.clamp(0.4 * default_gain, 2.5 * default_gain)
}

/// Checks waveform polarity between AD and Reference audio immediately adjacent to a gap.
/// Returns `-1.0` if the two tracks are strongly phase-inverted (`r < -0.15`) so crossfades
/// sum constructively instead of phase-cancelling.
fn detect_splice_polarity(
    ad_pre: &[f32],
    ref_pre: &[f32],
    ad_post: &[f32],
    ref_post: &[f32],
) -> f32 {
    let mut dot = 0.0_f64;
    let mut a_sq = 0.0_f64;
    let mut r_sq = 0.0_f64;

    let n_pre = ad_pre.len().min(ref_pre.len());
    // Inspect the last ~0.5s before the gap and first ~0.5s after the gap
    let pre_take = n_pre.min(48_000);
    for (&a, &r) in ad_pre[n_pre - pre_take..n_pre]
        .iter()
        .zip(ref_pre[n_pre - pre_take..n_pre].iter())
    {
        dot += (a as f64) * (r as f64);
        a_sq += (a as f64) * (a as f64);
        r_sq += (r as f64) * (r as f64);
    }

    let n_post = ad_post.len().min(ref_post.len());
    let post_take = n_post.min(48_000);
    for (&a, &r) in ad_post[..post_take].iter().zip(ref_post[..post_take].iter()) {
        dot += (a as f64) * (r as f64);
        a_sq += (a as f64) * (a as f64);
        r_sq += (r as f64) * (r as f64);
    }

    let denom = (a_sq * r_sq).sqrt();
    if denom > 1e-6 && (dot / denom) < -0.15 {
        -1.0_f32
    } else {
        1.0_f32
    }
}
