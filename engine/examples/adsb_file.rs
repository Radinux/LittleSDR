//! ADS-B from a raw RTL-SDR capture (unsigned 8-bit IQ), e.g.
//! `rtl_sdr -f 1090M -s 2.4M -g 49.6 capture.u8`, then
//! `cargo run --release --example adsb_file capture.u8 [rate] [lat lon]`
use littlesdr::adsb::AdsbDecoder;
use littlesdr::dsp::C32;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("capture.u8 [rate] [lat lon]");
    let rate: f64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(2.4e6);
    let bytes = std::fs::read(path).expect("read");
    let mut d = AdsbDecoder::new(rate);
    d.keep_raw = true;
    if let (Some(a), Some(b)) = (args.get(3), args.get(4)) {
        d.home = Some((a.parse().unwrap(), b.parse().unwrap()));
    }
    let t = std::time::Instant::now();
    let block = 2 * 65536;
    for (k, c) in bytes.chunks(block).enumerate() {
        let iq: Vec<C32> = c.as_chunks::<2>().0.iter().map(|p| C32::new((p[0] as f32 - 127.5) / 127.5, (p[1] as f32 - 127.5) / 127.5)).collect();
        d.process(&iq, k as f64 * (block / 2) as f64 / rate);
    }
    let dur = bytes.len() as f64 / 2.0 / rate;
    let mut raw = d.take_raw();
    raw.sort();
    raw.dedup();
    for r in raw.iter().take(40) {
        println!("*{r};");
    }
    println!("{} distinct messages\n{}", raw.len(), d.table(dur));
    println!("by DF: {:?}", d.stats.by_df.iter().enumerate().filter(|x| *x.1 > 0).collect::<Vec<_>>());
    eprintln!("{:.1} s of signal decoded in {:.2} s", dur, t.elapsed().as_secs_f64());
}
