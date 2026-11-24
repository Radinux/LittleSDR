//! The receivers the app can use, their settings, and the settings form shown for each.
//!
//! The form is our own schema (a list of controls: combo, slider, checkbox, text, info),
//! each with an `id`, a `label`, its value and a `role` (gain, agc, rate ...) so the app
//! knows what a control does without guessing from its label.

use crate::source::{self, Source};
use serde_json::{json, Value};

pub const SOURCES: [&str; 8] = ["RTL-SDR", "Airspy", "Airspy HF+", "SDRplay", "RTL-TCP", "SpyServer", "KiwiSDR", "Fichier IQ"];

/// Airspy R2 / Mini rates (the device keeps the closest it has: 2.5 / 10 MS/s on the R2,
/// 3 / 6 MS/s on the Mini)
pub const AIRSPY_RATES: [f64; 4] = [2.5e6, 3e6, 6e6, 10e6];
/// Airspy HF+ rates (the closest the device has is used)
pub const AIRSPYHF_RATES: [f64; 5] = [192e3, 256e3, 384e3, 768e3, 912e3];
/// SDRplay rates: 2 - 10 MS/s from the ADC, below with the API's decimation
pub const SDRPLAY_RATES: [f64; 9] = [250e3, 500e3, 1e6, 2e6, 3e6, 4e6, 6e6, 8e6, 10e6];

/// RTL-SDR sample rates offered (the tuner's stable ones)
pub const RTL_RATES: [f64; 10] = [250e3, 1.024e6, 1.536e6, 1.792e6, 1.92e6, 2.048e6, 2.16e6, 2.4e6, 2.56e6, 2.88e6];
/// R820T / R828D tuner gain steps (dB)
pub const RTL_GAINS: [f32; 29] = [
    0.0, 0.9, 1.4, 2.7, 3.7, 7.7, 8.7, 12.5, 14.4, 15.7, 16.6, 19.7, 20.7, 22.9, 25.4, 28.0, 29.7, 32.8, 33.8, 36.4, 37.2, 38.6, 40.2, 42.1, 43.4, 43.9, 44.5,
    48.0, 49.6,
];
/// Rates offered for network sources, kept modest: a server's upload limits them
pub const NET_RATES: [f64; 5] = [96e3, 192e3, 384e3, 768e3, 1.024e6];

#[derive(Clone, Debug)]
pub struct SourceSettings {
    pub rtl_rate: f64,
    pub rtl_gain: f32,
    pub rtl_agc: bool,
    pub bias_tee: bool,
    pub direct_sampling: u8,
    pub tcp_host: String,
    pub tcp_rate: f64,
    pub spy_host: String,
    pub spy_rate: f64,
    pub spy_gain: u32,
    pub kiwi_site: String,
    pub kiwi_agc: bool,
    pub kiwi_gain: f32,
    pub file_path: String,
    pub file_loop: bool,
    pub airspy_rate: f64,
    pub airspy_gain: f32,
    pub airspy_agc: bool,
    pub hf_rate: f64,
    pub hf_gain: f32,
    pub hf_agc: bool,
    pub sdrplay_rate: f64,
    pub sdrplay_gain: f32,
    pub sdrplay_agc: bool,
}

impl Default for SourceSettings {
    fn default() -> Self {
        SourceSettings {
            rtl_rate: 2.4e6,
            rtl_gain: 29.7,
            rtl_agc: false,
            bias_tee: false,
            direct_sampling: 0,
            tcp_host: "192.168.1.10:1234".into(),
            tcp_rate: 2.048e6,
            spy_host: String::new(),
            spy_rate: 192e3,
            spy_gain: 10,
            kiwi_site: String::new(),
            kiwi_agc: true,
            kiwi_gain: 50.0,
            file_path: String::new(),
            file_loop: true,
            airspy_rate: 2.5e6,
            airspy_gain: 12.0,
            airspy_agc: false,
            hf_rate: 768e3,
            hf_gain: 48.0,
            hf_agc: true,
            sdrplay_rate: 2e6,
            sdrplay_gain: 20.0,
            sdrplay_agc: true,
        }
    }
}

impl SourceSettings {
    pub fn to_json(&self) -> Value {
        json!({
            "rtl_rate": self.rtl_rate, "rtl_gain": self.rtl_gain, "rtl_agc": self.rtl_agc, "bias_tee": self.bias_tee,
            "direct_sampling": self.direct_sampling, "tcp_host": self.tcp_host, "tcp_rate": self.tcp_rate,
            "spy_host": self.spy_host, "spy_rate": self.spy_rate, "spy_gain": self.spy_gain,
            "kiwi_site": self.kiwi_site, "kiwi_agc": self.kiwi_agc, "kiwi_gain": self.kiwi_gain,
            "file_path": self.file_path, "file_loop": self.file_loop,
            "airspy_rate": self.airspy_rate, "airspy_gain": self.airspy_gain, "airspy_agc": self.airspy_agc,
            "hf_rate": self.hf_rate, "hf_gain": self.hf_gain, "hf_agc": self.hf_agc,
            "sdrplay_rate": self.sdrplay_rate, "sdrplay_gain": self.sdrplay_gain, "sdrplay_agc": self.sdrplay_agc,
        })
    }

    pub fn from_json(v: &Value) -> SourceSettings {
        let d = SourceSettings::default();
        let f = |k: &str, def: f64| v.get(k).and_then(Value::as_f64).unwrap_or(def);
        let b = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        let s = |k: &str, def: &str| v.get(k).and_then(Value::as_str).unwrap_or(def).to_string();
        SourceSettings {
            rtl_rate: f("rtl_rate", d.rtl_rate),
            rtl_gain: f("rtl_gain", d.rtl_gain as f64) as f32,
            rtl_agc: b("rtl_agc", d.rtl_agc),
            bias_tee: b("bias_tee", d.bias_tee),
            direct_sampling: f("direct_sampling", 0.0) as u8,
            tcp_host: s("tcp_host", &d.tcp_host),
            tcp_rate: f("tcp_rate", d.tcp_rate),
            spy_host: s("spy_host", ""),
            spy_rate: f("spy_rate", d.spy_rate),
            spy_gain: f("spy_gain", d.spy_gain as f64) as u32,
            kiwi_site: s("kiwi_site", ""),
            kiwi_agc: b("kiwi_agc", d.kiwi_agc),
            kiwi_gain: f("kiwi_gain", d.kiwi_gain as f64) as f32,
            file_path: s("file_path", ""),
            file_loop: b("file_loop", true),
            airspy_rate: f("airspy_rate", d.airspy_rate),
            airspy_gain: f("airspy_gain", d.airspy_gain as f64) as f32,
            airspy_agc: b("airspy_agc", d.airspy_agc),
            hf_rate: f("hf_rate", d.hf_rate),
            hf_gain: f("hf_gain", d.hf_gain as f64) as f32,
            hf_agc: b("hf_agc", d.hf_agc),
            sdrplay_rate: f("sdrplay_rate", d.sdrplay_rate),
            sdrplay_gain: f("sdrplay_gain", d.sdrplay_gain as f64) as f32,
            sdrplay_agc: b("sdrplay_agc", d.sdrplay_agc),
        }
    }

    /// Sample rate asked of the source
    pub fn rate(&self, kind: &str) -> f64 {
        match kind {
            "RTL-SDR" => self.rtl_rate,
            "RTL-TCP" => self.tcp_rate,
            "SpyServer" => self.spy_rate,
            "KiwiSDR" => 12_000.0,
            "Airspy" => self.airspy_rate,
            "Airspy HF+" => self.hf_rate,
            "SDRplay" => self.sdrplay_rate,
            _ => 0.0,
        }
    }

    /// Gain asked of the source (None = automatic)
    pub fn gain(&self, kind: &str) -> Option<f32> {
        match kind {
            "RTL-SDR" | "RTL-TCP" if !self.rtl_agc => Some(self.rtl_gain),
            "SpyServer" => Some(self.spy_gain as f32),
            "KiwiSDR" if !self.kiwi_agc => Some(self.kiwi_gain),
            "Airspy" if !self.airspy_agc => Some(self.airspy_gain),
            "Airspy HF+" if !self.hf_agc => Some(self.hf_gain),
            "SDRplay" if !self.sdrplay_agc => Some(self.sdrplay_gain),
            _ => None,
        }
    }
}

/// Live information from a running source, shown in its form
#[derive(Default, Clone)]
pub struct SourceInfo {
    pub status: String,
    /// SpyServer: the device's rates and gain range once connected
    pub spy_rates: Vec<f64>,
    pub spy_max_gain: u32,
    /// File: position and length (s)
    pub file_pos: f64,
    pub file_len: f64,
}

fn combo(id: &str, label: &str, role: &str, value: usize, items: Vec<String>) -> Value {
    json!({"t": "combo", "id": id, "label": label, "role": role, "value": value, "items": items})
}
fn check(id: &str, label: &str, role: &str, value: bool) -> Value {
    json!({"t": "checkbox", "id": id, "label": label, "role": role, "value": value})
}
fn slider(id: &str, label: &str, role: &str, value: f32, min: f32, max: f32, step: f32, fmt: &str, disabled: bool) -> Value {
    json!({"t": "slider_float", "id": id, "label": label, "role": role, "value": value, "min": min, "max": max, "step": step, "fmt": fmt, "decimals": 1, "disabled": disabled})
}
fn text(id: &str, label: &str, role: &str, value: &str) -> Value {
    json!({"t": "input_text", "id": id, "label": label, "role": role, "value": value, "maxLen": 256})
}
fn info(value: &str) -> Value {
    json!({"t": "text", "value": value})
}

fn rate_label(r: f64) -> String {
    if r >= 1e6 {
        format!("{:.3} MS/s", r / 1e6).replace(".000", "")
    } else {
        format!("{:.0} kS/s", r / 1e3)
    }
}

fn nearest(list: &[f64], v: f64) -> usize {
    list.iter().enumerate().min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs())).map_or(0, |x| x.0)
}

/// The settings form of a source
pub fn menu(kind: &str, s: &SourceSettings, i: &SourceInfo, usb_present: bool) -> Value {
    let mut w: Vec<Value> = Vec::new();
    match kind {
        "RTL-SDR" => {
            if !usb_present {
                w.push(info("Branchez une clé RTL-SDR (USB OTG) et autorisez l'accès."));
            }
            w.push(combo("rtl_rate", "Débit", "rate", nearest(&RTL_RATES, s.rtl_rate), RTL_RATES.iter().map(|&r| rate_label(r)).collect()));
            w.push(check("rtl_agc", "AGC du tuner", "agc", s.rtl_agc));
            w.push(slider("rtl_gain", "Gain", "gain", s.rtl_gain, 0.0, 49.6, 0.1, "%.1f dB", s.rtl_agc));
            w.push(check("bias_tee", "Bias-tee (alimente une antenne active)", "bias_tee", s.bias_tee));
            w.push(combo("direct_sampling", "Échantillonnage direct (HF)", "direct_sampling", s.direct_sampling as usize, vec!["Non".into(), "Branche I".into(), "Branche Q".into()]));
        }
        "Airspy" => {
            if !usb_present && cfg!(target_os = "android") {
                w.push(info("Branchez l'Airspy (USB OTG) et autorisez l'accès."));
            }
            w.push(combo("airspy_rate", "Débit", "rate", nearest(&AIRSPY_RATES, s.airspy_rate), AIRSPY_RATES.iter().map(|&r| rate_label(r)).collect()));
            w.push(check("airspy_agc", "AGC (LNA et mélangeur)", "agc", s.airspy_agc));
            w.push(slider("airspy_gain", "Gain (linéarité)", "gain", s.airspy_gain, 0.0, 21.0, 1.0, "%.0f", s.airspy_agc));
            w.push(check("bias_tee", "Bias-tee (alimente une antenne active)", "bias_tee", s.bias_tee));
        }
        "Airspy HF+" => {
            if !usb_present && cfg!(target_os = "android") {
                w.push(info("Branchez l'Airspy HF+ (USB OTG) et autorisez l'accès."));
            }
            w.push(combo("hf_rate", "Débit", "rate", nearest(&AIRSPYHF_RATES, s.hf_rate), AIRSPYHF_RATES.iter().map(|&r| rate_label(r)).collect()));
            w.push(check("hf_agc", "AGC du récepteur", "agc", s.hf_agc));
            w.push(slider("hf_gain", "Gain (atténuateur, pas de 6 dB)", "gain", s.hf_gain, 0.0, 48.0, 6.0, "%.0f dB", s.hf_agc));
        }
        "SDRplay" => {
            if cfg!(target_os = "android") {
                w.push(info("SDRplay ne fournit pas son pilote pour Android : disponible sur Linux seulement."));
            }
            w.push(combo("sdrplay_rate", "Débit", "rate", nearest(&SDRPLAY_RATES, s.sdrplay_rate), SDRPLAY_RATES.iter().map(|&r| rate_label(r)).collect()));
            w.push(check("sdrplay_agc", "AGC", "agc", s.sdrplay_agc));
            w.push(slider("sdrplay_gain", "Gain", "gain", s.sdrplay_gain, 0.0, 39.0, 1.0, "%.0f dB", s.sdrplay_agc));
            w.push(check("bias_tee", "Bias-tee (RSP1A/1B, RSP2, RSPduo, RSPdx)", "bias_tee", s.bias_tee));
            w.push(info("Pilote : API SDRplay 3 et son service (sudo systemctl start sdrplay)."));
        }
        "RTL-TCP" => {
            w.push(text("tcp_host", "Serveur (hôte:port)", "host", &s.tcp_host));
            w.push(combo("tcp_rate", "Débit", "rate", nearest(&RTL_RATES, s.tcp_rate), RTL_RATES.iter().map(|&r| rate_label(r)).collect()));
            w.push(check("rtl_agc", "AGC du tuner", "agc", s.rtl_agc));
            w.push(slider("rtl_gain", "Gain", "gain", s.rtl_gain, 0.0, 49.6, 0.1, "%.1f dB", s.rtl_agc));
            w.push(check("bias_tee", "Bias-tee", "bias_tee", s.bias_tee));
        }
        "SpyServer" => {
            w.push(text("spy_host", "Serveur (hôte:port)", "host", &s.spy_host));
            let rates: Vec<f64> = if i.spy_rates.is_empty() { NET_RATES.to_vec() } else { i.spy_rates.clone() };
            w.push(combo("spy_rate", "Débit", "rate", nearest(&rates, s.spy_rate), rates.iter().map(|&r| rate_label(r)).collect()));
            let max = if i.spy_max_gain > 0 { i.spy_max_gain } else { 21 };
            w.push(slider("spy_gain", "Gain", "gain", s.spy_gain.min(max) as f32, 0.0, max as f32, 1.0, "%.0f", false));
            w.push(info("Sur Internet, préférez un débit modeste (96-192 kS/s)."));
        }
        "KiwiSDR" => {
            w.push(text("kiwi_site", "KiwiSDR (hôte:port)", "host", &s.kiwi_site));
            w.push(check("kiwi_agc", "AGC du serveur", "agc", s.kiwi_agc));
            w.push(slider("kiwi_gain", "Gain", "gain", s.kiwi_gain, 0.0, 120.0, 1.0, "%.0f dB", s.kiwi_agc));
        }
        _ => {
            w.push(text("file_path", "Fichier WAV IQ", "path", &s.file_path));
            w.push(check("file_loop", "Lecture en boucle", "loop", s.file_loop));
            if i.file_len > 0.0 {
                w.push(info(&format!("{:.0} s / {:.0} s", i.file_pos, i.file_len)));
            }
        }
    }
    if !i.status.is_empty() {
        w.push(info(&i.status));
    }
    Value::Array(w)
}

/// What an action on the form changed
pub enum Change {
    None,
    /// Applied live through the source's controls
    Rate(f64),
    Gain(Option<f32>),
    BiasTee(bool),
    DirectSampling(u8),
    /// Needs the source to be opened again (address, file)
    Restart,
}

/// Applies a form action (control id, typed value). Returns what changed.
pub fn apply(kind: &str, s: &mut SourceSettings, i: &SourceInfo, id: &str, b: bool, idx: i32, f: f32, text_v: &str) -> Change {
    match id {
        "rtl_rate" | "tcp_rate" => {
            let r = RTL_RATES[(idx.max(0) as usize).min(RTL_RATES.len() - 1)];
            if id == "rtl_rate" {
                s.rtl_rate = r
            } else {
                s.tcp_rate = r
            };
            Change::Rate(r)
        }
        "spy_rate" => {
            let rates: Vec<f64> = if i.spy_rates.is_empty() { NET_RATES.to_vec() } else { i.spy_rates.clone() };
            s.spy_rate = rates[(idx.max(0) as usize).min(rates.len() - 1)];
            Change::Rate(s.spy_rate)
        }
        "rtl_agc" => {
            s.rtl_agc = b;
            Change::Gain(s.gain(kind))
        }
        "airspy_rate" => {
            s.airspy_rate = AIRSPY_RATES[(idx.max(0) as usize).min(AIRSPY_RATES.len() - 1)];
            Change::Rate(s.airspy_rate)
        }
        "hf_rate" => {
            s.hf_rate = AIRSPYHF_RATES[(idx.max(0) as usize).min(AIRSPYHF_RATES.len() - 1)];
            Change::Rate(s.hf_rate)
        }
        "sdrplay_rate" => {
            s.sdrplay_rate = SDRPLAY_RATES[(idx.max(0) as usize).min(SDRPLAY_RATES.len() - 1)];
            Change::Rate(s.sdrplay_rate)
        }
        "airspy_agc" => {
            s.airspy_agc = b;
            Change::Gain(s.gain(kind))
        }
        "airspy_gain" => {
            s.airspy_gain = f.round().clamp(0.0, 21.0);
            Change::Gain(s.gain(kind))
        }
        "hf_agc" => {
            s.hf_agc = b;
            Change::Gain(s.gain(kind))
        }
        "hf_gain" => {
            s.hf_gain = ((f / 6.0).round() * 6.0).clamp(0.0, 48.0);
            Change::Gain(s.gain(kind))
        }
        "sdrplay_agc" => {
            s.sdrplay_agc = b;
            Change::Gain(s.gain(kind))
        }
        "sdrplay_gain" => {
            s.sdrplay_gain = f.round().clamp(0.0, 39.0);
            Change::Gain(s.gain(kind))
        }
        "rtl_gain" => {
            // Snap to the tuner's steps
            s.rtl_gain = RTL_GAINS.iter().copied().min_by(|a, c| (a - f).abs().total_cmp(&(c - f).abs())).unwrap_or(f);
            Change::Gain(s.gain(kind))
        }
        "spy_gain" => {
            s.spy_gain = f.max(0.0).round() as u32;
            Change::Gain(s.gain(kind))
        }
        "kiwi_agc" => {
            s.kiwi_agc = b;
            Change::Gain(s.gain(kind))
        }
        "kiwi_gain" => {
            s.kiwi_gain = f;
            Change::Gain(s.gain(kind))
        }
        "bias_tee" => {
            s.bias_tee = b;
            Change::BiasTee(b)
        }
        "direct_sampling" => {
            s.direct_sampling = idx.clamp(0, 2) as u8;
            Change::DirectSampling(s.direct_sampling)
        }
        "tcp_host" => {
            s.tcp_host = text_v.trim().to_string();
            Change::Restart
        }
        "spy_host" => {
            s.spy_host = text_v.trim().to_string();
            Change::Restart
        }
        "kiwi_site" => {
            s.kiwi_site = text_v.trim().to_string();
            Change::Restart
        }
        "file_path" => {
            s.file_path = text_v.trim().to_string();
            Change::Restart
        }
        "file_loop" => {
            s.file_loop = b;
            Change::None
        }
        _ => Change::None,
    }
}

/// Opens the source; also returns its live status handles
pub fn open(kind: &str, s: &SourceSettings, usb_fd: Option<i32>) -> Result<(Box<dyn Source>, Handles), String> {
    let mut h = Handles::default();
    let src: Box<dyn Source> = match kind {
        #[cfg(feature = "rtlsdr")]
        "RTL-SDR" => {
            if cfg!(target_os = "android") && usb_fd.is_none() {
                return Err("aucune clé RTL-SDR (USB)".into());
            }
            Box::new(source::rtlsdr::RtlSdr { index: 0, fd: usb_fd })
        }
        "Airspy" => {
            if cfg!(target_os = "android") && usb_fd.is_none() {
                return Err("aucun Airspy (USB)".into());
            }
            Box::new(source::airspy::Airspy { fd: usb_fd })
        }
        "Airspy HF+" => {
            if cfg!(target_os = "android") && usb_fd.is_none() {
                return Err("aucun Airspy HF+ (USB)".into());
            }
            Box::new(source::airspy::AirspyHf { fd: usb_fd })
        }
        "SDRplay" => Box::new(source::sdrplay::SdrPlay),
        "RTL-TCP" => {
            if s.tcp_host.is_empty() {
                return Err("indiquez le serveur rtl_tcp".into());
            }
            Box::new(source::rtltcp::RtlTcp { addr: s.tcp_host.clone() })
        }
        "SpyServer" => {
            if s.spy_host.is_empty() {
                return Err("indiquez le serveur SpyServer".into());
            }
            let sp = source::spyserver::SpyServer::new(&s.spy_host);
            h.spy = Some(sp.status.clone());
            Box::new(sp)
        }
        "KiwiSDR" => {
            if s.kiwi_site.is_empty() {
                return Err("choisissez un KiwiSDR".into());
            }
            let k = source::kiwisdr::KiwiSdr::new(&s.kiwi_site);
            h.kiwi = Some(k.status.clone());
            Box::new(k)
        }
        "Fichier IQ" => {
            if s.file_path.is_empty() {
                return Err("choisissez un fichier IQ".into());
            }
            let f = source::wavfile::WavFile::open(&s.file_path, None)?;
            f.state.looping.store(s.file_loop, std::sync::atomic::Ordering::Relaxed);
            h.file = Some((f.state.clone(), f.info.clone()));
            Box::new(f)
        }
        _ => return Err(format!("source inconnue : {kind}")),
    };
    Ok((src, h))
}

/// Status handles of a running source
#[derive(Default)]
pub struct Handles {
    pub spy: Option<std::sync::Arc<std::sync::Mutex<source::spyserver::SpyStatus>>>,
    pub kiwi: Option<std::sync::Arc<std::sync::Mutex<source::kiwisdr::KiwiStatus>>>,
    pub file: Option<(std::sync::Arc<source::wavfile::WavState>, source::wavfile::WavInfo)>,
}

impl Handles {
    pub fn info(&self) -> SourceInfo {
        let mut i = SourceInfo::default();
        if let Some(sp) = &self.spy {
            let st = sp.lock().unwrap();
            i.status = st.message.clone();
            if let Some(d) = &st.device {
                i.spy_rates = d.rates().into_iter().map(|r| r.1).collect();
                i.spy_max_gain = d.max_gain;
            }
        }
        if let Some(k) = &self.kiwi {
            i.status = k.lock().unwrap().message.clone();
        }
        if let Some((st, inf)) = &self.file {
            i.file_pos = st.pos.load(std::sync::atomic::Ordering::Relaxed) as f64 / inf.rate;
            i.file_len = inf.duration();
        }
        i
    }
}
