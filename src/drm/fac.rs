//! Fast Access Channel: 65 4-QAM cells per frame, 72 bits: frame identity, spectrum
//! occupancy, interleaving, MSC/SDC modulations, service ID, language, audio/data.
//! Shared coding blocks (puncturing, bit interleaver, energy dispersal, CRC) are here too.

use super::cellmap::{CellMap, FAC};
use super::ofdm::Frame;
use crate::dab::fec::Viterbi;

/// Puncturing patterns (Dream TableMLC.h): per input step, which of the 4 mother bits are
/// sent. Index = code rate number of the standard's tables.
const PUNCT: [&[u8]; 13] = [
    &[0b1111],
    &[0b1111, 0b0111, 0b0111],
    &[0b0111],
    &[0b0111, 0b0111, 0b0111, 0b0011],
    &[0b0011],
    &[0b0011, 0b0101, 0b0011, 0b0001],
    &[0b0011, 0b0001, 0b0011],
    &[0b0011, 0b0001],
    &[0b0011, 0b0001, 0b0001, 0b0011, 0b0001, 0b0001, 0b0011, 0b0001],
    &[0b0011, 0b0001, 0b0001],
    &[0b0011, 0b0001, 0b0001, 0b0001],
    &[0b0011, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001],
    &[0b0011, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001],
];

/// Puncturing of the 6 tail steps, by the standard's r_p index
pub const TAIL: [[u8; 6]; 12] = [
    [0b0011; 6],
    [0b0111, 0b0011, 0b0011, 0b0011, 0b0011, 0b0011],
    [0b0111, 0b0011, 0b0011, 0b0111, 0b0011, 0b0011],
    [0b0111, 0b0111, 0b0011, 0b0111, 0b0011, 0b0011],
    [0b0111, 0b0111, 0b0011, 0b0111, 0b0111, 0b0011],
    [0b0111, 0b0111, 0b0111, 0b0111, 0b0111, 0b0011],
    [0b0111; 6],
    [0b1111, 0b0111, 0b0111, 0b0111, 0b0111, 0b0111],
    [0b1111, 0b0111, 0b0111, 0b1111, 0b0111, 0b0111],
    [0b1111, 0b1111, 0b0111, 0b1111, 0b0111, 0b0111],
    [0b1111, 0b1111, 0b0111, 0b1111, 0b0111, 0b1111],
    [0b1111, 0b1111, 0b1111, 0b1111, 0b0111, 0b1111],
];

/// Number of mother bits sent per pattern period
pub fn punct_ones(rate: usize) -> usize {
    PUNCT[rate].iter().map(|p| p.count_ones() as usize).sum()
}

pub fn punct_period(rate: usize) -> usize {
    PUNCT[rate].len()
}

/// Depunctures `soft` into 4 values per step (0 = erased) for `steps` data steps followed
/// by the 6 tail steps punctured with `tail` (or the data pattern when None, as the FAC).
pub fn depuncture(soft: &[i8], rate: usize, steps: usize, tail: Option<&[u8; 6]>) -> Vec<i8> {
    let pat = PUNCT[rate];
    let mut out = vec![0i8; 4 * (steps + 6)];
    let mut i = 0;
    for st in 0..steps + 6 {
        let mask = if st < steps { pat[st % pat.len()] } else { tail.map_or(pat[st % pat.len()], |t| t[st - steps]) };
        for b in 0..4 {
            if mask & (1 << b) != 0 {
                out[4 * st + b] = *soft.get(i).unwrap_or(&0);
                i += 1;
            }
        }
    }
    out
}

/// Bit interleaver permutation of the standard (7.3.3): t0 = 13 or 21
pub fn interleaver(size: usize, t0: usize) -> Vec<usize> {
    let mut s = 1usize;
    while s < size {
        s <<= 1;
    }
    let q = s / 4 - 1;
    let mut t = vec![0usize; size];
    for i in 1..size {
        let mut v = (t0 * t[i - 1] + q) % s;
        while v >= size {
            v = (t0 * v + q) % s;
        }
        t[i] = v;
    }
    t
}

/// Energy dispersal: x^9 + x^5 + 1, all ones at the start of each block
pub fn prbs(len: usize) -> Vec<u8> {
    let mut r: u32 = 0x1FF;
    (0..len)
        .map(|_| {
            let b = ((r >> 4) ^ (r >> 8)) & 1;
            r = ((r << 1) | b) & 0x1FF;
            b as u8
        })
        .collect()
}

/// DRM CRC (as Dream's CCRC): degree 8, 16 (and others) over bits; returns the check value
pub fn crc(bits: &[u8], degree: u32) -> u32 {
    let poly: u32 = match degree {
        8 => (1 << 2) | (1 << 3) | (1 << 4),
        16 => (1 << 5) | (1 << 12),
        6 => (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5),
        5 => (1 << 1) | (1 << 2) | (1 << 4),
        _ => 0,
    };
    let out_bit = 1u32 << degree;
    let mut st: u32 = !0;
    for &b in bits {
        st <<= 1;
        if st & out_bit != 0 {
            st |= 1;
        }
        if b != 0 {
            st ^= 1;
        }
        if st & 1 != 0 {
            st ^= poly;
        }
    }
    (!st) & (out_bit - 1)
}

pub fn bits_to_u32(b: &[u8]) -> u32 {
    b.iter().fold(0, |a, &x| (a << 1) | x as u32)
}

/// Soft value of a 4-QAM axis (bit 0 = +, as the Viterbi wants), weighted by the channel
pub fn soft4(v: f32, csi: f32, scale: f32) -> i8 {
    (v * csi * scale).clamp(-127.0, 127.0) as i8
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FacInfo {
    pub frame_id: u8,
    pub so: u8,
    pub long_interleave: bool,
    /// 0: 64-QAM SM, 1: 64-QAM HMmix, 2: 64-QAM HMsym, 3: 16-QAM
    pub msc_mode: u8,
    /// 0: 16-QAM, 1: 4-QAM
    pub sdc_mode: u8,
    pub services: u8,
    pub reconfig: u8,
    pub service_id: u32,
    pub short_id: u8,
    pub ca: bool,
    pub language: u8,
    pub data: bool,
    pub descriptor: u8,
}

pub const LANGUAGES: [&str; 16] = [
    "", "Arabic", "Bengali", "Chinese (Mandarin)", "Dutch", "English", "French", "German", "Hindi", "Japanese", "Javanese", "Korean",
    "Portuguese", "Russian", "Spanish", "other",
];

pub struct FacDecoder {
    viterbi: Viterbi,
    deint: Vec<usize>,
    pub ok: u64,
    pub bad: u64,
}

impl Default for FacDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FacDecoder {
    pub fn new() -> FacDecoder {
        FacDecoder { viterbi: Viterbi::new(), deint: interleaver(130, 21), ok: 0, bad: 0 }
    }

    /// The FAC of one frame (cells in table order)
    pub fn decode(&mut self, map: &CellMap, f: &Frame) -> Option<FacInfo> {
        let mut soft = Vec::with_capacity(130);
        let mut csis = Vec::with_capacity(65);
        let mut vals = Vec::with_capacity(65);
        for s in 0..map.sym_per_frame {
            for c in 0..map.carriers() {
                if map.kind[s][c] == FAC {
                    vals.push(f.cells[s][c]);
                    csis.push(f.csi[s][c]);
                }
            }
        }
        if vals.len() != 65 {
            return None;
        }
        let mean_csi = csis.iter().sum::<f32>() / 65.0;
        let scale = 60.0 / std::f32::consts::FRAC_1_SQRT_2 / mean_csi.max(1e-20);
        for (v, &c) in vals.iter().zip(&csis) {
            soft.push(soft4(v.re, c, scale));
            soft.push(soft4(v.im, c, scale));
        }
        // Deinterleave: the transmitter sent bit table[i] at position i
        let mut d = vec![0i8; 130];
        for (i, &t) in self.deint.iter().enumerate() {
            d[t] = soft[i];
        }
        let mother = depuncture(&d, 6, 72, None);
        let mut bits = self.viterbi.decode(&mother);
        for (b, p) in bits.iter_mut().zip(prbs(72)) {
            *b ^= p;
        }
        if crc(&bits[..64], 8) != bits_to_u32(&bits[64..72]) {
            self.bad += 1;
            return None;
        }
        self.ok += 1;
        let g = |a: usize, n: usize| bits_to_u32(&bits[a..a + n]);
        Some(FacInfo {
            frame_id: match g(1, 2) {
                1 => 1,
                2 => 2,
                _ => 0,
            },
            so: g(3, 4) as u8,
            long_interleave: g(7, 1) == 0,
            msc_mode: g(8, 2) as u8,
            sdc_mode: g(10, 1) as u8,
            services: g(11, 4) as u8,
            reconfig: g(15, 3) as u8,
            service_id: g(20, 24),
            short_id: g(44, 2) as u8,
            ca: g(46, 1) == 1,
            language: g(47, 4) as u8,
            data: g(51, 1) == 1,
            descriptor: g(52, 5) as u8,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaver_is_a_permutation() {
        for (n, t0) in [(130, 21), (2 * 1000, 13), (777, 21)] {
            let mut t = interleaver(n, t0);
            t.sort();
            assert!(t.iter().enumerate().all(|(i, &v)| i == v));
        }
    }

    #[test]
    fn fac_rate_fills_65_cells() {
        // 72 bits + 6 tail steps at rate 3/5 = 130 bits = 65 4-QAM cells
        let steps = 78;
        let sent: usize = (0..steps).map(|s| PUNCT[6][s % 3].count_ones() as usize).sum();
        assert_eq!(sent, 130);
    }
}
