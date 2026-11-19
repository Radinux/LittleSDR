//! Synthetic CW band through the skimmer: one station at a given SNR (dB in 50 Hz) and
//! speed, at the given sample rate. Prints what was decoded and the CPU load.
//!   cargo run --release --example cw_synth -- [snr_db] [wpm] [rate]

use littlesdr::cw::{encode, CwSkimmer};
use littlesdr::dsp::C32;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let snr: f64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let wpm: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(22.0);
    let rate: f64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(2_400_000.0);
    let text = "CQ CQ DE F4ABC F4ABC PSE K";
    let dit = 1.2 / wpm;
    let mut iv: Vec<(bool, f64)> = Vec::new();
    for (wi, w) in text.split(' ').enumerate() {
        if wi > 0 {
            iv.push((false, 7.0 * dit));
        }
        for (ci, c) in w.chars().enumerate() {
            if ci > 0 {
                iv.push((false, 3.0 * dit));
            }
            for (ei, e) in encode(c).unwrap().chars().enumerate() {
                if ei > 0 {
                    iv.push((false, dit));
                }
                iv.push((true, if e == '.' { dit } else { 3.0 * dit }));
            }
        }
    }
    iv.push((false, 10.0 * dit));
    let total: f64 = iv.iter().map(|x| x.1).sum();
    let key = |t: f64| {
        let mut t = t % total;
        for &(on, d) in &iv {
            if t < d {
                return if on { (t.min(d - t) / 0.004).min(1.0) as f32 } else { 0.0 };
            }
            t -= d;
        }
        0.0
    };
    // Noise: sigma per component; signal amplitude for the SNR in 50 Hz
    let sigma = 0.02f64;
    let n0 = 2.0 * sigma * sigma / rate; // power per Hz
    let amp = (10f64.powf(snr / 10.0) * n0 * 50.0).sqrt() as f32;
    let (center, f) = (7_030_000.0, 7_012_340.0);
    let mut sk = CwSkimmer::new();
    sk.set("enabled", 1.0);
    let mut seed = 1u32;
    let mut rng = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 + 0.5) / 4_294_967_296.0
    };
    let secs = 3.0 * total;
    let block = 16384;
    let mut n = 0usize;
    let mut spent = 0.0;
    let mut buf = Vec::with_capacity(block);
    while (n as f64) < secs * rate {
        buf.clear();
        for k in 0..block {
            let t = (n + k) as f64 / rate;
            let (u, v) = (rng(), rng());
            let r = (-2.0 * u.ln()).sqrt() * sigma as f32;
            let th = 2.0 * std::f32::consts::PI * v;
            let ph = 2.0 * std::f64::consts::PI * (f - center) * t;
            let k = amp * key(t);
            buf.push(C32::new(r * th.cos() + k * ph.cos() as f32, r * th.sin() + k * ph.sin() as f32));
        }
        let t0 = Instant::now();
        sk.feed_iq(&buf, rate, center, 7_020_000.0);
        spent += t0.elapsed().as_secs_f64();
        n += block;
    }
    println!("{snr} dB, {wpm} WPM, {:.0} kS/s: CPU {:.1} % of one core", rate / 1e3, 100.0 * spent / secs);
    println!("  {}", sk.json(false));
    for (f, t, _) in sk.lines() {
        println!("  {:.2} kHz  {t}", f / 1e3);
    }
}
