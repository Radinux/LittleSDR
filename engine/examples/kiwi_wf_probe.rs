//! KiwiSDR waterfall check: whole band, then a zoom on 40 m.
//!   cargo run --release --example kiwi_wf_probe -- [host:port]
use littlesdr::source::kiwisdr::KiwiWaterfall;
use std::time::Duration;
fn main() {
    let site = std::env::args().nth(1).unwrap_or("sk6ag1.ddns.net:8071".into());
    let wf = KiwiWaterfall::start(&site);
    for (label, view) in [("0-30 MHz", None), ("40 m", Some((7.1e6, 0.5e6)))] {
        if let Some((c, s)) = view {
            wf.set_view(c, s);
            std::thread::sleep(Duration::from_millis(1500));
            wf.take_rows();
        }
        std::thread::sleep(Duration::from_millis(1500));
        let rows = wf.take_rows();
        let Some(r) = rows.last() else {
            println!("no lines: {}", wf.status.lock().unwrap());
            return;
        };
        let mut v: Vec<(f32, usize)> = r.dbm.iter().copied().zip(0..).collect();
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        let floor = v[v.len() / 2].0;
        println!("{label}: {} lines in 1.5 s, {:.3}-{:.3} MHz, {} bins of {:.0} Hz, noise {:.0} dBm", rows.len(), r.start / 1e6, (r.start + r.bin_hz * r.dbm.len() as f64) / 1e6, r.dbm.len(), r.bin_hz, floor);
        let mut shown: Vec<usize> = Vec::new();
        for &(db, i) in &v {
            if shown.len() == 6 {
                break;
            }
            if shown.iter().any(|&j| j.abs_diff(i) < 8) {
                continue;
            }
            shown.push(i);
            println!("  {:>9.1} kHz  {:.0} dBm", (r.start + (i as f64 + 0.5) * r.bin_hz) / 1e3, db);
        }
    }
}
