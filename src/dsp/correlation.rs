use crate::dsp::filters::{filter_multiband_into, BAND_WEIGHTS};
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Smallest even 5-smooth length (`2^a * 3^b * 5^c`, `a >= 1`) >= `min_len`.
/// Matches SciPy's `next_fast_len` behavior so FFT sizes stay tightly bounded instead of jumping 2x.
pub fn next_fast_even_len(min_len: usize) -> usize {
    if min_len <= 2 {
        return 2;
    }
    let mut best = min_len.next_power_of_two();
    let mut p5 = 1usize;
    while p5 < best {
        let mut p35 = p5;
        while p35 < best {
            let mut n = match p35.checked_mul(2) {
                Some(v) => v,
                None => break,
            };
            while n < min_len {
                n = match n.checked_mul(2) {
                    Some(v) => v,
                    None => break,
                };
            }
            if n >= min_len && n < best {
                best = n;
            }
            p35 = match p35.checked_mul(3) {
                Some(v) => v,
                None => break,
            };
        }
        p5 = match p5.checked_mul(5) {
            Some(v) => v,
            None => break,
        };
    }
    best
}

struct CorrFftWorkspace {
    planner: FftPlanner<f32>,
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    cs: Vec<f64>,
}

impl CorrFftWorkspace {
    fn new() -> Self {
        Self {
            planner: FftPlanner::<f32>::new(),
            buf: Vec::new(),
            scratch: Vec::new(),
            cs: Vec::new(),
        }
    }

    fn run_normalized(&mut self, v_sig: &[f32], a_sig: &[f32]) -> Option<Vec<f64>> {
        let n = v_sig.len();
        let m = a_sig.len();
        if n < m || m == 0 {
            return None;
        }

        // Center template `a_sig` and compute its L2 norm in f64
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

        let v_mean: f64 = v_sig.iter().map(|&x| x as f64).sum::<f64>() / (n as f64);
        let fft_len = next_fast_even_len(n + m - 1);

        let fft_fwd = self.planner.plan_fft_forward(fft_len);
        let fft_inv = self.planner.plan_fft_inverse(fft_len);
        let req_scratch = fft_fwd
            .get_inplace_scratch_len()
            .max(fft_inv.get_inplace_scratch_len());

        if self.buf.len() < fft_len {
            self.buf.resize(fft_len, Complex::new(0.0_f32, 0.0_f32));
        }
        if self.scratch.len() < req_scratch {
            self.scratch
                .resize(req_scratch, Complex::new(0.0_f32, 0.0_f32));
        }

        // Pack both real signals into one complex buffer: re = v_centered, im = a_rev_centered.
        // This cuts 3 FFTs down to 2 FFTs and halves memory bandwidth.
        let buf = &mut self.buf[..fft_len];
        for slot in buf.iter_mut() {
            *slot = Complex::new(0.0_f32, 0.0_f32);
        }

        self.cs.clear();
        self.cs.reserve(n + 1);
        self.cs.push(0.0_f64);
        let mut running_sq = 0.0_f64;
        for (i, &x) in v_sig.iter().enumerate() {
            let c = (x as f64) - v_mean;
            buf[i].re = c as f32;
            running_sq += c * c;
            self.cs.push(running_sq);
        }

        for (i, &x) in a_sig.iter().rev().enumerate() {
            buf[i].im = ((x as f64) - a_mean) as f32;
        }

        fft_fwd.process_with_scratch(buf, &mut self.scratch[..req_scratch]);

        // Unpack V[k] and A[k] via Hermitian symmetry and multiply V[k] * A[k] in place
        let half = fft_len / 2;
        buf[0] = Complex::new(buf[0].re * buf[0].im, 0.0_f32);
        buf[half] = Complex::new(buf[half].re * buf[half].im, 0.0_f32);
        for k in 1..half {
            let j = fft_len - k;
            let z1 = buf[k];
            let z2 = buf[j];
            let v_re = 0.5_f32 * (z1.re + z2.re);
            let v_im = 0.5_f32 * (z1.im - z2.im);
            let a_re = 0.5_f32 * (z1.im + z2.im);
            let a_im = 0.5_f32 * (z2.re - z1.re);
            let p_re = v_re * a_re - v_im * a_im;
            let p_im = v_re * a_im + v_im * a_re;
            buf[k] = Complex::new(p_re, p_im);
            buf[j] = Complex::new(p_re, -p_im);
        }

        fft_inv.process_with_scratch(buf, &mut self.scratch[..req_scratch]);

        let out_len = n - m + 1;
        let inv_fft = 1.0 / (fft_len as f64);
        let mut out = Vec::with_capacity(out_len);
        // Guard against prefix-sum cancellation in near-silent passages (matches ADSync compute.py)
        let energy_tol = (4.0 * f64::EPSILON * (n as f64) * running_sq).max(1e-20);

        for k in 0..out_len {
            let raw_dot = (buf[k + m - 1].re as f64) * inv_fft;
            let win_energy_sq = self.cs[k + m] - self.cs[k];
            if win_energy_sq > energy_tol {
                let denom = a_energy * win_energy_sq.sqrt();
                out.push((raw_dot / denom).clamp(-1.0, 1.0));
            } else {
                out.push(0.0);
            }
        }

        Some(out)
    }
}

thread_local! {
    static CORR_WORKSPACE: RefCell<CorrFftWorkspace> = RefCell::new(CorrFftWorkspace::new());
}

/// Computes normalized sliding cross-correlation (`mode="valid"`) between `v_sig` (length N)
/// and `a_sig` (length M, N >= M) using cached 5-smooth f32 SIMD FFT convolution with
/// two-real-in-one packing and O(N) f64 prefix-sum energy normalization.
pub fn normalized_sliding_correlation(v_sig: &[f32], a_sig: &[f32]) -> Option<Vec<f64>> {
    CORR_WORKSPACE.with(|ws| ws.borrow_mut().run_normalized(v_sig, a_sig))
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
#[allow(clippy::too_many_arguments)]
pub fn evaluate_anchor_correlations<F>(
    base_samples: &[f32],
    ad_samples: &[f32],
    sr: u32,
    target_duration: f64,
    num_anchors: usize,
    multiband: bool,
    cancel_flag: &AtomicBool,
    on_anchor: F,
) -> Vec<AnchorMeasurement>
where
    F: Fn(usize, usize, &[AnchorMeasurement]) + Sync,
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
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        let batch_meas: Vec<AnchorMeasurement> = chunk
            .par_iter()
            .filter_map(|&anchor| {
                if cancel_flag.load(Ordering::Relaxed) {
                    return None;
                }
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
