//! DRM xHE-AAC audio: the audio super frame (one per 400 ms MSC frame) carries a continuous
//! sequence of USAC access units, with a directory of frame borders at its end and a CRC-16
//! after each access unit (ES 201 980 5.3.1.3). The AudioSpecificConfig libxaac needs is
//! rebuilt from the DRM static configuration of SDC type 9.

use super::fac::crc;
use super::sdc::AudioInfo;
use crate::xaac::XaacDecoder;
use std::collections::VecDeque;

struct BitReader<'a> {
    d: &'a [u8],
    pos: usize,
    bad: bool,
}

impl BitReader<'_> {
    fn get(&mut self, n: usize) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            let b = if self.pos < 8 * self.d.len() { (self.d[self.pos / 8] >> (7 - self.pos % 8)) & 1 } else {
                self.bad = true;
                0
            };
            v = (v << 1) | b as u32;
            self.pos += 1;
        }
        v
    }
    fn esc(&mut self, a: usize, b: usize, c: usize) -> u32 {
        let mut v = self.get(a);
        if v == (1 << a) - 1 {
            let add = self.get(b);
            v += add;
            if c > 0 && add == (1 << b) - 1 {
                v += self.get(c);
            }
        }
        v
    }
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    n: usize,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: usize) {
        for i in (0..bits).rev() {
            if self.n % 8 == 0 {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                *self.bytes.last_mut().unwrap() |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
    fn esc(&mut self, v: u32, a: usize, b: usize, c: usize) {
        let ma = (1 << a) - 1;
        if v < ma {
            return self.put(v, a);
        }
        self.put(ma, a);
        let v = v - ma;
        let mb = (1 << b) - 1;
        if v < mb {
            return self.put(v, b);
        }
        self.put(mb, b);
        self.put(v - mb, c);
    }
}

fn rate_index(rate: u32) -> Option<u32> {
    [96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000, 7_350].iter().position(|&r| r == rate).map(|i| i as u32)
}

/// AudioSpecificConfig (USAC) from the DRM static configuration
pub fn asc_from_drm(info: &AudioInfo) -> Option<Vec<u8>> {
    let mut r = BitReader { d: &info.config, pos: 0, bad: false };
    let stereo = info.mode != 0;
    let csfl = r.get(2) + 1;
    let sbr = csfl >= 2;
    let mut w = BitWriter::default();
    // Header: audio object type 42 (escaped), sampling frequency, channel configuration
    w.put(31, 5);
    w.put(42 - 32, 6);
    match rate_index(info.rate) {
        Some(i) => w.put(i, 4),
        None => {
            w.put(15, 4);
            w.put(info.rate, 24);
        }
    }
    w.put(if stereo { 2 } else { 1 }, 4);
    // UsacConfig
    match rate_index(info.rate) {
        Some(i) => w.put(i, 5),
        None => {
            w.put(0x1f, 5);
            w.put(info.rate, 24);
        }
    }
    w.put(csfl, 3);
    w.put(if stereo { 2 } else { 1 }, 5);
    // The single channel element, then the trailing extension elements
    let mut core = BitWriter::default();
    core.put(if stereo { 1 } else { 0 }, 2);
    core.put(0, 1); // tw_mdct
    core.put(r.get(1), 1); // noiseFilling
    if sbr {
        let (harm, inter, pvc) = (r.get(1), r.get(1), r.get(1));
        core.put(harm, 1);
        core.put(inter, 1);
        core.put(pvc, 1);
        core.put(r.get(8), 8); // dflt start / stop frequency
        let (e1, e2) = (r.get(1), r.get(1));
        core.put(e1, 1);
        core.put(e2, 1);
        if e1 == 1 {
            core.put(r.get(5), 5);
        }
        if e2 == 1 {
            core.put(r.get(6), 6);
        }
        if stereo {
            let sc = r.get(2);
            core.put(sc, 2);
            if sc > 0 {
                // Mps212Config, DRM form back to ISO
                let (fr, gain, ts) = (r.get(3), r.get(3), r.get(1));
                let (hr, pc, opp) = (r.get(1), r.get(1), r.get(1));
                core.put(fr, 3);
                core.put(gain, 3);
                core.put(if ts == 1 { 3 } else { 0 }, 2);
                core.put(0, 2); // decorrelation config
                core.put(hr, 1);
                core.put(pc, 1);
                core.put(opp, 1);
                if opp == 1 {
                    core.put(r.get(5), 5);
                }
                if sc > 1 {
                    core.put(r.get(5), 5);
                    core.put(r.get(1), 1);
                }
            }
        }
    }
    let trailing = r.esc(2, 4, 8);
    w.esc(trailing, 4, 8, 16); // numElements - 1
    for (i, &b) in core.bytes.iter().enumerate() {
        let bits = if i + 1 == core.bytes.len() && core.n % 8 != 0 { core.n % 8 } else { 8 };
        w.put((b >> (8 - bits)) as u32, bits);
    }
    for _ in 0..trailing {
        let ty = r.esc(4, 8, 16);
        let len = r.esc(4, 8, 16);
        let has_def = r.get(1);
        w.put(3, 2); // ID_USAC_EXT
        w.esc(ty, 4, 8, 16);
        w.esc(len, 4, 8, 16);
        w.put(has_def, 1);
        if has_def == 1 {
            let d = r.esc(8, 16, 0);
            w.esc(d, 8, 16, 0);
        }
        w.put(r.get(1), 1); // payload fragmentation
        for _ in 0..len {
            w.put(0, 8);
        }
    }
    w.put(0, 1); // no config extension
    (!r.bad).then_some(w.bytes)
}

/// Audio super frames -> access units -> PCM
pub struct XheAudio {
    dec: Option<XaacDecoder>,
    config: Vec<u8>,
    payload: VecDeque<u8>,
    synced: bool,
    pub frames_ok: u64,
    pub frames_bad: u64,
    pub rate: u32,
    pub channels: u8,
}

impl Default for XheAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl XheAudio {
    pub fn new() -> XheAudio {
        XheAudio { dec: None, config: Vec::new(), payload: VecDeque::new(), synced: false, frames_ok: 0, frames_bad: 0, rate: 0, channels: 0 }
    }

    /// One audio super frame (the stream's bytes of one MSC frame); PCM appended to `pcm`
    pub fn push(&mut self, info: &AudioInfo, sf: &[u8], pcm: &mut Vec<f32>) {
        if self.dec.is_none() || self.config != info.config {
            self.config = info.config.clone();
            self.dec = asc_from_drm(info).and_then(|a| XaacDecoder::new(&a));
            self.payload.clear();
            self.synced = false;
        }
        for au in self.split(sf) {
            match au {
                Some(au) => {
                    self.frames_ok += 1;
                    if let Some(d) = &mut self.dec {
                        if let Some((r, c)) = d.decode(&au, pcm) {
                            self.rate = r;
                            self.channels = c;
                        }
                    }
                }
                None => self.frames_bad += 1,
            }
        }
    }

    fn lost(&mut self) {
        self.payload.clear();
        self.synced = false;
    }

    /// Access units completed by this super frame (None = failed its CRC)
    fn split(&mut self, sf: &[u8]) -> Vec<Option<Vec<u8>>> {
        let n = sf.len();
        if n < 2 {
            return Vec::new();
        }
        let count = (sf[0] >> 4) as usize;
        let bits: Vec<u8> = (0..8).map(|k| (sf[0] >> (7 - k)) & 1).collect();
        if crc(&bits, 8) != sf[1] as u32 || n < 2 + 2 * count {
            self.lost();
            return Vec::new();
        }
        let dir = n - 2 * count;
        let start = self.payload.len();
        self.payload.extend(&sf[2..dir]);
        if count == 0 {
            if self.payload.len() > 64 * 1024 {
                self.lost();
            }
            return Vec::new();
        }
        // Directory, last border first: 12-bit index + the count again
        let mut borders = vec![0usize; count];
        for i in (0..count).rev() {
            let p = dir + 2 * (count - 1 - i);
            let v = ((sf[p] as usize) << 8) | sf[p + 1] as usize;
            if v & 15 != count {
                self.lost();
                return Vec::new();
            }
            borders[i] = v >> 4;
        }
        borders[0] = match borders[0] {
            0xFFE if start >= 2 => start - 2,
            0xFFF if start >= 1 => start - 1,
            0xFFE | 0xFFF => {
                self.lost();
                return Vec::new();
            }
            b => b + start,
        };
        for b in borders.iter_mut().skip(1) {
            *b += start;
        }
        if borders.windows(2).any(|w| w[1] < w[0]) || borders.iter().any(|&b| b > self.payload.len()) {
            self.lost();
            return Vec::new();
        }
        // Frame i ends at border i: the first one completes the frame begun earlier
        let mut out = Vec::new();
        let mut prev = 0;
        for (i, &b) in borders.iter().enumerate() {
            let fr: Vec<u8> = self.payload.range(prev..b).copied().collect();
            prev = b;
            if i == 0 && !self.synced {
                continue; // the tail of a frame we never had the start of
            }
            if fr.is_empty() {
                continue;
            }
            let ok = fr.len() > 2 && {
                let bits: Vec<u8> = fr[..fr.len() - 2].iter().flat_map(|&x| (0..8).map(move |k| (x >> (7 - k)) & 1)).collect();
                crc(&bits, 16) == ((fr[fr.len() - 2] as u32) << 8 | fr[fr.len() - 1] as u32)
            };
            out.push(ok.then(|| fr[..fr.len() - 2].to_vec()));
        }
        self.payload.drain(..prev);
        self.synced = true;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cnr1_config_to_asc() {
        // CNR-1 (SDC type 9): xHE-AAC 32 kHz mono, config 70 9a e8
        let info = AudioInfo { coding: 3, mode: 0, rate: 32_000, config: vec![0x70, 0x9a, 0xe8], ..Default::default() };
        let asc = asc_from_drm(&info).unwrap();
        // AOT 31 + 10 = 42, sf index 5, mono
        assert_eq!(asc[0] >> 3, 31);
        let mut r = BitReader { d: &asc, pos: 0, bad: false };
        assert_eq!((r.get(5), r.get(6), r.get(4), r.get(4)), (31, 10, 5, 1));
        assert_eq!((r.get(5), r.get(3), r.get(5)), (5, 2, 1)); // usac sf index, csfl 2, mono
    }
}
