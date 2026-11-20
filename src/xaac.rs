//! xHE-AAC (USAC) decoding through libxaac (Apache-2.0), when it was found at build time.

#[cfg(xaac)]
mod ffi {
    use std::ffi::c_int;
    #[repr(C)]
    pub struct Xaac {
        _p: [u8; 0],
    }
    extern "C" {
        pub fn xaac_open(asc: *const u8, len: c_int) -> *mut Xaac;
        pub fn xaac_decode(x: *mut Xaac, au: *const u8, len: c_int, pcm: *mut i16, cap: c_int, rate: *mut c_int, ch: *mut c_int) -> c_int;
        pub fn xaac_close(x: *mut Xaac);
    }
}

pub struct XaacDecoder {
    #[cfg(xaac)]
    h: *mut ffi::Xaac,
    pcm: Vec<i16>,
}

// The handle is only used from the thread that owns the decoder
unsafe impl Send for XaacDecoder {}

impl XaacDecoder {
    pub fn available() -> bool {
        cfg!(xaac)
    }

    /// `asc`: AudioSpecificConfig (audio object type 42, USAC)
    #[cfg(xaac)]
    pub fn new(asc: &[u8]) -> Option<XaacDecoder> {
        // SAFETY: asc is a valid slice; the handle is freed in Drop
        let h = unsafe { ffi::xaac_open(asc.as_ptr(), asc.len() as i32) };
        (!h.is_null()).then(|| XaacDecoder { h, pcm: vec![0; 8192] })
    }

    #[cfg(not(xaac))]
    pub fn new(_asc: &[u8]) -> Option<XaacDecoder> {
        None
    }

    /// One access unit -> interleaved float samples appended to `out`; (rate, channels)
    #[cfg(xaac)]
    pub fn decode(&mut self, au: &[u8], out: &mut Vec<f32>) -> Option<(u32, u8)> {
        let (mut rate, mut ch) = (0, 0);
        // SAFETY: buffers valid for the given sizes; the handle is live
        let n = unsafe { ffi::xaac_decode(self.h, au.as_ptr(), au.len() as i32, self.pcm.as_mut_ptr(), self.pcm.len() as i32, &mut rate, &mut ch) };
        if n <= 0 || rate <= 0 || ch <= 0 {
            return None;
        }
        out.extend(self.pcm[..n as usize].iter().map(|&s| s as f32 / 32768.0));
        Some((rate as u32, ch as u8))
    }

    #[cfg(not(xaac))]
    pub fn decode(&mut self, _au: &[u8], _out: &mut Vec<f32>) -> Option<(u32, u8)> {
        let _ = &self.pcm;
        None
    }
}

impl Drop for XaacDecoder {
    fn drop(&mut self) {
        #[cfg(xaac)]
        // SAFETY: created by xaac_open, freed once
        unsafe {
            ffi::xaac_close(self.h)
        }
    }
}
