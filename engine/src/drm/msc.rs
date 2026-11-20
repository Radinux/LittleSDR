//! Main Service Channel: cells of the logical MSC frames (they do not line up with the
//! transmission frames), cell deinterleaving over 5 frames (long) or 1 (short), multilevel
//! decoding (16-QAM: level 0, re-encoded to pick the points for level 1), energy dispersal.
//! Output: one MSC frame of bytes every 400 ms, the streams back to back.

use super::cellmap::{CellMap, MSC};
use super::fac::{depuncture, interleaver, prbs, punct_ones, punct_period, puncture, FacInfo, TAIL};
use super::ofdm::Frame;
use super::sdc::SdcInfo;
use crate::dab::fec::{encode, Viterbi};
use crate::dsp::C32;
use std::collections::VecDeque;

/// 16-QAM axis levels by (y0 << 1 | y1)
const QAM16: [f32; 4] = [0.948_683_3, -0.316_227_8, 0.316_227_8, -0.948_683_3];
/// Code rates (level 0, level 1) for protection levels 0..1 of 16-QAM (Dream iCodRateCombMSC16SM)
const RATES16: [[usize; 3]; 2] = [[2, 7, 3], [4, 9, 4]];
const LONG_DEPTH: usize = 5;

pub struct MscDecoder {
    viterbi: Viterbi,
    n_mux: usize,
    depth: usize,
    table: Vec<usize>,
    mem: Vec<Vec<(C32, f32)>>,
    cur: Vec<usize>,
    fifo: VecDeque<(C32, f32)>,
    aligned: bool,
    pub frames: u64,
}

impl Default for MscDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MscDecoder {
    pub fn new() -> MscDecoder {
        MscDecoder { viterbi: Viterbi::new(), n_mux: 0, depth: 0, table: Vec::new(), mem: Vec::new(), cur: Vec::new(), fifo: VecDeque::new(), aligned: false, frames: 0 }
    }

    fn setup(&mut self, n_mux: usize, depth: usize) {
        if n_mux == self.n_mux && depth == self.depth {
            return;
        }
        self.n_mux = n_mux;
        self.depth = depth;
        self.table = interleaver(n_mux, 5);
        self.mem = vec![vec![(C32::default(), 0.0); n_mux]; LONG_DEPTH];
        self.cur = (0..LONG_DEPTH).collect();
        self.fifo.clear();
        self.aligned = false;
    }

    /// One transmission frame (its position from the FAC). Returns the MSC frames completed.
    pub fn push(&mut self, map: &CellMap, f: &Frame, fac: &FacInfo, sdc: &SdcInfo) -> Vec<Vec<u8>> {
        self.setup(map.msc_per_frame, if fac.long_interleave { LONG_DEPTH } else { 1 });
        let fid = fac.frame_id as usize;
        if fid == 0 {
            self.aligned = true;
            self.fifo.clear();
        }
        if !self.aligned {
            return Vec::new();
        }
        let spf = map.sym_per_frame;
        for s in 0..spf {
            let row = &map.kind[fid * spf + s];
            for (c, &k) in row.iter().enumerate() {
                if k == MSC {
                    self.fifo.push_back((f.cells[s][c], f.csi[s][c]));
                }
            }
        }
        if fid == 2 {
            // The superframe ends with dummy cells
            let keep = (self.fifo.len() / self.n_mux) * self.n_mux;
            self.fifo.truncate(keep);
        }
        let mut out = Vec::new();
        while self.fifo.len() >= self.n_mux {
            let cells: Vec<(C32, f32)> = self.fifo.drain(..self.n_mux).collect();
            let de = self.deinterleave(&cells);
            self.frames += 1;
            if let Some(bytes) = self.mlc(fac, sdc, &de) {
                out.push(bytes);
            }
        }
        out
    }

    fn deinterleave(&mut self, cells: &[(C32, f32)]) -> Vec<(C32, f32)> {
        let d = self.depth;
        for (i, &c) in cells.iter().enumerate() {
            self.mem[self.cur[i % d]][self.table[i]] = c;
        }
        let out = self.mem[self.cur[d - 1]].clone();
        for c in self.cur.iter_mut().take(d) {
            *c = if *c == 0 { d - 1 } else { *c - 1 };
        }
        out
    }

    /// Multilevel decoding of one MSC frame (standard mapping, 16- or 64-QAM)
    fn mlc(&self, fac: &FacInfo, sdc: &SdcInfo, cells: &[(C32, f32)]) -> Option<Vec<u8>> {
        let qam = match fac.msc_mode {
            3 => Qam::Q16,
            0 => Qam::Q64,
            _ => return None, // hierarchical modulation: not supported
        };
        let len_a: usize = sdc.streams.iter().map(|s| s.0 as usize).sum();
        let bits = mlc_decode(&self.viterbi, qam, cells, len_a, sdc.prot.0 as usize, sdc.prot.1 as usize)?;
        Some(bits.chunks(8).filter(|c| c.len() == 8).map(|c| c.iter().fold(0u8, |a, &x| (a << 1) | x)).collect())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Qam {
    Q4,
    Q16,
    Q64,
}

impl Qam {
    fn levels(self) -> usize {
        match self {
            Qam::Q4 => 1,
            Qam::Q16 => 2,
            Qam::Q64 => 3,
        }
    }
    /// Axis value by index y0 y1 y2 (y0 = most significant)
    fn table(self) -> &'static [f32] {
        match self {
            Qam::Q4 => &[std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2],
            Qam::Q16 => &QAM16,
            Qam::Q64 => &[1.080_123_4, -0.154_303_35, 0.462_910_05, -0.771_516_75, 0.771_516_75, -0.462_910_05, 0.154_303_35, -1.080_123_4],
        }
    }
    /// Bit interleaver t0 per level (None: no interleaver)
    fn interleaver(self, lvl: usize) -> Option<usize> {
        match (self, lvl) {
            (Qam::Q4, 0) => Some(21),
            (Qam::Q16, 0) => Some(13),
            (Qam::Q16, 1) => Some(21),
            (Qam::Q64, 1) => Some(13),
            (Qam::Q64, 2) => Some(21),
            _ => None,
        }
    }
}

/// MSC code rates per protection level: (R per level, RY_lcm)
const RATES64: [([usize; 3], usize); 4] = [([0, 4, 9], 4), ([2, 7, 10], 15), ([4, 9, 11], 8), ([7, 10, 12], 45)];

fn msc_rates(qam: Qam, prot: usize) -> (Vec<usize>, usize) {
    match qam {
        Qam::Q16 => (RATES16[prot & 1][..2].to_vec(), RATES16[prot & 1][2]),
        Qam::Q64 => (RATES64[prot & 3].0.to_vec(), RATES64[prot & 3].1),
        Qam::Q4 => (vec![4], 1),
    }
}

/// Multilevel decoding (MSC): part A (`len_a` bytes, protection `pa`) then part B (`pb`)
pub fn mlc_decode(viterbi: &Viterbi, qam: Qam, cells: &[(C32, f32)], len_a: usize, pa: usize, pb: usize) -> Option<Vec<u8>> {
    let n = cells.len();
    let (ra, ry_lcm) = msc_rates(qam, pa);
    let (rb, _) = msc_rates(qam, pb);
    let r = |rate: usize| punct_period(rate) as f64 / punct_ones(rate) as f64;
    let n1 = if len_a == 0 { 0 } else { ((8.0 * len_a as f64) / (2.0 * ry_lcm as f64 * ra.iter().map(|&x| r(x)).sum::<f64>())).ceil() as usize * ry_lcm };
    if n1 >= n {
        return None;
    }
    levels_decode(viterbi, qam, cells, n1, &ra, &rb)
}

/// Decodes all levels: `n1` cells of part A at rates `ra`, the rest at rates `rb`.
/// Returns part A of each level, then part B of each level, energy dispersal removed.
pub fn levels_decode(viterbi: &Viterbi, qam: Qam, cells: &[(C32, f32)], n1: usize, ra: &[usize], rb: &[usize]) -> Option<Vec<u8>> {
    let n = cells.len();
    let n2 = n - n1;
    if 2 * n2 < 12 {
        return None;
    }
    let nl = qam.levels();
    let table = qam.table();
    let rr = |rate: usize| punct_period(rate) as f64 / punct_ones(rate) as f64;
    let m: Vec<(usize, usize)> = (0..nl)
        .map(|l| {
            let ma = (2.0 * n1 as f64 * rr(ra[l])) as usize;
            let mb = punct_period(rb[l]) * ((2 * n2 - 12) / punct_ones(rb[l]));
            (ma, mb)
        })
        .collect();
    let mean_csi = cells.iter().map(|c| c.1).sum::<f32>() / n as f32;
    let scale = 150.0 / mean_csi.max(1e-20);
    let parts = [(0, 2 * n1), (2 * n1, 2 * n2)];
    // Coded bit of each level on every cell axis, once that level is decided
    let mut y: Vec<Option<Vec<u8>>> = vec![None; nl];
    let mut decoded: Vec<Vec<u8>> = vec![Vec::new(); nl];
    // First pass level by level, then one iteration where every level sees the others
    let passes = if nl > 1 { 2 } else { 1 };
    for pass in 0..passes {
        for lvl in 0..nl {
            let shift = nl - 1 - lvl;
            let mut soft = vec![0i8; 2 * n];
            for (i, &(v, csi)) in cells.iter().enumerate() {
                for (ax, x) in [v.re, v.im].into_iter().enumerate() {
                    let k = 2 * i + ax;
                    let (mut t0, mut t1) = (f32::MAX, f32::MAX);
                    'pts: for (idx, &p) in table.iter().enumerate() {
                        // Other levels: decided ones must match, undecided ones are free
                        for (ol, yo) in y.iter().enumerate() {
                            if ol == lvl {
                                continue;
                            }
                            if let Some(yo) = yo {
                                if (idx >> (nl - 1 - ol)) & 1 != yo[k] as usize {
                                    continue 'pts;
                                }
                            }
                        }
                        let d = (x - p) * (x - p);
                        if (idx >> shift) & 1 == 0 {
                            t0 = t0.min(d);
                        } else {
                            t1 = t1.min(d);
                        }
                    }
                    soft[k] = ((t1 - t0) * csi * scale).clamp(-127.0, 127.0) as i8;
                }
            }
            let mut de = soft.clone();
            if let Some(t0) = qam.interleaver(lvl) {
                for (base, len) in parts {
                    if len > 0 {
                        for (i, &t) in interleaver(len, t0).iter().enumerate() {
                            de[base + t] = soft[base + i];
                        }
                    }
                }
            }
            let (ma, mb) = m[lvl];
            let ry = punct_ones(rb[lvl]);
            let tail = (2 * n2 - 12) - ry * ((2 * n2 - 12) / ry);
            let mother = depuncture_ab(&de, ra[lvl], ma, rb[lvl], mb, &TAIL[tail]);
            let bits = viterbi.decode(&mother);
            if pass + 1 < passes || lvl + 1 < nl {
                // Re-encode: this level's bit on every cell axis, for the other levels
                let coded = puncture_ab(&encode(&bits), ra[lvl], ma, rb[lvl], mb, &TAIL[tail]);
                let mut yl = coded.clone();
                if let Some(t0) = qam.interleaver(lvl) {
                    for (base, len) in parts {
                        if len > 0 {
                            for (i, &t) in interleaver(len, t0).iter().enumerate() {
                                yl[base + i] = coded[base + t];
                            }
                        }
                    }
                }
                yl.resize(2 * n, 0);
                y[lvl] = Some(yl);
            }
            decoded[lvl] = bits;
        }
    }
    let mut all = Vec::new();
    for (lvl, d) in decoded.iter().enumerate() {
        all.extend_from_slice(&d[..m[lvl].0]);
    }
    for (lvl, d) in decoded.iter().enumerate() {
        all.extend_from_slice(&d[m[lvl].0..m[lvl].0 + m[lvl].1]);
    }
    let pr = prbs(all.len());
    for (b, p) in all.iter_mut().zip(pr) {
        *b ^= p;
    }
    Some(all)
}

/// Depuncturing with part A's pattern for the first `ma` steps, part B's after
fn depuncture_ab(soft: &[i8], rate_a: usize, ma: usize, rate_b: usize, mb: usize, tail: &[u8; 6]) -> Vec<i8> {
    if ma == 0 {
        return depuncture(soft, rate_b, mb, Some(tail));
    }
    let used_a = (0..ma).map(|s| super::fac::mask(rate_a, s).count_ones() as usize).sum::<usize>();
    let a = depuncture(&soft[..used_a], rate_a, ma, None);
    let b = depuncture(&soft[used_a..], rate_b, mb, Some(tail));
    let mut out = a[..4 * ma].to_vec();
    out.extend_from_slice(&b);
    out
}

fn puncture_ab(mother: &[u8], rate_a: usize, ma: usize, rate_b: usize, mb: usize, tail: &[u8; 6]) -> Vec<u8> {
    if ma == 0 {
        return puncture(mother, rate_b, mb, Some(tail));
    }
    let mut out: Vec<u8> = Vec::new();
    for s in 0..ma {
        let mk = super::fac::mask(rate_a, s);
        for b in 0..4 {
            if mk & (1 << b) != 0 {
                out.push(mother[4 * s + b]);
            }
        }
    }
    out.extend(puncture(&mother[4 * ma..], rate_b, mb, Some(tail)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a transmitter does: dispersal, per-level coding, puncturing, interleaving, mapping
    fn transmit(qam: Qam, data: &[u8], n: usize, rb: &[usize], seed: &mut u32) -> Vec<(C32, f32)> {
        let nl = qam.levels();
        let ms: Vec<usize> = (0..nl).map(|l| punct_period(rb[l]) * ((2 * n - 12) / punct_ones(rb[l]))).collect();
        let pr = prbs(data.len());
        let scrambled: Vec<u8> = data.iter().zip(pr).map(|(a, b)| a ^ b).collect();
        let mut y: Vec<Vec<u8>> = Vec::new();
        let mut off = 0;
        for l in 0..nl {
            let bits = &scrambled[off..off + ms[l]];
            off += ms[l];
            let ry = punct_ones(rb[l]);
            let tail = (2 * n - 12) - ry * ((2 * n - 12) / ry);
            let coded = puncture(&encode(bits), rb[l], ms[l], Some(&TAIL[tail]));
            assert_eq!(coded.len(), 2 * n, "level {l}");
            let mut yl = coded.clone();
            if let Some(t0) = qam.interleaver(l) {
                for (i, &t) in interleaver(2 * n, t0).iter().enumerate() {
                    yl[i] = coded[t];
                }
            }
            y.push(yl);
        }
        let mut rnd = || {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed as f32 / u32::MAX as f32) - 0.5
        };
        (0..n)
            .map(|i| {
                let axis = |k: usize| {
                    let idx = (0..nl).fold(0usize, |a, l| (a << 1) | y[l][k] as usize);
                    qam.table()[idx]
                };
                (C32::new(axis(2 * i) + 0.12 * rnd(), axis(2 * i + 1) + 0.12 * rnd()), 1.0)
            })
            .collect()
    }

    #[test]
    fn multilevel_round_trip() {
        let v = Viterbi::new();
        for (qam, rb) in [(Qam::Q16, vec![2, 7]), (Qam::Q64, vec![0, 4, 9]), (Qam::Q64, vec![4, 9, 11]), (Qam::Q4, vec![4])] {
            let n = 900;
            let total: usize = rb.iter().map(|&r| punct_period(r) * ((2 * n - 12) / punct_ones(r))).sum();
            let mut seed = 99u32;
            let data: Vec<u8> = (0..total).map(|i| ((i * 7919 + 13) % 5 % 2) as u8).collect();
            let cells = transmit(qam, &data, n, &rb, &mut seed);
            let out = levels_decode(&v, qam, &cells, 0, &rb, &rb).unwrap();
            assert_eq!(out.len(), data.len());
            let errors = out.iter().zip(&data).filter(|(a, b)| a != b).count();
            assert_eq!(errors, 0, "{qam:?} {rb:?}: {errors} bit errors");
        }
    }
}
