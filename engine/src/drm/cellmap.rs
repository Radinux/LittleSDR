//! Which cell of the superframe carries what: MSC, SDC, FAC, or a pilot (and its value).
//! Port of Dream's CCellMappingTable::MakeTable (ofdmcellmapping/CellMappingTable.cpp).

use super::tables::{params, RobMode, FAC_CELLS, FRAMES_PER_SUPERFRAME};
use crate::dsp::C32;

pub const MSC: u8 = 1;
pub const SDC: u8 = 2;
pub const FAC: u8 = 4;
pub const SCAT: u8 = 8;
pub const TIME: u8 = 16;
pub const FREQ: u8 = 32;
pub const BOOST: u8 = 64;
pub const DC: u8 = 128;

pub fn is_pilot(t: u8) -> bool {
    t & (SCAT | TIME | FREQ) != 0
}

pub struct CellMap {
    pub mode: RobMode,
    pub so: usize,
    pub kmin: i32,
    pub kmax: i32,
    pub sym_per_frame: usize,
    /// [superframe symbol][carrier - kmin]
    pub kind: Vec<Vec<u8>>,
    pub pilot: Vec<Vec<C32>>,
    /// MSC cells used per frame (the rest of the superframe's are dummies)
    pub msc_per_frame: usize,
    pub sdc_cells: usize,
}

fn polar(amp: f32, phase1024: i32) -> C32 {
    let a = 2.0 * std::f32::consts::PI * phase1024.rem_euclid(1024) as f32 / 1024.0;
    C32::new(amp * a.cos(), amp * a.sin())
}

impl CellMap {
    pub fn new(mode: RobMode, so: usize) -> Option<CellMap> {
        let p = params(mode);
        let (kmin, kmax) = (p.kmin[so], p.kmax[so]);
        if kmin == 0 && kmax == 0 {
            return None;
        }
        let ncar = (kmax - kmin + 1) as usize;
        let nsym = p.sym_per_frame * FRAMES_PER_SUPERFRAME;
        let mut kind = vec![vec![0u8; ncar]; nsym];
        let mut pilot = vec![vec![C32::default(); ncar]; nsym];
        let (fi, ti) = (p.scat_freq_int, p.scat_time_int);
        let half = (fi + 1) / 2; // ceil(fi / 2)
        let pil_amp = 2f32.sqrt();
        let boost_amp = 2.0f32;
        let mut time_i = 0;
        let mut freq_i = 0;
        let mut fac_i = 0;
        for s in 0..nsym {
            let fs = (s % p.sym_per_frame) as i32;
            if fs == 0 {
                fac_i = 0;
            }
            // Start value of p (C truncation towards zero, as Dream)
            let mut scat_p = ((kmin - half - fi * fs.rem_euclid(ti)) as f64 / (fi * ti) as f64) as i32;
            for k in kmin..=kmax {
                let ci = (k - kmin) as usize;
                let mut t = MSC;
                if s < p.sdc_symbols {
                    t = SDC;
                }
                if fac_i < FAC_CELLS && p.fac[fac_i].0 as i32 == fs && p.fac[fac_i].1 as i32 == k {
                    fac_i += 1;
                    t = FAC;
                }
                // Scattered pilots first: time and frequency references take precedence
                if k == half + fi * fs.rem_euclid(ti) + fi * ti * scat_p {
                    scat_p += 1;
                    t = SCAT;
                    let [x, y, k0] = p.scat_const;
                    let n = fs.rem_euclid(y);
                    let m = fs / y;
                    let pp = ((k - k0 - n * x) as f64 / (x * y) as f64) as i32;
                    let idx = n as usize * p.scat_cols + m as usize;
                    let phase = (4 * p.scat_z[idx] + pp * p.scat_w[idx] + pp * pp * (1 + fs) * p.scat_q).rem_euclid(1024);
                    if p.boosted[so].contains(&k) {
                        pilot[s][ci] = polar(boost_amp, phase);
                        t |= BOOST;
                    } else {
                        pilot[s][ci] = polar(pil_amp, phase);
                    }
                }
                if fs == 0 && time_i < p.time_pilots.len() && p.time_pilots[time_i].0 as i32 == k {
                    t = if t & SCAT != 0 { t | TIME } else { TIME };
                    pilot[s][ci] = polar(pil_amp, p.time_pilots[time_i].1 as i32);
                    time_i = if time_i + 1 == p.time_pilots.len() { 0 } else { time_i + 1 };
                }
                if p.freq_pilots[freq_i].0 as i32 == k {
                    t = if t & (TIME | SCAT) != 0 { t | FREQ } else { FREQ };
                    let mut ph = p.freq_pilots[freq_i].1 as i32;
                    // Mode D: the first two frequency pilots flip on odd symbols
                    if mode == RobMode::D && freq_i != 2 && fs % 2 == 1 {
                        ph += 512;
                    }
                    pilot[s][ci] = polar(pil_amp, ph);
                    freq_i = if freq_i == 2 { 0 } else { freq_i + 1 };
                }
                if k == 0 || (mode == RobMode::A && (k == -1 || k == 1)) {
                    t = DC;
                }
                kind[s][ci] = t;
            }
        }
        let msc_total: usize = kind.iter().map(|r| r.iter().filter(|&&t| t == MSC).count()).sum();
        let sdc_cells = kind.iter().map(|r| r.iter().filter(|&&t| t == SDC).count()).sum();
        Some(CellMap { mode, so, kmin, kmax, sym_per_frame: p.sym_per_frame, kind, pilot, msc_per_frame: msc_total / FRAMES_PER_SUPERFRAME, sdc_cells })
    }

    pub fn carriers(&self) -> usize {
        (self.kmax - self.kmin + 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_b_10khz_layout() {
        let m = CellMap::new(RobMode::B, 3).unwrap();
        assert_eq!(m.carriers(), 207);
        let count = |t: u8| m.kind.iter().map(|r| r.iter().filter(|&&x| x == t).count()).sum::<usize>();
        // 65 FAC cells per frame
        assert_eq!(count(FAC), 3 * 65);
        // Every frame starts with the time reference: 19 time pilots (some also scattered)
        let time = m.kind[0].iter().filter(|&&t| t & TIME != 0).count();
        assert_eq!(time, 19);
        // DC carrier unused, frequency pilots in every symbol
        assert!(m.kind.iter().all(|r| r[103] == DC));
        assert!(m.kind.iter().all(|r| r[(16 + 103) as usize] & FREQ != 0));
        // Scattered pilots: every 6 carriers per symbol, shifted by 2 each symbol
        let row1: Vec<i32> = (0..207).filter(|&c| m.kind[1][c] & SCAT != 0).map(|c| c as i32 - 103).collect();
        assert!(row1.contains(&3) && row1.contains(&9) && row1.contains(&-3), "{row1:?}");
        // Standard (ES 201 980, table on MSC capacity): mode B SO 3 has 2 x ... cells; check
        // only that the SDC exists and the MSC is a whole number of cells per frame
        assert!(m.sdc_cells > 0 && m.msc_per_frame > 1000, "{} {}", m.sdc_cells, m.msc_per_frame);
    }
}
