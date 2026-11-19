//! RDS (EN 62106): the 1187.5 bit/s data on the 57 kHz subcarrier of broadcast FM.
//!
//! Composite in (FM demodulator output, 240 kHz) -> 57 kHz brought to 0 Hz, filtered to
//! +-2.4 kHz and decimated to 20 kHz -> Costas loop (the subcarrier is suppressed-carrier
//! BPSK) -> half-bit integration and chip timing (Gardner) -> biphase chips paired into bits
//! -> differential decoding -> 26-bit blocks found by their checkword and offset word ->
//! groups: station name (0A/0B), RadioText (2A/2B), programme type, PI.

use crate::dsp::{lowpass, DecimFir, C32};
use std::collections::VecDeque;
use std::f64::consts::PI;

const BIT_RATE: f64 = 1187.5;
const DECIM: usize = 12;

/// Offset words of blocks A, B, C, C', D (checkword XOR CRC of the information word)
const OFFSETS: [(u16, Block); 5] = [(0x0FC, Block::A), (0x198, Block::B), (0x168, Block::C), (0x350, Block::Cp), (0x1B4, Block::D)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    A,
    B,
    C,
    Cp,
    D,
}

impl Block {
    fn next(self) -> Block {
        match self {
            Block::A => Block::B,
            Block::B => Block::C,
            Block::C | Block::Cp => Block::D,
            Block::D => Block::A,
        }
    }
    fn slot(self) -> usize {
        match self {
            Block::A => 0,
            Block::B => 1,
            Block::C | Block::Cp => 2,
            Block::D => 3,
        }
    }
}

/// 10-bit CRC of a 16-bit information word, g(x) = x^10+x^8+x^7+x^5+x^4+x^3+1
fn crc(info: u16) -> u16 {
    let mut r: u32 = (info as u32) << 10;
    for i in (10..26).rev() {
        if r & (1 << i) != 0 {
            r ^= 0x5B9 << (i - 10);
        }
    }
    r as u16 & 0x3FF
}

fn block_type(word: u32) -> Option<Block> {
    let info = (word >> 10) as u16;
    let off = (word as u16 & 0x3FF) ^ crc(info);
    OFFSETS.iter().find(|(o, _)| *o == off).map(|(_, b)| *b)
}

pub const PTY_NAMES: [&str; 32] = [
    "", "News", "Current Affairs", "Information", "Sport", "Education", "Drama", "Culture", "Science", "Varied", "Pop Music",
    "Rock Music", "Easy Listening", "Light Classical", "Serious Classical", "Other Music", "Weather", "Finance", "Children's",
    "Social Affairs", "Religion", "Phone In", "Travel", "Leisure", "Jazz Music", "Country Music", "National Music", "Oldies Music",
    "Folk Music", "Documentary", "Alarm Test", "Alarm",
];

#[derive(Default, Clone, Debug)]
pub struct RdsInfo {
    pub pi: Option<u16>,
    pub ps: String,
    pub rt: String,
    pub pty: u8,
    pub groups: u64,
    pub blocks_ok: u64,
    pub blocks_bad: u64,
}

impl RdsInfo {
    /// The app's rdsJson format
    pub fn to_json(&self, active: bool) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "{{\"active\":{},\"ps\":\"{}\",\"rt\":\"{}\",\"pi\":\"{}\",\"callsign\":\"\",\"pty\":\"{}\"}}",
            active,
            esc(self.ps.trim()),
            esc(self.rt.trim()),
            self.pi.map_or(String::new(), |p| format!("{p:04X}")),
            PTY_NAMES[self.pty as usize & 31]
        )
    }
}

pub struct RdsDecoder {
    // 57 kHz down-conversion
    nco: f64,
    nco_step: f64,
    lp: DecimFir<C32>,
    // Costas loop (per 20 kHz sample)
    phase: f32,
    freq: f32,
    // Half-bit integrator and chip timing
    half: VecDeque<f32>,
    half_sum: f32,
    mf: Vec<f32>,
    t: f64,
    chip: f64,
    prev_chip: f32,
    // Chip pairing: which phase of the chip stream starts a bit
    chips: VecDeque<f32>,
    pair_score: [f32; 2],
    parity: usize,
    chip_count: usize,
    prev_bit_sign: f32,
    // Blocks
    reg: u32,
    nbits: u32,
    expect: Option<Block>,
    since: u32,
    bad_run: u32,
    group: [Option<u16>; 4],
    // Data being assembled
    ps: [u8; 8],
    rt: [u8; 64],
    rt_ab: Option<bool>,
    pub info: RdsInfo,
}

impl RdsDecoder {
    pub fn new(rate: f64) -> RdsDecoder {
        let out = rate / DECIM as f64;
        let spb = out / BIT_RATE; // samples per bit at 20 kHz: 16.84
        let half = (spb / 2.0).round() as usize;
        RdsDecoder {
            nco: 0.0,
            nco_step: -2.0 * PI * 57_000.0 / rate,
            lp: DecimFir::new(lowpass(2_800.0 / rate, 2_000.0 / rate), DECIM),
            phase: 0.0,
            freq: 0.0,
            half: std::iter::repeat(0.0).take(half).collect(),
            half_sum: 0.0,
            mf: vec![0.0; 4],
            t: 2.0,
            chip: spb / 2.0,
            prev_chip: 0.0,
            chips: VecDeque::new(),
            pair_score: [0.0; 2],
            parity: 0,
            chip_count: 0,
            prev_bit_sign: 1.0,
            reg: 0,
            nbits: 0,
            expect: None,
            since: 0,
            bad_run: 0,
            group: [None; 4],
            ps: [b' '; 8],
            rt: [b' '; 64],
            rt_ab: None,
            info: RdsInfo::default(),
        }
    }

    /// Recently decoded (blocks still arriving and making sense)
    pub fn active(&self) -> bool {
        self.expect.is_some()
    }

    /// Forget the station (after retuning)
    pub fn reset(&mut self) {
        self.expect = None;
        self.ps = [b' '; 8];
        self.rt = [b' '; 64];
        self.rt_ab = None;
        self.info = RdsInfo::default();
    }

    /// Composite samples at the rate given to `new`
    pub fn process(&mut self, composite: &[f32]) {
        let mut bb = Vec::with_capacity(composite.len());
        for &x in composite {
            let (s, c) = self.nco.sin_cos();
            bb.push(C32::new(x * c as f32, x * s as f32));
            self.nco = (self.nco + self.nco_step).rem_euclid(2.0 * PI);
        }
        let mut d = Vec::with_capacity(bb.len() / DECIM + 1);
        self.lp.process(&bb, &mut d);
        for z in d {
            // Costas loop: BPSK, error = I * Q (signs)
            let (s, c) = self.phase.sin_cos();
            let r = z * C32::new(c, -s);
            let e = (r.re.signum() * r.im).clamp(-1.0, 1.0) / (r.re.abs() + r.im.abs() + 1e-9);
            self.freq = (self.freq + 2e-5 * e).clamp(-0.05, 0.05);
            self.phase = (self.phase + self.freq + 0.02 * e).rem_euclid(2.0 * std::f32::consts::PI);
            // Half-bit integration of the real part: one value per chip at the right time
            self.half_sum += r.re - self.half.pop_front().unwrap();
            self.half.push_back(r.re);
            self.mf.push(self.half_sum);
        }
        self.chips_from_mf();
    }

    fn interp(&self, t: f64) -> f32 {
        let i = t.floor() as usize;
        let mu = (t - i as f64) as f32;
        let (a, b) = (self.mf[i], self.mf[i + 1]);
        a + (b - a) * mu
    }

    fn chips_from_mf(&mut self) {
        let chip = 20_000.0 / (2.0 * BIT_RATE);
        while self.t + 2.0 < self.mf.len() as f64 {
            let y = self.interp(self.t);
            let mid = self.interp(self.t - self.chip / 2.0);
            // Gardner timing error on the chip stream
            let e = (y - self.prev_chip) * mid;
            let norm = y.abs() + self.prev_chip.abs() + 1e-9;
            self.chip = chip * (1.0 + (-(e / (norm * norm)) as f64 * 0.002).clamp(-0.01, 0.01));
            self.t += self.chip;
            self.prev_chip = y;
            self.chip_in(y);
        }
        // The next chip may lie a little past the end of what is there: keep enough
        let keep = (self.t as usize).saturating_sub(4).min(self.mf.len().saturating_sub(4));
        if keep > 1024 {
            self.mf.drain(..keep);
            self.t -= keep as f64;
        }
    }

    /// Biphase: the two chips of a bit have opposite signs. Pair chips the way that makes
    /// them opposite most often, then a bit is the sign of (first - second).
    fn chip_in(&mut self, c: f32) {
        self.chips.push_back(c);
        if self.chips.len() > 2 {
            self.chips.pop_front();
        }
        self.chip_count += 1;
        if self.chips.len() == 2 {
            let (a, b) = (self.chips[0], self.chips[1]);
            let p = self.chip_count & 1;
            let score = (a - b).abs() - (a + b).abs();
            self.pair_score[p] = self.pair_score[p] * 0.99 + score * 0.01;
            if self.pair_score[1 - self.parity] > self.pair_score[self.parity] + 0.05 * (a.abs() + b.abs()) {
                self.parity = 1 - self.parity;
            }
            if p == self.parity {
                let sign = (a - b).signum();
                // Differential coding: a change of sign is a 1
                let bit = (sign != self.prev_bit_sign) as u32;
                self.prev_bit_sign = sign;
                self.bit_in(bit);
            }
        }
    }

    fn bit_in(&mut self, bit: u32) {
        self.reg = ((self.reg << 1) | bit) & 0x3FF_FFFF;
        self.nbits += 1;
        match self.expect {
            None => {
                // Searching: a valid block, then its successor exactly 26 bits later
                if self.nbits >= 26 {
                    if let Some(b) = block_type(self.reg) {
                        self.expect = Some(b.next());
                        self.since = 0;
                        self.bad_run = 0;
                        self.store(b, (self.reg >> 10) as u16);
                    }
                }
            }
            Some(want) => {
                self.since += 1;
                if self.since < 26 {
                    return;
                }
                self.since = 0;
                let got = block_type(self.reg);
                let ok = match (got, want) {
                    (Some(Block::Cp), Block::C) => true,
                    (Some(g), w) => g == w,
                    _ => false,
                };
                if ok {
                    self.info.blocks_ok += 1;
                    self.bad_run = 0;
                    self.store(got.unwrap(), (self.reg >> 10) as u16);
                } else {
                    self.info.blocks_bad += 1;
                    self.bad_run += 1;
                    self.group[want.slot()] = None;
                    if want == Block::D {
                        self.group = [None; 4];
                    }
                    if self.bad_run > 12 {
                        self.expect = None; // lost
                        self.nbits = 0;
                        return;
                    }
                }
                self.expect = Some(want.next());
            }
        }
    }

    fn store(&mut self, b: Block, info: u16) {
        self.group[b.slot()] = Some(info);
        if b == Block::A {
            self.info.pi = Some(info);
        }
        if b == Block::D {
            self.group_done();
            self.group = [None; 4];
        }
    }

    fn group_done(&mut self) {
        let Some(b) = self.group[1] else { return };
        self.info.groups += 1;
        let gtype = b >> 12;
        let version_b = b & 0x0800 != 0;
        self.info.pty = ((b >> 5) & 0x1F) as u8;
        let d = self.group[3];
        let c = self.group[2];
        match gtype {
            0 => {
                if let Some(d) = d {
                    let i = (b & 3) as usize * 2;
                    self.ps[i] = (d >> 8) as u8;
                    self.ps[i + 1] = d as u8;
                    self.info.ps = self.ps.iter().map(|&x| crate::dab::fic::ebu_char(x)).collect();
                }
            }
            2 => {
                let ab = b & 0x10 != 0;
                if self.rt_ab != Some(ab) {
                    self.rt = [b' '; 64];
                    self.rt_ab = Some(ab);
                }
                let seg = (b & 0x0F) as usize;
                let chars: Vec<u8> = if version_b {
                    d.map(|d| vec![(d >> 8) as u8, d as u8]).unwrap_or_default()
                } else {
                    match (c, d) {
                        (Some(c), Some(d)) => vec![(c >> 8) as u8, c as u8, (d >> 8) as u8, d as u8],
                        _ => vec![],
                    }
                };
                let base = seg * if version_b { 2 } else { 4 };
                for (k, ch) in chars.into_iter().enumerate() {
                    if base + k < 64 {
                        self.rt[base + k] = ch;
                    }
                }
                // 0x0D ends the text
                let end = self.rt.iter().position(|&x| x == 0x0D).unwrap_or(64);
                self.info.rt = self.rt[..end].iter().map(|&x| crate::dab::fic::ebu_char(x)).collect();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RDS transmitter: groups -> blocks with checkwords -> differential + biphase -> 57 kHz
    /// subcarrier, in a composite with mono audio and the pilot, plus noise
    fn transmit(groups: &[[u16; 4]], rate: f64) -> Vec<f32> {
        let offs = [0x0FC, 0x198, 0x168, 0x1B4];
        let mut bits = Vec::new();
        for g in groups {
            for (k, &info) in g.iter().enumerate() {
                let w = ((info as u32) << 10) | (crc(info) ^ offs[k]) as u32;
                for i in (0..26).rev() {
                    bits.push((w >> i) & 1);
                }
            }
        }
        let mut d = 0u32;
        let spb = rate / BIT_RATE;
        let mut out = Vec::new();
        let mut seed = 1u32;
        for (k, &b) in bits.iter().enumerate() {
            d ^= b;
            let a = if d == 1 { 1.0 } else { -1.0 };
            let start = (k as f64 * spb) as usize;
            let end = ((k + 1) as f64 * spb) as usize;
            for n in start..end {
                let t = n as f64 / rate;
                let chip = if ((n - start) as f64) < spb / 2.0 { a } else { -a };
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (seed as f64 / u32::MAX as f64 - 0.5) * 0.1;
                let audio = 0.4 * (2.0 * PI * 440.0 * t).sin();
                let pilot = 0.09 * (2.0 * PI * 19_000.0 * t).sin();
                let rds = 0.04 * chip * (2.0 * PI * 57_000.0 * t + 0.7).cos();
                out.push((audio + pilot + rds + noise) as f32);
            }
        }
        out
    }

    #[test]
    fn decodes_name_and_text() {
        let pi = 0xF201;
        let ps = b"FRANCE I";
        let rt = b"Le 7/9 de France Inter\r";
        let mut groups = Vec::new();
        for _ in 0..6 {
            for seg in 0..4u16 {
                // 0A: PTY 1 (News), segment address
                let b = (1 << 5) | seg;
                let dd = ((ps[2 * seg as usize] as u16) << 8) | ps[2 * seg as usize + 1] as u16;
                groups.push([pi, b, 0, dd]);
            }
            for seg in 0..6u16 {
                // 2A: 4 characters per group
                let ch = |i: usize| rt.get(i).copied().unwrap_or(b' ') as u16;
                let i = seg as usize * 4;
                groups.push([pi, (2 << 12) | (1 << 5) | seg, (ch(i) << 8) | ch(i + 1), (ch(i + 2) << 8) | ch(i + 3)]);
            }
        }
        let x = transmit(&groups, 240_000.0);
        let mut d = RdsDecoder::new(240_000.0);
        for ch in x.chunks(4096) {
            d.process(ch);
        }
        assert_eq!(d.info.pi, Some(pi));
        assert_eq!(d.info.ps, "FRANCE I");
        assert_eq!(d.info.rt, "Le 7/9 de France Inter");
        assert_eq!(PTY_NAMES[d.info.pty as usize], "News");
        assert!(d.to_json_ok());
    }

    impl RdsDecoder {
        fn to_json_ok(&self) -> bool {
            self.info.to_json(self.active()).contains("\"ps\":\"FRANCE I\"")
        }
    }

    #[test]
    fn offset_words_identify_blocks() {
        let info = 0xF201u16;
        for (off, b) in OFFSETS {
            let word = ((info as u32) << 10) | ((crc(info) ^ off) as u32);
            assert_eq!(block_type(word), Some(b));
            assert_eq!(block_type(word ^ 0x40), None, "a bit error must not pass");
        }
    }
}
