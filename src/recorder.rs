//! Recorder: the sound (48 kHz, stereo or mono) or the baseband IQ to 16-bit WAV. The IQ
//! is the whole band the source delivers, or, to save space, a slice around the tuned
//! frequency (`bbMax`). The disk is written by a thread of its
//! own, so a slow card never holds the radio up; the header is kept valid as the file grows,
//! so a recording cut by a crash still opens.
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
    fir: DecimFir<C32>,
}

pub struct Recorder {
    pub mode: RecMode,
    pub stereo: bool,
    /// Widest baseband recorded (Hz); 0 = the whole band
    pub bb_max: f64,
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
    let mut off = 0i64;
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
            gmtoff: std::ffi::c_long,
            zone: *const std::ffi::c_char,
        }
        extern "C" {
            fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
        }
        let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
        // SAFETY: plain C call with valid pointers to stack values
        if !unsafe { localtime_r(&secs, &mut tm) }.is_null() {
            off = tm.gmtoff as i64;
        }
    }
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
            bb_max: 0.0,
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
        let d = if self.bb_max > 0.0 { (self.src_rate / self.bb_max).ceil().max(1.0) as usize } else { 1 };
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
            "bbMax" if !self.recording() => self.bb_max = v.max(0.0),
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
                let fir = DecimFir::new(lowpass(0.45 * rate / self.src_rate, (0.1 * rate / self.src_rate).max(0.002)), d);
                self.slice = Some(Slice { in_rate: self.src_rate, offset: off, nco: Nco::new(), fir });
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

    /// Wideband IQ (before the channel filter)
    pub fn feed_iq(&mut self, iq: &[C32], rate: f64, center: f64, vfo: f64) {
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
        let out = if s.fir.factor() == 1 && s.offset == 0.0 {
            iq.to_vec() // the whole band, as it comes
        } else {
            let mut buf = iq.to_vec();
            s.nco.set(s.offset, rate);
            s.nco.mix(&mut buf);
            let mut out = Vec::with_capacity(buf.len() / s.fir.factor() + 2);
            s.fir.process(&buf, &mut out);
            out
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
    /// "path","bbRate","bbCenter","bbMax","error"}; reading it resets the peak level
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
            "{{\"recording\":{},\"mode\":{},\"stereo\":{},\"file\":\"{}\",\"seconds\":{:.2},\"bytes\":{:.0},\"samplerate\":{:.0},\"level\":[{:.1},{:.1}],\"path\":\"{}\",\"bbRate\":{:.0},\"bbCenter\":{:.0},\"bbMax\":{:.0},\"error\":\"{}\"}}",
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
        r.feed_iq(&first, rate, center, vfo);
        r.start();
        assert!(r.recording(), "{}", r.error);
        let json = r.json();
        assert!(json.contains("\"bbRate\":480000") && json.contains("\"bbCenter\":7400000"), "{json}");
        for k in 1..60 {
            let b: Vec<C32> = (k * 8192..(k + 1) * 8192).map(tone).collect();
            r.feed_iq(&b, rate, center, vfo);
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
    fn baseband_defaults_to_the_whole_band() {
        let dir = tmpdir("full");
        let mut r = Recorder::new();
        r.set_path(dir.to_str().unwrap());
        r.set("mode", 0.0);
        let iq: Vec<C32> = (0..4096).map(|i| C32::new((i as f32 * 0.01).sin() * 0.5, (i as f32 * 0.01).cos() * 0.5)).collect();
        r.feed_iq(&iq, 1_920_000.0, 7_060_000.0, 7_074_000.0);
        r.start();
        r.feed_iq(&iq, 1_920_000.0, 7_060_000.0, 7_074_000.0);
        let file = r.file().to_string();
        assert!(file.contains("baseband_7060000Hz_1920kHz"), "{file}");
        r.stop();
        let d = std::fs::read(&file).unwrap();
        assert_eq!(d.len(), 44 + 4096 * 4);
        let x = i16::from_le_bytes([d[44 + 400], d[45 + 400]]) as f32 / 32767.0;
        assert!((x - iq[100].re).abs() < 1e-3, "samples passed unchanged");
        let _ = std::fs::remove_dir_all(dir);
    }
}
