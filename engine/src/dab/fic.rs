//! Fast Information Channel: 3 symbols per frame -> 4 blocks -> 12 FIBs of 30 bytes + CRC,
//! carrying the FIGs that describe the ensemble: its name, the services, their labels, and
//! the sub-channels (position in the CIF, size, protection) where their audio is.

use super::fec::{crc16_ok, depuncture, pack, prbs, Viterbi};
use std::collections::{BTreeMap, BTreeSet};

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
    /// FIG 0/21: frequencies (Hz) of ensembles, this one included (other transmitters)
    pub ensemble_freqs: BTreeMap<u16, BTreeSet<u32>>,
    /// FIG 0/21: FM frequencies (Hz) by RDS PI code
    pub fm_freqs: BTreeMap<u16, BTreeSet<u32>>,
    /// FIG 0/24: other ensembles (EId) carrying a service (SId)
    pub other_ensembles: BTreeMap<u32, BTreeSet<u16>>,
}

impl Ensemble {
    /// Where else a service can be heard on DAB (Hz): other transmitters of this ensemble
    /// and the ensembles that also carry it, as the FIC announces them
    pub fn alternatives(&self, sid: u32) -> Vec<u32> {
        let mut eids: BTreeSet<u16> = self.other_ensembles.get(&sid).cloned().unwrap_or_default();
        eids.insert(self.eid);
        let mut v: BTreeSet<u32> = BTreeSet::new();
        for e in eids {
            if let Some(f) = self.ensemble_freqs.get(&e) {
                v.extend(f);
            }
        }
        v.into_iter().collect()
    }

    /// FM frequencies of the same programme (Hz): RDS PI = SId for most broadcasters
    pub fn fm_alternatives(&self, sid: u32) -> Vec<u32> {
        u16::try_from(sid).ok().and_then(|pi| self.fm_freqs.get(&pi)).map(|f| f.iter().copied().collect()).unwrap_or_default()
    }
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

/// EBU Latin character set (ETSI TS 101 756 table 1) to Unicode, all 256 codes
#[rustfmt::skip]
const EBU_LATIN: [u16; 256] = [
    0x0000, 0x0118, 0x012E, 0x0172, 0x0102, 0x0116, 0x010E, 0x0218,
    0x021A, 0x010A, 0x000A, 0x000B, 0x0120, 0x0139, 0x017B, 0x0143,
    0x0105, 0x0119, 0x012F, 0x0173, 0x0103, 0x0117, 0x010F, 0x0219,
    0x021B, 0x010B, 0x0147, 0x011A, 0x0121, 0x013A, 0x017C, 0x0082,
    0x0020, 0x0021, 0x0022, 0x0023, 0x0142, 0x0025, 0x0026, 0x0027,
    0x0028, 0x0029, 0x002A, 0x002B, 0x002C, 0x002D, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037,
    0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F,
    0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047,
    0x0048, 0x0049, 0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F,
    0x0050, 0x0051, 0x0052, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057,
    0x0058, 0x0059, 0x005A, 0x005B, 0x016E, 0x005D, 0x0141, 0x005F,
    0x0104, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067,
    0x0068, 0x0069, 0x006A, 0x006B, 0x006C, 0x006D, 0x006E, 0x006F,
    0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077,
    0x0078, 0x0079, 0x007A, 0x00AB, 0x016F, 0x00BB, 0x013D, 0x0126,
    0x00E1, 0x00E0, 0x00E9, 0x00E8, 0x00ED, 0x00EC, 0x00F3, 0x00F2,
    0x00FA, 0x00F9, 0x00D1, 0x00C7, 0x015E, 0x00DF, 0x00A1, 0x0178,
    0x00E2, 0x00E4, 0x00EA, 0x00EB, 0x00EE, 0x00EF, 0x00F4, 0x00F6,
    0x00FB, 0x00FC, 0x00F1, 0x00E7, 0x015F, 0x011F, 0x0131, 0x00FF,
    0x0136, 0x0145, 0x00A9, 0x0122, 0x011E, 0x011B, 0x0148, 0x0151,
    0x0150, 0x20AC, 0x00A3, 0x0024, 0x0100, 0x0112, 0x012A, 0x016A,
    0x0137, 0x0146, 0x013B, 0x0123, 0x013C, 0x0130, 0x0144, 0x0171,
    0x0170, 0x00BF, 0x013E, 0x00B0, 0x0101, 0x0113, 0x012B, 0x016B,
    0x00C1, 0x00C0, 0x00C9, 0x00C8, 0x00CD, 0x00CC, 0x00D3, 0x00D2,
    0x00DA, 0x00D9, 0x0158, 0x010C, 0x0160, 0x017D, 0x00D0, 0x013F,
    0x00C2, 0x00C4, 0x00CA, 0x00CB, 0x00CE, 0x00CF, 0x00D4, 0x00D6,
    0x00DB, 0x00DC, 0x0159, 0x010D, 0x0161, 0x017E, 0x0111, 0x0140,
    0x00C3, 0x00C5, 0x00C6, 0x0152, 0x0177, 0x00DD, 0x00D5, 0x00D8,
    0x00DE, 0x014A, 0x0154, 0x0106, 0x015A, 0x0179, 0x0164, 0x00F0,
    0x00E3, 0x00E5, 0x00E6, 0x0153, 0x0175, 0x00FD, 0x00F5, 0x00F8,
    0x00FE, 0x014B, 0x0155, 0x0107, 0x015B, 0x017A, 0x0165, 0x0127,
];

pub fn ebu_char(b: u8) -> char {
    char::from_u32(EBU_LATIN[b as usize] as u32).unwrap_or('?')
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
            21 => {
                // Frequency information: per ensemble (EId) or FM station (PI), its frequencies
                let mut i = 0;
                while i + 2 <= d.len() {
                    let len = (d[i + 1] & 0x1F) as usize;
                    let Some(list) = d.get(i + 2..i + 2 + len) else { break };
                    i += 2 + len;
                    let mut j = 0;
                    while j + 3 <= list.len() {
                        let id = u16::from_be_bytes([list[j], list[j + 1]]);
                        let (rm, n) = (list[j + 2] >> 4, (list[j + 2] & 7) as usize);
                        let Some(f) = list.get(j + 3..j + 3 + n) else { break };
                        j += 3 + n;
                        match rm {
                            // DAB ensemble: 5-bit control + 19-bit frequency in 16 kHz units
                            0 => {
                                for e in f.chunks_exact(3) {
                                    let hz = ((((e[0] & 7) as u32) << 16) | ((e[1] as u32) << 8) | e[2] as u32) * 16_000;
                                    if hz > 0 {
                                        self.ensemble.ensemble_freqs.entry(id).or_default().insert(hz);
                                    }
                                }
                            }
                            // FM with RDS: 87.5 MHz + n x 100 kHz
                            8 => {
                                for &c in f {
                                    if (1..=204).contains(&c) {
                                        self.ensemble.fm_freqs.entry(id).or_default().insert(87_500_000 + c as u32 * 100_000);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            24 => {
                // Other-ensemble services: SId -> the EIds that also carry it
                let sid_len = if pd { 4 } else { 2 };
                let mut i = 0;
                while i + sid_len + 1 <= d.len() {
                    let sid = if pd {
                        u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
                    } else {
                        u16::from_be_bytes([d[i], d[i + 1]]) as u32
                    };
                    i += sid_len;
                    let n = (d[i] & 0x0F) as usize;
                    i += 1;
                    for _ in 0..n {
                        if i + 2 > d.len() {
                            return;
                        }
                        let eid = u16::from_be_bytes([d[i], d[i + 1]]);
                        i += 2;
                        self.ensemble.other_ensembles.entry(sid).or_default().insert(eid);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// FIG 0/21 (ensemble 0x1001 on 11D, FM PI 0xD312 on 99.5 MHz) and FIG 0/24 (SId 0xD312
    /// also on ensemble 0x1002, itself on 5C): the service's alternatives
    #[test]
    fn frequency_information_and_other_ensembles() {
        let mut f = Fic::new();
        f.ensemble.eid = 0x1001;
        let khz16 = |hz: u32| hz / 16_000;
        let e = |hz: u32| [(khz16(hz) >> 16) as u8 & 7, (khz16(hz) >> 8) as u8, khz16(hz) as u8];
        // FIG 0/21 body: Rfa+len, then [Id, R&M|C|n, freqs...]
        let mut l = vec![];
        l.extend_from_slice(&[0x10, 0x01, 0x03]);
        l.extend_from_slice(&e(222_064_000));
        l.extend_from_slice(&[0x10, 0x02, 0x03]);
        l.extend_from_slice(&e(178_352_000));
        l.extend_from_slice(&[0xD3, 0x12, 0x81, 120]); // 87.5 + 12.0 = 99.5 MHz
        let mut b = vec![21u8, 0, l.len() as u8];
        b.extend(l);
        f.fig0(&b);
        f.fig0(&[24, 0xD3, 0x12, 0x01, 0x10, 0x02]);
        assert_eq!(f.ensemble.alternatives(0xD312), vec![178_352_000, 222_064_000]);
        assert_eq!(f.ensemble.fm_alternatives(0xD312), vec![99_500_000]);
    }
}
