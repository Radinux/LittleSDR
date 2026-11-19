//! FT8 / FT4 decodes of a baseband IQ recording, e.g. SDR++'s
//! "baseband_7074000Hz_22-49-45_19-11-2026.wav" (local time in the name).
//!   cargo run --release --example ft8_file -- file.wav [--utc-offset 1] [--center 7074000] [--grid KN34]
//! Also reads the engine's own "2026-11-19_22-49-45_baseband_7060000Hz_1920kHz.wav".

use littlesdr::dsp::C32;
use littlesdr::ft8::Ft8Receiver;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = a.get(1).expect("usage: ft8_file file.wav [--utc-offset H] [--center HZ]");
    let opt = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy().to_string();
    let parts: Vec<&str> = name.trim_end_matches(".wav").split('_').collect();
    let center: f64 = opt("--center")
        .and_then(|v| v.parse().ok())
        .or_else(|| parts.iter().find(|p| p.ends_with("Hz")).and_then(|p| p.trim_end_matches("Hz").parse().ok()))
        .expect("center frequency: --center HZ");
    // hh-mm-ss_dd-mm-yyyy in local time
    let offset_h: f64 = opt("--utc-offset").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let t_start = {
        let nums = |s: &&str| s.split('-').filter_map(|x| x.parse::<i64>().ok()).collect::<Vec<_>>();
        // LittleSDR: yyyy-mm-dd_hh-mm-ss_baseband_...; SDR++: ..._hh-mm-ss_dd-mm-yyyy
        let (hms, dmy) = match parts.first().map(nums) {
            Some(ymd) if ymd.len() == 3 && ymd[0] > 1900 => (parts.get(1).map(nums), Some(vec![ymd[2], ymd[1], ymd[0]])),
            _ => (parts.iter().rev().nth(1).map(nums), parts.last().map(nums)),
        };
        match (hms, dmy) {
            (Some(h), Some(d)) if h.len() == 3 && d.len() == 3 => {
                (days_from_civil(d[2], d[1], d[0]) * 86_400 + h[0] * 3600 + h[1] * 60 + h[2]) as f64 - offset_h * 3600.0
            }
            _ => panic!("no hh-mm-ss_dd-mm-yyyy in the file name"),
        }
    };
    let d = std::fs::read(path).expect("cannot read the file");
    let (mut ch, mut rate, mut bits, mut fmt) = (2usize, 0.0, 16u16, 1u16);
    let mut i = 12;
    let mut data: &[u8] = &[];
    while i + 8 <= d.len() {
        let id = &d[i..i + 4];
        let len = u32::from_le_bytes(d[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &d[i + 8..(i + 8 + len).min(d.len())];
        if id == b"fmt " {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            ch = u16::from_le_bytes([body[2], body[3]]) as usize;
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap()) as f64;
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = body;
        }
        i += 8 + len + (len & 1);
    }
    assert_eq!(ch, 2, "baseband IQ recordings are stereo (I, Q)");
    let bps = (bits / 8) as usize;
    let sample = |b: &[u8]| -> f32 {
        match (fmt, bits) {
            (3, 32) => f32::from_le_bytes(b.try_into().unwrap()),
            (_, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (_, 32) => i32::from_le_bytes(b.try_into().unwrap()) as f32 / 2_147_483_648.0,
            (_, 8) => (b[0] as f32 - 128.0) / 128.0,
            _ => 0.0,
        }
    };
    let n = data.len() / (2 * bps);
    eprintln!("{name}: {:.1} s at {rate} S/s, center {:.3} MHz, starts {:.0} UTC s", n as f64 / rate, center / 1e6, t_start);
    let mut rx = Ft8Receiver::new();
    rx.set("enabled", 1.0);
    if let Some(g) = opt("--grid").and_then(|g| littlesdr::ft8::message::grid_latlon(&g)) {
        rx.home = Some(g);
    }
    let block = 16384;
    let mut buf = Vec::with_capacity(block);
    let mut total = 0;
    let print = |rx: &mut Ft8Receiver| {
        for r in rx.take_news() {
            let s = (r.time / 1000 - if r.mode == "FT4" { 7 } else { 15 }).rem_euclid(86_400);
            println!("{:02}{:02}{:02} {} {:>4.0} {:>5.1} {:>5.0}  {}", s / 3600, s % 3600 / 60, s % 60, r.mode, r.snr, r.dt, r.audio_hz, r.message);
        }
    };
    for f in data.chunks_exact(2 * bps) {
        buf.push(C32::new(sample(&f[..bps]), sample(&f[bps..])));
        if buf.len() == block {
            total += block;
            rx.feed_iq_at(&buf, rate, center, 7_074_000.0, true, t_start + total as f64 / rate);
            buf.clear();
            // Decoding runs in threads; keep at most one slot ahead
            while rx.busy() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            print(&mut rx);
        }
    }
    while rx.busy() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    print(&mut rx);
}
