//! Signal meter and ADC overload detection, from the raw IQ.
//!
//! An averaged FFT of the incoming band gives the power inside the listened channel and the
//! noise floor around it (median of the neighbouring bins, which ignores the other stations),
//! hence a signal level in dBFS and a signal-to-noise ratio. Alongside, the share of samples
//! at full scale tells when the gain is too high: a clipping ADC adds noise and false signals
//! everywhere in the band.

use crate::dsp::C32;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

const N: usize = 16_384;
/// Readings are refreshed this often (seconds of signal)
const PERIOD: f64 = 0.25;
/// A sample at or beyond this magnitude (per component) is taken as clipped
const FULL_SCALE: f32 = 0.995;

#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    /// Power in the channel (dB relative to a full-scale complex sine)
    pub signal_dbfs: f32,
    /// Noise floor scaled to the channel width (same reference)
    pub noise_dbfs: f32,
    /// (signal + noise) / noise in the channel
    pub snr_db: f32,
    /// Share of raw samples at full scale (%)
    pub clip_pct: f32,
    /// RMS level of the raw samples (dBFS): how much of the ADC range is used
    pub adc_dbfs: f32,
}

impl Reading {
    /// Advice on the hardware gain, if it is clearly wrong
    pub fn gain_advice(&self) -> Option<&'static str> {
        if self.clip_pct > 1.0 {
            Some("ADC overloaded: lower the gain")
        } else if self.adc_dbfs < -40.0 {
            Some("very little signal reaches the ADC: raise the gain")
        } else {
            None
        }
    }
}

pub struct Meter {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex32>,
    psd: Vec<f32>,
    frames: usize,
    fill: usize,
    skip: usize,
    wsum: f32,
    samples: u64,
    clipped: u64,
    energy: f64,
    pub reading: Option<Reading>,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    pub fn new() -> Meter {
        // Blackman-Harris: low leakage, so a strong neighbour doesn't raise the floor
        let window: Vec<f32> = (0..N)
            .map(|i| {
                let x = 2.0 * std::f32::consts::PI * i as f32 / N as f32;
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
            })
            .collect();
        let wsum = window.iter().map(|w: &f32| w * w).sum();
        Meter {
            fft: FftPlanner::new().plan_fft_forward(N),
            window,
            fill: 0,
            skip: 0,
            wsum,
            buf: vec![Complex32::default(); N],
            psd: vec![0.0; N],
            frames: 0,
            samples: 0,
            clipped: 0,
            energy: 0.0,
            reading: None,
        }
    }

    /// Feeds a raw block. `lo..hi` is the channel, in Hz relative to the hardware center.
    pub fn feed(&mut self, iq: &[C32], rate: f64, lo: f64, hi: f64) {
        for z in iq {
            if z.re.abs() >= FULL_SCALE || z.im.abs() >= FULL_SCALE {
                self.clipped += 1;
            }
            self.energy += z.norm_sqr() as f64;
        }
        self.samples += iq.len() as u64;
        // FFT frames are filled across blocks (sources deliver blocks of any size), with gaps
        // in between: ~8 frames per reading is plenty for a meter
        let gap = ((rate * PERIOD / 8.0) as usize).saturating_sub(N);
        let mut rest = iq;
        while !rest.is_empty() {
            if self.skip > 0 {
                let k = self.skip.min(rest.len());
                self.skip -= k;
                rest = &rest[k..];
                continue;
            }
            let k = (N - self.fill).min(rest.len());
            for (i, z) in rest[..k].iter().enumerate() {
                let w = self.window[self.fill + i];
                self.buf[self.fill + i] = Complex32::new(z.re * w, z.im * w);
            }
            self.fill += k;
            rest = &rest[k..];
            if self.fill == N {
                self.fft.process(&mut self.buf);
                for (p, b) in self.psd.iter_mut().zip(&self.buf) {
                    *p += b.norm_sqr() / self.wsum; // per-bin power, full-scale sine = N
                }
                self.frames += 1;
                self.fill = 0;
                self.skip = gap;
            }
        }
        if self.samples as f64 >= rate * PERIOD && self.frames > 0 {
            self.finish(rate, lo, hi);
        }
    }

    fn finish(&mut self, rate: f64, lo: f64, hi: f64) {
        let bin_hz = rate / N as f64;
        // FFT order: bin k is k*bin_hz for k < N/2, negative frequencies above
        let freq = |k: usize| if k < N / 2 { k as f64 * bin_hz } else { (k as f64 - N as f64) * bin_hz };
        let scale = 1.0 / (self.frames as f32 * N as f32);
        let (mut chan, mut nchan) = (0.0f32, 0usize);
        let width = (hi - lo).max(bin_hz);
        let span = (width * 20.0).max(50e3).min(rate * 0.45);
        let mid = (lo + hi) / 2.0;
        let mut around = Vec::new();
        for k in 0..N {
            let f = freq(k);
            let p = self.psd[k] * scale;
            if f >= lo - bin_hz / 2.0 && f <= hi + bin_hz / 2.0 {
                chan += p;
                nchan += 1;
            } else if (f - mid).abs() < span && f.abs() < rate * 0.45 && f.abs() > 2.0 * bin_hz {
                around.push(p); // (the DC spike is left out)
            }
        }
        if nchan > 0 && !around.is_empty() {
            around.sort_by(|a, b| a.total_cmp(b));
            // Lower quartile, not median: in a crowded band (49 m at night) more than half
            // the neighbouring bins can be stations. For noise averaged over ~8 frames the
            // lower quartile sits ~1.2 dB under the mean, corrected here.
            let floor = around[around.len() / 4] * 1.32 * nchan as f32;
            let db = |p: f32| 10.0 * p.max(1e-20).log10();
            self.reading = Some(Reading {
                signal_dbfs: db(chan),
                noise_dbfs: db(floor),
                snr_db: db(chan) - db(floor),
                clip_pct: self.clipped as f32 / self.samples as f32 * 100.0,
                adc_dbfs: db((self.energy / self.samples as f64) as f32),
            });
        }
        self.psd.iter_mut().for_each(|p| *p = 0.0);
        self.frames = 0;
        self.samples = 0;
        self.clipped = 0;
        self.energy = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Deterministic complex white noise of the given total power
    fn noise(power: f32, n: usize) -> Vec<C32> {
        let mut s = 0x1234_5678u32;
        let mut u = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32).max(1e-9)
        };
        let sigma = (power / 2.0).sqrt();
        (0..n)
            .map(|_| {
                // Box-Muller
                let (a, b) = (u(), u());
                let r = (-2.0 * a.ln()).sqrt() * sigma;
                let t = 2.0 * std::f32::consts::PI * b;
                C32::new(r * t.cos(), r * t.sin())
            })
            .collect()
    }

    #[test]
    fn measures_a_carrier_against_the_noise() {
        let rate = 1.024e6;
        let n = (rate * 0.3) as usize;
        let mut x = noise(1e-4, n); // -40 dBFS over the whole band
        let amp = 0.01f32; // -40 dBFS carrier at +100 kHz
        for (i, z) in x.iter_mut().enumerate() {
            let p = 2.0 * PI * 100e3 * i as f64 / rate;
            *z += C32::new(amp * p.cos() as f32, amp * p.sin() as f32);
        }
        let mut m = Meter::new();
        for ch in x.chunks(5_000) {
            m.feed(ch, rate, 100e3 - 5e3, 100e3 + 5e3);
        }
        let r = m.reading.expect("a reading");
        assert!((r.signal_dbfs + 40.0).abs() < 1.0, "signal {}", r.signal_dbfs);
        // Noise in 10 kHz of a 1.024 MHz band: -40 - 10log10(102.4) = -60.1 dBFS
        assert!((r.noise_dbfs + 60.1).abs() < 1.5, "noise {}", r.noise_dbfs);
        assert!((r.snr_db - 20.0).abs() < 1.5, "snr {}", r.snr_db);
        assert!(r.clip_pct == 0.0);
        assert!(r.gain_advice().is_none(), "{:?}", r); // -37 dBFS at the ADC is fine
    }

    #[test]
    fn flags_clipping() {
        let rate = 1.024e6;
        let x: Vec<C32> = noise(0.5, (rate * 0.3) as usize).into_iter().map(|z| C32::new(z.re.clamp(-1.0, 1.0), z.im.clamp(-1.0, 1.0))).collect();
        let mut m = Meter::new();
        for ch in x.chunks(N * 2) {
            m.feed(ch, rate, -5e3, 5e3);
        }
        let r = m.reading.unwrap();
        assert!(r.clip_pct > 5.0, "{}", r.clip_pct);
        assert_eq!(r.gain_advice(), Some("ADC overloaded: lower the gain"));
    }
}
