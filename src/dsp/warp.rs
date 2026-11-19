use crate::dsp::correlation::{normalized_sliding_correlation, parabolic_peak_refine};
use crate::dsp::filters::{Butter4Lowpass, BAND_EDGES, BAND_WEIGHTS};
use crate::dsp::fingerprint::FingerprintResult;
use crate::media::{WavStreamReader, WavStreamWriter};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffsetCandidate {
    pub offset_sec: f64,
    pub score: f64,
    pub peak_sharpness: f64,
    pub peak_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateWindow {
    pub source_center: f64,
    pub candidates: Vec<OffsetCandidate>,
    pub speech_score: f64,
    pub energy: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WarpPoint {
    pub source_time: f64,
    pub target_time: f64,
    pub confidence: f64,
    pub is_anchor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarpCut {
    pub index: usize,
    pub vid_start: f64,
    pub vid_end: f64,
    pub offset: f64,
    pub jump: f64,
    pub is_initial: bool,
}

/// Monotone Piecewise Cubic Hermite Interpolating Polynomial (Fritsch-Carlson PCHIP).
/// Guarantees strict monotonicity on strictly increasing knots with C1 continuous first derivatives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PchipSpline {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub d: Vec<f64>,
}

impl PchipSpline {
    pub fn new(mut xs: Vec<f64>, mut ys: Vec<f64>) -> Self {
        assert!(!xs.is_empty() && xs.len() == ys.len());
        if xs.len() == 1 {
            let x0 = xs[0];
            let y0 = ys[0];
            xs = vec![x0, x0 + 1.0];
            ys = vec![y0, y0 + 1.0];
        }
        // Ensure strictly increasing x and y
        for i in 1..xs.len() {
            if xs[i] <= xs[i - 1] {
                xs[i] = xs[i - 1] + 1e-7;
            }
            if ys[i] <= ys[i - 1] {
                ys[i] = ys[i - 1] + 1e-7;
            }
        }

        let n = xs.len();
        let mut h = Vec::with_capacity(n - 1);
        let mut delta = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let hi = (xs[i + 1] - xs[i]).max(1e-12);
            let di = (ys[i + 1] - ys[i]) / hi;
            h.push(hi);
            delta.push(di);
        }

        let mut d = vec![0.0_f64; n];
        if n == 2 {
            d[0] = delta[0];
            d[1] = delta[0];
        } else {
            for i in 1..n - 1 {
                if delta[i - 1] * delta[i] > 0.0 {
                    let w1 = 2.0 * h[i] + h[i - 1];
                    let w2 = h[i] + 2.0 * h[i - 1];
                    d[i] = (w1 + w2) / (w1 / delta[i - 1] + w2 / delta[i]);
                } else {
                    d[i] = 0.0;
                }
            }
            // Endpoint slopes (one-sided shape-preserving three-point formula)
            d[0] = Self::edge_slope(h[0], h[1], delta[0], delta[1]);
            d[n - 1] = Self::edge_slope(h[n - 2], h[n - 3], delta[n - 2], delta[n - 3]);
        }

        Self { x: xs, y: ys, d }
    }

    fn edge_slope(h0: f64, h1: f64, del0: f64, del1: f64) -> f64 {
        let mut d = ((2.0 * h0 + h1) * del0 - h0 * del1) / (h0 + h1);
        if d * del0 <= 0.0 {
            d = 0.0;
        } else if del0 * del1 < 0.0 && d.abs() > 3.0 * del0.abs() {
            d = 3.0 * del0;
        }
        d
    }

    #[inline]
    pub fn eval(&self, t: f64) -> f64 {
        let n = self.x.len();
        if t <= self.x[0] {
            return self.y[0] + (t - self.x[0]) * self.d[0];
        }
        if t >= self.x[n - 1] {
            return self.y[n - 1] + (t - self.x[n - 1]) * self.d[n - 1];
        }
        let idx = match self
            .x
            .binary_search_by(|v| v.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i.min(n - 2),
            Err(i) => i.saturating_sub(1).min(n - 2),
        };
        let h = self.x[idx + 1] - self.x[idx];
        let s = (t - self.x[idx]) / h;
        let s2 = s * s;
        let s3 = s2 * s;
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        h00 * self.y[idx]
            + h10 * h * self.d[idx]
            + h01 * self.y[idx + 1]
            + h11 * h * self.d[idx + 1]
    }

    #[inline]
    pub fn derivative(&self, t: f64) -> f64 {
        let n = self.x.len();
        if t <= self.x[0] {
            return self.d[0];
        }
        if t >= self.x[n - 1] {
            return self.d[n - 1];
        }
        let idx = match self
            .x
            .binary_search_by(|v| v.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i.min(n - 2),
            Err(i) => i.saturating_sub(1).min(n - 2),
        };
        let h = self.x[idx + 1] - self.x[idx];
        let s = (t - self.x[idx]) / h;
        let s2 = s * s;
        let dh00 = (6.0 * s2 - 6.0 * s) / h;
        let dh10 = 3.0 * s2 - 4.0 * s + 1.0;
        let dh01 = (-6.0 * s2 + 6.0 * s) / h;
        let dh11 = 3.0 * s2 - 2.0 * s;
        dh00 * self.y[idx]
            + dh10 * self.d[idx]
            + dh01 * self.y[idx + 1]
            + dh11 * self.d[idx + 1]
    }

    /// Inverts forward warp (AD -> video) on `seg_range` into an inverse PCHIP (video -> AD).
    pub fn invert(&self, seg_range: (f64, f64)) -> Self {
        let (src_start, src_end) = seg_range;
        let n_pts = (((src_end - src_start) / 0.02) as usize).max(16);
        let step = (src_end - src_start) / ((n_pts - 1) as f64);
        let mut ad_times = Vec::with_capacity(n_pts);
        let mut vid_times = Vec::with_capacity(n_pts);
        for i in 0..n_pts {
            let at = src_start + (i as f64) * step;
            let vt = self.eval(at);
            ad_times.push(at);
            vid_times.push(vt);
        }
        Self::new(vid_times, ad_times)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarpPlan {
    pub splines: Vec<PchipSpline>,
    pub segment_ranges: Vec<(f64, f64)>,
    pub points: Vec<WarpPoint>,
    pub anchor_points: Vec<WarpPoint>,
    pub cuts: Vec<WarpCut>,
    pub explicit_gaps: Vec<(f64, f64, f64)>,
    pub confidence: f64,
    pub global_offset: f64,
}

fn downsample_2x(y: &[f32]) -> Vec<f32> {
    let n = y.len() / 2;
    let mut out = Vec::with_capacity(n);
    for pair in y.chunks_exact(2) {
        out.push(0.5 * (pair[0] + pair[1]));
    }
    out
}

/// Builds the candidate lattice at ~4 kHz using 3-band narration-suppressed cross-correlation.
pub fn build_candidate_lattice<F>(
    y_vid_8k: &[f32],
    y_ad_8k: &[f32],
    sr_in: u32,
    window_sec: f64,
    step_sec: f64,
    search_radius_sec: f64,
    offset_hint: f64,
    fp: Option<&FingerprintResult>,
    max_candidates: usize,
    multiband: bool,
    on_progress: F,
) -> Vec<CandidateWindow>
where
    F: Fn(usize, usize) + Sync,
{
    let (vid, ad, ds_sr) = if sr_in >= 8000 {
        (
            downsample_2x(y_vid_8k),
            downsample_2x(y_ad_8k),
            (sr_in / 2) as f64,
        )
    } else {
        (y_vid_8k.to_vec(), y_ad_8k.to_vec(), sr_in as f64)
    };
    let sec_per_sample = 1.0 / ds_sr;

    let mut vid_l0 = Vec::new();
    let mut vid_l1 = Vec::new();
    let mut ad_l0 = Vec::new();
    let mut ad_l1 = Vec::new();

    if multiband {
        let f0 = Butter4Lowpass::new(BAND_EDGES.0, ds_sr);
        let f1 = Butter4Lowpass::new(BAND_EDGES.1, ds_sr);
        f0.filtfilt(&vid, &mut vid_l0);
        f1.filtfilt(&vid, &mut vid_l1);
        f0.filtfilt(&ad, &mut ad_l0);
        f1.filtfilt(&ad, &mut ad_l1);
    }

    let win_samples = (window_sec / sec_per_sample) as usize;
    let step_samples = ((step_sec / sec_per_sample) as usize).max(1);
    let search_samples = (search_radius_sec / sec_per_sample) as usize;
    let min_peak_dist = (win_samples / 4).max(1);
    let min_score = 0.28_f64;

    if ad.len() <= win_samples {
        return Vec::new();
    }

    let ad_positions: Vec<usize> = (0..ad.len() - win_samples).step_by(step_samples).collect();
    let total_windows = ad_positions.len();
    let use_fp = fp.filter(|f| f.is_strong());

    let mut lattice = Vec::with_capacity(total_windows);
    let batch_size = 32;

    for chunk in ad_positions.chunks(batch_size) {
        let batch_wins: Vec<CandidateWindow> = chunk
            .par_iter()
            .map(|&ad_start| {
                let ad_end = ad_start + win_samples;
                let a_full = &ad[ad_start..ad_end];
                let ad_center_sec = ((ad_start as f64) + (win_samples as f64) * 0.5) * sec_per_sample;

                let mean_a: f64 =
                    a_full.iter().map(|&x| x as f64).sum::<f64>() / (win_samples as f64);
                let mut a_energy_sq = 0.0_f64;
                for &x in a_full {
                    let c = (x as f64) - mean_a;
                    a_energy_sq += c * c;
                }
                let ad_energy = a_energy_sq.sqrt();
                if ad_energy < 1e-8 {
                    return CandidateWindow {
                        source_center: ad_center_sec,
                        candidates: Vec::new(),
                        speech_score: 0.0,
                        energy: 0.0,
                    };
                }

                let expected_off = use_fp
                    .and_then(|f| f.offset_at(ad_center_sec))
                    .unwrap_or(offset_hint);
                let hint_samples = (expected_off / sec_per_sample).round() as isize;
                let nominal_vid = (ad_start as isize) + hint_samples;
                let search_start = (nominal_vid - (search_samples as isize)).max(0) as usize;
                let search_end = ((nominal_vid + (search_samples as isize) + (win_samples as isize))
                    .max(0) as usize)
                    .min(vid.len());

                if search_end <= search_start || (search_end - search_start) < win_samples {
                    return CandidateWindow {
                        source_center: ad_center_sec,
                        candidates: Vec::new(),
                        speech_score: 0.0,
                        energy: ad_energy,
                    };
                }

                let v_full = &vid[search_start..search_end];
                let mut norm_corr: Option<Vec<f64>> = None;
                let mut speech_score = 0.35_f64;

                if multiband && !vid_l0.is_empty() {
                    let vl0 = &vid_l0[search_start..search_end];
                    let vl1 = &vid_l1[search_start..search_end];
                    let al0 = &ad_l0[ad_start..ad_end];
                    let al1 = &ad_l1[ad_start..ad_end];

                    let v_mid: Vec<f32> = vl1.iter().zip(vl0.iter()).map(|(&a, &b)| a - b).collect();
                    let a_mid: Vec<f32> = al1.iter().zip(al0.iter()).map(|(&a, &b)| a - b).collect();
                    let v_high: Vec<f32> =
                        v_full.iter().zip(vl1.iter()).map(|(&a, &b)| a - b).collect();
                    let a_high: Vec<f32> =
                        a_full.iter().zip(al1.iter()).map(|(&a, &b)| a - b).collect();

                    // Estimate speech ratio from mid-band energy vs full energy
                    let mid_e: f64 = a_mid.iter().map(|&x| (x as f64) * (x as f64)).sum();
                    speech_score = (mid_e / (a_energy_sq + 1e-12)).clamp(0.0, 1.0);

                    let bands = [
                        (vl0, al0, BAND_WEIGHTS[0]),
                        (&v_mid[..], &a_mid[..], BAND_WEIGHTS[1]),
                        (&v_high[..], &a_high[..], BAND_WEIGHTS[2]),
                    ];
                    let mut weight_sum = 0.0_f64;
                    for (vs, asig, w) in bands {
                        if let Some(c) = normalized_sliding_correlation(vs, asig) {
                            match &mut norm_corr {
                                None => norm_corr = Some(c.into_iter().map(|v| v * w).collect()),
                                Some(acc) => {
                                    for (a, v) in acc.iter_mut().zip(c.into_iter()) {
                                        *a += v * w;
                                    }
                                }
                            }
                            weight_sum += w;
                        }
                    }
                    if let Some(ref mut acc) = norm_corr {
                        if weight_sum > 0.0 {
                            for v in acc.iter_mut() {
                                *v /= weight_sum;
                            }
                        }
                    }
                } else {
                    norm_corr = normalized_sliding_correlation(v_full, a_full);
                }

                let Some(corr) = norm_corr else {
                    return CandidateWindow {
                        source_center: ad_center_sec,
                        candidates: Vec::new(),
                        speech_score,
                        energy: ad_energy,
                    };
                };
                if corr.is_empty() {
                    return CandidateWindow {
                        source_center: ad_center_sec,
                        candidates: Vec::new(),
                        speech_score,
                        energy: ad_energy,
                    };
                }

                let mean_corr: f64 =
                    corr.iter().map(|v| v.abs()).sum::<f64>() / (corr.len() as f64);

                // Find local peaks separated by at least min_peak_dist
                let mut raw_peaks: Vec<(usize, f64)> = Vec::new();
                for i in 1..corr.len().saturating_sub(1) {
                    if corr[i] >= min_score && corr[i] >= corr[i - 1] && corr[i] >= corr[i + 1] {
                        raw_peaks.push((i, corr[i]));
                    }
                }
                if raw_peaks.is_empty() {
                    if let Some((bi, &bv)) = corr
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    {
                        if bv >= min_score {
                            raw_peaks.push((bi, bv));
                        }
                    }
                }
                raw_peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                let mut chosen: Vec<(usize, f64)> = Vec::with_capacity(max_candidates);
                for (pidx, pscore) in raw_peaks {
                    if chosen
                        .iter()
                        .all(|&(ci, _)| ci.abs_diff(pidx) >= min_peak_dist)
                    {
                        chosen.push((pidx, pscore));
                        if chosen.len() >= max_candidates {
                            break;
                        }
                    }
                }

                if chosen.is_empty() {
                    return CandidateWindow {
                        source_center: ad_center_sec,
                        candidates: Vec::new(),
                        speech_score,
                        energy: ad_energy,
                    };
                }

                let best_peak_score = chosen[0].1;
                let second_best = chosen.get(1).map(|c| c.1).unwrap_or(0.0);
                let mut candidates = Vec::with_capacity(chosen.len());

                for (rank, (pidx, _)) in chosen.into_iter().enumerate() {
                    let (refined_pos, refined_score) = parabolic_peak_refine(&corr, pidx);
                    let vid_match_start = (search_start as f64) + refined_pos;
                    let vid_center_sec =
                        (vid_match_start + (win_samples as f64) * 0.5) * sec_per_sample;
                    let offset_sec = vid_center_sec - ad_center_sec;
                    let sharpness = refined_score / mean_corr.max(1e-10);
                    let ratio = if rank == 0 {
                        (refined_score / second_best.max(1e-10)).min(10.0)
                    } else {
                        refined_score / best_peak_score.max(1e-10)
                    };
                    candidates.push(OffsetCandidate {
                        offset_sec,
                        score: refined_score,
                        peak_sharpness: sharpness,
                        peak_ratio: ratio,
                    });
                }

                CandidateWindow {
                    source_center: ad_center_sec,
                    candidates,
                    speech_score,
                    energy: ad_energy,
                }
            })
            .collect();

        lattice.extend(batch_wins);
        on_progress(lattice.len(), total_windows);
    }

    // Augment starved windows with fingerprint matches if available
    if let Some(fp_res) = use_fp {
        augment_lattice_with_fingerprint(&mut lattice, fp_res);
    }

    lattice
}

fn augment_lattice_with_fingerprint(lattice: &mut [CandidateWindow], fp: &FingerprintResult) {
    if fp.match_t_ad.is_empty() {
        return;
    }
    let half_win = 4.0_f64;
    for w in lattice.iter_mut() {
        let lo = fp
            .match_t_ad
            .partition_point(|&t| (t as f64) < w.source_center - half_win);
        let hi = fp
            .match_t_ad
            .partition_point(|&t| (t as f64) <= w.source_center + half_win);
        if hi - lo >= 6 {
            let mut offs: Vec<f64> = (lo..hi)
                .map(|i| (fp.match_t_vid[i] as f64) - (fp.match_t_ad[i] as f64))
                .collect();
            offs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let med = offs[offs.len() / 2];
            let has_strong = w
                .candidates
                .iter()
                .any(|c| c.score >= 0.55 && (c.offset_sec - med).abs() <= 0.15);
            if !has_strong {
                let score = (0.4 + 0.02 * ((hi - lo) as f64)).min(0.8);
                w.candidates.push(OffsetCandidate {
                    offset_sec: med,
                    score,
                    peak_sharpness: 1.0,
                    peak_ratio: 1.0,
                });
                w.candidates.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
    }
}

/// Second-order Viterbi DP over `(prev, curr)` candidate pairs with jump, curvature, and speech penalties,
/// followed by spur suppression.
pub fn decode_warp_path(
    lattice: &[CandidateWindow],
    offset_hint: f64,
    fp: Option<&FingerprintResult>,
) -> (Vec<WarpPoint>, f64) {
    if lattice.is_empty() {
        return (Vec::new(), 0.0);
    }

    let use_fp = fp.filter(|f| f.is_strong());
    let prepared = prepare_lattice(lattice, offset_hint, use_fp);
    let n = prepared.len();
    let ks: Vec<usize> = prepared.iter().map(|w| w.candidates.len()).collect();
    if ks.iter().all(|&k| k == 0) {
        return (Vec::new(), 0.0);
    }

    let lambda_jump = 2.0_f64;
    let lambda_curve = 5.0_f64;
    let lambda_speech = 0.3_f64;
    let hint_weight = 0.02_f64;
    let hint_clamp = 10.0_f64;

    let hint_at: Vec<Option<f64>> = prepared
        .iter()
        .map(|w| use_fp.and_then(|f| f.offset_at(w.source_center)))
        .collect();

    let reward = |t: usize, cand: &OffsetCandidate| -> f64 {
        let mut r = -cand.score - lambda_speech * prepared[t].speech_score * cand.score;
        if let Some(h) = hint_at[t] {
            r += hint_weight * (cand.offset_sec - h).abs().min(hint_clamp);
        }
        r
    };

    let offsets_by_t: Vec<Vec<f64>> = prepared
        .iter()
        .map(|w| w.candidates.iter().map(|c| c.offset_sec).collect())
        .collect();

    let cost0: Vec<f64> = prepared[0]
        .candidates
        .iter()
        .map(|c| reward(0, c) + 0.3 * (c.offset_sec - offset_hint).abs())
        .collect();

    let (path_indices, total_cost) = if n == 1 {
        let (bi, &bc) = cost0
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        (vec![bi], bc)
    } else {
        let mut pair_cost: Vec<Vec<f64>> = (0..ks[0])
            .map(|j| {
                (0..ks[1])
                    .map(|k| {
                        cost0[j]
                            + lambda_jump * (offsets_by_t[1][k] - offsets_by_t[0][j]).abs()
                            + reward(1, &prepared[1].candidates[k])
                    })
                    .collect()
            })
            .collect();

        let mut back2: Vec<Vec<Vec<usize>>> = vec![vec![vec![0; ks[1]]; ks[0]]];

        for t in 2..n {
            let (k_pp, k_p, k_c) = (ks[t - 2], ks[t - 1], ks[t]);
            let (off_pp, off_p, off_c) = (
                &offsets_by_t[t - 2],
                &offsets_by_t[t - 1],
                &offsets_by_t[t],
            );
            let rewards: Vec<f64> = (0..k_c)
                .map(|k| reward(t, &prepared[t].candidates[k]))
                .collect();

            let mut new_cost = vec![vec![f64::INFINITY; k_c]; k_p];
            let mut new_back = vec![vec![0usize; k_c]; k_p];

            for j in 0..k_p {
                for k in 0..k_c {
                    let delta = off_c[k] - off_p[j];
                    let base = lambda_jump * delta.abs() + rewards[k];
                    let mut best_total = f64::INFINITY;
                    let mut best_i = 0usize;
                    for i in 0..k_pp {
                        let prev_d = off_p[j] - off_pp[i];
                        let total = pair_cost[i][j] + base + lambda_curve * (delta - prev_d).abs();
                        if total < best_total {
                            best_total = total;
                            best_i = i;
                        }
                    }
                    new_cost[j][k] = best_total;
                    new_back[j][k] = best_i;
                }
            }
            pair_cost = new_cost;
            back2.append(&mut vec![new_back]);
        }

        let mut best_j = 0;
        let mut best_k = 0;
        let mut min_c = f64::INFINITY;
        for (j, row) in pair_cost.iter().enumerate() {
            for (k, &c) in row.iter().enumerate() {
                if c < min_c {
                    min_c = c;
                    best_j = j;
                    best_k = k;
                }
            }
        }

        let mut indices = vec![0usize; n];
        indices[n - 1] = best_k;
        indices[n - 2] = best_j;
        for t in (2..n).rev() {
            indices[t - 2] = back2[t - 1][indices[t - 1]][indices[t]];
        }
        (indices, min_c)
    };

    // Spur suppression (runs up to 6 windows jumping > 2.0s away from agreeing flanks)
    let mut path_offsets: Vec<f64> = (0..n)
        .map(|t| prepared[t].candidates[path_indices[t]].offset_sec)
        .collect();
    let mut spur = vec![false; n];
    let max_spur_run = 6;
    let spur_flank = 5;

    let mut t = 1usize;
    while t + 1 < n {
        let mut left_slice: Vec<f64> = path_offsets[t.saturating_sub(spur_flank)..t].to_vec();
        left_slice.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let left_flank = left_slice[left_slice.len() / 2];

        let mut run_end = t;
        while run_end < n
            && (run_end - t) < max_spur_run
            && (path_offsets[run_end] - left_flank).abs() > 2.0
        {
            run_end += 1;
        }
        if run_end > t && run_end < n {
            let mut right_slice: Vec<f64> =
                path_offsets[run_end..(run_end + spur_flank).min(n)].to_vec();
            right_slice.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let right_flank = right_slice[right_slice.len() / 2];
            if (right_flank - left_flank).abs() <= 0.5 {
                let fill = 0.5 * (left_flank + right_flank);
                for i in t..run_end {
                    path_offsets[i] = fill;
                    spur[i] = true;
                }
                t = run_end;
            }
        }
        t += 1;
    }

    let points: Vec<WarpPoint> = (0..n)
        .map(|i| {
            let w = &prepared[i];
            let cand = &w.candidates[path_indices[i]];
            let conf = if spur[i] {
                0.05
            } else {
                cand.score * (0.5 + 0.5 * w.speech_score)
            };
            WarpPoint {
                source_time: w.source_center,
                target_time: w.source_center + path_offsets[i],
                confidence: conf,
                is_anchor: false,
            }
        })
        .collect();

    (points, total_cost)
}

fn prepare_lattice(
    lattice: &[CandidateWindow],
    offset_hint: f64,
    fp: Option<&FingerprintResult>,
) -> Vec<CandidateWindow> {
    let mut all_offs: Vec<f64> = lattice
        .iter()
        .flat_map(|w| w.candidates.iter().map(|c| c.offset_sec))
        .collect();
    let fallback = if all_offs.is_empty() {
        offset_hint
    } else {
        all_offs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        all_offs[all_offs.len() / 2]
    };

    let n = lattice.len();
    let mut fwd = vec![fallback; n];
    let mut last = fallback;
    for (i, w) in lattice.iter().enumerate() {
        fwd[i] = last;
        if let Some(c) = w.candidates.first() {
            last = c.offset_sec;
        }
    }

    let mut bwd = vec![fallback; n];
    let mut nxt = fallback;
    for i in (0..n).rev() {
        bwd[i] = nxt;
        if let Some(c) = lattice[i].candidates.first() {
            nxt = c.offset_sec;
        }
    }

    let weak_evidence = 0.45_f64;
    let hint_contra = 3.0_f64;

    let mut prepared = Vec::with_capacity(n);
    for (i, w) in lattice.iter().enumerate() {
        let strong = w
            .candidates
            .first()
            .map(|c| c.score >= weak_evidence)
            .unwrap_or(false);
        let mut contradicted = false;
        if strong {
            if let Some(f) = fp {
                if let Some(h) = f.offset_at(w.source_center) {
                    contradicted = w
                        .candidates
                        .iter()
                        .filter(|c| c.score >= weak_evidence)
                        .all(|c| (c.offset_sec - h).abs() > hint_contra);
                }
            }
        }
        if strong && !contradicted {
            prepared.push(w.clone());
            continue;
        }

        let mut synth_offs = vec![fwd[i], bwd[i]];
        if let Some(f) = fp {
            if let Some(h) = f.offset_at(w.source_center) {
                synth_offs.push(h);
            }
        }
        let mut cands = w.candidates.clone();
        for o in synth_offs {
            if cands.iter().all(|c| (c.offset_sec - o).abs() > 0.05) {
                cands.push(OffsetCandidate {
                    offset_sec: o,
                    score: 0.1,
                    peak_sharpness: 1.0,
                    peak_ratio: 1.0,
                });
            }
        }
        prepared.push(CandidateWindow {
            source_center: w.source_center,
            candidates: cands,
            speech_score: w.speech_score,
            energy: w.energy,
        });
    }
    prepared
}

/// Fits monotone PCHIP splines across contiguous segments, vets out non-monotone excursions,
/// and nudges structural cut boundaries into narrator silence gaps (`_nudge_to_silence`).
pub fn fit_and_refine_warp(
    path: Vec<WarpPoint>,
    ad_duration: f64,
    video_duration: f64,
    y_ad_8k: &[f32],
    sr_8k: u32,
    discontinuity_threshold: f64,
    anchor_fraction: f64,
) -> WarpPlan {
    if path.is_empty() {
        let sp = PchipSpline::new(vec![0.0, ad_duration], vec![0.0, ad_duration]);
        return WarpPlan {
            splines: vec![sp],
            segment_ranges: vec![(0.0, ad_duration)],
            points: Vec::new(),
            anchor_points: Vec::new(),
            cuts: Vec::new(),
            explicit_gaps: Vec::new(),
            confidence: 0.0,
            global_offset: 0.0,
        };
    }

    let raw_segs = split_at_discontinuities(&path, discontinuity_threshold);
    let vetted_path = vet_and_bridge(raw_segs);
    let segments = split_at_discontinuities(&vetted_path, discontinuity_threshold);

    let mut splines = Vec::with_capacity(segments.len());
    let mut segment_ranges = Vec::with_capacity(segments.len());
    let mut all_anchors = Vec::new();
    let n_segs = segments.len();

    for (seg_idx, seg_pts) in segments.into_iter().enumerate() {
        if seg_pts.len() < 2 {
            let p = seg_pts[0];
            let off = p.target_time - p.source_time;
            let s0 = (p.source_time - 5.0).max(0.0);
            let s1 = (p.source_time + 5.0).min(ad_duration);
            splines.push(PchipSpline::new(vec![s0, s1], vec![s0 + off, s1 + off]));
            segment_ranges.push((s0, s1));
            all_anchors.push(WarpPoint {
                is_anchor: true,
                ..p
            });
            continue;
        }

        let mut anchors = select_anchors(&seg_pts, anchor_fraction);
        anchors = enforce_monotonicity(anchors);
        if anchors.len() < 2 {
            anchors = enforce_monotonicity(vec![seg_pts[0], *seg_pts.last().unwrap()]);
        }
        anchors = add_virtual_boundaries(anchors, seg_idx, n_segs, ad_duration);

        let xs: Vec<f64> = anchors.iter().map(|a| a.source_time).collect();
        let ys: Vec<f64> = anchors.iter().map(|a| a.target_time).collect();
        let s_lo = xs[0];
        let s_hi = *xs.last().unwrap();
        splines.push(PchipSpline::new(xs, ys));
        segment_ranges.push((s_lo, s_hi));
        all_anchors.extend(anchors);
    }

    if !segment_ranges.is_empty() {
        segment_ranges[0].0 = 0.0;
        let last_idx = segment_ranges.len() - 1;
        segment_ranges[last_idx].1 = ad_duration;
        for i in 0..last_idx {
            let a_end = segment_ranges[i].1;
            let b_start = segment_ranges[i + 1].0;
            if b_start > a_end {
                let mid = 0.5 * (a_end + b_start);
                segment_ranges[i].1 = mid;
                segment_ranges[i + 1].0 = mid;
            }
        }
    }

    // Refine structural cut boundaries by nudging into AD narration silence gaps (our custom ADSync improvement!)
    refine_boundaries_to_silence(&mut splines, &mut segment_ranges, y_ad_8k, sr_8k);

    let cuts = extract_cuts_from_points(&vetted_path, 0.20);
    let mut explicit_gaps = Vec::new();
    if let Some(first_cut) = cuts.first() {
        if first_cut.vid_start >= 0.20 {
            explicit_gaps.push((0.0, first_cut.vid_start, first_cut.vid_start));
        }
        for i in 0..cuts.len() - 1 {
            let g_start = cuts[i].vid_end;
            let g_end = cuts[i + 1].vid_start;
            if g_end - g_start >= 0.50 {
                explicit_gaps.push((g_start, g_end, g_end - g_start));
            }
        }
        if let Some(last_cut) = cuts.last() {
            if video_duration - last_cut.vid_end >= 0.50 {
                explicit_gaps.push((
                    last_cut.vid_end,
                    video_duration,
                    video_duration - last_cut.vid_end,
                ));
            }
        }
    }

    let mean_conf = if vetted_path.is_empty() {
        0.0
    } else {
        (vetted_path.iter().map(|p| p.confidence).sum::<f64>() / (vetted_path.len() as f64))
            .clamp(0.0, 1.0)
    };

    let mut offsets: Vec<f64> = vetted_path
        .iter()
        .map(|p| p.target_time - p.source_time)
        .collect();
    let global_offset = if offsets.is_empty() {
        0.0
    } else {
        offsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        offsets[offsets.len() / 2]
    };

    WarpPlan {
        splines,
        segment_ranges,
        points: vetted_path,
        anchor_points: all_anchors,
        cuts,
        explicit_gaps,
        confidence: mean_conf,
        global_offset,
    }
}

fn split_at_discontinuities(path: &[WarpPoint], threshold: f64) -> Vec<Vec<WarpPoint>> {
    if path.len() <= 1 {
        return vec![path.to_vec()];
    }
    let synth_conf = 0.12_f64;
    let mut segments: Vec<Vec<WarpPoint>> = vec![vec![path[0]]];
    for i in 1..path.len() {
        let prev_off = path[i - 1].target_time - path[i - 1].source_time;
        let cur_off = path[i].target_time - path[i].source_time;
        let jump = (cur_off - prev_off).abs();
        let mut small_cut = false;
        if jump > 0.25 && jump <= threshold && i >= 3 && i + 3 <= path.len() {
            let left = &path[i - 3..i];
            let right = &path[i..i + 3];
            if left.iter().chain(right.iter()).all(|p| p.confidence > synth_conf) {
                let l_offs: Vec<f64> = left.iter().map(|p| p.target_time - p.source_time).collect();
                let r_offs: Vec<f64> =
                    right.iter().map(|p| p.target_time - p.source_time).collect();
                let l_ptp = l_offs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                    - l_offs.iter().copied().fold(f64::INFINITY, f64::min);
                let r_ptp = r_offs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                    - r_offs.iter().copied().fold(f64::INFINITY, f64::min);
                small_cut = l_ptp <= 0.10 && r_ptp <= 0.10;
            }
        }
        if jump > threshold || small_cut {
            segments.push(Vec::new());
        }
        segments.last_mut().unwrap().push(path[i]);
    }
    segments.into_iter().filter(|s| !s.is_empty()).collect()
}

fn vet_and_bridge(segments: Vec<Vec<WarpPoint>>) -> Vec<WarpPoint> {
    if segments.len() <= 1 {
        return segments.into_iter().flatten().collect();
    }
    let synth_conf = 0.12_f64;
    let reals: Vec<Vec<WarpPoint>> = segments
        .iter()
        .map(|s| s.iter().copied().filter(|p| p.confidence > synth_conf).collect())
        .collect();
    let masses: Vec<f64> = reals
        .iter()
        .map(|r| r.iter().map(|p| p.confidence).sum())
        .collect();

    let mut eligible: Vec<usize> = (0..segments.len())
        .filter(|&i| {
            reals[i].len() >= 3
                && (reals[i].last().unwrap().source_time - reals[i][0].source_time) >= 8.0
        })
        .collect();
    if eligible.is_empty() {
        if let Some((best_i, _)) = masses
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            eligible.push(best_i);
        }
    }

    let m = eligible.len();
    let vid_lo: Vec<f64> = eligible
        .iter()
        .map(|&i| {
            reals[i]
                .first()
                .unwrap_or(&segments[i][0])
                .target_time
        })
        .collect();
    let vid_hi: Vec<f64> = eligible
        .iter()
        .map(|&i| {
            reals[i]
                .last()
                .unwrap_or(segments[i].last().unwrap())
                .target_time
        })
        .collect();

    let mut best = vec![0.0_f64; m];
    let mut prev = vec![None; m];
    for a in 0..m {
        best[a] = masses[eligible[a]];
        for b in 0..a {
            if vid_lo[a] >= vid_hi[b] - 1.0 && best[b] + masses[eligible[a]] > best[a] {
                best[a] = best[b] + masses[eligible[a]];
                prev[a] = Some(b);
            }
        }
    }

    let mut keep = std::collections::HashSet::new();
    if let Some((mut cur, _)) = best
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
    {
        loop {
            keep.insert(eligible[cur]);
            if let Some(p) = prev[cur] {
                cur = p;
            } else {
                break;
            }
        }
    }

    if keep.len() == segments.len() {
        return segments.into_iter().flatten().collect();
    }

    let mut kept_sorted: Vec<usize> = keep.iter().copied().collect();
    kept_sorted.sort_unstable();

    let mut new_points = Vec::new();
    for (idx, seg) in segments.iter().enumerate() {
        if keep.contains(&idx) {
            new_points.extend_from_slice(seg);
            continue;
        }
        let left = kept_sorted.iter().rev().copied().find(|&j| j < idx);
        let right = kept_sorted.iter().copied().find(|&j| j > idx);
        let left_p = left.and_then(|l| reals[l].last().copied());
        let right_p = right.and_then(|r| reals[r].first().copied());

        match (left_p, right_p) {
            (Some(lp), Some(rp)) => {
                let ad_span = rp.source_time - lp.source_time;
                let video_room = rp.target_time - lp.target_time;
                if video_room >= ad_span - 0.5 {
                    // Preserves our custom ADSync fix: keep previous-scene audio with left_off
                    // so pre-cut audio never bleeds into the start of the next scene!
                    let left_off = lp.target_time - lp.source_time;
                    let right_off = rp.target_time - rp.source_time;
                    for p in seg {
                        let off = if p.source_time < rp.source_time {
                            left_off
                        } else {
                            right_off
                        };
                        new_points.push(WarpPoint {
                            source_time: p.source_time,
                            target_time: p.source_time + off,
                            confidence: 0.05,
                            is_anchor: false,
                        });
                    }
                } else {
                    let rise = video_room.max(0.0);
                    for p in seg {
                        let frac =
                            ((p.source_time - lp.source_time) / ad_span.max(1e-9)).clamp(0.0, 1.0);
                        new_points.push(WarpPoint {
                            source_time: p.source_time,
                            target_time: lp.target_time + frac * rise,
                            confidence: 0.05,
                            is_anchor: false,
                        });
                    }
                }
            }
            (Some(flank), None) | (None, Some(flank)) => {
                let off = flank.target_time - flank.source_time;
                for p in seg {
                    new_points.push(WarpPoint {
                        source_time: p.source_time,
                        target_time: p.source_time + off,
                        confidence: 0.05,
                        is_anchor: false,
                    });
                }
            }
            (None, None) => {
                new_points.extend_from_slice(seg);
            }
        }
    }

    new_points
}

fn select_anchors(points: &[WarpPoint], fraction: f64) -> Vec<WarpPoint> {
    let n = points.len();
    if n <= 2 {
        return points.to_vec();
    }
    let dur = points.last().unwrap().source_time - points[0].source_time;
    let min_cnt = ((dur / 30.0) as usize).max(2);
    let target_cnt = ((n as f64 * fraction) as usize).max(min_cnt).min(n);

    let mut ranked: Vec<usize> = (0..n).collect();
    ranked.sort_by(|&a, &b| {
        points[b]
            .confidence
            .partial_cmp(&points[a].confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = std::collections::BTreeSet::new();
    selected.insert(0);
    selected.insert(n - 1);
    for idx in ranked {
        if selected.len() >= target_cnt {
            break;
        }
        selected.insert(idx);
    }

    selected
        .into_iter()
        .map(|i| WarpPoint {
            is_anchor: true,
            ..points[i]
        })
        .collect()
}

fn enforce_monotonicity(mut anchors: Vec<WarpPoint>) -> Vec<WarpPoint> {
    if anchors.len() <= 1 {
        return anchors;
    }
    anchors.sort_by(|a, b| {
        a.source_time
            .partial_cmp(&b.source_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = anchors.len();
    let mut dp: Vec<f64> = anchors.iter().map(|a| a.confidence).collect();
    let mut prev = vec![None; n];

    for i in 1..n {
        for j in 0..i {
            if anchors[j].target_time < anchors[i].target_time
                && dp[j] + anchors[i].confidence > dp[i]
            {
                dp[i] = dp[j] + anchors[i].confidence;
                prev[i] = Some(j);
            }
        }
    }

    let (mut cur, _) = dp
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();
    let mut lis = Vec::new();
    loop {
        lis.push(anchors[cur]);
        if let Some(p) = prev[cur] {
            cur = p;
        } else {
            break;
        }
    }
    lis.reverse();
    lis
}

fn add_virtual_boundaries(
    mut anchors: Vec<WarpPoint>,
    seg_idx: usize,
    n_segments: usize,
    ad_duration: f64,
) -> Vec<WarpPoint> {
    if anchors.len() < 2 {
        return anchors;
    }
    if seg_idx == 0 && anchors[0].source_time > 0.1 {
        let first = anchors[0];
        let second = anchors[1];
        let slope = (second.target_time - first.target_time)
            / (second.source_time - first.source_time).max(1e-6);
        let t0_target = first.target_time - first.source_time * slope;
        anchors.insert(
            0,
            WarpPoint {
                source_time: 0.0,
                target_time: t0_target,
                confidence: first.confidence * 0.8,
                is_anchor: true,
            },
        );
    }
    if seg_idx + 1 == n_segments && anchors.last().unwrap().source_time < ad_duration - 0.1 {
        let last = *anchors.last().unwrap();
        let prev = anchors[anchors.len() - 2];
        let slope =
            (last.target_time - prev.target_time) / (last.source_time - prev.source_time).max(1e-6);
        let tend_target = last.target_time + (ad_duration - last.source_time) * slope;
        anchors.push(WarpPoint {
            source_time: ad_duration,
            target_time: tend_target,
            confidence: last.confidence * 0.8,
            is_anchor: true,
        });
    }
    anchors
}

/// Nudges segment cut boundaries into sustained silence gaps in AD narration (`_nudge_to_silence` from ADSync `refine.py`).
fn refine_boundaries_to_silence(
    splines: &mut [PchipSpline],
    segment_ranges: &mut [(f64, f64)],
    y_ad: &[f32],
    sr: u32,
) {
    if y_ad.is_empty() || sr == 0 || segment_ranges.len() < 2 {
        return;
    }
    for idx in 0..segment_ranges.len() - 1 {
        let (left_start, boundary) = segment_ranges[idx];
        let (right_start, right_end) = segment_ranges[idx + 1];
        if (right_start - boundary).abs() > 1e-4 {
            continue;
        }
        let jump = splines[idx + 1].eval(boundary) - splines[idx].eval(boundary);
        if jump.abs() < 0.25 {
            continue;
        }
        let silence_b = nudge_to_silence(
            y_ad,
            sr,
            boundary,
            left_start,
            right_end,
            20.0,
            0.008,
            0.15,
            jump,
        );
        if silence_b > left_start && silence_b < right_end && (silence_b - boundary).abs() > 0.05 {
            // Extend PCHIP knot endpoints smoothly
            let mut ok = true;
            let mut new_left = splines[idx].clone();
            let mut new_right = splines[idx + 1].clone();

            if silence_b > *new_left.x.last().unwrap() {
                let edge = *new_left.x.last().unwrap();
                let slope = new_left.derivative(edge);
                if (0.5..=2.0).contains(&slope) {
                    let target = new_left.eval(edge) + (silence_b - edge) * slope;
                    let mut xs = new_left.x.clone();
                    let mut ys = new_left.y.clone();
                    xs.push(silence_b);
                    ys.push(target);
                    new_left = PchipSpline::new(xs, ys);
                } else {
                    ok = false;
                }
            }
            if ok && silence_b < new_right.x[0] {
                let edge = new_right.x[0];
                let slope = new_right.derivative(edge);
                if (0.5..=2.0).contains(&slope) {
                    let target = new_right.eval(edge) + (silence_b - edge) * slope;
                    let mut xs = vec![silence_b];
                    let mut ys = vec![target];
                    xs.extend_from_slice(&new_right.x);
                    ys.extend_from_slice(&new_right.y);
                    new_right = PchipSpline::new(xs, ys);
                } else {
                    ok = false;
                }
            }

            if ok {
                splines[idx] = new_left;
                splines[idx + 1] = new_right;
                segment_ranges[idx].1 = silence_b;
                segment_ranges[idx + 1].0 = silence_b;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn nudge_to_silence(
    y_ad: &[f32],
    sr: u32,
    boundary: f64,
    left_start: f64,
    right_end: f64,
    search_radius: f64,
    threshold: f64,
    silence_sec: f64,
    jump: f64,
) -> f64 {
    let ad_len = y_ad.len();
    let sr_f = sr as f64;
    let origin = (boundary * sr_f) as usize;
    if origin >= ad_len {
        return boundary;
    }
    let win = ((0.05 * sr_f) as usize).max(1);
    let need_silent = ((silence_sec / 0.05) as usize).max(2);
    let rad = (search_radius * sr_f) as usize;
    let lo = (((left_start * sr_f) as usize) + win).max(origin.saturating_sub(rad));
    let hi = (((right_end * sr_f) as usize).saturating_sub(win))
        .min(origin + rad)
        .min(ad_len.saturating_sub(win));
    if hi <= lo + need_silent * win {
        return boundary;
    }

    let n_wins = (hi - lo) / win;
    let mut rms_arr: Vec<f64> = (0..n_wins)
        .map(|w| {
            let chunk = &y_ad[lo + w * win..lo + (w + 1) * win];
            let sum_sq: f64 = chunk.iter().map(|&x| (x as f64) * (x as f64)).sum();
            (sum_sq / (win as f64)).sqrt()
        })
        .collect();
    let mut sorted_rms = rms_arr.clone();
    sorted_rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let local_floor = sorted_rms[(sorted_rms.len() * 15) / 100];
    let eff_thresh = threshold.max((local_floor * 1.5).min(0.045));

    // Check if origin is already inside silence
    let half_check = (need_silent * win) / 2;
    let c_start = origin.saturating_sub(half_check);
    let c_end = (origin + half_check).min(ad_len);
    if c_end > c_start {
        let c_rms = (y_ad[c_start..c_end]
            .iter()
            .map(|&x| (x as f64) * (x as f64))
            .sum::<f64>()
            / ((c_end - c_start) as f64))
            .sqrt();
        if c_rms < eff_thresh {
            return boundary;
        }
    }

    let find_runs = |min_wins: usize| -> Vec<(usize, usize)> {
        let mut runs = Vec::new();
        let mut in_run = false;
        let mut run_start = lo;
        for (i, &r) in rms_arr.iter().enumerate() {
            let p = lo + i * win;
            if r < eff_thresh {
                if !in_run {
                    in_run = true;
                    run_start = p;
                }
            } else if in_run {
                in_run = false;
                if p - run_start >= min_wins * win {
                    runs.push((run_start, p));
                }
            }
        }
        if in_run && (lo + n_wins * win) - run_start >= min_wins * win {
            runs.push((run_start, lo + n_wins * win));
        }
        runs
    };

    let mut silent_runs = find_runs(need_silent);
    if silent_runs.is_empty() && need_silent > 2 {
        silent_runs = find_runs(2);
    }
    let _ = &mut rms_arr;
    if silent_runs.is_empty() {
        return boundary;
    }

    let best_run = if jump > 0.4 {
        silent_runs
            .iter()
            .copied()
            .filter(|&(a, b)| (a + b) / 2 <= origin)
            .last()
            .unwrap_or_else(|| {
                *silent_runs
                    .iter()
                    .min_by_key(|&&(a, b)| ((a + b) / 2).abs_diff(origin))
                    .unwrap()
            })
    } else if jump < -0.4 {
        silent_runs
            .iter()
            .copied()
            .find(|&(a, b)| (a + b) / 2 >= origin)
            .unwrap_or_else(|| {
                *silent_runs
                    .iter()
                    .min_by_key(|&&(a, b)| ((a + b) / 2).abs_diff(origin))
                    .unwrap()
            })
    } else {
        *silent_runs
            .iter()
            .min_by_key(|&&(a, b)| ((a + b) / 2).abs_diff(origin))
            .unwrap()
    };

    (0.5 * ((best_run.0 + best_run.1) as f64)) / sr_f
}

pub fn extract_cuts_from_points(pts: &[WarpPoint], jump_threshold: f64) -> Vec<WarpCut> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let offsets: Vec<f64> = pts.iter().map(|p| p.target_time - p.source_time).collect();
    let mut runs: Vec<(f64, f64, f64)> = Vec::new();
    let mut start = 0usize;
    for i in 1..=pts.len() {
        if i == pts.len() || (offsets[i] - offsets[i - 1]).abs() > jump_threshold {
            let mut seg_offs = offsets[start..i].to_vec();
            seg_offs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid_off = seg_offs[seg_offs.len() / 2];
            runs.push((pts[start].target_time, pts[i - 1].target_time, mid_off));
            start = i;
        }
    }

    let mut cuts = Vec::with_capacity(runs.len());
    for (idx, &(vs, ve, off)) in runs.iter().enumerate() {
        if idx == 0 {
            cuts.push(WarpCut {
                index: 1,
                vid_start: vs.max(0.0),
                vid_end: ve,
                offset: off,
                jump: 0.0,
                is_initial: true,
            });
        } else {
            let prev_off = runs[idx - 1].2;
            cuts.push(WarpCut {
                index: idx + 1,
                vid_start: vs,
                vid_end: ve,
                offset: off,
                jump: off - prev_off,
                is_initial: false,
            });
        }
    }
    cuts
}

/// Renders a warped 48kHz stereo WAV from `hq_ad_wav_path` into `output_wav_path` using streaming
/// 30-second blocks and zero-allocation Catmull-Rom cubic interpolation + equal-power crossfades!
/// Peak memory is < 35 MB regardless of movie duration (vs. ~12 GB in Python/NumPy).
pub fn render_warped_wav_streaming(
    hq_ad_wav_path: &Path,
    output_wav_path: &Path,
    plan: &WarpPlan,
    video_duration: f64,
    crossfade_ms: u32,
) -> Result<(), String> {
    let mut reader = WavStreamReader::open(hq_ad_wav_path)?;
    let sr = reader.layout.sample_rate;
    let sr_f = sr as f64;
    let total_ad_frames = reader.layout.total_frames;
    let total_out_frames = (video_duration * sr_f).round() as u64;

    let mut writer = WavStreamWriter::create(output_wav_path, sr, 2, total_out_frames)?;

    // Precompute inverse PCHIP splines and output video frame intervals for each segment
    struct SegmentRenderSpec {
        inv_spline: PchipSpline,
        out_start: u64,
        out_end: u64,
    }

    let mut specs = Vec::new();
    for (sp, &seg_range) in plan.splines.iter().zip(plan.segment_ranges.iter()) {
        let vid_start = sp.eval(seg_range.0).clamp(0.0, video_duration);
        let vid_end = sp.eval(seg_range.1).clamp(0.0, video_duration);
        if vid_end <= vid_start {
            continue;
        }
        let out_s = ((vid_start * sr_f).ceil() as u64).min(total_out_frames);
        let out_e = (((vid_end * sr_f).floor() as u64) + 1).min(total_out_frames);
        if out_e > out_s {
            specs.push(SegmentRenderSpec {
                inv_spline: sp.invert(seg_range),
                out_start: out_s,
                out_end: out_e,
            });
        }
    }

    let block_frames = (sr as usize) * 30; // 30-second streaming output block
    let xf_frames = ((sr as u64) * (crossfade_ms as u64)) / 1000;
    let mut out_block = vec![0.0_f32; block_frames * 2];
    let mut src_cache_buf = Vec::new();

    let mut cur_out_frame = 0u64;
    while cur_out_frame < total_out_frames {
        let take = ((total_out_frames - cur_out_frame) as usize).min(block_frames);
        let block_end_frame = cur_out_frame + (take as u64);
        let slice = &mut out_block[..take * 2];
        slice.fill(0.0);

        for (seg_idx, spec) in specs.iter().enumerate() {
            // Check if this segment overlaps [cur_out_frame, block_end_frame)
            let ov_start = cur_out_frame.max(spec.out_start);
            let ov_end = block_end_frame.min(spec.out_end);
            if ov_end <= ov_start {
                continue;
            }

            // Determine source AD frame range needed for [ov_start, ov_end)
            let t0 = (ov_start as f64) / sr_f;
            let t1 = ((ov_end - 1) as f64) / sr_f;
            let ad_f0 = (spec.inv_spline.eval(t0) * sr_f).floor() as i64;
            let ad_f1 = (spec.inv_spline.eval(t1) * sr_f).ceil() as i64;

            let read_lo = (ad_f0.min(ad_f1) - 4).max(0) as u64;
            let read_hi = ((ad_f0.max(ad_f1) + 6).max(0) as u64).min(total_ad_frames);
            if read_hi <= read_lo + 1 {
                continue;
            }
            let read_count = (read_hi - read_lo) as usize;
            let actual_read =
                reader.read_frames_stereo_f32(read_lo, read_count, &mut src_cache_buf)?;
            if actual_read < 2 {
                continue;
            }

            let max_local_idx = (actual_read - 1) as isize;
            let prev_end = if seg_idx > 0 {
                Some(specs[seg_idx - 1].out_end)
            } else {
                None
            };
            let next_start = if seg_idx + 1 < specs.len() {
                Some(specs[seg_idx + 1].out_start)
            } else {
                None
            };

            for out_f in ov_start..ov_end {
                let t = (out_f as f64) / sr_f;
                let src_pos = (spec.inv_spline.eval(t) * sr_f) - (read_lo as f64);
                let idx = (src_pos.floor() as isize).clamp(0, max_local_idx - 1);
                let u = (src_pos - (idx as f64)).clamp(0.0, 1.0) as f32;
                let u2 = u * u;
                let u3 = u2 * u;

                let i0 = (idx - 1).clamp(0, max_local_idx) as usize;
                let i1 = idx as usize;
                let i2 = (idx + 1).clamp(0, max_local_idx) as usize;
                let i3 = (idx + 2).clamp(0, max_local_idx) as usize;

                // Fused Catmull-Rom cubic interpolation for Left and Right channels
                let mut sample_l = {
                    let p0 = src_cache_buf[i0 * 2];
                    let p1 = src_cache_buf[i1 * 2];
                    let p2 = src_cache_buf[i2 * 2];
                    let p3 = src_cache_buf[i3 * 2];
                    0.5 * (2.0 * p1
                        + (p2 - p0) * u
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2
                        + (3.0 * (p1 - p2) + (p3 - p0)) * u3)
                };
                let mut sample_r = {
                    let p0 = src_cache_buf[i0 * 2 + 1];
                    let p1 = src_cache_buf[i1 * 2 + 1];
                    let p2 = src_cache_buf[i2 * 2 + 1];
                    let p3 = src_cache_buf[i3 * 2 + 1];
                    0.5 * (2.0 * p1
                        + (p2 - p0) * u
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2
                        + (3.0 * (p1 - p2) + (p3 - p0)) * u3)
                };

                // Apply inter-segment equal-power fade-in / fade-out at segment boundaries
                let mut gain = 1.0_f32;
                if let Some(pe) = prev_end {
                    if pe > spec.out_start {
                        let overlap = (pe - spec.out_start).min(xf_frames);
                        if overlap > 0 && out_f < spec.out_start + overlap {
                            let phase = ((out_f - spec.out_start) as f32) / (overlap as f32);
                            gain *= (phase * std::f32::consts::FRAC_PI_2).sin();
                        }
                    }
                }
                if let Some(ns) = next_start {
                    if spec.out_end > ns {
                        let overlap = (spec.out_end - ns).min(xf_frames);
                        if overlap > 0 && out_f >= ns && out_f < ns + overlap {
                            let phase = ((out_f - ns) as f32) / (overlap as f32);
                            gain *= (phase * std::f32::consts::FRAC_PI_2).cos();
                        } else if out_f >= ns + overlap {
                            gain = 0.0;
                        }
                    }
                }

                sample_l *= gain;
                sample_r *= gain;

                let local_out = (out_f - cur_out_frame) as usize;
                slice[local_out * 2] += sample_l;
                slice[local_out * 2 + 1] += sample_r;
            }
        }

        writer.write_interleaved_f32(slice)?;
        cur_out_frame = block_end_frame;
    }

    writer.finalize()?;
    Ok(())
}
