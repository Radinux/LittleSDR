//! CRC-14 and the LDPC (174,91) code: encoder (for tests and signal subtraction) and a
//! belief-propagation (sum-product) decoder.

use super::tables::{GENERATOR, MN, NM};
use std::sync::OnceLock;

/// CRC-14 (polynomial 0x2757) of the 77 message bits, as WSJT-X: computed over 96 bits
/// = the message, 5 zero bits, then the 14 zeroed CRC bits.
pub fn crc14(msg77: &[u8]) -> u16 {
    let mut bits = [0u8; 96];
    bits[..77].copy_from_slice(&msg77[..77]);
    let mut r: u32 = 0;
    for &b in &bits[..96] {
        r = (r << 1) | b as u32;
        if r & (1 << 14) != 0 {
            r ^= 0x6757;
        }
    }
    (r & 0x3FFF) as u16
}

/// 91 bits: message + CRC
pub fn add_crc(msg77: &[u8]) -> [u8; 91] {
    let mut a = [0u8; 91];
    a[..77].copy_from_slice(&msg77[..77]);
    let c = crc14(msg77);
    for i in 0..14 {
        a[77 + i] = (c >> (13 - i)) as u8 & 1;
    }
    a
}

pub fn crc_ok(a91: &[u8]) -> bool {
    let c = crc14(&a91[..77]);
    (0..14).all(|i| a91[77 + i] == (c >> (13 - i)) as u8 & 1)
}

fn generator() -> &'static [[u8; 91]; 83] {
    static G: OnceLock<[[u8; 91]; 83]> = OnceLock::new();
    G.get_or_init(|| {
        let mut g = [[0u8; 91]; 83];
        for (i, row) in GENERATOR.iter().enumerate() {
            for (j, c) in row.chars().enumerate() {
                let v = c.to_digit(16).unwrap();
                for k in 0..4 {
                    let col = 4 * j + k;
                    if col < 91 {
                        g[i][col] = (v >> (3 - k)) as u8 & 1;
                    }
                }
            }
        }
        g
    })
}

/// 174-bit codeword: the 91 bits then 83 parity bits
pub fn encode(a91: &[u8; 91]) -> [u8; 174] {
    let mut cw = [0u8; 174];
    cw[..91].copy_from_slice(a91);
    for (i, row) in generator().iter().enumerate() {
        let s: u32 = row.iter().zip(a91).map(|(g, m)| (g & m) as u32).sum();
        cw[91 + i] = (s & 1) as u8;
    }
    cw
}

/// Number of unsatisfied parity checks
pub fn parity_errors(cw: &[u8]) -> usize {
    NM.iter().filter(|row| row.iter().filter(|&&n| n != 255).fold(0u8, |a, &n| a ^ cw[n as usize]) != 0).count()
}

/// Belief propagation on log-likelihood ratios (`llr` > 0 means bit 1). Returns the hard
/// decisions and the number of checks still failing (0 = a codeword).
pub fn decode(llr: &[f32; 174], max_iter: usize) -> ([u8; 174], usize) {
    // Internally λ = log P(0)/P(1)
    let lam: Vec<f32> = llr.iter().map(|&l| -l).collect();
    // Messages per (check, slot in NM row)
    let mut tov = [[0f32; 7]; 83]; // check -> variable
    let mut toc = [[0f32; 7]; 83]; // variable -> check
    for (m, row) in NM.iter().enumerate() {
        for (k, &n) in row.iter().enumerate() {
            if n != 255 {
                toc[m][k] = lam[n as usize];
            }
        }
    }
    let mut best = ([0u8; 174], usize::MAX);
    for _ in 0..max_iter {
        // Check nodes: tanh rule
        for m in 0..83 {
            let row = &NM[m];
            let t: Vec<f32> = (0..7).map(|k| if row[k] == 255 { 1.0 } else { (0.5 * toc[m][k]).tanh() }).collect();
            for k in 0..7 {
                if row[k] == 255 {
                    continue;
                }
                let mut p = 1.0f32;
                for (kk, &v) in t.iter().enumerate() {
                    if kk != k {
                        p *= v;
                    }
                }
                let p = p.clamp(-0.999_999, 0.999_999);
                tov[m][k] = 2.0 * p.atanh();
            }
        }
        // Variable nodes
        let mut hard = [0u8; 174];
        let mut total = [0f32; 174];
        for n in 0..174 {
            let mut s = lam[n];
            for &m in &MN[n] {
                let k = NM[m as usize].iter().position(|&x| x as usize == n).unwrap();
                s += tov[m as usize][k];
            }
            total[n] = s;
            hard[n] = (s < 0.0) as u8;
        }
        let errs = parity_errors(&hard);
        if errs < best.1 {
            best = (hard, errs);
        }
        if errs == 0 {
            return best;
        }
        for (m, row) in NM.iter().enumerate() {
            for (k, &n) in row.iter().enumerate() {
                if n != 255 {
                    toc[m][k] = total[n as usize] - tov[m][k];
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codewords_satisfy_the_checks() {
        let mut seed = 5u32;
        for _ in 0..20 {
            let mut m = [0u8; 77];
            for b in m.iter_mut() {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *b = (seed >> 16) as u8 & 1;
            }
            let a = add_crc(&m);
            assert!(crc_ok(&a));
            let cw = encode(&a);
            assert_eq!(parity_errors(&cw), 0, "generator and check tables disagree");
        }
    }

    #[test]
    fn decodes_through_noise() {
        let mut seed = 77u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 + 0.5) / 4_294_967_296.0
        };
        let mut ok = 0;
        for _ in 0..50 {
            let mut m = [0u8; 77];
            for b in m.iter_mut() {
                *b = (rnd() < 0.5) as u8;
            }
            let cw = encode(&add_crc(&m));
            // BPSK at Eb/N0 ~ 2.9 dB: sigma 0.7
            let mut llr = [0f32; 174];
            for i in 0..174 {
                let (u, v) = (rnd(), rnd());
                let g = (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos();
                let y = if cw[i] == 1 { 1.0 } else { -1.0 } + 0.7 * g;
                llr[i] = 2.0 * y / (0.7 * 0.7);
            }
            let (d, e) = decode(&llr, 30);
            if e == 0 && d == cw {
                ok += 1;
            }
        }
        assert!(ok >= 45, "{ok}/50 decoded");
    }
}
