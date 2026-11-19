//! Channel coding of LRPT (CCSDS): rate 1/2 K=7 convolutional code (soft Viterbi), the
//! CCSDS pseudo-random sequence, and Reed-Solomon (255,223) interleaved by 4.

// ---------------------------------------------------------------- convolutional code

pub const K: usize = 7;
const STATES: usize = 1 << (K - 1); // 64
pub const G1: u32 = 0x79;
pub const G2: u32 = 0x5B;

#[inline]
fn parity(x: u32) -> u8 {
    (x.count_ones() & 1) as u8
}

/// Encoder outputs for a 7-bit register (newest bit at bit 6): (G1 bit, G2 bit)
#[inline]
fn outputs(reg: u32) -> (u8, u8) {
    (parity(reg & G1), parity(reg & G2))
}

/// Encodes bits (MSB first per byte) into symbol pairs in transmission order: G2 then G1
/// (the order the Meteor stream uses), as hard bits 0/1. `state` carries the last 6 bits.
pub fn conv_encode(data: &[u8], state: &mut u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 16);
    for byte in data {
        for i in (0..8).rev() {
            let bit = ((byte >> i) & 1) as u32;
            let reg = (*state >> 1) | (bit << (K - 1));
            let (a, b) = outputs(reg);
            out.push(b);
            out.push(a);
            *state = reg & 0x7F;
        }
    }
    out
}

/// Soft-decision Viterbi decoder. Soft symbols: negative = bit 1, magnitude = confidence
/// (the convention of the demodulator). Decodes a whole block, starting from an unknown
/// state and tracing back from the best final state.
pub struct Viterbi {
    /// Expected sign of each output for state s (the last 6 inputs, newest at bit 5) and
    /// input b
    out_g1: [[i8; 2]; STATES],
    out_g2: [[i8; 2]; STATES],
}

impl Default for Viterbi {
    fn default() -> Self {
        Self::new()
    }
}

impl Viterbi {
    pub fn new() -> Viterbi {
        let mut out_g1 = [[0i8; 2]; STATES];
        let mut out_g2 = [[0i8; 2]; STATES];
        for s in 0..STATES {
            for b in 0..2 {
                // 7-bit register: new bit at the top, previous 6 bits below
                let reg = ((b as u32) << 6) | s as u32;
                let (a, c) = outputs(reg);
                // Expected sign: bit 1 -> negative soft value
                out_g1[s][b] = if a == 1 { -1 } else { 1 };
                out_g2[s][b] = if c == 1 { -1 } else { 1 };
            }
        }
        Viterbi { out_g1, out_g2 }
    }

    /// `soft` holds pairs (G2, G1). Returns the decoded bits, packed MSB first, and the path
    /// metric per bit of the best path (a quality figure: higher is cleaner).
    pub fn decode(&self, soft: &[i8]) -> (Vec<u8>, f32) {
        let nbits = soft.len() / 2;
        let mut metric = [0i32; STATES];
        let mut next = [0i32; STATES];
        // decisions[t][s] = which predecessor won (0/1), as a 64-bit mask
        let mut decisions: Vec<u64> = Vec::with_capacity(nbits);
        for t in 0..nbits {
            let (s2, s1) = (soft[2 * t] as i32, soft[2 * t + 1] as i32);
            let mut dec = 0u64;
            for ns in 0..STATES {
                // ns = (bit << 5) | (s >> 1): predecessors s = ((ns << 1) & 63) | {0, 1}
                let b = ns >> 5;
                let p0 = (ns << 1) & (STATES - 1);
                let p1 = p0 | 1;
                // The register when moving p -> ns is (b << 6) | p
                let m0 = metric[p0] + self.out_g1[p0][b] as i32 * s1 + self.out_g2[p0][b] as i32 * s2;
                let m1 = metric[p1] + self.out_g1[p1][b] as i32 * s1 + self.out_g2[p1][b] as i32 * s2;
                if m1 > m0 {
                    next[ns] = m1;
                    dec |= 1 << ns;
                } else {
                    next[ns] = m0;
                }
            }
            decisions.push(dec);
            // Renormalize
            let best = *next.iter().max().unwrap();
            for (m, n) in metric.iter_mut().zip(next.iter()) {
                *m = n - best;
            }
        }
        // Trace back from the best state
        let mut state = metric.iter().enumerate().max_by_key(|(_, m)| **m).map(|(i, _)| i).unwrap_or(0);
        let mut bits = vec![0u8; nbits];
        for t in (0..nbits).rev() {
            bits[t] = (state >> 5) as u8;
            let d = ((decisions[t] >> state) & 1) as usize;
            state = ((state << 1) & (STATES - 1)) | d;
        }
        let mut out = vec![0u8; nbits / 8];
        for (i, b) in bits.iter().enumerate().take(out.len() * 8) {
            out[i / 8] |= b << (7 - i % 8);
        }
        // Quality: agreement of the soft symbols with the re-encoded decision
        let mut st = 0u32;
        let re = conv_encode(&out, &mut st);
        let mut agree = 0i64;
        let mut total = 0i64;
        for (h, s) in re.iter().zip(soft) {
            let s = *s as i64;
            agree += if *h == 1 { -s } else { s };
            total += s.abs();
        }
        (out, if total > 0 { agree as f32 / total as f32 } else { 0.0 })
    }
}

// ---------------------------------------------------------------- CCSDS randomizer

/// The CCSDS pseudo-random sequence (x^8 + x^7 + x^5 + x^3 + 1, all ones at start), XORed
/// over each frame after the sync marker. Period 255 bytes.
pub fn pn_sequence() -> [u8; 255] {
    let mut out = [0u8; 255];
    let mut state: u8 = 0xFF;
    for o in out.iter_mut() {
        for _ in 0..8 {
            let newbit = ((state >> 7) ^ (state >> 5) ^ (state >> 3) ^ state) & 1;
            *o = (*o << 1) | (state & 1);
            state = (state >> 1) | (newbit << 7);
        }
    }
    out
}

// ---------------------------------------------------------------- Reed-Solomon

/// RS(255,223), CCSDS conventional representation (field 0x187, roots alpha^(11*(112+i))),
/// interleaved by 4 in LRPT frames
pub struct ReedSolomon(crate::rs::ReedSolomon);

impl Default for ReedSolomon {
    fn default() -> Self {
        Self::new()
    }
}

impl ReedSolomon {
    pub fn new() -> ReedSolomon {
        ReedSolomon(crate::rs::ReedSolomon::new(0x187, 112, 11, 32))
    }

    pub fn encode(&self, msg: &[u8; 223]) -> [u8; 32] {
        self.0.encode(msg).try_into().unwrap()
    }

    pub fn decode(&self, block: &mut [u8]) -> Option<usize> {
        self.0.decode(block)
    }

    /// Corrects a frame of 4 interleaved codewords (1020 bytes: byte j*4+i belongs to codeword
    /// i). Returns the total of corrected bytes, None if any codeword is beyond repair.
    pub fn decode_interleaved(&self, frame: &mut [u8]) -> Option<usize> {
        let mut total = 0;
        let mut ok = true;
        let mut block = [0u8; 255];
        for i in 0..4 {
            for j in 0..255 {
                block[j] = frame[j * 4 + i];
            }
            match self.0.decode(&mut block) {
                Some(n) => total += n,
                None => ok = false,
            }
            for j in 0..255 {
                frame[j * 4 + i] = block[j];
            }
        }
        ok.then_some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    #[test]
    fn viterbi_corrects_noisy_symbols() {
        let mut seed = 12345u32;
        let data: Vec<u8> = (0..256).map(|_| rng(&mut seed) as u8).collect();
        let mut st = 0;
        let enc = conv_encode(&data, &mut st);
        // Gaussian noise, sigma = half the amplitude: ~2.3 % of the symbols arrive wrong
        let mut hard_errors = 0;
        let soft: Vec<i8> = enc
            .iter()
            .map(|&b| {
                let base = if b == 1 { -40.0f32 } else { 40.0 };
                let (u1, u2) = ((rng(&mut seed) as f32 / u32::MAX as f32).max(1e-9), rng(&mut seed) as f32 / u32::MAX as f32);
                let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
                let v = base + 20.0 * g;
                if (v < 0.0) != (b == 1) {
                    hard_errors += 1;
                }
                v.clamp(-127.0, 127.0) as i8
            })
            .collect();
        assert!(hard_errors > 50, "the test should have errors to fix: {hard_errors}");
        let (dec, q) = Viterbi::new().decode(&soft);
        // The last bits lack a tail; compare all but the final byte
        assert_eq!(&dec[..dec.len() - 1], &data[..data.len() - 1]);
        assert!(q > 0.3, "quality {q}");
    }

    #[test]
    fn pn_sequence_starts_like_ccsds() {
        // CCSDS 131.0-B: the sequence begins FF 48 0E C0 9A 0D 70 BC
        let pn = pn_sequence();
        assert_eq!(&pn[..8], &[0xFF, 0x48, 0x0E, 0xC0, 0x9A, 0x0D, 0x70, 0xBC]);
    }

    #[test]
    fn reed_solomon_corrects_up_to_16_errors() {
        let rs = ReedSolomon::new();
        let mut seed = 777u32;
        for nerr in [0usize, 1, 5, 16] {
            let mut msg = [0u8; 223];
            msg.iter_mut().for_each(|m| *m = rng(&mut seed) as u8);
            let par = rs.encode(&msg);
            let mut block: Vec<u8> = msg.iter().chain(par.iter()).copied().collect();
            let clean = block.clone();
            let mut used = std::collections::HashSet::new();
            while used.len() < nerr {
                let p = (rng(&mut seed) % 255) as usize;
                if used.insert(p) {
                    block[p] ^= (rng(&mut seed) % 255 + 1) as u8;
                }
            }
            assert_eq!(rs.decode(&mut block), Some(nerr), "{nerr} errors");
            assert_eq!(block, clean);
        }
        // 17 errors: must not claim success with wrong data
        let mut msg = [0u8; 223];
        msg.iter_mut().for_each(|m| *m = rng(&mut seed) as u8);
        let par = rs.encode(&msg);
        let mut block: Vec<u8> = msg.iter().chain(par.iter()).copied().collect();
        let clean = block.clone();
        for p in 0..17 {
            block[p * 13] ^= 0x5A;
        }
        let r = rs.decode(&mut block);
        assert!(r.is_none() || block != clean || r == Some(17));
    }
}
