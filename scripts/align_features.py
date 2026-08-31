"""
align_features.py - Advanced Audio Alignment Features: Multiband Correlation,
Automatic PAL Speed Detection, and Native Viterbi DP / PCHIP Warp Engine.
"""

from __future__ import annotations

import logging
import math
import os
import subprocess
import sys
from pathlib import Path
from typing import Callable, Tuple, Optional, List

import numpy as np
from numpy.typing import NDArray
from scipy.signal import butter, fftconvolve, sosfiltfilt, resample_poly

log = logging.getLogger("ADMaster")

# ── Band filter edges & weights for speech suppression ────────────────────────
BAND_EDGES = (250.0, 1200.0)
BAND_WEIGHTS = (0.4, 0.2, 0.4)  # low (<250Hz), mid (speech 250-1200Hz), high (>1200Hz)


def create_band_filters(sr: int) -> Tuple[NDArray, NDArray]:
    """Create SOS Butterworth lowpass filters for the band edges."""
    nyq = sr / 2.0
    sos_0 = butter(4, min(BAND_EDGES[0] / nyq, 0.99), btype="low", output="sos")
    sos_1 = butter(4, min(BAND_EDGES[1] / nyq, 0.99), btype="low", output="sos")
    return sos_0, sos_1


def filter_multiband(y: NDArray, sr: int) -> Tuple[NDArray, NDArray, NDArray]:
    """
    Split audio into 3 bands:
      - Low: < 250 Hz (bass, explosions, room rumble)
      - Mid: 250 Hz - 1200 Hz (human vocal fundamentals / narrator speech)
      - High: > 1200 Hz (foley, percussion, crisp sibilance)
    """
    y = np.asarray(y, dtype=np.float64)
    sos_0, sos_1 = create_band_filters(sr)
    l0 = sosfiltfilt(sos_0, y)
    l1 = sosfiltfilt(sos_1, y)
    
    low = l0
    mid = l1 - l0
    high = y - l1
    return low, mid, high


def multiband_normalized_correlation(
    v_region: NDArray,
    a_segment: NDArray,
    sr: int,
    multiband: bool = True
) -> Optional[NDArray]:
    """
    Compute normalized sliding cross-correlation between a reference audio region
    and an AD audio segment, applying multiband narration suppression.
    """
    v_region = np.asarray(v_region, dtype=np.float64)
    a_segment = np.asarray(a_segment, dtype=np.float64)
    win_samples = len(a_segment)

    if len(v_region) < win_samples or win_samples == 0:
        return None

    def _norm_corr_single(v_sig: NDArray, a_sig: NDArray) -> Optional[NDArray]:
        a_cent = a_sig - np.mean(a_sig)
        a_energy = np.sqrt(np.sum(a_cent ** 2))
        if a_energy < 1e-10:
            return None

        v_cent = v_sig - np.mean(v_sig)
        raw = fftconvolve(v_cent, a_cent[::-1], mode="valid")
        if len(raw) == 0:
            return None

        v_sq = v_cent ** 2
        cs_b = np.empty(len(v_sq) + 1, dtype=np.float64)
        cs_b[0] = 0.0
        np.cumsum(v_sq, out=cs_b[1:])
        v_norms_b = np.sqrt(np.maximum(
            cs_b[win_samples: win_samples + len(raw)] - cs_b[:len(raw)], 1e-20
        ))
        return raw / (a_energy * v_norms_b)

    if not multiband:
        return _norm_corr_single(v_region, a_segment)

    # Compute 3-band breakdown
    v_low, v_mid, v_high = filter_multiband(v_region, sr)
    a_low, a_mid, a_high = filter_multiband(a_segment, sr)

    pairs = (
        (v_low, a_low),
        (v_mid, a_mid),
        (v_high, a_high)
    )

    norm_corr = None
    weight_sum = 0.0
    for (v_s, a_s), w in zip(pairs, BAND_WEIGHTS):
        band_corr = _norm_corr_single(v_s, a_s)
        if band_corr is None:
            continue
        if norm_corr is None:
            norm_corr = w * band_corr
        else:
            norm_corr += w * band_corr
        weight_sum += w

    if norm_corr is not None and weight_sum > 0:
        norm_corr /= weight_sum
    return norm_corr


def detect_speed_pal(
    y_vid: NDArray,
    y_ad: NDArray,
    sr: int = 16000
) -> Tuple[bool, float, float]:
    """
    Acoustic landmark speed prober:
    Probes standard transfer ratios (25/24 = 1.041667 and 24/25 = 0.96)
    using landmark constellation hashes.

    Returns:
        (is_pal, stretch_factor, confidence)
    """
    try:
        from adsync.align.fingerprint import fingerprint_align
        from adsync.align.speed import detect_speed

        fp_baseline = fingerprint_align(y_vid, y_ad, sr)
        det = detect_speed(y_vid, y_ad, sr, baseline=fp_baseline)

        if det is not None:
            is_pal = (det.stretch > 1.02)
            conf = float(det.fp.n_matches if det.fp else 0)
            return is_pal, float(det.stretch), conf
    except Exception as e:
        log.warning(f"Speed detection probe failed: {e}")

    return False, 1.0, 0.0
