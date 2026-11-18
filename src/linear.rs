//! AM, SSB and CW demodulation, from a 48 kHz IF centered on the tuned frequency.
//!
//! - AM: channel filter, envelope, normalized by the carrier level (which is also the AGC:
//!   weak and strong stations come out at the same loudness).
//! - SSB / CW: the wanted band is shifted to 0 Hz, decimated to 12 kHz where the sharp
//!   channel filter and the AGC run cheaply, brought back to 48 kHz and shifted to its audio
//!   frequency; the real part is the sound. CW is heard at `pitch` Hz.
//!
//! Bandwidth, sideband and CW pitch change live: filters keep their history, oscillators keep
//! their phase, nothing restarts.

use crate::dsp::{lowpass, DecimFir, Frame, Nco, SwapFir, Upsample, C32};
use std::collections::VecDeque;

pub const IF_RATE: f64 = 48_000.0;
const NARROW_RATE: f64 = 12_000.0;
const NARROW_DECIM: usize = 4;
/// Lowest audio frequency kept in SSB
pub const SSB_LOW_CUT: f64 = 300.0;
const AM_SCALE: f32 = 0.35;

/// Gain control on complex samples: peak envelope with fast attack, hang, then slow decay.
/// A short look-ahead lets the gain drop before a strong signal arrives: no overshoot click.
pub struct Agc {
    target: f32,
    max_gain: f32,
    env: f32,
    attack: f32,
    decay: f32,
    hang_len: usize,
    hang: usize,
    delay: VecDeque<C32>,
}

impl Agc {
    pub fn new(rate: f64, attack_ms: f64, hang_ms: f64, decay_ms: f64) -> Agc {
        let k = |ms: f64| (1.0 - (-1000.0 / (rate * ms)).exp()) as f32;
        let look = (rate * attack_ms * 2.0 / 1000.0).ceil() as usize;
        Agc {
            target: 0.3,
            max_gain: 3000.0, // ~70 dB: no signal, the noise stays at a reasonable level
            env: 0.0,
            attack: k(attack_ms),
            decay: k(decay_ms),
            hang_len: (rate * hang_ms / 1000.0) as usize,
            hang: 0,
            delay: std::iter::repeat(C32::default()).take(look).collect(),
        }
    }

    pub fn gain(&self) -> f32 {
        self.target / self.env.max(self.target / self.max_gain)
    }

    pub fn process(&mut self, buf: &mut [C32]) {
        for z in buf.iter_mut() {
            let m = z.norm_sqr().sqrt();
            if m > self.env {
                self.env += self.attack * (m - self.env);
                self.hang = self.hang_len;
            } else if self.hang > 0 {
                self.hang -= 1;
            } else {
                self.env += self.decay * (m - self.env);
            }
            self.delay.push_back(*z);
            *z = self.delay.pop_front().unwrap() * self.gain();
        }
    }
}

// ---------------------------------------------------------------- AM

pub struct AmDemod {
    bw: f64,
    chan: SwapFir<C32>,
    carrier: f32,
    a: f32,
}

impl AmDemod {
    pub fn new(bw: f64) -> AmDemod {
        AmDemod {
            bw,
            chan: SwapFir::new(Self::taps(bw), 1, 480),
            carrier: 0.0,
            a: (1.0 - (-1.0 / (IF_RATE * 0.08)).exp()) as f32, // 80 ms
        }
    }

    fn taps(bw: f64) -> Vec<f32> {
        let tw = (bw * 0.15).clamp(300.0, 1500.0);
        lowpass((bw / 2.0 + tw / 2.0) / IF_RATE, tw / IF_RATE)
    }

    pub fn set_bandwidth(&mut self, bw: f64) {
        self.bw = bw;
        self.chan.set_taps(Self::taps(bw));
    }

    pub fn bandwidth(&self) -> f64 {
        self.bw
    }

    pub fn process(&mut self, iq: &[C32], out: &mut Vec<Frame>) {
        let mut ch = Vec::with_capacity(iq.len());
        self.chan.process(iq, &mut ch);
        for z in ch {
            let m = z.norm_sqr().sqrt();
            if self.carrier <= 0.0 {
                self.carrier = m;
            }
            self.carrier += self.a * (m - self.carrier);
            // 100 % modulation -> +-0.35; the carrier itself (DC) is removed. With no station
            // the "carrier" is the noise envelope and this ratio is ~0.5 RMS of noise: the
            // scale keeps that hiss clear of the limiter, speech at SSB-like loudness.
            let v = (m - self.carrier) / self.carrier.max(1e-4) * AM_SCALE;
            out.push(Frame::mono(v.clamp(-1.0, 1.0)));
        }
    }
}

// ---------------------------------------------------------------- SSB / CW

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sideband {
    Upper,
    Lower,
    Cw,
}

pub struct SsbDemod {
    sb: Sideband,
    bw: f64,
    pitch: f64,
    down: Nco,
    decim: DecimFir<C32>,
    chan: SwapFir<C32>,
    agc: Agc,
    up: Upsample<C32>,
    upmix: Nco,
}

impl SsbDemod {
    pub fn new(sb: Sideband, bw: f64, pitch: f64) -> SsbDemod {
        // Fixed 48 -> 12 kHz stages: pass the widest channel (+-2.2 kHz), stop before what
        // would fold back into it
        let wide = lowpass(5_850.0 / IF_RATE, 7_300.0 / IF_RATE);
        let (hang, decay) = if sb == Sideband::Cw { (200.0, 150.0) } else { (300.0, 300.0) };
        SsbDemod {
            sb,
            bw,
            pitch,
            down: Nco::new(),
            decim: DecimFir::new(wide.clone(), NARROW_DECIM),
            chan: SwapFir::new(Self::taps(bw), 1, 120),
            agc: Agc::new(NARROW_RATE, 2.0, hang, decay),
            up: Upsample::new(&wide, NARROW_DECIM),
            upmix: Nco::new(),
        }
    }

    fn taps(bw: f64) -> Vec<f32> {
        let tw = (bw * 0.12).clamp(50.0, 300.0);
        lowpass((bw / 2.0 + tw / 2.0) / NARROW_RATE, tw / NARROW_RATE)
    }

    /// Center of the received band, relative to the tuned frequency
    fn center(&self) -> f64 {
        match self.sb {
            Sideband::Upper => SSB_LOW_CUT + self.bw / 2.0,
            Sideband::Lower => -(SSB_LOW_CUT + self.bw / 2.0),
            Sideband::Cw => 0.0,
        }
    }

    /// Where that center lands in the audio
    fn audio_center(&self) -> f64 {
        match self.sb {
            Sideband::Cw => self.pitch,
            _ => self.center(),
        }
    }

    pub fn set_bandwidth(&mut self, bw: f64) {
        self.bw = bw;
        self.chan.set_taps(Self::taps(bw));
    }

    pub fn set_sideband(&mut self, sb: Sideband) {
        self.sb = sb;
    }

    pub fn set_pitch(&mut self, hz: f64) {
        self.pitch = hz;
    }

    pub fn bandwidth(&self) -> f64 {
        self.bw
    }

    pub fn agc_gain_db(&self) -> f32 {
        20.0 * self.agc.gain().log10()
    }

    pub fn process(&mut self, iq: &[C32], out: &mut Vec<Frame>) {
        let mut x = iq.to_vec();
        self.down.set(self.center(), IF_RATE);
        self.down.mix(&mut x);
        let mut n = Vec::with_capacity(x.len() / NARROW_DECIM + 2);
        self.decim.process(&x, &mut n);
        let mut c = Vec::with_capacity(n.len());
        self.chan.process(&n, &mut c);
        self.agc.process(&mut c);
        let mut y = Vec::with_capacity(c.len() * NARROW_DECIM);
        self.up.process(&c, &mut y);
        self.upmix.set(-self.audio_center(), IF_RATE);
        self.upmix.mix(&mut y);
        out.extend(y.into_iter().map(|z| Frame::mono(z.re)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn tone(f: f64, amp: f32, n: usize) -> Vec<C32> {
        (0..n)
            .map(|i| {
                let p = 2.0 * PI * f * i as f64 / IF_RATE;
                C32::new(p.cos() as f32, p.sin() as f32) * amp
            })
            .collect()
    }

    /// Power of `f` in the audio and the total power (after the first `skip` samples)
    fn analyze(a: &[Frame], f: f64, skip: usize) -> (f64, f64) {
        let mut acc = (0.0f64, 0.0f64);
        let mut tot = 0.0f64;
        for (i, s) in a.iter().enumerate().skip(skip) {
            let p = 2.0 * PI * f * i as f64 / IF_RATE;
            acc.0 += s.l as f64 * p.cos();
            acc.1 += s.l as f64 * p.sin();
            tot += (s.l as f64).powi(2);
        }
        let n = (a.len() - skip) as f64;
        (2.0 * (acc.0 * acc.0 + acc.1 * acc.1) / n / n, tot / n)
    }

    fn run_ssb(sb: Sideband, input: &[C32]) -> Vec<Frame> {
        let mut d = SsbDemod::new(sb, 2_700.0, 700.0);
        let mut out = Vec::new();
        for ch in input.chunks(1000) {
            d.process(ch, &mut out);
        }
        out
    }

    #[test]
    fn usb_hears_the_upper_side_only() {
        // A carrier 1 kHz above the tuned frequency is a 1 kHz tone in USB...
        let out = run_ssb(Sideband::Upper, &tone(1_000.0, 0.01, 48_000));
        assert_eq!(out.len(), 48_000);
        let (p, tot) = analyze(&out, 1_000.0, 12_000);
        assert!(p / tot > 0.98, "USB tone purity {}", p / tot);
        assert!(p.sqrt() > 0.1, "AGC brought it up: {}", p.sqrt());
        // ...and nothing in LSB
        let out = run_ssb(Sideband::Lower, &tone(1_000.0, 0.01, 48_000));
        let (_, tot_lsb) = analyze(&out, 1_000.0, 12_000);
        assert!(10.0 * (tot_lsb / tot).log10() < -50.0, "opposite sideband {} dB", 10.0 * (tot_lsb / tot).log10());
    }

    #[test]
    fn lsb_mirrors_usb() {
        let out = run_ssb(Sideband::Lower, &tone(-1_500.0, 0.01, 48_000));
        let (p, tot) = analyze(&out, 1_500.0, 12_000);
        assert!(p / tot > 0.98, "LSB tone purity {}", p / tot);
    }

    #[test]
    fn cw_carrier_is_heard_at_the_pitch() {
        // Carrier 100 Hz above the dial: heard at 700 + 100 Hz
        let out = run_ssb(Sideband::Cw, &tone(100.0, 0.01, 48_000));
        let (p, tot) = analyze(&out, 800.0, 12_000);
        assert!(p / tot > 0.98, "CW purity {}", p / tot);
        // A carrier 1 kHz away is outside the 500 Hz filter
        let mut d = SsbDemod::new(Sideband::Cw, 500.0, 700.0);
        let mut far = Vec::new();
        let mut near = Vec::new();
        let x = tone(1_000.0, 0.01, 48_000);
        let y = tone(0.0, 0.01, 48_000);
        let mix: Vec<C32> = x.iter().zip(&y).map(|(a, b)| *a + *b).collect();
        for ch in mix.chunks(1000) {
            d.process(ch, &mut far);
        }
        let (p_far, _) = analyze(&far, 1_700.0, 12_000);
        let mut d = SsbDemod::new(Sideband::Cw, 500.0, 700.0);
        for ch in y.chunks(1000) {
            d.process(ch, &mut near);
        }
        let (p_near, _) = analyze(&near, 700.0, 12_000);
        assert!(10.0 * (p_far / p_near).log10() < -50.0, "rejection {} dB", 10.0 * (p_far / p_near).log10());
    }

    #[test]
    fn bandwidth_change_is_seamless() {
        let x = tone(1_000.0, 0.01, 96_000);
        let mut d = SsbDemod::new(Sideband::Upper, 2_700.0, 700.0);
        let mut out = Vec::new();
        for (i, ch) in x.chunks(1000).enumerate() {
            if i == 48 {
                d.set_bandwidth(1_800.0);
            }
            d.process(ch, &mut out);
        }
        // No jump: the sample-to-sample step never exceeds what a 1 kHz sine allows
        let amp = out[40_000..47_000].iter().map(|f| f.l.abs()).fold(0.0, f32::max);
        let max_step = (2.0 * PI * 1_000.0 / IF_RATE) as f32 * amp * 1.2;
        let worst = out[30_000..].windows(2).map(|w| (w[1].l - w[0].l).abs()).fold(0.0, f32::max);
        assert!(worst < max_step, "step {worst} > {max_step}");
    }

    #[test]
    fn am_recovers_the_modulation() {
        // 50 % modulated 1 kHz
        let n = 48_000;
        let car = tone(0.0, 0.02, n);
        let iq: Vec<C32> = car
            .iter()
            .enumerate()
            .map(|(i, z)| *z * (1.0 + 0.5 * (2.0 * PI * 1_000.0 * i as f64 / IF_RATE).sin() as f32))
            .collect();
        let mut d = AmDemod::new(9_000.0);
        let mut out = Vec::new();
        for ch in iq.chunks(777) {
            d.process(ch, &mut out);
        }
        let (p, tot) = analyze(&out, 1_000.0, 24_000);
        assert!(p / tot > 0.98, "AM purity {}", p / tot);
        // 50 % -> half of AM_SCALE, peak (analyze gives the RMS)
        let want = 0.5 * AM_SCALE as f64;
        assert!((p.sqrt() * 2f64.sqrt() - want).abs() < 0.02, "level {}", p.sqrt());
    }
}
