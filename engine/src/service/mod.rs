//! The engine as a service for the app: one receiver at a time (source + DSP thread),
//! the display spectrum, the sound for the platform's output, the state the UI polls, the
//! settings (persisted as JSON in the app's folder) and every decoder's JSON. Platform-free:
//! the Android module (JNI, AAudio, Vulkan) and tests sit on top of it.

pub mod fft;
pub mod sources;

use crate::audio::{AudioSink, RingShared, RingSink};
use crate::dsp::Frame;
use crate::engine::{Engine, EngineCtl, Mode};
use crate::source::{self, IqBlock, SourceConfig, SourceCtl, SourceHandle};
use fft::{DisplayFft, FftHistory};
use serde_json::{json, Value};
use sources::{Change, Handles, SourceSettings};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The app's demodulator numbering (SDR++'s, kept): NFM, WFM, AM, DSB, USB, CW, LSB, RAW, then
/// ours: DAB, DRM, LRPT
pub fn mode_of(i: i32) -> Mode {
    match i {
        0 => Mode::Nfm,
        1 => Mode::Wfm,
        4 => Mode::Usb,
        5 => Mode::Cw,
        6 => Mode::Lsb,
        8 => Mode::Dab,
        9 => Mode::Drm,
        10 => Mode::Lrpt,
        _ => Mode::Am,
    }
}

/// Shared with the DSP thread
struct Shared {
    engine: Mutex<Engine>,
    hist: Mutex<FftHistory>,
    display: Mutex<Display>,
    squelch: Mutex<(bool, f32)>,
    /// Rate and centre of the latest block (f64 bits)
    rate: AtomicU64,
    center: AtomicU64,
}

struct Display {
    fft: DisplayFft,
    smoothing: f32,
    peak_decay: f32,
}

struct Running {
    src: SourceHandle,
    worker: JoinHandle<()>,
    stop: Arc<AtomicBool>,
    handles: Handles,
}

struct Inner {
    root: String,
    kind: String,
    settings: SourceSettings,
    usb_fd: Option<i32>,
    mode_idx: i32,
    freq: f64,
    center: f64,
    bandwidth: [f64; 11],
    volume: f32,
    options: Value,
    running: Option<Running>,
    last_save: Instant,
    view: (f64, f64),
}

pub struct Service {
    inner: Mutex<Inner>,
    shared: Arc<Shared>,
    /// The ring the platform's audio callback reads, for the whole life of the app
    pub audio: Arc<RingShared>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

pub fn service() -> Option<&'static Service> {
    SERVICE.get()
}

fn default_options() -> Value {
    json!({
        "input": {"decimation": 1, "dcBlocking": false, "invertIQ": false, "offset": 0.0},
        "fft": {"size": 4096, "rate": 25.0, "window": 2, "smoothing": 0.4, "peakHold": false, "peakDecay": 10.0},
        "audio": {"device": 0, "latency": 4},
        "operator": {"location": "", "callsign": ""},
        "recorder": {"path": ""},
        "demod": {"nb": false, "nbLevel": 8.0, "stereo": true, "snapInterval": 1000.0},
    })
}

impl Service {
    /// Once at start: the app's folder (settings), and the FFT size of the display
    pub fn init(root: &str, fft_size: usize) -> &'static Service {
        SERVICE.get_or_init(|| {
            let cfg: Value = std::fs::read_to_string(format!("{root}/littlesdr.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
            let mut options = default_options();
            if let Some(o) = cfg.get("options").and_then(Value::as_object) {
                for (k, v) in o {
                    if let (Some(dst), Some(src)) = (options.get_mut(k).and_then(Value::as_object_mut), v.as_object()) {
                        for (kk, vv) in src {
                            dst.insert(kk.clone(), vv.clone());
                        }
                    }
                }
            }
            if fft_size > 0 && cfg.get("options").is_none() {
                options["fft"]["size"] = json!(fft_size);
            }
            let fsz = options["fft"]["size"].as_u64().unwrap_or(4096) as usize;
            let mode_idx = cfg.get("mode").and_then(Value::as_i64).unwrap_or(1) as i32;
            let freq = cfg.get("freq").and_then(Value::as_f64).unwrap_or(100e6);
            let mut bandwidth = [0.0; 11];
            for (i, b) in bandwidth.iter_mut().enumerate() {
                *b = mode_of(i as i32).default_bandwidth().unwrap_or(0.0);
            }
            if let Some(bw) = cfg.get("bandwidth").and_then(Value::as_array) {
                for (i, v) in bw.iter().enumerate().take(11) {
                    if let Some(x) = v.as_f64() {
                        bandwidth[i] = x;
                    }
                }
            }
            let volume = cfg.get("volume").and_then(Value::as_f64).unwrap_or(0.8) as f32;
            let mut engine = Engine::new(mode_of(mode_idx), freq);
            engine.control(&EngineCtl::Volume(volume));
            let sq = cfg.get("squelch").and_then(Value::as_array).map(|a| (a[0].as_bool().unwrap_or(false), a[1].as_f64().unwrap_or(-60.0) as f32)).unwrap_or((false, -60.0));
            let shared = Arc::new(Shared {
                engine: Mutex::new(engine),
                hist: Mutex::new(FftHistory::new(fsz)),
                display: Mutex::new(Display {
                    fft: DisplayFft::new(fsz, options["fft"]["rate"].as_f64().unwrap_or(25.0) as f32, options["fft"]["window"].as_i64().unwrap_or(2) as i32),
                    smoothing: options["fft"]["smoothing"].as_f64().unwrap_or(0.4) as f32,
                    peak_decay: options["fft"]["peakDecay"].as_f64().unwrap_or(10.0) as f32,
                }),
                squelch: Mutex::new(sq),
                rate: AtomicU64::new(0),
                center: AtomicU64::new(freq.to_bits()),
            });
            let audio = RingSink::new(48_000.0, 160.0).shared;
            let svc = Service {
                inner: Mutex::new(Inner {
                    root: root.to_string(),
                    kind: cfg.get("source").and_then(Value::as_str).filter(|k| sources::SOURCES.contains(k)).unwrap_or("RTL-SDR").to_string(),
                    settings: cfg.get("sources").map(SourceSettings::from_json).unwrap_or_default(),
                    usb_fd: None,
                    mode_idx,
                    freq,
                    center: cfg.get("center").and_then(Value::as_f64).unwrap_or(freq),
                    bandwidth,
                    volume,
                    options,
                    running: None,
                    last_save: Instant::now(),
                    view: (0.0, 0.0),
                }),
                shared,
                audio,
            };
            svc.apply_options();
            svc
        })
    }

    fn save(&self, i: &mut Inner) {
        i.last_save = Instant::now();
        let sq = *self.shared.squelch.lock().unwrap();
        let v = json!({
            "source": i.kind, "freq": i.freq, "center": i.center, "mode": i.mode_idx, "volume": i.volume,
            "bandwidth": i.bandwidth.to_vec(), "squelch": [sq.0, sq.1],
            "sources": i.settings.to_json(), "options": i.options,
        });
        let _ = std::fs::create_dir_all(&i.root);
        let _ = std::fs::write(format!("{}/littlesdr.json", i.root), v.to_string());
    }

    // ------------------------------------------------------------------ sources

    pub fn source_names(&self) -> Vec<String> {
        sources::SOURCES.iter().map(|s| s.to_string()).collect()
    }

    pub fn selected_source(&self) -> String {
        self.inner.lock().unwrap().kind.clone()
    }

    pub fn select_source(&self, name: &str) {
        if !sources::SOURCES.contains(&name) {
            return;
        }
        let was = self.stop();
        let mut i = self.inner.lock().unwrap();
        i.kind = name.to_string();
        self.save(&mut i);
        drop(i);
        if was {
            self.start();
        }
    }

    pub fn set_usb_device(&self, fd: Option<i32>) {
        self.inner.lock().unwrap().usb_fd = fd;
    }

    /// The settings form of the selected source, after applying an action if `id` is set
    pub fn source_menu(&self, id: &str, b: bool, idx: i32, f: f32, s: &str) -> String {
        let mut restart = false;
        {
            let mut i = self.inner.lock().unwrap();
            if !id.is_empty() {
                let kind = i.kind.clone();
                let info = i.running.as_ref().map(|r| r.handles.info()).unwrap_or_default();
                let change = sources::apply(&kind, &mut i.settings, &info, id, b, idx, f, s);
                let ctl = |c: SourceCtl| {
                    if let Some(r) = &i.running {
                        let _ = r.src.ctl.send(c);
                    }
                };
                match change {
                    Change::Rate(r) => ctl(SourceCtl::Rate(r as u32)),
                    Change::Gain(g) => ctl(SourceCtl::Gain(g)),
                    Change::BiasTee(on) => ctl(SourceCtl::BiasTee(on)),
                    Change::DirectSampling(m) => ctl(SourceCtl::DirectSampling(m)),
                    Change::Restart => restart = i.running.is_some(),
                    Change::None => {}
                }
                self.save(&mut i);
            }
        }
        if restart {
            self.stop();
            self.start();
        }
        let i = self.inner.lock().unwrap();
        let info = i.running.as_ref().map(|r| r.handles.info()).unwrap_or_default();
        sources::menu(&i.kind, &i.settings, &info, i.usb_fd.is_some()).to_string()
    }

    pub fn kiwi_set_site(&self, host_port: &str) {
        let restart = {
            let mut i = self.inner.lock().unwrap();
            i.settings.kiwi_site = host_port.to_string();
            self.save(&mut i);
            i.kind == "KiwiSDR" && i.running.is_some()
        };
        if restart {
            self.stop();
            self.start();
        }
    }

    // ------------------------------------------------------------------ playback

    pub fn is_playing(&self) -> bool {
        self.inner.lock().unwrap().running.is_some()
    }

    /// Opens the source and starts the DSP thread. Err: why it could not start.
    pub fn start(&self) -> bool {
        self.try_start().is_ok()
    }

    pub fn try_start(&self) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        if i.running.is_some() {
            return Ok(());
        }
        let kind = i.kind.clone();
        let (src, handles) = sources::open(&kind, &i.settings, i.usb_fd)?;
        let mode = mode_of(i.mode_idx);
        let mut rate = i.settings.rate(&kind);
        if mode == Mode::Dab && (kind == "RTL-SDR" || kind == "RTL-TCP") {
            rate = crate::dab::ofdm::RATE;
        }
        if rate > 0.0 && (i.center - i.freq).abs() > 0.45 * rate {
            i.center = plan_center(i.freq, rate, mode);
        }
        let cfg = SourceConfig {
            rate: rate as u32,
            center: i.center as u64,
            gain: i.settings.gain(&kind),
            direct_sampling: if kind == "RTL-SDR" { i.settings.direct_sampling } else { 0 },
            bias_tee: i.settings.bias_tee && (kind == "RTL-SDR" || kind == "RTL-TCP"),
        };
        let (tx, rx) = sync_channel::<IqBlock>(64);
        let src = source::spawn(src, cfg, tx);
        let stop = Arc::new(AtomicBool::new(false));
        let sh = self.shared.clone();
        let st = stop.clone();
        let lat = 40.0 * i.options["audio"]["latency"].as_f64().unwrap_or(4.0).clamp(1.0, 10.0);
        let mut sink = RingSink::with_shared(self.audio.clone(), 48_000.0, lat);
        self.audio.clear();
        let worker = std::thread::Builder::new()
            .name("dsp".into())
            .spawn(move || {
                let mut out: Vec<Frame> = Vec::new();
                let mut rows: Vec<Vec<f32>> = Vec::new();
                loop {
                    let block = match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(b) => b,
                        Err(RecvTimeoutError::Timeout) if !st.load(Ordering::Relaxed) => continue,
                        _ => break,
                    };
                    sh.rate.store(block.rate.to_bits(), Ordering::Relaxed);
                    sh.center.store(block.center.to_bits(), Ordering::Relaxed);
                    {
                        let mut d = sh.display.lock().unwrap();
                        rows.clear();
                        d.fft.feed(&block.samples, block.rate, &mut rows);
                        let (sm, fall, size) = (d.smoothing, d.peak_decay / d.fft.rate, d.fft.size);
                        drop(d);
                        let mut h = sh.hist.lock().unwrap();
                        if h.bins != size {
                            h.resize(size);
                        }
                        for r in &rows {
                            h.push(r, sm, fall);
                        }
                    }
                    let (rate, center) = (block.rate, block.center);
                    out.clear();
                    let (lo, hi, off) = {
                        let mut e = sh.engine.lock().unwrap();
                        e.process(block, &mut out);
                        let (lo, hi) = e.channel();
                        (lo, hi, e.frequency() - center)
                    };
                    let (sq_on, sq_lvl) = *sh.squelch.lock().unwrap();
                    if sq_on && sh.hist.lock().unwrap().level(rate, off + lo, off + hi) < sq_lvl {
                        out.iter_mut().for_each(|f| *f = Frame::default());
                    }
                    sink.write(&out);
                }
            })
            .map_err(|e| e.to_string())?;
        i.running = Some(Running { src, worker, stop, handles });
        Ok(())
    }

    /// Stops the receiver; true if it was running
    pub fn stop(&self) -> bool {
        let r = {
            let mut i = self.inner.lock().unwrap();
            self.save(&mut i);
            i.running.take()
        };
        let Some(r) = r else { return false };
        let _ = r.src.ctl.send(SourceCtl::Stop);
        r.stop.store(true, Ordering::Relaxed);
        let _ = r.src.thread.join();
        let _ = r.worker.join();
        self.audio.clear();
        true
    }

    /// The ring the platform's audio callback reads
    pub fn audio(&self) -> Arc<RingShared> {
        self.audio.clone()
    }

    // ------------------------------------------------------------------ tuning

    fn retune(&self, i: &mut Inner, freq: f64) {
        let mode = mode_of(i.mode_idx);
        let rate = f64::from_bits(self.shared.rate.load(Ordering::Relaxed));
        let rate = if rate > 0.0 { rate } else { i.settings.rate(&i.kind) };
        let fixed = i.kind == "Fichier IQ";
        if !fixed && rate > 0.0 && needs_retune(freq, i.center, rate, mode) {
            i.center = plan_center(freq, rate, mode);
            if let Some(r) = &i.running {
                let _ = r.src.ctl.send(SourceCtl::Center(i.center as u64));
            }
        }
        i.freq = freq;
        let mut e = self.shared.engine.lock().unwrap();
        e.control(&EngineCtl::Tune(freq));
        drop(e);
        if i.last_save.elapsed() > Duration::from_secs(2) {
            self.save(i);
        }
    }

    pub fn tune(&self, hz: f64) {
        let mut i = self.inner.lock().unwrap();
        self.retune(&mut i, hz);
    }

    /// Moves the hardware centre, the tuned frequency with it
    pub fn tune_center(&self, hz: f64) {
        let mut i = self.inner.lock().unwrap();
        if i.kind != "Fichier IQ" {
            i.center = hz;
            if let Some(r) = &i.running {
                let _ = r.src.ctl.send(SourceCtl::Center(hz as u64));
            }
        }
        self.retune(&mut i, hz);
    }

    pub fn set_view(&self, bw: f64, offset: f64) {
        self.inner.lock().unwrap().view = (bw, offset);
    }

    /// What the UI polls: [vfo, centre, span, vfo offset, channel width, level dB, mode,
    /// playing, squelch on, squelch level, volume]
    pub fn state(&self) -> [f64; 11] {
        let i = self.inner.lock().unwrap();
        let rate = f64::from_bits(self.shared.rate.load(Ordering::Relaxed));
        let span = if rate > 0.0 { rate } else { i.settings.rate(&i.kind) };
        let center = if i.running.is_some() && rate > 0.0 { f64::from_bits(self.shared.center.load(Ordering::Relaxed)) } else { i.center };
        let (lo, hi, bw) = {
            let e = self.shared.engine.lock().unwrap();
            let (lo, hi) = e.channel();
            (lo, hi, hi - lo)
        };
        let off = i.freq - center;
        let level = self.shared.hist.lock().unwrap().level(span, off + lo, off + hi) as f64;
        let (sq_on, sq) = *self.shared.squelch.lock().unwrap();
        [i.freq, center, span, off, bw, level, i.mode_idx as f64, i.running.is_some() as u8 as f64, sq_on as u8 as f64, sq as f64, i.volume as f64]
    }

    // ------------------------------------------------------------------ demodulator

    pub fn set_mode(&self, m: i32) {
        let mut i = self.inner.lock().unwrap();
        let was_dab = mode_of(i.mode_idx) == Mode::Dab;
        i.mode_idx = m;
        let mode = mode_of(m);
        let bw = i.bandwidth[(m as usize).min(10)];
        {
            let mut e = self.shared.engine.lock().unwrap();
            e.control(&EngineCtl::Mode(mode));
            if bw > 0.0 && mode.default_bandwidth().is_some() {
                e.control(&EngineCtl::Bandwidth(bw));
            }
        }
        // DAB needs its own sample rate on an RTL-SDR
        let is_rtl = i.kind == "RTL-SDR" || i.kind == "RTL-TCP";
        if is_rtl && (mode == Mode::Dab) != was_dab {
            let rate = if mode == Mode::Dab { crate::dab::ofdm::RATE } else { i.settings.rate(&i.kind) };
            if let Some(r) = &i.running {
                let _ = r.src.ctl.send(SourceCtl::Rate(rate as u32));
            }
        }
        let f = i.freq;
        self.retune(&mut i, f);
        self.save(&mut i);
    }

    pub fn set_bandwidth(&self, hz: f32) {
        let mut i = self.inner.lock().unwrap();
        let m = (i.mode_idx as usize).min(10);
        if mode_of(i.mode_idx).default_bandwidth().is_none() {
            return; // fixed (FM, digital)
        }
        i.bandwidth[m] = hz as f64;
        self.shared.engine.lock().unwrap().control(&EngineCtl::Bandwidth(hz as f64));
        self.save(&mut i);
    }

    pub fn set_squelch(&self, on: bool, level: f32) {
        *self.shared.squelch.lock().unwrap() = (on, level);
    }

    pub fn set_volume(&self, v: f32) {
        let mut i = self.inner.lock().unwrap();
        i.volume = v;
        self.shared.engine.lock().unwrap().control(&EngineCtl::Volume(v));
    }

    pub fn estimate_bandwidth(&self, min: f64, max: f64) -> f32 {
        let i = self.inner.lock().unwrap();
        let rate = f64::from_bits(self.shared.rate.load(Ordering::Relaxed));
        let center = f64::from_bits(self.shared.center.load(Ordering::Relaxed));
        self.shared.hist.lock().unwrap().occupied(rate, i.freq - center, min, max)
    }

    // ------------------------------------------------------------------ options

    pub fn options_json(&self) -> String {
        let i = self.inner.lock().unwrap();
        let mut o = i.options.clone();
        let e = self.shared.engine.lock().unwrap();
        o["demod"]["demod"] = json!(format!("{:?}", e.mode()));
        o["demod"]["stereoAllowed"] = json!(e.mode() == Mode::Wfm);
        o["demod"]["nbAllowed"] = json!(true);
        o["demod"]["deempAllowed"] = json!(false);
        o["demod"]["ifnrAllowed"] = json!(false);
        o["fft"]["peakHold"] = json!(self.shared.hist.lock().unwrap().peak_hold);
        o.to_string()
    }

    fn apply_options(&self) {
        let i = self.inner.lock().unwrap();
        let o = &i.options;
        let mut e = self.shared.engine.lock().unwrap();
        e.control(&EngineCtl::Stereo(o["demod"]["stereo"].as_bool().unwrap_or(true)));
        let nb = o["demod"]["nb"].as_bool().unwrap_or(false);
        e.control(&EngineCtl::Blanker(nb, o["demod"]["nbLevel"].as_f64().map(|v| v as f32)));
        let loc = o["operator"]["location"].as_str().unwrap_or("");
        e.ft8.home = crate::ft8::message::grid_latlon(loc);
        let path = o["recorder"]["path"].as_str().unwrap_or("");
        if !path.is_empty() {
            e.rec.set_path(path);
        }
        drop(e);
        self.shared.hist.lock().unwrap().peak_hold = o["fft"]["peakHold"].as_bool().unwrap_or(false);
    }

    pub fn set_option(&self, key: &str, v: f64) {
        let mut i = self.inner.lock().unwrap();
        let Some((group, name)) = key.split_once('.') else { return };
        if !i.options[group].is_object() {
            return;
        }
        let val = match (group, name) {
            ("fft", "peakHold") | ("input", "dcBlocking") | ("input", "invertIQ") | ("demod", "nb") | ("demod", "stereo") => json!(v != 0.0),
            _ => json!(v),
        };
        i.options[group][name] = val;
        if group == "fft" {
            let o = &i.options["fft"];
            let mut d = self.shared.display.lock().unwrap();
            if name == "size" || name == "window" || name == "rate" {
                d.fft = DisplayFft::new(o["size"].as_u64().unwrap_or(4096) as usize, o["rate"].as_f64().unwrap_or(25.0) as f32, o["window"].as_i64().unwrap_or(2) as i32);
            }
            d.smoothing = o["smoothing"].as_f64().unwrap_or(0.4).clamp(0.0, 0.95) as f32;
            d.peak_decay = o["peakDecay"].as_f64().unwrap_or(10.0) as f32;
        }
        self.save(&mut i);
        drop(i);
        self.apply_options();
    }

    pub fn set_option_string(&self, key: &str, v: &str) {
        let mut i = self.inner.lock().unwrap();
        match key {
            "operator.location" => i.options["operator"]["location"] = json!(v),
            "operator.callsign" => i.options["operator"]["callsign"] = json!(v),
            "recorder.path" => i.options["recorder"]["path"] = json!(v),
            _ => return,
        }
        self.save(&mut i);
        drop(i);
        self.apply_options();
    }

    // ------------------------------------------------------------------ decoders

    pub fn with_engine<T>(&self, f: impl FnOnce(&mut Engine) -> T) -> T {
        f(&mut self.shared.engine.lock().unwrap())
    }

    pub fn ft8_json(&self) -> String {
        self.with_engine(|e| e.ft8.json())
    }
    pub fn ft8_set(&self, k: &str, v: f64) {
        self.with_engine(|e| e.ft8.set(k, v));
    }
    pub fn cw_json(&self) -> String {
        self.with_engine(|e| e.cw_json())
    }
    pub fn cw_set(&self, k: &str, v: f64) {
        self.with_engine(|e| e.cw.set(k, v));
    }
    pub fn rec_json(&self) -> String {
        self.with_engine(|e| e.rec.json())
    }
    pub fn rec_set(&self, k: &str, v: f64) {
        self.with_engine(|e| e.rec.set(k, v));
    }
    pub fn sstv_json(&self) -> String {
        self.with_engine(|e| e.sstv.json())
    }
    pub fn sstv_set(&self, k: &str, v: f64) {
        self.with_engine(|e| e.sstv.set(k, v));
    }
    /// [width, height, ARGB...]
    pub fn sstv_image(&self, which: i32) -> Vec<i32> {
        self.with_engine(|e| {
            let (px, w, h) = e.sstv.image(which);
            let mut v = Vec::with_capacity(px.len() + 2);
            v.push(w as i32);
            v.push(h as i32);
            v.extend(px.iter().map(|&p| p as i32));
            v
        })
    }
    pub fn rds_json(&self) -> String {
        self.with_engine(|e| e.rds_json())
    }

    /// The display data for the renderer
    pub fn history(&self) -> &Mutex<FftHistory> {
        &self.shared.hist
    }

    /// View (width shown, centre offset) as the UI set it
    pub fn view(&self) -> (f64, f64) {
        self.inner.lock().unwrap().view
    }
}

/// Hardware centre for a frequency: off the DC spike of wide receivers
pub fn plan_center(freq: f64, rate: f64, mode: Mode) -> f64 {
    if rate >= 1e6 && mode != Mode::Dab {
        freq - 250e3
    } else {
        freq
    }
}

/// The channel must fit in the clean part of the band, clear of the DC spike
pub fn needs_retune(freq: f64, center: f64, rate: f64, mode: Mode) -> bool {
    let d = (freq - center).abs();
    if mode == Mode::Dab {
        return d > 1.0;
    }
    let edge = rate * 0.45 - mode.half_band();
    let guard = if rate >= 1e6 { mode.half_band() + 5e3 } else { 0.0 };
    d > edge || d < guard
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole service on a recorded IQ file: start, tune, state, sound, display, stop
    #[test]
    fn plays_a_file() {
        let dir = std::env::temp_dir().join(format!("littlesdr-svc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 1 s of a 1 kHz AM tone at +20 kHz in a 192 kS/s file
        let path = dir.join("2026-11-20_12-00-00_baseband_7000000Hz_192kHz.wav");
        let rate = 192_000u32;
        let mut d = Vec::new();
        let n = rate;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + n * 4).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        for v in [16u32.to_le_bytes(), [1, 0, 2, 0], rate.to_le_bytes(), (rate * 4).to_le_bytes(), [4, 0, 16, 0]] {
            d.extend_from_slice(&v);
        }
        d.extend_from_slice(b"data");
        d.extend_from_slice(&(n * 4).to_le_bytes());
        for k in 0..n {
            let t = k as f64 / rate as f64;
            let a = 0.3 * (1.0 + 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin());
            let p = 2.0 * std::f64::consts::PI * 20_000.0 * t;
            d.extend_from_slice(&((a * p.cos() * 32767.0) as i16).to_le_bytes());
            d.extend_from_slice(&((a * p.sin() * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, d).unwrap();
        let svc = Service::init(dir.to_str().unwrap(), 2048);
        svc.select_source("Fichier IQ");
        svc.source_menu("file_path", false, 0, 0.0, path.to_str().unwrap());
        svc.set_mode(2);
        svc.tune(7_020_000.0);
        assert!(svc.start(), "start");
        std::thread::sleep(Duration::from_millis(700));
        let st = svc.state();
        assert_eq!((st[0], st[1], st[2], st[7]), (7_020_000.0, 7_000_000.0, 192_000.0, 1.0), "{st:?}");
        assert!(st[5] > -40.0, "level {}", st[5]);
        let mut buf = vec![0f32; 2 * 4800];
        let ring = svc.audio();
        std::thread::sleep(Duration::from_millis(300));
        ring.read(&mut buf, 2);
        let rms = (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt();
        assert!(rms > 0.01, "sound rms {rms}");
        assert!(svc.history().lock().unwrap().written > 5);
        assert!(svc.source_menu("", false, 0, 0.0, "").contains("file_path"));
        assert!(svc.stop());
        assert!(dir.join("littlesdr.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
