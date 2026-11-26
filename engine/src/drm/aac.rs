//! DRM AAC audio (AAC / HE-AAC / HE-AAC v2, the codec of most broadcasters, RRI among them).
//!
//! An audio super frame (400 ms, robustness modes A-D) holds 5 AAC frames at 12 kHz or 10 at
//! 24 kHz, each 960 samples (ES 201 980 5.3.1.1):
//! - header: the frame borders, 12 bits each (one fewer than frames), byte aligned;
//! - higher protected part: per frame, its first bytes then an 8-bit CRC over them;
//! - lower protected part: per frame, the rest of its bytes.
//!
//! The higher part's size per frame comes from the stream's length in part A (UEP); with
//! equal protection it is 0 and only the CRCs are there.
//!
//! Decoding: FAAD2's DRM build (libfaad_drm: ER AAC LC with 960-sample frames, SBR and PS
//! as DRM carries them), loaded at run time like the receiver drivers, so the engine builds
//! and runs without it. It takes each frame with its CRC byte first, as Dream does.

use super::sdc::AudioInfo;
use std::ffi::{c_uchar, c_ulong, c_void};
use std::sync::OnceLock;

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

struct Faad {
    open: unsafe extern "C" fn() -> *mut c_void,
    close: unsafe extern "C" fn(*mut c_void),
    get_conf: unsafe extern "C" fn(*mut c_void) -> *mut Config,
    set_conf: unsafe extern "C" fn(*mut c_void, *mut Config) -> c_uchar,
    init_drm: unsafe extern "C" fn(*mut *mut c_void, c_ulong, c_uchar) -> i8,
    decode: unsafe extern "C" fn(*mut c_void, *mut FrameInfo, *mut c_uchar, c_ulong) -> *mut c_void,
    _lib: libloading::Library,
}

/// The library, resolving its own symbols first. It exports the same names as the ordinary
/// FAAD2 the engine links for DAB+, and calls some of them itself (NeAACDecInitDRM calls
/// NeAACDecClose): without RTLD_DEEPBIND those calls land in the other build and crash.
/// Android: the ordinary FAAD2 is linked statically, its symbols hidden; no clash there.
unsafe fn open(name: &str) -> Option<libloading::Library> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        const RTLD_DEEPBIND: std::ffi::c_int = 0x0008;
        use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
        Library::open(Some(name), RTLD_NOW | RTLD_LOCAL | RTLD_DEEPBIND).ok().map(Into::into)
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        libloading::Library::new(name).ok()
    }
}

/// FAAD2's DRM build, if installed (Linux: faad2 package; Android: in the APK)
fn faad() -> Option<&'static Faad> {
    static F: OnceLock<Option<Faad>> = OnceLock::new();
    F.get_or_init(|| unsafe {
        let lib = ["libfaad_drm.so.2", "libfaad_drm.so"].iter().find_map(|n| open(n))?;
        Some(Faad {
            open: *lib.get(b"NeAACDecOpen\0").ok()?,
            close: *lib.get(b"NeAACDecClose\0").ok()?,
            get_conf: *lib.get(b"NeAACDecGetCurrentConfiguration\0").ok()?,
            set_conf: *lib.get(b"NeAACDecSetConfiguration\0").ok()?,
            init_drm: *lib.get(b"NeAACDecInitDRM\0").ok()?,
            decode: *lib.get(b"NeAACDecDecode\0").ok()?,
            _lib: lib,
        })
    })
    .as_ref()
}

/// One FAAD2 DRM decoder, set for a sample rate and channel layout
struct Decoder {
    h: *mut c_void,
    key: (u32, u8),
}

unsafe impl Send for Decoder {}

impl Decoder {
    fn new(core_rate: u32, layout: u8) -> Option<Decoder> {
        let f = faad()?;
        unsafe {
            let mut h = (f.open)();
            if h.is_null() {
                return None;
            }
            if (f.init_drm)(&mut h, core_rate as c_ulong, layout) < 0 {
                (f.close)(h);
                return None;
            }
            // After the DRM init: it closes and reopens the decoder, the configuration back
            // to its defaults (16-bit output, read as floats: noise)
            let c = (f.get_conf)(h);
            (*c).output_format = 4; // float
            (f.set_conf)(h, c);
            Some(Decoder { h, key: (core_rate, layout) })
        }
    }

    /// One frame, CRC byte first; interleaved float samples appended, (rate, channels)
    fn decode(&mut self, frame: &mut [u8], out: &mut Vec<f32>) -> Option<(u32, u8)> {
        let f = faad()?;
        unsafe {
            let mut info: FrameInfo = std::mem::zeroed();
            let p = (f.decode)(self.h, &mut info, frame.as_mut_ptr(), frame.len() as c_ulong) as *const f32;
            if info.error != 0 || p.is_null() || info.samples == 0 {
                return None;
            }
            out.extend_from_slice(std::slice::from_raw_parts(p, info.samples as usize));
            Some((info.samplerate as u32, info.channels))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if let Some(f) = faad() {
            unsafe { (f.close)(self.h) }
        }
    }
}

/// FAAD2's DRM channel layouts (neaacdec.h DRMCH_*)
fn layout(info: &AudioInfo) -> u8 {
    match (info.mode, info.sbr) {
        (0, false) => 1,     // mono
        (2, false) => 2,     // stereo
        (0, true) => 3,      // SBR mono
        (2, true) => 4,      // SBR stereo
        (1, _) => 5,         // SBR + parametric stereo
        _ => 1,
    }
}

/// The frames of one audio super frame: (frame with its CRC byte first)
pub fn split(sf: &[u8], frames: usize, len_a: usize) -> Option<Vec<Vec<u8>>> {
    let borders = frames - 1;
    let header = (borders * 12).div_ceil(8);
    let payload = sf.len().checked_sub(header + frames)?;
    // Frame lengths from the borders (12 bits each)
    let mut lens = Vec::with_capacity(frames);
    let mut prev = 0usize;
    for i in 0..borders {
        let bit = i * 12;
        let v = ((sf[bit / 8] as usize) << 8 | sf[bit / 8 + 1] as usize) >> (4 - bit % 8) & 0xFFF;
        if v <= prev || v > payload {
            return None;
        }
        lens.push(v - prev);
        prev = v;
    }
    if payload <= prev {
        return None;
    }
    lens.push(payload - prev);
    // Higher protected part: the same number of bytes from each frame, then its CRC
    let higher = (len_a.saturating_sub(header + frames) / frames).min(*lens.iter().min()?);
    let mut pos = header;
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(frames);
    for &l in &lens {
        let mut f = Vec::with_capacity(l + 1);
        f.push(*sf.get(pos + higher)?); // CRC first, then the frame
        f.extend_from_slice(&sf[pos..pos + higher]);
        pos += higher + 1;
        out.push(f);
    }
    for (f, &l) in out.iter_mut().zip(&lens) {
        let lower = l - higher;
        f.extend_from_slice(sf.get(pos..pos + lower)?);
        pos += lower;
    }
    Some(out)
}

/// Audio super frames -> AAC frames -> PCM
#[derive(Default)]
pub struct AacAudio {
    dec: Option<Decoder>,
    pub frames_ok: u64,
    pub frames_bad: u64,
    pub rate: u32,
    pub channels: u8,
    /// Samples (all channels) of the last good frame: the length of a lost one
    last_len: usize,
    fade: bool,
    /// The DRM build of FAAD2 is missing: said once
    pub missing: bool,
}

impl AacAudio {
    /// One audio super frame of the stream (without its text bytes); `len_a`: the stream's
    /// bytes in the higher protected part
    pub fn push(&mut self, info: &AudioInfo, len_a: usize, sf: &[u8], pcm: &mut Vec<f32>) {
        let frames = match info.rate {
            12_000 => 5,
            24_000 => 10,
            _ => return, // robustness mode E rates: not handled
        };
        let key = (info.rate, layout(info));
        if self.dec.as_ref().is_none_or(|d| d.key != key) {
            self.dec = Decoder::new(info.rate, key.1);
            if self.dec.is_none() && !self.missing {
                self.missing = true;
                eprintln!("[drm] AAC: FAAD2's DRM build (libfaad_drm) is missing: no sound for AAC stations");
            }
        }
        let Some(dec) = &mut self.dec else { return };
        let Some(list) = split(sf, frames, len_a) else {
            // Super frame unreadable: its 400 ms as silence, playback keeps time
            self.frames_bad += frames as u64;
            pcm.extend(std::iter::repeat_n(0.0, self.last_len * frames));
            self.fade = true;
            return;
        };
        for mut f in list {
            let start = pcm.len();
            match dec.decode(&mut f, pcm) {
                Some((r, c)) => {
                    self.frames_ok += 1;
                    self.rate = r;
                    self.channels = c;
                    self.last_len = pcm.len() - start;
                    if self.fade {
                        // Back after a gap: 10 ms ramp instead of a click
                        let n = (r as usize / 100 * c as usize).min(pcm.len() - start);
                        for (i, x) in pcm[start..start + n].iter_mut().enumerate() {
                            *x *= i as f32 / n as f32;
                        }
                        self.fade = false;
                    }
                }
                None => {
                    self.frames_bad += 1;
                    pcm.truncate(start);
                    pcm.extend(std::iter::repeat_n(0.0, self.last_len));
                    self.fade = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn super_frame_layout() {
        // 5 frames of 3, 4, 5, 6, 7 bytes; 2 bytes of each in the higher part
        let lens = [3usize, 4, 5, 6, 7];
        let mut borders = Vec::new();
        let mut acc = 0;
        for l in &lens[..4] {
            acc += l;
            borders.push(acc as u32);
        }
        // 4 x 12 bits = 6 bytes of header
        let mut hdr = vec![0u8; 6];
        for (i, &b) in borders.iter().enumerate() {
            let bit = i * 12;
            let v = (b & 0xFFF) << (4 - bit % 8);
            hdr[bit / 8] |= (v >> 8) as u8;
            hdr[bit / 8 + 1] |= v as u8;
        }
        let frames: Vec<Vec<u8>> = lens.iter().enumerate().map(|(k, &l)| (0..l).map(|j| (k * 16 + j) as u8).collect()).collect();
        let mut sf = hdr;
        for (k, f) in frames.iter().enumerate() {
            sf.extend_from_slice(&f[..2]);
            sf.push(0xC0 + k as u8); // CRC byte
        }
        for f in &frames {
            sf.extend_from_slice(&f[2..]);
        }
        let len_a = 6 + 5 * 2 + 5;
        let out = split(&sf, 5, len_a).unwrap();
        for (k, f) in out.iter().enumerate() {
            assert_eq!(f[0], 0xC0 + k as u8);
            assert_eq!(&f[1..], &frames[k][..]);
        }
        // Equal protection: no higher part, the CRCs right after the header
        let mut sf2 = sf[..6].to_vec();
        sf2.extend((0..5).map(|k| 0xC0 + k as u8));
        for f in &frames {
            sf2.extend_from_slice(f);
        }
        let out2 = split(&sf2, 5, 0).unwrap();
        assert_eq!(out2[3][0], 0xC3);
        assert_eq!(&out2[3][1..], &frames[3][..]);
    }
}

#[cfg(test)]
mod faad_tests {
    use super::*;

    /// Opening, decoding a damaged frame and closing must not crash (FAAD2 DRM build)
    #[test]
    fn faad_drm_survives_garbage() {
        let Some(mut d) = Decoder::new(12_000, 1) else { return }; // not installed: nothing to test
        let mut out = Vec::new();
        for k in 0..20u8 {
            let mut f: Vec<u8> = (0..60u8).map(|i| i.wrapping_mul(37).wrapping_add(k)).collect();
            let _ = d.decode(&mut f, &mut out);
        }
        drop(d);
        let d2 = Decoder::new(24_000, 3);
        drop(d2);
    }
}
