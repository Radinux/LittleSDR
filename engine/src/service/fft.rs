//! Display spectrum: FFT lines of the IQ stream for the waterfall, the smoothed trace and
//! the peak hold over them, and what the UI reads from them (signal level in the channel,
//! occupied bandwidth). The renderer reads the history directly.

use crate::dsp::C32;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

pub const HISTORY_ROWS: usize = 1024;

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
        FftHistory { bins, rows: HISTORY_ROWS, lines: vec![-150.0; bins * HISTORY_ROWS], latest: vec![-150.0; bins], peak: vec![-150.0; bins], peak_hold: false, written: 0, generation: 1 }
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
    pub size: usize,
    /// Lines per second
    pub rate: f32,
    /// 0 rectangular, 1 Blackman, 2 Nuttall
    pub window: i32,
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    buf: Vec<C32>,
    /// Input samples to skip before the next line
    skip: usize,
}

impl DisplayFft {
    pub fn new(size: usize, rate: f32, window: i32) -> DisplayFft {
        let size = size.clamp(256, 1 << 17);
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
        let win = win.iter().map(|w| w / sum).collect();
        DisplayFft { size, rate: rate.clamp(1.0, 120.0), window, fft: FftPlanner::new().plan_fft_forward(size), win, buf: Vec::new(), skip: 0 }
    }

    /// New samples at `sample_rate`; complete lines appended to `out` (each `size` long)
    pub fn feed(&mut self, iq: &[C32], sample_rate: f64, out: &mut Vec<Vec<f32>>) {
        let hop = ((sample_rate / self.rate as f64) as usize).max(self.size / 4);
        let mut i = 0;
        while i < iq.len() {
            if self.skip > 0 {
                let n = self.skip.min(iq.len() - i);
                self.skip -= n;
                i += n;
                continue;
            }
            let need = self.size - self.buf.len();
            let n = need.min(iq.len() - i);
            self.buf.extend_from_slice(&iq[i..i + n]);
            i += n;
            if self.buf.len() == self.size {
                let mut v: Vec<Complex32> = self.buf.iter().zip(&self.win).map(|(z, w)| Complex32::new(z.re * w, z.im * w)).collect();
                self.fft.process(&mut v);
                let half = self.size / 2;
                let row: Vec<f32> = (0..self.size).map(|k| 10.0 * (v[(k + half) % self.size].norm_sqr() + 1e-20).log10()).collect();
                out.push(row);
                self.buf.clear();
                self.skip = hop.saturating_sub(self.size);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
