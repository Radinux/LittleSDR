//! Decodes RTTY from a raw RTL-SDR capture (u8 IQ) through the whole engine.
//!   cargo run --release --example rtty_file -- capture.u8 RATE CENTER DIAL usb|lsb [key=value ...]
//! keys: shift, baud, center, rev, ita2, sql (as the `rtty` command)
use littlesdr::engine::{Engine, EngineCtl, Mode};
use littlesdr::source::{u8_to_iq, IqBlock};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).expect("read");
    let (rate, center, dial): (f64, f64, f64) = (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap());
    let mut e = Engine::new(Mode::parse(&a[4]).expect("mode"), dial);
    for kv in &a[5..] {
        let (k, v) = kv.split_once('=').expect("key=value");
        e.control(&EngineCtl::RttyParam(k.into(), v.parse().expect("number")));
    }
    e.control(&EngineCtl::Rtty(true));
    let mut out = Vec::new();
    let mut text = String::new();
    for chunk in raw.chunks(16384) {
        let mut s = Vec::new();
        u8_to_iq(chunk, &mut s);
        e.process(IqBlock { samples: s, rate, center, settle_ms: 0.0, time: None }, &mut out);
        out.clear();
        text += &e.take_text();
    }
    println!("{text}");
    eprintln!("{}", e.rtty.as_ref().unwrap().describe());
}
