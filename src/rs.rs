//! Reed-Solomon over GF(256) with any field polynomial, first consecutive root, root spacing
//! and number of parity bytes, on full or shortened blocks (Berlekamp-Massey, Chien, Forney).
//! Used by LRPT (CCSDS 255,223) and DAB+ (120,110 shortened from 255,245).
//! The first byte of a block is the highest-degree coefficient.

pub struct ReedSolomon {
    exp: [u8; 512],
    log: [u8; 256],
    genpoly: Vec<u8>,
    nroots: usize,
    fcr: usize,
    prim: usize,
}

impl ReedSolomon {
    /// `gfpoly` e.g. 0x187 (CCSDS) or 0x11D; roots alpha^(prim * (fcr + i)), i < nroots
    pub fn new(gfpoly: u16, fcr: usize, prim: usize, nroots: usize) -> ReedSolomon {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= gfpoly;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        let mut rs = ReedSolomon { exp, log, genpoly: Vec::new(), nroots, fcr, prim };
        let mut g = vec![1u8];
        for i in 0..nroots {
            let root = rs.alpha(prim * (fcr + i));
            let mut ng = vec![0u8; g.len() + 1];
            for (j, &c) in g.iter().enumerate() {
                ng[j] ^= c;
                ng[j + 1] ^= rs.mul(c, root);
            }
            g = ng;
        }
        rs.genpoly = g;
        rs
    }

    fn alpha(&self, e: usize) -> u8 {
        self.exp[e % 255]
    }
    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize]
        }
    }
    fn div(&self, a: u8, b: u8) -> u8 {
        if a == 0 {
            0
        } else {
            self.exp[(self.log[a as usize] as usize + 255 - self.log[b as usize] as usize) % 255]
        }
    }

    /// Parity bytes of a message (its length + nroots <= 255)
    pub fn encode(&self, msg: &[u8]) -> Vec<u8> {
        let n = self.nroots;
        let mut rem = vec![0u8; n];
        for &m in msg {
            let fb = m ^ rem[0];
            rem.rotate_left(1);
            rem[n - 1] = 0;
            if fb != 0 {
                for j in 0..n {
                    rem[j] ^= self.mul(fb, self.genpoly[j + 1]);
                }
            }
        }
        rem
    }

    /// Corrects a block (message + parity, possibly shortened) in place: the number of
    /// corrected bytes, None if beyond repair
    pub fn decode(&self, block: &mut [u8]) -> Option<usize> {
        let (n, nr) = (block.len(), self.nroots);
        let mut s = vec![0u8; nr];
        let mut any = false;
        for (i, si) in s.iter_mut().enumerate() {
            let x = self.alpha(self.prim * (self.fcr + i));
            let mut acc = 0u8;
            for &b in block.iter() {
                acc = self.mul(acc, x) ^ b;
            }
            *si = acc;
            any |= acc != 0;
        }
        if !any {
            return Some(0);
        }
        // Berlekamp-Massey
        let mut lambda = vec![0u8; nr + 1];
        let mut b = vec![0u8; nr + 1];
        lambda[0] = 1;
        b[0] = 1;
        let (mut l, mut m, mut bb) = (0usize, 1usize, 1u8);
        for k in 0..nr {
            let mut d = s[k];
            for i in 1..=l {
                d ^= self.mul(lambda[i], s[k - i]);
            }
            if d == 0 {
                m += 1;
            } else {
                let t = lambda.clone();
                let coef = self.div(d, bb);
                for i in m..=nr {
                    lambda[i] ^= self.mul(coef, b[i - m]);
                }
                if 2 * l <= k {
                    l = k + 1 - l;
                    b = t;
                    bb = d;
                    m = 1;
                } else {
                    m += 1;
                }
            }
        }
        if l > nr / 2 {
            return None;
        }
        // Chien search: position p (0 = first byte) has locator X = alpha^(prim * (n-1-p))
        let mut found = Vec::with_capacity(l);
        for p in 0..n {
            let e = (self.prim * (n - 1 - p)) % 255;
            let xi = self.alpha(255 - e); // X^-1
            let mut acc = 0u8;
            let mut pw = 1u8;
            for &c in lambda.iter().take(l + 1) {
                acc ^= self.mul(c, pw);
                pw = self.mul(pw, xi);
            }
            if acc == 0 {
                found.push((p, e, xi));
            }
        }
        if found.len() != l {
            return None;
        }
        // Forney: e = X^(1-fcr) * omega(X^-1) / lambda'(X^-1)
        let mut omega = vec![0u8; nr];
        for i in 0..nr {
            let mut acc = 0u8;
            for j in 0..=i.min(l) {
                acc ^= self.mul(lambda[j], s[i - j]);
            }
            omega[i] = acc;
        }
        for &(p, e, xi) in &found {
            let mut num = 0u8;
            let mut pw = 1u8;
            for &c in &omega {
                num ^= self.mul(c, pw);
                pw = self.mul(pw, xi);
            }
            let mut den = 0u8;
            let xi2 = self.mul(xi, xi);
            let mut pw = 1u8;
            let mut i = 1;
            while i <= l {
                den ^= self.mul(lambda[i], pw);
                pw = self.mul(pw, xi2);
                i += 2;
            }
            if den == 0 {
                return None;
            }
            let xe = (e as i64 * (1 - self.fcr as i64)).rem_euclid(255) as usize;
            block[p] ^= self.mul(self.div(num, den), self.alpha(xe));
        }
        Some(l)
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

    fn roundtrip(rs: &ReedSolomon, k: usize, nerr: usize, seed: &mut u32) {
        let msg: Vec<u8> = (0..k).map(|_| rng(seed) as u8).collect();
        let par = rs.encode(&msg);
        let mut block: Vec<u8> = msg.iter().chain(par.iter()).copied().collect();
        let clean = block.clone();
        let mut used = std::collections::HashSet::new();
        while used.len() < nerr {
            let p = rng(seed) as usize % block.len();
            if used.insert(p) {
                block[p] ^= (rng(seed) % 255 + 1) as u8;
            }
        }
        assert_eq!(rs.decode(&mut block), Some(nerr), "{nerr} errors in {}", block.len());
        assert_eq!(block, clean);
    }

    #[test]
    fn ccsds_and_dab_codes_correct_their_errors() {
        let mut seed = 31337;
        let ccsds = ReedSolomon::new(0x187, 112, 11, 32);
        for e in [0, 1, 7, 16] {
            roundtrip(&ccsds, 223, e, &mut seed);
        }
        let dab = ReedSolomon::new(0x11D, 0, 1, 10); // DAB+ (120,110), shortened
        for e in [0, 1, 3, 5] {
            roundtrip(&dab, 110, e, &mut seed);
        }
    }
}
