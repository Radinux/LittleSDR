//! RTL-SDR over USB, through the system librtlsdr (rtl-sdr-blog fork handles the Blog V4).
//!
//! Asynchronous reads: a few USB transfers are always queued, so no sample is lost between two
//! reads (synchronous reads dropped ~2 % of the stream: the dongle's FIFO overflows in the gap).
//! Settings are applied from the control thread while the stream runs, like rtl_tcp does; the
//! stream is never stopped. The transfers already queued when the rate changes hold old-rate
//! samples: the first block after the switch tells the engine how long to mute (≤ 4 buffers).

use super::{u8_to_iq, IqBlock, Source, SourceConfig, SourceCtl};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

type AsyncCb = extern "C" fn(buf: *mut u8, len: u32, ctx: *mut c_void);

#[link(name = "rtlsdr")]
extern "C" {
    fn rtlsdr_get_device_count() -> u32;
    fn rtlsdr_get_device_name(index: u32) -> *const c_char;
    fn rtlsdr_open(dev: *mut *mut c_void, index: u32) -> c_int;
    fn rtlsdr_close(dev: *mut c_void) -> c_int;
    fn rtlsdr_set_center_freq(dev: *mut c_void, freq: u32) -> c_int;
    fn rtlsdr_set_sample_rate(dev: *mut c_void, rate: u32) -> c_int;
    fn rtlsdr_set_tuner_gain_mode(dev: *mut c_void, manual: c_int) -> c_int;
    fn rtlsdr_set_tuner_gain(dev: *mut c_void, gain: c_int) -> c_int;
    fn rtlsdr_get_tuner_gains(dev: *mut c_void, gains: *mut c_int) -> c_int;
    fn rtlsdr_set_bias_tee(dev: *mut c_void, on: c_int) -> c_int;
    fn rtlsdr_set_direct_sampling(dev: *mut c_void, on: c_int) -> c_int;
    fn rtlsdr_reset_buffer(dev: *mut c_void) -> c_int;
    fn rtlsdr_read_async(dev: *mut c_void, cb: AsyncCb, ctx: *mut c_void, buf_num: u32, buf_len: u32) -> c_int;
    fn rtlsdr_cancel_async(dev: *mut c_void) -> c_int;
}

/// Queued USB transfers and their size: 4 x 16 KiB = 14 ms at 2.4 MS/s in flight
const BUF_NUM: u32 = 4;
const BUF_LEN: u32 = 16 * 1024;

pub struct RtlSdr {
    pub index: u32,
}

pub fn list() -> Vec<String> {
    unsafe {
        (0..rtlsdr_get_device_count())
            .map(|i| CStr::from_ptr(rtlsdr_get_device_name(i)).to_string_lossy().into_owned())
            .collect()
    }
}

/// Device handle, shared by the control thread and the USB callback thread.
#[derive(Clone, Copy)]
struct Dev(*mut c_void);
unsafe impl Send for Dev {}
unsafe impl Sync for Dev {}

impl Dev {
    fn set_gain(self, gain: Option<f32>) {
        unsafe {
            match gain {
                None => {
                    rtlsdr_set_tuner_gain_mode(self.0, 0);
                }
                Some(db) => {
                    // Nearest supported gain (tenths of dB)
                    let mut gains = [0 as c_int; 64];
                    let n = rtlsdr_get_tuner_gains(self.0, gains.as_mut_ptr()).max(0) as usize;
                    let want = (db * 10.0) as c_int;
                    let g = gains[..n].iter().copied().min_by_key(|g| (g - want).abs()).unwrap_or(want);
                    rtlsdr_set_tuner_gain_mode(self.0, 1);
                    rtlsdr_set_tuner_gain(self.0, g);
                }
            }
        }
    }
}

/// State read by the USB callback
struct Shared {
    dev: Dev,
    out: SyncSender<IqBlock>,
    rate: AtomicU32,
    center: AtomicU64,
    /// ms of signal to mute, as f32 bits; taken by the next block
    settle: AtomicU32,
    samples: AtomicU64,
}

extern "C" fn on_samples(buf: *mut u8, len: u32, ctx: *mut c_void) {
    let sh = unsafe { &*(ctx as *const Shared) };
    let bytes = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    u8_to_iq(bytes, &mut samples);
    sh.samples.fetch_add(samples.len() as u64, Ordering::Relaxed);
    let block = IqBlock {
        samples,
        rate: sh.rate.load(Ordering::Acquire) as f64,
        center: sh.center.load(Ordering::Relaxed) as f64,
        settle_ms: f32::from_bits(sh.settle.swap(0, Ordering::AcqRel)) as f64,
        time: None,
    };
    if sh.out.send(block).is_err() {
        unsafe { rtlsdr_cancel_async(sh.dev.0) }; // engine gone
    }
}

impl Source for RtlSdr {
    fn name(&self) -> String {
        format!("rtlsdr#{}", self.index)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        if unsafe { rtlsdr_open(&mut raw, self.index) } != 0 {
            return Err(format!("cannot open RTL-SDR #{} (busy or permission?)", self.index));
        }
        let dev = Dev(raw);
        unsafe {
            rtlsdr_set_sample_rate(dev.0, cfg.rate);
            if cfg.direct_sampling > 0 {
                rtlsdr_set_direct_sampling(dev.0, cfg.direct_sampling as c_int);
            }
            rtlsdr_set_center_freq(dev.0, cfg.center as u32);
            if cfg.bias_tee {
                rtlsdr_set_bias_tee(dev.0, 1);
            }
        }
        let mut bias_on = cfg.bias_tee;
        dev.set_gain(cfg.gain);
        unsafe { rtlsdr_reset_buffer(dev.0) };

        let shared = Arc::new(Shared {
            dev,
            out,
            rate: AtomicU32::new(cfg.rate),
            center: AtomicU64::new(cfg.center),
            settle: AtomicU32::new(0),
            samples: AtomicU64::new(0),
        });
        let sh = shared.clone();
        let reader = std::thread::Builder::new()
            .name("rtlsdr usb".into())
            .spawn(move || {
                let ctx = Arc::as_ptr(&sh) as *mut c_void;
                let r = unsafe { rtlsdr_read_async(sh.dev.0, on_samples, ctx, BUF_NUM, BUF_LEN) };
                if r != 0 {
                    eprintln!("[source] rtlsdr_read_async ended with {r}");
                }
            })
            .map_err(|e| e.to_string())?;

        // Control thread: settings while the transfers keep flowing
        let started = Instant::now();
        let mut last_check = (Instant::now(), 0u64);
        loop {
            match ctl.recv_timeout(Duration::from_millis(200)) {
                Ok(SourceCtl::Rate(r)) if r != shared.rate.load(Ordering::Relaxed) => {
                    unsafe { rtlsdr_set_sample_rate(dev.0, r) };
                    shared.rate.store(r, Ordering::Release);
                    // The transfers already queued were sampled at the old rate
                    let in_flight_ms = (BUF_NUM * BUF_LEN) as f32 / 2.0 / r as f32 * 1000.0;
                    shared.settle.store((in_flight_ms + 2.0).to_bits(), Ordering::Release);
                    last_check = (Instant::now(), shared.samples.load(Ordering::Relaxed));
                }
                Ok(SourceCtl::Rate(_)) => {}
                Ok(SourceCtl::Center(f)) => {
                    unsafe { rtlsdr_set_center_freq(dev.0, f as u32) };
                    shared.center.store(f, Ordering::Relaxed);
                }
                Ok(SourceCtl::Gain(g)) => dev.set_gain(g),
                Ok(SourceCtl::BiasTee(on)) => {
                    unsafe { rtlsdr_set_bias_tee(dev.0, on as c_int) };
                    bias_on = on;
                }
                Ok(SourceCtl::DirectSampling(m)) => unsafe {
                    rtlsdr_set_direct_sampling(dev.0, m as c_int);
                    // The tuner is re-enabled / bypassed: the frequency must be set again
                    rtlsdr_set_center_freq(dev.0, shared.center.load(Ordering::Relaxed) as u32);
                },
                Ok(SourceCtl::Seek(_) | SourceCtl::Pause(_) | SourceCtl::Loop(_)) => {}
                Ok(SourceCtl::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if reader.is_finished() {
                break;
            }
            // Delivered vs expected samples: a lossless stream stays at ~100 %
            let (t, n) = last_check;
            if t.elapsed() > Duration::from_secs(5) && started.elapsed() > Duration::from_secs(2) {
                let now = shared.samples.load(Ordering::Relaxed);
                let expected = shared.rate.load(Ordering::Relaxed) as f64 * t.elapsed().as_secs_f64();
                let pct = (now - n) as f64 / expected * 100.0;
                if pct < 99.5 {
                    eprintln!("[source] samples lost: only {pct:.2} % of the nominal rate delivered");
                }
                last_check = (Instant::now(), now);
            }
        }
        unsafe { rtlsdr_cancel_async(dev.0) };
        let _ = reader.join();
        if bias_on {
            // The dongle keeps the voltage after close: never leave it on behind us
            unsafe { rtlsdr_set_bias_tee(dev.0, 0) };
        }
        unsafe { rtlsdr_close(dev.0) };
        Ok(())
    }
}
