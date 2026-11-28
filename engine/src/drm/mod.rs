//! DRM (Digital Radio Mondiale) receiver: OFDM sync and channel estimation, FAC, then (next
//! steps) SDC, MSC and audio.

pub mod aac;
pub mod audio;
pub mod cellmap;
pub mod fac;
pub mod msc;
pub mod ofdm;
pub mod sdc;
pub mod tables;

use crate::dsp::{Frame as AudioFrame, Interp, C32};
use std::collections::VecDeque;
use fac::{FacDecoder, FacInfo};
use ofdm::{Frame, Ofdm};
use aac::AacAudio;
use audio::{TextDecoder, XheAudio};
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
    /// AAC / HE-AAC stations (most of them): FAAD2's DRM build
    pub aac: AacAudio,
    /// The station's audio is AAC (else xHE-AAC)
    aac_on: bool,
    /// FACs failed in a row since the lock (or the last good one); any good one this lock
    fac_misses: u32,
    fac_seen: bool,
    /// Spectrum occupancy the last FAC announced, and how many FACs in a row agreed on it
    so_vote: (u8, u32),
    pub text: TextDecoder,
    /// New text message, not yet taken
    pub text_news: Option<String>,
    /// Decoded audio (interleaved float) not yet taken
    pub pcm: Vec<f32>,
    pub last_fac: Option<FacInfo>,
    pub sdc_info: Option<SdcInfo>,
    pub frames: u64,
    /// 48 kHz output: resampler from the codec's rate, and a reserve that smooths the
    /// 400 ms bursts of the decoder into a steady stream
    rs: Option<(u32, Interp<AudioFrame>)>,
    reserve: VecDeque<AudioFrame>,
    playing: bool,
}

impl DrmReceiver {
    /// `rate`: 12, 24 or 48 kHz of complex baseband centred on the DRM channel
    pub fn new(rate: f64) -> DrmReceiver {
        DrmReceiver { ofdm: Ofdm::new(rate), fac: FacDecoder::new(), sdc: SdcDecoder::new(), msc: MscDecoder::new(), msc_frames: Vec::new(), audio: XheAudio::new(), aac: AacAudio::default(), aac_on: false, fac_misses: 0, fac_seen: false, so_vote: (0, 0), text: TextDecoder::default(), text_news: None, pcm: Vec::new(), last_fac: None, sdc_info: None, frames: 0, rs: None, reserve: VecDeque::new(), playing: false }
    }

    pub fn process(&mut self, iq: &[C32]) {
        let mut frames: Vec<Frame> = Vec::new();
        self.ofdm.process(iq, &mut frames);
        for f in frames {
            self.frames += 1;
            if let Some(map) = self.ofdm.map() {
                // A frame whose FAC fails keeps its place: the next one after the last known
                let fac = match self.fac.decode(map, &f) {
                    Some(info) => {
                        self.fac_misses = 0;
                        self.fac_seen = true;
                        // Two FACs agreeing on another occupancy than the one guessed: theirs
                        self.so_vote = if self.so_vote.0 == info.so { (info.so, self.so_vote.1 + 1) } else { (info.so, 1) };
                        if self.so_vote.1 >= 2 && map.so != info.so as usize {
                            self.ofdm.set_so(info.so as usize);
                            self.last_fac = Some(info);
                            continue;
                        }
                        Some(info)
                    }
                    None => {
                        self.fac_misses += 1;
                        // Locked on the wrong robustness mode (no FAC at all in 4 s): search
                        // again without it. Lost for good (16 s without one): search afresh.
                        if (!self.fac_seen && self.fac_misses >= 10) || self.fac_misses >= 40 {
                            if self.fac_seen {
                                self.ofdm.reset();
                            } else {
                                self.ofdm.relock_without_mode();
                            }
                            self.fac_misses = 0;
                            self.fac_seen = false;
                            self.last_fac = None;
                            continue;
                        }
                        self.last_fac.clone().map(|mut i| {
                            i.frame_id = (i.frame_id + 1) % 3;
                            i
                        })
                    }
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
                        // Audio of the first service, from its stream: xHE-AAC or AAC
                        if let Some(a) = sdc.audio.iter().find(|a| a.coding == 3 || a.coding == 0) {
                            let s = a.stream as usize;
                            if let Some(&(la, lb)) = sdc.streams.get(s) {
                                let off: usize = sdc.streams[..s].iter().map(|x| (x.0 + x.1) as usize).sum();
                                let len = (la + lb) as usize - if a.text { 4 } else { 0 };
                                for m in &got {
                                    if off + len <= m.len() {
                                        self.aac_on = a.coding == 0;
                                        if self.aac_on {
                                            self.aac.push(a, la as usize, &m[off..off + len], &mut self.pcm);
                                        } else {
                                            self.audio.push(a, &m[off..off + len], &mut self.pcm);
                                        }
                                        if a.text && off + len + 4 <= m.len() {
                                            if let Some(t) = self.text.push(&m[off + len..off + len + 4]) {
                                                self.text_news = Some(t);
                                            }
                                        }
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

    /// Engine use: IF samples at `if_rate`; exactly as many 48 kHz frames out as went in
    /// (so the sound card is fed steadily), audio delayed by a small reserve
    pub fn process_audio(&mut self, iq: &[C32], if_rate: f64, out: &mut Vec<AudioFrame>) {
        self.process(iq);
        let (ch, rate) = if self.aac_on { (self.aac.channels.max(1) as usize, self.aac.rate) } else { (self.audio.channels.max(1) as usize, self.audio.rate) };
        if rate > 0 && !self.pcm.is_empty() {
            if self.rs.as_ref().is_none_or(|r| r.0 != rate) {
                self.rs = Some((rate, Interp::new(rate as f64, 48_000.0)));
            }
            let frames: Vec<AudioFrame> = self.pcm.chunks(ch).map(|c| AudioFrame::new(c[0], *c.get(1).unwrap_or(&c[0]))).collect();
            self.pcm.clear();
            let mut up = Vec::with_capacity(frames.len() * 48_000 / rate as usize + 4);
            self.rs.as_mut().unwrap().1.process(&frames, &mut up);
            self.reserve.extend(up);
        }
        self.pcm.clear();
        let want = (iq.len() as f64 * 48_000.0 / if_rate).round() as usize;
        // Start once 1 s is in reserve; too much reserve (clock drift): drop some
        if !self.playing && self.reserve.len() >= 48_000 {
            self.playing = true;
        }
        if self.reserve.len() > 144_000 {
            let extra = self.reserve.len() - 72_000;
            self.reserve.drain(..extra);
        }
        for _ in 0..want {
            let f = if self.playing { self.reserve.pop_front() } else { None };
            if f.is_none() {
                self.playing = false; // ran dry: fill the reserve again
            }
            out.push(f.unwrap_or_default());
        }
    }

    pub fn take_text(&mut self) -> Option<String> {
        self.text_news.take()
    }

    /// Frames found and the FAC read: the signal is being received
    pub fn synced(&self) -> bool {
        self.ofdm.info.frame_found && self.last_fac.is_some()
    }

    /// Reception quality 0..1 (recent FAC blocks decoded), 0 when not synced
    pub fn quality(&self) -> f32 {
        if self.synced() { self.fac.quality } else { 0.0 }
    }

    /// Station label alone (SDC), once known
    pub fn label(&self) -> String {
        self.sdc_info.as_ref().and_then(|s| s.labels.first()).map(|l| l.1.trim().to_string()).unwrap_or_default()
    }

    /// Station name and audio, once known
    pub fn station(&self) -> String {
        match &self.sdc_info {
            Some(s) => {
                let label = s.labels.first().map(|l| l.1.as_str()).unwrap_or("");
                let audio = s.audio.first().map(|a| a.describe()).unwrap_or_default();
                format!("{label} ({audio})")
            }
            None => String::new(),
        }
    }

    pub fn status(&self) -> String {
        let i = &self.ofdm.info;
        format!(
            "mode {} SO {} | guard corr {:.2}, offset {:+.1} Hz, clock {:+.0} ppm, frame sync {} | {} frames, FAC {} ok / {} bad, SDC {} ok / {} bad | audio frames {} ok / {} bad, {} Hz",
            i.mode.map_or("?".into(), |m| format!("{m:?}")),
            i.so,
            i.guard_corr,
            i.freq_offset,
            i.drift_ppm,
            if i.frame_found { "yes" } else { "no" },
            self.frames,
            self.fac.ok,
            self.fac.bad,
            self.sdc.ok,
            self.sdc.bad,
            if self.aac_on { self.aac.frames_ok } else { self.audio.frames_ok },
            if self.aac_on { self.aac.frames_bad } else { self.audio.frames_bad },
            if self.aac_on { self.aac.rate } else { self.audio.rate }
        )
    }
}
