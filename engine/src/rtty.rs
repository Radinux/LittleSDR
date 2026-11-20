//! RTTY decoder: 2-tone FSK, asynchronous start/stop framing, 5-bit Baudot (ITA2) text.
//!
//! Works on the demodulated audio (USB / LSB): the two tones are brought down around 0 Hz,
//! low-passed and decimated, an FM discriminator gives the instantaneous frequency, a filter
//! one half-bit long smooths it and a slicer decides mark / space. The slicer threshold
//! follows the two tone levels, so a receiver tuned a little off still decodes.
//!
//! Polarity differs between services (amateurs: mark = higher RF frequency; DWD weather: the
//! reverse). By default both are framed in parallel and the one with valid stop bits wins.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    /// Both tried, the one that frames correctly wins
    Auto,
    /// Mark is the higher RF frequency (amateur convention)
    Normal,
    /// Mark is the lower RF frequency (e.g. DWD weather)
    Reversed,
}

/// Start/stop framing and Baudot decoding for one polarity
struct Framer {
    /// Mark is the higher audio tone
    mark_high: bool,
    last: bool,
    uart: Uart,
    figs: bool,
    text: String,
    /// Share of recent characters with a valid stop bit: near 1 on a real signal with the
    /// right polarity, around 0.5 on noise or the wrong polarity
    quality: f32,
    open: bool,
    pending: VecDeque<char>,
    chars: u64,
    errors: u64,
}

impl Framer {
    fn new(mark_high: bool) -> Framer {
        Framer {
            mark_high,
            last: true,
            uart: Uart::Idle,
            figs: false,
            text: String::new(),
            quality: 0.0,
            open: false,
            pending: VecDeque::new(),
            chars: 0,
            errors: 0,
        }
    }

    fn step(&mut self, high: bool, bit_len: f64, cfg: &FrameCfg) {
        let mark = high == self.mark_high;
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
                if t < bit_len * (bit as f64 + 0.5) {
                    Uart::Char { t, code, bit }
                } else {
                    match bit {
                        0 if mark => Uart::Idle, // glitch, not a start bit
                        0 => Uart::Char { t, code, bit: 1 },
                        1..=5 => Uart::Char { t, code: code | ((mark as u8) << (bit - 1)), bit: bit + 1 },
                        _ => {
                            self.end_char(code, mark, cfg);
                            Uart::Idle
                        }
                    }
                }
            }
        };
        self.last = mark;
    }

    fn end_char(&mut self, code: u8, stop_ok: bool, cfg: &FrameCfg) {
        self.quality += 0.15 * ((stop_ok as u8 as f32) - self.quality);
        if !cfg.squelch {
            self.open = true;
        } else if self.open && (self.quality < 0.6 || cfg.in_band < 0.3) {
            self.open = false;
            self.pending.clear();
        } else if !self.open && self.quality > 0.8 && cfg.in_band > 0.45 {
            self.open = true;
            self.text.extend(self.pending.drain(..));
        }
        if !stop_ok {
            self.errors += 1;
            return;
        }
        self.chars += 1;
        match code {
            CODE_FIGS => self.figs = true,
            CODE_LTRS => self.figs = false,
            _ => {
                let figs = if cfg.ita2 { &FIGS_ITA2 } else { &FIGS };
                let c = if self.figs { figs[code as usize] } else { LTRS[code as usize] };
                if c == ' ' && cfg.usos {
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

struct FrameCfg {
    squelch: bool,
    in_band: f32,
    ita2: bool,
    usos: bool,
}

pub struct Rtty {
    pub center: f64,
    pub shift: f64,
    pub baud: f64,
    /// Sideband: mark (higher RF) is the higher audio tone in USB, the lower in LSB
    pub usb: bool,
    pub polarity: Polarity,
    /// Unshift on space: a space after figures returns to letters (common practice)
    pub usos: bool,
    /// International ITA2 figures instead of the US amateur variant
    pub ita2: bool,
    /// Squelch enabled (`sql` parameter)
    pub squelch: bool,
    nco: Nco,
    lp: DecimFir<C32>,
    prev: C32,
    smooth: VecDeque<f32>,
    smooth_sum: f32,
    /// Levels of the higher and lower tone (Hz from `center`), followed so that a receiver
    /// tuned a little off still decodes
    hi_lvl: f32,
    lo_lvl: f32,
    /// Share of the 300-3000 Hz power that falls on the two tones (~1 on RTTY, ~0.1 on noise)
    in_band: f32,
    band_pow: f32,
    wide_pow: f32,
    wide_nco: Nco,
    wide_lp: DecimFir<C32>,
    /// [mark = higher audio tone, mark = lower audio tone]
    framers: [Framer; 2],
    active: usize,
}

impl Rtty {
    pub fn new() -> Rtty {
        let mut r = Rtty {
            center: 2_210.0,
            shift: 170.0,
            baud: 45.45,
            usb: true,
            polarity: Polarity::Auto,
            usos: true,
            ita2: false,
            squelch: true,
            nco: Nco::new(),
            lp: DecimFir::new(vec![1.0], DECIM),
            prev: C32::new(1.0, 0.0),
            smooth: VecDeque::new(),
            smooth_sum: 0.0,
            hi_lvl: 85.0,
            lo_lvl: -85.0,
            in_band: 0.0,
            band_pow: 0.0,
            wide_pow: 1e-12,
            wide_nco: Nco::new(),
            wide_lp: DecimFir::new(lowpass(1_400.0 / AUDIO_RATE, 300.0 / AUDIO_RATE), DECIM),
            framers: [Framer::new(true), Framer::new(false)],
            active: 0,
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
        self.hi_lvl = half as f32;
        self.lo_lvl = -half as f32;
    }

    /// `rev`: < 0 auto, 0 normal, > 0 reversed
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "shift" => self.shift = v.clamp(50.0, 1000.0),
            "baud" => self.baud = v.clamp(20.0, 300.0),
            "center" => {
                self.center = v.clamp(300.0, 3000.0);
                return true;
            }
            "rev" | "reverse" => {
                self.polarity = if v < 0.0 {
                    Polarity::Auto
                } else if v == 0.0 {
                    Polarity::Normal
                } else {
                    Polarity::Reversed
                };
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

    /// Sideband of the receiver (the mark is the higher RF frequency)
    pub fn set_sideband(&mut self, usb: bool) {
        self.usb = usb;
    }

    /// The framer matching a forced polarity
    fn forced(&self) -> Option<usize> {
        let normal = if self.usb { 0 } else { 1 }; // normal: mark = higher RF
        match self.polarity {
            Polarity::Auto => None,
            Polarity::Normal => Some(normal),
            Polarity::Reversed => Some(1 - normal),
        }
    }

    /// Polarity currently decoded
    pub fn decoding(&self) -> Polarity {
        if (self.active == 0) == self.usb {
            Polarity::Normal
        } else {
            Polarity::Reversed
        }
    }

    pub fn describe(&self) -> String {
        let f = &self.framers[self.active];
        let pol = match (self.polarity, self.decoding()) {
            (Polarity::Auto, Polarity::Normal) => "auto: normal",
            (Polarity::Auto, _) => "auto: reversed",
            (Polarity::Normal, _) => "normal",
            (Polarity::Reversed, _) => "reversed",
        };
        format!(
            "RTTY {:.2} Bd, shift {:.0} Hz, center {:.0} Hz, polarity {}{}, {} chars, {} framing errors, squelch {} (tones {:.0} % of the band)",
            self.baud,
            self.shift,
            self.center,
            pol,
            if self.ita2 { ", ITA2" } else { "" },
            f.chars,
            f.errors,
            if !self.squelch { "off" } else if f.open { "open" } else { "closed" },
            self.in_band * 100.0
        )
    }

    pub fn take_text(&mut self) -> String {
        let a = self.active;
        self.framers[1 - a].text.clear();
        std::mem::take(&mut self.framers[a].text)
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
        let cfg = FrameCfg { squelch: self.squelch, in_band: self.in_band, ita2: self.ita2, usos: self.usos };
        let bit_len = RATE / self.baud;
        let k = (RATE / (2.0 * PI)) as f32;
        let forced = self.forced();
        for s in d {
            let f = (s * self.prev.conj()).arg() * k; // Hz from the center
            self.prev = s;
            self.smooth_sum += f - self.smooth.pop_front().unwrap();
            self.smooth.push_back(f);
            let f = self.smooth_sum / self.smooth.len() as f32;
            let high = f > (self.hi_lvl + self.lo_lvl) / 2.0;
            self.learn(f, high);
            for (i, fr) in self.framers.iter_mut().enumerate() {
                if forced.map_or(true, |w| w == i) {
                    fr.step(high, bit_len, &cfg);
                }
            }
        }
        self.active = match forced {
            Some(w) => w,
            None => {
                // Switch only to a framer whose squelch is open and that frames clearly better
                let (a, b) = (self.active, 1 - self.active);
                let (fa, fb) = (&self.framers[a], &self.framers[b]);
                if fb.open && (!fa.open || fb.quality > fa.quality + 0.15) {
                    b
                } else {
                    a
                }
            }
        };
    }

    /// Follows the two tone levels, kept at least a third of the shift apart
    fn learn(&mut self, f: f32, high: bool) {
        let a = (1.0 / (RATE * 0.25)) as f32;
        if high {
            self.hi_lvl += a * (f - self.hi_lvl);
        } else {
            self.lo_lvl += a * (f - self.lo_lvl);
        }
        let half = (self.shift / 2.0) as f32;
        let mid = (self.hi_lvl + self.lo_lvl) / 2.0;
        if self.hi_lvl - self.lo_lvl < half * 0.66 {
            self.hi_lvl = mid + half * 0.33;
            self.lo_lvl = mid - half * 0.33;
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
    fn finds_the_polarity_by_itself() {
        // Reversed signal (mark = lower tone in USB), polarity left on auto
        let a = modulate(&encode(MSG), 2_125.0, 2_295.0, 45.45);
        let mut r = Rtty::new();
        for ch in a.chunks(1000) {
            r.process(ch);
        }
        assert_eq!(r.take_text(), MSG);
        assert_eq!(r.decoding(), Polarity::Reversed);
        // Forced to the wrong polarity it can't be read (some garbage may frame by chance)
        let got = decode(&a, |r| {
            r.set("rev", 0.0);
        });
        assert_ne!(got, MSG);
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
        let got = decode(&a, |r| r.set_sideband(false));
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
                e.process(IqBlock { samples: ch.to_vec(), rate, center, settle_ms: 0.0, time: None }, &mut out);
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
            r.set_sideband(true);
        });
        assert_eq!(got, msg);
    }
}
