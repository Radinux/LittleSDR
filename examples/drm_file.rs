//! DRM from an IQ recording (WAV, e.g. a KiwiSDR capture): prints sync state and the FAC.
//!   cargo run --release --example drm_file -- capture.wav
use littlesdr::drm::DrmReceiver;
use littlesdr::source::wavfile::probe;
use littlesdr::dsp::C32;
fn main() {
    let path = std::env::args().nth(1).expect("capture.wav");
    let info = probe(&path, Some(0.0)).unwrap();
    let d = std::fs::read(&path).unwrap();
    let data = &d[info.data_offset as usize..(info.data_offset + info.data_len) as usize];
    assert_eq!(info.bits, 16);
    let iq: Vec<C32> = data.chunks_exact(4).map(|b| C32::new(i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0, i16::from_le_bytes([b[2], b[3]]) as f32 / 32768.0)).collect();
    eprintln!("{path}: {:.1} s at {} S/s", iq.len() as f64 / info.rate, info.rate);
    let mut rx = DrmReceiver::new(info.rate);
    let mut last = None;
    for (i, c) in iq.chunks(info.rate as usize / 10).enumerate() {
        rx.process(c);
        if rx.last_fac != last {
            last = rx.last_fac.clone();
            eprintln!("{:6.1} s  FAC {:?}", i as f64 / 10.0, last);
        }
        if i % 50 == 49 {
            eprintln!("{:6.1} s  {}", (i + 1) as f64 / 10.0, rx.status());
        }
    }
    eprintln!("end: {}", rx.status());
}
