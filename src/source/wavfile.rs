//! Baseband IQ recording (WAV) as a source: played back in real time, so it can be tuned,
//! demodulated and decoded like a live receiver.
//!
//! - Formats: PCM 8 (unsigned), 16, 24, 32-bit integer, 32-bit float; 2 channels (I, Q).
//! - Centre frequency: the "auxi" chunk (SDR#, HDSDR), else the file name: this engine's
//!   "2026-11-19_22-49-45_baseband_7074000Hz_46kHz.wav" or SDR++'s
//!   "baseband_7074000Hz_22-49-45_19-11-2026.wav".
//! - Recording time (from the name or auxi): carried with the blocks, so FT8 slots are cut at
//!   the right UTC seconds on playback.
//! - Controls: pause, seek, loop.

use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct WavInfo {
    pub rate: f64,
    pub center: f64,
    pub bits: u16,
    pub float: bool,
    pub data_offset: u64,
    pub data_len: u64,
    /// UTC time of the first sample (s since the epoch), if known
    pub start_utc: Option<f64>,
    /// IQ frames in the file
    pub frames: u64,
}

impl WavInfo {
    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.rate
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Offset of local time from UTC (s), as the C library sees it now
pub fn local_offset() -> i64 {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm {
            v: [i32; 9],
            gmtoff: std::ffi::c_long,
            zone: *const std::ffi::c_char,
        }
        extern "C" {
            fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
        let mut tm = Tm { v: [0; 9], gmtoff: 0, zone: std::ptr::null() };
        // SAFETY: plain C call with valid pointers to stack values
        if !unsafe { localtime_r(&secs, &mut tm) }.is_null() {
            return tm.gmtoff as i64;
        }
    }
    0
}

/// Centre frequency and local start time from the file name
fn from_name(name: &str) -> (Option<f64>, Option<f64>) {
    let stem = name.trim_end_matches(".wav").trim_end_matches(".WAV");
    let parts: Vec<&str> = stem.split('_').collect();
    let center = parts.iter().find(|p| p.ends_with("Hz") && !p.ends_with("kHz")).and_then(|p| p.trim_end_matches("Hz").parse().ok());
    let nums = |s: &str| s.split('-').filter_map(|x| x.parse::<i64>().ok()).collect::<Vec<_>>();
    let mut local = None;
    for w in parts.windows(2) {
        let (a, b) = (nums(w[0]), nums(w[1]));
        if a.len() == 3 && b.len() == 3 {
            local = if a[0] > 1900 {
                // yyyy-mm-dd_hh-mm-ss
                Some(days_from_civil(a[0], a[1], a[2]) * 86_400 + b[0] * 3600 + b[1] * 60 + b[2])
            } else if b[2] > 1900 {
                // hh-mm-ss_dd-mm-yyyy
                Some(days_from_civil(b[2], b[1], b[0]) * 86_400 + a[0] * 3600 + a[1] * 60 + a[2])
            } else {
                None
            };
            if local.is_some() {
                break;
            }
        }
    }
    (center, local.map(|t| (t - local_offset()) as f64))
}

/// `center`: the centre frequency when the file does not say it (or says it wrong)
pub fn probe(path: &str, center: Option<f64>) -> Result<WavInfo, String> {
    let mut f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut ch, mut rate, mut bits, mut fmt) = (0u16, 0u32, 0u16, 0u16);
    let (mut data_offset, mut data_len) = (0u64, 0u64);
    let (mut auxi_center, mut auxi_time) = (None, None);
    let mut pos = 12u64;
    loop {
        let mut h = [0u8; 8];
        if f.read_exact(&mut h).is_err() {
            break;
        }
        let len = u32::from_le_bytes(h[4..8].try_into().unwrap()) as u64;
        let body_at = pos + 8;
        match &h[0..4] {
            b"fmt " => {
                let mut b = vec![0u8; len.min(64) as usize];
                f.read_exact(&mut b).map_err(|e| e.to_string())?;
                fmt = u16::from_le_bytes([b[0], b[1]]);
                ch = u16::from_le_bytes([b[2], b[3]]);
                rate = u32::from_le_bytes(b[4..8].try_into().unwrap());
                bits = u16::from_le_bytes([b[14], b[15]]);
                if fmt == 0xFFFE && b.len() >= 26 {
                    fmt = u16::from_le_bytes([b[24], b[25]]); // WAVE_FORMAT_EXTENSIBLE sub-format
                }
            }
            b"auxi" => {
                // SDR# / HDSDR: SYSTEMTIME start, SYSTEMTIME stop, centre frequency (u32)
                let mut b = vec![0u8; len.min(64) as usize];
                f.read_exact(&mut b).map_err(|e| e.to_string())?;
                if b.len() >= 36 {
                    let w = |i: usize| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]) as i64;
                    let (y, mo, d, hh, mi, s) = (w(0), w(1), w(3), w(4), w(5), w(6));
                    if y > 1900 {
                        auxi_time = Some((days_from_civil(y, mo, d) * 86_400 + hh * 3600 + mi * 60 + s - local_offset()) as f64);
                    }
                    let c = u32::from_le_bytes(b[32..36].try_into().unwrap());
                    if c > 0 {
                        auxi_center = Some(c as f64);
                    }
                }
            }
            b"data" => {
                data_offset = body_at;
                // Recordings cut short leave 0 or a too-large size: use what the file holds
                let file_len = f.metadata().map_err(|e| e.to_string())?.len();
                data_len = if len == 0 || body_at + len > file_len { file_len - body_at } else { len };
                break;
            }
            _ => {}
        }
        pos = body_at + len + (len & 1);
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
    }
    if data_offset == 0 {
        return Err("no data in the file".into());
    }
    if ch != 2 {
        return Err(format!("{ch} channel(s): not an IQ recording (needs 2: I and Q)"));
    }
    let float = fmt == 3;
    if !(fmt == 1 || float) || !(matches!(bits, 8 | 16 | 24 | 32)) || (float && bits != 32) {
        return Err(format!("unsupported sample format ({fmt}, {bits} bits)"));
    }
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (name_center, name_time) = from_name(&name);
    let center = center.or(auxi_center).or(name_center).ok_or("centre frequency unknown (not in the name nor an auxi chunk): give it")?;
    let frames = data_len / (2 * bits as u64 / 8);
    Ok(WavInfo { rate: rate as f64, center, bits, float, data_offset, data_len, start_utc: auxi_time.or(name_time), frames })
}

/// Playback position and controls shared with the app
#[derive(Default)]
pub struct WavState {
    /// Frames played
    pub pos: AtomicU64,
    pub paused: AtomicBool,
    pub looping: AtomicBool,
    pub ended: AtomicBool,
}

pub struct WavFile {
    pub path: String,
    pub info: WavInfo,
    pub state: Arc<WavState>,
}

impl WavFile {
    pub fn open(path: &str, center: Option<f64>) -> Result<WavFile, String> {
        let info = probe(path, center)?;
        Ok(WavFile { path: path.to_string(), info, state: Arc::default() })
    }
}

fn convert(info: &WavInfo, raw: &[u8], out: &mut Vec<C32>) {
    let b = info.bits as usize / 8;
    let s = |x: &[u8]| -> f32 {
        match (info.float, b) {
            (true, _) => f32::from_le_bytes(x.try_into().unwrap()),
            (false, 1) => (x[0] as f32 - 127.5) / 127.5,
            (false, 2) => i16::from_le_bytes([x[0], x[1]]) as f32 / 32768.0,
            (false, 3) => (i32::from_le_bytes([0, x[0], x[1], x[2]]) >> 8) as f32 / 8_388_608.0,
            _ => i32::from_le_bytes(x.try_into().unwrap()) as f32 / 2_147_483_648.0,
        }
    };
    out.extend(raw.chunks_exact(2 * b).map(|f| C32::new(s(&f[..b]), s(&f[b..]))));
}

/// Length of the blocks sent (ms of signal)
pub const BLOCK_MS: f64 = 10.0;

impl Source for WavFile {
    fn name(&self) -> String {
        format!("file {}", self.path)
    }

    fn run(self: Box<Self>, _cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let info = self.info.clone();
        let st = self.state.clone();
        let mut f = File::open(&self.path).map_err(|e| e.to_string())?;
        let frame_bytes = 2 * info.bits as u64 / 8;
        let block = ((info.rate * BLOCK_MS / 1000.0) as u64).max(256);
        let mut raw = vec![0u8; (block * frame_bytes) as usize];
        let mut pos: u64 = 0;
        let mut t0 = Instant::now();
        let mut sent_since: u64 = 0; // frames since t0
        f.seek(SeekFrom::Start(info.data_offset)).map_err(|e| e.to_string())?;
        loop {
            // Controls
            loop {
                match ctl.try_recv() {
                    Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Ok(SourceCtl::Seek(sec)) => {
                        pos = ((sec.max(0.0) * info.rate) as u64).min(info.frames);
                        f.seek(SeekFrom::Start(info.data_offset + pos * frame_bytes)).map_err(|e| e.to_string())?;
                        st.ended.store(false, Ordering::Relaxed);
                        t0 = Instant::now();
                        sent_since = 0;
                    }
                    Ok(SourceCtl::Pause(p)) => {
                        st.paused.store(p, Ordering::Relaxed);
                        t0 = Instant::now();
                        sent_since = 0;
                    }
                    Ok(SourceCtl::Loop(l)) => st.looping.store(l, Ordering::Relaxed),
                    Ok(_) => {} // a file has no tuner: centre, rate and gain are what was recorded
                    Err(TryRecvError::Empty) => break,
                }
            }
            if st.paused.load(Ordering::Relaxed) || st.ended.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
                t0 = Instant::now();
                sent_since = 0;
                continue;
            }
            // Real time: wait until this block is due
            let due = t0 + Duration::from_secs_f64(sent_since as f64 / info.rate);
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
            let n = block.min(info.frames - pos);
            if n == 0 {
                if st.looping.load(Ordering::Relaxed) {
                    pos = 0;
                    f.seek(SeekFrom::Start(info.data_offset)).map_err(|e| e.to_string())?;
                    continue;
                }
                st.ended.store(true, Ordering::Relaxed);
                continue;
            }
            let bytes = (n * frame_bytes) as usize;
            f.read_exact(&mut raw[..bytes]).map_err(|e| e.to_string())?;
            let mut samples = Vec::with_capacity(n as usize);
            convert(&info, &raw[..bytes], &mut samples);
            let time = info.start_utc.map(|t| t + pos as f64 / info.rate);
            pos += n;
            sent_since += n;
            st.pos.store(pos, Ordering::Relaxed);
            if out.send(IqBlock { samples, rate: info.rate, center: info.center, settle_ms: 0.0, time }).is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_give_centre_and_time() {
        let off = local_offset() as f64;
        let (c, t) = from_name("2026-11-19_22-49-45_baseband_7074000Hz_46kHz.wav");
        assert_eq!(c, Some(7_074_000.0));
        assert_eq!(t, Some(1_795_128_585.0 - off));
        let (c, t) = from_name("baseband_7060000Hz_22-49-45_19-11-2026.wav");
        assert_eq!(c, Some(7_060_000.0));
        assert_eq!(t, Some(1_795_128_585.0 - off));
    }

    #[test]
    fn plays_a_recording_back() {
        // Write a short IQ file with the recorder's format, then play it
        let dir = std::env::temp_dir().join(format!("littlesdr-wavsrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-11-19_22-49-45_baseband_7074000Hz_48kHz.wav");
        let n = 4800u32;
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + n * 4).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&48_000u32.to_le_bytes());
        d.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        d.extend_from_slice(&4u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&(n * 4).to_le_bytes());
        for i in 0..n {
            d.extend_from_slice(&(i as i16).to_le_bytes());
            d.extend_from_slice(&(-(i as i16)).to_le_bytes());
        }
        std::fs::write(&path, d).unwrap();
        let w = WavFile::open(path.to_str().unwrap(), None).unwrap();
        assert_eq!((w.info.rate, w.info.center, w.info.frames), (48_000.0, 7_074_000.0, 4800));
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let cfg = SourceConfig { rate: 0, center: 0, gain: None, direct_sampling: 0, bias_tee: false };
        let h = super::super::spawn(Box::new(w), cfg, tx);
        let mut got = Vec::new();
        let mut first_time = None;
        while got.len() < 4800 {
            let b = rx.recv_timeout(Duration::from_secs(2)).expect("blocks");
            first_time.get_or_insert(b.time);
            assert_eq!(b.center, 7_074_000.0);
            got.extend(b.samples);
        }
        let _ = h.ctl.send(SourceCtl::Stop);
        assert!((got[100].re - 100.0 / 32768.0).abs() < 1e-6 && (got[100].im + 100.0 / 32768.0).abs() < 1e-6);
        assert!(first_time.flatten().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
