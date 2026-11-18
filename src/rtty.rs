//! RTTY decoder: 2-tone FSK, asynchronous start/stop framing, 5-bit Baudot (ITA2) text.
//!
//! Works on the demodulated audio (USB / LSB): the two tones are brought down around 0 Hz,
//! low-passed and decimated, an FM discriminator gives the instantaneous frequency, a filter
//! one half-bit long smooths it and a slicer decides mark / space. The slicer threshold
//! follows the mark and space levels it sees, so a receiver tuned a little off still decodes.
//!
//! Defaults are amateur RTTY: 45.45 Bd, 170 Hz shift, tones around 2210 Hz in the audio.
//! The mark is the higher RF frequency: higher audio tone in USB, lower in LSB.

use crate::dsp::{lowpass, DecimFir, Nco, C32};
use std::collections::VecDeque;
use std::f64::consts::PI;

const AUDIO_RATE: f64 = 48_000.0;
const DECIM: usize = 24;
const RATE: f64 = AUDIO_RATE / DECIM as f64; // 2 kHz

/// US TTY variant of ITA2, as used by amateur RTTY software
const LTRS: [char; 32] = [
    '\0', 'E', '\n', 'A', ' ', 'S', 'I', 'U', '\r', 'D', 'R', 'J', 'N', 'F', 'C', 'K', 'T', 'Z', 'L', 'W', 'H', 'Y', 'P', 'Q', 'O',
    'B', 'G', '\0', 'M', 'X', 'V', '\0',
];
const FIGS: [char; 32] = [
    '\0', '3', '\n', '-', ' ', '\u{7}', '8', '7', '\r', '$', '4', '\'', ',', '!', ':', '(', '5', '"', ')', '2', '#', '6', '0', '1', '9',
    '?', '&', '\0', '.', '/', ';', '\0',
];
/// International ITA2 figures (weather and press services): `=`, `+`, `'`... where the US
/// variant has `;`, `"`, bell
const FIGS_ITA2: [char; 32] = [
    '\0', '3', '\n', '-', ' ', '\'', '8', '7', '\r', '\0', '4', '\u{7}', ',', '!', ':', '(', '5', '+', ')', '2', '£', '6', '0', '1',
    '9', '?', '&', '\0', '.', '/', '=', '\0',
];
const CODE_FIGS: u8 = 27;
const CODE_LTRS: u8 = 31;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Uart {
    /// Waiting for a start bit (mark -> space)
    Idle,
    /// Inside a character: time since the start edge (samples), bits so far
    Char { t: f64, code: u8, bit: u8 },
}

pub struct Rtty {
    pub center: f64,
    pub shift: f64,
    pub baud: f64,
    /// Mark above the center frequency in the audio (USB); false in LSB
    pub mark_high: bool,
    /// Manual polarity inversion, on top of the sideband
    pub reverse: bool,
    /// Unshift on space: a space after figures returns to letters (common practice)
    pub usos: bool,
    /// International ITA2 figures instead of the US amateur variant
    pub ita2: bool,
    nco: Nco,
    lp: DecimFir<C32>,
    prev: C32,
    smooth: VecDeque<f32>,
    smooth_sum: f32,
    mark_lvl: f32,
    space_lvl: f32,
    last: bool,
    uart: Uart,
    figs: bool,
    /// Characters decoded since the last `take_text`
    text: String,
    /// Squelch: share of recent characters with a valid stop bit. On a real signal it stays
    /// near 1, on noise around 0.5. Characters wait in `pending` until it is convincing.
    quality: f32,
    /// Share of the 300-3000 Hz power that falls on the two tones (~1 on RTTY, ~0.1 on noise)
    in_band: f32,
    band_pow: f32,
    wide_pow: f32,
    wide_nco: Nco,
    wide_lp: DecimFir<C32>,
    /// Squelch enabled (`sql` parameter)
    pub squelch: bool,
    open: bool,
    pending: VecDeque<char>,
    pub chars: u64,
    pub errors: u64,
}

impl Rtty {
    pub fn new() -> Rtty {
        let mut r = Rtty {
            center: 2_210.0,
            shift: 170.0,
            baud: 45.45,
            mark_high: true,
            reverse: false,
            usos: true,
            ita2: false,
            nco: Nco::new(),
            lp: DecimFir::new(vec![1.0], DECIM),
            prev: C32::new(1.0, 0.0),
            smooth: VecDeque::new(),
            smooth_sum: 0.0,
            mark_lvl: 85.0,
            space_lvl: -85.0,
            last: true,
            uart: Uart::Idle,
            figs: false,
            text: String::new(),
            quality: 0.0,
            in_band: 0.0,
            band_pow: 0.0,
            wide_pow: 1e-12,
            wide_nco: Nco::new(),
            wide_lp: DecimFir::new(lowpass(1_400.0 / AUDIO_RATE, 300.0 / AUDIO_RATE), DECIM),
            squelch: true,
            open: false,
            pending: VecDeque::new(),
            chars: 0,
            errors: 0,
        };
        r.configure();
        r
    }

    /// Applies `shift` and `baud` (filters, expected levels)
    pub fn configure(&mut self) {
        let half = self.shift / 2.0;
        let cut = half + self.baud;
        self.lp.set_taps(lowpass(cut / AUDIO_RATE, (cut * 0.8).max(150.0) / AUDIO_RATE));
        let n = ((RATE / self.baud) * 0.5).round().max(1.0) as usize;
        self.smooth = std::iter::repeat(0.0).take(n).collect();
        self.smooth_sum = 0.0;
        self.mark_lvl = half as f32;
        self.space_lvl = -half as f32;
    }

    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "shift" => self.shift = v.clamp(50.0, 1000.0),
            "baud" => self.baud = v.clamp(20.0, 300.0),
            "center" => {
                self.center = v.clamp(300.0, 3000.0);
                return true;
            }
            "rev" | "reverse" => {
                self.reverse = v != 0.0;
                let mh = self.mark_high;
                self.set_polarity(mh);
                return true;
            }
            "ita2" => {
                self.ita2 = v != 0.0;
                return true;
            }
            "sql" | "squelch" => {
                self.squelch = v != 0.0;
                return true;
            }
            _ => return false,
        }
        self.configure();
        true
    }

    pub fn describe(&self) -> String {
        format!(
            "RTTY {:.2} Bd, shift {:.0} Hz, center {:.0} Hz{}{}, {} chars, {} framing errors, squelch {} (tones {:.0} % of the band)",
            self.baud,
            self.shift,
            self.center,
            if self.reverse { ", reversed" } else { "" },
            if self.ita2 { ", ITA2" } else { "" },
            self.chars,
            self.errors,
            if !self.squelch { "off" } else if self.open { "open" } else { "closed" },
            self.in_band * 100.0
        )
    }

    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    pub fn process(&mut self, audio: &[f32]) {
        let mut z: Vec<C32> = audio.iter().map(|&x| C32::new(x, 0.0)).collect();
        self.nco.set(self.center, AUDIO_RATE);
        self.nco.mix(&mut z);
        let mut d = Vec::with_capacity(z.len() / DECIM + 1);
        self.lp.process(&z, &mut d);
        // Reference: the whole SSB audio band, 300-3000 Hz
        let mut w: Vec<C32> = audio.iter().map(|&x| C32::new(x, 0.0)).collect();
        self.wide_nco.set(1_650.0, AUDIO_RATE);
        self.wide_nco.mix(&mut w);
        let mut wd = Vec::with_capacity(w.len() / DECIM + 1);
        self.wide_lp.process(&w, &mut wd);
        let a = (1.0 / (RATE * 0.5)) as f32; // ~0.5 s
        for (s, r) in d.iter().zip(&wd) {
            self.band_pow += a * (s.norm_sqr() - self.band_pow);
            self.wide_pow += a * (r.norm_sqr() - self.wide_pow);
        }
        self.in_band = (self.band_pow / self.wide_pow.max(1e-12)).min(1.0);
        let k = (RATE / (2.0 * PI)) as f32;
        for s in d {
            let f = (s * self.prev.conj()).arg() * k; // Hz from the center
            self.prev = s;
            self.smooth_sum += f - self.smooth.pop_front().unwrap();
            self.smooth.push_back(f);
            let f = self.smooth_sum / self.smooth.len() as f32;
            self.step(f);
        }
    }

    fn is_mark(&self, f: f32) -> bool {
        let thr = (self.mark_lvl + self.space_lvl) / 2.0;
        let high = f > thr;
        let mark_above = self.mark_lvl > self.space_lvl;
        high == mark_above
    }

    fn step(&mut self, f: f32) {
        let mark = self.is_mark(f);
        let bit_len = RATE / self.baud;
        self.uart = match self.uart {
            Uart::Idle => {
                if self.last && !mark {
                    Uart::Char { t: 0.0, code: 0, bit: 0 }
                } else {
                    Uart::Idle
                }
            }
            Uart::Char { t, code, bit } => {
                let t = t + 1.0;
                // Sample points: middle of the start bit, of the 5 data bits, of the stop bit
                let at = bit_len * (bit as f64 + 0.5);
                if t < at {
                    Uart::Char { t, code, bit }
                } else {
                    self.learn(f, mark);
                    match bit {
                        0 if mark => Uart::Idle, // glitch, not a start bit
                        0 => Uart::Char { t, code, bit: 1 },
                        1..=5 => Uart::Char { t, code: code | ((mark as u8) << (bit - 1)), bit: bit + 1 },
                        _ => {
                            self.quality += 0.15 * ((mark as u8 as f32) - self.quality);
                            if !self.squelch {
                                self.open = true;
                            } else if self.open && (self.quality < 0.6 || self.in_band < 0.3) {
                                self.open = false;
                                self.pending.clear();
                            } else if !self.open && self.quality > 0.8 && self.in_band > 0.45 {
                                self.open = true;
                                self.text.extend(self.pending.drain(..));
                            }
                            if mark {
                                self.emit(code);
                            } else {
                                self.errors += 1;
                            }
                            Uart::Idle
                        }
                    }
                }
            }
        };
        self.last = mark;
    }

    /// Follows the mark and space levels (receiver mistuning) from the sampled bits
    fn learn(&mut self, f: f32, mark: bool) {
        let a = 0.05;
        if mark {
            self.mark_lvl += a * (f - self.mark_lvl);
        } else {
            self.space_lvl += a * (f - self.space_lvl);
        }
        // Keep them apart by at least a third of the shift, on the expected sides
        let half = (self.shift / 2.0) as f32;
        let sign = if self.mark_high != self.reverse { 1.0 } else { -1.0 };
        let mid = (self.mark_lvl + self.space_lvl) / 2.0;
        if (self.mark_lvl - self.space_lvl) * sign < half * 0.66 {
            self.mark_lvl = mid + sign * half * 0.33;
            self.space_lvl = mid - sign * half * 0.33;
        }
    }

    /// Resets the expected levels for the current sideband / polarity
    pub fn set_polarity(&mut self, mark_high: bool) {
        self.mark_high = mark_high;
        let sign = if self.mark_high != self.reverse { 1.0 } else { -1.0 };
        let half = (self.shift / 2.0) as f32;
        self.mark_lvl = sign * half;
        self.space_lvl = -sign * half;
    }

    fn emit(&mut self, code: u8) {
        self.chars += 1;
        match code {
            CODE_FIGS => self.figs = true,
            CODE_LTRS => self.figs = false,
            _ => {
                let figs = if self.ita2 { &FIGS_ITA2 } else { &FIGS };
                let c = if self.figs { figs[code as usize] } else { LTRS[code as usize] };
                if c == ' ' && self.usos {
                    self.figs = false;
                }
                match c {
                    '\0' | '\u{7}' | '\r' => {}
                    c if self.open => self.text.push(c),
                    c => {
                        // Held until the squelch opens: the start of a message isn't lost
                        self.pending.push_back(c);
                        if self.pending.len() > 16 {
                            self.pending.pop_front();
                        }
                    }
                }
            }
        }
    }
}

impl Default for Rtty {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baudot encoding of `text`, with the shifts it needs
    fn encode(text: &str) -> Vec<u8> {
        encode_with(text, &FIGS)
    }

    fn encode_with(text: &str, figs_table: &[char; 32]) -> Vec<u8> {
        let mut out = vec![CODE_LTRS];
        let mut figs = false;
        for ch in text.chars() {
            let c = ch.to_ascii_uppercase();
            if let Some(i) = LTRS.iter().position(|&x| x == c).filter(|&i| i != 0) {
                if figs && c != ' ' && c != '\n' {
                    out.push(CODE_LTRS);
                    figs = false;
                }
                out.push(i as u8);
                if c == ' ' {
                    figs = false; // USOS on the receiving side
                }
            } else if let Some(i) = figs_table.iter().position(|&x| x == c).filter(|&i| i != 0) {
                if !figs {
                    out.push(CODE_FIGS);
                    figs = true;
                }
                out.push(i as u8);
            }
        }
        out
    }

    /// AFSK audio: idle mark, then each code as start + 5 bits + 1.5 stop
    fn modulate(codes: &[u8], mark_hz: f64, space_hz: f64, baud: f64) -> Vec<f32> {
        let sps = AUDIO_RATE / baud;
        let mut bits: Vec<(bool, f64)> = vec![(true, 20.0)]; // idle
        for &c in codes {
            bits.push((false, 1.0));
            for b in 0..5 {
                bits.push(((c >> b) & 1 == 1, 1.0));
            }
            bits.push((true, 1.5));
        }
        bits.push((true, 10.0));
        let (mut out, mut phase, mut t) = (Vec::new(), 0.0f64, 0.0f64);
        for (mark, len) in bits {
            let end = t + len * sps;
            while (out.len() as f64) < end {
                let f = if mark { mark_hz } else { space_hz };
                phase += 2.0 * PI * f / AUDIO_RATE; // continuous phase
                out.push((0.3 * phase.sin()) as f32);
            }
            t = end;
        }
        out
    }

    fn add_noise(x: &mut [f32], amp: f32) {
        let mut s = 0x9e37_79b9u32;
        for v in x.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v += amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
    }

    fn decode(audio: &[f32], setup: impl FnOnce(&mut Rtty)) -> String {
        let mut r = Rtty::new();
        setup(&mut r);
        for ch in audio.chunks(1000) {
            r.process(ch);
        }
        r.take_text()
    }

    const MSG: &str = "CQ CQ DE YO3ABC YO3ABC PSE K\nRST 599 73 GL 1234-5678.";

    #[test]
    fn stays_quiet_on_noise() {
        let mut a = vec![0.0f32; 48_000 * 20];
        add_noise(&mut a, 0.5);
        let got = decode(&a, |_| {});
        assert!(got.len() <= 3, "garbage on noise: {got:?}");
    }

    #[test]
    fn decodes_usb_rtty() {
        let a = modulate(&encode(MSG), 2_295.0, 2_125.0, 45.45);
        assert_eq!(decode(&a, |_| {}), MSG);
    }

    #[test]
    fn decodes_lsb_rtty_and_survives_mistuning_and_noise() {
        // LSB: mark is the lower audio tone; receiver 40 Hz off; noise ~ -6 dB SNR in 3 kHz
        let mut a = modulate(&encode(MSG), 2_125.0 + 40.0, 2_295.0 + 40.0, 45.45);
        add_noise(&mut a, 0.6);
        let got = decode(&a, |r| r.set_polarity(false));
        assert_eq!(got, MSG);
    }

    /// Through the whole engine: an RTTY signal as raw IQ, received in LSB then in USB
    #[test]
    fn decodes_through_the_engine() {
        use crate::engine::{Engine, EngineCtl, Mode};
        use crate::source::IqBlock;
        let (rate, dial, center) = (250e3, 14_085e3, 14_035e3);
        // Mark is the higher RF frequency, 170 Hz above space
        let (mark_rf, space_rf) = (dial - 2_125.0, dial - 2_295.0);
        let codes = encode(MSG);
        let sps = rate / 45.45;
        let mut iq = Vec::new();
        let mut phase = 0.0f64;
        let mut push = |mark: bool, bits: f64, iq: &mut Vec<C32>| {
            let n = (bits * sps) as usize;
            for _ in 0..n {
                let f = if mark { mark_rf } else { space_rf } - center;
                phase += 2.0 * PI * f / rate;
                iq.push(C32::new(0.05 * phase.cos() as f32, 0.05 * phase.sin() as f32));
            }
        };
        push(true, 30.0, &mut iq);
        for &c in &codes {
            push(false, 1.0, &mut iq);
            for b in 0..5 {
                push((c >> b) & 1 == 1, 1.0, &mut iq);
            }
            push(true, 1.5, &mut iq);
        }
        push(true, 30.0, &mut iq);

        let run = |mode: Mode, freq: f64| {
            let mut e = Engine::new(mode, freq);
            e.control(&EngineCtl::Rtty(true));
            let mut out = Vec::new();
            let mut text = String::new();
            for ch in iq.chunks(8192) {
                e.process(IqBlock { samples: ch.to_vec(), rate, center, settle_ms: 0.0 }, &mut out);
                text += &e.take_text();
            }
            text
        };
        assert_eq!(run(Mode::Lsb, dial), MSG);
        // Same signal in USB: dial 4.42 kHz lower puts the tones at 2125 / 2295 Hz again,
        // mark now the higher audio tone
        assert_eq!(run(Mode::Usb, dial - 2_125.0 - 2_295.0), MSG);
    }

    #[test]
    fn decodes_weather_rtty_50_baud_450_shift() {
        let msg = "ZCZC 123 METAR EDDF 191020Z 24012KT 9999 FEW030 08/03 Q1012=";
        let a = modulate(&encode_with(msg, &FIGS_ITA2), 2_210.0 + 225.0, 2_210.0 - 225.0, 50.0);
        let got = decode(&a, |r| {
            r.set("ita2", 1.0);
            r.set("shift", 450.0);
            r.set("baud", 50.0);
            r.set_polarity(true);
        });
        assert_eq!(got, msg);
    }
}
