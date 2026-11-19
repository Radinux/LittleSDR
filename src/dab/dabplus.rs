//! DAB+ audio (ETSI TS 102 563): superframes of 5 CIFs (120 ms), Reed-Solomon (120,110)
//! interleaved across the superframe, a fire code to find its start, then 2 to 6 HE-AAC
//! access units, each with a CRC, decoded by FAAD2.

use super::fec::{crc16_ok, firecode};
use crate::aac::AacDecoder;
use crate::rs::ReedSolomon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub rate_48k: bool,
    pub sbr: bool,
    pub stereo: bool,
    pub ps: bool,
}

impl AudioFormat {
    fn from_byte(b: u8) -> AudioFormat {
        AudioFormat { rate_48k: b & 0x40 != 0, sbr: b & 0x20 != 0, stereo: b & 0x10 != 0, ps: b & 0x08 != 0 }
    }

    pub fn describe(&self) -> String {
        format!(
            "{} {} kHz {}",
            if self.sbr { if self.ps { "HE-AAC v2" } else { "HE-AAC" } } else { "AAC-LC" },
            if self.rate_48k { 48 } else { 32 },
            if self.stereo || self.ps { "stereo" } else { "mono" }
        )
    }

    /// AudioSpecificConfig with the 960-sample transform DAB+ uses
    fn asc(&self) -> Vec<u8> {
        let core_sr: u8 = match (self.rate_48k, self.sbr) {
            (true, true) => 6,   // 24 kHz core
            (true, false) => 3,  // 48 kHz
            (false, true) => 8,  // 16 kHz core
            (false, false) => 5, // 32 kHz
        };
        let ext_sr: u8 = if self.rate_48k { 3 } else { 5 };
        let ch: u8 = if self.stereo { 2 } else { 1 };
        let mut a = vec![(2 << 3) | (core_sr >> 1), ((core_sr & 1) << 7) | (ch << 3) | 0b100];
        if self.sbr {
            // Explicit backward-compatible SBR (and PS) signalling
            a.extend_from_slice(&[0x56, 0xE5, 0x80 | (ext_sr << 3)]);
            if self.ps {
                let l = a.len() - 1;
                a[l] |= 0x05;
                a.extend_from_slice(&[0x48, 0x80]);
            }
        }
        a
    }
}

#[derive(Default, Clone, Debug)]
pub struct DabPlusStats {
    pub superframes: u64,
    pub rs_corrected: u64,
    pub rs_failed: u64,
    pub au_ok: u64,
    pub au_crc: u64,
    pub aac_errors: u64,
}

pub struct DabPlus {
    rs: ReedSolomon,
    frames: Vec<Vec<u8>>,
    pub format: Option<AudioFormat>,
    aac: Option<AacDecoder>,
    pub stats: DabPlusStats,
}

impl Default for DabPlus {
    fn default() -> Self {
        Self::new()
    }
}

impl DabPlus {
    pub fn new() -> DabPlus {
        DabPlus { rs: ReedSolomon::new(0x11D, 0, 1, 10), frames: Vec::new(), format: None, aac: None, stats: DabPlusStats::default() }
    }

    /// One CIF's bytes of the sub-channel; decoded audio (interleaved float) is appended
    /// to `pcm`. Returns (sample rate, channels) when audio was produced.
    pub fn push(&mut self, cif: Vec<u8>, pcm: &mut Vec<f32>) -> Option<(u32, u8)> {
        self.frames.push(cif);
        if self.frames.len() > 5 {
            self.frames.remove(0);
        }
        if self.frames.len() < 5 {
            return None;
        }
        let flen = self.frames[0].len();
        let sf_len = 5 * flen;
        if sf_len % 120 != 0 {
            return None;
        }
        let mut sf: Vec<u8> = self.frames.concat();
        // RS: column i of the superframe (bytes i, i + cols, ...) is one 120-byte codeword
        let cols = sf_len / 120;
        let mut word = vec![0u8; 120];
        let mut corrected = 0;
        let mut failed = false;
        for i in 0..cols {
            for p in 0..120 {
                word[p] = sf[p * cols + i];
            }
            match self.rs.decode(&mut word) {
                Some(n) => corrected += n,
                None => failed = true,
            }
            for p in 0..110 {
                sf[p * cols + i] = word[p];
            }
        }
        // Superframe start? (fire code over bytes 2..11)
        if (sf[3] == 0 && sf[4] == 0) || u16::from_be_bytes([sf[0], sf[1]]) != firecode(&sf[2..11]) {
            return None; // not aligned yet: slide by one CIF
        }
        self.frames.clear();
        self.stats.superframes += 1;
        self.stats.rs_corrected += corrected as u64;
        if failed {
            self.stats.rs_failed += 1;
        }
        let fmt = AudioFormat::from_byte(sf[2]);
        if self.format != Some(fmt) || self.aac.is_none() {
            self.format = Some(fmt);
            self.aac = AacDecoder::new(&fmt.asc());
        }
        let num_aus = match (fmt.rate_48k, fmt.sbr) {
            (true, true) => 3,
            (true, false) => 6,
            (false, true) => 2,
            (false, false) => 4,
        };
        let mut starts = vec![0usize; num_aus + 1];
        starts[0] = match (fmt.rate_48k, fmt.sbr) {
            (true, true) => 6,
            (true, false) => 11,
            (false, true) => 5,
            (false, false) => 8,
        };
        starts[num_aus] = sf_len / 120 * 110;
        let nib = |hi: usize| -> usize {
            // 12-bit values packed from byte 3
            let b = 3 + hi * 3 / 2;
            if hi % 2 == 0 {
                (sf[b] as usize) << 4 | (sf[b + 1] as usize) >> 4
            } else {
                (sf[b] as usize & 0x0F) << 8 | sf[b + 1] as usize
            }
        };
        for (i, s) in starts.iter_mut().enumerate().take(num_aus).skip(1) {
            *s = nib(i - 1);
        }
        let mut result = None;
        for i in 0..num_aus {
            let (a, b) = (starts[i], starts[i + 1]);
            if a + 2 >= b || b > sf.len() {
                self.stats.au_crc += 1;
                continue;
            }
            let au = &sf[a..b];
            if !crc16_ok(au) {
                self.stats.au_crc += 1;
                continue;
            }
            self.stats.au_ok += 1;
            if let Some(dec) = &mut self.aac {
                match dec.decode(&au[..au.len() - 2], pcm) {
                    Some(r) => result = Some(r),
                    None => self.stats.aac_errors += 1,
                }
            }
        }
        result
    }

    pub fn aac_error(&self) -> Option<&str> {
        self.aac.as_ref().map(|a| a.last_error.as_str()).filter(|s| !s.is_empty())
    }
}
