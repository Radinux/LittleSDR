//! The 11D capture with added noise (sigma in u8 units): how much audio comes out, and the
//! DAB+ stats. Usage: dab_noisy <sigma>
use littlesdr::dab::DabReceiver;
use littlesdr::source::u8_to_iq;
fn main() {
    let sigma: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let raw = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/dab_11D_2048k_g40.u8")).unwrap();
    let mut seed = 12345u64;
    let mut gauss = || {
        let mut s = 0.0f32;
        for _ in 0..6 {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            s += (seed >> 40) as f32 / (1u64 << 24) as f32;
        }
        (s - 3.0) * 1.414
    };
    let noisy: Vec<u8> = raw.iter().map(|&b| (b as f32 + sigma * gauss()).round().clamp(0.0, 255.0) as u8).collect();
    let mut rx = DabReceiver::new();
    rx.select("radio classique");
    let mut out = Vec::new();
    for ch in noisy.chunks(65536) {
        let mut s = Vec::new();
        u8_to_iq(ch, &mut s);
        rx.process(&s, &mut out);
    }
    println!("sigma {sigma}: {:.1} s of audio | {}", out.len() as f64 / 48_000.0, rx.describe());
}
