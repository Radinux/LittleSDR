//! Sound out through AAudio: a float stereo stream at 48 kHz whose callback reads the
//! service's ring. Restarts by itself when the output changes (headphones, Bluetooth).

use super::log;
use crate::audio::RingShared;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

#[repr(C)]
struct Builder {
    _p: [u8; 0],
}
#[repr(C)]
struct Stream {
    _p: [u8; 0],
}

type DataCb = extern "C" fn(*mut Stream, *mut c_void, *mut c_void, i32) -> i32;
type ErrorCb = extern "C" fn(*mut Stream, *mut c_void, i32);

#[link(name = "aaudio")]
extern "C" {
    fn AAudio_createStreamBuilder(b: *mut *mut Builder) -> i32;
    fn AAudioStreamBuilder_setDirection(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setSharingMode(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setPerformanceMode(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setUsage(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setContentType(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setSampleRate(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setChannelCount(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setFormat(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setDeviceId(b: *mut Builder, v: i32);
    fn AAudioStreamBuilder_setDataCallback(b: *mut Builder, cb: DataCb, user: *mut c_void);
    fn AAudioStreamBuilder_setErrorCallback(b: *mut Builder, cb: ErrorCb, user: *mut c_void);
    fn AAudioStreamBuilder_openStream(b: *mut Builder, s: *mut *mut Stream) -> i32;
    fn AAudioStreamBuilder_delete(b: *mut Builder) -> i32;
    fn AAudioStream_requestStart(s: *mut Stream) -> i32;
    fn AAudioStream_requestStop(s: *mut Stream) -> i32;
    fn AAudioStream_close(s: *mut Stream) -> i32;
    fn AAudioStream_getFramesPerBurst(s: *mut Stream) -> i32;
    fn AAudioStream_setBufferSizeInFrames(s: *mut Stream, n: i32) -> i32;
    fn AAudioStream_getSampleRate(s: *mut Stream) -> i32;
}

const DIRECTION_OUTPUT: i32 = 0;
const SHARING_SHARED: i32 = 1;
const PERF_POWER_SAVING: i32 = 11;
const USAGE_MEDIA: i32 = 1;
const CONTENT_SPEECH: i32 = 1;
const FORMAT_FLOAT: i32 = 2;
const ERROR_DISCONNECTED: i32 = -899;

struct Out {
    stream: *mut Stream,
    /// Owned by the stream's callbacks (Box::into_raw), freed after close
    ring: *mut Arc<RingShared>,
}

// The raw pointers are only touched under the mutex
unsafe impl Send for Out {}

static OUT: Mutex<Option<Out>> = Mutex::new(None);
/// Chosen output device (0: the system's default)
static DEVICE: Mutex<i32> = Mutex::new(0);

extern "C" fn on_data(_: *mut Stream, user: *mut c_void, data: *mut c_void, frames: i32) -> i32 {
    // SAFETY: user is the Arc box set at open, alive until the stream is closed; data holds
    // frames × 2 floats
    let (ring, buf) = unsafe { (&*(user as *const Arc<RingShared>), std::slice::from_raw_parts_mut(data as *mut f32, frames as usize * 2)) };
    ring.read(buf, 2);
    0 // continue
}

extern "C" fn on_error(_: *mut Stream, user: *mut c_void, err: i32) {
    log(&format!("AAudio error {err}"));
    if err == ERROR_DISCONNECTED {
        // Never close a stream from its own callback: a thread reopens it on the new output
        // SAFETY: as in on_data
        let ring = unsafe { (*(user as *const Arc<RingShared>)).clone() };
        std::thread::spawn(move || {
            stop();
            start(ring);
        });
    }
}

pub fn set_device(id: i32) {
    *DEVICE.lock().unwrap() = id;
}

/// Opens the output on this ring (nothing if it is already open)
pub fn start(ring: Arc<RingShared>) {
    let mut out = OUT.lock().unwrap();
    if out.is_some() {
        return;
    }
    let user = Box::into_raw(Box::new(ring));
    // SAFETY: plain AAudio C calls on pointers it hands out; `user` outlives the stream
    unsafe {
        let mut b = std::ptr::null_mut();
        if AAudio_createStreamBuilder(&mut b) != 0 {
            log("AAudio: builder failed");
            drop(Box::from_raw(user));
            return;
        }
        AAudioStreamBuilder_setDirection(b, DIRECTION_OUTPUT);
        AAudioStreamBuilder_setSharingMode(b, SHARING_SHARED);
        AAudioStreamBuilder_setPerformanceMode(b, PERF_POWER_SAVING);
        AAudioStreamBuilder_setUsage(b, USAGE_MEDIA);
        AAudioStreamBuilder_setContentType(b, CONTENT_SPEECH);
        AAudioStreamBuilder_setSampleRate(b, 48_000);
        AAudioStreamBuilder_setChannelCount(b, 2);
        AAudioStreamBuilder_setFormat(b, FORMAT_FLOAT);
        let dev = *DEVICE.lock().unwrap();
        if dev > 0 {
            AAudioStreamBuilder_setDeviceId(b, dev);
        }
        AAudioStreamBuilder_setDataCallback(b, on_data, user as *mut c_void);
        AAudioStreamBuilder_setErrorCallback(b, on_error, user as *mut c_void);
        let mut s = std::ptr::null_mut();
        let r = AAudioStreamBuilder_openStream(b, &mut s);
        AAudioStreamBuilder_delete(b);
        if r != 0 {
            log(&format!("AAudio: open failed {r}"));
            drop(Box::from_raw(user));
            return;
        }
        let burst = AAudioStream_getFramesPerBurst(s);
        AAudioStream_setBufferSizeInFrames(s, (burst * 4).max(960));
        if AAudioStream_requestStart(s) != 0 {
            log("AAudio: start failed");
            AAudioStream_close(s);
            drop(Box::from_raw(user));
            return;
        }
        log(&format!("AAudio: playing at {} Hz, burst {burst}", AAudioStream_getSampleRate(s)));
        *out = Some(Out { stream: s, ring: user });
    }
}

pub fn stop() {
    let Some(o) = OUT.lock().unwrap().take() else { return };
    // SAFETY: the stream is ours; after close no callback runs, so the ring box can go
    unsafe {
        AAudioStream_requestStop(o.stream);
        AAudioStream_close(o.stream);
        drop(Box::from_raw(o.ring));
    }
}
