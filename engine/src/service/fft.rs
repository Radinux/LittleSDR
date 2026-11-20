//! Display spectrum: FFT lines of the IQ stream for the waterfall, the smoothed trace and
//! the peak hold over them, and what the UI reads from them (signal level in the channel,
//! occupied bandwidth). The renderer reads the history directly.

use crate::dsp::C32;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

pub const HISTORY_ROWS: usize = 1024;
/// Memory cap of the history (floats): fewer rows at very high resolutions (64 MB)
const HISTORY_FLOATS: usize = 16 << 20;
/// Most FFTs averaged into one display line (beyond, they are spaced out)
const MAX_AVG: usize = 16;

/// FFT lines (dB, lowest frequency first) in a ring, as the renderer uploads them
pub struct FftHistory {
    pub bins: usize,
    pub rows: usize,
    pub lines: Vec<f32>,
    pub latest: Vec<f32>,
    pub peak: Vec<f32>,
    pub peak_hold: bool,
    /// Lines written since (re)allocation
    pub written: u64,
    /// Bumps when bins/rows change
    pub generation: u64,
}

impl FftHistory {
    pub fn new(bins: usize) -> FftHistory {
        let rows = (HISTORY_FLOATS / bins.max(1)).clamp(128, HISTORY_ROWS);
        FftHistory { bins, rows, lines: vec![-150.0; bins * rows], latest: vec![-150.0; bins], peak: vec![-150.0; bins], peak_hold: false, written: 0, generation: 1 }
    }

    pub fn resize(&mut self, bins: usize) {
        let (g, ph) = (self.generation, self.peak_hold);
        *self = FftHistory::new(bins);
        self.generation = g + 1;
        self.peak_hold = ph;
    }

    /// One new line: stored, smoothed into the trace (`smoothing` = weight of the old trace),
    /// peak hold falling by `fall` dB per line
    pub fn push(&mut self, row: &[f32], smoothing: f32, fall: f32) {
        if row.len() != self.bins {
            return;
        }
        let r = (self.written % self.rows as u64) as usize;
        self.lines[r * self.bins..(r + 1) * self.bins].copy_from_slice(row);
        if self.written == 0 || smoothing <= 0.0 {
            self.latest.copy_from_slice(row);
        } else {
            for (l, &x) in self.latest.iter_mut().zip(row) {
                *l = *l * smoothing + x * (1.0 - smoothing);
            }
        }
        if self.peak_hold {
            for (p, &l) in self.peak.iter_mut().zip(&self.latest) {
                *p = l.max(*p - fall);
            }
        }
        self.written += 1;
    }

    /// Highest level (dB) between two offsets from the centre, `span` Hz across the bins
    pub fn level(&self, span: f64, lo: f64, hi: f64) -> f32 {
        if self.written == 0 || span <= 0.0 {
            return -150.0;
        }
        let b = |o: f64| (((o / span + 0.5) * self.bins as f64) as isize).clamp(0, self.bins as isize - 1) as usize;
        self.latest[b(lo)..=b(hi).max(b(lo))].iter().copied().fold(-150.0, f32::max)
    }

    /// Occupied width of the signal around offset `c` (FM), in [min, max]; 0 if none
    pub fn occupied(&self, span: f64, c: f64, min_bw: f64, max_bw: f64) -> f32 {
        if self.written == 0 || span <= 0.0 {
            return 0.0;
        }
        let line = &self.latest;
        let bins = line.len();
        let bin_hz = span / bins as f64;
        let mut sorted = line.clone();
        let k = bins / 5;
        let noise = *sorted.select_nth_unstable_by(k, f32::total_cmp).1;
        let ci = (((c / span + 0.5) * bins as f64) as isize).clamp(0, bins as isize - 1);
        let min_half = ((min_bw / 2.0 / bin_hz) as isize).max(1);
        let max_half = ((max_bw / 2.0 / bin_hz) as isize).max(min_half);
        let peak = (ci - min_half..=ci + min_half).filter(|&i| i >= 0 && (i as usize) < bins).map(|i| line[i as usize]).fold(-1e9f32, f32::max);
        if peak - noise < 8.0 {
            return 0.0;
        }
        let thr = (noise + 6.0).max(peak - 30.0);
        let quiet_run = ((1000.0 / bin_hz) as isize).max(2);
        let edge = |dir: isize| {
            let (mut quiet, mut last) = (0, 0);
            for k in 1..=max_half {
                let i = ci + dir * k;
                if i < 0 || i as usize >= bins {
                    break;
                }
                if line[i as usize] < thr {
                    quiet += 1;
                    if quiet >= quiet_run {
                        break;
                    }
                } else {
                    quiet = 0;
                    last = k;
                }
            }
            last
        };
        let half = edge(-1).max(edge(1));
        (2.0 * (half + 1) as f64 * bin_hz * 1.15).clamp(min_bw, max_bw) as f32
    }
}

/// FFT lines from the IQ stream at a given line rate
pub struct DisplayFft {
    /// FFT size in use: the requested one, smaller on narrow streams (see `effective`)
    pub size: usize,
    /// FFT size asked for in the settings (a maximum)
    pub requested: usize,
    /// Lines per second
    pub rate: f32,
    /// 0 rectangular, 1 Blackman, 2 Nuttall
    pub window: i32,
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    buf: Vec<C32>,
    /// Input samples to skip before the next FFT
    skip: usize,
    /// Power summed over the FFTs of the line being built, and how many
    acc: Vec<f32>,
    nacc: u32,
    /// Samples covered since the last line
    since: f64,
    /// Power averaged across lines, for streams too narrow to average within a line
    ema: Vec<f32>,
    ema_on: bool,
}

/// FFT size for a stream: the one asked for, but no longer than 4 line intervals. On a narrow
/// stream (KiwiSDR 12 kHz: 8192 points = 0.7 s) a long FFT gives one noisy line now and then
/// with bins finer than any pixel: sand. 12 kHz at 25 lines/s: 1024 points, 12 Hz per bin.
fn effective(requested: usize, sample_rate: f64, rate: f32) -> usize {
    let cap = ((4.0 * sample_rate / rate as f64) as usize).max(512);
    let pow2 = 1usize << (usize::BITS - 1 - cap.leading_zeros());
    requested.min(pow2)
}

impl DisplayFft {
    pub fn new(size: usize, rate: f32, window: i32) -> DisplayFft {
        let size = size.clamp(256, 1 << 17);
        let mut d = DisplayFft {
            size: 0,
            requested: size,
            rate: rate.clamp(1.0, 120.0),
            window,
            fft: FftPlanner::new().plan_fft_forward(1),
            win: Vec::new(),
            buf: Vec::new(),
            skip: 0,
            acc: Vec::new(),
            nacc: 0,
            since: 0.0,
            ema: Vec::new(),
            ema_on: false,
        };
        d.plan(size);
        d
    }

    /// (Re)builds the transform for `size` points
    fn plan(&mut self, size: usize) {
        let window = self.window;
        let n = size as f32;
        let win: Vec<f32> = (0..size)
            .map(|i| {
                let x = 2.0 * std::f32::consts::PI * i as f32 / n;
                match window {
                    0 => 1.0,
                    1 => 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos(),
                    _ => 0.355_768 - 0.487_396 * x.cos() + 0.144_232 * (2.0 * x).cos() - 0.012_604 * (3.0 * x).cos(),
                }
            })
            .collect();
        // dB of a full-scale tone: normalise by the window's sum
        let sum: f32 = win.iter().sum();
        self.win = win.iter().map(|w| w / sum).collect();
        self.fft = FftPlanner::new().plan_fft_forward(size);
        self.size = size;
        self.buf.clear();
        self.skip = 0;
        self.acc = vec![0.0; size];
        self.nacc = 0;
        self.since = 0.0;
        self.ema = vec![0.0; size];
        self.ema_on = false;
    }

    /// New samples at `sample_rate`; complete lines appended to `out` (each `size` long).
    ///
    /// Each line is the average power of every FFT since the previous one (Welch, frames 50 %
    /// overlapped): a single FFT per line leaves the noise floor grainy, "moving sand", and
    /// zoomed out (highest bin per pixel) it turns into bright dots.
    pub fn feed(&mut self, iq: &[C32], sample_rate: f64, out: &mut Vec<Vec<f32>>) {
        let eff = effective(self.requested, sample_rate, self.rate);
        if eff != self.size {
            self.plan(eff);
        }
        let hop = (sample_rate / self.rate as f64).max(1.0);
        let step = ((hop / MAX_AVG as f64) as usize).max(self.size / 2).max(1);
        let mut i = 0;
        while i < iq.len() {
            if self.skip > 0 {
                let n = self.skip.min(iq.len() - i);
                self.skip -= n;
                i += n;
                continue;
            }
            let n = (self.size - self.buf.len()).min(iq.len() - i);
            self.buf.extend_from_slice(&iq[i..i + n]);
            i += n;
            if self.buf.len() < self.size {
                continue;
            }
            let mut v: Vec<Complex32> = self.buf.iter().zip(&self.win).map(|(z, w)| Complex32::new(z.re * w, z.im * w)).collect();
            self.fft.process(&mut v);
            for (a, z) in self.acc.iter_mut().zip(&v) {
                *a += z.norm_sqr();
            }
            self.nacc += 1;
            self.since += step as f64;
            if self.since >= hop {
                let (half, k) = (self.size / 2, 1.0 / self.nacc as f32);
                // Fewer than 4 FFTs in this line (narrow stream): also average across lines,
                // ~4 lines' worth (160 ms at 25/s)
                let old = if self.ema_on { (4u32.saturating_sub(self.nacc) as f32 / 4.0 * 0.8).max(0.0) } else { 0.0 };
                for (e, a) in self.ema.iter_mut().zip(&self.acc) {
                    *e = *e * old + a * k * (1.0 - old);
                }
                self.ema_on = true;
                out.push((0..self.size).map(|b| 10.0 * (self.ema[(b + half) % self.size] + 1e-20).log10()).collect());
                self.acc.iter_mut().for_each(|a| *a = 0.0);
                self.nacc = 0;
                self.since = (self.since - hop).min(hop);
            }
            if step >= self.size {
                self.buf.clear();
                self.skip = step - self.size;
            } else {
                self.buf.drain(..step);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White noise: averaged lines must be smooth (a single FFT per line: ~5.6 dB spread)
    #[test]
    fn noise_floor_is_smooth() {
        let rate = 2_400_000.0;
        let mut f = DisplayFft::new(4096, 25.0, 2);
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let iq: Vec<C32> = (0..240_000).map(|_| C32::new(rnd(), rnd())).collect();
        let mut rows = Vec::new();
        f.feed(&iq, rate, &mut rows);
        assert_eq!(rows.len(), 2, "lines at 25/s over 0.1 s");
        let r = &rows[1];
        let mean = r.iter().sum::<f32>() / r.len() as f32;
        let sd = (r.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / r.len() as f32).sqrt();
        assert!(sd < 1.8, "noise floor spread {sd:.2} dB");
    }

    /// KiwiSDR-like 12 kHz stream with 8192 points asked: the FFT shrinks and stays smooth
    #[test]
    fn narrow_stream_is_smooth() {
        let rate = 12_000.0;
        let mut f = DisplayFft::new(8192, 25.0, 2);
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let iq: Vec<C32> = (0..24_000).map(|_| C32::new(rnd(), rnd())).collect();
        let mut rows = Vec::new();
        f.feed(&iq, rate, &mut rows);
        assert_eq!(f.size, 1024, "effective size");
        assert!(rows.len() >= 20, "{} lines in 2 s", rows.len());
        let r = rows.last().unwrap();
        let mean = r.iter().sum::<f32>() / r.len() as f32;
        let sd = (r.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / r.len() as f32).sqrt();
        assert!(sd < 3.5, "noise floor spread {sd:.2} dB");
    }

    #[test]
    fn tone_lands_in_its_bin_at_0_db() {
        let mut f = DisplayFft::new(1024, 20.0, 2);
        let rate = 1_024_000.0;
        let iq: Vec<C32> = (0..200_000).map(|n| {
            let p = 2.0 * std::f64::consts::PI * 100_000.0 * n as f64 / rate;
            C32::new(p.cos() as f32, p.sin() as f32)
        }).collect();
        let mut rows = Vec::new();
        f.feed(&iq, rate, &mut rows);
        assert!(rows.len() >= 3, "{} rows", rows.len());
        let r = &rows[0];
        let (k, &mx) = r.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        assert_eq!(k, 512 + 100, "bin");
        assert!(mx.abs() < 1.0, "{mx} dB");
        let mut h = FftHistory::new(1024);
        h.push(r, 0.0, 0.0);
        assert!((h.level(rate, 90_000.0, 110_000.0) - mx).abs() < 0.01);
        assert!(h.level(rate, -50_000.0, -40_000.0) < -60.0);
    }
}
