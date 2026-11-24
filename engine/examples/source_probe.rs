//! Opens a receiver as the app would and reports what happens (driver missing, no device...)
//! Usage: source_probe "Airspy" | "Airspy HF+" | "SDRplay" | "RTL-SDR"
use littlesdr::service::sources::{open, SourceSettings};
use littlesdr::source::{self, SourceConfig};
fn main() {
    let kind = std::env::args().nth(1).unwrap_or("SDRplay".into());
    let s = SourceSettings::default();
    let (src, _h) = match open(&kind, &s, None) {
        Ok(x) => x,
        Err(e) => return println!("{kind}: ouverture refusée : {e}"),
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let cfg = SourceConfig { rate: s.rate(&kind) as u32, center: 100_000_000, gain: None, direct_sampling: 0, bias_tee: false };
    let h = source::spawn(src, cfg, tx);
    let mut n = 0usize;
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs() < 2 {
        if let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
            n += b.samples.len();
        }
    }
    let _ = h.ctl.send(source::SourceCtl::Stop);
    let _ = h.thread.join();
    let err = h.error.lock().unwrap().clone();
    println!("{kind}: {n} échantillons en 2 s{}", err.map(|e| format!(", arrêt : {e}")).unwrap_or_default());
}
