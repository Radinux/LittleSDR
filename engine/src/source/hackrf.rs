//! HackRF One (libhackrf, BSD-3-Clause), loaded when the receiver opens: the engine builds and runs
//! without it. The Android app ships it (kit/jniLibs, SDR++'s fork with `hackrf_open_by_fd`);
//! on Linux it comes from the distribution.
//!
//! 1 MHz - 6 GHz, 2 - 20 MS/s of 8-bit IQ. Gain is spread over the LNA (IF, 0 - 40 dB in 8 dB
//! steps) and the VGA (baseband, 0 - 62 dB in 2 dB steps); the RF preamp (+14 dB) is separate.

use super::airspy::{load, Feed};
use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::ffi::{c_int, c_void};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

/// Total gain range of the slider (LNA 40 + VGA 62)
pub const MAX_GAIN: f32 = 102.0;

#[repr(C)]
struct Transfer {
    device: *mut c_void,
    buffer: *mut u8,
    buffer_length: c_int,
    valid_length: c_int,
    rx_ctx: *mut c_void,
    tx_ctx: *mut c_void,
}

type RxCb = unsafe extern "C" fn(*mut Transfer) -> c_int;

unsafe extern "C" fn rx_samples(t: *mut Transfer) -> c_int {
    // SAFETY: the driver hands a transfer whose `valid_length` bytes are interleaved signed
    // 8-bit I/Q, and the context is the Feed kept alive until streaming stops
    let t = &*t;
    let feed = &*(t.rx_ctx as *const Feed);
    let n = t.valid_length.max(0) as usize;
    let s = std::slice::from_raw_parts(t.buffer as *const i8, n);
    feed.push(s.chunks_exact(2).map(|p| C32::new(p[0] as f32 / 128.0, p[1] as f32 / 128.0)).collect());
    0
}

/// LNA and VGA settings for a total gain: shared in proportion (the LNA gets about 40 %),
/// on their steps
pub fn split_gain(db: f32) -> (u32, u32) {
    let g = db.clamp(0.0, MAX_GAIN);
    let lna = ((g * 40.0 / MAX_GAIN / 8.0).round() * 8.0).clamp(0.0, 40.0);
    let vga = (((g - lna) / 2.0).round() * 2.0).clamp(0.0, 62.0);
    (lna as u32, vga as u32)
}

/// Gain when none is given (the HackRF has no AGC): LNA 16, VGA 20
const DEFAULT_GAIN: f32 = 36.0;

pub struct HackRf {
    /// Android: the USB descriptor the app opened
    pub fd: Option<i32>,
    /// RF preamp (+14 dB) at start
    pub amp: bool,
}

struct Dev(*mut c_void);
// The handle is only used from the control thread (the driver's own thread calls back into us)
unsafe impl Send for Dev {}

impl Source for HackRf {
    fn name(&self) -> String {
        "HackRF".into()
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let lib = load(if cfg!(target_os = "android") { &["libhackrf.so"] } else { &["libhackrf.so.0", "libhackrf.so"] })?;
        let e = |n: &'static str| move |err: libloading::Error| format!("libhackrf {n}: {err}");
        // SAFETY: symbols of libhackrf with their C signatures (hackrf.h); the library stays
        // loaded (`lib`) for as long as they are used
        unsafe {
            let init: unsafe extern "C" fn() -> c_int = *lib.get(b"hackrf_init\0").map_err(e("init"))?;
            let exit: unsafe extern "C" fn() -> c_int = *lib.get(b"hackrf_exit\0").map_err(e("exit"))?;
            let open: unsafe extern "C" fn(*mut *mut c_void) -> c_int = *lib.get(b"hackrf_open\0").map_err(e("open"))?;
            let open_fd: Option<unsafe extern "C" fn(c_int, *mut *mut c_void) -> c_int> = lib.get(b"hackrf_open_by_fd\0").ok().map(|s| *s);
            let close: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"hackrf_close\0").map_err(e("close"))?;
            let set_rate: unsafe extern "C" fn(*mut c_void, f64) -> c_int = *lib.get(b"hackrf_set_sample_rate\0").map_err(e("rate"))?;
            let filter_bw: unsafe extern "C" fn(u32) -> u32 = *lib.get(b"hackrf_compute_baseband_filter_bw\0").map_err(e("bw"))?;
            let set_filter: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"hackrf_set_baseband_filter_bandwidth\0").map_err(e("filter"))?;
            let set_freq: unsafe extern "C" fn(*mut c_void, u64) -> c_int = *lib.get(b"hackrf_set_freq\0").map_err(e("freq"))?;
            let set_lna: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"hackrf_set_lna_gain\0").map_err(e("lna"))?;
            let set_vga: unsafe extern "C" fn(*mut c_void, u32) -> c_int = *lib.get(b"hackrf_set_vga_gain\0").map_err(e("vga"))?;
            let set_amp: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"hackrf_set_amp_enable\0").map_err(e("amp"))?;
            let set_antenna: unsafe extern "C" fn(*mut c_void, u8) -> c_int = *lib.get(b"hackrf_set_antenna_enable\0").map_err(e("antenna"))?;
            let start: unsafe extern "C" fn(*mut c_void, RxCb, *mut c_void) -> c_int = *lib.get(b"hackrf_start_rx\0").map_err(e("start"))?;
            let stop: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"hackrf_stop_rx\0").map_err(e("stop"))?;
            let streaming: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"hackrf_is_streaming\0").map_err(e("streaming"))?;

            if init() != 0 {
                return Err("libhackrf ne s'initialise pas (libusb)".into());
            }
            let mut raw = std::ptr::null_mut();
            let r = match (self.fd, open_fd) {
                (Some(fd), Some(f)) => f(fd, &mut raw),
                _ => open(&mut raw),
            };
            if r != 0 || raw.is_null() {
                exit();
                // HACKRF_ERROR_NOT_FOUND = -5
                return Err(if r == -5 { "aucun HackRF branché".to_string() } else { format!("impossible d'ouvrir le HackRF (erreur {r} : libre, droits USB ?)") });
            }
            let dev = Dev(raw);
            let rate = |r: u32| {
                let r = r.clamp(2_000_000, 20_000_000);
                set_rate(dev.0, r as f64);
                // The baseband filter just under the rate (the driver's closest step)
                set_filter(dev.0, filter_bw((r as f32 * 0.75) as u32));
                r
            };
            let mut cur_rate = rate(cfg.rate);
            set_freq(dev.0, cfg.center);
            let gain = |g: Option<f32>| {
                let (lna, vga) = split_gain(g.unwrap_or(DEFAULT_GAIN));
                set_lna(dev.0, lna);
                set_vga(dev.0, vga);
            };
            gain(cfg.gain);
            set_amp(dev.0, self.amp as u8);
            set_antenna(dev.0, cfg.bias_tee as u8);

            let feed = Feed::new(out, cur_rate, cfg.center);
            let ctx = Arc::as_ptr(&feed) as *mut c_void;
            if start(dev.0, rx_samples, ctx) != 0 {
                close(dev.0);
                exit();
                return Err("le HackRF refuse de démarrer".into());
            }
            let result = loop {
                match ctl.recv_timeout(Duration::from_millis(300)) {
                    Ok(SourceCtl::Center(f)) => {
                        set_freq(dev.0, f);
                        feed.center.store(f, Relaxed);
                    }
                    Ok(SourceCtl::Gain(g)) => gain(g),
                    Ok(SourceCtl::Amp(on)) => {
                        set_amp(dev.0, on as u8);
                    }
                    Ok(SourceCtl::BiasTee(on)) => {
                        set_antenna(dev.0, on as u8);
                    }
                    Ok(SourceCtl::Rate(r)) if r.clamp(2_000_000, 20_000_000) != cur_rate => {
                        stop(dev.0);
                        cur_rate = rate(r);
                        feed.rate.store(cur_rate, Relaxed);
                        start(dev.0, rx_samples, ctx);
                    }
                    Ok(SourceCtl::Stop) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => {
                        // HACKRF_TRUE = 1
                        if streaming(dev.0) != 1 {
                            break Err("le HackRF s'est arrêté (débranché ?)".to_string());
                        }
                    }
                }
            };
            stop(dev.0);
            set_antenna(dev.0, 0);
            close(dev.0);
            exit();
            drop(feed);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The total is shared on the chips' steps, within their ranges
    #[test]
    fn gain_split() {
        assert_eq!(split_gain(0.0), (0, 0));
        assert_eq!(split_gain(MAX_GAIN), (40, 62));
        assert_eq!(split_gain(36.0), (16, 20));
        for g in 0..=102 {
            let (l, v) = split_gain(g as f32);
            assert!(l % 8 == 0 && l <= 40 && v % 2 == 0 && v <= 62);
            assert!((l + v) as i32 - g <= 2 && g - (l + v) as i32 <= 2, "{g} -> {l}+{v}");
        }
    }
}
