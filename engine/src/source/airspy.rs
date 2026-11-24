//! Airspy R2 / Mini (libairspy, BSD) and Airspy HF+ (libairspyhf, BSD or GPL-2), loaded when
//! the receiver opens: the engine builds and runs without them. The Android app ships both
//! (kit/jniLibs); on Linux they come from the distribution.
//!
//! On Android the device is opened from the USB descriptor the app got (`*_open_fd`).

use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use libloading::Library;
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

/// The first of these libraries that loads
pub(crate) fn load(names: &[&str]) -> Result<Library, String> {
    let mut last = String::new();
    for n in names {
        // SAFETY: loading a shared library runs its initialisers; these are the receivers'
        // own drivers
        match unsafe { Library::new(n) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("pilote {} introuvable ({last})", names[0]))
}

/// What the driver's sample callback needs: where blocks go, at which rate and centre
pub(crate) struct Feed {
    pub out: SyncSender<IqBlock>,
    pub rate: AtomicU32,
    pub center: AtomicU64,
    /// Blocks dropped because the DSP thread was behind
    pub dropped: AtomicU64,
}

impl Feed {
    pub fn new(out: SyncSender<IqBlock>, rate: u32, center: u64) -> Arc<Feed> {
        Arc::new(Feed { out, rate: AtomicU32::new(rate), center: AtomicU64::new(center), dropped: AtomicU64::new(0) })
    }

    /// From the driver's thread: never blocks the USB transfers
    pub fn push(&self, samples: Vec<C32>) {
        let block = IqBlock { samples, rate: self.rate.load(Relaxed) as f64, center: self.center.load(Relaxed) as f64, settle_ms: 0.0, time: None };
        if self.out.try_send(block).is_err() {
            self.dropped.fetch_add(1, Relaxed);
        }
    }
}

/// The supported rate closest to the one asked for
fn nearest(rates: &[u32], want: u32) -> u32 {
    rates.iter().copied().min_by_key(|r| (*r as i64 - want as i64).abs()).unwrap_or(want)
}

struct Dev(*mut c_void);
// The handle is only used from the control thread (the driver's own thread calls back into us)
unsafe impl Send for Dev {}

// ---------------------------------------------------------------------------------- Airspy

#[repr(C)]
struct AirspyTransfer {
    device: *mut c_void,
    ctx: *mut c_void,
    samples: *mut c_void,
    sample_count: c_int,
    dropped_samples: u64,
    sample_type: c_int,
}

type AirspyCb = unsafe extern "C" fn(*mut AirspyTransfer) -> c_int;

unsafe extern "C" fn airspy_samples(t: *mut AirspyTransfer) -> c_int {
    // SAFETY: the driver hands a valid transfer of `sample_count` float IQ pairs, and the
    // context is the Feed kept alive until streaming stops
    let t = &*t;
    let feed = &*(t.ctx as *const Feed);
    let n = t.sample_count.max(0) as usize;
    let s = std::slice::from_raw_parts(t.samples as *const f32, 2 * n);
    feed.push(s.chunks_exact(2).map(|p| C32::new(p[0], p[1])).collect());
    0
}

/// Airspy R2 / Mini: 2.5 / 10 MS/s (R2) or 3 / 6 MS/s (Mini), 24 MHz - 1.8 GHz
pub struct Airspy {
    /// Android: the USB descriptor the app opened
    pub fd: Option<i32>,
}

impl Source for Airspy {
    fn name(&self) -> String {
        "Airspy".into()
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let lib = load(if cfg!(target_os = "android") { &["libairspy.so"] } else { &["libairspy.so.0", "libairspy.so"] })?;
        let e = |n: &'static str| move |err: libloading::Error| format!("libairspy {n}: {err}");
        // SAFETY: symbols of libairspy with their C signatures (airspy.h); the library stays
        // loaded (`lib`) for as long as they are used
        unsafe {
            let open: unsafe extern "C" fn(*mut *mut c_void) -> c_int = *lib.get(b"airspy_open\0").map_err(e("open"))?;
            let open_fd: Option<unsafe extern "C" fn(*mut *mut c_void, c_int) -> c_int> = lib.get(b"airspy_open_fd\0").ok().map(|s| *s);
            let close: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspy_close\0").map_err(e("close"))?;
            let get_rates: unsafe extern "C" fn(*mut c_void, *mut u32, u32) -> c_int = *lib.get(b"airspy_get_samplerates\0").map_err(e("rates"))?;
            let set_rate: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"airspy_set_samplerate\0").map_err(e("rate"))?;
            let set_type: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = *lib.get(b"airspy_set_sample_type\0").map_err(e("type"))?;
            let set_freq: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"airspy_set_freq\0").map_err(e("freq"))?;
            let lna_agc: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspy_set_lna_agc\0").map_err(e("agc"))?;
            let mixer_agc: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspy_set_mixer_agc\0").map_err(e("agc"))?;
            let linearity: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspy_set_linearity_gain\0").map_err(e("gain"))?;
            let rf_bias: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspy_set_rf_bias\0").map_err(e("bias"))?;
            let start: unsafe extern "C" fn(*mut c_void, AirspyCb, *mut c_void) -> c_int = *lib.get(b"airspy_start_rx\0").map_err(e("start"))?;
            let stop: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspy_stop_rx\0").map_err(e("stop"))?;
            let streaming: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspy_is_streaming\0").map_err(e("streaming"))?;

            let mut raw = std::ptr::null_mut();
            let r = match (self.fd, open_fd) {
                (Some(fd), Some(f)) => f(&mut raw, fd),
                _ => open(&mut raw),
            };
            if r != 0 || raw.is_null() {
                return Err(if r == -5 { "aucun Airspy branché".to_string() } else { format!("impossible d'ouvrir l'Airspy (erreur {r} : libre, droits USB ?)") });
            }
            let dev = Dev(raw);
            let mut n = [0u32; 1];
            get_rates(dev.0, n.as_mut_ptr(), 0);
            let mut rates = vec![0u32; n[0].clamp(1, 16) as usize];
            get_rates(dev.0, rates.as_mut_ptr(), rates.len() as u32);
            rates.retain(|&r| r > 0);
            let rate = nearest(&rates, cfg.rate);
            set_type(dev.0, 0); // float32 IQ
            set_rate(dev.0, rate);
            set_freq(dev.0, cfg.center as u32);
            let gain = |g: Option<f32>| match g {
                Some(db) => {
                    lna_agc(dev.0, 0);
                    mixer_agc(dev.0, 0);
                    linearity(dev.0, db.round().clamp(0.0, 21.0) as u8);
                }
                None => {
                    lna_agc(dev.0, 1);
                    mixer_agc(dev.0, 1);
                }
            };
            gain(cfg.gain);
            rf_bias(dev.0, cfg.bias_tee as u8);

            let feed = Feed::new(out, rate, cfg.center);
            let ctx = Arc::as_ptr(&feed) as *mut c_void;
            if start(dev.0, airspy_samples, ctx) != 0 {
                close(dev.0);
                return Err("l'Airspy refuse de démarrer".into());
            }
            let result = loop {
                match ctl.recv_timeout(Duration::from_millis(300)) {
                    Ok(SourceCtl::Center(f)) => {
                        set_freq(dev.0, f as u32);
                        feed.center.store(f, Relaxed);
                    }
                    Ok(SourceCtl::Gain(g)) => gain(g),
                    Ok(SourceCtl::BiasTee(on)) => {
                        rf_bias(dev.0, on as u8);
                    }
                    Ok(SourceCtl::Rate(r)) => {
                        // The rate only changes while stopped
                        let r = nearest(&rates, r);
                        stop(dev.0);
                        set_rate(dev.0, r);
                        feed.rate.store(r, Relaxed);
                        start(dev.0, airspy_samples, ctx);
                    }
                    Ok(SourceCtl::Stop) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => {
                        if streaming(dev.0) == 0 {
                            break Err("l'Airspy s'est arrêté (débranché ?)".to_string());
                        }
                    }
                }
            };
            stop(dev.0);
            close(dev.0);
            drop(feed);
            result
        }
    }
}

// ---------------------------------------------------------------------------------- Airspy HF+

#[repr(C)]
struct HfTransfer {
    device: *mut c_void,
    ctx: *mut c_void,
    samples: *mut f32,
    sample_count: c_int,
    dropped_samples: u64,
}

type HfCb = unsafe extern "C" fn(*mut HfTransfer) -> c_int;

unsafe extern "C" fn hf_samples(t: *mut HfTransfer) -> c_int {
    // SAFETY: as for the Airspy: `sample_count` complex floats, Feed alive while streaming
    let t = &*t;
    let feed = &*(t.ctx as *const Feed);
    let n = t.sample_count.max(0) as usize;
    let s = std::slice::from_raw_parts(t.samples, 2 * n);
    feed.push(s.chunks_exact(2).map(|p| C32::new(p[0], p[1])).collect());
    0
}

/// Airspy HF+ (Dual Port / Discovery): 192 - 912 kS/s, 9 kHz - 31 MHz and 60 - 260 MHz.
/// Gain is the attenuator (0 to 48 dB in 6 dB steps), or the receiver's AGC
pub struct AirspyHf {
    pub fd: Option<i32>,
}

impl Source for AirspyHf {
    fn name(&self) -> String {
        "Airspy HF+".into()
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let lib = load(if cfg!(target_os = "android") { &["libairspyhf.so"] } else { &["libairspyhf.so.0", "libairspyhf.so"] })?;
        let e = |n: &'static str| move |err: libloading::Error| format!("libairspyhf {n}: {err}");
        // SAFETY: symbols of libairspyhf with their C signatures (airspyhf.h)
        unsafe {
            let open: unsafe extern "C" fn(*mut *mut c_void) -> c_int = *lib.get(b"airspyhf_open\0").map_err(e("open"))?;
            let open_fd: Option<unsafe extern "C" fn(*mut *mut c_void, c_int) -> c_int> = lib.get(b"airspyhf_open_fd\0").ok().map(|s| *s);
            let close: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspyhf_close\0").map_err(e("close"))?;
            let get_rates: unsafe extern "C" fn(*mut c_void, *mut u32, u32) -> c_int = *lib.get(b"airspyhf_get_samplerates\0").map_err(e("rates"))?;
            let set_rate: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"airspyhf_set_samplerate\0").map_err(e("rate"))?;
            let set_freq: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"airspyhf_set_freq\0").map_err(e("freq"))?;
            let agc: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspyhf_set_hf_agc\0").map_err(e("agc"))?;
            let att: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspyhf_set_hf_att\0").map_err(e("att"))?;
            let lna: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"airspyhf_set_hf_lna\0").map_err(e("lna"))?;
            let start: unsafe extern "C" fn(*mut c_void, HfCb, *mut c_void) -> c_int = *lib.get(b"airspyhf_start\0").map_err(e("start"))?;
            let stop: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspyhf_stop\0").map_err(e("stop"))?;
            let streaming: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"airspyhf_is_streaming\0").map_err(e("streaming"))?;

            let mut raw = std::ptr::null_mut();
            let r = match (self.fd, open_fd) {
                (Some(fd), Some(f)) => f(&mut raw, fd),
                _ => open(&mut raw),
            };
            if r != 0 || raw.is_null() {
                return Err(format!("impossible d'ouvrir l'Airspy HF+ (erreur {r} : branché, libre, droits USB ?)"));
            }
            let dev = Dev(raw);
            let mut n = [0u32; 1];
            get_rates(dev.0, n.as_mut_ptr(), 0);
            let mut rates = vec![0u32; n[0].clamp(1, 16) as usize];
            get_rates(dev.0, rates.as_mut_ptr(), rates.len() as u32);
            rates.retain(|&r| r > 0);
            let rate = nearest(&rates, cfg.rate);
            set_rate(dev.0, rate);
            set_freq(dev.0, cfg.center as u32);
            lna(dev.0, 1);
            let gain = |g: Option<f32>| match g {
                // "Gain" = less attenuation: 48 dB -> no attenuation
                Some(db) => {
                    agc(dev.0, 0);
                    att(dev.0, ((48.0 - db.clamp(0.0, 48.0)) / 6.0).round() as u8);
                }
                None => {
                    att(dev.0, 0);
                    agc(dev.0, 1);
                }
            };
            gain(cfg.gain);

            let feed = Feed::new(out, rate, cfg.center);
            let ctx = Arc::as_ptr(&feed) as *mut c_void;
            if start(dev.0, hf_samples, ctx) != 0 {
                close(dev.0);
                return Err("l'Airspy HF+ refuse de démarrer".into());
            }
            let result = loop {
                match ctl.recv_timeout(Duration::from_millis(300)) {
                    Ok(SourceCtl::Center(f)) => {
                        set_freq(dev.0, f as u32);
                        feed.center.store(f, Relaxed);
                    }
                    Ok(SourceCtl::Gain(g)) => gain(g),
                    Ok(SourceCtl::Rate(r)) => {
                        let r = nearest(&rates, r);
                        stop(dev.0);
                        set_rate(dev.0, r);
                        feed.rate.store(r, Relaxed);
                        start(dev.0, hf_samples, ctx);
                    }
                    Ok(SourceCtl::Stop) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => {
                        if streaming(dev.0) == 0 {
                            break Err("l'Airspy HF+ s'est arrêté (débranché ?)".to_string());
                        }
                    }
                }
            };
            stop(dev.0);
            close(dev.0);
            drop(feed);
            result
        }
    }
}
