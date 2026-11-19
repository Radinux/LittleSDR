//! Plays an IQ recording (WAV, float32 or int16) through the whole engine, as if it came
//! from a receiver: `engine_wav file.wav MODE CENTER_HZ TUNE_HZ [commands...]`.
//! Commands are those of the engine (`EngineCtl`) in a tiny subset: none needed for LRPT.
use littlesdr::dsp::{Frame, C32};
use littlesdr::engine::{Engine, EngineCtl, Mode};
use littlesdr::source::IqBlock;
use std::io::Read;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(&a[0]).expect("open"));
    let mode = Mode::parse(&a[1]).expect("mode");
    let center: f64 = a[2].parse().unwrap();
    let tune: f64 = a[3].parse().unwrap();
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr).unwrap();
    let (mut rate, mut bits) = (0u32, 0u16);
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
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
        }
    }
    let mut e = Engine::new(mode, tune);
    e.echo = true;
    let bpp = bits as usize / 8 * 2;
    let mut buf = vec![0u8; bpp * 16384];
    let mut out: Vec<Frame> = Vec::new();
    let (mut n_in, mut next) = (0usize, 0usize);
    let mut have = 0usize; // bytes in buf, a partial sample may stay for the next read
    loop {
        let n = f.read(&mut buf[have..]).unwrap();
        if n == 0 {
            break;
        }
        have += n;
        let n = have - have % bpp;
        let s: Vec<C32> = buf[..n]
            .chunks_exact(bpp)
            .map(|c| match bits {
                32 => C32::new(f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap())),
                _ => C32::new(i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0, i16::from_le_bytes([c[2], c[3]]) as f32 / 32768.0),
            })
            .collect();
        buf.copy_within(n..have, 0);
        have -= n;
        n_in += s.len();
        out.clear();
        e.process(IqBlock { samples: s, rate: rate as f64, center, settle_ms: 0.0 }, &mut out);
        if n_in >= next {
            next += rate as usize * 60;
            if let Some(st) = e.lrpt_status() {
                eprintln!("[{:4.0} s] {st}", n_in as f64 / rate as f64);
            }
        }
    }
    e.control(&EngineCtl::LrptSave(a.get(4).cloned()));
}
