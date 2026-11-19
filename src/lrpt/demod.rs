//! QPSK / OQPSK demodulator for LRPT, at any input rate of at least ~2.2 samples per symbol.
//!
//! Matched root-raised-cosine filter, then strobes placed by a symbol clock and read by cubic
//! interpolation (no resampling needed), automatic gain, a Costas loop for the carrier and a
//! decision-directed Mueller-Mueller loop for the symbol clock. OQPSK is read at half-symbol
//! strobes: I on one, Q on the next. Output: soft symbols (I, Q) as i8, negative = bit 1.
//! The 4-fold phase ambiguity (and the I/Q pairing in OQPSK) is left to the frame sync.

use crate::dsp::{DecimFir, C32};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

const ACQ_N: usize = 8192;

pub const SYMBOL_RATE: f64 = 72_000.0;
const ROLLOFF: f64 = 0.6;

/// Root-raised-cosine taps, `sps` samples per symbol, `span` symbols each side, unity DC gain
pub fn rrc_taps(sps: f64, alpha: f64, span: usize) -> Vec<f32> {
    let n = (span as f64 * sps).round() as isize;
    let mut taps: Vec<f64> = (-n..=n)
        .map(|i| {
            let t = i as f64 / sps;
            if t == 0.0 {
                1.0 - alpha + 4.0 * alpha / PI
            } else if (1.0 - (4.0 * alpha * t).powi(2)).abs() < 1e-9 {
                alpha / 2f64.sqrt() * ((1.0 + 2.0 / PI) * (PI / (4.0 * alpha)).sin() + (1.0 - 2.0 / PI) * (PI / (4.0 * alpha)).cos())
            } else {
                ((PI * t * (1.0 - alpha)).sin() + 4.0 * alpha * t * (PI * t * (1.0 + alpha)).cos())
                    / (PI * t * (1.0 - (4.0 * alpha * t).powi(2)))
            }
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    taps.iter_mut().for_each(|t| *t /= sum);
    taps.into_iter().map(|t| t as f32).collect()
}

pub struct Demod {
    pub oqpsk: bool,
    rrc: DecimFir<C32>,
    buf: Vec<C32>,
    /// Next strobe, as a fractional index into `buf`
    t: f64,
    /// Strobe spacing (one symbol in QPSK, half a symbol in OQPSK), and its nominal value
    step: f64,
    step0: f64,
    gain: f32,
    // Carrier loop (per strobe)
    phase: f32,
    freq: f32,
    ca: f32,
    cb: f32,
    freq_max: f32,
    // Clock loop
    ta: f64,
    tb: f64,
    clock: f64, // integrator (fraction of the nominal step)
    // OQPSK strobe parity and held I
    second: bool,
    held_i: f32,
    prev_i: f32,
    prev_q: f32,
    /// Lock indicator: ~1 locked, ~0.5 or below unlocked
    pub lock: f32,
    err_avg: f32,
    pub symbols: u64,
    /// Symbol clock loop enabled (tests)
    pub clock_loop: bool,
    // Acquisition: averaged spectrum of the input; its centre of gravity is the carrier
    rate: f64,
    fft: Arc<dyn Fft<f32>>,
    acq_buf: Vec<Complex32>,
    acq_psd: Vec<f32>,
    acq_frames: usize,
    /// Strobes left before the loop is judged again after a new estimate
    holdoff: u64,
    pub acquisitions: u32,
    // Eye shape: E|x| and E[x^2] of the output components. A clean eye has |x| grouped
    // (E|x|^2 / E[x^2] near 1); reading OQPSK branches mid-transition scatters it
    eye_m1: f32,
    eye_m2: f32,
    eye_wait: u64,
    pub flips: u32,
}

impl Demod {
    pub fn new(rate: f64, oqpsk: bool) -> Demod {
        let sps = rate / SYMBOL_RATE;
        let step0 = if oqpsk { sps / 2.0 } else { sps };
        // Loop bandwidths per strobe: carrier ~ 100 Hz, clock very narrow (crystal drift only)
        let strobe_rate = SYMBOL_RATE * if oqpsk { 2.0 } else { 1.0 };
        let (ca, cb) = loop_gains(0.707, 2.0 * PI * 100.0 / strobe_rate);
        let (ta, tb) = loop_gains(1.0, 2.0 * PI * 20.0 / strobe_rate);
        Demod {
            oqpsk,
            rrc: DecimFir::new(rrc_taps(sps, ROLLOFF, 8), 1),
            buf: vec![C32::default(); 4],
            t: 2.0,
            step: step0,
            step0,
            gain: 1.0,
            phase: 0.0,
            freq: 0.0,
            ca: ca as f32,
            cb: cb as f32,
            freq_max: (2.0 * PI * 8_000.0 / strobe_rate) as f32,
            ta,
            tb,
            clock: 0.0,
            second: false,
            held_i: 0.0,
            prev_i: 0.0,
            prev_q: 0.0,
            lock: 0.0,
            err_avg: 1.0,
            symbols: 0,
            clock_loop: true,
            rate,
            fft: FftPlanner::new().plan_fft_forward(ACQ_N),
            acq_buf: Vec::with_capacity(ACQ_N),
            acq_psd: vec![0.0; ACQ_N],
            acq_frames: 0,
            holdoff: 0,
            acquisitions: 0,
            eye_m1: 0.0,
            eye_m2: 1.0,
            eye_wait: 0,
            flips: 0,
        }
    }

    /// Carrier offset currently tracked (Hz)
    pub fn freq_hz(&self) -> f64 {
        self.freq as f64 * SYMBOL_RATE * if self.oqpsk { 2.0 } else { 1.0 } / (2.0 * PI)
    }

    /// Starting carrier estimate (Hz), e.g. from the spectrum
    pub fn set_freq_hz(&mut self, hz: f64) {
        let strobe_rate = SYMBOL_RATE * if self.oqpsk { 2.0 } else { 1.0 };
        self.freq = (2.0 * PI * hz / strobe_rate) as f32;
    }

    fn interp(&self, t: f64) -> C32 {
        let i = t.floor() as usize;
        let mu = (t - i as f64) as f32;
        let (y0, y1, y2, y3) = (self.buf[i - 1], self.buf[i], self.buf[i + 1], self.buf[i + 2]);
        let c1 = (y2 - y0) * 0.5;
        let c2 = y0 - y1 * 2.5 + y2 * 2.0 - y3 * 0.5;
        let c3 = (y3 - y0) * 0.5 + (y1 - y2) * 1.5;
        ((c3 * mu + c2) * mu + c1) * mu + y1
    }

    /// Carrier estimate from the 4th power of the signal: raising (O)QPSK to the 4th power
    /// strips the modulation and leaves a line at 4 x the carrier offset (checked on a real
    /// Meteor-M2-4 pass: 30 dB above the floor). Precise to a few Hz after 0.2 s.
    fn acquire(&mut self, input: &[C32]) {
        if self.lock >= 0.6 || self.holdoff > 0 {
            return;
        }
        for z in input {
            let k = self.acq_buf.len();
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * k as f32 / ACQ_N as f32).cos();
            let z2 = *z * *z;
            let z4 = z2 * z2;
            // Normalized by |z|^3: keeps the phase, tames the dynamic range
            let m = z.norm_sqr().sqrt().max(1e-12);
            let z4 = z4 * (1.0 / (m * m * m));
            self.acq_buf.push(Complex32::new(z4.re * w, z4.im * w));
            if self.acq_buf.len() == ACQ_N {
                self.fft.process(&mut self.acq_buf);
                for (p, b) in self.acq_psd.iter_mut().zip(&self.acq_buf) {
                    *p += b.norm_sqr();
                }
                self.acq_buf.clear();
                self.acq_frames += 1;
            }
        }
        if (self.acq_frames * ACQ_N) as f64 >= self.rate * 0.2 {
            let bin = self.rate / ACQ_N as f64;
            let f = |k: usize| if k < ACQ_N / 2 { k as f64 * bin } else { (k as f64 - ACQ_N as f64) * bin };
            let span = 4.0 * self.freq_max_hz();
            let mut sorted = self.acq_psd.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let median = sorted[ACQ_N / 2].max(1e-20);
            let (mut best, mut bk) = (0.0f32, 0usize);
            for k in 0..ACQ_N {
                if f(k).abs() <= span && self.acq_psd[k] > best {
                    best = self.acq_psd[k];
                    bk = k;
                }
            }
            if best > 20.0 * median {
                // Interpolate the peak between bins (parabola on the log power)
                let l = |k: usize| (self.acq_psd[k % ACQ_N].max(1e-20) as f64).ln();
                let (a, b, c) = (l(bk + ACQ_N - 1), l(bk), l(bk + 1));
                let d = if a - 2.0 * b + c != 0.0 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
                let est = (f(bk) + d * bin) / 4.0;
                if std::env::var_os("LRPT_DEBUG").is_some() {
                    eprintln!("acquire: {:.1} Hz, line {:.0} dB (was {:.0}, lock {:.2}, eye {:.2})", est, 10.0 * (best / median).log10(), self.freq_hz(), self.lock, self.eye());
                }
                if (est - self.freq_hz()).abs() > 20.0 {
                    self.set_freq_hz(est);
                }
                self.acquisitions += 1;
                self.err_avg = 0.3; // judge the loop afresh
                self.holdoff = (SYMBOL_RATE * 0.5) as u64; // let it pull in
            }
            self.acq_psd.iter_mut().for_each(|p| *p = 0.0);
            self.acq_frames = 0;
        }
    }

    fn freq_max_hz(&self) -> f64 {
        self.freq_max as f64 * SYMBOL_RATE * if self.oqpsk { 2.0 } else { 1.0 } / (2.0 * PI)
    }

    /// Frames are decoding: the carrier is right, don't go looking for it again for a while
    pub fn confirm_lock(&mut self) {
        self.holdoff = self.holdoff.max(SYMBOL_RATE as u64);
    }

    pub fn process(&mut self, input: &[C32], out: &mut Vec<i8>) {
        self.acquire(input);
        self.rrc.process(input, &mut self.buf);
        while self.t + 3.0 < self.buf.len() as f64 {
            let s = self.interp(self.t) * self.gain;
            // AGC: mean magnitude -> 1
            let m = s.norm_sqr().sqrt();
            self.gain *= 1.0 + 0.0005 * (1.0 - m);
            self.gain = self.gain.clamp(1e-3, 1e7);
            // Carrier
            let (sn, cs) = self.phase.sin_cos();
            let r = s * C32::new(cs, -sn);
            self.phase += self.freq;
            let timing_err;
            let mut carrier_err = None;
            if !self.oqpsk {
                let (i, q) = (r.re, r.im);
                carrier_err = Some(i.signum() * q - q.signum() * i);
                timing_err = mm(self.prev_i, i) + mm(self.prev_q, q);
                self.prev_i = i;
                self.prev_q = q;
                push(out, i, q);
                self.symbols += 1;
            } else if !self.second {
                self.held_i = r.re;
                timing_err = mm(self.prev_i, r.re);
                self.prev_i = r.re;
            } else {
                let (i, q) = (self.held_i, r.im);
                carrier_err = Some(i.signum() * q - q.signum() * i);
                timing_err = mm(self.prev_q, q);
                self.prev_q = q;
                push(out, i, q);
                self.symbols += 1;
                self.watch_eye(i, q);
            }
            if self.oqpsk {
                self.second = !self.second;
            }
            if let Some(e) = carrier_err {
                let e = e.clamp(-1.0, 1.0);
                self.phase += self.ca * e;
                self.freq += self.cb * e;
                // Lock: the phase error stays small when locked
                self.err_avg += 0.001 * (e.abs() - self.err_avg);
                self.lock = 1.0 - self.err_avg;
                self.holdoff = self.holdoff.saturating_sub(1);
                self.freq = self.freq.clamp(-self.freq_max, self.freq_max);
                self.phase = self.phase.rem_euclid(2.0 * std::f32::consts::PI);
            }
            // Clock: Mueller-Mueller, positive error = strobe early
            let e = if self.clock_loop { timing_err.clamp(-1.0, 1.0) } else { 0.0 };
            self.clock = (self.clock + self.tb * e).clamp(-0.005, 0.005);
            self.t += self.step0 * (1.0 + self.clock) + self.ta * e * self.step0;
            self.step = self.step0 * (1.0 + self.clock);
        }
        // Keep what the next strobes still need
        let keep_from = (self.t as usize).saturating_sub(2);
        if keep_from > 4096 {
            self.buf.drain(..keep_from);
            self.t -= keep_from as f64;
        }
    }

    /// OQPSK false lock: carrier locked 90 degrees off, each branch read mid-transition.
    /// Seen as a scattered eye while the carrier loop is otherwise happy: turn a quarter.
    fn watch_eye(&mut self, i: f32, q: f32) {
        for x in [i.abs(), q.abs()] {
            self.eye_m1 += 0.002 * (x - self.eye_m1);
            self.eye_m2 += 0.002 * (x * x - self.eye_m2);
        }
        self.eye_wait = self.eye_wait.saturating_sub(1);
        if self.oqpsk && self.eye_wait == 0 && self.lock > 0.65 && self.eye() < 0.85 {
            self.phase += std::f32::consts::FRAC_PI_2;
            self.flips += 1;
            self.eye_m1 = 0.0;
            self.eye_m2 = 1.0;
            self.eye_wait = 5_000;
        }
    }

    /// Eye opening, ~1 clean, ~0.6 closed
    pub fn eye(&self) -> f32 {
        self.eye_m1 * self.eye_m1 / self.eye_m2.max(1e-9)
    }

    /// Symbol clock relative to nominal (ppm)
    pub fn clock_ppm(&self) -> f64 {
        (self.step / self.step0 - 1.0) * 1e6
    }
}

#[inline]
fn mm(prev: f32, cur: f32) -> f64 {
    (prev.signum() * cur - cur.signum() * prev) as f64
}

#[inline]
fn push(out: &mut Vec<i8>, i: f32, q: f32) {
    out.push((i * 60.0).clamp(-127.0, 127.0) as i8);
    out.push((q * 60.0).clamp(-127.0, 127.0) as i8);
}

fn loop_gains(damp: f64, bw: f64) -> (f64, f64) {
    let d = 1.0 + 2.0 * damp * bw + bw * bw;
    (4.0 * damp * bw / d, 4.0 * bw * bw / d)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rng(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    pub(crate) fn gauss(seed: &mut u32) -> f32 {
        let u1 = (rng(seed) as f32 / u32::MAX as f32).max(1e-9);
        let u2 = rng(seed) as f32 / u32::MAX as f32;
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }

    /// Modulates hard bits (pairs I, Q; 1 -> negative) into RRC-shaped (O)QPSK at `rate`,
    /// with a carrier offset, a symbol clock error and noise (SNR per symbol, dB)
    pub(crate) fn modulate(bits: &[u8], rate: f64, oqpsk: bool, offset_hz: f64, clock_ppm: f64, snr_db: f32, seed: &mut u32) -> Vec<C32> {
        let sym_rate = SYMBOL_RATE * (1.0 + clock_ppm * 1e-6);
        let sps = rate / sym_rate;
        let nsym = bits.len() / 2;
        let taps = rrc_taps(16.0, ROLLOFF, 8); // shaping at 16x, then read at `rate`
        let half = taps.len() as isize / 2;
        // Impulses at 16x oversampling, then filter, then pick samples by linear interpolation
        let over = 16usize;
        let len16 = nsym * over + taps.len() + over;
        let mut i_imp = vec![0f32; len16];
        let mut q_imp = vec![0f32; len16];
        for k in 0..nsym {
            let a = |b: u8| if b == 1 { -1.0 } else { 1.0 };
            i_imp[k * over] = a(bits[2 * k]);
            q_imp[k * over + if oqpsk { over / 2 } else { 0 }] = a(bits[2 * k + 1]);
        }
        let conv = |x: &[f32]| -> Vec<f32> {
            let mut y = vec![0f32; x.len()];
            for (n, yn) in y.iter_mut().enumerate() {
                let mut acc = 0.0;
                for (j, &t) in taps.iter().enumerate() {
                    let idx = n as isize + half - j as isize;
                    if idx >= 0 && (idx as usize) < x.len() {
                        acc += x[idx as usize] * t;
                    }
                }
                *yn = acc * over as f32;
            }
            y
        };
        let (iw, qw) = (conv(&i_imp), conv(&q_imp));
        let n_out = (nsym as f64 * sps) as usize;
        let sigma = (10f32.powf(-snr_db / 10.0) / 2.0).sqrt() * (sps as f32).sqrt();
        (0..n_out)
            .map(|n| {
                let pos = n as f64 / sps * over as f64;
                let k = pos.floor() as usize;
                let f = (pos - k as f64) as f32;
                let i = iw[k] * (1.0 - f) + iw[(k + 1).min(iw.len() - 1)] * f;
                let q = qw[k] * (1.0 - f) + qw[(k + 1).min(qw.len() - 1)] * f;
                let p = 2.0 * PI * offset_hz * n as f64 / rate;
                let z = C32::new(i, q) * C32::new(p.cos() as f32, p.sin() as f32);
                z * 0.3 + C32::new(gauss(seed), gauss(seed)) * (0.3 * sigma)
            })
            .collect()
    }

    /// Agreement of `soft` (from soft index `from`) with `bits`: the alignment (any soft
    /// offset, odd ones included) and the phase / mirror transform are searched with a probe,
    /// then the fraction of matching hard decisions is measured over the rest
    pub(crate) fn best_match(soft: &[i8], bits: &[u8], from: usize) -> f32 {
        let probe = 512usize;
        let mut best = (0usize, 0isize, 0usize);
        for d in -2000isize..2000 {
            for t in 0..8 {
                let mut ok = 0;
                for k in 0..probe / 2 {
                    let si = from + 2 * k;
                    let bi = si as isize - d;
                    if bi < 0 || bi as usize + 1 >= bits.len() || si + 1 >= soft.len() {
                        continue;
                    }
                    let (x, y) = transform(soft[si], soft[si + 1], t);
                    let bi = bi as usize;
                    ok += ((x < 0) == (bits[bi] == 1)) as usize + ((y < 0) == (bits[bi + 1] == 1)) as usize;
                }
                if ok > best.0 {
                    best = (ok, d, t);
                }
            }
        }
        let (_, d, t) = best;
        let (mut ok, mut n) = (0usize, 0usize);
        let mut si = from;
        while si + 1 < soft.len() {
            let bi = si as isize - d;
            if bi >= 0 && (bi as usize) + 1 < bits.len() {
                let (x, y) = transform(soft[si], soft[si + 1], t);
                let bi = bi as usize;
                ok += ((x < 0) == (bits[bi] == 1)) as usize + ((y < 0) == (bits[bi + 1] == 1)) as usize;
                n += 2;
            }
            si += 2;
        }
        ok as f32 / n.max(1) as f32
    }

    fn transform(a: i8, b: i8, t: usize) -> (i32, i32) {
        let (a, b) = (a as i32, b as i32);
        let (a, b) = if t >= 4 { (b, a) } else { (a, b) };
        match t % 4 {
            0 => (a, b),
            1 => (b, -a),
            2 => (-a, -b),
            _ => (-b, a),
        }
    }

    fn run(oqpsk: bool, offset: f64) -> f32 {
        let mut seed = 99u32;
        let bits: Vec<u8> = (0..2 * 60_000).map(|_| (rng(&mut seed) & 1) as u8).collect();
        let rate = 187_500.0;
        let x = modulate(&bits, rate, oqpsk, offset, 30.0, 12.0, &mut seed);
        let mut d = Demod::new(rate, oqpsk);
        let mut soft = Vec::new();
        for ch in x.chunks(4096) {
            d.process(ch, &mut soft);
        }
        assert!(d.lock > 0.6, "not locked: {} (freq {:.0} Hz)", d.lock, d.freq_hz());
        // Compare the last third (acquisition takes 0.2 s + pull-in) with the bits sent
        let from = soft.len() * 2 / 3 & !1;
        best_match(&soft, &bits, from).max(best_match(&soft, &bits, from + 1))
    }

    #[test]
    #[ignore]
    fn debug_oqpsk() {
        for (clock, off) in [(false, 0.0), (false, -900.0), (true, 0.0)] {
            let mut seed = 99u32;
            let bits: Vec<u8> = (0..2 * 20_000).map(|_| (rng(&mut seed) & 1) as u8).collect();
            let x = modulate(&bits, 187_500.0, true, off, 0.0, 20.0, &mut seed);
            let mut d = Demod::new(187_500.0, true);
            d.clock_loop = clock;
            let mut soft = Vec::new();
            for (i, ch) in x.chunks(2048).enumerate() {
                d.process(ch, &mut soft);
                if i % 8 == 0 {
                    println!("clock {clock} off {off}: lock {:.2} freq {:.0} ppm {:.0} eye {:.2} flips {} acq {}", d.lock, d.freq_hz(), d.clock_ppm(), d.eye(), d.flips, d.acquisitions);
                }
            }
            let from = soft.len() / 2 & !1;
            println!("  match {:.3} / {:.3}", best_match(&soft, &bits, from), best_match(&soft, &bits, from + 1));
        }
    }

    #[test]
    fn qpsk_locks_and_reads_the_symbols() {
        let ok = run(false, 600.0);
        assert!(ok > 0.99, "QPSK symbol agreement {ok}");
    }

    #[test]
    fn oqpsk_locks_and_reads_the_symbols() {
        let ok = run(true, -900.0);
        assert!(ok > 0.99, "OQPSK symbol agreement {ok}");
    }
}
