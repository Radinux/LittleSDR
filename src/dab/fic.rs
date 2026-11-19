//! Fast Information Channel: 3 symbols per frame -> 4 blocks -> 12 FIBs of 30 bytes + CRC,
//! carrying the FIGs that describe the ensemble: its name, the services, their labels, and
//! the sub-channels (position in the CIF, size, protection) where their audio is.

use super::fec::{crc16_ok, depuncture, pack, prbs, Viterbi};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protection {
    /// Equal error protection, profile A or B, level 1..4
    Eep { b: bool, level: u8 },
    /// Unequal (DAB MP2 audio): table index
    Uep { index: u8 },
}

#[derive(Clone, Debug)]
pub struct SubChannel {
    pub id: u8,
    pub start: u16, // CU
    pub size: u16,  // CU
    pub protection: Protection,
}

impl SubChannel {
    /// Bit rate (kbit/s), for EEP
    pub fn bitrate(&self) -> Option<u32> {
        match self.protection {
            Protection::Eep { b: false, level } => {
                let k = [12, 8, 6, 4][(level - 1) as usize];
                Some(8 * self.size as u32 / k)
            }
            Protection::Eep { b: true, level } => {
                let k = [27, 21, 18, 15][(level - 1) as usize];
                Some(32 * self.size as u32 / k)
            }
            Protection::Uep { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Service {
    pub sid: u32,
    pub label: String,
    /// Primary audio component: sub-channel and audio type (63 = DAB+, 0 = MP2)
    pub subch: Option<u8>,
    pub ascty: u8,
}

#[derive(Default, Debug, Clone)]
pub struct Ensemble {
    pub eid: u16,
    pub label: String,
    pub services: BTreeMap<u32, Service>,
    pub subchannels: BTreeMap<u8, SubChannel>,
}

#[derive(Default, Debug, Clone)]
pub struct FicStats {
    pub fibs: u64,
    pub crc_errors: u64,
}

pub struct Fic {
    viterbi: Viterbi,
    prbs: Vec<u8>,
    pub ensemble: Ensemble,
    pub stats: FicStats,
}

impl Default for Fic {
    fn default() -> Self {
        Self::new()
    }
}

/// EBU Latin (EN 300 401 annex C) to text: ASCII range as is, the accented letters that matter
fn ebu_char(b: u8) -> char {
    match b {
        0x20..=0x7E => b as char,
        0x82 => 'é', 0x83 => 'è', 0x84 => 'ê', 0x85 => 'ë', 0x86 => 'à', 0x87 => 'â', 0x88 => 'ç',
        0x89 => 'î', 0x8A => 'ï', 0x8B => 'ô', 0x8C => 'û', 0x8D => 'ü', 0x8E => 'ö', 0x8F => 'ä',
        0x91 => 'ñ', 0x99 => 'Ä', 0x9A => 'Ö', 0x9B => 'Ü', 0x9C => 'ß', 0xA0 => 'á', 0xA1 => 'í',
        0xA2 => 'ó', 0xA3 => 'ú', 0xA8 => 'É', 0xA9 => 'È', 0xAA => 'Ê', 0xAC => 'À',
        _ => '?',
    }
}

fn label(bytes: &[u8], charset: u8) -> String {
    let s: String = if charset == 15 {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        bytes.iter().map(|&b| ebu_char(b)).collect()
    };
    s.trim_end().to_string()
}

impl Fic {
    pub fn new() -> Fic {
        Fic { viterbi: Viterbi::new(), prbs: prbs(768), ensemble: Ensemble::default(), stats: FicStats::default() }
    }

    /// The 9216 soft bits of a frame's FIC
    pub fn process(&mut self, soft: &[i8]) {
        for block in soft.chunks_exact(2304) {
            let (mother, _) = depuncture(block, &[(21, 16), (3, 15)]);
            let mut bits = self.viterbi.decode(&mother);
            for (b, p) in bits.iter_mut().zip(&self.prbs) {
                *b ^= p;
            }
            let bytes = pack(&bits);
            for fib in bytes.chunks_exact(32) {
                self.stats.fibs += 1;
                if crc16_ok(fib) {
                    self.fib(&fib[..30]);
                } else {
                    self.stats.crc_errors += 1;
                }
            }
        }
    }

    fn fib(&mut self, d: &[u8]) {
        let mut i = 0;
        while i < d.len() {
            let h = d[i];
            if h == 0xFF {
                break; // end marker / padding
            }
            let (ty, len) = (h >> 5, (h & 0x1F) as usize);
            let body = d.get(i + 1..i + 1 + len);
            i += 1 + len;
            let Some(body) = body else { break };
            match ty {
                0 => self.fig0(body),
                1 => self.fig1(body),
                _ => {}
            }
        }
    }

    fn fig0(&mut self, b: &[u8]) {
        if b.is_empty() {
            return;
        }
        let (cn, pd, ext) = (b[0] & 0x80 != 0, b[0] & 0x20 != 0, b[0] & 0x1F);
        let _ = cn;
        let d = &b[1..];
        match ext {
            0 if d.len() >= 2 => self.ensemble.eid = u16::from_be_bytes([d[0], d[1]]),
            1 => {
                // Sub-channel organisation
                let mut i = 0;
                while i + 3 <= d.len() {
                    let id = d[i] >> 2;
                    let start = ((d[i] as u16 & 3) << 8) | d[i + 1] as u16;
                    if d[i + 2] & 0x80 == 0 {
                        // Short form: UEP table
                        let index = d[i + 2] & 0x3F;
                        let size = UEP_SIZES.get(index as usize).copied().unwrap_or(0);
                        self.ensemble.subchannels.insert(id, SubChannel { id, start, size, protection: Protection::Uep { index } });
                        i += 3;
                    } else {
                        if i + 4 > d.len() {
                            break;
                        }
                        let option = (d[i + 2] >> 4) & 7;
                        let level = ((d[i + 2] >> 2) & 3) + 1;
                        let size = ((d[i + 2] as u16 & 3) << 8) | d[i + 3] as u16;
                        if option <= 1 {
                            self.ensemble
                                .subchannels
                                .insert(id, SubChannel { id, start, size, protection: Protection::Eep { b: option == 1, level } });
                        }
                        i += 4;
                    }
                }
            }
            2 => {
                // Services and their components
                let mut i = 0;
                let sid_len = if pd { 4 } else { 2 };
                while i + sid_len + 1 <= d.len() {
                    let sid = if pd {
                        u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
                    } else {
                        u16::from_be_bytes([d[i], d[i + 1]]) as u32
                    };
                    i += sid_len;
                    let ncomp = (d[i] & 0x0F) as usize;
                    i += 1;
                    for _ in 0..ncomp {
                        if i + 2 > d.len() {
                            return;
                        }
                        let (c0, c1) = (d[i], d[i + 1]);
                        i += 2;
                        let tmid = c0 >> 6;
                        let primary = c1 & 0x02 != 0;
                        if tmid == 0 && primary && !pd {
                            let s = self.ensemble.services.entry(sid).or_insert_with(|| Service { sid, ..Default::default() });
                            s.ascty = c0 & 0x3F;
                            s.subch = Some(c1 >> 2);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn fig1(&mut self, b: &[u8]) {
        if b.is_empty() {
            return;
        }
        let (charset, ext) = (b[0] >> 4, b[0] & 7);
        let d = &b[1..];
        match ext {
            0 if d.len() >= 18 => self.ensemble.label = label(&d[2..18], charset),
            1 if d.len() >= 18 => {
                let sid = u16::from_be_bytes([d[0], d[1]]) as u32;
                let s = self.ensemble.services.entry(sid).or_insert_with(|| Service { sid, ..Default::default() });
                s.label = label(&d[2..18], charset);
            }
            _ => {}
        }
    }
}

/// Sub-channel sizes (CU) of the 64 UEP table entries (clause 6.2.1, table 8)
const UEP_SIZES: [u16; 64] = [
    16, 21, 24, 29, 35, 24, 29, 35, 42, 52, 29, 35, 42, 52, 32, 42, 48, 58, 70, 40, 52, 58, 70, 84, 48, 58, 70, 84, 104, 58, 70, 84, 104, 64,
    84, 96, 116, 140, 80, 104, 116, 140, 168, 96, 116, 140, 168, 208, 116, 140, 168, 208, 232, 128, 168, 192, 232, 280, 160, 208, 280, 192,
    280, 416,
];
