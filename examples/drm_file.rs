//! DRM from an IQ recording (WAV, e.g. a KiwiSDR capture): prints sync state and the FAC.
//!   cargo run --release --example drm_file -- capture.wav
use littlesdr::drm::DrmReceiver;
use littlesdr::source::wavfile::probe;
use littlesdr::dsp::C32;
fn main() {
    let path = std::env::args().nth(1).expect("capture.wav");
    let info = probe(&path, Some(0.0)).unwrap();
    let d = std::fs::read(&path).unwrap();
    let data = &d[info.data_offset as usize..(info.data_offset + info.data_len) as usize];
    assert_eq!(info.bits, 16);
    let iq: Vec<C32> = data.chunks_exact(4).map(|b| C32::new(i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0, i16::from_le_bytes([b[2], b[3]]) as f32 / 32768.0)).collect();
    eprintln!("{path}: {:.1} s at {} S/s", iq.len() as f64 / info.rate, info.rate);
    let mut rx = DrmReceiver::new(info.rate);
    let mut audio: Vec<f32> = Vec::new();
    let mut last = None;
    let mut last_sdc = None;
    let (mut msc_total, mut hdr_ok, mut dir_good) = (0, 0, 0);
    for (i, c) in iq.chunks(info.rate as usize / 10).enumerate() {
        rx.process(c);
        if rx.last_fac.as_ref().map(|f| (f.service_id, f.language, f.msc_mode)) != last {
            last = rx.last_fac.as_ref().map(|f| (f.service_id, f.language, f.msc_mode));
            eprintln!("{:6.1} s  FAC {:?}", i as f64 / 10.0, rx.last_fac);
        }
        if rx.sdc_info != last_sdc {
            last_sdc = rx.sdc_info.clone();
            if let Some(s) = &last_sdc {
                eprintln!("{:6.1} s  SDC streams {:?} prot {:?} labels {:?}", i as f64 / 10.0, s.streams, s.prot, s.labels);
                for a in &s.audio {
                    eprintln!("          audio: {} (service {}, stream {}, config {:02x?})", a.describe(), a.short_id, a.stream, a.config);
                }
            }
        }
        audio.append(&mut rx.pcm);
        for m in rx.msc_frames.drain(..) {
            // xHE-AAC audio super frame header: border count, reservoir, CRC-8
            let bits: Vec<u8> = (0..8).map(|k| (m[0] >> (7 - k)) & 1).collect();
            msc_total += 1;
            if littlesdr::drm::fac::crc(&bits, 8) == m[1] as u32 {
                hdr_ok += 1;
                let n = (m[0] >> 4) as usize;
                // Directory at the end: 12-bit index + 4-bit repeat of the count
                let dir_ok = (0..n).all(|j| m[m.len() - 2 * (j + 1) + 1] as usize & 15 == n);
                if dir_ok {
                    dir_good += 1;
                }
            }
        }
        if i % 50 == 49 {
            eprintln!("{:6.1} s  {}", (i + 1) as f64 / 10.0, rx.status());
        }
    }
    eprintln!("end: {}", rx.status());
    if rx.audio.rate > 0 && !audio.is_empty() {
        let out = std::env::args().nth(2).unwrap_or_else(|| "drm.wav".into());
        let ch = rx.audio.channels.max(1) as u16;
        let rate = rx.audio.rate;
        let data: Vec<u8> = audio.iter().flat_map(|&x| ((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&ch.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
        wav.extend_from_slice(&(ch * 2).to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(&out, wav).unwrap();
        let frames = audio.len() / ch as usize;
        eprintln!("{:.1} s of audio at {} Hz -> {out}", frames as f64 / rx.audio.rate as f64, rx.audio.rate);
    }
    eprintln!("MSC frames {msc_total}: audio super frame header CRC ok {hdr_ok}, directory consistent {dir_good}");
}
