/// Exact 4th-order Butterworth lowpass filter decomposed into 2 second-order sections (SOS),
/// plus forward-backward zero-phase filtering (`filtfilt`) for 3-band narration speech suppression.

#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    /// Designs a 2nd-order Butterworth lowpass section for pole angle `theta` and cutoff `fc` at sample rate `sr`.
    pub fn butterworth_lp_section(fc: f64, sr: f64, pole_angle: f64) -> Self {
        let nyq = sr * 0.5;
        let wn = (fc / nyq).clamp(1e-4, 0.99);
        let k = (std::f64::consts::PI * wn * 0.5).tan();
        let k2 = k * k;
        let two_k_cos = 2.0 * k * pole_angle.cos();
        let norm = 1.0 / (1.0 + two_k_cos + k2);
        let b0 = k2 * norm;
        let b1 = 2.0 * b0;
        let b2 = b0;
        let a1 = 2.0 * (k2 - 1.0) * norm;
        let a2 = (1.0 - two_k_cos + k2) * norm;
        Self { b0, b1, b2, a1, a2 }
    }

    #[inline]
    pub fn process_in_place(&self, data: &mut [f32]) {
        let mut z1 = 0.0_f64;
        let mut z2 = 0.0_f64;
        for x_ref in data.iter_mut() {
            let x = *x_ref as f64;
            let y = self.b0 * x + z1;
            z1 = self.b1 * x - self.a1 * y + z2;
            z2 = self.b2 * x - self.a2 * y;
            *x_ref = y as f32;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Butter4Lowpass {
    pub s0: Biquad,
    pub s1: Biquad,
}

impl Butter4Lowpass {
    pub fn new(fc: f64, sr: f64) -> Self {
        // 4th-order Butterworth pole angles: pi/8 and 3*pi/8
        let s0 = Biquad::butterworth_lp_section(fc, sr, std::f64::consts::PI / 8.0);
        let s1 = Biquad::butterworth_lp_section(fc, sr, 3.0 * std::f64::consts::PI / 8.0);
        Self { s0, s1 }
    }

    /// Forward-backward zero-phase filtering (`sosfiltfilt`) into `out`.
    pub fn filtfilt(&self, input: &[f32], out: &mut Vec<f32>) {
        out.clear();
        out.extend_from_slice(input);
        if out.len() < 4 {
            return;
        }
        // Forward pass
        self.s0.process_in_place(out);
        self.s1.process_in_place(out);
        // Reverse in place
        out.reverse();
        // Backward pass
        self.s0.process_in_place(out);
        self.s1.process_in_place(out);
        // Restore time order
        out.reverse();
    }
}

pub const BAND_EDGES: (f64, f64) = (250.0, 1200.0);
pub const BAND_WEIGHTS: [f64; 3] = [0.4, 0.2, 0.4]; // low (<250Hz), mid (speech 250-1200Hz), high (>1200Hz)

/// Splits signal `y` into 3 zero-phase bands `(low, mid, high)` in-place without intermediate heap churn.
pub fn filter_multiband_into(
    y: &[f32],
    sr: u32,
    low: &mut Vec<f32>,
    mid: &mut Vec<f32>,
    high: &mut Vec<f32>,
) {
    let f0 = Butter4Lowpass::new(BAND_EDGES.0, sr as f64);
    let f1 = Butter4Lowpass::new(BAND_EDGES.1, sr as f64);
    f0.filtfilt(y, low);
    f1.filtfilt(y, mid);

    high.clear();
    high.reserve(y.len());
    for i in 0..y.len() {
        let l0 = low[i];
        let l1 = mid[i];
        mid[i] = l1 - l0;
        high.push(y[i] - l1);
    }
}
