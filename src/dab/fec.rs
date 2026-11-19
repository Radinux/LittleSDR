//! DAB channel coding (ETSI EN 300 401 clause 11): rate 1/4, K=7 convolutional code
//! terminated by 6 tail bits, punctured with the 24 PI vectors; energy dispersal; CRCs.
//!
//! Soft bits: negative = 1, 0 = erased (punctured), as everywhere in this engine.

/// Generators 133, 171, 145, 133 (octal), as masks on a 7-bit register whose newest bit is
/// bit 6 (same convention as the LRPT code)
const POLYS: [u32; 4] = [0o133, 0o171, 0o145, 0o133];
const STATES: usize = 64;

/// Puncturing vectors PI_1..PI_24 (clause 11.1.2): 32 bits each, 1 = transmitted
#[rustfmt::skip]
const PI_TABLE: [[u8; 32]; 24] = [
    [1,1,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0],
    [1,1,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0],
    [1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,0,0,0, 1,0,0,0],
    [1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0],
    [1,1,0,0, 1,1,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,0,0,0],
    [1,1,0,0, 1,1,0,0, 1,1,0,0, 1,0,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,0,0,0],
    [1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,0,0,0],
    [1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0],
    [1,1,1,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,0,0],
    [1,1,1,0, 1,1,0,0, 1,1,0,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,0,0, 1,1,0,0],
    [1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,0,0, 1,1,0,0],
    [1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0],
    [1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,0,0],
    [1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,0,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,0,0],
    [1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,0,0],
    [1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0],
    [1,1,1,1, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,0],
    [1,1,1,1, 1,1,1,0, 1,1,1,0, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,0, 1,1,1,0],
    [1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,0, 1,1,1,0],
    [1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0],
    [1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,0],
    [1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,0, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,0],
    [1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,0],
    [1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1, 1,1,1,1],
];

/// The tail: 24 coded bits of the 6 tail bits, half transmitted
const PI_X: [u8; 24] = [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0];

/// Re-inserts erasures: `blocks` = (number of 128-bit blocks, PI index) in order, then the
/// tail. Returns the mother-code soft bits (4 per data bit + 24 tail), and how many input
/// bits were used.
pub fn depuncture(input: &[i8], blocks: &[(usize, usize)]) -> (Vec<i8>, usize) {
    let total: usize = blocks.iter().map(|b| b.0).sum::<usize>() * 128 + 24;
    let mut out = vec![0i8; total];
    let (mut o, mut k) = (0usize, 0usize);
    for &(count, p) in blocks {
        let v = PI_TABLE[p - 1];
        for _ in 0..count * 4 {
            for &keep in &v {
                if keep != 0 {
                    out[o] = input.get(k).copied().unwrap_or(0);
                    k += 1;
                }
                o += 1;
            }
        }
    }
    for &keep in &PI_X {
        if keep != 0 {
            out[o] = input.get(k).copied().unwrap_or(0);
            k += 1;
        }
        o += 1;
    }
    (out, k)
}

#[inline]
fn parity(x: u32) -> u8 {
    (x.count_ones() & 1) as u8
}

/// Encoder (tests): bits -> 4 coded bits each, plus the 24 of the zero tail
pub fn encode(bits: &[u8]) -> Vec<u8> {
    let mut st = 0u32;
    let mut out = Vec::with_capacity(bits.len() * 4 + 24);
    for &b in bits.iter().chain([0u8; 6].iter()) {
        let reg = (st >> 1) | ((b as u32) << 6);
        for p in POLYS {
            out.push(parity(reg & p));
        }
        st = reg & 0x7F;
    }
    out
}

/// Viterbi for the terminated rate-1/4 code: starts and ends in state 0
pub struct Viterbi {
    /// Expected sign of each output, for (previous state, input bit)
    sign: Vec<[[i8; 4]; 2]>,
}

impl Default for Viterbi {
    fn default() -> Self {
        Self::new()
    }
}

impl Viterbi {
    pub fn new() -> Viterbi {
        let mut sign = vec![[[0i8; 4]; 2]; STATES];
        for (s, row) in sign.iter_mut().enumerate() {
            for b in 0..2 {
                let reg = ((b as u32) << 6) | s as u32;
                for (j, p) in POLYS.iter().enumerate() {
                    row[b][j] = if parity(reg & p) == 1 { -1 } else { 1 };
                }
            }
        }
        Viterbi { sign }
    }

    /// `soft`: 4 per data bit + 24 for the tail. Returns the data bits (0/1).
    pub fn decode(&self, soft: &[i8]) -> Vec<u8> {
        let nbits = soft.len() / 4 - 6;
        let steps = nbits + 6;
        const NEG: i32 = i32::MIN / 4;
        let mut metric = [NEG; STATES];
        metric[0] = 0;
        let mut next = [0i32; STATES];
        let mut dec: Vec<u64> = Vec::with_capacity(steps);
        for t in 0..steps {
            let s = &soft[4 * t..4 * t + 4];
            let (s0, s1, s2, s3) = (s[0] as i32, s[1] as i32, s[2] as i32, s[3] as i32);
            let mut d = 0u64;
            for ns in 0..STATES {
                let b = ns >> 5;
                let p0 = (ns << 1) & (STATES - 1);
                let p1 = p0 | 1;
                let bm = |p: usize| {
                    let g = &self.sign[p][b];
                    g[0] as i32 * s0 + g[1] as i32 * s1 + g[2] as i32 * s2 + g[3] as i32 * s3
                };
                let (m0, m1) = (metric[p0] + bm(p0), metric[p1] + bm(p1));
                if m1 > m0 {
                    next[ns] = m1;
                    d |= 1 << ns;
                } else {
                    next[ns] = m0;
                }
            }
            dec.push(d);
            let best = *next.iter().max().unwrap();
            for (m, n) in metric.iter_mut().zip(next.iter()) {
                *m = (n - best).max(NEG);
            }
        }
        // Terminated: trace back from state 0
        let mut state = 0usize;
        let mut bits = vec![0u8; steps];
        for t in (0..steps).rev() {
            bits[t] = (state >> 5) as u8;
            let d = ((dec[t] >> state) & 1) as usize;
            state = ((state << 1) & (STATES - 1)) | d;
        }
        bits.truncate(nbits);
        bits
    }
}

/// Energy dispersal sequence (x^9 + x^5 + 1, all ones), XORed over each decoded block
pub fn prbs(len: usize) -> Vec<u8> {
    let mut sr = [1u8; 9];
    (0..len)
        .map(|_| {
            let b = sr[8] ^ sr[4];
            sr.copy_within(0..8, 1);
            sr[0] = b;
            b
        })
        .collect()
}

/// Packs bits (MSB first) into bytes
pub fn pack(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b) << (8 - c.len())).collect()
}

/// CRC-16 CCITT (x^16 + x^12 + x^5 + 1), initial all ones, transmitted inverted: FIBs and
/// DAB+ access units
pub fn crc16_ok(data: &[u8]) -> bool {
    let n = data.len();
    if n < 2 {
        return false;
    }
    let mut crc: u16 = 0xFFFF;
    for &b in &data[..n - 2] {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    !crc == u16::from_be_bytes([data[n - 2], data[n - 1]])
}

/// DAB+ superframe fire code (x^16 + x^14 + x^13 + x^12 + x^11 + x^5 + x^3 + x^2 + x + 1)
pub fn firecode(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x782F } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puncturing_vectors_have_8_plus_n_ones() {
        for n in 1..=24 {
            assert_eq!(PI_TABLE[n - 1].iter().filter(|&&b| b == 1).count(), 8 + n, "PI_{n}");
        }
    }

    #[test]
    fn viterbi_decodes_a_punctured_fic_block() {
        // A FIC block: 768 bits, 21 x 128 with PI_16, 3 x 128 with PI_15, tail: 2304 bits sent
        let mut seed = 7u32;
        let bits: Vec<u8> = (0..768)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed & 1) as u8
            })
            .collect();
        let coded = encode(&bits);
        let soft_all: Vec<i8> = coded.iter().map(|&b| if b == 1 { -60 } else { 60 }).collect();
        // Puncture like the transmitter
        let mut sent = Vec::new();
        let mut o = 0;
        for &(count, p) in &[(21usize, 16usize), (3, 15)] {
            for _ in 0..count * 4 {
                for &k in &PI_TABLE[p - 1] {
                    if k == 1 {
                        sent.push(soft_all[o]);
                    }
                    o += 1;
                }
            }
        }
        for &k in &PI_X {
            if k == 1 {
                sent.push(soft_all[o]);
            }
            o += 1;
        }
        assert_eq!(sent.len(), 2304);
        // Flip a few
        for i in (0..2304).step_by(97) {
            sent[i] = -sent[i];
        }
        let (mother, used) = depuncture(&sent, &[(21, 16), (3, 15)]);
        assert_eq!(used, 2304);
        assert_eq!(Viterbi::new().decode(&mother), bits);
    }

    #[test]
    fn crc_and_prbs() {
        // PRBS starts 0000 0111 1011 (EN 300 401 clause 10)
        let p = prbs(12);
        assert_eq!(p, vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 1]);
        // A FIB-like block with its CRC
        let mut d: Vec<u8> = (0..30u8).collect();
        let mut crc: u16 = 0xFFFF;
        for &b in &d {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        d.extend_from_slice(&(!crc).to_be_bytes());
        assert!(crc16_ok(&d));
        d[3] ^= 1;
        assert!(!crc16_ok(&d));
    }
}
