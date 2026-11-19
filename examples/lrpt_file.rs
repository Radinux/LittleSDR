//! Decodes a Meteor LRPT recording into pictures.
//!   cargo run --release --example lrpt_file -- pass.wav [oqpsk|qpsk] [out_prefix]
//! Input: WAV with complex float32 samples (SDR++ / SatDump baseband) or 16-bit IQ.
use littlesdr::dsp::C32;
use littlesdr::lrpt::LrptDecoder;
use std::io::Read;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let path = &a[0];
    let oqpsk = a.get(1).map_or(true, |m| m != "qpsk");
    let prefix = a.get(2).cloned().unwrap_or_else(|| path.trim_end_matches(".wav").to_string());
    let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).expect("open"));
    // WAV header: find "fmt " and "data"
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr).unwrap();
    assert_eq!(&hdr[..4], b"RIFF", "not a WAV file");
    let (mut rate, mut bits, mut format) = (0u32, 0u16, 0u16);
    loop {
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch).unwrap();
        let size = u32::from_le_bytes(ch[4..8].try_into().unwrap()) as usize;
        if &ch[..4] == b"data" {
            break;
        }
        let mut body = vec![0u8; size + (size & 1)];
        f.read_exact(&mut body).unwrap();
        if &ch[..4] == b"fmt " {
            format = u16::from_le_bytes(body[0..2].try_into().unwrap());
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
        }
    }
    eprintln!("{path}: {rate} S/s, {} IQ, {}", if format == 3 { "float32" } else { "int" }, if oqpsk { "OQPSK" } else { "QPSK" });
    let mut dec = LrptDecoder::new(rate as f64, oqpsk);
    let bytes_per = (bits as usize / 8) * 2;
    let mut buf = vec![0u8; bytes_per * 65536];
    let (mut done, mut next_report) = (0usize, 0usize);
    let t0 = std::time::Instant::now();
    let mut have = 0usize; // bytes in buf, a partial sample may stay for the next read
    loop {
        let n = f.read(&mut buf[have..]).unwrap();
        if n == 0 {
            break;
        }
        have += n;
        let n = have - have % bytes_per;
        let iq: Vec<C32> = buf[..n]
            .chunks_exact(bytes_per)
            .map(|c| match bits {
                32 => C32::new(f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap())),
                16 => C32::new(i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0, i16::from_le_bytes([c[2], c[3]]) as f32 / 32768.0),
                _ => C32::new((c[0] as f32 - 127.5) / 127.5, (c[1] as f32 - 127.5) / 127.5),
            })
            .collect();
        buf.copy_within(n..have, 0);
        have -= n;
        dec.process(&iq);
        done += iq.len();
        if done >= next_report {
            next_report += rate as usize * 30;
            eprintln!("[{:5.0} s] {}", done as f64 / rate as f64, dec.describe());
        }
    }
    eprintln!("decoded {:.0} s of signal in {:.1} s", done as f64 / rate as f64, t0.elapsed().as_secs_f64());
    eprintln!("{}", dec.describe());
    for f in dec.save(&prefix).expect("write images") {
        println!("{f}");
    }
}
