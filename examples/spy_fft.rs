//! SpyServer spectrum check: connects with the server FFT on and prints the strongest bins.
//!   cargo run --release --example spy_fft -- host:port freq_hz
use littlesdr::source::spyserver::SpyServer;
use littlesdr::source::{spawn, SourceConfig};
use std::time::Duration;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut s = SpyServer::new(a.get(1).map_or("62.2.184.6:5556", |v| v.as_str()));
    s.with_fft = true;
    let st = s.status.clone();
    let freq: u64 = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(7_100_000);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let h = spawn(Box::new(s), SourceConfig { rate: 96_000, center: freq, gain: None, direct_sampling: 0, bias_tee: false }, tx);
    std::thread::spawn(move || while rx.recv().is_ok() {});
    std::thread::sleep(Duration::from_secs(6));
    let rows: Vec<_> = st.lock().unwrap().fft.drain(..).collect();
    let _ = h.ctl.send(littlesdr::source::SourceCtl::Stop);
    let Some((c, span, db)) = rows.last() else {
        println!("no spectrum: {}", st.lock().unwrap().message);
        return;
    };
    let mut v: Vec<(f32, usize)> = db.iter().copied().zip(0..).collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("{} rows in 6 s, {} bins over {:.0} kHz around {:.3} MHz, median {:.0} dB", rows.len(), db.len(), span / 1e3, c / 1e6, v[v.len() / 2].0);
    for &(d, i) in v.iter().take(6) {
        println!("  {:>9.1} kHz  {d:.0} dB", (c - span / 2.0 + (i as f64 + 0.5) * span / db.len() as f64) / 1e3);
    }
}
