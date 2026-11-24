//! DAB+ audio (ETSI TS 102 563): superframes of 5 CIFs (120 ms), Reed-Solomon (120,110)
//! interleaved across the superframe, a fire code to find its start, then 2 to 6 HE-AAC
//! access units, each with a CRC, decoded by FAAD2.

use super::fec::{crc16_ok, firecode};
use super::pad::PadDecoder;
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
    /// Audio frames made up to cover losses (failed RS, bad CRC, lost superframes)
    pub concealed: u64,
}

/// Loss concealment: a lost audio frame is replaced by the last good one replayed with a
/// fade (an isolated loss is barely heard), runs of losses fade to silence, and the next
/// good frame fades in (no click)
#[derive(Default)]
struct Concealer {
    /// Last good frame's samples (interleaved)
    last: Vec<f32>,
    channels: usize,
    rate: u32,
    /// Losses in a row
    run: u32,
}

impl Concealer {
    /// Gain at the start of the n-th loss in a row (1, 0.5, 0.25...), 0 from the 4th
    fn gain(n: u32) -> f32 {
        if n >= 4 { 0.0 } else { 0.5f32.powi(n as i32) }
    }

    /// One made-up frame appended to `pcm`; None until a good frame has been heard
    fn lost(&mut self, pcm: &mut Vec<f32>) -> Option<(u32, u8)> {
        if self.last.is_empty() || self.channels == 0 {
            return None;
        }
        let (g0, g1) = (Self::gain(self.run), Self::gain(self.run + 1));
        self.run += 1;
        let frames = self.last.len() / self.channels;
        for (k, s) in self.last.iter().enumerate() {
            let t = (k / self.channels) as f32 / frames.max(1) as f32;
            pcm.push(s * (g0 + (g1 - g0) * t));
        }
        Some((self.rate, self.channels as u8))
    }

    /// A good frame was decoded into pcm[start..]: fade it in after losses, keep it
    fn good(&mut self, pcm: &mut [f32], start: usize, rate: u32, channels: u8) {
        let ch = channels.max(1) as usize;
        let frame = &mut pcm[start..];
        if self.run > 0 {
            let g0 = Self::gain(self.run);
            let n = (frame.len() / ch).min(rate as usize / 100).max(1); // ~10 ms
            for i in 0..n {
                let g = g0 + (1.0 - g0) * i as f32 / n as f32;
                for c in 0..ch {
                    if let Some(s) = frame.get_mut(i * ch + c) {
                        *s *= g;
                    }
                }
            }
        }
        self.run = 0;
        self.last.clear();
        self.last.extend_from_slice(frame);
        self.channels = ch;
        self.rate = rate;
    }
}

pub struct DabPlus {
    rs: ReedSolomon,
    frames: Vec<Vec<u8>>,
    pub format: Option<AudioFormat>,
    aac: Option<AacDecoder>,
    pub stats: DabPlusStats,
    /// Text and slideshow carried with the audio
    pub pad: PadDecoder,
    conceal: Concealer,
    /// CIFs since the last superframe, and audio frames per superframe: a superframe lost
    /// outright (damaged header) is noticed after 5 CIFs and concealed in its place
    cifs_lost: usize,
    aus_per_sf: usize,
}

impl Default for DabPlus {
    fn default() -> Self {
        Self::new()
    }
}

impl DabPlus {
    pub fn new() -> DabPlus {
        DabPlus {
            rs: ReedSolomon::new(0x11D, 0, 1, 10),
            frames: Vec::new(),
            format: None,
            aac: None,
            stats: DabPlusStats::default(),
            pad: PadDecoder::new(),
            conceal: Concealer::default(),
            cifs_lost: 0,
            aus_per_sf: 0,
        }
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
            // Not aligned yet: slide by one CIF. Once playing, 5 CIFs without a superframe
            // = one lost: its place is filled
            if self.stats.superframes > 0 {
                self.cifs_lost += 1;
                if self.cifs_lost >= 5 {
                    self.cifs_lost -= 5;
                    let mut r = None;
                    for _ in 0..self.aus_per_sf.max(1) {
                        if let Some(x) = self.conceal.lost(pcm) {
                            self.stats.concealed += 1;
                            r = Some(x);
                        }
                    }
                    return r;
                }
            }
            return None;
        }
        self.frames.clear();
        self.cifs_lost = 0;
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
        self.aus_per_sf = num_aus;
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
            let bad = a + 2 >= b || b > sf.len() || !crc16_ok(&sf[a..b]);
            if bad {
                self.stats.au_crc += 1;
                if let Some(r) = self.conceal.lost(pcm) {
                    self.stats.concealed += 1;
                    result = Some(r);
                }
                continue;
            }
            let au = &sf[a..b];
            self.stats.au_ok += 1;
            self.pad.process_au(&au[..au.len() - 2]);
            if let Some(dec) = &mut self.aac {
                let start = pcm.len();
                match dec.decode(&au[..au.len() - 2], pcm) {
                    Some((rate, ch)) => {
                        self.conceal.good(pcm, start, rate, ch);
                        result = Some((rate, ch));
                    }
                    None => {
                        self.stats.aac_errors += 1;
                        if let Some(r) = self.conceal.lost(pcm) {
                            self.stats.concealed += 1;
                            result = Some(r);
                        }
                    }
                }
            }
        }
        result
    }

    pub fn aac_error(&self) -> Option<&str> {
        self.aac.as_ref().map(|a| a.last_error.as_str()).filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod conceal_tests {
    use super::*;

    /// A lost frame repeats the last one fading 1 -> 0.5, the next good one fades in
    #[test]
    fn losses_fade_and_recover_without_a_click() {
        let mut c = Concealer::default();
        let mut pcm = vec![1.0f32; 200]; // 100 stereo frames of a good AU
        c.good(&mut pcm, 0, 1_000, 2);
        let mut out = Vec::new();
        assert!(c.lost(&mut out).is_some());
        assert_eq!(out.len(), 200);
        assert!((out[0] - 1.0).abs() < 1e-6 && (out[199] - 0.5).abs() < 0.02, "{} {}", out[0], out[199]);
        for _ in 0..4 {
            out.clear();
            c.lost(&mut out);
        }
        assert!(out.iter().all(|&s| s == 0.0), "long run fades to silence");
        let mut next = vec![1.0f32; 200];
        c.good(&mut next, 0, 1_000, 2);
        assert_eq!(next[0], 0.0, "fades in from silence");
        assert_eq!(next[199], 1.0);
    }
}
