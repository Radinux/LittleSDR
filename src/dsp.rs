//! DSP building blocks. Every block keeps its own state between calls, so a chain can be
//! rebuilt piece by piece (e.g. only the front end on a sample-rate change) while the others
//! carry on without a click.

use std::f64::consts::PI;
use std::ops::{Add, AddAssign, Mul, Sub};

/// Complex sample, single precision.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct C32 {
    pub re: f32,
    pub im: f32,
}

impl C32 {
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }
    pub fn norm_sqr(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
    pub fn arg(self) -> f32 {
        self.im.atan2(self.re)
    }
}

impl Add for C32 {
    type Output = C32;
    fn add(self, o: C32) -> C32 {
        C32::new(self.re + o.re, self.im + o.im)
    }
}
impl AddAssign for C32 {
    fn add_assign(&mut self, o: C32) {
        self.re += o.re;
        self.im += o.im;
    }
}
impl Sub for C32 {
    type Output = C32;
    fn sub(self, o: C32) -> C32 {
        C32::new(self.re - o.re, self.im - o.im)
    }
}
impl Mul for C32 {
    type Output = C32;
    fn mul(self, o: C32) -> C32 {
        C32::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl Mul<f32> for C32 {
    type Output = C32;
    fn mul(self, k: f32) -> C32 {
        C32::new(self.re * k, self.im * k)
    }
}

/// What the generic filters work on: real audio or complex IQ.
pub trait Sample: Copy + Default + Add<Output = Self> + Sub<Output = Self> + Mul<f32, Output = Self> {}
impl Sample for f32 {}
impl Sample for C32 {}

/// Low-pass FIR taps, Blackman window, unity gain at DC.
/// `cutoff` and `transition` are normalized to the sample rate (0..0.5).
pub fn lowpass(cutoff: f64, transition: f64) -> Vec<f32> {
    // Blackman: ~5.5 / transition taps for ~74 dB stopband; odd length, capped
    let mut n = ((5.5 / transition.max(1e-4)).ceil() as usize) | 1;
    n = n.clamp(15, 1023);
    let m = (n - 1) as f64 / 2.0;
    let mut taps: Vec<f64> = (0..n)
        .map(|i| {
            let k = i as f64 - m;
            let sinc = if k == 0.0 { 2.0 * cutoff } else { (2.0 * PI * cutoff * k).sin() / (PI * k) };
            let w = 0.42 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos() + 0.08 * (4.0 * PI * i as f64 / (n - 1) as f64).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    taps.iter_mut().for_each(|t| *t /= sum);
    taps.into_iter().map(|t| t as f32).collect()
}

/// FIR filter + integer decimation, computing only the kept outputs.
pub struct DecimFir<T: Sample> {
    taps: Vec<f32>,
    factor: usize,
    buf: Vec<T>,
    next: usize, // index in buf of the last input sample of the next output
}

impl<T: Sample> DecimFir<T> {
    pub fn new(taps: Vec<f32>, factor: usize) -> Self {
        let n = taps.len();
        Self { taps, factor: factor.max(1), buf: vec![T::default(); n - 1], next: n - 1 }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn process(&mut self, input: &[T], out: &mut Vec<T>) {
        let n = self.taps.len();
        self.buf.extend_from_slice(input);
        let mut i = self.next;
        while i < self.buf.len() {
            let window = &self.buf[i + 1 - n..=i];
            let mut acc = T::default();
            for (s, &t) in window.iter().zip(self.taps.iter()) {
                acc = acc + *s * t;
            }
            out.push(acc);
            i += self.factor;
        }
        let drop = self.buf.len() - (n - 1);
        self.buf.drain(..drop);
        self.next = i - drop;
    }
}

/// Arbitrary-ratio resampler, 4-point Catmull-Rom interpolation. Meant for signals already
/// low-passed well below the output Nyquist (after a DecimFir), or for small ratio trims.
/// The ratio can change at any time without a discontinuity.
pub struct Interp<T: Sample> {
    buf: Vec<T>,
    pos: f64,  // read position in buf
    step: f64, // input samples per output sample
}

impl<T: Sample> Interp<T> {
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        Self { buf: vec![T::default(); 2], pos: 1.0, step: in_rate / out_rate }
    }

    pub fn set_ratio(&mut self, in_rate: f64, out_rate: f64) {
        self.step = in_rate / out_rate;
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    pub fn set_step(&mut self, step: f64) {
        self.step = step;
    }

    pub fn process(&mut self, input: &[T], out: &mut Vec<T>) {
        self.buf.extend_from_slice(input);
        while self.pos + 2.0 < self.buf.len() as f64 {
            let i = self.pos as usize;
            let mu = (self.pos - i as f64) as f32;
            let (y0, y1, y2, y3) = (self.buf[i - 1], self.buf[i], self.buf[i + 1], self.buf[i + 2]);
            let c1 = (y2 - y0) * 0.5;
            let c2 = y0 - y1 * 2.5 + y2 * 2.0 - y3 * 0.5;
            let c3 = (y3 - y0) * 0.5 + (y1 - y2) * 1.5;
            out.push(((c3 * mu + c2) * mu + c1) * mu + y1);
            self.pos += self.step;
        }
        let keep_from = (self.pos as usize).saturating_sub(1);
        self.buf.drain(..keep_from);
        self.pos -= keep_from as f64;
    }
}

/// Frequency shifter (numerically controlled oscillator). The phase is kept across rate and
/// frequency changes, so retuning or a new sample rate does not make the signal jump.
pub struct Nco {
    phase: f64,
    step: f64,
}

impl Nco {
    pub fn new() -> Self {
        Self { phase: 0.0, step: 0.0 }
    }

    /// Shifts by -`offset_hz` (brings a signal at +offset to 0 Hz).
    pub fn set(&mut self, offset_hz: f64, rate: f64) {
        self.step = -2.0 * PI * offset_hz / rate;
    }

    pub fn mix(&mut self, buf: &mut [C32]) {
        if self.step == 0.0 {
            return;
        }
        let (mut s, mut c) = self.phase.sin_cos();
        let (ds, dc) = self.step.sin_cos();
        for z in buf.iter_mut() {
            *z = *z * C32::new(c as f32, s as f32);
            let nc = c * dc - s * ds;
            s = s * dc + c * ds;
            c = nc;
        }
        self.phase = (self.phase + self.step * buf.len() as f64).rem_euclid(2.0 * PI);
    }
}

impl Default for Nco {
    fn default() -> Self {
        Self::new()
    }
}

/// FM discriminator: output 1.0 = `deviation` Hz.
pub struct FmDemod {
    prev: C32,
    gain: f32,
}

impl FmDemod {
    pub fn new(rate: f64, deviation: f64) -> Self {
        Self { prev: C32::new(1.0, 0.0), gain: (rate / (2.0 * PI * deviation)) as f32 }
    }

    pub fn process(&mut self, input: &[C32], out: &mut Vec<f32>) {
        for &z in input {
            out.push((z * self.prev.conj()).arg() * self.gain);
            self.prev = z;
        }
    }
}

/// One-pole de-emphasis (50 µs in Europe, 75 µs in the Americas).
pub struct Deemphasis {
    a: f32,
    y: f32,
}

impl Deemphasis {
    pub fn new(rate: f64, tau: f64) -> Self {
        Self { a: (1.0 - (-1.0 / (rate * tau)).exp()) as f32, y: 0.0 }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            self.y += self.a * (*x - self.y);
            *x = self.y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: f64, f: f64, n: usize) -> Vec<C32> {
        (0..n)
            .map(|i| {
                let p = 2.0 * PI * f * i as f64 / rate;
                C32::new(p.cos() as f32, p.sin() as f32)
            })
            .collect()
    }

    #[test]
    fn lowpass_has_unity_dc_gain() {
        let t = lowpass(0.1, 0.02);
        assert!((t.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert_eq!(t.len() % 2, 1);
    }

    #[test]
    fn decimator_passes_in_band_and_rejects_out_of_band() {
        let rate = 1e6;
        let power = |f: f64| {
            let mut d = DecimFir::<C32>::new(lowpass(0.05, 0.02), 4);
            let mut out = Vec::new();
            d.process(&tone(rate, f, 40_000), &mut out);
            out[out.len() / 2..].iter().map(|z| z.norm_sqr()).sum::<f32>() / (out.len() / 2) as f32
        };
        assert!((power(10e3) - 1.0).abs() < 0.01, "pass band");
        assert!(power(200e3) < 1e-6, "stop band");
    }

    #[test]
    fn decimator_is_block_size_independent() {
        let x = tone(1e6, 12_345.0, 10_000);
        let taps = lowpass(0.1, 0.05);
        let mut a = DecimFir::<C32>::new(taps.clone(), 3);
        let mut b = DecimFir::<C32>::new(taps, 3);
        let (mut oa, mut ob) = (Vec::new(), Vec::new());
        a.process(&x, &mut oa);
        for chunk in x.chunks(777) {
            b.process(chunk, &mut ob);
        }
        assert_eq!(oa.len(), ob.len());
        assert!(oa.iter().zip(&ob).all(|(p, q)| (*p - *q).norm_sqr() < 1e-10));
    }

    #[test]
    fn interp_produces_the_right_number_of_samples() {
        let mut r = Interp::<f32>::new(256_000.0, 240_000.0);
        let mut out = Vec::new();
        for _ in 0..10 {
            r.process(&vec![0.5; 25_600], &mut out);
        }
        assert!((out.len() as i64 - 240_000).abs() <= 2, "{}", out.len());
    }

    #[test]
    fn fm_demod_measures_frequency() {
        let rate = 240_000.0;
        let mut d = FmDemod::new(rate, 75_000.0);
        let mut out = Vec::new();
        d.process(&tone(rate, 37_500.0, 1000), &mut out);
        assert!((out[500] - 0.5).abs() < 1e-3, "{}", out[500]);
    }

    #[test]
    fn nco_shifts_a_tone_to_dc() {
        let rate = 1e6;
        let mut x = tone(rate, 100e3, 1000);
        let mut n = Nco::new();
        n.set(100e3, rate);
        n.mix(&mut x);
        assert!((x[999].re - 1.0).abs() < 1e-3 && x[999].im.abs() < 1e-3);
    }
}
