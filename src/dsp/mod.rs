// SPDX-License-Identifier: MIT
//
// Copyright (c) 2026 torrq
//
// Portions of the non-linear audio alignment algorithms in this module
// (3-band narration speech suppression, STFT acoustic landmark constellation
// speed probing, second-order Viterbi candidate lattice dynamic programming,
// monotone Fritsch-Carlson PCHIP cubic spline warping, and silence-nudged cut
// boundary refinement) are adapted from ADSync:
//   https://github.com/JohnnyTheCoder1/ADSync
//   Copyright (c) 2026 Anilo (MIT License)

pub mod correlation;
pub mod filters;
pub mod fingerprint;
pub mod gap_fill;
pub mod warp;
