//! Switchable audio processing at 48 kHz, on stereo frames.
//!
//! Every processor runs all the time, enabled or not; its switch only crossfades (10 ms)
//! between the processed and the direct sound. Turning a filter on is therefore instant and
//! click-free, and adaptive ones (noise estimate, notch) are already converged. The direct
//! sound is delayed by the processor's latency, so switching never shifts the sound in time.

use crate::dsp::Frame;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

pub const RATE: f32 = 48_000.0;
const XFADE: f32 = 480.0; // 10 ms

pub trait AudioProc: Send {
    fn process(&mut self, buf: &mut [Frame]);
    /// Delay added by the processor (samples)
    fn latency(&self) -> usize {
        0
    }
    /// Live parameter change; false if the key is unknown
    fn set(&mut self, _key: &str, _value: f32) -> bool {
        false
    }
    fn describe(&self) -> String {
        String::new()
    }
}

// ---------------------------------------------------------------- biquad (RBJ cookbook)

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z: [[f32; 2]; 2], // per channel, transposed direct form II
}

impl Biquad {
    fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Biquad {
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z: [[0.0; 2]; 2] }
    }
    fn w(f: f32) -> (f32, f32) {
        let w0 = 2.0 * PI * f.clamp(1.0, RATE * 0.49) / RATE;
        (w0.cos(), w0.sin())
    }
    pub fn lowpass(f: f32, q: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = s / (2.0 * q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    pub fn highpass(f: f32, q: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = s / (2.0 * q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    /// Band-pass, 0 dB at the center
    pub fn bandpass(f: f32, q: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = s / (2.0 * q);
        Self::norm(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    pub fn notch(f: f32, q: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = s / (2.0 * q);
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    pub fn low_shelf(f: f32, db: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = 10f32.powf(db / 40.0);
        let al = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * al;
        Self::norm(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }
    pub fn high_shelf(f: f32, db: f32) -> Biquad {
        let (c, s) = Self::w(f);
        let a = 10f32.powf(db / 40.0);
        let al = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * al;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        )
    }
    /// New coefficients, same state: retuning does not reset the filter
    pub fn retune(&mut self, other: Biquad) {
        let z = self.z;
        *self = other;
        self.z = z;
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b0 * x + z[0];
        z[0] = self.b1 * x - self.a1 * y + z[1];
        z[1] = self.b2 * x - self.a2 * y;
        y
    }
    pub fn process(&mut self, buf: &mut [Frame]) {
        for f in buf.iter_mut() {
            f.l = self.run(0, f.l);
            f.r = self.run(1, f.r);
        }
    }
}

// ---------------------------------------------------------------- filters

/// 4th-order Butterworth high-pass: hum, CTCSS tones (< 300 Hz) of repeaters
pub struct HighPass {
    hz: f32,
    st: [Biquad; 2],
}
impl HighPass {
    pub fn new(hz: f32) -> Self {
        Self { hz, st: [Biquad::highpass(hz, 0.5412), Biquad::highpass(hz, 1.3066)] }
    }
}
impl AudioProc for HighPass {
    fn process(&mut self, buf: &mut [Frame]) {
        self.st.iter_mut().for_each(|b| b.process(buf));
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "hz" {
            return false;
        }
        self.hz = v.clamp(20.0, 2000.0);
        self.st[0].retune(Biquad::highpass(self.hz, 0.5412));
        self.st[1].retune(Biquad::highpass(self.hz, 1.3066));
        true
    }
    fn describe(&self) -> String {
        format!("{:.0} Hz", self.hz)
    }
}

/// 4th-order Butterworth low-pass: hiss above the voice band
pub struct LowPass {
    hz: f32,
    st: [Biquad; 2],
}
impl LowPass {
    pub fn new(hz: f32) -> Self {
        Self { hz, st: [Biquad::lowpass(hz, 0.5412), Biquad::lowpass(hz, 1.3066)] }
    }
}
impl AudioProc for LowPass {
    fn process(&mut self, buf: &mut [Frame]) {
        self.st.iter_mut().for_each(|b| b.process(buf));
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "hz" {
            return false;
        }
        self.hz = v.clamp(500.0, 20_000.0);
        self.st[0].retune(Biquad::lowpass(self.hz, 0.5412));
        self.st[1].retune(Biquad::lowpass(self.hz, 1.3066));
        true
    }
    fn describe(&self) -> String {
        format!("{:.0} Hz", self.hz)
    }
}

/// Narrow band-pass for CW: only the morse note goes through
pub struct CwPeak {
    hz: f32,
    width: f32,
    st: [Biquad; 2],
}
impl CwPeak {
    pub fn new(hz: f32, width: f32) -> Self {
        let mut p = Self { hz, width, st: [Biquad::default(); 2] };
        p.tune(false);
        p
    }
    fn tune(&mut self, keep: bool) {
        // Two stages: steeper skirts, ~width at -3 dB
        let q = self.hz / (self.width * 1.55);
        let b = Biquad::bandpass(self.hz, q);
        for s in self.st.iter_mut() {
            if keep {
                s.retune(b)
            } else {
                *s = b
            }
        }
    }
}
impl AudioProc for CwPeak {
    fn process(&mut self, buf: &mut [Frame]) {
        self.st.iter_mut().for_each(|b| b.process(buf));
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        match key {
            "hz" => self.hz = v.clamp(200.0, 2000.0),
            "width" => self.width = v.clamp(50.0, 1000.0),
            _ => return false,
        }
        self.tune(true);
        true
    }
    fn describe(&self) -> String {
        format!("{:.0} Hz ± {:.0}", self.hz, self.width / 2.0)
    }
}

/// Manual notch: removes one frequency (two cascaded notches for depth)
pub struct Notch {
    hz: f32,
    st: [Biquad; 2],
}
impl Notch {
    pub fn new(hz: f32) -> Self {
        Self { hz, st: [Biquad::notch(hz, 8.0); 2] }
    }
}
impl AudioProc for Notch {
    fn process(&mut self, buf: &mut [Frame]) {
        self.st.iter_mut().for_each(|b| b.process(buf));
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "hz" {
            return false;
        }
        self.hz = v.clamp(50.0, 15_000.0);
        let b = Biquad::notch(self.hz, 8.0);
        self.st.iter_mut().for_each(|s| s.retune(b));
        true
    }
    fn describe(&self) -> String {
        format!("{:.0} Hz", self.hz)
    }
}

/// Bass / treble shelving equalizer
pub struct Equalizer {
    bass: f32,
    treble: f32,
    lo: Biquad,
    hi: Biquad,
}
impl Equalizer {
    pub fn new() -> Self {
        Self { bass: 0.0, treble: 0.0, lo: Biquad::low_shelf(250.0, 0.0), hi: Biquad::high_shelf(3500.0, 0.0) }
    }
}
impl Default for Equalizer {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioProc for Equalizer {
    fn process(&mut self, buf: &mut [Frame]) {
        self.lo.process(buf);
        self.hi.process(buf);
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        match key {
            "bass" => {
                self.bass = v.clamp(-15.0, 15.0);
                self.lo.retune(Biquad::low_shelf(250.0, self.bass));
            }
            "treble" => {
                self.treble = v.clamp(-15.0, 15.0);
                self.hi.retune(Biquad::high_shelf(3500.0, self.treble));
            }
            _ => return false,
        }
        true
    }
    fn describe(&self) -> String {
        format!("graves {:+.0} dB, aigus {:+.0} dB", self.bass, self.treble)
    }
}

/// Automatic notch: an adaptive line enhancer (normalized LMS) predicts what repeats from the
/// past (steady tones, heterodyne whistles) and subtracts it; speech, which does not repeat
/// over the prediction delay, goes through.
pub struct AutoNotch {
    ch: [Lms; 2],
}
struct Lms {
    w: Vec<f32>,
    hist: Vec<f32>, // ring, TAPS + DELAY
    pos: usize,
    mu: f32,
}
impl Lms {
    const TAPS: usize = 64;
    const DELAY: usize = 64; // 1.3 ms: longer than the speech correlation, shorter than a tone
    fn new() -> Lms {
        Lms { w: vec![0.0; Self::TAPS], hist: vec![0.0; Self::TAPS + Self::DELAY], pos: 0, mu: 0.004 }
    }
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let n = self.hist.len();
        // Reference: the samples DELAY..DELAY+TAPS in the past
        let mut y = 0.0;
        let mut energy = 1e-6;
        for k in 0..Self::TAPS {
            let r = self.hist[(self.pos + n - Self::DELAY - k) % n];
            y += self.w[k] * r;
            energy += r * r;
        }
        let e = x - y;
        let g = self.mu * e / energy;
        for k in 0..Self::TAPS {
            let r = self.hist[(self.pos + n - Self::DELAY - k) % n];
            self.w[k] = self.w[k] * 0.99999 + g * r; // slight leak: no drift
        }
        self.pos = (self.pos + 1) % n;
        self.hist[self.pos] = x;
        e
    }
}
impl AutoNotch {
    pub fn new() -> Self {
        Self { ch: [Lms::new(), Lms::new()] }
    }
}
impl Default for AutoNotch {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioProc for AutoNotch {
    fn process(&mut self, buf: &mut [Frame]) {
        for f in buf.iter_mut() {
            f.l = self.ch[0].run(f.l);
            f.r = self.ch[1].run(f.r);
        }
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "speed" {
            return false;
        }
        let mu = v.clamp(0.0005, 0.05);
        self.ch.iter_mut().for_each(|c| c.mu = mu);
        true
    }
}

/// Spectral noise reduction: short-time FFT (512 points, 50 % overlap), noise floor tracked
/// per frequency bin, decision-directed Wiener gain with a floor (limits "musical noise").
pub struct NoiseReduction {
    ch: [NrChannel; 2],
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    floor: f32,
    reduction_db: f32,
}
struct NrChannel {
    input: VecDeque<f32>,
    count: usize,
    olap: Vec<f32>,
    out: VecDeque<f32>,
    noise: Vec<f32>,
    psm: Vec<f32>,
    gprev: Vec<f32>,
    gamma_prev: Vec<f32>,
    frames: usize,
}
impl NoiseReduction {
    const N: usize = 512;
    const HOP: usize = 256;

    pub fn new(reduction_db: f32) -> Self {
        let mut p = FftPlanner::new();
        let n = Self::N;
        // sqrt-Hann analysis and synthesis: sums to 1 at 50 % overlap
        let win = (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos()).sqrt()).collect();
        let ch = || NrChannel {
            input: std::iter::repeat(0.0).take(n).collect(),
            count: 0,
            olap: vec![0.0; n],
            out: std::iter::repeat(0.0).take(Self::HOP).collect(),
            noise: vec![0.0; n / 2 + 1],
            psm: vec![0.0; n / 2 + 1],
            gprev: vec![1.0; n / 2 + 1],
            gamma_prev: vec![1.0; n / 2 + 1],
            frames: 0,
        };
        let mut nr = Self { ch: [ch(), ch()], fft: p.plan_fft_forward(n), ifft: p.plan_fft_inverse(n), win, floor: 0.0, reduction_db: 0.0 };
        nr.set("db", reduction_db);
        nr
    }

    fn frame(&self, c: &mut NrChannel) {
        let n = Self::N;
        let mut x: Vec<Complex32> = c.input.iter().zip(&self.win).map(|(s, w)| Complex32::new(s * w, 0.0)).collect();
        self.fft.process(&mut x);
        c.frames += 1;
        for k in 0..=n / 2 {
            let p = x[k].norm_sqr();
            c.psm[k] = 0.85 * c.psm[k] + 0.15 * p;
            // Noise floor: follows the smoothed power down at once, up slowly (~1.6 dB/s)
            if c.frames <= 8 || c.psm[k] < c.noise[k] {
                c.noise[k] = if c.frames <= 8 { c.psm[k].max(1e-12) } else { c.psm[k] };
            } else {
                c.noise[k] *= 1.002;
            }
            // The tracked minimum sits below the mean noise power: compensate the bias
            let gamma = p / (c.noise[k] * 1.8).max(1e-12);
            let xi = 0.98 * c.gprev[k] * c.gprev[k] * c.gamma_prev[k] + 0.02 * (gamma - 1.0).max(0.0);
            let g = (xi / (1.0 + xi)).max(self.floor);
            c.gprev[k] = g;
            c.gamma_prev[k] = gamma.min(1e4);
            x[k] = x[k] * g;
            if k > 0 && k < n / 2 {
                x[n - k] = x[k].conj();
            }
        }
        self.ifft.process(&mut x);
        let scale = 1.0 / n as f32;
        for i in 0..n {
            c.olap[i] += x[i].re * scale * self.win[i];
        }
        c.out.extend(c.olap.drain(..Self::HOP));
        c.olap.extend(std::iter::repeat(0.0).take(Self::HOP));
    }

    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let mut c = std::mem::replace(&mut self.ch[ch], NrChannel::placeholder());
        c.input.pop_front();
        c.input.push_back(x);
        c.count += 1;
        if c.count == Self::HOP {
            c.count = 0;
            self.frame(&mut c);
        }
        let y = c.out.pop_front().unwrap_or(0.0);
        self.ch[ch] = c;
        y
    }
}
impl NrChannel {
    fn placeholder() -> NrChannel {
        NrChannel {
            input: VecDeque::new(),
            count: 0,
            olap: Vec::new(),
            out: VecDeque::new(),
            noise: Vec::new(),
            psm: Vec::new(),
            gprev: Vec::new(),
            gamma_prev: Vec::new(),
            frames: 0,
        }
    }
}
impl AudioProc for NoiseReduction {
    fn process(&mut self, buf: &mut [Frame]) {
        for f in buf.iter_mut() {
            f.l = self.run(0, f.l);
            f.r = self.run(1, f.r);
        }
    }
    fn latency(&self) -> usize {
        Self::N
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "db" {
            return false;
        }
        self.reduction_db = v.clamp(3.0, 30.0);
        self.floor = 10f32.powf(-self.reduction_db / 20.0);
        true
    }
    fn describe(&self) -> String {
        format!("-{:.0} dB max", self.reduction_db)
    }
}

/// Audio AGC: steady loudness from weak to strong stations (both channels share one gain)
pub struct Agc {
    target: f32,
    max_gain: f32,
    env: f32,
    gain: f32,
}
impl Agc {
    pub fn new() -> Self {
        Self { target: 0.3, max_gain: 10.0, env: 0.0, gain: 1.0 }
    }
}
impl Default for Agc {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioProc for Agc {
    fn process(&mut self, buf: &mut [Frame]) {
        let att = 1.0 - (-1.0 / (RATE * 0.005)).exp();
        let rel = 1.0 - (-1.0 / (RATE * 0.4)).exp();
        let smooth = 1.0 - (-1.0 / (RATE * 0.05)).exp();
        for f in buf.iter_mut() {
            let p = f.peak();
            self.env += if p > self.env { att } else { rel } * (p - self.env);
            let want = (self.target / self.env.max(1e-4)).clamp(0.1, self.max_gain);
            self.gain += smooth * (want - self.gain);
            *f = *f * self.gain;
        }
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        match key {
            "target" => self.target = v.clamp(0.05, 0.9),
            "max" => self.max_gain = 10f32.powf(v.clamp(0.0, 40.0) / 20.0),
            _ => return false,
        }
        true
    }
    fn describe(&self) -> String {
        format!("gain {:+.1} dB", 20.0 * self.gain.log10())
    }
}

/// Peak limiter: never above the ceiling (instant attack, 80 ms release), then a hard clamp
pub struct Limiter {
    ceiling: f32,
    gain: f32,
}
impl Limiter {
    pub fn new(ceiling: f32) -> Self {
        Self { ceiling, gain: 1.0 }
    }
}
impl AudioProc for Limiter {
    fn process(&mut self, buf: &mut [Frame]) {
        let rel = 1.0 - (-1.0 / (RATE * 0.08)).exp();
        for f in buf.iter_mut() {
            let p = f.peak();
            self.gain += rel * (1.0 - self.gain);
            if p * self.gain > self.ceiling {
                self.gain = self.ceiling / p;
            }
            *f = *f * self.gain;
            f.l = f.l.clamp(-1.0, 1.0);
            f.r = f.r.clamp(-1.0, 1.0);
        }
    }
    fn set(&mut self, key: &str, v: f32) -> bool {
        if key != "ceiling" {
            return false;
        }
        self.ceiling = v.clamp(0.1, 1.0);
        true
    }
}

// ---------------------------------------------------------------- switch and chain

pub struct Switch {
    pub id: &'static str,
    pub label: &'static str,
    proc: Box<dyn AudioProc>,
    pub enabled: bool,
    mix: f32,
    dry: VecDeque<Frame>,
    scratch: Vec<Frame>,
}

impl Switch {
    pub fn new(id: &'static str, label: &'static str, proc: Box<dyn AudioProc>, enabled: bool) -> Switch {
        let lat = proc.latency();
        Switch {
            id,
            label,
            proc,
            enabled,
            mix: if enabled { 1.0 } else { 0.0 },
            dry: std::iter::repeat(Frame::default()).take(lat).collect(),
            scratch: Vec::new(),
        }
    }

    pub fn latency(&self) -> usize {
        self.proc.latency()
    }

    pub fn process(&mut self, buf: &mut [Frame]) {
        // Direct sound, delayed like the processed one
        self.scratch.clear();
        for f in buf.iter() {
            self.dry.push_back(*f);
            self.scratch.push(self.dry.pop_front().unwrap());
        }
        self.proc.process(buf); // always runs: state stays warm
        let target = if self.enabled { 1.0 } else { 0.0 };
        if self.mix == target {
            if target == 0.0 {
                buf.copy_from_slice(&self.scratch);
            }
            return;
        }
        let step = 1.0 / XFADE;
        for (w, d) in buf.iter_mut().zip(&self.scratch) {
            self.mix = if target > self.mix { (self.mix + step).min(1.0) } else { (self.mix - step).max(0.0) };
            *w = *d * (1.0 - self.mix) + *w * self.mix;
        }
    }

    pub fn set(&mut self, key: &str, v: f32) -> bool {
        self.proc.set(key, v)
    }

    pub fn describe(&self) -> String {
        self.proc.describe()
    }
}

/// The processing chain, in a fixed order; filters are switched, never inserted or removed.
pub struct AudioChain {
    pub stages: Vec<Switch>,
}

impl AudioChain {
    pub fn new() -> AudioChain {
        AudioChain {
            stages: vec![
                Switch::new("hp", "Passe-haut (ronflement, CTCSS)", Box::new(HighPass::new(300.0)), false),
                Switch::new("lp", "Passe-bas (souffle aigu)", Box::new(LowPass::new(3000.0)), false),
                Switch::new("cw", "Filtre CW", Box::new(CwPeak::new(700.0, 250.0)), false),
                Switch::new("notch", "Notch manuel", Box::new(Notch::new(1000.0)), false),
                Switch::new("anotch", "Notch automatique", Box::new(AutoNotch::new()), false),
                Switch::new("nr", "Réduction de bruit", Box::new(NoiseReduction::new(12.0)), false),
                Switch::new("eq", "Égaliseur", Box::new(Equalizer::new()), false),
                Switch::new("agc", "AGC audio", Box::new(Agc::new()), false),
                Switch::new("limit", "Limiteur", Box::new(Limiter::new(0.9)), true),
            ],
        }
    }

    pub fn process(&mut self, buf: &mut [Frame]) {
        for s in self.stages.iter_mut() {
            s.process(buf);
        }
    }

    pub fn latency(&self) -> usize {
        self.stages.iter().map(|s| s.latency()).sum()
    }

    pub fn enable(&mut self, id: &str, on: bool) -> bool {
        self.stages.iter_mut().find(|s| s.id == id).map(|s| s.enabled = on).is_some()
    }

    pub fn set(&mut self, id: &str, key: &str, v: f32) -> bool {
        self.stages.iter_mut().find(|s| s.id == id).map_or(false, |s| s.set(key, v))
    }

    pub fn describe(&self) -> String {
        self.stages
            .iter()
            .map(|s| format!("{:7} {} {}  {}", s.id, if s.enabled { "ON " } else { "off" }, s.label, s.describe()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for AudioChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, amp: f32, n: usize) -> Vec<Frame> {
        (0..n).map(|i| Frame::mono(amp * (2.0 * PI * f * i as f32 / RATE).sin())).collect()
    }
    fn rms(buf: &[Frame]) -> f32 {
        (buf.iter().map(|f| f.l * f.l).sum::<f32>() / buf.len() as f32).sqrt()
    }
    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-9).log10()
    }
    /// Deterministic white noise
    fn noise(amp: f32, n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0)
            })
            .collect()
    }

    #[test]
    fn highpass_cuts_hum_and_keeps_voice() {
        let mut hp = HighPass::new(300.0);
        let mut lo = sine(60.0, 0.5, 48_000);
        let mut hi = sine(1000.0, 0.5, 48_000);
        hp.process(&mut lo);
        let mut hp2 = HighPass::new(300.0);
        hp2.process(&mut hi);
        assert!(db(rms(&lo[24_000..]) / 0.3536) < -50.0, "60 Hz: {}", db(rms(&lo[24_000..]) / 0.3536));
        assert!(db(rms(&hi[24_000..]) / 0.3536).abs() < 0.5);
    }

    #[test]
    fn switching_a_filter_is_click_free() {
        // Toggle the high-pass on a 1 kHz tone: the largest sample-to-sample step must stay
        // close to the tone's own (a click would be a big jump)
        let mut s = Switch::new("hp", "", Box::new(HighPass::new(300.0)), false);
        let mut buf = sine(1000.0, 0.8, 48_000);
        let max_step_tone = 0.8 * 2.0 * PI * 1000.0 / RATE;
        for (i, chunk) in buf.chunks_mut(512).enumerate() {
            if i % 10 == 5 {
                s.enabled = !s.enabled;
            }
            s.process(chunk);
        }
        let worst = buf.windows(2).skip(4800).map(|w| (w[1].l - w[0].l).abs()).fold(0.0, f32::max);
        assert!(worst < max_step_tone * 1.3, "step {worst} vs tone {max_step_tone}");
    }

    #[test]
    fn noise_reduction_lowers_the_hiss_and_keeps_the_signal() {
        // Speech-like: 300 ms bursts of tone, 300 ms pauses, over steady hiss
        let n = 48_000 * 4;
        let tone = sine(800.0, 0.3, n);
        let hiss = noise(0.1, n, 7);
        let on = |i: usize| (i / 14_400) % 2 == 0;
        let clean: Vec<f32> = (0..n).map(|i| if on(i) { tone[i].l } else { 0.0 }).collect();
        let mut buf: Vec<Frame> = (0..n).map(|i| Frame::mono(clean[i] + hiss[i])).collect();
        let mut nr = NoiseReduction::new(18.0);
        nr.process(&mut buf);
        let lat = nr.latency();
        // Measure from the 2nd second, away from burst edges (± 20 ms)
        let (mut hiss_in, mut hiss_out, mut sig_in, mut sig_out) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 48_000..n - lat {
            let pos = i % 14_400;
            if !(960..14_400 - 960).contains(&pos) {
                continue;
            }
            let y = buf[i + lat].l as f64;
            if on(i) {
                sig_in += (clean[i] as f64).powi(2);
                sig_out += y * y;
            } else {
                hiss_in += (hiss[i] as f64).powi(2);
                hiss_out += y * y;
            }
        }
        let hiss_drop = 10.0 * (hiss_out / hiss_in).log10();
        let sig_change = 10.0 * (sig_out / sig_in).log10();
        assert!(hiss_drop < -8.0, "hiss only {hiss_drop:.1} dB lower");
        assert!(sig_change.abs() < 2.0, "signal changed by {sig_change:.1} dB");
    }

    #[test]
    fn auto_notch_removes_a_whistle_but_not_noise_like_sound() {
        let n = 48_000 * 2;
        let whistle = sine(1500.0, 0.4, n);
        let voice = noise(0.2, n, 3); // broadband, does not repeat
        let mut buf: Vec<Frame> = whistle.iter().zip(&voice).map(|(w, v)| Frame::mono(w.l + v)).collect();
        let mut an = AutoNotch::new();
        an.process(&mut buf);
        // Tone power left in the output (correlate with the whistle) and broadband power kept
        let tail = 48_000..n;
        let tone_left: f32 = tail.clone().map(|i| buf[i].l * whistle[i].l).sum::<f32>() / tail.len() as f32 / (0.4 * 0.4 / 2.0);
        let total = rms(&buf[tail.clone()]);
        assert!(tone_left.abs() < 0.1, "whistle left: {tone_left}");
        assert!(total > 0.2 * 0.577 * 0.7, "too much of the noise-like sound removed: {total}");
    }

    #[test]
    fn agc_brings_a_weak_signal_up() {
        let mut agc = Agc::new();
        let mut buf = sine(1000.0, 0.03, 48_000 * 2);
        agc.process(&mut buf);
        let peak = buf[48_000..].iter().map(|f| f.peak()).fold(0.0, f32::max);
        assert!((peak - 0.3).abs() < 0.05, "peak {peak}");
    }

    #[test]
    fn limiter_never_exceeds_its_ceiling() {
        let mut lim = Limiter::new(0.9);
        let mut buf = sine(440.0, 3.0, 48_000);
        lim.process(&mut buf);
        assert!(buf.iter().all(|f| f.peak() <= 0.9001));
    }

    #[test]
    fn chain_latency_is_constant_whatever_is_enabled() {
        let mut c = AudioChain::new();
        let lat = c.latency();
        c.enable("nr", true);
        assert_eq!(c.latency(), lat);
        // An impulse comes out exactly `lat` samples later with everything off
        let mut c = AudioChain::new();
        c.enable("limit", false);
        let mut buf = vec![Frame::default(); 4096];
        buf[100] = Frame::mono(0.5);
        c.process(&mut buf);
        let at = buf.iter().position(|f| f.l.abs() > 0.25).unwrap();
        assert_eq!(at, 100 + lat);
    }
}
