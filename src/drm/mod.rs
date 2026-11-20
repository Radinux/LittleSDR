//! DRM (Digital Radio Mondiale) receiver: OFDM sync and channel estimation, FAC, then (next
//! steps) SDC, MSC and audio.

pub mod cellmap;
pub mod fac;
pub mod ofdm;
pub mod sdc;
pub mod tables;

use crate::dsp::C32;
use fac::{FacDecoder, FacInfo};
use ofdm::{Frame, Ofdm};
use sdc::{SdcDecoder, SdcInfo};

pub struct DrmReceiver {
    ofdm: Ofdm,
    fac: FacDecoder,
    sdc: SdcDecoder,
    pub last_fac: Option<FacInfo>,
    pub sdc_info: Option<SdcInfo>,
    pub frames: u64,
}

impl DrmReceiver {
    /// `rate`: 12, 24 or 48 kHz of complex baseband centred on the DRM channel
    pub fn new(rate: f64) -> DrmReceiver {
        DrmReceiver { ofdm: Ofdm::new(rate), fac: FacDecoder::new(), sdc: SdcDecoder::new(), last_fac: None, sdc_info: None, frames: 0 }
    }

    pub fn process(&mut self, iq: &[C32]) {
        let mut frames: Vec<Frame> = Vec::new();
        self.ofdm.process(iq, &mut frames);
        for f in frames {
            self.frames += 1;
            if let Some(map) = self.ofdm.map() {
                if let Some(info) = self.fac.decode(map, &f) {
                    // The SDC opens each superframe (frame 0)
                    if info.frame_id == 0 {
                        if let Some(s) = self.sdc.decode(map, &f, info.sdc_mode == 1) {
                            self.sdc_info = Some(s);
                        }
                    }
                    self.last_fac = Some(info);
                }
            }
        }
    }

    pub fn status(&self) -> String {
        let i = &self.ofdm.info;
        format!(
            "mode {} SO {} | guard corr {:.2}, offset {:+.1} Hz, frame sync {} | {} frames, FAC {} ok / {} bad, SDC {} ok / {} bad",
            i.mode.map_or("?".into(), |m| format!("{m:?}")),
            i.so,
            i.guard_corr,
            i.freq_offset,
            if i.frame_found { "yes" } else { "no" },
            self.frames,
            self.fac.ok,
            self.fac.bad,
            self.sdc.ok,
            self.sdc.bad
        )
    }
}
