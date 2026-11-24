//! Plays a raw RTL-SDR capture (u8 IQ) through the whole engine into a WAV file:
//!   engine_u8 capture.u8 RATE CENTER_HZ MODE TUNE_HZ out.wav [dab service]
use littlesdr::audio::{AudioSink, WavSink};
use littlesdr::engine::{Engine, EngineCtl, Mode};
use littlesdr::source::{u8_to_iq, IqBlock};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).unwrap();
    let (rate, center): (f64, f64) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let mut e = Engine::new(Mode::parse(&a[3]).unwrap(), a[4].parse().unwrap());
    e.echo = true;
    if let Some(s) = a.get(6) {
        e.control(&EngineCtl::DabSelect(s.clone()));
    }
    let mut wav = WavSink::create(&a[5]).unwrap();
    let mut out = Vec::new();
    for ch in raw.chunks(16384) {
        let mut s = Vec::new();
        u8_to_iq(ch, &mut s);
        out.clear();
        e.process(IqBlock { samples: s, rate, center, settle_ms: 0.0, time: None }, &mut out);
        wav.write(&out);
    }
    wav.finish();
    eprintln!("{}", e.status().lines().filter(|l| l.contains("dab") || l.contains("adsb") || l.contains("engine") || l.contains("rds")).collect::<Vec<_>>().join("\n"));
}
