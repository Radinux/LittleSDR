//! DAB transmission mode I OFDM receiver: 2.048 MS/s in, soft bits out.
//!
//! A frame (96 ms) is a null symbol, the phase reference symbol (PRS) and 75 data symbols of
//! 1536 pi/4-DQPSK carriers. Synchronisation:
//! - the null symbol (a dip in power) gives the frame start roughly;
//! - the PRS, compared with its known value, gives the frequency offset in whole carriers
//!   (1 kHz steps) and, through the channel impulse response, the exact timing;
//! - the cyclic prefix of every symbol gives the frequency offset within a carrier.
//! The offset is removed by an oscillator in front, then each frame is located again from
//! its PRS, so clock drift is followed.

use crate::dsp::C32;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

pub const RATE: f64 = 2_048_000.0;
pub const TU: usize = 2048;
pub const TG: usize = 504;
pub const TS: usize = TU + TG;
pub const TNULL: usize = 2656;
pub const SYMBOLS: usize = 76; // PRS + 75 data symbols
pub const TF: usize = TNULL + SYMBOLS * TS; // 196608
pub const K: usize = 1536;
pub const BITS_PER_SYMBOL: usize = 2 * K;
/// How far before the strongest path an earlier one is looked for (samples, 1 = 0.49 us)
const PRE_ECHO: usize = 300;
/// Coherent detection: weight of the newest symbol in the channel estimate, and smoothing
/// across carriers (each side). Measured on a weak SFN multiplex (8B), more averaging only
/// made things worse, and plain differential detection stayed best: coherent is off by
/// default.
const CHAN_ALPHA: f32 = 0.5;
const CHAN_SPAN: i32 = 2;

/// Averages the channel estimate over neighbouring carriers (DC carrier excluded)
fn smooth(h: &[Complex32]) -> Vec<Complex32> {
    let mut out = vec![Complex32::default(); TU];
    for k in -768i32..=768 {
        if k == 0 {
            continue;
        }
        let mut acc = Complex32::default();
        let mut n = 0.0;
        for j in (k - CHAN_SPAN)..=(k + CHAN_SPAN) {
            if j != 0 && (-768..=768).contains(&j) {
                acc += h[bin(j)];
                n += 1.0;
            }
        }
        out[bin(k)] = acc / n;
    }
    out
}

/// Largest frequency correction believed (Hz): ~150 ppm at 240 MHz
const MAX_OFFSET: f64 = 35_000.0;
/// The FFT window starts this many samples into the guard interval (echoes arrive late)
const EARLY: isize = 24;

/// PRS phase table, mode I (clause 14.3.2): (k_min, i, n) per block of 32 carriers
const PRS_BLOCKS: [(i32, usize, i32); 48] = [
    (-768, 0, 1), (-736, 1, 2), (-704, 2, 0), (-672, 3, 1), (-640, 0, 3), (-608, 1, 2), (-576, 2, 2), (-544, 3, 3),
    (-512, 0, 2), (-480, 1, 1), (-448, 2, 2), (-416, 3, 3), (-384, 0, 1), (-352, 1, 2), (-320, 2, 3), (-288, 3, 3),
    (-256, 0, 2), (-224, 1, 2), (-192, 2, 2), (-160, 3, 1), (-128, 0, 1), (-96, 1, 3), (-64, 2, 1), (-32, 3, 2),
    (1, 0, 3), (33, 3, 1), (65, 2, 1), (97, 1, 1), (129, 0, 2), (161, 3, 2), (193, 2, 1), (225, 1, 0),
    (257, 0, 2), (289, 3, 2), (321, 2, 3), (353, 1, 3), (385, 0, 0), (417, 3, 2), (449, 2, 1), (481, 1, 3),
    (513, 0, 3), (545, 3, 3), (577, 2, 3), (609, 1, 0), (641, 0, 3), (673, 3, 0), (705, 2, 1), (737, 1, 1),
];
const H: [[i32; 16]; 4] = [
    [0, 2, 0, 0, 0, 0, 1, 1, 2, 0, 0, 0, 2, 2, 1, 1],
    [0, 3, 2, 3, 0, 1, 3, 0, 2, 1, 2, 3, 2, 3, 3, 0],
    [0, 0, 0, 2, 0, 2, 1, 3, 2, 2, 0, 2, 2, 0, 1, 3],
    [0, 1, 2, 1, 0, 3, 3, 2, 2, 3, 2, 1, 2, 1, 3, 2],
];

/// FFT bin of carrier k (-768..=768, no DC)
#[inline]
fn bin(k: i32) -> usize {
    k.rem_euclid(TU as i32) as usize
}

fn prs_phase(k: i32) -> f64 {
    for &(kmin, i, n) in PRS_BLOCKS.iter() {
        if k >= kmin && k < kmin + 32 {
            return PI / 2.0 * (H[i][((k - kmin) % 16) as usize] + n) as f64;
        }
    }
    0.0
}

/// Frequency interleaving (clause 14.6): carrier of soft-bit pair i
fn interleaver() -> Vec<i32> {
    let mut tmp = vec![0i32; TU];
    for i in 1..TU {
        tmp[i] = (13 * tmp[i - 1] + 511) % TU as i32;
    }
    tmp.into_iter().filter(|&t| t != 1024 && (256..=256 + K as i32).contains(&t)).map(|t| t - 1024).collect()
}

/// One decoded frame: soft bits of the 3 FIC symbols and of the 72 MSC symbols
pub struct OfdmFrame {
    pub fic: Vec<i8>,
    pub msc: Vec<i8>,
}

#[derive(Default, Clone, Debug)]
pub struct OfdmStats {
    pub frames: u64,
    pub resyncs: u64,
    pub freq_hz: f64,
    pub snr_db: f32,
}

pub struct Ofdm {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    prs: Vec<Complex32>, // reference spectrum, by FFT bin
    perm: Vec<i32>,
    buf: Vec<C32>,
    /// Start of the next frame's PRS useful part in `buf` (None: searching)
    next: Option<usize>,
    // Frequency correction applied to the input
    freq: f64, // Hz
    phase: f64,
    pub stats: OfdmStats,
    scratch: Vec<Complex32>,
    /// Coherent detection against a smoothed channel estimate (else plain differential,
    /// the default: better on the signals tried)
    pub coherent: bool,
    /// Print the impulse response of the next frames (diagnostics)
    pub debug_cir: bool,
}

impl Default for Ofdm {
    fn default() -> Self {
        Self::new()
    }
}

impl Ofdm {
    pub fn new() -> Ofdm {
        let mut planner = FftPlanner::new();
        let mut prs = vec![Complex32::default(); TU];
        for k in -768i32..=768 {
            if k != 0 {
                let p = prs_phase(k);
                prs[bin(k)] = Complex32::new(p.cos() as f32, p.sin() as f32);
            }
        }
        Ofdm {
            fft: planner.plan_fft_forward(TU),
            ifft: planner.plan_fft_inverse(TU),
            prs,
            perm: interleaver(),
            buf: Vec::new(),
            next: None,
            freq: 0.0,
            phase: 0.0,
            stats: OfdmStats::default(),
            scratch: vec![Complex32::default(); TU],
            coherent: false,
            debug_cir: false,
        }
    }

    pub fn locked(&self) -> bool {
        self.next.is_some()
    }

    fn spectrum(&mut self, at: usize) -> Vec<Complex32> {
        let mut x: Vec<Complex32> = self.buf[at..at + TU].iter().map(|z| Complex32::new(z.re, z.im)).collect();
        self.fft.process_with_scratch(&mut x, &mut self.scratch);
        x
    }

    /// Integer carrier offset from the PRS spectrum (differential correlation: insensitive
    /// to timing), searched within +-`range` carriers
    /// Returns (offset, how much the best peak stands out: best / mean of the others)
    fn coarse_offset(&self, x: &[Complex32], range: i32) -> (i32, f32) {
        let mut best = (0, 0.0f32);
        let mut all = Vec::with_capacity(2 * range as usize + 1);
        for m in -range..=range {
            let mut acc = Complex32::default();
            let mut k = -768;
            while k < 768 {
                let (k1, k2) = (k, if k + 1 == 0 { 1 } else { k + 1 });
                if k1 != 0 {
                    let a = x[bin(k1 + m)] * x[bin(k2 + m)].conj();
                    let r = self.prs[bin(k1)] * self.prs[bin(k2)].conj();
                    acc += a * r.conj();
                }
                k += 1;
            }
            let v = acc.norm();
            all.push(v);
            if v > best.1 {
                best = (m, v);
            }
        }
        let others = (all.iter().sum::<f32>() - best.1) / (all.len() - 1) as f32;
        (best.0, best.1 / others.max(1e-20))
    }

    /// Back to searching from scratch (no frequency correction)
    pub fn reset(&mut self) {
        self.next = None;
        self.freq = 0.0;
        self.phase = 0.0;
        self.buf.clear();
        self.stats.resyncs += 1;
    }

    /// Channel impulse response from the PRS: (peak position, signed, in samples; peak to
    /// mean ratio)
    fn impulse(&mut self, x: &[Complex32]) -> (isize, f32) {
        let mut h: Vec<Complex32> = x.iter().zip(&self.prs).map(|(a, r)| a * r.conj()).collect();
        self.ifft.process_with_scratch(&mut h, &mut self.scratch);
        let mags: Vec<f32> = h.iter().map(|c| c.norm_sqr()).collect();
        if self.debug_cir {
            // Taps within 30 dB of the peak, position in samples (1 sample = 0.49 us)
            let mx = mags.iter().cloned().fold(0.0f32, f32::max);
            let mut taps: Vec<(isize, f32)> = mags
                .iter()
                .enumerate()
                .filter(|(_, &m)| m > mx * 0.063)
                .map(|(i, &m)| (if i > TU / 2 { i as isize - TU as isize } else { i as isize }, 10.0 * (m / mx).log10()))
                .collect();
            taps.sort_by_key(|t| t.0);
            let floor = 10.0 * (mags.iter().sum::<f32>() / mags.len() as f32 / mx).log10();
            eprintln!("CIR (floor {floor:.0} dB): {:?}", taps.iter().map(|(p, d)| format!("{p}:{d:.0}")).collect::<Vec<_>>());
        }
        let (pk, &mx) = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        let mean = mags.iter().sum::<f32>() / mags.len() as f32;
        let signed = |i: usize| if i > TU / 2 { i as isize - TU as isize } else { i as isize };
        // Time the window on the EARLIEST strong path, not the strongest: in a single
        // frequency network, transmitters nearly as strong as the main one can arrive
        // earlier; timed on the strongest, they would spill into the next symbol
        let peak = signed(pk);
        let mut first = peak;
        for back in 1..=PRE_ECHO {
            let i = (pk as isize - back as isize).rem_euclid(TU as isize) as usize;
            if mags[i] >= mx * 0.1 {
                first = peak - back as isize;
            }
        }
        (first, mx / mean.max(1e-20))
    }

    pub fn process(&mut self, input: &[C32], out: &mut Vec<OfdmFrame>) {
        // Frequency correction
        let step = -2.0 * PI * self.freq / RATE;
        for z in input {
            let (s, c) = self.phase.sin_cos();
            self.buf.push(*z * C32::new(c as f32, s as f32));
            self.phase = (self.phase + step).rem_euclid(2.0 * PI);
        }
        loop {
            match self.next {
                None => {
                    if self.buf.len() < 2 * TF {
                        return;
                    }
                    self.acquire();
                }
                Some(at) => {
                    // Need the whole frame from its PRS, plus margin for re-timing
                    if self.buf.len() < at + SYMBOLS * TS + TU + 256 {
                        return;
                    }
                    match self.frame(at) {
                        Some(f) => out.push(f),
                        None if self.next.is_some() => return,
                        None => {
                            self.stats.resyncs += 1;
                            continue;
                        }
                    }
                    // Keep the buffer short
                    let next = self.next.unwrap();
                    if next > TF + TNULL {
                        let cut = next - TNULL - TG;
                        self.buf.drain(..cut);
                        self.next = Some(next - cut);
                    }
                }
            }
        }
    }

    /// Finds a frame: lowest-power window (the null symbol), then the PRS after it
    fn acquire(&mut self) {
        let n = TNULL - 200; // inside the null symbol whatever the alignment
        let mut acc: f64 = self.buf[..n].iter().map(|z| z.norm_sqr() as f64).sum();
        let (mut best, mut at) = (acc, 0usize);
        for i in 0..TF {
            acc += self.buf[i + n].norm_sqr() as f64 - self.buf[i].norm_sqr() as f64;
            if acc < best {
                best = acc;
                at = i + 1;
            }
        }
        // The null ends ~100 samples after this window; the PRS useful part starts TG later
        let prs = at + n + 100 + TG;
        if prs + TU + 512 > self.buf.len() {
            self.buf.drain(..TF);
            return;
        }
        let x = self.spectrum(prs);
        let (m, standout) = self.coarse_offset(&x, 40);
        if standout < 4.0 {
            // No clear phase reference where the null suggested: noise, or a wrong dip
            self.buf.drain(..TF);
            return;
        }
        if m != 0 {
            // Correct the carrier offset; the buffer is already mixed: restart from here.
            // An RTL-SDR is within ~100 ppm: beyond +-MAX_OFFSET it is a false peak.
            let f = self.freq + m as f64 * 1000.0;
            self.freq = if f.abs() <= MAX_OFFSET { f } else { 0.0 };
            self.buf.clear();
            self.phase = 0.0;
            return;
        }
        let (pos, ratio) = self.impulse(&x);
        if ratio < 20.0 {
            self.buf.drain(..TF); // no DAB signal here
            return;
        }
        // Window `EARLY` samples ahead of the useful part: the impulse peak should sit at -EARLY
        let at = prs as isize + pos + EARLY;
        if at < TG as isize {
            self.buf.drain(..TF);
            return;
        }
        self.next = Some(at as usize);
    }

    /// Demodulates the frame whose PRS useful part starts at `at`; sets the next position
    fn frame(&mut self, at: usize) -> Option<OfdmFrame> {
        // Re-time on the PRS
        let x = self.spectrum(at);
        let (pos, ratio) = self.impulse(&x);
        if ratio < 10.0 {
            self.next = None;
            return None;
        }
        let at = (at as isize + pos + EARLY).max(TG as isize) as usize;
        if at + SYMBOLS * TS + TU > self.buf.len() {
            self.next = Some(at); // wait for more samples
            return None;
        }
        let mut prev = self.spectrum(at);
        // Fine frequency: cyclic prefix correlation over all symbols
        let mut cp = C32::default();
        for l in 0..SYMBOLS {
            let u = at + l * TS; // useful part of symbol l
            let g = u - TG + EARLY as usize; // guard (minus the part we skip)
            for n in g..g + TG - 2 * EARLY as usize {
                cp += self.buf[n] * self.buf[n + TU].conj();
            }
        }
        let err_hz = -(cp.im.atan2(cp.re) as f64) * RATE / (2.0 * PI * TU as f64);
        let mut fic = Vec::with_capacity(3 * BITS_PER_SYMBOL);
        let mut msc = Vec::with_capacity(72 * BITS_PER_SYMBOL);
        let mut sig = 0.0f32;
        let mut noise = 0.0f32;
        // Channel estimate per FFT bin, from the PRS, then followed with the decisions:
        // comparing each symbol with this smoothed reference instead of the (as noisy)
        // previous symbol saves up to 3 dB on weak signals
        let mut chan: Vec<Complex32> = vec![Complex32::default(); TU];
        let mut tx: Vec<Complex32> = self.prs.clone(); // transmitted phasor of the last symbol
        if self.coherent {
            for k in -768i32..=768 {
                if k != 0 {
                    chan[bin(k)] = prev[bin(k)] * self.prs[bin(k)].conj();
                }
            }
            chan = smooth(&chan);
        }
        for l in 1..SYMBOLS {
            let cur = self.spectrum(at + l * TS);
            let d: Vec<Complex32> = if self.coherent {
                let mut d: Vec<Complex32> = self.perm.iter().map(|&k| cur[bin(k)] * (chan[bin(k)] * tx[bin(k)]).conj()).collect();
                // Common phase error (oscillator phase noise turns every carrier alike):
                // the 4th power strips the pi/4-DQPSK data, the mean angle is the error
                let mut acc = Complex32::default();
                for z in &d {
                    let z2 = z * z;
                    acc += z2 * z2;
                }
                let cpe = (-acc).arg() / 4.0;
                let rot = Complex32::new(cpe.cos(), -cpe.sin());
                d.iter_mut().for_each(|z| *z *= rot);
                // The reference turns with it
                chan.iter_mut().for_each(|h| *h *= rot.conj());
                // Decisions -> transmitted phasors -> refreshed channel estimate
                let r = std::f32::consts::FRAC_1_SQRT_2;
                let mut raw = chan.clone();
                for (&k, z) in self.perm.iter().zip(&d) {
                    let b = bin(k);
                    let dec = Complex32::new(r * z.re.signum(), r * z.im.signum());
                    tx[b] *= dec;
                    let h = cur[b] * tx[b].conj();
                    raw[b] = chan[b] * (1.0 - CHAN_ALPHA) + h * CHAN_ALPHA;
                }
                chan = smooth(&raw);
                d
            } else {
                self.perm.iter().map(|&k| cur[bin(k)] * prev[bin(k)].conj()).collect()
            };
            let mean = d.iter().map(|c| c.re.abs() + c.im.abs()).sum::<f32>() / (2 * K) as f32;
            let scale = 40.0 / mean.max(1e-20);
            let dst = if l <= 3 { &mut fic } else { &mut msc };
            let base = dst.len();
            dst.resize(base + BITS_PER_SYMBOL, 0);
            for (i, c) in d.iter().enumerate() {
                dst[base + i] = (c.re * scale).clamp(-127.0, 127.0) as i8;
                dst[base + K + i] = (c.im * scale).clamp(-127.0, 127.0) as i8;
                // SNR: distance from the nearest DQPSK point (|re| = |im|)
                let (a, b) = (c.re.abs(), c.im.abs());
                sig += (a + b) * (a + b) / 2.0;
                noise += (a - b) * (a - b) / 2.0;
            }
            prev = cur;
        }
        self.stats.frames += 1;
        self.stats.snr_db = 10.0 * (sig / noise.max(1e-20)).log10();
        // Frequency loop: slow, the CP estimate is precise
        self.freq += 0.3 * err_hz;
        self.stats.freq_hz = self.freq;
        self.next = Some(at + TF);
        Some(OfdmFrame { fic, msc })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaver_covers_every_carrier_once() {
        let p = interleaver();
        assert_eq!(p.len(), K);
        let mut seen = std::collections::HashSet::new();
        for &k in &p {
            assert!(k != 0 && (-768..=768).contains(&k));
            assert!(seen.insert(k));
        }
    }
}
