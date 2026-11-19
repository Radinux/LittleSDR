//! DAB / DAB+ digital radio reception (transmission mode I, Band III): OFDM, FIC (ensemble
//! and services), MSC sub-channel decoding, DAB+ superframes and HE-AAC audio (FAAD2).

pub mod dabplus;
pub mod fec;
pub mod fic;
pub mod msc;
pub mod ofdm;

use crate::dsp::{Frame, Interp, C32};
use dabplus::DabPlus;
use fic::{Fic, Service};
use msc::{SubChannelDecoder, CIF_BITS};
use ofdm::{Ofdm, OfdmFrame};

pub struct DabReceiver {
    pub ofdm: Ofdm,
    pub fic: Fic,
    /// Wanted service: SId, or the start of its label (case-insensitive)
    want: Option<String>,
    playing: Option<(u32, SubChannelDecoder, DabPlus)>,
    frames: Vec<OfdmFrame>,
    pcm: Vec<f32>,
    resampler: Option<(u32, Interp<Frame>)>,
    /// Frames in a row without a single good FIB while synced (false lock detection)
    bad_frames: u32,
    pub false_locks: u32,
}

impl Default for DabReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl DabReceiver {
    pub fn new() -> DabReceiver {
        DabReceiver { ofdm: Ofdm::new(), fic: Fic::new(), want: None, playing: None, frames: Vec::new(), pcm: Vec::new(), resampler: None, bad_frames: 0, false_locks: 0 }
    }

    pub fn services(&self) -> Vec<Service> {
        self.fic.ensemble.services.values().cloned().collect()
    }

    /// Selects a service by SId (hex) or label prefix; it starts once the FIC has described it
    pub fn select(&mut self, what: &str) {
        self.want = Some(what.to_string());
        self.playing = None;
    }

    fn wanted(&self) -> Option<Service> {
        let w = self.want.as_ref()?;
        let services = self.services();
        if let Ok(sid) = u32::from_str_radix(w.trim_start_matches("0x"), 16) {
            if let Some(s) = services.iter().find(|s| s.sid == sid) {
                return Some(s.clone());
            }
        }
        let wl = w.trim().to_lowercase();
        if let Some(s) = services.iter().find(|s| s.label.to_lowercase() == wl) {
            return Some(s.clone()); // exact name first: "nostalgie" is not "NOSTALGIE 60/70"
        }
        services.into_iter().find(|s| !s.label.is_empty() && s.label.to_lowercase().starts_with(&wl))
    }

    pub fn playing(&self) -> Option<&Service> {
        let sid = self.playing.as_ref()?.0;
        self.fic.ensemble.services.get(&sid)
    }

    /// IQ at 2.048 MS/s in; 48 kHz stereo frames of the selected service out
    pub fn process(&mut self, iq: &[C32], out: &mut Vec<Frame>) {
        self.frames.clear();
        self.ofdm.process(iq, &mut self.frames);
        let frames = std::mem::take(&mut self.frames);
        for f in &frames {
            let good_before = self.fic.stats.fibs - self.fic.stats.crc_errors;
            self.fic.process(&f.fic);
            if self.fic.stats.fibs - self.fic.stats.crc_errors == good_before {
                self.bad_frames += 1;
                if self.bad_frames >= 10 {
                    // ~1 s synced without one readable FIB: wrong timing or frequency
                    self.bad_frames = 0;
                    self.false_locks += 1;
                    self.ofdm.reset();
                    break;
                }
            } else {
                self.bad_frames = 0;
            }
            if self.playing.is_none() {
                if let Some(s) = self.wanted() {
                    let sub = s.subch.and_then(|i| self.fic.ensemble.subchannels.get(&i)).cloned();
                    if let (Some(sub), 63) = (sub, s.ascty) {
                        if let Some(dec) = SubChannelDecoder::new(sub) {
                            self.playing = Some((s.sid, dec, DabPlus::new()));
                        }
                    }
                }
            }
            if let Some((_, dec, dp)) = &mut self.playing {
                for cif in f.msc.chunks_exact(CIF_BITS) {
                    if let Some(bytes) = dec.process(cif) {
                        self.pcm.clear();
                        if let Some((rate, ch)) = dp.push(bytes, &mut self.pcm) {
                            Self::emit(&mut self.resampler, &self.pcm, rate, ch, out);
                        }
                    }
                }
            }
        }
        self.frames = frames;
    }

    fn emit(rs: &mut Option<(u32, Interp<Frame>)>, pcm: &[f32], rate: u32, ch: u8, out: &mut Vec<Frame>) {
        let frames: Vec<Frame> = if ch >= 2 {
            pcm.chunks_exact(ch as usize).map(|c| Frame::new(c[0], c[1])).collect()
        } else {
            pcm.iter().map(|&v| Frame::mono(v)).collect()
        };
        if rate == 48_000 {
            out.extend(frames);
            return;
        }
        if rs.as_ref().map_or(true, |r| r.0 != rate) {
            *rs = Some((rate, Interp::new(rate as f64, 48_000.0)));
        }
        rs.as_mut().unwrap().1.process(&frames, out);
    }

    pub fn describe(&self) -> String {
        let o = &self.ofdm.stats;
        let e = &self.fic.ensemble;
        let mut s = format!(
            "DAB: {}, offset {:+.0} Hz, SNR {:.1} dB, resyncs {} | ensemble \"{}\", {} services, FIC {} / {} ok",
            if self.ofdm.locked() { "synced" } else { "searching" },
            o.freq_hz,
            o.snr_db,
            o.resyncs,
            e.label,
            e.services.len(),
            self.fic.stats.fibs - self.fic.stats.crc_errors,
            self.fic.stats.fibs
        );
        if let Some((sid, dec, dp)) = &self.playing {
            let name = e.services.get(sid).map_or("?".into(), |s| s.label.clone());
            let st = &dp.stats;
            s += &format!(
                " | playing \"{name}\" {} kbit/s {} | superframes {}, RS fixed {} / failed {}, AU ok {} / CRC {}, AAC errors {}",
                dec.bitrate(),
                dp.format.map_or("?".into(), |f| f.describe()),
                st.superframes,
                st.rs_corrected,
                st.rs_failed,
                st.au_ok,
                st.au_crc,
                st.aac_errors
            );
        } else if let Some(w) = &self.want {
            s += &format!(" | waiting for \"{w}\"");
        }
        s
    }
}
