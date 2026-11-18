//! FM stereo decoding (broadcast multiplex): mono L+R below 15 kHz, a 19 kHz pilot, and L-R on
//! a suppressed 38 kHz carrier locked to twice the pilot.
//!
//! A PLL locks on the pilot; L-R is brought down with sin(2θ). Without a usable pilot (mono
//! station, weak signal) the decoder fades smoothly to mono.

use crate::dsp::{lowpass, DecimFir, Frame};
use std::f64::consts::PI;

pub struct PilotPll {
    theta: f64,
    w0: f64,
    dw: f64,
    i_lp: f32,
    q_lp: f32,
    a: f32,
    kp: f64,
    ki: f64,
    err_avg: f32,
}

impl PilotPll {
    pub fn new(rate: f64) -> Self {
        let wn = 2.0 * PI * 20.0 / rate; // ~20 Hz loop bandwidth
        Self {
            theta: 0.0,
            w0: 2.0 * PI * 19_000.0 / rate,
            dw: 0.0,
            i_lp: 0.0,
            q_lp: 0.0,
            a: (1.0 - (-2.0 * PI * 200.0 / rate).exp()) as f32,
            kp: 2.0 * 0.707 * wn,
            ki: wn * wn,
            err_avg: 1.0,
        }
    }

    /// One composite sample; returns sin(2θ), the regenerated 38 kHz carrier.
    #[inline]
    pub fn step(&mut self, x: f32) -> f32 {
        let (s, c) = self.theta.sin_cos();
        // Pilot = A sin(φ): x·sinθ -> A/2 cos(φ-θ), x·cosθ -> A/2 sin(φ-θ)
        self.i_lp += self.a * (x * s as f32 - self.i_lp);
        self.q_lp += self.a * (x * c as f32 - self.q_lp);
        let err = self.q_lp.atan2(self.i_lp) as f64;
        self.dw += self.ki * err;
        self.theta = (self.theta + self.w0 + self.dw + self.kp * err).rem_euclid(2.0 * PI);
        self.err_avg += 0.001 * (err.abs() as f32 - self.err_avg);
        (2.0 * s * c) as f32
    }

    /// Pilot amplitude (FM demod units: 1.0 = full deviation)
    pub fn level(&self) -> f32 {
        2.0 * (self.i_lp * self.i_lp + self.q_lp * self.q_lp).sqrt()
    }

    pub fn locked(&self) -> bool {
        self.level() > 0.03 && self.err_avg < 0.25
    }
}

/// Composite (FM demodulator output at `rate`) -> stereo frames at rate / `factor`.
pub struct StereoDecoder {
    pll: PilotPll,
    mono_lp: DecimFir<f32>,
    side_lp: DecimFir<f32>,
    pub enabled: bool,
    blend: f32, // 0 mono .. 1 stereo, ramps
    step: f32,
}

impl StereoDecoder {
    pub fn new(rate: f64, factor: usize) -> Self {
        // 15 kHz audio, the 19 kHz pilot and 38 kHz products in the stop band
        let taps = lowpass(16_875.0 / rate, 3_750.0 / rate);
        Self {
            pll: PilotPll::new(rate),
            mono_lp: DecimFir::new(taps.clone(), factor),
            side_lp: DecimFir::new(taps, factor),
            enabled: true,
            blend: 0.0,
            step: (factor as f64 / (rate * 0.05)) as f32, // 50 ms mono <-> stereo fade
        }
    }

    pub fn locked(&self) -> bool {
        self.pll.locked()
    }

    pub fn pilot_level(&self) -> f32 {
        self.pll.level()
    }

    pub fn process(&mut self, composite: &[f32], out: &mut Vec<Frame>) {
        let mut side = Vec::with_capacity(composite.len());
        for &x in composite {
            side.push(x * 2.0 * self.pll.step(x));
        }
        let (mut m, mut s) = (Vec::new(), Vec::new());
        self.mono_lp.process(composite, &mut m);
        self.side_lp.process(&side, &mut s);
        let want = if self.enabled && self.pll.locked() { 1.0 } else { 0.0 };
        for (m, s) in m.into_iter().zip(s) {
            self.blend = if want > self.blend { (self.blend + self.step).min(1.0) } else { (self.blend - self.step).max(0.0) };
            let s = s * self.blend;
            out.push(Frame::new(m + s, m - s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Broadcast multiplex: 0.45 (L+R) + 0.45 (L-R) sin(2φ) + 0.1 sin(φ), as FM demod output
    fn composite(rate: f64, l: impl Fn(f64) -> f64, r: impl Fn(f64) -> f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f64 / rate;
                let phi = 2.0 * PI * 19_000.0 * t;
                (0.45 * (l(t) + r(t)) + 0.45 * (l(t) - r(t)) * (2.0 * phi).sin() + 0.1 * phi.sin()) as f32
            })
            .collect()
    }

    #[test]
    fn separates_left_from_right() {
        let rate = 240_000.0;
        let x = composite(rate, |t| (2.0 * PI * 1000.0 * t).sin(), |_| 0.0, 240_000 * 2);
        let mut d = StereoDecoder::new(rate, 5);
        let mut out = Vec::new();
        for chunk in x.chunks(4096) {
            d.process(chunk, &mut out);
        }
        assert!(d.locked(), "pilot not locked, level {}", d.pilot_level());
        let tail = &out[48_000..];
        let p = |f: fn(&Frame) -> f32| (tail.iter().map(|x| f(x) * f(x)).sum::<f32>() / tail.len() as f32).sqrt();
        let (l, r) = (p(|x| x.l), p(|x| x.r));
        let sep = 20.0 * (l / r.max(1e-9)).log10();
        assert!(sep > 25.0, "separation {sep:.1} dB (L {l}, R {r})");
        assert!((l - 0.9 * 0.7071).abs() < 0.05, "left level {l}");
    }

    #[test]
    fn falls_back_to_mono_without_pilot() {
        let rate = 240_000.0;
        let x: Vec<f32> = (0..240_000).map(|i| (0.5 * (2.0 * PI * 1000.0 * i as f64 / rate).sin()) as f32).collect();
        let mut d = StereoDecoder::new(rate, 5);
        let mut out = Vec::new();
        d.process(&x, &mut out);
        assert!(!d.locked());
        assert!(out[24_000..].iter().all(|f| (f.l - f.r).abs() < 1e-6));
    }
}
