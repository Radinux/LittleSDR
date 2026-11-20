#![cfg_attr(not(faad), allow(dead_code, unused_imports))]
//! AAC / HE-AAC v1 / v2 decoding through FAAD2 (GPL-2.0-or-later, the system libfaad).
//! Used by DAB+ (and DRM later): raw access units with an AudioSpecificConfig.

use std::os::raw::{c_char, c_uchar, c_ulong, c_void};

#[repr(C)]
struct Config {
    def_object_type: c_uchar,
    def_sample_rate: c_ulong,
    output_format: c_uchar,
    down_matrix: c_uchar,
    use_old_adts_format: c_uchar,
    dont_upsample_implicit_sbr: c_uchar,
}

#[repr(C)]
struct FrameInfo {
    bytesconsumed: c_ulong,
    samples: c_ulong,
    channels: c_uchar,
    error: c_uchar,
    samplerate: c_ulong,
    sbr: c_uchar,
    object_type: c_uchar,
    header_type: c_uchar,
    num_front_channels: c_uchar,
    num_side_channels: c_uchar,
    num_back_channels: c_uchar,
    num_lfe_channels: c_uchar,
    channel_position: [c_uchar; 64],
    ps: c_uchar,
}

// Desktop: the system's libfaad; Android: the static build (build.rs links it)
#[cfg(faad)]
#[cfg_attr(not(target_os = "android"), link(name = "faad"))]
extern "C" {
    fn NeAACDecOpen() -> *mut c_void;
    fn NeAACDecClose(h: *mut c_void);
    fn NeAACDecGetCurrentConfiguration(h: *mut c_void) -> *mut Config;
    fn NeAACDecSetConfiguration(h: *mut c_void, c: *mut Config) -> c_uchar;
    fn NeAACDecInit2(h: *mut c_void, asc: *mut c_uchar, len: c_ulong, rate: *mut c_ulong, channels: *mut c_uchar) -> c_char;
    fn NeAACDecDecode(h: *mut c_void, info: *mut FrameInfo, buf: *mut c_uchar, len: c_ulong) -> *mut c_void;
    fn NeAACDecGetErrorMessage(e: c_uchar) -> *const c_char;
}

pub struct AacDecoder {
    h: *mut c_void,
    pub rate: u32,
    pub channels: u8,
    pub errors: u64,
    pub last_error: String,
}

// The decoder handle is only ever used from one thread at a time
unsafe impl Send for AacDecoder {}

impl AacDecoder {
    /// `asc`: MPEG-4 AudioSpecificConfig
    pub fn new(asc: &[u8]) -> Option<AacDecoder> {
        #[cfg(not(faad))]
        {
            let _ = asc;
            None // built without FAAD2 (Android until it is cross-compiled)
        }
        #[cfg(faad)]
        unsafe {
            let h = NeAACDecOpen();
            if h.is_null() {
                return None;
            }
            let c = NeAACDecGetCurrentConfiguration(h);
            (*c).output_format = 4; // float
            (*c).dont_upsample_implicit_sbr = 0;
            NeAACDecSetConfiguration(h, c);
            let mut asc = asc.to_vec();
            let (mut rate, mut ch): (c_ulong, c_uchar) = (0, 0);
            if NeAACDecInit2(h, asc.as_mut_ptr(), asc.len() as c_ulong, &mut rate, &mut ch) < 0 {
                NeAACDecClose(h);
                return None;
            }
            Some(AacDecoder { h, rate: rate as u32, channels: ch, errors: 0, last_error: String::new() })
        }
    }

    /// Decodes one access unit; appends interleaved float samples, returns (rate, channels)
    pub fn decode(&mut self, au: &[u8], out: &mut Vec<f32>) -> Option<(u32, u8)> {
        #[cfg(not(faad))]
        {
            let _ = (au, out);
            None
        }
        #[cfg(faad)]
        unsafe {
            let mut info: FrameInfo = std::mem::zeroed();
            let mut buf = au.to_vec();
            let p = NeAACDecDecode(self.h, &mut info, buf.as_mut_ptr(), buf.len() as c_ulong) as *const f32;
            if info.error != 0 || p.is_null() {
                self.errors += 1;
                self.last_error = std::ffi::CStr::from_ptr(NeAACDecGetErrorMessage(info.error)).to_string_lossy().into_owned();
                return None;
            }
            out.extend_from_slice(std::slice::from_raw_parts(p, info.samples as usize));
            self.rate = info.samplerate as u32;
            self.channels = info.channels;
            Some((self.rate, self.channels))
        }
    }
}

impl Drop for AacDecoder {
    fn drop(&mut self) {
        #[cfg(faad)]
        unsafe {
            NeAACDecClose(self.h)
        }
    }
}
