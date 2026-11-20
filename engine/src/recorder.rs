//! Recorder: the sound (48 kHz, stereo or mono) or the baseband IQ to 16-bit WAV. The IQ
//! is, as chosen:
//! - "focus" (light): what the current mode's demodulator works on, around the tuned
//!   frequency (48 kHz for SSB/CW/AM/NFM, 240 kHz for broadcast FM, ...), enough to
//!   demodulate the recording again later;
//! - "full": the whole band the source delivers, to find other signals in it later;
//! - "custom": a slice of `bbMax` Hz around the tuned frequency.
//!
//! The disk is written by a thread of its own, so a slow card never holds the radio up; the
//! header is kept valid as the file grows, so a recording cut by a crash still opens.
//! State and settings match the app's recJson / recSet.

use crate::dsp::{lowpass, DecimFir, Frame, Nco, C32};
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

const AUDIO_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecMode {
    Baseband = 0,
    Audio = 1,
}

fn wav_header(rate: u32, channels: u16, data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    h[32..34].copy_from_slice(&(channels * 2).to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// WAV file written by a background thread
struct Writer {
    tx: Option<Sender<Vec<u8>>>,
    thread: Option<JoinHandle<()>>,
    bytes: Arc<AtomicU64>,
    rate: u32,
    channels: u16,
    frames: u64,
}

impl Writer {
    fn open(path: &PathBuf, rate: u32, channels: u16) -> std::io::Result<Writer> {
        let mut f = BufWriter::with_capacity(1 << 20, File::create(path)?);
        f.write_all(&wav_header(rate, channels, 0))?;
        let (tx, rx) = channel::<Vec<u8>>();
        let bytes = Arc::new(AtomicU64::new(44));
        let b2 = bytes.clone();
        let thread = std::thread::spawn(move || {
            let mut data: u64 = 0;
            let mut since_header: u64 = 0;
            let fix = |f: &mut BufWriter<File>, data: u64| -> std::io::Result<()> {
                f.flush()?;
                let file = f.get_mut();
                let d = data.min(u32::MAX as u64 - 36) as u32;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&wav_header(rate, channels, d))?;
                file.seek(SeekFrom::End(0))?;
                Ok(())
            };
            while let Ok(buf) = rx.recv() {
                if f.write_all(&buf).is_err() {
                    break;
                }
                data += buf.len() as u64;
                since_header += buf.len() as u64;
                b2.store(44 + data, Ordering::Relaxed);
                // Valid header about every second of data
                if since_header >= (rate as u64) * channels as u64 * 2 {
                    since_header = 0;
                    let _ = fix(&mut f, data);
                }
            }
            let _ = fix(&mut f, data);
        });
        Ok(Writer { tx: Some(tx), thread: Some(thread), bytes, rate, channels, frames: 0 })
    }

    fn write_i16(&mut self, samples: &[f32]) {
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            buf.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        self.frames += (samples.len() / self.channels as usize) as u64;
        if let Some(tx) = &self.tx {
            let _ = tx.send(buf);
        }
    }

    fn close(&mut self) {
        self.tx = None; // ends the thread's loop
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Narrowing of the IQ to the recorded slice
struct Slice {
    in_rate: f64,
    offset: f64,
    nco: Nco,
    /// Decimation in steps (a single 40x filter would be far too long for a phone)
    stages: Vec<DecimFir<C32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BbWidth {
    Full = 0,
    Focus = 1,
    Custom = 2,
}

/// Smallest integer >= d made of factors 2, 3, 5, 7 (cheap filter stages)
fn smooth_factor(d: usize) -> usize {
    (d.max(1)..)
        .find(|&k| {
            let mut k = k;
            for p in [2, 3, 5, 7] {
                while k.is_multiple_of(p) {
                    k /= p;
                }
            }
            k == 1
        })
        .unwrap()
}

/// Stages decimating by `d` while keeping +-0.45 x the final rate
fn stages(in_rate: f64, d: usize) -> Vec<DecimFir<C32>> {
    let mut f = Vec::new();
    let mut left = d;
    for p in [7, 5, 3, 2] {
        while left.is_multiple_of(p) && left > 1 {
            f.push(p);
            left /= p;
        }
    }
    // Biggest factors first: the long filter of the last stage runs at the lowest rate
    let fin = in_rate / d as f64;
    let mut r = in_rate;
    let n = f.len();
    f.into_iter()
        .enumerate()
        .map(|(i, p)| {
            let out = r / p as f64;
            let tw = if i + 1 == n { 0.1 * fin } else { out - 0.9 * fin };
            let st = DecimFir::new(lowpass((0.45 * fin + 0.5 * tw) / r, (tw / r).max(0.002)), p);
            r = out;
            st
        })
        .collect()
}

pub struct Recorder {
    pub mode: RecMode,
    pub stereo: bool,
    pub bb_width: BbWidth,
    /// Slice width of BbWidth::Custom (Hz)
    pub bb_max: f64,
    /// What the current mode demodulates (Hz), for BbWidth::Focus
    focus_rate: f64,
    pub path: PathBuf,
    writer: Option<Writer>,
    file: String,
    slice: Option<Slice>,
    center: f64,
    /// Source rate and tuning seen last (for the plan shown before recording)
    src_rate: f64,
    src_center: f64,
    vfo: f64,
    peak: [f32; 2],
    pub error: String,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

/// "2026-11-19_22-49-45" in local time (UTC if the C library can't tell)
fn local_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let off = crate::source::wavfile::local_offset();
    let t = secs + off;
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder {
            mode: RecMode::Audio,
            stereo: true,
            bb_width: BbWidth::Focus,
            bb_max: 500e3,
            focus_rate: 0.0,
            path: PathBuf::from("."),
            writer: None,
            file: String::new(),
            slice: None,
            center: 0.0,
            src_rate: 0.0,
            src_center: 0.0,
            vfo: 0.0,
            peak: [0.0; 2],
            error: String::new(),
        }
    }

    pub fn recording(&self) -> bool {
        self.writer.is_some()
    }

    /// Baseband if started now: (rate, centre, decimation)
    fn plan(&self) -> (f64, f64, usize) {
        if self.src_rate <= 0.0 {
            return (0.0, 0.0, 1);
        }
        let max = match self.bb_width {
            BbWidth::Full => 0.0,
            BbWidth::Focus => self.focus_rate,
            BbWidth::Custom => self.bb_max,
        };
        let d = if max > 0.0 && max < self.src_rate { smooth_factor((self.src_rate / max).ceil() as usize) } else { 1 };
        let rate = self.src_rate / d as f64;
        let max_off = ((self.src_rate - rate) / 2.0).max(0.0);
        let off = (self.vfo - self.src_center).clamp(-max_off, max_off);
        (rate, self.src_center + off, d)
    }

    /// App settings: "mode" (0 baseband / 1 audio), "stereo", "start", "stop"
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "start" => self.start(),
            "stop" => self.stop(),
            "mode" | "stereo" if self.recording() => {} // fixed for the file being written
            "mode" => self.mode = if v != 0.0 { RecMode::Audio } else { RecMode::Baseband },
            "stereo" => self.stereo = v != 0.0,
            "bbWidth" if !self.recording() => {
                self.bb_width = match v as i32 {
                    0 => BbWidth::Full,
                    2 => BbWidth::Custom,
                    _ => BbWidth::Focus,
                }
            }
            "bbMax" if !self.recording() => {
                self.bb_max = v.max(10e3);
                self.bb_width = BbWidth::Custom;
            }
            "status" => {}
            _ => return false,
        }
        true
    }

    pub fn set_path(&mut self, path: &str) {
        if !path.is_empty() && !self.recording() {
            self.path = PathBuf::from(path);
        }
    }

    pub fn start(&mut self) {
        if self.recording() {
            return;
        }
        self.error.clear();
        if let Err(e) = std::fs::create_dir_all(&self.path) {
            self.error = format!("{}: {e}", self.path.display());
            return;
        }
        let (name, rate, channels) = match self.mode {
            RecMode::Audio => (format!("{}_audio.wav", local_stamp()), AUDIO_RATE, if self.stereo { 2 } else { 1 }),
            RecMode::Baseband => {
                let (rate, center, d) = self.plan();
                if rate <= 0.0 {
                    self.error = "no signal yet".into();
                    return;
                }
                self.center = center;
                let off = center - self.src_center;
                self.slice = Some(Slice { in_rate: self.src_rate, offset: off, nco: Nco::new(), stages: stages(self.src_rate, d) });
                (format!("{}_baseband_{:.0}Hz_{:.0}kHz.wav", local_stamp(), center, rate / 1000.0), rate.round() as u32, 2)
            }
        };
        let full = self.path.join(&name);
        match Writer::open(&full, rate, channels) {
            Ok(w) => {
                self.writer = Some(w);
                self.file = full.display().to_string();
            }
            Err(e) => self.error = format!("{}: {e}", full.display()),
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut w) = self.writer.take() {
            w.close();
        }
        self.slice = None;
    }

    /// Wideband IQ (before the channel filter); `focus_rate` = what the current mode
    /// demodulates (its IF rate)
    pub fn feed_iq(&mut self, iq: &[C32], rate: f64, center: f64, vfo: f64, focus_rate: f64) {
        if !self.recording() {
            self.focus_rate = focus_rate;
        }
        self.src_rate = rate;
        self.src_center = center;
        self.vfo = vfo;
        if self.mode != RecMode::Baseband || self.writer.is_none() {
            return;
        }
        let s = self.slice.as_mut().unwrap();
        if s.in_rate != rate || (center + s.offset - self.center).abs() > 1.0 {
            // The source changed under the recording: the file would lie about its frequency
            self.stop();
            self.error = "stopped: sample rate or frequency changed".into();
            return;
        }
        let out = if s.stages.is_empty() && s.offset == 0.0 {
            iq.to_vec() // the whole band, as it comes
        } else {
            let mut buf = iq.to_vec();
            s.nco.set(s.offset, rate);
            s.nco.mix(&mut buf);
            for st in &mut s.stages {
                let mut out = Vec::with_capacity(buf.len() / st.factor() + 2);
                st.process(&buf, &mut out);
                buf = out;
            }
            buf
        };
        let mut flat = Vec::with_capacity(out.len() * 2);
        for z in &out {
            flat.push(z.re);
            flat.push(z.im);
            self.peak[0] = self.peak[0].max(z.re.abs());
            self.peak[1] = self.peak[1].max(z.im.abs());
        }
        self.writer.as_mut().unwrap().write_i16(&flat);
    }

    /// The sound (48 kHz), before the volume
    pub fn feed_audio(&mut self, audio: &[Frame]) {
        for f in audio {
            self.peak[0] = self.peak[0].max(f.l.abs());
            self.peak[1] = self.peak[1].max(f.r.abs());
        }
        if self.mode != RecMode::Audio {
            return;
        }
        let Some(w) = self.writer.as_mut() else { return };
        let flat: Vec<f32> = if w.channels == 2 { audio.iter().flat_map(|f| [f.l, f.r]).collect() } else { audio.iter().map(|f| 0.5 * (f.l + f.r)).collect() };
        w.write_i16(&flat);
    }

    /// recJson: {"recording","mode","stereo","file","seconds","bytes","samplerate","level":[l,r],
    /// "path","bbRate","bbCenter","bbWidth" (0 full, 1 focus, 2 custom),"bbMax","error"}; reading it resets the peak level
    pub fn json(&mut self) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let (bb_rate, bb_center, _) = self.plan();
        let (seconds, bytes, rate) = match &self.writer {
            Some(w) => (w.frames as f64 / w.rate as f64, w.bytes.load(Ordering::Relaxed) as f64, w.rate as f64),
            None => (0.0, 0.0, 0.0),
        };
        let db = |v: f32| if v > 1e-6 { 20.0 * v.log10() } else { -120.0 };
        let level = [db(self.peak[0]), db(self.peak[1])];
        self.peak = [0.0; 2];
        format!(
            "{{\"recording\":{},\"mode\":{},\"stereo\":{},\"file\":\"{}\",\"seconds\":{:.2},\"bytes\":{:.0},\"samplerate\":{:.0},\"level\":[{:.1},{:.1}],\"path\":\"{}\",\"bbRate\":{:.0},\"bbCenter\":{:.0},\"bbWidth\":{},\"bbMax\":{:.0},\"error\":\"{}\"}}",
            self.recording(),
            self.mode as i32,
            self.stereo,
            esc(if self.recording() { &self.file } else { "" }),
            seconds,
            bytes,
            rate,
            level[0],
            level[1],
            esc(&self.path.display().to_string()),
            bb_rate,
            if self.recording() && self.mode == RecMode::Baseband { self.center } else { bb_center },
            self.bb_width as i32,
            self.bb_max,
            esc(&self.error)
        )
    }

    pub fn file(&self) -> &str {
        &self.file
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("littlesdr-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn audio_wav_is_valid() {
        let dir = tmpdir("audio");
        let mut r = Recorder::new();
        r.set_path(dir.to_str().unwrap());
        r.set("stereo", 0.0);
        r.start();
        assert!(r.recording(), "{}", r.error);
        let frames: Vec<Frame> = (0..48_000).map(|i| { let v = (i as f32 * 0.05).sin() * 0.5; Frame { l: v, r: v } }).collect();
        for c in frames.chunks(4800) {
            r.feed_audio(c);
        }
        assert!(r.json().contains("\"seconds\":1.00"));
        let file = r.file().to_string();
        r.stop();
        let d = std::fs::read(&file).unwrap();
        assert_eq!(d.len(), 44 + 48_000 * 2);
        assert_eq!(u32::from_le_bytes(d[40..44].try_into().unwrap()), 96_000);
        assert_eq!(u16::from_le_bytes([d[22], d[23]]), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn baseband_slice_keeps_the_tuned_signal() {
        let dir = tmpdir("iq");
        let mut r = Recorder::new();
        r.set_path(dir.to_str().unwrap());
        r.set("mode", 0.0);
        r.set("bbMax", 500e3);
        let (rate, center, vfo) = (2_400_000.0, 7_100_000.0, 7_400_000.0);
        // A tone 10 kHz above the tuned frequency, and one far away that must be filtered out
        let tone = |n: usize| {
            let t = n as f64 / rate;
            let a = 2.0 * std::f64::consts::PI * (vfo + 10_000.0 - center) * t;
            let b = 2.0 * std::f64::consts::PI * (6_400_000.0 - center) * t;
            C32::new((0.3 * a.cos() + 0.3 * b.cos()) as f32, (0.3 * a.sin() + 0.3 * b.sin()) as f32)
        };
        let first: Vec<C32> = (0..8192).map(tone).collect();
        r.feed_iq(&first, rate, center, vfo, 48e3);
        r.start();
        assert!(r.recording(), "{}", r.error);
        let json = r.json();
        assert!(json.contains("\"bbRate\":480000") && json.contains("\"bbCenter\":7400000"), "{json}");
        for k in 1..60 {
            let b: Vec<C32> = (k * 8192..(k + 1) * 8192).map(tone).collect();
            r.feed_iq(&b, rate, center, vfo, 48e3);
        }
        let file = r.file().to_string();
        assert!(file.contains("baseband_7400000Hz_480kHz"), "{file}");
        r.stop();
        let d = std::fs::read(&file).unwrap();
        assert_eq!(u32::from_le_bytes(d[24..28].try_into().unwrap()), 480_000);
        // Power of the recorded slice: only the near tone (0.3 -> 0.09 power)
        let s: Vec<f32> = d[44 + 4000..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).collect();
        let p = s.iter().map(|x| x * x).sum::<f32>() / (s.len() / 2) as f32;
        assert!((p - 0.09).abs() < 0.01, "power {p}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn baseband_full_band() {
        let dir = tmpdir("full");
        let mut r = Recorder::new();
        r.set_path(dir.to_str().unwrap());
        r.set("mode", 0.0);
        r.set("bbWidth", 0.0);
        let iq: Vec<C32> = (0..4096).map(|i| C32::new((i as f32 * 0.01).sin() * 0.5, (i as f32 * 0.01).cos() * 0.5)).collect();
        r.feed_iq(&iq, 1_920_000.0, 7_060_000.0, 7_074_000.0, 48e3);
        r.start();
        r.feed_iq(&iq, 1_920_000.0, 7_060_000.0, 7_074_000.0, 48e3);
        let file = r.file().to_string();
        assert!(file.contains("baseband_7060000Hz_1920kHz"), "{file}");
        r.stop();
        let d = std::fs::read(&file).unwrap();
        assert_eq!(d.len(), 44 + 4096 * 4);
        let x = i16::from_le_bytes([d[44 + 400], d[45 + 400]]) as f32 / 32767.0;
        assert!((x - iq[100].re).abs() < 1e-3, "samples passed unchanged");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn focus_follows_the_mode() {
        let dir = tmpdir("focus");
        let mut r = Recorder::new();
        r.set_path(dir.to_str().unwrap());
        r.set("mode", 0.0);
        let (rate, center, vfo) = (2_048_000.0, 7_100_000.0, 7_074_000.0);
        let tone = |n: usize| {
            let a = 2.0 * std::f64::consts::PI * (vfo + 1_500.0 - center) * n as f64 / rate;
            C32::new(0.3 * a.cos() as f32, 0.3 * a.sin() as f32)
        };
        r.feed_iq(&(0..8192).map(tone).collect::<Vec<_>>(), rate, center, vfo, 48e3);
        r.start();
        for k in 1..100 {
            r.feed_iq(&(k * 8192..(k + 1) * 8192).map(tone).collect::<Vec<_>>(), rate, center, vfo, 48e3);
        }
        // 2.048 MS/s / 48 kHz = 42.7 -> 45 = 3 x 3 x 5 -> 45.5 kHz
        let file = r.file().to_string();
        assert!(file.contains("baseband_7074000Hz_46kHz"), "{file}");
        r.stop();
        let d = std::fs::read(&file).unwrap();
        let s: Vec<f32> = d[44 + 8000..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).collect();
        let p = s.iter().map(|x| x * x).sum::<f32>() / (s.len() / 2) as f32;
        assert!((p - 0.09).abs() < 0.01, "power {p}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
