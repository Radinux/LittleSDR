//! SDRplay RSP1, RSP1A / 1B, RSP2, RSPduo (tuner A), RSPdx / dx-R2, through SDRplay's API 3
//! (proprietary, installed by the user with its service `sdrplay_apiService`). It is loaded
//! at run time, never linked nor shipped: LittleSDR stays GPL, and works without it. SDRplay
//! offers no such library for Android, so this source is for Linux.
//!
//! The structures below mirror the API 3.15 headers (sdrplay_api_*.h); `layout` tests check
//! the offsets that matter against the C compiler.

use super::airspy::{load, Feed};
use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DeviceT {
    ser_no: [c_char; 64],
    hw_ver: u8,
    tuner: c_int,
    rsp_duo_mode: c_int,
    valid: u8,
    rsp_duo_sample_freq: f64,
    dev: *mut c_void,
}

#[repr(C)]
pub(crate) struct FsFreqT {
    fs_hz: f64,
    sync_update: u8,
    re_cal: u8,
}

#[repr(C)]
pub(crate) struct DevParamsT {
    ppm: f64,
    fs_freq: FsFreqT,
    sync_update: [u32; 2],
    reset_flags: [u8; 3],
    mode: c_int,
    samples_per_pkt: u32,
    rsp1a: [u8; 2],
    rsp2: u8,
    rsp_duo_ext_ref: c_int,
    rsp_dx: RspDxParamsT,
}

#[repr(C)]
pub(crate) struct RspDxParamsT {
    hdr_enable: u8,
    bias_t_enable: u8,
    antenna_sel: c_int,
    rf_notch: u8,
    rf_dab_notch: u8,
}

#[repr(C)]
pub(crate) struct GainT {
    g_rdb: c_int,
    lna_state: u8,
    sync_update: u8,
    min_gr: c_int,
    gain_vals: [f32; 3],
}

#[repr(C)]
pub(crate) struct RfFreqT {
    rf_hz: f64,
    sync_update: u8,
}

#[repr(C)]
pub(crate) struct TunerParamsT {
    bw_type: c_int,
    if_type: c_int,
    lo_mode: c_int,
    gain: GainT,
    rf_freq: RfFreqT,
    dc_offset_tuner: [c_int; 3], // dcCal, speedUp (packed in the first), trackTime, refreshRateTime
}

#[repr(C)]
pub(crate) struct AgcT {
    enable: c_int,
    set_point_dbfs: c_int,
    attack_ms: u16,
    decay_ms: u16,
    decay_delay_ms: u16,
    decay_threshold_db: u16,
    sync_update: c_int,
}

#[repr(C)]
pub(crate) struct ControlParamsT {
    dc_offset: [u8; 2],
    decimation_enable: u8,
    decimation_factor: u8,
    wide_band_signal: u8,
    agc: AgcT,
    adsb_mode: c_int,
}

#[repr(C)]
pub(crate) struct Rsp2TunerParamsT {
    bias_t_enable: u8,
    am_port_sel: c_int,
    antenna_sel: c_int,
    rf_notch: u8,
}

#[repr(C)]
pub(crate) struct RspDuoTunerParamsT {
    bias_t_enable: u8,
    tuner1_am_port_sel: c_int,
    tuner1_am_notch: u8,
    rf_notch: u8,
    rf_dab_notch: u8,
    reset_slave_flags: [u8; 2],
}

#[repr(C)]
pub(crate) struct RxChannelParamsT {
    tuner: TunerParamsT,
    ctrl: ControlParamsT,
    rsp1a_bias_t_enable: u8,
    rsp2: Rsp2TunerParamsT,
    rsp_duo: RspDuoTunerParamsT,
    rsp_dx_hdr_bw: c_int,
}

#[repr(C)]
pub(crate) struct DeviceParamsT {
    dev: *mut DevParamsT,
    rx_a: *mut RxChannelParamsT,
    rx_b: *mut RxChannelParamsT,
}

type StreamCb = unsafe extern "C" fn(*mut i16, *mut i16, *mut c_void, u32, u32, *mut c_void);
type EventCb = unsafe extern "C" fn(c_int, c_int, *mut c_void, *mut c_void);

#[repr(C)]
struct CallbackFnsT {
    stream_a: StreamCb,
    stream_b: StreamCb,
    event: EventCb,
}

const RSP1A: u8 = 255;
const RSP2: u8 = 2;
const RSPDUO: u8 = 3;
const RSPDX: u8 = 4;
const RSP1B: u8 = 6;
const RSPDXR2: u8 = 7;
const TUNER_A: c_int = 1;
const AGC_50HZ: c_int = 2;
const UPDATE_DEV_FS: u32 = 0x0000_0001;
const UPDATE_RSP1A_BIAS: u32 = 0x0000_0010;
const UPDATE_RSP2_BIAS: u32 = 0x0000_0080;
const UPDATE_TUNER_GR: u32 = 0x0000_8000;
const UPDATE_TUNER_FRF: u32 = 0x0002_0000;
const UPDATE_TUNER_BW: u32 = 0x0004_0000;
const UPDATE_CTRL_DECIMATION: u32 = 0x0080_0000;
const UPDATE_CTRL_AGC: u32 = 0x0100_0000;
const UPDATE_RSPDUO_BIAS: u32 = 0x0800_0000;
const UPDATE_EXT1_RSPDX_BIAS: u32 = 0x0000_0002;

unsafe extern "C" fn stream(xi: *mut i16, xq: *mut i16, _p: *mut c_void, n: u32, _reset: u32, ctx: *mut c_void) {
    // SAFETY: the API hands `n` I and `n` Q samples; ctx is the Feed alive until Uninit
    let feed = &*(ctx as *const Feed);
    let (i, q) = (std::slice::from_raw_parts(xi, n as usize), std::slice::from_raw_parts(xq, n as usize));
    feed.push(i.iter().zip(q).map(|(&a, &b)| C32::new(a as f32 / 32768.0, b as f32 / 32768.0)).collect());
}

unsafe extern "C" fn stream_b(_: *mut i16, _: *mut i16, _: *mut c_void, _: u32, _: u32, _: *mut c_void) {}

unsafe extern "C" fn event(_: c_int, _: c_int, _: *mut c_void, _: *mut c_void) {}

/// The hardware sample rate (2 - 10 MS/s), the decimation that brings it to the rate asked
/// for, and the IF filter for it
fn plan(rate: u32) -> (f64, u8, c_int) {
    let rate = rate.clamp(62_500, 10_000_000);
    let mut dec = 1u32;
    while rate * dec < 2_000_000 && dec < 32 {
        dec *= 2;
    }
    let fs = (rate * dec) as f64;
    let out_khz = (rate / 1000) as c_int;
    let bw: c_int = [8000, 7000, 6000, 5000, 1536, 600, 300, 200].into_iter().find(|&b| b <= out_khz).unwrap_or(200);
    (fs, dec as u8, bw)
}

pub struct SdrPlay;

impl Source for SdrPlay {
    fn name(&self) -> String {
        "SDRplay".into()
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        if cfg!(target_os = "android") {
            return Err("SDRplay ne fournit pas son pilote pour Android".into());
        }
        let lib = load(&["libsdrplay_api.so.3", "libsdrplay_api.so", "libsdrplay_api.so.2"])
            .map_err(|e| format!("{e} : installe l'API SDRplay 3 (paquet libsdrplay ou sdrplay.com)"))?;
        let e = |n: &'static str| move |err: libloading::Error| format!("API SDRplay {n} : {err}");
        // SAFETY: the API's functions with their C signatures (sdrplay_api.h), the library kept
        // loaded while they are used; structures as the API 3.15 lays them out
        unsafe {
            let open: unsafe extern "C" fn() -> c_int = *lib.get(b"sdrplay_api_Open\0").map_err(e("Open"))?;
            let close: unsafe extern "C" fn() -> c_int = *lib.get(b"sdrplay_api_Close\0").map_err(e("Close"))?;
            let version: unsafe extern "C" fn(*mut f32) -> c_int = *lib.get(b"sdrplay_api_ApiVersion\0").map_err(e("ApiVersion"))?;
            let lock: unsafe extern "C" fn() -> c_int = *lib.get(b"sdrplay_api_LockDeviceApi\0").map_err(e("Lock"))?;
            let unlock: unsafe extern "C" fn() -> c_int = *lib.get(b"sdrplay_api_UnlockDeviceApi\0").map_err(e("Unlock"))?;
            let get_devices: unsafe extern "C" fn(*mut DeviceT, *mut u32, u32) -> c_int = *lib.get(b"sdrplay_api_GetDevices\0").map_err(e("GetDevices"))?;
            let select: unsafe extern "C" fn(*mut DeviceT) -> c_int = *lib.get(b"sdrplay_api_SelectDevice\0").map_err(e("SelectDevice"))?;
            let release: unsafe extern "C" fn(*mut DeviceT) -> c_int = *lib.get(b"sdrplay_api_ReleaseDevice\0").map_err(e("ReleaseDevice"))?;
            let error_string: unsafe extern "C" fn(c_int) -> *const c_char = *lib.get(b"sdrplay_api_GetErrorString\0").map_err(e("GetErrorString"))?;
            let get_params: unsafe extern "C" fn(*mut c_void, *mut *mut DeviceParamsT) -> c_int = *lib.get(b"sdrplay_api_GetDeviceParams\0").map_err(e("GetDeviceParams"))?;
            let init: unsafe extern "C" fn(*mut c_void, *mut CallbackFnsT, *mut c_void) -> c_int = *lib.get(b"sdrplay_api_Init\0").map_err(e("Init"))?;
            let uninit: unsafe extern "C" fn(*mut c_void) -> c_int = *lib.get(b"sdrplay_api_Uninit\0").map_err(e("Uninit"))?;
            let update: unsafe extern "C" fn(*mut c_void, c_int, u32, u32) -> c_int = *lib.get(b"sdrplay_api_Update\0").map_err(e("Update"))?;
            let msg = |r: c_int| CStr::from_ptr(error_string(r)).to_string_lossy().into_owned();

            let r = open();
            if r != 0 {
                return Err(format!("API SDRplay : {} (le service est-il lancé ? sudo systemctl start sdrplay)", msg(r)));
            }
            let mut ver = 0f32;
            version(&mut ver);
            if ver < 3.07 {
                close();
                return Err(format!("API SDRplay {ver:.2} trop ancienne (3.07 au moins)"));
            }
            lock();
            let mut devs: [DeviceT; 16] = std::mem::zeroed();
            let mut n = 0u32;
            get_devices(devs.as_mut_ptr(), &mut n, 16);
            if n == 0 {
                unlock();
                close();
                return Err("aucun SDRplay branché".into());
            }
            let mut dev = devs[0];
            if dev.hw_ver == RSPDUO {
                dev.tuner = TUNER_A;
                dev.rsp_duo_mode = 1; // single tuner
            }
            let r = select(&mut dev);
            unlock();
            if r != 0 {
                close();
                return Err(format!("SDRplay occupé : {}", msg(r)));
            }
            let mut params: *mut DeviceParamsT = std::ptr::null_mut();
            get_params(dev.dev, &mut params);
            if params.is_null() || (*params).dev.is_null() || (*params).rx_a.is_null() {
                release(&mut dev);
                close();
                return Err("SDRplay : paramètres indisponibles".into());
            }
            let (devp, rx) = (&mut *(*params).dev, &mut *(*params).rx_a);
            let (fs, dec, bw) = plan(cfg.rate);
            devp.fs_freq.fs_hz = fs;
            rx.ctrl.decimation_enable = (dec > 1) as u8;
            rx.ctrl.decimation_factor = dec;
            rx.tuner.bw_type = bw;
            rx.tuner.if_type = 0; // zero IF
            rx.tuner.rf_freq.rf_hz = cfg.center as f64;
            let set_gain = |rx: &mut RxChannelParamsT, g: Option<f32>| match g {
                // "Gain" 0 - 39 dB = less gain reduction (59 - 20 dB), LNA at full gain
                Some(db) => {
                    rx.ctrl.agc.enable = 0;
                    rx.tuner.gain.g_rdb = (59.0 - db.clamp(0.0, 39.0)).round() as c_int;
                    rx.tuner.gain.lna_state = 0;
                }
                None => {
                    rx.ctrl.agc.enable = AGC_50HZ;
                    rx.ctrl.agc.set_point_dbfs = -30;
                }
            };
            set_gain(rx, cfg.gain);
            let bias = |devp: &mut DevParamsT, rx: &mut RxChannelParamsT, on: bool| -> (u32, u32) {
                let v = on as u8;
                match dev.hw_ver {
                    RSP1A | RSP1B => {
                        rx.rsp1a_bias_t_enable = v;
                        (UPDATE_RSP1A_BIAS, 0)
                    }
                    RSP2 => {
                        rx.rsp2.bias_t_enable = v;
                        (UPDATE_RSP2_BIAS, 0)
                    }
                    RSPDUO => {
                        rx.rsp_duo.bias_t_enable = v;
                        (UPDATE_RSPDUO_BIAS, 0)
                    }
                    RSPDX | RSPDXR2 => {
                        devp.rsp_dx.bias_t_enable = v;
                        (0, UPDATE_EXT1_RSPDX_BIAS)
                    }
                    _ => (0, 0), // RSP1: no bias-tee
                }
            };
            bias(devp, rx, cfg.bias_tee);

            let feed = Feed::new(out, (fs / dec as f64) as u32, cfg.center);
            let ctx = Arc::as_ptr(&feed) as *mut c_void;
            let mut cbs = CallbackFnsT { stream_a: stream, stream_b, event };
            let r = init(dev.dev, &mut cbs, ctx);
            if r != 0 {
                release(&mut dev);
                close();
                return Err(format!("SDRplay ne démarre pas : {}", msg(r)));
            }
            let result = loop {
                let (devp, rx) = (&mut *(*params).dev, &mut *(*params).rx_a);
                match ctl.recv_timeout(Duration::from_millis(300)) {
                    Ok(SourceCtl::Center(f)) => {
                        rx.tuner.rf_freq.rf_hz = f as f64;
                        update(dev.dev, TUNER_A, UPDATE_TUNER_FRF, 0);
                        feed.center.store(f, Relaxed);
                    }
                    Ok(SourceCtl::Gain(g)) => {
                        set_gain(rx, g);
                        update(dev.dev, TUNER_A, UPDATE_CTRL_AGC | if g.is_some() { UPDATE_TUNER_GR } else { 0 }, 0);
                    }
                    Ok(SourceCtl::BiasTee(on)) => {
                        let (r, x) = bias(devp, rx, on);
                        if r != 0 || x != 0 {
                            update(dev.dev, TUNER_A, r, x);
                        }
                    }
                    Ok(SourceCtl::Rate(want)) => {
                        let (fs, dec, bw) = plan(want);
                        devp.fs_freq.fs_hz = fs;
                        rx.ctrl.decimation_enable = (dec > 1) as u8;
                        rx.ctrl.decimation_factor = dec;
                        rx.tuner.bw_type = bw;
                        update(dev.dev, TUNER_A, UPDATE_DEV_FS | UPDATE_CTRL_DECIMATION | UPDATE_TUNER_BW, 0);
                        feed.rate.store((fs / dec as f64) as u32, Relaxed);
                    }
                    Ok(SourceCtl::Stop) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
                    Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                }
            };
            uninit(dev.dev);
            release(&mut dev);
            close();
            drop(feed);
            result
        }
    }
}

#[cfg(test)]
mod layout {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// Offsets and sizes as gcc lays out the API 3.15 headers (checked with a C program
    /// against /usr/include/sdrplay_api*.h)
    #[test]
    fn matches_the_c_headers() {
        assert_eq!(size_of::<DeviceT>(), 96);
        assert_eq!(offset_of!(DeviceT, hw_ver), 64);
        assert_eq!(offset_of!(DeviceT, dev), 88);
        assert_eq!(offset_of!(DevParamsT, fs_freq), 8);
        assert_eq!(offset_of!(DevParamsT, rsp_dx), 52);
        assert_eq!(size_of::<DevParamsT>(), 64);
        assert_eq!(offset_of!(TunerParamsT, gain), 12);
        assert_eq!(offset_of!(TunerParamsT, rf_freq), 40);
        assert_eq!(size_of::<TunerParamsT>(), 72);
        assert_eq!(offset_of!(RxChannelParamsT, ctrl), 72);
        assert_eq!(offset_of!(ControlParamsT, agc), 8);
        assert_eq!(offset_of!(RxChannelParamsT, rsp1a_bias_t_enable), 104);
        assert_eq!(offset_of!(RxChannelParamsT, rsp2), 108);
        assert_eq!(offset_of!(RxChannelParamsT, rsp_duo), 124);
        assert_eq!(offset_of!(RxChannelParamsT, rsp_dx_hdr_bw), 140);
        assert_eq!(size_of::<RxChannelParamsT>(), 144);
    }

    #[test]
    fn rate_plan() {
        assert_eq!(plan(2_000_000), (2e6, 1, 1536));
        assert_eq!(plan(500_000), (2e6, 4, 300));
        assert_eq!(plan(8_000_000), (8e6, 1, 8000));
        assert_eq!(plan(250_000), (2e6, 8, 200));
    }
}
