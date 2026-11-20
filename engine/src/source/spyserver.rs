//! SpyServer (Airspy's network server: Airspy R2 / Mini / HF+, RTL-SDR) as a source.
//!
//! Binary TCP protocol, little endian. The client says HELLO; the server answers with the
//! device (maximum rate, decimation stages, gain range) and a sync message (who controls
//! the tuner, current frequency). The client then sets the IQ format, decimation and
//! frequency and starts streaming. IQ messages carry the digital gain applied by the server
//! in the top 16 bits of their type. Optionally the server also sends its own spectrum
//! (FFT) of the device's whole band, for the waterfall.

use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PROTOCOL_VERSION: u32 = (2 << 24) | 1700;
const CMD_HELLO: u32 = 0;
const CMD_SET_SETTING: u32 = 2;

const SET_STREAMING_MODE: u32 = 0;
const SET_STREAMING_ENABLED: u32 = 1;
const SET_GAIN: u32 = 2;
const SET_IQ_FORMAT: u32 = 100;
const SET_IQ_FREQUENCY: u32 = 101;
const SET_IQ_DECIMATION: u32 = 102;
const SET_IQ_DIGITAL_GAIN: u32 = 103;
const SET_FFT_FORMAT: u32 = 200;
const SET_FFT_FREQUENCY: u32 = 201;
const SET_FFT_DECIMATION: u32 = 202;
const SET_FFT_DB_OFFSET: u32 = 203;
const SET_FFT_DB_RANGE: u32 = 204;
const SET_FFT_DISPLAY_PIXELS: u32 = 205;

const MODE_IQ: u32 = 1;
const MODE_FFT_IQ: u32 = 5;
const FORMAT_UINT8: u32 = 1;
const FORMAT_INT16: u32 = 2;

const MSG_DEVICE_INFO: u32 = 0;
const MSG_CLIENT_SYNC: u32 = 1;
const MSG_UINT8_IQ: u32 = 100;
const MSG_INT16_IQ: u32 = 101;
const MSG_FLOAT_IQ: u32 = 103;
const MSG_UINT8_FFT: u32 = 301;

/// FFT rows: pixels and dB range
pub const FFT_PIXELS: u32 = 1024;
const FFT_DB_RANGE: u32 = 127;

#[derive(Clone, Debug, Default)]
pub struct DeviceInfo {
    pub device: u32,
    pub serial: u32,
    pub max_rate: u32,
    pub max_bandwidth: u32,
    pub decim_stages: u32,
    pub gain_stages: u32,
    pub max_gain: u32,
    pub min_freq: u32,
    pub max_freq: u32,
    pub resolution: u32,
    pub min_decim: u32,
    pub forced_format: u32,
}

impl DeviceInfo {
    pub fn name(&self) -> &'static str {
        match self.device {
            1 => "Airspy R2/Mini",
            2 => "Airspy HF+",
            3 => "RTL-SDR",
            _ => "?",
        }
    }
    /// Sample rates the server can stream: (decimation stage, rate)
    pub fn rates(&self) -> Vec<(u32, f64)> {
        (self.min_decim..=self.decim_stages).map(|i| (i, self.max_rate as f64 / (1u64 << i) as f64)).collect()
    }
}

/// What the app shows: device, who controls it, and the server's spectrum rows
#[derive(Default)]
pub struct SpyStatus {
    pub connected: bool,
    pub device: Option<DeviceInfo>,
    pub can_control: bool,
    pub rate: f64,
    pub message: String,
    /// Spectrum rows (dB, low to high frequency) with the band they cover (centre, width Hz)
    pub fft: std::collections::VecDeque<(f64, f64, Vec<f32>)>,
}

pub struct SpyServer {
    pub host: String,
    pub port: u16,
    /// Ask the server for its spectrum too
    pub with_fft: bool,
    pub status: Arc<Mutex<SpyStatus>>,
}

impl SpyServer {
    /// "host[:port]" (5555 by default), also "sdr://host:port"
    pub fn new(addr: &str) -> SpyServer {
        let a = addr.trim().trim_start_matches("sdr://").trim_start_matches("tcp://").trim_end_matches('/');
        let (host, port) = match a.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h.to_string(), p.parse().unwrap()),
            _ => (a.to_string(), 5555),
        };
        SpyServer { host, port, with_fft: false, status: Arc::default() }
    }
}

fn send_cmd(s: &mut TcpStream, cmd: u32, body: &[u8]) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(8 + body.len());
    b.extend_from_slice(&cmd.to_le_bytes());
    b.extend_from_slice(&(body.len() as u32).to_le_bytes());
    b.extend_from_slice(body);
    s.write_all(&b)
}

fn set(s: &mut TcpStream, setting: u32, value: u32) -> std::io::Result<()> {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&setting.to_le_bytes());
    b[4..].copy_from_slice(&value.to_le_bytes());
    send_cmd(s, CMD_SET_SETTING, &b)
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())
}

/// Server-side digital gain that keeps the samples' level as the decimation removes bits
fn digital_gain(dev: &DeviceInfo, gain: u32, decim: u32) -> u32 {
    let d = (decim as f32 * 3.01) as u32;
    if dev.device == 1 {
        dev.max_gain.saturating_sub(gain) + d
    } else {
        d
    }
}

impl Source for SpyServer {
    fn name(&self) -> String {
        format!("SpyServer {}:{}", self.host, self.port)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let st = self.status.clone();
        let msg = |m: String| st.lock().unwrap().message = m;
        let addr = (self.host.as_str(), self.port).to_socket_addrs().map_err(|e| format!("{}: {e}", self.host))?.next().ok_or("no address")?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(8)).map_err(|e| format!("{}:{}: {e}", self.host, self.port))?;
        s.set_nodelay(true).ok();
        s.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let mut hello = PROTOCOL_VERSION.to_le_bytes().to_vec();
        hello.extend_from_slice(b"LittleSDR");
        send_cmd(&mut s, CMD_HELLO, &hello).map_err(|e| e.to_string())?;
        msg("connected, waiting for the device".into());

        let mut dev: Option<DeviceInfo> = None;
        let mut started = false;
        let mut center = cfg.center as f64;
        let mut want_rate = cfg.rate as f64;
        let mut gain_idx: Option<u32> = cfg.gain.map(|g| g.max(0.0) as u32);
        let mut decim = 0u32;
        let mut rate = 0.0;
        let mut settle = 0.0;
        let mut header = [0u8; 20];
        let mut have = 0usize;
        let mut body: Vec<u8> = Vec::new();
        let mut body_need = 0usize;
        loop {
            // Commands from the engine
            loop {
                match ctl.try_recv() {
                    Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => {
                        let _ = set(&mut s, SET_STREAMING_ENABLED, 0);
                        st.lock().unwrap().connected = false;
                        return Ok(());
                    }
                    Ok(SourceCtl::Center(f)) => {
                        center = f as f64;
                        if started {
                            set(&mut s, SET_IQ_FREQUENCY, f as u32).map_err(|e| e.to_string())?;
                            if self.with_fft {
                                set(&mut s, SET_FFT_FREQUENCY, f as u32).map_err(|e| e.to_string())?;
                            }
                        }
                    }
                    Ok(SourceCtl::Rate(r)) => {
                        want_rate = r as f64;
                        if let Some(d) = &dev {
                            let (nd, nr) = pick_rate(d, want_rate);
                            if nd != decim && started {
                                decim = nd;
                                rate = nr;
                                settle = 50.0;
                                set(&mut s, SET_IQ_DECIMATION, decim).map_err(|e| e.to_string())?;
                                set(&mut s, SET_IQ_DIGITAL_GAIN, digital_gain(d, gain_idx.unwrap_or(0), decim)).map_err(|e| e.to_string())?;
                            }
                        }
                    }
                    Ok(SourceCtl::Gain(g)) => {
                        gain_idx = g.map(|g| g.max(0.0) as u32);
                        if let (Some(d), Some(gi)) = (&dev, gain_idx) {
                            let gi = gi.min(d.max_gain);
                            set(&mut s, SET_GAIN, gi).map_err(|e| e.to_string())?;
                            set(&mut s, SET_IQ_DIGITAL_GAIN, digital_gain(d, gi, decim)).map_err(|e| e.to_string())?;
                        }
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                }
            }
            // Read: header, then body
            let blen = body.len();
            let r = if have < 20 { s.read(&mut header[have..]) } else { s.read(&mut body[blen - body_need..]) };
            match r {
                Ok(0) => {
                    st.lock().unwrap().connected = false;
                    msg("closed by the server".into());
                    return Err("SpyServer closed the connection".into());
                }
                Ok(n) => {
                    if have < 20 {
                        have += n;
                        if have == 20 {
                            let size = u32_at(&header, 4) as usize;
                            if size > 1 << 22 {
                                return Err("SpyServer: bad message size".into());
                            }
                            body = vec![0; size];
                            body_need = size;
                        }
                    } else {
                        body_need -= n;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    st.lock().unwrap().connected = false;
                    return Err(format!("SpyServer: {e}"));
                }
            }
            if have < 20 || body_need > 0 {
                continue;
            }
            have = 0;
            let mtype = u32_at(&header, 1);
            let (kind, flags) = (mtype & 0xFFFF, mtype >> 16);
            match kind {
                MSG_DEVICE_INFO if body.len() >= 48 => {
                    let d = DeviceInfo {
                        device: u32_at(&body, 0),
                        serial: u32_at(&body, 1),
                        max_rate: u32_at(&body, 2),
                        max_bandwidth: u32_at(&body, 3),
                        decim_stages: u32_at(&body, 4),
                        gain_stages: u32_at(&body, 5),
                        max_gain: u32_at(&body, 6),
                        min_freq: u32_at(&body, 7),
                        max_freq: u32_at(&body, 8),
                        resolution: u32_at(&body, 9),
                        min_decim: u32_at(&body, 10),
                        forced_format: u32_at(&body, 11),
                    };
                    st.lock().unwrap().device = Some(d.clone());
                    dev = Some(d);
                }
                MSG_CLIENT_SYNC if body.len() >= 36 => {
                    let can = u32_at(&body, 0) == 1;
                    let mut stt = st.lock().unwrap();
                    stt.can_control = can;
                    if !can {
                        // Someone else tunes this server: follow its frequency
                        center = u32_at(&body, 3) as f64;
                    }
                    drop(stt);
                    if !started {
                        let Some(d) = dev.clone() else { continue };
                        let (nd, nr) = pick_rate(&d, want_rate);
                        decim = nd;
                        rate = nr;
                        // 16-bit samples when the stream is narrow, else 8 bits (half the traffic)
                        let fmt = if d.forced_format != 0 { d.forced_format } else if rate <= 600e3 { FORMAT_INT16 } else { FORMAT_UINT8 };
                        let cmds = [
                            (SET_IQ_FORMAT, fmt),
                            (SET_IQ_DECIMATION, decim),
                            (SET_IQ_FREQUENCY, center as u32),
                            (SET_STREAMING_MODE, if self.with_fft { MODE_FFT_IQ } else { MODE_IQ }),
                        ];
                        for (k, v) in cmds {
                            set(&mut s, k, v).map_err(|e| e.to_string())?;
                        }
                        if self.with_fft {
                            for (k, v) in [(SET_FFT_FORMAT, FORMAT_UINT8), (SET_FFT_FREQUENCY, center as u32), (SET_FFT_DECIMATION, 0), (SET_FFT_DB_OFFSET, 0), (SET_FFT_DB_RANGE, FFT_DB_RANGE), (SET_FFT_DISPLAY_PIXELS, FFT_PIXELS)] {
                                set(&mut s, k, v).map_err(|e| e.to_string())?;
                            }
                        }
                        if can {
                            let gi = gain_idx.unwrap_or(d.max_gain / 2).min(d.max_gain);
                            set(&mut s, SET_GAIN, gi).map_err(|e| e.to_string())?;
                            set(&mut s, SET_IQ_DIGITAL_GAIN, digital_gain(&d, gi, decim)).map_err(|e| e.to_string())?;
                        }
                        set(&mut s, SET_STREAMING_ENABLED, 1).map_err(|e| e.to_string())?;
                        started = true;
                        let mut stt = st.lock().unwrap();
                        stt.connected = true;
                        stt.rate = rate;
                        stt.message = format!("{} {:.0} kS/s{}", d.name(), rate / 1e3, if can { "" } else { " (tuned by another listener)" });
                        eprintln!("[source] SpyServer {}:{}: {} {:.0} kS/s{}", self.host, self.port, d.name(), rate / 1e3, if can { "" } else { ", tuned by another listener" });
                    }
                }
                MSG_UINT8_IQ | MSG_INT16_IQ | MSG_FLOAT_IQ => {
                    let g = 10f32.powf(flags as f32 / 20.0);
                    let samples: Vec<C32> = match kind {
                        MSG_UINT8_IQ => body.chunks_exact(2).map(|p| C32::new((p[0] as f32 - 128.0) / (128.0 * g), (p[1] as f32 - 128.0) / (128.0 * g))).collect(),
                        MSG_INT16_IQ => body
                            .chunks_exact(4)
                            .map(|p| C32::new(i16::from_le_bytes([p[0], p[1]]) as f32 / (32768.0 * g), i16::from_le_bytes([p[2], p[3]]) as f32 / (32768.0 * g)))
                            .collect(),
                        _ => body
                            .chunks_exact(8)
                            .map(|p| C32::new(f32::from_le_bytes(p[..4].try_into().unwrap()) / g, f32::from_le_bytes(p[4..].try_into().unwrap()) / g))
                            .collect(),
                    };
                    if rate > 0.0 && !samples.is_empty() {
                        let block = IqBlock { samples, rate, center, settle_ms: std::mem::take(&mut settle), time: None };
                        if out.send(block).is_err() {
                            return Ok(());
                        }
                    }
                }
                MSG_UINT8_FFT if !body.is_empty() => {
                    // Bytes over 0..255 = the dB range below the offset
                    let db: Vec<f32> = body.iter().map(|&v| v as f32 * FFT_DB_RANGE as f32 / 255.0 - FFT_DB_RANGE as f32).collect();
                    let span = dev.as_ref().map_or(0.0, |d| d.max_rate as f64);
                    let mut stt = st.lock().unwrap();
                    stt.fft.push_back((center, span, db));
                    while stt.fft.len() > 100 {
                        stt.fft.pop_front();
                    }
                }
                _ => {}
            }
        }
    }
}

/// The streamable rate closest to (and preferably not below) the wanted one
fn pick_rate(d: &DeviceInfo, want: f64) -> (u32, f64) {
    let rates = d.rates();
    rates.iter().filter(|r| r.1 >= want * 0.99).min_by(|a, b| a.1.total_cmp(&b.1)).or_else(|| rates.iter().max_by(|a, b| a.1.total_cmp(&b.1))).copied().unwrap_or((0, d.max_rate as f64))
}
