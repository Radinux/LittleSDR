//! Decodes a DAB+ service from a raw RTL-SDR capture (u8 IQ, 2.048 MS/s) into a WAV file.
//!   dab_file capture.u8 [service] [out.wav]   (no service: lists the ensemble)
use littlesdr::audio::{AudioSink, WavSink};
use littlesdr::dab::DabReceiver;
use littlesdr::source::u8_to_iq;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).unwrap();
    let mut rx = DabReceiver::new();
    rx.ofdm.coherent = std::env::var_os("DAB_COHERENT").is_some();
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
        if let Some(l) = rx.take_label() {
            println!("[{:5.1} s] text: {l}", total as f64 / 48_000.0);
        }
        if let Some(sl) = rx.take_slide() {
            let ext = if sl.mime == "image/png" { "png" } else { "jpg" };
            let path = format!("{}_slide{}.{ext}", a.get(2).map_or("dab", |p| p.trim_end_matches(".wav")), total);
            std::fs::write(&path, &sl.data).unwrap();
            println!("[{:5.1} s] slide: {} ({}, {} bytes) -> {path}", total as f64 / 48_000.0, sl.name, sl.mime, sl.data.len());
        }
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
