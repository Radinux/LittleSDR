//! VOR (VHF omnidirectional range, 108 - 117.95 MHz): which radial of the beacon we are on.
//!
//! The carrier is amplitude modulated by
//! - a 30 Hz "variable" tone whose phase is the bearing from the beacon (rotating pattern),
//! - a 9960 Hz subcarrier frequency modulated (+-480 Hz) by the 30 Hz "reference",
//! - the beacon's Morse ident on 1020 Hz (about 7 words a minute, every 10 s), sometimes voice.
//!
//! The radial is the reference's phase minus the variable's, both measured at 30 Hz over the
//! same second (one DFT bin each, on a common time base: no filter in the variable path, the
//! reference path's delays compensated). Doppler VORs swap which tone is AM and which FM, with
//! the rotation reversed: the same difference gives their radial. Radials are magnetic.

use crate::cw::MorseDecoder;
use crate::dsp::{lowpass, DecimFir, C32};
use std::collections::VecDeque;
use std::f64::consts::PI;

/// Input rate: the AM envelope at 48 kHz
pub const RATE: f64 = 48_000.0;
const SUB: f64 = 9_960.0;
const IDENT: f64 = 1_020.0;
/// Reference path: the subcarrier brought to 0 Hz, filtered, kept at 3 kHz
const DECIM: usize = 16;
/// One measurement a second (30 cycles of 30 Hz)
const BLOCK: usize = 48_000;
/// Ident tone envelope frames: 4 ms (250 Hz)
const ID_FRAME: usize = 192;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VorStatus {
    /// Magnetic radial (deg, 0 - 360) from the beacon, when the signal is a VOR's
    pub radial: Option<f64>,
    /// 30 Hz variable modulation depth (nominally 0.3) and reference deviation (nominally 480 Hz)
    pub depth: f64,
    pub deviation: f64,
    /// Agreement of the last seconds' radials (1 = steady)
    pub steadiness: f64,
    /// The ident, once heard twice; the Morse heard lately
    pub ident: String,
    pub morse: String,
}

pub struct VorDecoder {
    t: u64,
    carrier: f64,
    var_acc: (f64, f64),
    /// The variable's 30 Hz bin of each second already over, waiting for the reference's
    var_done: VecDeque<(f64, f64)>,
    sub_fir: DecimFir<C32>,
    prev: C32,
    ref_acc: (f64, f64),
    ref_n: usize,
    /// Reference path delay (s): the filter's group delay and the discriminator's half sample
    delay: f64,
    /// Last radials as unit vectors, for the average and its steadiness
    recent: Vec<(f64, f64)>,
    id_mix: f64,
    id_acc: C32,
    id_n: usize,
    morse: MorseDecoder,
    heard: String,
    words: Vec<String>,
    pub status: VorStatus,
}

impl Default for VorDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VorDecoder {
    pub fn new() -> VorDecoder {
        // The 9960 Hz subcarrier +-480 Hz, nothing of the 30 Hz AM or the 1020 Hz ident
        let taps = lowpass(900.0 / RATE, 700.0 / RATE);
        let delay = (taps.len() - 1) as f64 / 2.0 / RATE + 0.5 * DECIM as f64 / RATE;
        VorDecoder {
            t: 0,
            carrier: 0.0,
            var_acc: (0.0, 0.0),
            var_done: VecDeque::new(),
            sub_fir: DecimFir::new(taps, DECIM),
            prev: C32::new(0.0, 0.0),
            ref_acc: (0.0, 0.0),
            ref_n: 0,
            delay,
            recent: Vec::new(),
            id_mix: 0.0,
            id_acc: C32::new(0.0, 0.0),
            id_n: 0,
            morse: MorseDecoder::new(RATE / ID_FRAME as f64),
            heard: String::new(),
            words: Vec::new(),
            status: VorStatus::default(),
        }
    }

    /// AM envelope samples (|IQ|) at 48 kHz
    pub fn process(&mut self, env: &[f32]) {
        let mut mixed = Vec::with_capacity(env.len());
        let w30 = 2.0 * PI * 30.0 / RATE;
        for &e in env {
            let e = e as f64;
            if self.carrier <= 0.0 {
                self.carrier = e.max(1e-9);
            }
            // Carrier level over ~0.5 s: the modulation as a fraction of it
            self.carrier += (e - self.carrier) * 4e-5;
            let x = e / self.carrier.max(1e-12) - 1.0;
            // Variable: the 30 Hz bin of the modulation itself
            let ph = w30 * (self.t % BLOCK as u64) as f64;
            self.var_acc.0 += x * ph.cos();
            self.var_acc.1 -= x * ph.sin();
            // Reference: the subcarrier to 0 Hz
            let ps = 2.0 * PI * SUB * self.t as f64 / RATE;
            mixed.push(C32::new((x * ps.cos()) as f32, (-x * ps.sin()) as f32));
            // Ident: the 1020 Hz tone's amplitude over 4 ms frames
            self.id_mix += 2.0 * PI * IDENT / RATE;
            if self.id_mix > 2.0 * PI {
                self.id_mix -= 2.0 * PI;
            }
            self.id_acc += C32::new((x * self.id_mix.cos()) as f32, (-x * self.id_mix.sin()) as f32);
            self.id_n += 1;
            if self.id_n == ID_FRAME {
                self.morse.process(self.id_acc.norm_sqr().sqrt() / ID_FRAME as f32);
                self.id_acc = C32::new(0.0, 0.0);
                self.id_n = 0;
            }
            self.t += 1;
            if self.t.is_multiple_of(BLOCK as u64) {
                self.var_done.push_back(std::mem::take(&mut self.var_acc));
            }
        }
        // Reference: FM discriminator on the subcarrier (3 kHz), its 30 Hz bin
        let mut sub = Vec::with_capacity(mixed.len() / DECIM + 1);
        self.sub_fir.process(&mixed, &mut sub);
        let w = 2.0 * PI * 30.0 * DECIM as f64 / RATE;
        let per = BLOCK / DECIM;
        for z in sub {
            let f = (z * self.prev.conj()).arg() as f64 * (RATE / DECIM as f64) / (2.0 * PI);
            self.prev = z;
            let ph = w * self.ref_n as f64;
            self.ref_acc.0 += f * ph.cos();
            self.ref_acc.1 -= f * ph.sin();
            self.ref_n += 1;
            // The same second over in both paths: one measurement
            if self.ref_n == per {
                self.ref_n = 0;
                let r = std::mem::take(&mut self.ref_acc);
                if let Some(v) = self.var_done.pop_front() {
                    self.measure(v, r);
                }
            }
        }
        for c in self.morse.take().chars() {
            self.heard.push(c);
        }
        self.idents();
    }

    fn measure(&mut self, (vr, vi): (f64, f64), (rr, ri): (f64, f64)) {
        let n = BLOCK as f64;
        let depth = 2.0 * (vr * vr + vi * vi).sqrt() / n;
        let deviation = 2.0 * (rr * rr + ri * ri).sqrt() / (n / DECIM as f64);
        // The reference arrives `delay` late: its phase put back by 30 Hz x delay
        let ref_ph = ri.atan2(rr) + 2.0 * PI * 30.0 * self.delay;
        let var_ph = vi.atan2(vr);
        let radial = (ref_ph - var_ph).to_degrees().rem_euclid(360.0);
        self.status.depth = depth;
        self.status.deviation = deviation;
        // A VOR: some 30 Hz AM and a deviation near the nominal 480 Hz
        if depth > 0.05 && (150.0..900.0).contains(&deviation) {
            let r = radial.to_radians();
            self.recent.push((r.cos(), r.sin()));
            if self.recent.len() > 5 {
                self.recent.remove(0);
            }
            let (c, s) = self.recent.iter().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
            let k = self.recent.len() as f64;
            self.status.steadiness = (c * c + s * s).sqrt() / k;
            self.status.radial = Some(s.atan2(c).to_degrees().rem_euclid(360.0));
        } else {
            self.recent.clear();
            self.status.steadiness = 0.0;
            self.status.radial = None;
        }
    }

    /// Idents are 2 or 3 letters (sometimes a digit), repeated: the one heard twice
    fn idents(&mut self) {
        // Words end at spaces
        while let Some(i) = self.heard.find(' ') {
            let w: String = self.heard[..i].chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            self.heard.drain(..=i);
            if (2..=4).contains(&w.len()) {
                self.words.push(w);
                if self.words.len() > 6 {
                    self.words.remove(0);
                }
            }
        }
        if self.heard.len() > 12 {
            self.heard.clear();
        }
        self.status.morse = self.words.join(" ");
        if let Some(w) = self.words.iter().rev().find(|w| self.words.iter().filter(|x| x == w).count() >= 2) {
            self.status.ident = w.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VOR's AM envelope: carrier 1, 30 Hz variable (30 %) at the radial's phase, 9960 Hz
    /// subcarrier (30 %) FM +-480 Hz by the 30 Hz reference, ident `id` on 1020 Hz (10 %)
    fn vor_signal(radial: f64, secs: f64, id: &str, noise: f64) -> Vec<f32> {
        let n = (secs * RATE) as usize;
        // Morse keying at 8 wpm (dot 150 ms), the ident repeated with gaps
        let dot = 0.15;
        let unit = (dot * RATE) as usize;
        let mut key: Vec<bool> = Vec::new();
        while key.len() < n {
            for ch in id.chars() {
                for s in crate::cw::encode(ch).unwrap().chars() {
                    key.extend(std::iter::repeat_n(true, unit * if s == '.' { 1 } else { 3 }));
                    key.extend(std::iter::repeat_n(false, unit));
                }
                key.extend(std::iter::repeat_n(false, 2 * unit));
            }
            key.extend(std::iter::repeat_n(false, 10 * unit));
        }
        let mut sub_ph = 0.0f64;
        let mut seed = 12345u64;
        (0..n)
            .map(|i| {
                let t = i as f64 / RATE;
                let var = 0.3 * (2.0 * PI * 30.0 * t - radial.to_radians()).cos();
                let fref = 480.0 * (2.0 * PI * 30.0 * t).cos();
                sub_ph += 2.0 * PI * (SUB + fref) / RATE;
                let sub = 0.3 * sub_ph.cos();
                let ident = if key[i] { 0.1 * (2.0 * PI * IDENT * t).sin() } else { 0.0 };
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let r = ((seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0 * noise;
                (1.0 + var + sub + ident + r) as f32
            })
            .collect()
    }

    #[test]
    fn radial_and_ident() {
        for radial in [0.0, 47.0, 183.0, 271.5, 359.0] {
            let mut d = VorDecoder::new();
            for chunk in vor_signal(radial, 25.0, "PGS", 0.2).chunks(4800) {
                d.process(chunk);
            }
            let got = d.status.radial.expect("a VOR");
            let err = ((got - radial + 540.0) % 360.0 - 180.0).abs();
            assert!(err < 1.0, "radial {radial}: got {got:.2}");
            assert!((d.status.deviation - 480.0).abs() < 30.0, "deviation {}", d.status.deviation);
            assert!((d.status.depth - 0.3).abs() < 0.03, "depth {}", d.status.depth);
            assert_eq!(d.status.ident, "PGS", "morse heard: {}", d.status.morse);
        }
    }

    #[test]
    fn noise_is_not_a_vor() {
        let mut d = VorDecoder::new();
        let mut seed = 7u64;
        let noise: Vec<f32> = (0..(5.0 * RATE) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                1.0 + ((seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5) as f32
            })
            .collect();
        d.process(&noise);
        assert_eq!(d.status.radial, None);
    }
}
