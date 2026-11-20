//! DRM OFDM front end: robustness mode, symbol timing and frequency from the guard-interval
//! correlation; FFT; integer frequency offset from the frequency pilots (same phase every
//! symbol); frame start from the time reference pilots; channel estimate from all pilots
//! (time then frequency interpolation). Outputs equalised frames of cells.

use super::cellmap::{self, CellMap};
use super::tables::{params, RobMode, ALL_MODES};
use crate::dsp::C32;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;

/// One frame, equalised: cells[symbol][carrier - kmin] and their channel power (reliability)
pub struct Frame {
    pub cells: Vec<Vec<C32>>,
    pub csi: Vec<Vec<f32>>,
}

#[derive(Default, Clone, Debug)]
pub struct SyncInfo {
    pub mode: Option<RobMode>,
    pub guard_corr: f32,
    pub freq_offset: f64,
    pub frame_found: bool,
    pub so: usize,
    pub symbols: u64,
}

struct Locked {
    mode: RobMode,
    tu: usize,
    tg: usize,
    fft: Arc<dyn Fft<f32>>,
    /// Absolute sample index of the next symbol's guard start
    next: f64,
    /// Correlation profile around the timing (EMA), offsets -R..=R
    prof: Vec<C32>,
    /// Integer carrier offset
    k_off: i32,
    k_found: bool,
    /// FFT outputs kept for the integer offset and frame searches (all bins)
    raw: VecDeque<Vec<C32>>,
    /// Symbols since lock (absolute counter of the next symbol)
    nsym: u64,
    /// Earliest symbol for the next timing move
    next_move: u64,
    /// Frame start: symbol counter value of a frame's first symbol
    frame0: Option<u64>,
    frame_score: Vec<f32>,
    map: Option<CellMap>,
    /// Carrier-ordered symbols waiting for channel estimation: (frame symbol, cells)
    pending: VecDeque<(usize, Vec<C32>)>,
}

const PROF_R: i64 = 3;
const BACKOFF_FRAC: f64 = 0.25; // of the guard: FFT window starts this far before the guard end

pub struct Ofdm {
    rate: f64,
    /// Raw input, and the absolute index of its first sample
    buf: Vec<C32>,
    base: u64,
    freq: f64,
    /// Mixer phase at sample `ph_n` (the frequency changes keep the phase continuous)
    ph_n: i64,
    ph: f64,
    lock: Option<Locked>,
    pub info: SyncInfo,
}

fn guard_profile(z: &[C32], tu: usize, tg: usize, from: usize, nsym: usize) -> Vec<(C32, f32)> {
    let l = tu + tg;
    let mut acc = vec![(C32::default(), 0f32); l];
    for s in 0..nsym {
        let b = from + s * l;
        if b + l + tu > z.len() {
            break;
        }
        // Products along the symbol, then a running sum over the guard length
        let prod: Vec<C32> = (0..l + tg).map(|i| z[b + i] * z[b + i + tu].conj()).collect();
        let en: Vec<f32> = (0..l + tg).map(|i| 0.5 * (z[b + i].norm_sqr() + z[b + i + tu].norm_sqr())).collect();
        let (mut p, mut e) = (C32::default(), 0f32);
        for i in 0..tg {
            p += prod[i];
            e += en[i];
        }
        for pos in 0..l {
            acc[pos].0 += p;
            acc[pos].1 += e;
            p = p - prod[pos] + prod[pos + tg];
            e = e - en[pos] + en[pos + tg];
        }
    }
    acc
}

impl Ofdm {
    pub fn new(rate: f64) -> Ofdm {
        Ofdm { rate, buf: Vec::new(), base: 0, freq: 0.0, ph_n: 0, ph: 0.0, lock: None, info: SyncInfo::default() }
    }

    pub fn map(&self) -> Option<&CellMap> {
        self.lock.as_ref().and_then(|l| l.map.as_ref())
    }

    pub fn reset(&mut self) {
        *self = Ofdm::new(self.rate);
    }

    /// New samples; complete frames are appended to `out`
    pub fn process(&mut self, input: &[C32], out: &mut Vec<Frame>) {
        self.buf.extend_from_slice(input);
        if self.lock.is_none() {
            self.acquire();
        }
        while self.lock.is_some() && self.symbol(out) {}
        // Drop consumed samples
        let keep_from = match &self.lock {
            Some(l) => (l.next as i64 - self.base as i64 - 2 * (l.tu + l.tg) as i64).max(0) as usize,
            None => self.buf.len().saturating_sub((0.8 * self.rate) as usize),
        };
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.base += keep_from as u64;
        }
    }

    fn acquire(&mut self) {
        let need = (0.8 * self.rate) as usize;
        if self.buf.len() < need {
            return;
        }
        let z = &self.buf[self.buf.len() - need..];
        let mut best: Option<(f32, RobMode, usize, C32)> = None;
        for m in ALL_MODES {
            let p = params(m);
            let (tu, tg) = (p.tu(self.rate), p.tg(self.rate));
            let nsym = need / (tu + tg) - 2;
            let prof = guard_profile(z, tu, tg, 0, nsym);
            let (pos, &(c, e)) = prof.iter().enumerate().max_by(|a, b| (a.1 .0.norm_sqr().sqrt() / a.1 .1).total_cmp(&(b.1 .0.norm_sqr().sqrt() / b.1 .1))).unwrap();
            let r = c.norm_sqr().sqrt() / e.max(1e-20);
            if best.is_none_or(|b| r > b.0) {
                best = Some((r, m, pos, c));
            }
        }
        let (r, mode, pos, c) = best.unwrap();
        self.info.guard_corr = r;
        if r < 0.35 {
            return; // no DRM (or too weak): try again with newer samples
        }
        let p = params(mode);
        let (tu, tg) = (p.tu(self.rate), p.tg(self.rate));
        // Phase of the correlation over one useful period = frequency offset
        self.freq = -(c.im.atan2(c.re) as f64) * self.rate / (2.0 * std::f64::consts::PI * tu as f64);
        let start = (self.base + (self.buf.len() - need) as u64 + pos as u64) as f64;
        self.lock = Some(Locked {
            mode,
            tu,
            tg,
            fft: FftPlanner::new().plan_fft_forward(tu),
            next: start,
            prof: vec![C32::default(); (2 * PROF_R + 1) as usize],
            k_off: 0,
            k_found: false,
            raw: VecDeque::new(),
            nsym: 0,
            next_move: 8,
            frame0: None,
            frame_score: vec![0.0; p.sym_per_frame],
            map: None,
            pending: VecDeque::new(),
        });
        self.info.mode = Some(mode);
        self.info.freq_offset = self.freq;
    }

    /// Mixed sample at absolute index n
    fn sample(&self, n: i64) -> C32 {
        let i = n - self.base as i64;
        if i < 0 || i as usize >= self.buf.len() {
            return C32::default();
        }
        let ph = self.ph - 2.0 * std::f64::consts::PI * self.freq * (n - self.ph_n) as f64 / self.rate;
        let (s, c) = ph.sin_cos();
        self.buf[i as usize] * C32::new(c as f32, s as f32)
    }

    /// Changes the mixer frequency without a phase jump at sample `n`
    fn set_freq(&mut self, f: f64, n: i64) {
        self.ph = (self.ph - 2.0 * std::f64::consts::PI * self.freq * (n - self.ph_n) as f64 / self.rate).rem_euclid(2.0 * std::f64::consts::PI);
        self.ph_n = n;
        self.freq = f;
    }

    /// One symbol if enough samples; false when waiting for more
    fn symbol(&mut self, out: &mut Vec<Frame>) -> bool {
        let rate = self.rate;
        let (tu, tg, next) = {
            let l = self.lock.as_ref().unwrap();
            (l.tu, l.tg, l.next)
        };
        let g = next.round() as i64;
        let end = g + (tu + tg) as i64 + tu as i64 + PROF_R + 1;
        if end > (self.base as usize + self.buf.len()) as i64 {
            return false;
        }
        // Timing / frequency tracking from the guard correlation around g
        let mut prof = Vec::with_capacity((2 * PROF_R + 1) as usize);
        let mut energy = 0f32;
        for d in -PROF_R..=PROF_R {
            let mut p = C32::default();
            for i in 0..tg as i64 {
                let a = self.sample(g + d + i);
                let b = self.sample(g + d + i + tu as i64);
                p += a * b.conj();
                if d == 0 {
                    energy += 0.5 * (a.norm_sqr() + b.norm_sqr());
                }
            }
            prof.push(p);
        }
        // FFT of the useful part, started a little inside the guard
        let backoff = (BACKOFF_FRAC * tg as f64) as i64;
        let start = g + tg as i64 - backoff;
        let mut v: Vec<Complex32> = (0..tu as i64).map(|i| {
            let s = self.sample(start + i);
            Complex32::new(s.re, s.im)
        }).collect();
        let l = self.lock.as_mut().unwrap();
        l.fft.process(&mut v);
        // Undo the early start (a linear phase over the carriers)
        let bins: Vec<C32> = (0..tu)
            .map(|m| {
                let k = if m < tu / 2 { m as f64 } else { m as f64 - tu as f64 };
                let ph = 2.0 * std::f64::consts::PI * k * backoff as f64 / tu as f64;
                let (s, c) = ph.sin_cos();
                C32::new(v[m].re, v[m].im) * C32::new(c as f32, s as f32)
            })
            .collect();
        for (a, b) in l.prof.iter_mut().zip(&prof) {
            *a = *a * 0.9 + *b * 0.1;
        }
        // Move the window by one sample only on clear evidence (a neighbour of the profile
        // stronger than the centre), at most once per frame: each move turns the phase of
        // the outer carriers by a lot, which the channel estimate must not see as a change
        let mag = |i: usize| l.prof[i].norm_sqr().sqrt();
        let c = PROF_R as usize;
        let mut shift = 0i64;
        if l.nsym >= l.next_move {
            if mag(c + 1) > 1.03 * mag(c) && mag(c + 1) >= mag(c - 1) {
                shift = 1;
            } else if mag(c - 1) > 1.03 * mag(c) {
                shift = -1;
            }
        }
        if shift != 0 {
            l.next_move = l.nsym + params(l.mode).sym_per_frame as u64;
            l.prof.rotate_left(((shift + 2 * PROF_R + 1) % (2 * PROF_R + 1)) as usize);
            // Symbols waiting for equalisation: same timing reference as the new ones
            let (kmin, tuf) = (l.map.as_ref().map_or(0, |m| m.kmin), tu as f32);
            for (_, cells) in l.pending.iter_mut() {
                for (i, x) in cells.iter_mut().enumerate() {
                    let k = (kmin + i as i32) as f32;
                    let ph = 2.0 * std::f32::consts::PI * k * shift as f32 / tuf;
                    *x = *x * C32::new(ph.cos(), ph.sin());
                }
            }
        }
        l.next += (tu + tg) as f64 + shift as f64;
        let c0 = prof[PROF_R as usize];
        let resid = -(c0.im.atan2(c0.re) as f64) * rate / (2.0 * std::f64::consts::PI * tu as f64);
        let f = self.freq + 0.05 * resid;
        self.set_freq(f, g + (tu + tg) as i64);
        self.info.freq_offset = self.freq;
        self.info.guard_corr = 0.95 * self.info.guard_corr + 0.05 * (c0.norm_sqr().sqrt() / energy.max(1e-20));
        self.info.symbols += 1;
        self.after_fft(bins, out);
        true
    }

    fn after_fft(&mut self, bins: Vec<C32>, out: &mut Vec<Frame>) {
        let l = self.lock.as_mut().unwrap();
        let p = params(l.mode);
        let tu = l.tu as i32;
        let n = l.nsym;
        l.nsym += 1;
        l.raw.push_back(bins);
        if l.raw.len() > 2 * p.sym_per_frame + 2 {
            l.raw.pop_front();
        }
        // Integer offset: the frequency pilots keep their phase from symbol to symbol
        if !l.k_found {
            if l.raw.len() < 12 {
                return;
            }
            let mut best = (0f32, 0i32);
            for d in -12i32..=12 {
                let mut s = 0f32;
                for &(k, _) in &p.freq_pilots {
                    let m = ((k as i32 + d).rem_euclid(tu)) as usize;
                    let mut acc = C32::default();
                    let mut e = 0f32;
                    for t in 1..l.raw.len() {
                        acc += l.raw[t][m] * l.raw[t - 1][m].conj();
                        e += l.raw[t][m].norm_sqr();
                    }
                    s += acc.norm_sqr().sqrt() / e.max(1e-20);
                }
                if s > best.0 {
                    best = (s, d);
                }
            }
            l.k_off = best.1;
            l.k_found = true;
            self.freq += best.1 as f64 * self.rate / tu as f64;
            self.info.freq_offset = self.freq;
            // The kept spectra are off by d carriers: shift them
            let d = best.1;
            for r in l.raw.iter_mut() {
                let old = r.clone();
                for m in 0..tu {
                    r[m as usize] = old[((m + d).rem_euclid(tu)) as usize];
                }
            }
        }
        // Frame start: the time reference pattern (phase steps between its carriers)
        if l.frame0.is_none() {
            let r = l.raw.back().unwrap();
            let tp = p.time_pilots;
            let mut acc = C32::default();
            let mut e = 0f32;
            for w in tp.windows(2) {
                let (k1, p1) = w[0];
                let (k2, p2) = w[1];
                let x1 = r[(k1 as i32).rem_euclid(tu) as usize];
                let x2 = r[(k2 as i32).rem_euclid(tu) as usize];
                let ref_ph = 2.0 * std::f32::consts::PI * (p1 as f32 - p2 as f32) / 1024.0;
                acc += x1 * x2.conj() * C32::new(ref_ph.cos(), -ref_ph.sin());
                e += x1.norm_sqr().sqrt() * x2.norm_sqr().sqrt();
            }
            let score = acc.re / e.max(1e-20);
            let spf = p.sym_per_frame;
            let idx = (n % spf as u64) as usize;
            l.frame_score[idx] = 0.7 * l.frame_score[idx] + score;
            if n >= 3 * spf as u64 {
                let (bi, &bs) = l.frame_score.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
                let mut sorted = l.frame_score.clone();
                sorted.sort_by(|a, b| b.total_cmp(a));
                if bs > 0.5 && bs > 2.0 * sorted[1].max(0.0) {
                    l.frame0 = Some(bi as u64);
                    self.info.frame_found = true;
                    // Spectrum occupancy: the widest one whose edges carry power
                    let mean = |k: i32| -> f32 { l.raw.iter().map(|r| r[(k.rem_euclid(tu)) as usize].norm_sqr()).sum::<f32>() / l.raw.len() as f32 };
                    let inband: f32 = (1..20).map(|k| mean(k)).sum::<f32>() / 19.0;
                    let mut so = 0;
                    for cand in 0..6 {
                        let (kmin, kmax) = (p.kmin[cand], p.kmax[cand]);
                        if kmin == 0 && kmax == 0 || kmax >= tu / 2 || -kmin >= tu / 2 {
                            continue;
                        }
                        if mean(kmax - 1) > 0.1 * inband && mean(kmin + 1) > 0.1 * inband {
                            so = cand;
                        }
                    }
                    l.map = CellMap::new(l.mode, so);
                    self.info.so = so;
                    // Symbols already received of the current frame are dropped: start clean
                }
            }
            return;
        }
        // Collect carriers kmin..=kmax with their frame-symbol index
        let Some(map) = l.map.as_ref() else { return };
        let spf = p.sym_per_frame;
        let fs = ((n - l.frame0.unwrap()) % spf as u64) as usize;
        let r = l.raw.back().unwrap();
        let cells: Vec<C32> = (map.kmin..=map.kmax).map(|k| r[(k.rem_euclid(tu)) as usize]).collect();
        l.pending.push_back((fs, cells));
        // Window: `margin` symbols before a frame start, the frame, `margin` after
        let margin = p.scat_time_int as usize;
        while l.pending.len() > margin && l.pending[margin].0 != 0 {
            l.pending.pop_front();
        }
        if l.pending.len() >= spf + 2 * margin {
            out.push(equalise(map, &l.pending, margin, spf));
            for _ in 0..spf {
                l.pending.pop_front();
            }
        }
    }
}

/// Channel estimate over one frame (symbols margin..margin+spf of `win`) and equalisation
fn equalise(map: &CellMap, win: &VecDeque<(usize, Vec<C32>)>, margin: usize, spf: usize) -> Frame {
    let nc = map.carriers();
    let nw = win.len();
    // Pilot estimates H = X / P for every symbol in the window (frame-0 rows: the pilot
    // pattern is the same in the three frames of a superframe)
    let mut hp: Vec<Vec<Option<C32>>> = vec![vec![None; nc]; nw];
    for (t, (fs, x)) in win.iter().enumerate() {
        for c in 0..nc {
            let kind = map.kind[*fs][c];
            if cellmap::is_pilot(kind) {
                let p = map.pilot[*fs][c];
                let d = p.norm_sqr();
                hp[t][c] = Some(x[c] * p.conj() * (1.0 / d));
            }
        }
    }
    let mut cells = Vec::with_capacity(spf);
    let mut csi = Vec::with_capacity(spf);
    for t in margin..margin + spf {
        // Time interpolation on every carrier that has pilots in the window
        let mut h_t: Vec<Option<C32>> = vec![None; nc];
        for c in 0..nc {
            if let Some(h) = hp[t][c] {
                h_t[c] = Some(h);
                continue;
            }
            let before = (0..t).rev().find_map(|u| hp[u][c].map(|h| (u, h)));
            let after = (t + 1..nw).find_map(|u| hp[u][c].map(|h| (u, h)));
            h_t[c] = match (before, after) {
                (Some((u0, h0)), Some((u1, h1))) if u1 - u0 <= 2 * map_time_period(map) => {
                    let a = (t - u0) as f32 / (u1 - u0) as f32;
                    Some(h0 * (1.0 - a) + h1 * a)
                }
                _ => None,
            };
        }
        // Frequency interpolation between the carriers that have an estimate
        let known: Vec<usize> = (0..nc).filter(|&c| h_t[c].is_some()).collect();
        let mut h = vec![C32::default(); nc];
        for c in 0..nc {
            h[c] = match h_t[c] {
                Some(v) => v,
                None => {
                    let lo = known.iter().rev().find(|&&k| k < c).copied();
                    let hi = known.iter().find(|&&k| k > c).copied();
                    match (lo, hi) {
                        (Some(a), Some(b)) => {
                            let w = (c - a) as f32 / (b - a) as f32;
                            h_t[a].unwrap() * (1.0 - w) + h_t[b].unwrap() * w
                        }
                        (Some(a), None) => h_t[a].unwrap(),
                        (None, Some(b)) => h_t[b].unwrap(),
                        _ => C32::new(1.0, 0.0),
                    }
                }
            };
        }
        let x = &win[t].1;
        let eq: Vec<C32> = (0..nc)
            .map(|c| {
                let d = h[c].norm_sqr().max(1e-20);
                x[c] * h[c].conj() * (1.0 / d)
            })
            .collect();
        cells.push(eq);
        csi.push(h.iter().map(|v| v.norm_sqr()).collect());
    }
    Frame { cells, csi }
}

fn map_time_period(map: &CellMap) -> usize {
    params(map.mode).scat_time_int as usize
}
