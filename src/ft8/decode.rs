//! FT8 / FT4 decoding of one slot of 12 kHz audio.
//!
//! 1. Spectrogram: an FFT over exactly one symbol (rectangular window = matched filter for
//!    orthogonal tones), zero-padded to half-tone bins, every half symbol.
//! 2. Candidates: positions (time, frequency) where the Costas sync tones stand out of their
//!    neighbours.
//! 3. Each candidate: log-likelihood of the 174 bits (max-log over the Gray-coded tones),
//!    LDPC belief propagation, CRC, unpacking.
//! 4. Decoded signals are subtracted from the spectrogram and the search runs again, so
//!    weaker signals under strong ones come out on a second pass.

use super::ldpc;
use super::message::{unpack, CallHashes, Message};
use rustfft::{num_complex::Complex32, FftPlanner};

pub const RATE: f64 = 12_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Ft8,
    Ft4,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Ft8 => "FT8",
            Protocol::Ft4 => "FT4",
        }
    }
    /// Samples per symbol at 12 kHz
    pub fn sym(self) -> usize {
        match self {
            Protocol::Ft8 => 1920,
            Protocol::Ft4 => 576,
        }
    }
    pub fn tones(self) -> usize {
        match self {
            Protocol::Ft8 => 8,
            Protocol::Ft4 => 4,
        }
    }
    pub fn spacing(self) -> f64 {
        RATE / self.sym() as f64
    }
    /// Channel symbols (sync + data)
    pub fn nn(self) -> usize {
        match self {
            Protocol::Ft8 => 79,
            Protocol::Ft4 => 103,
        }
    }
    pub fn slot(self) -> f64 {
        match self {
            Protocol::Ft8 => 15.0,
            Protocol::Ft4 => 7.5,
        }
    }
    /// Nominal start of the signal in the slot (s)
    pub fn start(self) -> f64 {
        match self {
            Protocol::Ft8 => 0.5,
            Protocol::Ft4 => 0.5,
        }
    }
    fn bits_per_symbol(self) -> usize {
        match self {
            Protocol::Ft8 => 3,
            Protocol::Ft4 => 2,
        }
    }
    fn gray(self) -> &'static [usize] {
        match self {
            Protocol::Ft8 => &[0, 1, 3, 2, 5, 6, 4, 7],
            Protocol::Ft4 => &[0, 1, 3, 2],
        }
    }
    /// Sync blocks: (first symbol, tones)
    fn syncs(self) -> &'static [(usize, &'static [usize])] {
        const C7: &[usize] = &[3, 1, 4, 0, 6, 5, 2];
        match self {
            Protocol::Ft8 => &[(0, C7), (36, C7), (72, C7)],
            Protocol::Ft4 => &[(0, &[0, 1, 3, 2]), (33, &[1, 0, 2, 3]), (66, &[2, 3, 1, 0]), (99, &[3, 2, 0, 1])],
        }
    }
    fn is_sync(self, s: usize) -> bool {
        self.syncs().iter().any(|&(a, c)| s >= a && s < a + c.len())
    }
}

/// FT4 message bits are scrambled with this sequence
const FT4_RVEC: [u8; 77] = [
    0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 1, 0,
    0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0, 1,
];

/// Channel tones of a 77-bit message
pub fn tones(proto: Protocol, msg77: &[u8]) -> Vec<usize> {
    let mut m = msg77[..77].to_vec();
    if proto == Protocol::Ft4 {
        for (b, r) in m.iter_mut().zip(FT4_RVEC) {
            *b ^= r;
        }
    }
    let cw = ldpc::encode(&ldpc::add_crc(&m));
    let bps = proto.bits_per_symbol();
    let mut data = cw.chunks(bps).map(|c| proto.gray()[c.iter().fold(0usize, |a, &b| (a << 1) | b as usize)]);
    let mut t = vec![0usize; proto.nn()];
    for (s, slot) in t.iter_mut().enumerate() {
        *slot = match proto.syncs().iter().find(|&&(a, c)| s >= a && s < a + c.len()) {
            Some(&(a, c)) => c[s - a],
            None => data.next().unwrap(),
        };
    }
    t
}

#[derive(Clone, Debug)]
pub struct Decode {
    pub msg: Message,
    /// Audio frequency of the lowest tone (Hz)
    pub freq: f64,
    /// Start of the signal relative to its nominal place (s)
    pub dt: f64,
    /// SNR in 2500 Hz (dB), as WSJT-X reports it
    pub snr: f64,
}

/// Spectrogram: power per frame (half symbol) and half-tone bin
struct Spectro {
    pow: Vec<f32>,
    frames: usize,
    bins: usize,
}

impl Spectro {
    fn new(proto: Protocol, audio: &[f32]) -> Spectro {
        let sym = proto.sym();
        let nfft = 2 * sym;
        let hop = sym / 2;
        let bins = sym; // half-tone bins up to 6 kHz
        let frames = if audio.len() >= sym { (audio.len() - sym) / hop + 1 } else { 0 };
        let fft = FftPlanner::new().plan_fft_forward(nfft);
        let mut buf = vec![Complex32::default(); nfft];
        let mut pow = vec![0f32; frames * bins];
        for t in 0..frames {
            for (i, z) in buf.iter_mut().enumerate() {
                *z = if i < sym { Complex32::new(audio[t * hop + i], 0.0) } else { Complex32::default() };
            }
            fft.process(&mut buf);
            for k in 0..bins {
                pow[t * bins + k] = buf[k].norm_sqr();
            }
        }
        Spectro { pow, frames, bins }
    }

    fn at(&self, t: usize, k: usize) -> f32 {
        self.pow[t * self.bins + k]
    }
}

struct Candidate {
    t0: usize, // frame of symbol 0
    k0: usize, // half-tone bin of tone 0
    score: f32,
}

/// Decodes one slot. `audio` starts `lead` seconds before the nominal signal start.
pub fn decode(proto: Protocol, audio: &[f32], lead: f64, hashes: &mut CallHashes, f_lo: f64, f_hi: f64) -> Vec<Decode> {
    let mut sp = Spectro::new(proto, audio);
    let ntones = proto.tones();
    let nn = proto.nn();
    let span = 2 * (nn - 1) + 1; // frames covered by the symbols
    if sp.frames < span {
        return Vec::new();
    }
    let half = proto.spacing() / 2.0;
    let k_lo = ((f_lo / half).ceil() as usize).max(2);
    let k_hi = ((f_hi / half) as usize).min(sp.bins - 2 * ntones - 2);
    let mut out: Vec<Decode> = Vec::new();
    for pass in 0..3 {
        let lp: Vec<f32> = sp.pow.iter().map(|&p| (p + 1e-12).log10()).collect();
        let lat = |t: usize, k: usize| lp[t * sp.bins + k];
        // Noise floor per bin: median over time (for SNR and the candidate threshold)
        let mut cands = Vec::new();
        for t0 in 0..=(sp.frames - span) {
            for k0 in k_lo..=k_hi {
                let mut score = 0.0f32;
                let mut n = 0;
                for &(a, costas) in proto.syncs() {
                    for (j, &tone) in costas.iter().enumerate() {
                        let t = t0 + 2 * (a + j);
                        let k = k0 + 2 * tone;
                        let p = lat(t, k);
                        if tone > 0 {
                            score += p - lat(t, k - 2);
                            n += 1;
                        }
                        if tone + 1 < ntones {
                            score += p - lat(t, k + 2);
                            n += 1;
                        }
                        if t >= 2 {
                            score += p - lat(t - 2, k);
                            n += 1;
                        }
                        if t + 2 < sp.frames {
                            score += p - lat(t + 2, k);
                            n += 1;
                        }
                    }
                }
                let score = score / n as f32;
                if score > 0.2 {
                    cands.push(Candidate { t0, k0, score });
                }
            }
        }
        // Local maxima only (neighbouring half-steps see the same signal)
        cands.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut kept: Vec<Candidate> = Vec::new();
        for c in cands {
            if kept.iter().any(|k| k.t0.abs_diff(c.t0) <= 1 && k.k0.abs_diff(c.k0) <= 1) {
                continue;
            }
            kept.push(c);
            if kept.len() >= 300 {
                break;
            }
        }
        let mut found = 0;
        for (rank, c) in kept.iter().enumerate() {
            // Already decoded at this place
            if out.iter().any(|d| (d.freq - c.k0 as f64 * half).abs() < 1.5 * half && ((d.dt + lead) - c.t0 as f64 * proto.sym() as f64 / 2.0 / RATE).abs() < 0.2) {
                continue;
            }
            // Exact position first; then the spectrogram, where decoded signals are removed
            let (tf, ff) = refine_sync(proto, audio, c.t0 as f64 * proto.sym() as f64 / 2.0 / RATE, c.k0 as f64 * half);
            let res = if rank < 150 { try_decode(proto, llr_fine(proto, audio, tf, ff)) } else { None };
            let Some((msg77, llr_ok)) = res.or_else(|| try_decode(proto, llr_coarse(proto, &sp, &lp, c))) else { continue };
            if !llr_ok {
                continue;
            }
            let Some(msg) = unpack(&msg77, hashes) else { continue };
            if out.iter().any(|d| d.msg.text == msg.text) {
                continue;
            }
            let tn = tones(proto, &msg77);
            let t = c.t0 as f64 * proto.sym() as f64 / 2.0 / RATE;
            let (t, f, s) = refine(proto, audio, t, c.k0 as f64 * half, &tn);
            let snr = estimate_snr(proto, &sp, c, s);
            subtract(&mut sp, c, &tn);
            out.push(Decode { msg, freq: f, dt: t - lead, snr });
            found += 1;
        }
        if found == 0 || pass == 2 {
            break;
        }
    }
    out
}

/// |X|^2 of one symbol-long window at `start` (samples, may be fractional-free) for `f` Hz
fn sym_power(audio: &[f32], start: isize, len: usize, f: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / RATE;
    let (dc, ds) = (w.cos(), w.sin());
    let (mut c, mut s) = (1.0f64, 0.0f64);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for n in 0..len as isize {
        let idx = start + n;
        if idx >= 0 && (idx as usize) < audio.len() {
            let x = audio[idx as usize] as f64;
            re += x * c;
            im -= x * s;
        }
        let nc = c * dc - s * ds;
        s = s * dc + c * ds;
        c = nc;
    }
    re * re + im * im
}

/// Candidate position refined on the sync tones: (t s, f Hz)
fn refine_sync(proto: Protocol, audio: &[f32], t: f64, f: f64) -> (f64, f64) {
    let sym = proto.sym();
    let score = |t: f64, f: f64| -> f64 {
        let s0 = (t * RATE).round() as isize;
        let mut tot = 0.0;
        for &(a, costas) in proto.syncs() {
            for (j, &tone) in costas.iter().enumerate() {
                tot += sym_power(audio, s0 + ((a + j) * sym) as isize, sym, f + tone as f64 * proto.spacing());
            }
        }
        tot
    };
    let (dt, df) = (sym as f64 / RATE / 8.0, proto.spacing() / 8.0);
    let mut best = (t, f, score(t, f));
    for _ in 0..2 {
        let (t0, f0) = (best.0, best.1);
        for i in -2i32..=2 {
            for j in -2i32..=2 {
                if i == 0 && j == 0 {
                    continue;
                }
                let (tt, ff) = (t0 + i as f64 * dt, f0 + j as f64 * df);
                let p = score(tt, ff);
                if p > best.2 {
                    best = (tt, ff, p);
                }
            }
        }
        if best.0 == t0 && best.1 == f0 {
            break;
        }
    }
    (best.0, best.1)
}

/// Bit log-likelihoods from the tone amplitudes measured exactly at (t, f)
fn llr_fine(proto: Protocol, audio: &[f32], t: f64, f: f64) -> [f32; 174] {
    let sym = proto.sym();
    let s0 = (t * RATE).round() as isize;
    let gray = proto.gray();
    let mut mags = Vec::with_capacity(proto.nn());
    for s in 0..proto.nn() {
        if proto.is_sync(s) {
            continue;
        }
        let mut s2 = [0f32; 8];
        for (j, v) in s2.iter_mut().enumerate().take(proto.tones()) {
            *v = sym_power(audio, s0 + (s * sym) as isize, sym, f + gray[j] as f64 * proto.spacing()) as f32;
        }
        mags.push(s2);
    }
    // Noise power per tone: the weaker tones of each symbol are noise
    let mut all: Vec<f32> = mags.iter().flat_map(|m| m[..proto.tones()].iter().copied()).collect();
    let mid = all.len() / 2;
    let med = *all.select_nth_unstable_by(mid, f32::total_cmp).1;
    let n0 = med / std::f32::consts::LN_2 * 1.3; // median of mostly-noise bins, inflated by the signal ones
    let ntones = proto.tones();
    let bps = proto.bits_per_symbol();
    let mut llr = [0f32; 174];
    let mut i = 0;
    for m in &mags {
        // log I0(2 sqrt(Es P / N0^2)) ~ sqrt(Es P)/N0 for strong tones: amplitude-based metric
        let e: Vec<f32> = m[..ntones].iter().map(|&p| 2.0 * (p / n0).sqrt()).collect();
        for b in 0..bps {
            let mask = 1 << (bps - 1 - b);
            let lse = |want: bool| {
                let v: Vec<f32> = (0..ntones).filter(|&j| (j & mask != 0) == want).map(|j| e[j]).collect();
                let mx = v.iter().copied().fold(f32::MIN, f32::max);
                mx + v.iter().map(|x| (x - mx).exp()).sum::<f32>().ln()
            };
            llr[i] = lse(true) - lse(false);
            i += 1;
        }
    }
    llr
}

/// Bit log-likelihoods from the spectrogram (after subtraction of decoded signals)
fn llr_coarse(proto: Protocol, sp: &Spectro, lp: &[f32], c: &Candidate) -> [f32; 174] {
    let gray = proto.gray();
    let mut mags = Vec::with_capacity(proto.nn());
    for s in 0..proto.nn() {
        if proto.is_sync(s) {
            continue;
        }
        let t = c.t0 + 2 * s;
        let mut s2 = [0f32; 8];
        for (j, v) in s2.iter_mut().enumerate().take(proto.tones()) {
            *v = lp[t * sp.bins + c.k0 + 2 * gray[j]];
        }
        mags.push(s2);
    }
    bits_from(proto, &mags)
}

/// Max-log bit metrics from per-symbol tone values (indexed by the Gray-coded bit value)
fn bits_from(proto: Protocol, mags: &[[f32; 8]]) -> [f32; 174] {
    let ntones = proto.tones();
    let bps = proto.bits_per_symbol();
    let mut llr = [0f32; 174];
    let mut i = 0;
    for s2 in mags {
        for b in 0..bps {
            let mask = 1 << (bps - 1 - b);
            let (mut one, mut zero) = (f32::MIN, f32::MIN);
            for (j, &v) in s2.iter().enumerate().take(ntones) {
                if j & mask != 0 {
                    one = one.max(v);
                } else {
                    zero = zero.max(v);
                }
            }
            llr[i] = one - zero;
            i += 1;
        }
    }
    llr
}

fn try_decode(proto: Protocol, mut llr: [f32; 174]) -> Option<(Vec<u8>, bool)> {
    // Same scale for every candidate (variance 24, as ft8_lib)
    let mean = llr.iter().sum::<f32>() / 174.0;
    let var = llr.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / 174.0;
    let k = (24.0 / var.max(1e-6)).sqrt();
    llr.iter_mut().for_each(|v| *v *= k);
    let (cw, errs) = ldpc::decode(&llr, 30);
    if errs != 0 || !ldpc::crc_ok(&cw[..91]) {
        return None;
    }
    let mut m = cw[..77].to_vec();
    if m.iter().all(|&b| b == 0) {
        return None;
    }
    if proto == Protocol::Ft4 {
        for (b, r) in m.iter_mut().zip(FT4_RVEC) {
            *b ^= r;
        }
    }
    Some((m, true))
}

/// Mean matched-filter power of the signal's tones (same scale as the spectrogram)
fn tone_power(proto: Protocol, audio: &[f32], t: f64, f: f64, tn: &[usize]) -> f64 {
    let sym = proto.sym();
    let s0 = (t * RATE).round() as isize;
    let mut total = 0.0f64;
    for (i, &tone) in tn.iter().enumerate() {
        let w = 2.0 * std::f64::consts::PI * (f + tone as f64 * proto.spacing()) / RATE;
        let (dc, ds) = (w.cos(), w.sin());
        let (mut c, mut s) = (1.0f64, 0.0f64);
        let (mut re, mut im) = (0.0f64, 0.0f64);
        let start = s0 + (i * sym) as isize;
        for n in 0..sym as isize {
            let idx = start + n;
            if idx >= 0 && (idx as usize) < audio.len() {
                let x = audio[idx as usize] as f64;
                re += x * c;
                im -= x * s;
            }
            let nc = c * dc - s * ds;
            s = s * dc + c * ds;
            c = nc;
        }
        total += re * re + im * im;
    }
    total / tn.len() as f64
}

/// Time (s from the window start) and frequency to a quarter of the search grid: the
/// position where the known tones are strongest. Returns (t, f, mean tone power).
fn refine(proto: Protocol, audio: &[f32], t: f64, f: f64, tn: &[usize]) -> (f64, f64, f64) {
    let (dt, df) = (proto.sym() as f64 / RATE / 8.0, proto.spacing() / 8.0);
    let mut best = (t, f, tone_power(proto, audio, t, f, tn));
    for _ in 0..2 {
        let (t0, f0) = (best.0, best.1);
        for i in -2i32..=2 {
            for j in -2i32..=2 {
                if i == 0 && j == 0 {
                    continue;
                }
                let (tt, ff) = (t0 + i as f64 * dt, f0 + j as f64 * df);
                let p = tone_power(proto, audio, tt, ff, tn);
                if p > best.2 {
                    best = (tt, ff, p);
                }
            }
        }
        if best.0 == t0 && best.1 == f0 {
            break;
        }
    }
    best
}

/// SNR in 2500 Hz: tone power over the noise of a bin, from the matched-filter outputs
fn estimate_snr(proto: Protocol, sp: &Spectro, c: &Candidate, s: f64) -> f64 {
    // Noise: median of the bins around the signal over its duration
    let mut v: Vec<f32> = Vec::new();
    let span = 2 * (proto.nn() - 1);
    for t in (c.t0..=c.t0 + span).step_by(2) {
        for k in c.k0.saturating_sub(40)..(c.k0 + 2 * proto.tones() + 40).min(sp.bins) {
            v.push(sp.at(t, k));
        }
    }
    let mid = v.len() / 2;
    let med = *v.select_nth_unstable_by(mid, f32::total_cmp).1 as f64;
    // Median of an exponential distribution = ln 2 x mean
    let n = med / std::f64::consts::LN_2;
    let ratio = ((s - n) / n).max(1e-3);
    // 4 / (2500 Hz x symbol length): matched filter bandwidth to 2500 Hz
    let t_sym = proto.sym() as f64 / RATE;
    (10.0 * (ratio * 4.0 / (2500.0 * t_sym * 4.0)).log10()).round()
}

/// Removes a decoded signal from the spectrogram: its tone bins drop to the local noise
fn subtract(sp: &mut Spectro, c: &Candidate, tn: &[usize]) {
    for (i, &tone) in tn.iter().enumerate() {
        let t = c.t0 + 2 * i;
        let k = c.k0 + 2 * tone;
        // Noise estimate from the bins a few tones away
        let left = if k >= 24 { sp.at(t, k - 24) } else { sp.at(t, k + 24) };
        let right = if k + 24 < sp.bins { sp.at(t, k + 24) } else { left };
        let floor = left.min(right);
        for dk in -2i32..=2 {
            let kk = (k as i32 + dk) as usize;
            for tt in [t.saturating_sub(1), t, t + 1] {
                if tt < sp.frames && kk < sp.bins {
                    let p = &mut sp.pow[tt * sp.bins + kk];
                    *p = p.min(floor);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::message::pack;
    use super::*;

    /// Audio of a message: continuous-phase FSK (GFSK smoothing is not needed to decode)
    pub fn synth(proto: Protocol, msg: &str, f0: f64, start: f64, amp: f32, len: f64) -> Vec<f32> {
        let tn = tones(proto, &pack(msg).unwrap());
        let sym = proto.sym();
        let mut out = vec![0f32; (len * RATE) as usize];
        let s0 = (start * RATE) as usize;
        let mut ph = 0.0f64;
        for (i, &t) in tn.iter().enumerate() {
            let f = f0 + t as f64 * proto.spacing();
            for n in 0..sym {
                let idx = s0 + i * sym + n;
                if idx < out.len() {
                    out[idx] += amp * ph.sin() as f32;
                }
                ph += 2.0 * std::f64::consts::PI * f / RATE;
            }
        }
        out
    }

    pub fn noise(out: &mut [f32], sigma: f32, seed: u32) {
        let mut s = seed.max(1);
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 + 0.5) / 4_294_967_296.0
        };
        for x in out.iter_mut() {
            let (u, v) = (rnd(), rnd());
            *x += sigma * (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos();
        }
    }

    /// Amplitude of a sine for an SNR (2500 Hz) against noise of std `sigma`
    fn amp_for(snr_db: f64, sigma: f32) -> f32 {
        // SNR = (A^2/2) / (sigma^2 x 2500 / 6000)
        ((10f64.powf(snr_db / 10.0) * (sigma as f64).powi(2) * 2500.0 / 6000.0 * 2.0).sqrt()) as f32
    }

    #[test]
    fn decodes_several_ft8_signals() {
        let len = 16.0;
        let lead = 1.5;
        let sigs = [("CQ F4ABC JN05", 1000.0, 0.0, -5.0), ("F4ABC DL1XX -12", 1520.3, 0.3, -15.0), ("DL1XX YO3RAD R-08", 621.0, -0.7, -18.0), ("TNX BOB 73 GL", 2310.0, 1.1, -10.0)];
        let sigma = 0.1;
        let mut audio = vec![0f32; (len * RATE) as usize];
        for (m, f, dt, snr) in sigs {
            let s = synth(Protocol::Ft8, m, f, lead + dt, amp_for(snr, sigma), len);
            audio.iter_mut().zip(s).for_each(|(a, b)| *a += b);
        }
        noise(&mut audio, sigma, 42);
        let mut h = CallHashes::default();
        let d = decode(Protocol::Ft8, &audio, lead, &mut h, 200.0, 3000.0);
        for (m, f, dt, snr) in sigs {
            let x = d.iter().find(|x| x.msg.text == m).unwrap_or_else(|| panic!("{m} not decoded: {d:?}"));
            assert!((x.freq - f).abs() < 4.0, "{m}: {} Hz", x.freq);
            assert!((x.dt - dt).abs() < 0.1, "{m}: dt {}", x.dt);
            assert!((x.snr - snr).abs() <= 3.0, "{m}: SNR {} instead of {snr}", x.snr);
        }
        assert_eq!(d.len(), sigs.len(), "{d:?}");
    }

    #[test]
    fn decodes_ft4() {
        let len = 8.5;
        let lead = 1.0;
        let sigma = 0.1;
        let mut audio = synth(Protocol::Ft4, "CQ YO3RAD KN34", 1200.0, lead + 0.2, amp_for(-8.0, sigma), len);
        let b = synth(Protocol::Ft4, "YO3RAD F4ABC -05", 700.0, lead - 0.1, amp_for(-12.0, sigma), len);
        audio.iter_mut().zip(b).for_each(|(a, b)| *a += b);
        noise(&mut audio, sigma, 7);
        let mut h = CallHashes::default();
        let d = decode(Protocol::Ft4, &audio, lead, &mut h, 200.0, 3000.0);
        let texts: Vec<&str> = d.iter().map(|x| x.msg.text.as_str()).collect();
        assert!(texts.contains(&"CQ YO3RAD KN34") && texts.contains(&"YO3RAD F4ABC -05"), "{texts:?}");
    }

    #[test]
    fn noise_decodes_nothing() {
        let mut audio = vec![0f32; (16.0 * RATE) as usize];
        noise(&mut audio, 0.1, 3);
        let mut h = CallHashes::default();
        assert!(decode(Protocol::Ft8, &audio, 1.5, &mut h, 200.0, 3000.0).is_empty());
    }

    /// Decode rate against SNR (cargo test --release sensitivity -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn sensitivity() {
        for snr in [-16.0, -18.0, -19.0, -20.0, -21.0, -22.0] {
            let mut ok = 0;
            for trial in 0..20u32 {
                let lead = 1.5;
                let mut audio = synth(Protocol::Ft8, "DL1XX YO3RAD KN34", 900.0 + trial as f64 * 37.3, lead + 0.13 * (trial % 5) as f64, amp_for(snr, 0.1), 16.1);
                noise(&mut audio, 0.1, 100 + trial);
                let mut h = CallHashes::default();
                if decode(Protocol::Ft8, &audio, lead, &mut h, 200.0, 3000.0).iter().any(|d| d.msg.text == "DL1XX YO3RAD KN34") {
                    ok += 1;
                }
            }
            eprintln!("{snr} dB: {ok}/20");
        }
    }
}
