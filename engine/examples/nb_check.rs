//! Runs the noise blanker over a raw RTL-SDR capture (u8 IQ) and reports how much it blanks.
//!   cargo run --release --example nb_check -- capture.u8 [rate]
use littlesdr::dsp::{NoiseBlanker, C32};
use littlesdr::source::u8_to_iq;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("capture.u8");
    let rate: f64 = a.next().and_then(|r| r.parse().ok()).unwrap_or(2.4e6);
    let raw = std::fs::read(path).expect("read");
    let mut iq: Vec<C32> = Vec::new();
    u8_to_iq(&raw, &mut iq);
    let mut nb = NoiseBlanker::new(rate, 6.0);
    for chunk in iq.chunks_mut(8192) {
        nb.process(chunk);
    }
    println!("{} samples, {} blanked ({:.3} %)", iq.len(), nb.blanked, nb.blanked as f64 / iq.len() as f64 * 100.0);
}
