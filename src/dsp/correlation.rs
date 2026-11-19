use crate::dsp::filters::{filter_multiband_into, BAND_WEIGHTS};
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AnchorMeasurement {
    pub anchor_sec: f64,
    pub offset_sec: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OffsetCluster {
    pub median_offset: f64,
    pub max_peak: f64,
    pub count: usize,
}

/// Computes normalized sliding cross-correlation (`mode="valid"`) between `v_sig` (length N)
/// and `a_sig` (length M, N >= M) using FFT convolution and O(N) prefix-sum energy normalization.
pub fn normalized_sliding_correlation(v_sig: &[f32], a_sig: &[f32]) -> Option<Vec<f64>> {
    let n = v_sig.len();
    let m = a_sig.len();
    if n < m || m == 0 {
        return None;
    }

    // Center template `a_sig` and compute its L2 norm
    let a_mean: f64 = a_sig.iter().map(|&x| x as f64).sum::<f64>() / (m as f64);
    let mut a_energy_sq = 0.0_f64;
    for &x in a_sig {
        let c = (x as f64) - a_mean;
        a_energy_sq += c * c;
    }
    let a_energy = a_energy_sq.sqrt();
    if a_energy < 1e-10 {
        return None;
    }

    // Center search region `v_sig`
    let v_mean: f64 = v_sig.iter().map(|&x| x as f64).sum::<f64>() / (n as f64);

    // Choose power-of-two FFT length >= n + m - 1
    let fft_len = (n + m - 1).next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let fft_fwd = planner.plan_fft_forward(fft_len);
    let fft_inv = planner.plan_fft_inverse(fft_len);

    let mut v_buf = vec![Complex::new(0.0_f64, 0.0_f64); fft_len];
    let mut a_buf = vec![Complex::new(0.0_f64, 0.0_f64); fft_len];

    // Also build prefix sum of squared centered v_sig for O(1) window energy lookup
    let mut cs = Vec::with_capacity(n + 1);
    cs.push(0.0_f64);
    let mut running_sq = 0.0_f64;
    for (i, &x) in v_sig.iter().enumerate() {
        let c = (x as f64) - v_mean;
        v_buf[i].re = c;
        running_sq += c * c;
        cs.push(running_sq);
    }

    // Reverse template a_sig into a_buf[0..m] for cross-correlation via convolution
    for (i, &x) in a_sig.iter().rev().enumerate() {
        a_buf[i].re = (x as f64) - a_mean;
    }

    fft_fwd.process(&mut v_buf);
    fft_fwd.process(&mut a_buf);

    for (vc, ac) in v_buf.iter_mut().zip(a_buf.iter()) {
        *vc *= *ac;
    }

    fft_inv.process(&mut v_buf);

    let out_len = n - m + 1;
    let inv_fft = 1.0 / (fft_len as f64);
    let mut out = Vec::with_capacity(out_len);

    for k in 0..out_len {
        let raw_dot = v_buf[k + m - 1].re * inv_fft;
        let win_energy_sq = (cs[k + m] - cs[k]).max(1e-20);
        let denom = a_energy * win_energy_sq.sqrt();
        out.push(raw_dot / denom);
    }

    Some(out)
}

/// Computes 3-band narration-suppressed normalized sliding cross-correlation.
pub fn multiband_normalized_correlation(
    v_region: &[f32],
    a_segment: &[f32],
    sr: u32,
    multiband: bool,
) -> Option<Vec<f64>> {
    if v_region.len() < a_segment.len() || a_segment.is_empty() {
        return None;
    }
    if !multiband {
        return normalized_sliding_correlation(v_region, a_segment);
    }

    let mut v_low = Vec::new();
    let mut v_mid = Vec::new();
    let mut v_high = Vec::new();
    filter_multiband_into(v_region, sr, &mut v_low, &mut v_mid, &mut v_high);

    let mut a_low = Vec::new();
    let mut a_mid = Vec::new();
    let mut a_high = Vec::new();
    filter_multiband_into(a_segment, sr, &mut a_low, &mut a_mid, &mut a_high);

    let bands = [
        (&v_low[..], &a_low[..], BAND_WEIGHTS[0]),
        (&v_mid[..], &a_mid[..], BAND_WEIGHTS[1]),
        (&v_high[..], &a_high[..], BAND_WEIGHTS[2]),
    ];

    let mut combined: Option<Vec<f64>> = None;
    let mut weight_sum = 0.0_f64;

    for (vs, asig, w) in bands {
        if let Some(corr) = normalized_sliding_correlation(vs, asig) {
            match &mut combined {
                None => {
                    combined = Some(corr.into_iter().map(|c| c * w).collect());
                }
                Some(acc) => {
                    for (a, c) in acc.iter_mut().zip(corr.into_iter()) {
                        *a += c * w;
                    }
                }
            }
            weight_sum += w;
        }
    }

    if let Some(ref mut acc) = combined {
        if weight_sum > 0.0 {
            let inv = 1.0 / weight_sum;
            for v in acc.iter_mut() {
                *v *= inv;
            }
        }
    }
    combined
}

/// Sub-sample parabolic peak interpolation around `best_idx` for sub-millisecond offset accuracy.
#[inline]
pub fn parabolic_peak_refine(corr: &[f64], best_idx: usize) -> (f64, f64) {
    let y_peak = corr[best_idx];
    if best_idx > 0 && best_idx + 1 < corr.len() {
        let y_prev = corr[best_idx - 1];
        let y_next = corr[best_idx + 1];
        let denom = y_prev - 2.0 * y_peak + y_next;
        if denom.abs() > 1e-12 {
            let sub = (0.5 * (y_prev - y_next) / denom).clamp(-0.5, 0.5);
            let score = y_peak - 0.25 * (y_prev - y_next) * sub;
            return ((best_idx as f64) + sub, score);
        }
    }
    (best_idx as f64, y_peak)
}

/// Clusters anchor measurements to find consensus winner and runner-up candidate offsets.
pub fn find_offset_candidates(
    measurements: &[AnchorMeasurement],
    min_conf: f64,
    cluster_tol: f64,
) -> Vec<OffsetCluster> {
    let mut valid: Vec<AnchorMeasurement> = measurements
        .iter()
        .copied()
        .filter(|m| m.confidence >= min_conf)
        .collect();
    if valid.is_empty() {
        valid = measurements.to_vec();
    }
    if valid.is_empty() {
        return vec![OffsetCluster {
            median_offset: 0.0,
            max_peak: 0.0,
            count: 0,
        }];
    }

    valid.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    struct RawCluster {
        seed_offset: f64,
        max_peak: f64,
        offsets: Vec<f64>,
    }

    let mut clusters: Vec<RawCluster> = Vec::new();
    for m in valid {
        let mut matched = false;
        for cl in &mut clusters {
            if (cl.seed_offset - m.offset_sec).abs() <= cluster_tol {
                cl.offsets.push(m.offset_sec);
                if m.confidence > cl.max_peak {
                    cl.max_peak = m.confidence;
                }
                matched = true;
                break;
            }
        }
        if !matched {
            clusters.push(RawCluster {
                seed_offset: m.offset_sec,
                max_peak: m.confidence,
                offsets: vec![m.offset_sec],
            });
        }
    }

    let mut out: Vec<OffsetCluster> = clusters
        .into_iter()
        .map(|mut cl| {
            cl.offsets
                .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = cl.offsets.len() / 2;
            let median = if cl.offsets.len() % 2 == 1 {
                cl.offsets[mid]
            } else {
                0.5 * (cl.offsets[mid - 1] + cl.offsets[mid])
            };
            OffsetCluster {
                median_offset: median,
                max_peak: cl.max_peak,
                count: cl.offsets.len(),
            }
        })
        .collect();

    out.sort_by(|a, b| {
        b.count.cmp(&a.count).then_with(|| {
            b.max_peak
                .partial_cmp(&a.max_peak)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    out
}

/// Evaluates `num_anchors` across 5%..88% of `target_duration` in parallel using `rayon`.
pub fn evaluate_anchor_correlations<F>(
    base_samples: &[f32],
    ad_samples: &[f32],
    sr: u32,
    target_duration: f64,
    num_anchors: usize,
    multiband: bool,
    on_anchor: F,
) -> Vec<AnchorMeasurement>
where
    F: Fn(usize, usize, & [AnchorMeasurement]) + Sync,
{
    let n_anchors = num_anchors.max(1);
    let t_start = target_duration * 0.05;
    let t_end = target_duration * 0.88;
    let step = if n_anchors > 1 {
        (t_end - t_start) / ((n_anchors - 1) as f64)
    } else {
        0.0
    };

    let mut anchor_times = Vec::with_capacity(n_anchors);
    for i in 0..n_anchors {
        let a = t_start + (i as f64) * step;
        let search_start_s = (a - 30.0).max(0.0);
        if a + 30.0 <= target_duration
            && ((search_start_s * (sr as f64)) as usize) < ad_samples.len()
            && ((a * (sr as f64)) as usize) < base_samples.len()
        {
            anchor_times.push(a);
        }
    }

    let total_valid = anchor_times.len();
    let sr_f = sr as f64;

    // Parallel evaluation in batches of 8 anchors so UI/CLI progress updates stay live and responsive
    let mut results = Vec::with_capacity(total_valid);
    let batch_size = 8;
    for chunk in anchor_times.chunks(batch_size) {
        let batch_meas: Vec<AnchorMeasurement> = chunk
            .par_iter()
            .filter_map(|&anchor| {
                let base_start = (anchor * sr_f) as usize;
                let base_end = (((anchor + 30.0) * sr_f) as usize).min(base_samples.len());
                if base_start >= base_end {
                    return None;
                }
                let base_slice = &base_samples[base_start..base_end];

                let search_start_s = (anchor - 30.0).max(0.0);
                let search_end_s = anchor + 60.0;
                let ad_start = (search_start_s * sr_f) as usize;
                let ad_end = ((search_end_s * sr_f) as usize).min(ad_samples.len());
                if ad_start >= ad_end || (ad_end - ad_start) < base_slice.len() {
                    return None;
                }
                let ad_slice = &ad_samples[ad_start..ad_end];

                let corr = multiband_normalized_correlation(ad_slice, base_slice, sr, multiband)?;
                if corr.is_empty() {
                    return None;
                }
                let mut best_idx = 0usize;
                let mut best_val = f64::NEG_INFINITY;
                for (idx, &v) in corr.iter().enumerate() {
                    if v > best_val {
                        best_val = v;
                        best_idx = idx;
                    }
                }
                let (refined_idx, refined_peak) = parabolic_peak_refine(&corr, best_idx);
                // Note: offset convention matches sync_ad_track.py:
                // offset = search_start_s + (best_idx / sr) - anchor
                // Positive offset -> AD track has extra intro (needs atrim=start=offset)
                // Negative offset -> AD track starts later in movie (needs adelay=|offset|)
                let offset_sec = search_start_s + (refined_idx / sr_f) - anchor;
                Some(AnchorMeasurement {
                    anchor_sec: anchor,
                    offset_sec,
                    confidence: refined_peak,
                })
            })
            .collect();

        results.extend(batch_meas);
        on_anchor(results.len(), total_valid, &results);
    }

    results
}
