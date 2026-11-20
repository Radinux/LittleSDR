//! Service Description Channel: in the first symbols of each superframe. Carries the
//! multiplex description (stream lengths, protection), service labels and the audio
//! coding (AAC / xHE-AAC, sample rate, stereo, text).

use super::cellmap::{CellMap, SDC};
use super::fac::{bits_to_u32, crc, depuncture, interleaver, prbs, punct_ones, punct_period, soft4, TAIL};
use super::ofdm::Frame;
use crate::dab::fec::Viterbi;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioInfo {
    pub short_id: u8,
    pub stream: u8,
    /// 0 AAC, 3 xHE-AAC
    pub coding: u8,
    pub sbr: bool,
    /// 0 mono, 1 parametric stereo, 2 stereo
    pub mode: u8,
    /// Hz
    pub rate: u32,
    pub text: bool,
    /// xHE-AAC configuration bytes
    pub config: Vec<u8>,
}

impl AudioInfo {
    pub fn describe(&self) -> String {
        let codec = match (self.coding, self.sbr) {
            (0, false) => "AAC",
            (0, true) => "HE-AAC",
            (3, _) => "xHE-AAC",
            _ => "?",
        };
        let mode = ["mono", "parametric stereo", "stereo", "?"][self.mode as usize & 3];
        format!("{codec} {} Hz {mode}{}", self.rate, if self.text { " + text" } else { "" })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SdcInfo {
    pub afs: u8,
    /// Protection levels of parts A and B
    pub prot: (u8, u8),
    /// Per stream: (length of part A, of part B) in bytes per frame
    pub streams: Vec<(u16, u16)>,
    pub labels: Vec<(u8, String)>,
    pub audio: Vec<AudioInfo>,
}

pub struct SdcDecoder {
    viterbi: Viterbi,
    pub ok: u64,
    pub bad: u64,
}

impl Default for SdcDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SdcDecoder {
    pub fn new() -> SdcDecoder {
        SdcDecoder { viterbi: Viterbi::new(), ok: 0, bad: 0 }
    }

    /// The SDC of a superframe's first frame. `qam4`: SDC mode from the FAC (16-QAM not yet).
    pub fn decode(&mut self, map: &CellMap, f: &Frame, qam4: bool) -> Option<SdcInfo> {
        if !qam4 {
            return None;
        }
        let mut vals = Vec::new();
        for s in 0..map.sym_per_frame {
            for c in 0..map.carriers() {
                if map.kind[s][c] == SDC {
                    vals.push((f.cells[s][c], f.csi[s][c]));
                }
            }
        }
        let n = vals.len();
        if n < 20 {
            return None;
        }
        let mean_csi = vals.iter().map(|v| v.1).sum::<f32>() / n as f32;
        let scale = 60.0 / std::f32::consts::FRAC_1_SQRT_2 / mean_csi.max(1e-20);
        let mut soft = Vec::with_capacity(2 * n);
        for (v, c) in &vals {
            soft.push(soft4(v.re, *c, scale));
            soft.push(soft4(v.im, *c, scale));
        }
        let mut d = vec![0i8; 2 * n];
        for (i, &t) in interleaver(2 * n, 21).iter().enumerate() {
            d[t] = soft[i];
        }
        // Code rate 1/2 (pattern 4): M = RX * floor((2N - 12) / RY)
        let rate = 4;
        let (rx, ry) = (punct_period(rate), punct_ones(rate));
        let m = rx * ((2 * n - 12) / ry);
        let tail = (2 * n - 12) - ry * ((2 * n - 12) / ry);
        let mother = depuncture(&d, rate, m, Some(&TAIL[tail]));
        let mut bits = self.viterbi.decode(&mother);
        for (b, p) in bits.iter_mut().zip(prbs(m)) {
            *b ^= p;
        }
        // Data field: whole bytes after the 4-bit AFS index, then the CRC
        let data_bytes = (m - 20) / 8;
        let useful = 20 + 8 * data_bytes;
        let mut crc_in = vec![0u8; 4];
        crc_in.extend_from_slice(&bits[..useful - 16]);
        if crc(&crc_in, 16) != bits_to_u32(&bits[useful - 16..useful]) {
            self.bad += 1;
            return None;
        }
        self.ok += 1;
        Some(parse(&bits[..useful - 16]))
    }
}

fn parse(b: &[u8]) -> SdcInfo {
    let mut info = SdcInfo { afs: bits_to_u32(&b[..4]) as u8, ..Default::default() };
    let mut i = 4;
    let get = |i: &mut usize, n: usize| -> Option<u32> {
        let v = b.get(*i..*i + n).map(bits_to_u32);
        *i += n;
        v
    };
    while let Some(len) = get(&mut i, 7) {
        if len == 0 {
            break;
        }
        let Some(_version) = get(&mut i, 1) else { break };
        let Some(ty) = get(&mut i, 4) else { break };
        let body_end = i + len as usize * 8 + 4;
        if body_end > b.len() {
            break;
        }
        match ty {
            0 => {
                let pa = get(&mut i, 2).unwrap_or(0) as u8;
                let pb = get(&mut i, 2).unwrap_or(0) as u8;
                info.prot = (pa, pb);
                info.streams.clear();
                for _ in 0..len / 3 {
                    let a = get(&mut i, 12).unwrap_or(0) as u16;
                    let bl = get(&mut i, 12).unwrap_or(0) as u16;
                    info.streams.push((a, bl));
                }
            }
            1 => {
                let sid = get(&mut i, 2).unwrap_or(0) as u8;
                let _rfu = get(&mut i, 2);
                let bytes: Vec<u8> = (0..len).filter_map(|_| get(&mut i, 8).map(|v| v as u8)).collect();
                info.labels.push((sid, String::from_utf8_lossy(&bytes).trim_end_matches('\0').trim().to_string()));
            }
            9 if len >= 2 => {
                let short_id = get(&mut i, 2).unwrap_or(0) as u8;
                let stream = get(&mut i, 2).unwrap_or(0) as u8;
                let coding = get(&mut i, 2).unwrap_or(0) as u8;
                let sbr = get(&mut i, 1) == Some(1);
                let mode = get(&mut i, 2).unwrap_or(0) as u8;
                let asr = get(&mut i, 3).unwrap_or(0);
                let rate = if coding == 3 {
                    [9_600, 12_000, 16_000, 19_200, 24_000, 32_000, 38_400, 48_000][asr as usize]
                } else {
                    match asr {
                        1 => 12_000,
                        3 => 24_000,
                        5 => 48_000,
                        _ => 0,
                    }
                };
                let text = get(&mut i, 1) == Some(1);
                let _enh = get(&mut i, 1);
                let _coder = get(&mut i, 5);
                let _rfa = get(&mut i, 1);
                let config = if coding == 3 { (0..len - 2).filter_map(|_| get(&mut i, 8).map(|v| v as u8)).collect() } else { Vec::new() };
                info.audio.push(AudioInfo { short_id, stream, coding, sbr, mode, rate, text, config });
            }
            _ => {}
        }
        i = body_end;
    }
    info
}
