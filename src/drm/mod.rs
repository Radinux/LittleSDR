//! DRM (Digital Radio Mondiale) receiver: OFDM sync and channel estimation, FAC, then (next
//! steps) SDC, MSC and audio.

pub mod audio;
pub mod cellmap;
pub mod fac;
pub mod msc;
pub mod ofdm;
pub mod sdc;
pub mod tables;

use crate::dsp::C32;
use fac::{FacDecoder, FacInfo};
use ofdm::{Frame, Ofdm};
use audio::XheAudio;
use msc::MscDecoder;
use sdc::{SdcDecoder, SdcInfo};

pub struct DrmReceiver {
    ofdm: Ofdm,
    fac: FacDecoder,
    sdc: SdcDecoder,
    msc: MscDecoder,
    /// MSC frames (400 ms of the multiplex) decoded and not yet taken
    pub msc_frames: Vec<Vec<u8>>,
    pub audio: XheAudio,
    /// Decoded audio (interleaved float) not yet taken
    pub pcm: Vec<f32>,
    pub last_fac: Option<FacInfo>,
    pub sdc_info: Option<SdcInfo>,
    pub frames: u64,
}

impl DrmReceiver {
    /// `rate`: 12, 24 or 48 kHz of complex baseband centred on the DRM channel
    pub fn new(rate: f64) -> DrmReceiver {
        DrmReceiver { ofdm: Ofdm::new(rate), fac: FacDecoder::new(), sdc: SdcDecoder::new(), msc: MscDecoder::new(), msc_frames: Vec::new(), audio: XheAudio::new(), pcm: Vec::new(), last_fac: None, sdc_info: None, frames: 0 }
    }

    pub fn process(&mut self, iq: &[C32]) {
        let mut frames: Vec<Frame> = Vec::new();
        self.ofdm.process(iq, &mut frames);
        for f in frames {
            self.frames += 1;
            if let Some(map) = self.ofdm.map() {
                // A frame whose FAC fails keeps its place: the next one after the last known
                let fac = match self.fac.decode(map, &f) {
                    Some(info) => Some(info),
                    None => self.last_fac.clone().map(|mut i| {
                        i.frame_id = (i.frame_id + 1) % 3;
                        i
                    }),
                };
                if let Some(info) = fac {
                    // The SDC opens each superframe (frame 0)
                    if info.frame_id == 0 {
                        if let Some(s) = self.sdc.decode(map, &f, info.sdc_mode == 1) {
                            self.sdc_info = Some(s);
                        }
                    }
                    if let Some(sdc) = &self.sdc_info {
                        let got = self.msc.push(map, &f, &info, sdc);
                        // Audio of the first service, from its stream
                        if let Some(a) = sdc.audio.iter().find(|a| a.coding == 3) {
                            let s = a.stream as usize;
                            if let Some(&(la, lb)) = sdc.streams.get(s) {
                                let off: usize = sdc.streams[..s].iter().map(|x| (x.0 + x.1) as usize).sum();
                                let len = (la + lb) as usize - if a.text { 4 } else { 0 };
                                for m in &got {
                                    if off + len <= m.len() {
                                        self.audio.push(a, &m[off..off + len], &mut self.pcm);
                                    }
                                }
                            }
                        }
                        self.msc_frames.extend(got);
                    }
                    self.last_fac = Some(info);
                }
            }
        }
    }

    pub fn status(&self) -> String {
        let i = &self.ofdm.info;
        format!(
            "mode {} SO {} | guard corr {:.2}, offset {:+.1} Hz, frame sync {} | {} frames, FAC {} ok / {} bad, SDC {} ok / {} bad | audio frames {} ok / {} bad, {} Hz",
            i.mode.map_or("?".into(), |m| format!("{m:?}")),
            i.so,
            i.guard_corr,
            i.freq_offset,
            if i.frame_found { "yes" } else { "no" },
            self.frames,
            self.fac.ok,
            self.fac.bad,
            self.sdc.ok,
            self.sdc.bad,
            self.audio.frames_ok,
            self.audio.frames_bad,
            self.audio.rate
        )
    }
}
