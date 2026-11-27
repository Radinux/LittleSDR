//! SoapySDR: every receiver it has a module for (LimeSDR, bladeRF, USRP, PlutoSDR, RX888 /
//! SDDC, RSP, Red Pitaya, SoapyRemote...), on Linux, Windows and macOS. libSoapySDR is loaded
//! when the source opens (the engine builds and runs without it), like the receiver drivers.
//!
//! A device is named by SoapySDR arguments ("driver=lime", "driver=rtlsdr,serial=..."); empty
//! = the first one found. The C API's setupStream changed with ABI 0.8: both are handled.

use super::airspy::{load, Feed};
use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use libloading::Library;
use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const RX: c_int = 1;
const TIMEOUT: c_int = -1;
const OVERFLOW: c_int = -4;

/// Rates offered (the device takes the closest it has)
pub const RATES: [f64; 10] = [250e3, 1e6, 2e6, 2.4e6, 3.2e6, 5e6, 8e6, 10e6, 16e6, 20e6];

fn library() -> Result<Library, String> {
    load(if cfg!(windows) {
        &["SoapySDR.dll", "libSoapySDR.dll"]
    } else if cfg!(target_os = "macos") {
        &["libSoapySDR.0.8.dylib", "libSoapySDR.dylib"]
    } else {
        &["libSoapySDR.so.0.9", "libSoapySDR.so.0.8", "libSoapySDR.so.0.7", "libSoapySDR.so"]
    })
}

#[repr(C)]
struct Kwargs {
    size: usize,
    keys: *mut *mut c_char,
    vals: *mut *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    minimum: f64,
    maximum: f64,
    step: f64,
}

/// A device found by enumeration: what to show, the arguments that open it
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub label: String,
    pub args: String,
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Every device SoapySDR sees (its modules probe USB, PCIe, the network: can take seconds)
fn enumerate() -> Result<Vec<Found>, String> {
    let lib = library()?;
    // SAFETY: SoapySDR's C API (Device.h, Types.h); the list is freed by the library
    unsafe {
        let en: unsafe extern "C" fn(*const c_char, *mut usize) -> *mut Kwargs = *lib.get(b"SoapySDRDevice_enumerateStrArgs\0").map_err(|e| e.to_string())?;
        let clear: unsafe extern "C" fn(*mut Kwargs, usize) = *lib.get(b"SoapySDRKwargsList_clear\0").map_err(|e| e.to_string())?;
        let mut n = 0usize;
        let list = en(c"".as_ptr(), &mut n);
        let mut found = Vec::new();
        for i in 0..n {
            let k = &*list.add(i);
            let mut label = String::new();
            let mut args = Vec::new();
            for j in 0..k.size {
                let (key, val) = (cstr(*k.keys.add(j)), cstr(*k.vals.add(j)));
                match key.as_str() {
                    "label" => label = val,
                    // What identifies the device; the rest (product, manufacturer...) is noise
                    "driver" | "serial" | "device_id" | "uri" | "addr" | "remote" | "soapy" => args.push(format!("{key}={val}")),
                    _ => {}
                }
            }
            let args = args.join(",");
            found.push(Found { label: if label.is_empty() { args.clone() } else { label }, args });
        }
        if !list.is_null() {
            clear(list, n);
        }
        Ok(found)
    }
}

struct Scan {
    at: Option<Instant>,
    running: bool,
    found: Vec<Found>,
    error: Option<String>,
}

fn scan() -> &'static Mutex<Scan> {
    static S: OnceLock<Mutex<Scan>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Scan { at: None, running: false, found: Vec::new(), error: None }))
}

/// The devices last found, and an error if SoapySDR could not be used. Starts a new search in
/// the background when the last one is older than 10 s (the form never waits for it).
pub fn devices() -> (Vec<Found>, Option<String>) {
    let mut s = scan().lock().unwrap();
    if !s.running && s.at.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
        s.running = true;
        std::thread::spawn(|| {
            let r = enumerate();
            let mut s = scan().lock().unwrap();
            s.running = false;
            s.at = Some(Instant::now());
            match r {
                Ok(f) => {
                    s.found = f;
                    s.error = None;
                }
                Err(e) => s.error = Some(e),
            }
        });
    }
    (s.found.clone(), s.error.clone())
}

/// The supported rate closest to the one asked for, from the device's ranges
fn pick_rate(ranges: &[Range], want: f64) -> f64 {
    ranges
        .iter()
        .map(|r| {
            let mut v = want.clamp(r.minimum, r.maximum);
            if r.step > 0.0 {
                v = (r.minimum + ((v - r.minimum) / r.step).round() * r.step).min(r.maximum);
            }
            v
        })
        .min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()))
        .unwrap_or(want)
}

#[derive(Default, Clone)]
pub struct SoapyStatus {
    pub message: String,
    /// Gain range of the open device (dB)
    pub gain: Option<(f64, f64)>,
}

pub struct Soapy {
    pub args: String,
    pub status: Arc<Mutex<SoapyStatus>>,
}

struct Dev(*mut c_void);
// The handle is only used from the source's thread
unsafe impl Send for Dev {}

impl Source for Soapy {
    fn name(&self) -> String {
        format!("SoapySDR {}", self.args)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let lib = library()?;
        let e = |n: &'static str| move |err: libloading::Error| format!("SoapySDR {n} : {err}");
        // SAFETY: SoapySDR's C API with its C signatures (Device.h); the library stays loaded
        // (`lib`) for as long as they are used, the device and stream are freed below
        unsafe {
            let abi: unsafe extern "C" fn() -> *const c_char = *lib.get(b"SoapySDR_getABIVersion\0").map_err(e("version"))?;
            let last_error: unsafe extern "C" fn() -> *const c_char = *lib.get(b"SoapySDRDevice_lastError\0").map_err(e("erreur"))?;
            let make: unsafe extern "C" fn(*const c_char) -> *mut c_void = *lib.get(b"SoapySDRDevice_makeStrArgs\0").map_err(e("make"))?;
            let unmake: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"SoapySDRDevice_unmake\0").map_err(e("unmake"))?;
            let free: unsafe extern "C" fn(*mut c_void) = *lib.get(b"SoapySDR_free\0").map_err(e("free"))?;
            let rate_ranges: unsafe extern "C" fn(*mut c_void, c_int, usize, *mut usize) -> *mut Range = *lib.get(b"SoapySDRDevice_getSampleRateRange\0").map_err(e("rates"))?;
            let set_rate: unsafe extern "C" fn(*mut c_void, c_int, usize, f64) -> c_int = *lib.get(b"SoapySDRDevice_setSampleRate\0").map_err(e("rate"))?;
            let set_freq: unsafe extern "C" fn(*mut c_void, c_int, usize, f64, *const c_void) -> c_int = *lib.get(b"SoapySDRDevice_setFrequency\0").map_err(e("freq"))?;
            let has_agc: unsafe extern "C" fn(*mut c_void, c_int, usize) -> bool = *lib.get(b"SoapySDRDevice_hasGainMode\0").map_err(e("agc"))?;
            let set_agc: unsafe extern "C" fn(*mut c_void, c_int, usize, bool) -> c_int = *lib.get(b"SoapySDRDevice_setGainMode\0").map_err(e("agc"))?;
            let set_gain: unsafe extern "C" fn(*mut c_void, c_int, usize, f64) -> c_int = *lib.get(b"SoapySDRDevice_setGain\0").map_err(e("gain"))?;
            let gain_range: unsafe extern "C" fn(*mut c_void, c_int, usize) -> Range = *lib.get(b"SoapySDRDevice_getGainRange\0").map_err(e("gain"))?;
            let write_setting: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int = *lib.get(b"SoapySDRDevice_writeSetting\0").map_err(e("setting"))?;
            let close_stream: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int = *lib.get(b"SoapySDRDevice_closeStream\0").map_err(e("stream"))?;
            let mtu: unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize = *lib.get(b"SoapySDRDevice_getStreamMTU\0").map_err(e("stream"))?;
            let activate: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, i64, usize) -> c_int = *lib.get(b"SoapySDRDevice_activateStream\0").map_err(e("stream"))?;
            let deactivate: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, i64) -> c_int = *lib.get(b"SoapySDRDevice_deactivateStream\0").map_err(e("stream"))?;
            let read: unsafe extern "C" fn(*mut c_void, *mut c_void, *const *mut c_void, usize, *mut c_int, *mut i64, c_long) -> c_int = *lib.get(b"SoapySDRDevice_readStream\0").map_err(e("stream"))?;
            // ABI 0.8 and up return the stream; 0.7 writes it through a pointer
            let old_abi = cstr(abi()).starts_with("0.7");
            let setup = lib.get::<*const c_void>(b"SoapySDRDevice_setupStream\0").map_err(e("stream"))?;
            let setup_ptr: *const c_void = *setup;

            let args = CString::new(self.args.trim()).map_err(|_| "arguments SoapySDR invalides".to_string())?;
            let raw = make(args.as_ptr());
            if raw.is_null() {
                let why = cstr(last_error());
                return Err(if why.is_empty() { "aucun appareil SoapySDR trouvé".into() } else { format!("SoapySDR : {why}") });
            }
            let dev = Dev(raw);

            let mut n = 0usize;
            let rr = rate_ranges(dev.0, RX, 0, &mut n);
            let ranges: Vec<Range> = if rr.is_null() { Vec::new() } else { std::slice::from_raw_parts(rr, n).to_vec() };
            if !rr.is_null() {
                free(rr as *mut c_void);
            }
            let rate = |want: u32| {
                let r = if ranges.is_empty() { want as f64 } else { pick_rate(&ranges, want as f64) };
                set_rate(dev.0, RX, 0, r);
                r
            };
            let mut cur = rate(cfg.rate.max(1));
            set_freq(dev.0, RX, 0, cfg.center as f64, std::ptr::null());
            let g = gain_range(dev.0, RX, 0);
            let agc_ok = has_agc(dev.0, RX, 0);
            self.status.lock().unwrap().gain = Some((g.minimum, g.maximum));
            let gain = |v: Option<f32>| match v {
                Some(db) => {
                    if agc_ok {
                        set_agc(dev.0, RX, 0, false);
                    }
                    set_gain(dev.0, RX, 0, (db as f64).clamp(g.minimum, g.maximum.max(g.minimum)));
                }
                None if agc_ok => {
                    set_agc(dev.0, RX, 0, true);
                }
                // No AGC: the middle of the range
                None => {
                    set_gain(dev.0, RX, 0, (g.minimum + g.maximum) / 2.0);
                }
            };
            gain(cfg.gain);
            // The usual name of the bias-tee setting in SoapySDR modules (ignored by the others)
            let bias = |on: bool| {
                write_setting(dev.0, c"biastee".as_ptr(), if on { c"true".as_ptr() } else { c"false".as_ptr() });
            };
            if cfg.bias_tee {
                bias(true);
            }

            let chans = [0usize];
            let mut stream: *mut c_void = std::ptr::null_mut();
            let ok = if old_abi {
                let f: unsafe extern "C" fn(*mut c_void, *mut *mut c_void, c_int, *const c_char, *const usize, usize, *const c_void) -> c_int = std::mem::transmute(setup_ptr);
                f(dev.0, &mut stream, RX, c"CF32".as_ptr(), chans.as_ptr(), 1, std::ptr::null()) == 0
            } else {
                let f: unsafe extern "C" fn(*mut c_void, c_int, *const c_char, *const usize, usize, *const c_void) -> *mut c_void = std::mem::transmute(setup_ptr);
                stream = f(dev.0, RX, c"CF32".as_ptr(), chans.as_ptr(), 1, std::ptr::null());
                !stream.is_null()
            };
            if !ok || stream.is_null() {
                let why = cstr(last_error());
                unmake(dev.0);
                return Err(format!("SoapySDR : flux impossible ({why})"));
            }
            if activate(dev.0, stream, 0, 0, 0) != 0 {
                close_stream(dev.0, stream);
                unmake(dev.0);
                return Err("SoapySDR : l'appareil refuse de démarrer".into());
            }
            self.status.lock().unwrap().message = format!("{:.3} MS/s · gain {:.0} à {:.0} dB", cur / 1e6, g.minimum, g.maximum);

            let feed = Feed::new(out, cur as u32, cfg.center);
            let mut buf = vec![C32::new(0.0, 0.0); mtu(dev.0, stream).clamp(4096, 1 << 18)];
            let mut errors = 0u32;
            let result = loop {
                match ctl.try_recv() {
                    Ok(SourceCtl::Center(f)) => {
                        set_freq(dev.0, RX, 0, f as f64, std::ptr::null());
                        feed.center.store(f, Relaxed);
                    }
                    Ok(SourceCtl::Gain(v)) => gain(v),
                    Ok(SourceCtl::BiasTee(on)) => bias(on),
                    Ok(SourceCtl::Rate(r)) => {
                        deactivate(dev.0, stream, 0, 0);
                        cur = rate(r);
                        feed.rate.store(cur as u32, Relaxed);
                        activate(dev.0, stream, 0, 0, 0);
                    }
                    Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => break Ok(()),
                    Ok(_) | Err(TryRecvError::Empty) => {}
                }
                let ptr = buf.as_mut_ptr() as *mut c_void;
                let (mut flags, mut t) = (0 as c_int, 0i64);
                let r = read(dev.0, stream, &ptr, buf.len(), &mut flags, &mut t, 100_000);
                if r > 0 {
                    errors = 0;
                    feed.push(buf[..r as usize].to_vec());
                } else if r == OVERFLOW {
                    feed.dropped.fetch_add(1, Relaxed);
                } else if r != TIMEOUT && r != 0 {
                    errors += 1;
                    if errors > 50 {
                        break Err(format!("SoapySDR : lecture impossible (erreur {r}, débranché ?)"));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            };
            deactivate(dev.0, stream, 0, 0);
            close_stream(dev.0, stream);
            if cfg.bias_tee {
                bias(false);
            }
            unmake(dev.0);
            drop(feed);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_from_ranges() {
        // A continuous range and a list of fixed rates (min = max)
        let r = [Range { minimum: 1e6, maximum: 10e6, step: 0.0 }];
        assert_eq!(pick_rate(&r, 2.4e6), 2.4e6);
        assert_eq!(pick_rate(&r, 20e6), 10e6);
        let fixed = [Range { minimum: 2.5e6, maximum: 2.5e6, step: 0.0 }, Range { minimum: 10e6, maximum: 10e6, step: 0.0 }];
        assert_eq!(pick_rate(&fixed, 8e6), 10e6);
        let stepped = [Range { minimum: 0.0, maximum: 10e6, step: 1e6 }];
        assert_eq!(pick_rate(&stepped, 2.4e6), 2e6);
    }
}
