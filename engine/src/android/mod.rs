//! Android platform layer, all Rust: JNI entry points for the Kotlin app, sound through
//! AAudio, the spectrum and waterfall drawn with Vulkan on the app's Surface.

pub mod audio;
pub mod jni;
pub mod render;

use std::ffi::{c_char, c_int, c_void, CString};

#[repr(C)]
pub struct ANativeWindow {
    _p: [u8; 0],
}

#[link(name = "android")]
extern "C" {
    pub fn ANativeWindow_fromSurface(env: *mut c_void, surface: *mut c_void) -> *mut ANativeWindow;
    pub fn ANativeWindow_release(w: *mut ANativeWindow);
    pub fn ANativeWindow_getWidth(w: *mut ANativeWindow) -> i32;
    pub fn ANativeWindow_getHeight(w: *mut ANativeWindow) -> i32;
}

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// To logcat, tag LittleSDR
pub fn log(msg: &str) {
    let tag = c"LittleSDR";
    if let Ok(t) = CString::new(msg.replace('\0', " ")) {
        // SAFETY: two valid NUL-terminated strings
        unsafe { __android_log_write(4, tag.as_ptr(), t.as_ptr()) };
    }
}
