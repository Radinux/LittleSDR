//! KiwiSDR receiver over the network: 12 kHz of IQ (20.25 kHz on wideband servers) around
//! the tuned frequency, 0-30 MHz, anywhere in the world.
//!
//! Protocol (WebSocket "/kiwi/<ms>/SND"): a few "SET ..." text commands, then "SND" binary
//! packets: 3 bytes "SND", flags (0x08 = IQ), sequence (4), S-meter (2), GPS header (10), then
//! 512 IQ pairs in 16-bit big-endian. "MSG" packets carry the state (sample rate, too busy,
//! password needed ...).
//!
//! Some servers accept only their own web page: their admin allows no channel to other
//! applications ("ext_api_nchans" = 0), and the server sends "too_busy=0" then closes about
//! 10 s in. That is the admin's choice; such servers are reported as refusing applications.

use super::ws::{Message, WebSocket};
use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Start of the error when a KiwiSDR turns applications away (the app remembers those servers)
pub const KIWI_REFUSED: &str = "Pas de place pour les applications";

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
        set_msg("connexion…");
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let mut ws = WebSocket::connect(&host, port, &format!("/kiwi/{ms}/SND"), Duration::from_secs(8))?;
        let mut center = cfg.center as f64;
        for c in [
            format!("SET auth t=kiwi p={}", if self.password.is_empty() { "#" } else { &self.password }),
            "SET AR OK in=12000 out=48000".to_string(),
            "SERVER DE CLIENT LittleSDR SND".to_string(),
            "SET compression=0".to_string(),
            // The server's AGC, as its web page does (a fixed gain overloads on strong stations)
            "SET agc=1 hang=0 thresh=-100 slope=6 decay=1000 manGain=50".to_string(),
            "SET squelch=0 max=0".to_string(),
            tune_cmd(center),
        ] {
            ws.send_text(&c)?;
        }
        st.lock().unwrap().connected = true;
        set_msg("connecté, en attente des données");
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
                    set_msg(&format!("déconnecté : {e}"));
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
                    // "too_busy=N": N = channels its admin allows to applications (not to its web
                    // page), all taken. 0 = its web page only. Often N slots held for good by the
                    // owner's own decoders (WSPR, FT8 skimmers): the server lets anyone in for
                    // ~10 s, then cuts. The admin's choice: reported, not worked around.
                    match msg_value(&text, "too_busy") {
                        Some("0") => {
                            set_msg("refuse les applications");
                            return Err(format!("{KIWI_REFUSED} : ce KiwiSDR n'accepte que sa propre page web. Choisis-en un autre.").into());
                        }
                        Some(n) => {
                            set_msg("places pour applications prises");
                            return Err(format!(
                                "{KIWI_REFUSED} : ce KiwiSDR ne laisse que {n} connexion(s) aux applications et elles sont prises, souvent par ses propres décodeurs automatiques. Choisis-en un autre."
                            )
                            .into());
                        }
                        None => {}
                    }
                    if msg_value(&text, "badp").is_some_and(|v| v == "1") {
                        set_msg("mot de passe requis");
                        return Err("Ce KiwiSDR demande un mot de passe".into());
                    }
                    if msg_value(&text, "down").is_some() {
                        set_msg("serveur arrêté");
                        return Err("Ce KiwiSDR est arrêté".into());
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
                            set_msg("réception");
                            eprintln!("[source] KiwiSDR {}: receiving, {:.3} kS/s around {:.3} kHz", self.site, rate / 1e3, center / 1e3);
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

/// One waterfall line from the server: power (dBm) of `bins.len()` bins from `start` Hz,
/// each `bin_hz` wide
#[derive(Clone, Debug)]
pub struct WfRow {
    pub start: f64,
    pub bin_hz: f64,
    pub dbm: Vec<f32>,
}

/// The server's own waterfall: the whole 0-30 MHz band or any zoom on it, independent of the
/// 12 kHz that are listened to (SDR++ only shows those 12 kHz)
pub struct KiwiWaterfall {
    rows: Arc<Mutex<std::collections::VecDeque<WfRow>>>,
    view: Arc<Mutex<Option<(f64, f64)>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    pub status: Arc<Mutex<String>>,
}

pub const KIWI_SPAN: f64 = 30e6;
const WF_BINS: f64 = 1024.0;
const ZOOM_MAX: u32 = 14;

/// Zoom level for a wanted span (each level halves the 30 MHz)
pub fn zoom_for(span: f64) -> u32 {
    let mut z = 0;
    while z < ZOOM_MAX && KIWI_SPAN / 2f64.powi(z as i32 + 1) >= span * 0.999 {
        z += 1;
    }
    z
}

impl KiwiWaterfall {
    /// Starts the waterfall connection (a second WebSocket, next to the sound)
    pub fn start(site: &str) -> KiwiWaterfall {
        let site = site.trim().trim_start_matches("http://").trim_end_matches('/').to_string();
        let w = KiwiWaterfall { rows: Arc::default(), view: Arc::new(Mutex::new(Some((15e6, 30e6)))), stop: Arc::default(), status: Arc::default() };
        let (rows, view, stop, status) = (w.rows.clone(), w.view.clone(), w.stop.clone(), w.status.clone());
        std::thread::Builder::new()
            .name("kiwi waterfall".into())
            .spawn(move || {
                if let Err(e) = wf_loop(&site, &rows, &view, &stop) {
                    *status.lock().unwrap() = e;
                }
            })
            .expect("spawn waterfall thread");
        w
    }

    /// What to show: centre and width (Hz). The server snaps the width to 30 MHz / 2^n.
    pub fn set_view(&self, center: f64, span: f64) {
        *self.view.lock().unwrap() = Some((center, span));
    }

    /// Lines received since the last call
    pub fn take_rows(&self) -> Vec<WfRow> {
        self.rows.lock().unwrap().drain(..).collect()
    }
}

impl Drop for KiwiWaterfall {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn wf_loop(
    site: &str,
    rows: &Mutex<std::collections::VecDeque<WfRow>>,
    view: &Mutex<Option<(f64, f64)>>,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let (host, port) = match site.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().map_err(|_| format!("bad port in {site}"))?),
        None => (site.to_string(), 8073u16),
    };
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let mut ws = WebSocket::connect(&host, port, &format!("/kiwi/{ms}/W/F"), Duration::from_secs(8))?;
    for c in ["SET auth t=kiwi p=#", "SERVER DE CLIENT LittleSDR W/F", "SET send_dB=1", "SET maxdb=0 mindb=-100", "SET wf_speed=4", "SET wf_comp=0"] {
        ws.send_text(c)?;
    }
    let mut cal = -13.0f32;
    let mut last_keepalive = Instant::now();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some((center, span)) = view.lock().unwrap().take() {
            let z = zoom_for(span);
            let width = KIWI_SPAN / 2f64.powi(z as i32);
            let cf = center.clamp(width / 2.0, KIWI_SPAN - width / 2.0);
            ws.send_text(&format!("SET zoom={z} cf={:.3}", cf / 1000.0))?;
        }
        if last_keepalive.elapsed() > Duration::from_secs(4) {
            ws.send_text("SET keepalive")?;
            last_keepalive = Instant::now();
        }
        let Message::Binary(b) = ws.recv()? else { continue };
        if b.len() < 3 {
            continue;
        }
        match &b[..3] {
            b"MSG" => {
                let text = String::from_utf8_lossy(&b[4.min(b.len())..]).into_owned();
                if let Some(c) = msg_value(&text, "wf_cal").and_then(|v| v.parse().ok()) {
                    cal = c;
                }
            }
            b"W/F" if b.len() > 16 => {
                // "W/F", 1 byte, start bin at the deepest zoom (u32), zoom + flags (u32), sequence
                let x_bin = u32::from_le_bytes(b[4..8].try_into().unwrap()) as f64;
                let zoom = (u32::from_le_bytes(b[8..12].try_into().unwrap()) & 0xFFFF).min(ZOOM_MAX);
                let data = &b[16..];
                let span = KIWI_SPAN / 2f64.powi(zoom as i32);
                let start = x_bin * KIWI_SPAN / (WF_BINS * 2f64.powi(ZOOM_MAX as i32));
                let dbm = data.iter().map(|&v| v as f32 - 255.0 + cal).collect();
                let mut r = rows.lock().unwrap();
                r.push_back(WfRow { start, bin_hz: span / data.len() as f64, dbm });
                while r.len() > 100 {
                    r.pop_front(); // nobody reading: keep only recent lines
                }
            }
            _ => {}
        }
    }
    Ok(())
}
