//! Feeds a raw capture through the whole engine with the noise blanker on.
use littlesdr::engine::{Engine, EngineCtl, Mode};
use littlesdr::source::{u8_to_iq, IqBlock};

fn main() {
    let path = std::env::args().nth(1).expect("capture.u8");
    let raw = std::fs::read(path).expect("read");
    let mut e = Engine::new(Mode::Wfm, 250e3);
    let mut out = Vec::new();
    for (i, chunk) in raw.chunks(16384).enumerate() {
        if i == 20 {
            e.control(&EngineCtl::Blanker(true, None));
        }
        let mut s = Vec::new();
        u8_to_iq(chunk, &mut s);
        e.process(IqBlock { samples: s, rate: 2.4e6, center: 96.25e6, settle_ms: 0.0 }, &mut out);
    }
    println!("{}", e.status().lines().next().unwrap());
}
