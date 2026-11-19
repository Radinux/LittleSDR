//! Decodes a DAB+ service from a raw RTL-SDR capture (u8 IQ, 2.048 MS/s) into a WAV file.
//!   dab_file capture.u8 [service] [out.wav]   (no service: lists the ensemble)
use littlesdr::audio::{AudioSink, WavSink};
use littlesdr::dab::DabReceiver;
use littlesdr::source::u8_to_iq;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).unwrap();
    let mut rx = DabReceiver::new();
    if let Some(s) = a.get(1) {
        rx.select(s);
    }
    let mut wav = a.get(2).map(|p| WavSink::create(p).unwrap());
    let mut out = Vec::new();
    let mut total = 0usize;
    for ch in raw.chunks(65536) {
        let mut s = Vec::new();
        u8_to_iq(ch, &mut s);
        out.clear();
        rx.process(&s, &mut out);
        total += out.len();
        if let Some(w) = &mut wav {
            w.write(&out);
        }
    }
    if let Some(w) = &mut wav {
        w.finish();
    }
    eprintln!("{}", rx.describe());
    eprintln!("{:.1} s of audio", total as f64 / 48_000.0);
    for s in rx.services() {
        println!("{:04X}  {}", s.sid, s.label);
    }
}
