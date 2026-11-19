//! KiwiSDR receiver over the network: 12 kHz of IQ (20.25 kHz on wideband servers) around
//! the tuned frequency, 0-30 MHz, anywhere in the world.
//!
//! Protocol (WebSocket "/kiwi/<ms>/SND"): a few "SET ..." text commands, then "SND" binary
//! packets: 3 bytes "SND", flags (0x08 = IQ), sequence (4), S-meter (2), GPS header (10), then
//! 512 IQ pairs in 16-bit big-endian. "MSG" packets carry the state (sample rate, too busy,
//! password needed ...).

use super::ws::{Message, WebSocket};
use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// IQ passband requested from the server (Hz each side)
const HALF_BAND: f64 = 6_000.0;

#[derive(Default, Debug, Clone)]
pub struct KiwiStatus {
    pub connected: bool,
    pub rate: f64,
    pub message: String,
    /// S-meter (dBm) from the server
    pub rssi: f32,
}

pub struct KiwiSdr {
    /// "host:port" (port 8073 if left out)
    pub site: String,
    pub password: String,
    pub status: Arc<Mutex<KiwiStatus>>,
}

impl KiwiSdr {
    pub fn new(site: &str) -> KiwiSdr {
        KiwiSdr { site: site.trim().trim_start_matches("http://").trim_end_matches('/').to_string(), password: String::new(), status: Arc::default() }
    }
}

fn tune_cmd(freq: f64) -> String {
    format!("SET mod=iq low_cut={:.0} high_cut={:.0} freq={:.3}", -HALF_BAND, HALF_BAND, freq / 1000.0)
}

/// "key=value key2=value2" of a MSG packet
fn msg_value<'a>(msg: &'a str, key: &str) -> Option<&'a str> {
    msg.split(' ').find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

impl Source for KiwiSdr {
    fn name(&self) -> String {
        format!("KiwiSDR {}", self.site)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let (host, port) = match self.site.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().map_err(|_| format!("bad port in {}", self.site))?),
            None => (self.site.clone(), 8073u16),
        };
        let st = self.status.clone();
        let set_msg = |m: &str| st.lock().unwrap().message = m.to_string();
        set_msg("connecting");
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let mut ws = WebSocket::connect(&host, port, &format!("/kiwi/{ms}/SND"), Duration::from_secs(8))?;
        let mut center = cfg.center as f64;
        for c in [
            format!("SET auth t=kiwi p={}", if self.password.is_empty() { "#" } else { &self.password }),
            "SET AR OK in=12000 out=48000".to_string(),
            "SERVER DE CLIENT LittleSDR SND".to_string(),
            "SET compression=0".to_string(),
            "SET agc=0 hang=0 thresh=-100 slope=6 decay=1000 manGain=50".to_string(),
            "SET squelch=0 max=0".to_string(),
            tune_cmd(center),
        ] {
            ws.send_text(&c)?;
        }
        st.lock().unwrap().connected = true;
        set_msg("connected, waiting for data");
        let mut rate = 12_000.0;
        let mut last_keepalive = Instant::now();
        let mut buf: Vec<C32> = Vec::with_capacity(2048);
        loop {
            loop {
                match ctl.try_recv() {
                    Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => {
                        st.lock().unwrap().connected = false;
                        return Ok(());
                    }
                    Ok(SourceCtl::Center(f)) => {
                        center = f as f64;
                        ws.send_text(&tune_cmd(center))?;
                    }
                    Ok(SourceCtl::Gain(g)) => {
                        // The server's manual gain (dB), or its AGC
                        let c = match g {
                            Some(db) => format!("SET agc=0 hang=0 thresh=-100 slope=6 decay=1000 manGain={:.0}", db.clamp(0.0, 120.0)),
                            None => "SET agc=1 hang=0 thresh=-100 slope=6 decay=1000 manGain=50".into(),
                        };
                        ws.send_text(&c)?;
                    }
                    Ok(_) => {} // the server fixes rate, bias-tee etc.
                    Err(TryRecvError::Empty) => break,
                }
            }
            if last_keepalive.elapsed() > Duration::from_secs(4) {
                ws.send_text("SET keepalive")?;
                last_keepalive = Instant::now();
            }
            let m = match ws.recv() {
                Ok(m) => m,
                Err(e) => {
                    st.lock().unwrap().connected = false;
                    set_msg(&format!("disconnected: {e}"));
                    return Err(e);
                }
            };
            let Message::Binary(b) = m else { continue };
            if b.len() < 3 {
                continue;
            }
            match &b[..3] {
                b"MSG" => {
                    let text = String::from_utf8_lossy(&b[4.min(b.len())..]).into_owned();
                    if let Some(r) = msg_value(&text, "sample_rate").and_then(|v| v.parse::<f64>().ok()) {
                        rate = r;
                        st.lock().unwrap().rate = r;
                    }
                    if msg_value(&text, "too_busy").is_some_and(|v| v != "0") {
                        set_msg("server full (too busy)");
                        return Err("KiwiSDR full: all its channels are in use".into());
                    }
                    if msg_value(&text, "badp").is_some_and(|v| v == "1") {
                        set_msg("password needed");
                        return Err("KiwiSDR needs a password".into());
                    }
                    if msg_value(&text, "down").is_some() {
                        set_msg("server down");
                        return Err("KiwiSDR is down".into());
                    }
                }
                b"SND" if b.len() >= 20 && b[3] & 0x08 != 0 => {
                    let smeter = u16::from_be_bytes([b[8], b[9]]);
                    st.lock().unwrap().rssi = 0.1 * smeter as f32 - 127.0;
                    for p in b[20..].chunks_exact(4) {
                        let i = i16::from_be_bytes([p[0], p[1]]) as f32 / 32768.0;
                        let q = i16::from_be_bytes([p[2], p[3]]) as f32 / 32768.0;
                        buf.push(C32::new(i, q));
                    }
                    if buf.len() >= 1024 {
                        if st.lock().unwrap().message != "receiving" {
                            set_msg("receiving");
                        }
                        let samples = std::mem::replace(&mut buf, Vec::with_capacity(2048));
                        if out.send(IqBlock { samples, rate, center, settle_ms: 0.0, time: None }).is_err() {
                            return Ok(());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
