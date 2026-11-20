//! Main Service Channel: one sub-channel out of the CIFs (4 per frame, 18 symbols each),
//! time de-interleaving over 16 CIFs, EEP de-puncturing, Viterbi, energy dispersal.

use super::fec::{depuncture, pack, prbs, Viterbi};
use super::fic::{Protection, SubChannel};

pub const CIF_BITS: usize = 18 * super::ofdm::BITS_PER_SYMBOL; // 55296 = 864 CU of 64 bits
const MAP: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

pub struct SubChannelDecoder {
    pub sub: SubChannel,
    bitrate: u32,
    /// (number of 128-bit blocks, PI) for the de-puncturing
    blocks: [(usize, usize); 2],
    history: Vec<Vec<i8>>, // the last 16 CIFs of this sub-channel
    index: usize,
    filled: usize,
    viterbi: Viterbi,
    prbs: Vec<u8>,
}

impl SubChannelDecoder {
    pub fn new(sub: SubChannel) -> Option<SubChannelDecoder> {
        let bitrate = sub.bitrate()?;
        let Protection::Eep { b, level } = sub.protection else { return None };
        let blocks = if !b {
            let n = (bitrate / 8) as usize;
            match level {
                1 => [(6 * n - 3, 24), (3, 23)],
                2 if n == 1 => [(5, 13), (1, 12)],
                2 => [(2 * n - 3, 14), (4 * n + 3, 13)],
                3 => [(6 * n - 3, 8), (3, 7)],
                _ => [(4 * n - 3, 3), (2 * n + 3, 2)],
            }
        } else {
            let n = (bitrate / 32) as usize;
            let (p1, p2) = match level {
                1 => (10, 9),
                2 => (6, 5),
                3 => (4, 3),
                _ => (2, 1),
            };
            [(24 * n - 3, p1), (3, p2)]
        };
        let len = sub.size as usize * 64;
        Some(SubChannelDecoder {
            sub,
            bitrate,
            blocks,
            history: vec![vec![0; len]; 16],
            index: 0,
            filled: 0,
            viterbi: Viterbi::new(),
            prbs: prbs(24 * bitrate as usize),
        })
    }

    pub fn bitrate(&self) -> u32 {
        self.bitrate
    }

    /// One CIF (55296 soft bits); returns the sub-channel's bytes for it (3 x bitrate)
    /// once the de-interleaver is full
    pub fn process(&mut self, cif: &[i8]) -> Option<Vec<u8>> {
        let len = self.sub.size as usize * 64;
        let start = self.sub.start as usize * 64;
        let data = &cif[start..start + len];
        // Bit i of the output CIF comes from the CIF received MAP[i % 16] CIFs later
        let mut out = vec![0i8; len];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.history[(self.index + MAP[i & 15]) & 15][i];
        }
        self.history[self.index].copy_from_slice(data);
        self.index = (self.index + 1) & 15;
        if self.filled < 16 {
            self.filled += 1;
            return None;
        }
        let (mother, _) = depuncture(&out, &self.blocks);
        let mut bits = self.viterbi.decode(&mother);
        for (b, p) in bits.iter_mut().zip(&self.prbs) {
            *b ^= p;
        }
        Some(pack(&bits))
    }
}
