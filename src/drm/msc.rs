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
        for j in 0..d {
            self.cur[j] = if self.cur[j] == 0 { d - 1 } else { self.cur[j] - 1 };
        }
        out
    }

    /// Multilevel decoding (16-QAM, standard mapping)
    fn mlc(&self, fac: &FacInfo, sdc: &SdcInfo, cells: &[(C32, f32)]) -> Option<Vec<u8>> {
        if fac.msc_mode != 3 {
            return None; // 64-QAM: not yet
        }
        let n = cells.len();
        let len_a: usize = sdc.streams.iter().map(|s| s.0 as usize).sum();
        let (pa, pb) = (sdc.prot.0 as usize & 1, sdc.prot.1 as usize & 1);
        let rate_ab = |p: usize, lvl: usize| RATES16[p][lvl];
        // Cells of part A: N1 = ceil(8 X / (2 RYlcm sum R_p)) * RYlcm
        let ry_lcm = RATES16[pa][2];
        let r = |rate: usize| punct_period(rate) as f64 / punct_ones(rate) as f64;
        let n1 = if len_a == 0 { 0 } else { ((8.0 * len_a as f64) / (2.0 * ry_lcm as f64 * (r(rate_ab(pa, 0)) + r(rate_ab(pa, 1))))).ceil() as usize * ry_lcm };
        let n2 = n - n1;
        let m: Vec<(usize, usize)> = (0..2)
            .map(|lvl| {
                let ma = (2.0 * n1 as f64 * r(rate_ab(pa, lvl))) as usize;
                let rb = rate_ab(pb, lvl);
                let mb = punct_period(rb) * ((2 * n2 - 12) / punct_ones(rb));
                (ma, mb)
            })
            .collect();
        let mean_csi = cells.iter().map(|c| c.1).sum::<f32>() / n as f32;
        let scale = 150.0 / mean_csi.max(1e-20);
        let d2 = |v: f32, p: f32| (v - p) * (v - p);
        let mut decoded: Vec<Vec<u8>> = Vec::new();
        let mut y0_cells: Vec<u8> = Vec::new();
        for lvl in 0..2 {
            // Metrics in cell order: real then imaginary of each cell
            let mut soft = vec![0i8; 2 * n];
            for (i, &(v, csi)) in cells.iter().enumerate() {
                for (ax, x) in [v.re, v.im].into_iter().enumerate() {
                    let k = 2 * i + ax;
                    let (t0, t1) = if lvl == 0 {
                        (d2(x, QAM16[0]).min(d2(x, QAM16[1])), d2(x, QAM16[2]).min(d2(x, QAM16[3])))
                    } else {
                        let b = (y0_cells[k] as usize) << 1;
                        (d2(x, QAM16[b]), d2(x, QAM16[b | 1]))
                    };
                    soft[k] = ((t1 - t0) * csi * scale).clamp(-127.0, 127.0) as i8;
                }
            }
            // Bit deinterleaver: level 0 t0 = 13, level 1 t0 = 21 (parts A and B apart)
            let t0 = if lvl == 0 { 13 } else { 21 };
            let mut de = vec![0i8; 2 * n];
            for (base, len) in [(0, 2 * n1), (2 * n1, 2 * n2)] {
                if len == 0 {
                    continue;
                }
                for (i, &t) in interleaver(len, t0).iter().enumerate() {
                    de[base + t] = soft[base + i];
                }
            }
            // Part A then part B through one encoder: rates switch at the boundary
            let (ma, mb) = m[lvl];
            let (rate_a, rate_b) = (rate_ab(pa, lvl), rate_ab(pb, lvl));
            let ry = punct_ones(rate_b);
            let tail = (2 * n2 - 12) - ry * ((2 * n2 - 12) / ry);
            let mother = depuncture_ab(&de, rate_a, ma, rate_b, mb, &TAIL[tail]);
            let bits = self.viterbi.decode(&mother);
            if lvl == 0 {
                // Re-encode: the level-0 bit of every cell axis, for level 1
                let coded = puncture_ab(&encode(&bits), rate_a, ma, rate_b, mb, &TAIL[tail]);
                let mut y = vec![0u8; 2 * n];
                for (base, len) in [(0, 2 * n1), (2 * n1, 2 * n2)] {
                    if len == 0 {
                        continue;
                    }
                    for (i, &t) in interleaver(len, t0).iter().enumerate() {
                        y[base + i] = coded[base + t];
                    }
                }
                y0_cells = y;
            }
            decoded.push(bits);
        }
        // Departitioning: part A of each level, then part B of each level
        let mut all = Vec::with_capacity(decoded[0].len() + decoded[1].len());
        for lvl in 0..2 {
            all.extend_from_slice(&decoded[lvl][..m[lvl].0]);
        }
        for lvl in 0..2 {
            all.extend_from_slice(&decoded[lvl][m[lvl].0..m[lvl].0 + m[lvl].1]);
        }
        let pr = prbs(all.len());
        for (b, p) in all.iter_mut().zip(pr) {
            *b ^= p;
        }
        Some(all.chunks(8).filter(|c| c.len() == 8).map(|c| c.iter().fold(0u8, |a, &x| (a << 1) | x)).collect())
    }
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
