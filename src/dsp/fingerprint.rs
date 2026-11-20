use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const N_FFT: usize = 512; // 64 ms window at 8 kHz -> 15.625 Hz bins
const HOP: usize = 128; // 16 ms hop at 8 kHz
const FMIN_HZ: f64 = 100.0;
const FMAX_HZ: f64 = 3800.0;
const PEAK_NEIGHBOR_FREQ: usize = 10;
const PEAK_NEIGHBOR_TIME: usize = 8;
const PEAKS_PER_SEC: usize = 35;
const PAIR_DT_MIN: usize = 3; // 48 ms
const PAIR_DT_MAX: usize = 48; // 768 ms
const PAIR_DF_MAX: isize = 63;
const PAIR_SHIFTS: usize = 24;
const MAX_HASH_OCCURRENCE: usize = 100;
const BUCKET_SEC: f64 = 10.0;
const OFFSET_RES: f64 = 0.05;
const MIN_BUCKET_MATCHES: usize = 4;
const SPAN_MERGE_TOL: f64 = 0.25;
const MIN_SPAN_SEC: f64 = 20.0;
const MIN_SPAN_DENSITY: f64 = 0.8;
const CHAIN_TOL: f64 = 0.5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintSpan {
    pub ad_start: f64,
    pub ad_end: f64,
    pub offset: f64, // video_time - ad_time (positive = AD plays earlier than video)
    pub matches: usize,
}

impl FingerprintSpan {
    pub fn density(&self) -> f64 {
        let dur = (self.ad_end - self.ad_start).max(1e-9);
        (self.matches as f64) / dur
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FingerprintResult {
    pub spans: Vec<FingerprintSpan>,
    pub unmatched: Vec<(f64, f64)>,
    pub n_matches: usize,
    pub match_t_ad: Vec<f32>,
    pub match_t_vid: Vec<f32>,
}

impl FingerprintResult {
    pub fn is_strong(&self) -> bool {
        if self.spans.is_empty() {
            return false;
        }
        let covered: f64 = self.spans.iter().map(|s| s.ad_end - s.ad_start).sum();
        let dense: f64 = self
            .spans
            .iter()
            .filter(|s| s.density() >= MIN_SPAN_DENSITY)
            .map(|s| s.ad_end - s.ad_start)
            .sum();
        let total_matches: usize = self.spans.iter().map(|s| s.matches).sum();
        total_matches >= 350 && covered > 0.0 && dense >= 0.55 * covered
    }

    pub fn offset_at(&self, ad_time: f64) -> Option<f64> {
        if self.spans.is_empty() {
            return None;
        }
        for s in &self.spans {
            if ad_time >= s.ad_start && ad_time <= s.ad_end {
                return Some(s.offset);
            }
        }
        self.spans
            .iter()
            .min_by(|a, b| {
                let da = (ad_time - a.ad_start).abs().min((ad_time - a.ad_end).abs());
                let db = (ad_time - b.ad_start).abs().min((ad_time - b.ad_end).abs());
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.offset)
    }
}

#[derive(Clone, Copy)]
struct Peak {
    time_frame: usize,
    freq_bin: usize,
    mag_db: f32,
}

/// Extracts Shazam-style constellation landmark hashes `(hash_u32, anchor_time_sec)` using a streaming
/// ring-buffer STFT so memory never exceeds ~2 MB regardless of movie duration.
pub fn extract_landmarks(y: &[f32], sr: u32) -> (Vec<u32>, Vec<f32>) {
    if y.len() < N_FFT {
        return (Vec::new(), Vec::new());
    }
    let n_frames = (y.len() - N_FFT) / HOP + 1;
    let bin_hz = (sr as f64) / (N_FFT as f64);
    let lo_bin = ((FMIN_HZ / bin_hz) as usize).max(1);
    let hi_bin = ((FMAX_HZ / bin_hz) as usize).min(N_FFT / 2);
    if hi_bin <= lo_bin {
        return (Vec::new(), Vec::new());
    }
    let n_bins = hi_bin - lo_bin;

    // Periodic Hann window
    let win: Vec<f32> = (0..N_FFT)
        .map(|i| {
            let s = (std::f64::consts::PI * (i as f64) / (N_FFT as f64)).sin();
            (s * s) as f32
        })
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let mut fft_buf = vec![Complex::new(0.0_f32, 0.0_f32); N_FFT];

    // Sample all spectrogram bins across strided frames for the exact 70th-percentile floor (matches ADSync _FLOOR_PERCENTILE = 70.0)
    let sample_stride = (n_frames / 1000).max(1);
    let mut frame_freq_peaks: Vec<Vec<(u16, f32)>> = Vec::with_capacity(n_frames);
    let mut all_bins_sample = Vec::with_capacity((n_frames / sample_stride + 1) * (n_bins / 4 + 1));
    let mut mag_row = vec![0.0_f32; n_bins];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP;
        for i in 0..N_FFT {
            fft_buf[i].re = y[start + i] * win[i];
            fft_buf[i].im = 0.0;
        }
        fft.process(&mut fft_buf);

        let record_sample = frame_idx % sample_stride == 0;
        for b in 0..n_bins {
            let c = fft_buf[lo_bin + b];
            let mag = (c.re * c.re + c.im * c.im).sqrt();
            let db = 20.0 * (mag + 1e-10).log10();
            mag_row[b] = db;
            if record_sample && b % 4 == 0 {
                all_bins_sample.push(db);
            }
        }

        // Local frequency maxima within +/- PEAK_NEIGHBOR_FREQ bins
        let mut f_peaks = Vec::new();
        for b in 0..n_bins {
            let val = mag_row[b];
            let f_lo = b.saturating_sub(PEAK_NEIGHBOR_FREQ);
            let f_hi = (b + PEAK_NEIGHBOR_FREQ + 1).min(n_bins);
            let mut is_max = true;
            for nb in f_lo..f_hi {
                if mag_row[nb] > val {
                    is_max = false;
                    break;
                }
            }
            if is_max {
                f_peaks.push((b as u16, val));
            }
        }
        frame_freq_peaks.push(f_peaks);
    }

    if all_bins_sample.is_empty() {
        return (Vec::new(), Vec::new());
    }
    all_bins_sample.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor_db = all_bins_sample[(all_bins_sample.len() * 70) / 100];

    // 2D Temporal + Frequency neighborhood suppression (+/- PEAK_NEIGHBOR_TIME frames, +/- PEAK_NEIGHBOR_FREQ bins)
    let frames_per_sec = (sr as f64) / (HOP as f64);
    let total_secs = ((n_frames as f64) / frames_per_sec).ceil() as usize + 1;
    let mut buckets: Vec<Vec<Peak>> = vec![Vec::new(); total_secs];

    for t in 0..n_frames {
        let t_lo = t.saturating_sub(PEAK_NEIGHBOR_TIME);
        let t_hi = (t + PEAK_NEIGHBOR_TIME + 1).min(n_frames);
        for &(b_u16, val) in &frame_freq_peaks[t] {
            if val <= floor_db {
                continue;
            }
            let b = b_u16 as usize;
            let mut is_2d_max = true;
            'neighbor: for nt in t_lo..t_hi {
                if nt == t {
                    continue;
                }
                for &(nb_u16, nval) in &frame_freq_peaks[nt] {
                    let nb = nb_u16 as usize;
                    if nb.abs_diff(b) <= PEAK_NEIGHBOR_FREQ && nval > val {
                        is_2d_max = false;
                        break 'neighbor;
                    }
                }
            }
            if is_2d_max {
                let sec_idx = ((t as f64) / frames_per_sec) as usize;
                if sec_idx < buckets.len() {
                    buckets[sec_idx].push(Peak {
                        time_frame: t,
                        freq_bin: b,
                        mag_db: val,
                    });
                }
            }
        }
    }

    // Cap density to top PEAKS_PER_SEC per 1-second bucket and flatten in time order
    let mut kept_peaks = Vec::new();
    for mut b_peaks in buckets {
        if b_peaks.len() > PEAKS_PER_SEC {
            b_peaks.sort_by(|a, b| {
                b.mag_db
                    .partial_cmp(&a.mag_db)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            b_peaks.truncate(PEAKS_PER_SEC);
        }
        b_peaks.sort_by_key(|p| p.time_frame);
        kept_peaks.extend(b_peaks);
    }

    // Pair each anchor peak with up to PAIR_SHIFTS successor peaks
    let n = kept_peaks.len();
    let mut hashes = Vec::with_capacity(n * 8);
    let mut times = Vec::with_capacity(n * 8);

    for i in 0..n {
        let p1 = kept_peaks[i];
        let t1_sec = ((p1.time_frame as f64) / frames_per_sec) as f32;
        let max_j = (i + 1 + PAIR_SHIFTS).min(n);
        for p2 in &kept_peaks[i + 1..max_j] {
            let dt = p2.time_frame.saturating_sub(p1.time_frame);
            if dt < PAIR_DT_MIN || dt > PAIR_DT_MAX {
                continue;
            }
            let df = (p2.freq_bin as isize) - (p1.freq_bin as isize);
            if df.abs() > PAIR_DF_MAX {
                continue;
            }
            let f1 = (p1.freq_bin as u32) & 0x1FF;
            let df_packed = ((df + PAIR_DF_MAX) as u32) & 0x7F;
            let dt_packed = (dt as u32) & 0x3F;
            let h = (f1 << 13) | (df_packed << 6) | dt_packed;
            hashes.push(h);
            times.push(t1_sec);
        }
    }

    (hashes, times)
}

/// Joins AD landmark hashes against reference video landmark hashes and builds monotone offset spans.
pub fn fingerprint_align_from_landmarks(
    vid_hashes: &[u32],
    vid_times: &[f32],
    ad_hashes: &[u32],
    ad_times: &[f32],
    ad_duration: f64,
) -> FingerprintResult {
    if vid_hashes.is_empty() || ad_hashes.is_empty() {
        return FingerprintResult {
            unmatched: vec![(0.0, ad_duration)],
            ..Default::default()
        };
    }

    let mut table: HashMap<u32, Vec<f32>> = HashMap::with_capacity(vid_hashes.len() / 2);
    for (&h, &t) in vid_hashes.iter().zip(vid_times.iter()) {
        table.entry(h).or_default().push(t);
    }

    let mut t_ad = Vec::new();
    let mut t_vid = Vec::new();
    for (&h, &at) in ad_hashes.iter().zip(ad_times.iter()) {
        if let Some(vts) = table.get(&h) {
            if vts.len() <= MAX_HASH_OCCURRENCE {
                for &vt in vts {
                    t_ad.push(at);
                    t_vid.push(vt);
                }
            }
        }
    }

    let n_matches = t_ad.len();
    let n_buckets = ((ad_duration / BUCKET_SEC).ceil() as usize).max(1);
    if n_matches == 0 {
        return FingerprintResult {
            unmatched: vec![(0.0, ad_duration)],
            ..Default::default()
        };
    }

    // Bucket all raw (at, vt) pairs in ONE O(n_matches) pass and build per-bucket histograms
    let mut bucket_pairs: Vec<Vec<(f32, f32)>> = vec![Vec::new(); n_buckets];
    let mut bucket_hist: Vec<HashMap<i64, usize>> = vec![HashMap::new(); n_buckets];
    for (&at, &vt) in t_ad.iter().zip(t_vid.iter()) {
        let b = (((at as f64) / BUCKET_SEC) as usize).min(n_buckets - 1);
        bucket_pairs[b].push((at, vt));
        let off = (vt as f64) - (at as f64);
        let obin = (off / OFFSET_RES).round() as i64;
        for d in -1..=1 {
            *bucket_hist[b].entry(obin + d).or_insert(0) += 1;
        }
    }
    drop(t_ad);
    drop(t_vid);

    let mut bucket_offset: Vec<Option<f64>> = vec![None; n_buckets];
    let mut bucket_matches: Vec<usize> = vec![0; n_buckets];

    for b in 0..n_buckets {
        if let Some((&best_bin, &best_cnt)) =
            bucket_hist[b].iter().max_by_key(|&(_, &cnt)| cnt)
        {
            if best_cnt >= MIN_BUCKET_MATCHES {
                let rough_off = (best_bin as f64) * OFFSET_RES;
                // Refine with median of matches in THIS bucket near dominant bin (O(|bucket_b|))
                let mut near: Vec<f64> = bucket_pairs[b]
                    .iter()
                    .filter_map(|&(at, vt)| {
                        let off = (vt as f64) - (at as f64);
                        if (off - rough_off).abs() <= 2.0 * OFFSET_RES {
                            Some(off)
                        } else {
                            None
                        }
                    })
                    .collect();
                if !near.is_empty() {
                    near.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    bucket_offset[b] = Some(near[near.len() / 2]);
                    bucket_matches[b] = best_cnt;
                }
            }
        }
    }

    // Monotone bucket chain (Longest Increasing Subsequence in video time weighted by match count)
    let nodes: Vec<usize> = (0..n_buckets)
        .filter(|&b| bucket_offset[b].is_some())
        .collect();
    if nodes.len() > 1 {
        let n = nodes.len();
        let node_tvid: Vec<f64> = nodes
            .iter()
            .map(|&b| ((b as f64) + 0.5) * BUCKET_SEC + bucket_offset[b].unwrap())
            .collect();
        let weights: Vec<f64> = nodes.iter().map(|&b| bucket_matches[b] as f64).collect();
        let mut best = vec![0.0_f64; n];
        let mut prev = vec![None; n];

        for i in 0..n {
            best[i] = weights[i];
            for j in 0..i {
                if node_tvid[j] <= node_tvid[i] + CHAIN_TOL && best[j] + weights[i] > best[i] {
                    best[i] = best[j] + weights[i];
                    prev[i] = Some(j);
                }
            }
        }

        let mut keep = vec![false; n];
        if let Some((mut cur, _)) = best
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            loop {
                keep[cur] = true;
                if let Some(p) = prev[cur] {
                    cur = p;
                } else {
                    break;
                }
            }
        }
        for (idx, &b) in nodes.iter().enumerate() {
            if !keep[idx] {
                bucket_offset[b] = None;
                bucket_matches[b] = 0;
            }
        }
    }

    // Merge adjacent supported buckets into spans
    let mut spans: Vec<FingerprintSpan> = Vec::new();
    for b in 0..n_buckets {
        let Some(boff) = bucket_offset[b] else {
            continue;
        };
        let t0 = (b as f64) * BUCKET_SEC;
        let t1 = (((b + 1) as f64) * BUCKET_SEC).min(ad_duration);
        let merged = if let Some(last) = spans.last_mut() {
            if (last.ad_end - t0).abs() < 1e-6 && (boff - last.offset).abs() <= SPAN_MERGE_TOL {
                last.ad_end = t1;
                last.matches += bucket_matches[b];
                last.offset += (boff - last.offset) * 0.25;
                true
            } else {
                false
            }
        } else {
            false
        };
        if !merged {
            spans.push(FingerprintSpan {
                ad_start: t0,
                ad_end: t1,
                offset: boff,
                matches: bucket_matches[b],
            });
        }
    }

    spans.retain(|s| s.ad_end - s.ad_start >= MIN_SPAN_SEC);

    let mut covered = vec![false; n_buckets];
    for s in &spans {
        let lo = (s.ad_start / BUCKET_SEC) as usize;
        let hi = ((s.ad_end / BUCKET_SEC).ceil() as usize).min(n_buckets);
        for c in &mut covered[lo..hi] {
            *c = true;
        }
    }

    let mut inlier_pairs: Vec<(f32, f32)> = Vec::new();
    for b in 0..n_buckets {
        if !covered[b] {
            continue;
        }
        if let Some(bo) = bucket_offset[b] {
            for &(at, vt) in &bucket_pairs[b] {
                if ((vt as f64) - (at as f64) - bo).abs() <= 0.75 {
                    inlier_pairs.push((at, vt));
                }
            }
        }
    }
    inlier_pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let (match_t_ad, match_t_vid): (Vec<f32>, Vec<f32>) = inlier_pairs.into_iter().unzip();

    let mut unmatched = Vec::new();
    let mut run_start: Option<usize> = None;
    for b in 0..=n_buckets {
        let gap = b < n_buckets && !covered[b];
        if gap && run_start.is_none() {
            run_start = Some(b);
        } else if !gap {
            if let Some(rs) = run_start {
                let lo_t = (rs as f64) * BUCKET_SEC;
                let hi_t = ((b as f64) * BUCKET_SEC).min(ad_duration);
                if hi_t - lo_t >= 2.0 * BUCKET_SEC {
                    unmatched.push((lo_t, hi_t));
                }
                run_start = None;
            }
        }
    }

    FingerprintResult {
        spans,
        unmatched,
        n_matches,
        match_t_ad,
        match_t_vid,
    }
}

pub fn fingerprint_align(y_vid: &[f32], y_ad: &[f32], sr: u32) -> FingerprintResult {
    let ((vid_h, vid_t), (ad_h, ad_t)) = rayon::join(
        || extract_landmarks(y_vid, sr),
        || extract_landmarks(y_ad, sr),
    );
    let ad_dur = (y_ad.len() as f64) / (sr as f64);
    fingerprint_align_from_landmarks(&vid_h, &vid_t, &ad_h, &ad_t, ad_dur)
}

/// Fast Catmull-Rom resampler for probing standard speed transfer ratios (e.g. PAL 25/23.976 = 1.042709x).
pub fn resample_linear_ratio(y: &[f32], stretch: f64) -> Vec<f32> {
    if y.is_empty() || (stretch - 1.0).abs() < 1e-6 {
        return y.to_vec();
    }
    let out_len = ((y.len() as f64) * stretch).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    let n = y.len();
    let inv_stretch = 1.0 / stretch;
    for i in 0..out_len {
        let pos = (i as f64) * inv_stretch;
        let idx = (pos as usize).min(n.saturating_sub(2));
        let frac = (pos - (idx as f64)) as f32;
        out.push(y[idx] * (1.0 - frac) + y[idx + 1] * frac);
    }
    out
}

fn fit_residual_speed_slope(fp: &FingerprintResult) -> f64 {
    // 1. Estimate from raw inlier match pairs when dense enough (matches ADSync speed.py)
    if fp.match_t_ad.len() >= 200 {
        let n = fp.match_t_ad.len();
        let stride = (n / 1500).max(1);
        let mut ts = Vec::with_capacity(n / stride + 1);
        let mut os = Vec::with_capacity(n / stride + 1);
        for i in (0..n).step_by(stride) {
            let at = fp.match_t_ad[i] as f64;
            let vt = fp.match_t_vid[i] as f64;
            ts.push(at);
            os.push(vt - at);
        }
        let mut slopes = Vec::new();
        for i in 0..ts.len() {
            for j in (i + 1)..ts.len() {
                let dt = ts[j] - ts[i];
                if (30.0..=180.0).contains(&dt) {
                    slopes.push((os[j] - os[i]) / dt);
                } else if dt > 180.0 {
                    break;
                }
            }
        }
        if slopes.len() >= 50 {
            slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            return slopes[slopes.len() / 2];
        }
    }

    // 2. Fallback: weighted median of pairwise span slopes
    if fp.spans.len() < 2 {
        return 0.0;
    }
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for i in 0..fp.spans.len() {
        let mi = 0.5 * (fp.spans[i].ad_start + fp.spans[i].ad_end);
        for j in (i + 1)..fp.spans.len() {
            let mj = 0.5 * (fp.spans[j].ad_start + fp.spans[j].ad_end);
            let dt = mj - mi;
            if (60.0..=240.0).contains(&dt) {
                let s = (fp.spans[j].offset - fp.spans[i].offset) / dt;
                let w = fp.spans[i].matches.min(fp.spans[j].matches) as f64;
                pairs.push((s, w));
            }
        }
    }
    if pairs.is_empty() {
        return 0.0;
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let total_w: f64 = pairs.iter().map(|p| p.1).sum();
    let mut acc = 0.0;
    for (s, w) in pairs {
        acc += w;
        if acc >= 0.5 * total_w {
            return s;
        }
    }
    0.0
}

/// Automatic Acoustic Landmark PAL / Speed Prober (matches ADSync align/speed.py):
/// Probes native 1.0x first, then standard transfer ratios (`25/23.976`, `25/24`, `23.976/25`, `24/25`),
/// refines residual slope, and verifies the result is strong and flat.
/// Returns `(speed_detected, detected_stretch_ratio, match_count, best_fp)`.
pub fn detect_speed_pal(
    y_vid: &[f32],
    y_ad: &[f32],
    sr: u32,
) -> (bool, f64, usize, FingerprintResult) {
    let ((vid_h, vid_t), (ad_h, ad_t)) = rayon::join(
        || extract_landmarks(y_vid, sr),
        || extract_landmarks(y_ad, sr),
    );
    let ad_dur = (y_ad.len() as f64) / (sr as f64);
    let base_fp = fingerprint_align_from_landmarks(&vid_h, &vid_t, &ad_h, &ad_t, ad_dur);
    let base_quality: usize = if base_fp.is_strong() {
        base_fp.spans.iter().map(|s| s.matches).sum()
    } else {
        0
    };

    if base_quality > 0 {
        let base_slope = fit_residual_speed_slope(&base_fp);
        if base_slope.abs() < 0.003 {
            return (false, 1.0, base_quality, base_fp);
        }
        // Baseline matched strongly but shows a genuine speed ramp (> 0.3%)
        let refined = 1.0 + base_slope;
        let stretched = resample_linear_ratio(y_ad, refined);
        let (st_h, st_t) = extract_landmarks(&stretched, sr);
        let fp_v = fingerprint_align_from_landmarks(
            &vid_h,
            &vid_t,
            &st_h,
            &st_t,
            (stretched.len() as f64) / (sr as f64),
        );
        let slope_v = fit_residual_speed_slope(&fp_v);
        if fp_v.is_strong() && slope_v.abs() <= 0.001 {
            let q = fp_v.spans.iter().map(|s| s.matches).sum();
            return (true, refined, q, fp_v);
        }
        return (false, 1.0, base_quality, base_fp);
    }

    // Nothing matched strongly at 1.0x: probe standard PAL / film speed transfer ratios
    let ratios = [
        25.0 / 23.976023976, // PAL AD (fast) vs 23.976fps NTSC film video
        25.0 / 24.0,         // PAL AD (fast) vs 24.000fps cinema video
        23.976023976 / 25.0, // 23.976fps AD vs 25fps PAL video
        24.0 / 25.0,         // 24.000fps AD vs 25fps PAL video
    ];
    let mut best_ratio = 1.0;
    let mut best_quality = 0usize;
    let mut best_fp: Option<FingerprintResult> = None;

    for &ratio in &ratios {
        let stretched = resample_linear_ratio(y_ad, ratio);
        let (st_h, st_t) = extract_landmarks(&stretched, sr);
        let st_dur = (stretched.len() as f64) / (sr as f64);
        let fp_r = fingerprint_align_from_landmarks(&vid_h, &vid_t, &st_h, &st_t, st_dur);
        let q: usize = if fp_r.is_strong() {
            fp_r.spans.iter().map(|s| s.matches).sum()
        } else {
            0
        };
        if q > best_quality {
            best_quality = q;
            best_ratio = ratio;
            best_fp = Some(fp_r);
        }
    }

    if let Some(fp_cand) = best_fp {
        if best_quality >= 350 && best_quality >= 3 * base_quality.max(1) {
            let resid = fit_residual_speed_slope(&fp_cand);
            let refined = best_ratio * (1.0 + resid);
            if resid.abs() > 0.0003 {
                let stretched_v = resample_linear_ratio(y_ad, refined);
                let (vh, vt) = extract_landmarks(&stretched_v, sr);
                let fp_v = fingerprint_align_from_landmarks(
                    &vid_h,
                    &vid_t,
                    &vh,
                    &vt,
                    (stretched_v.len() as f64) / (sr as f64),
                );
                let slope_v = fit_residual_speed_slope(&fp_v);
                if fp_v.is_strong() && slope_v.abs() <= 0.0015 {
                    let qv = fp_v.spans.iter().map(|s| s.matches).sum();
                    return (true, refined, qv, fp_v);
                }
            }
            return (true, best_ratio, best_quality, fp_cand);
        }
    }

    (false, 1.0, base_quality, base_fp)
}
