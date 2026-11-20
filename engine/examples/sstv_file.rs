//! Decodes the SSTV pictures of a WAV recording (PCM 16-bit or float, mono or stereo).
//!   cargo run --release --example sstv_file -- in.wav [out-prefix]

use littlesdr::sstv::Decoder;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = a.get(1).expect("usage: sstv_file in.wav [out-prefix]");
    let prefix = a.get(2).cloned().unwrap_or_else(|| "sstv".into());
    let d = std::fs::read(path).expect("cannot read the file");
    let (mut ch, mut rate, mut bits, mut fmt) = (1usize, 48_000.0, 16u16, 1u16);
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
    let bps = (bits / 8) as usize;
    let mono: Vec<f32> = data
        .chunks_exact(bps * ch)
        .map(|f| {
            let s = |k: usize| -> f32 {
                let b = &f[k * bps..(k + 1) * bps];
                match (fmt, bits) {
                    (3, 32) => f32::from_le_bytes(b.try_into().unwrap()),
                    (_, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                    (_, 8) => (b[0] as f32 - 128.0) / 128.0,
                    _ => 0.0,
                }
            };
            (0..ch).map(s).sum::<f32>() / ch as f32
        })
        .collect();
    eprintln!("{path}: {:.1} s at {rate} Hz, {ch} channel(s)", mono.len() as f64 / rate);
    let mut dec = Decoder::new();
    let (mut finished, mut version) = (0, 0);
    for c in mono.chunks((rate / 10.0) as usize) {
        dec.feed(c, rate);
        let st = dec.status();
        if st.receiving && st.version != version && st.line == 0 {
            eprintln!("receiving {} ({}x{})", st.mode, st.width, st.height);
        }
        version = st.version;
        if st.finished != finished {
            finished = st.finished;
            let out = format!("{prefix}-{finished}.png");
            std::fs::write(&out, dec.png(1).unwrap()).unwrap();
            eprintln!(
                "{}: {} rows{}, slant {:+.0} ppm -> {out}",
                st.finished_mode,
                st.finished_rows,
                if st.last_complete { " (complete)" } else { "" },
                st.slant_ppm
            );
        }
    }
    dec.finish_now();
    let st = dec.status();
    if st.finished != finished {
        let out = format!("{prefix}-{}.png", st.finished);
        std::fs::write(&out, dec.png(1).unwrap()).unwrap();
        eprintln!("{}: {} rows at the end of the file -> {out}", st.finished_mode, st.finished_rows);
    }
}
