//! Prints the channel impulse response (from the phase reference symbol) of a DAB capture.
use littlesdr::dab::ofdm::Ofdm;
use littlesdr::source::u8_to_iq;
fn main() {
    let raw = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let mut o = Ofdm::new();
    o.debug_cir = true;
    let mut frames = Vec::new();
    for ch in raw.chunks(65536).take(200) {
        let mut s = Vec::new();
        u8_to_iq(ch, &mut s);
        o.process(&s, &mut frames);
    }
}
