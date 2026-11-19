//! Demodulation chain with live reconfiguration.
//!
//! IQ (any rate) -> [noise blanker] -> NCO (VFO offset) -> decimating FIR -> fractional
//! resampler -> fixed IF rate -> demodulator -> switchable audio filters -> 48 kHz frames.
//! Demodulators: WFM (stereo decoder, de-emphasis), NFM, AM, USB / LSB / CW (`linear`).
//!
//! The engine is tuned by absolute frequency: the VFO offset is worked out per block from the
//! hardware center the block was captured at, so retuning inside the captured band only moves
//! the NCO, and a hardware retune is followed exactly when its samples arrive.
//!
//! The IF rate is fixed per mode, so a new hardware sample rate only rebuilds the front end
//! (NCO step, FIR, resampler ratio). The demodulator, stereo decoder, filters and the sound
//! output keep their state and keep running: no gap, no click. Data that may be mislabeled
//! around the switch (in-flight buffers) is faded out / in over a few ms.

use crate::audio::AudioSink;
use crate::audio_fx::AudioChain;
use crate::dsp::{lowpass, Deemphasis, DecimFir, FmDemod, Frame, Interp, Nco, NoiseBlanker, C32};
use crate::linear::{AmDemod, Sideband, SsbDemod, SSB_LOW_CUT};
use crate::dab::DabReceiver;
use crate::lrpt::LrptDecoder;
use crate::meter::{Meter, Reading};
use crate::rds::RdsDecoder;
use crate::rtty::Rtty;
use crate::source::IqBlock;
use crate::stereo::StereoDecoder;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

pub const AUDIO_RATE: f64 = 48_000.0;
/// Output delay line: lets a fade-out start before a switch that is only known afterwards
const DELAY: usize = 960; // 20 ms
const FADE: usize = 240; // 5 ms

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Wfm,
    Nfm,
    Am,
    Usb,
    Lsb,
    Cw,
    /// Meteor-M weather satellite pictures (no sound)
    Lrpt,
    /// DAB+ digital radio (Band III, 2.048 MS/s)
    Dab,
}

impl Mode {
    pub const ALL: [Mode; 8] = [Mode::Wfm, Mode::Nfm, Mode::Am, Mode::Usb, Mode::Lsb, Mode::Cw, Mode::Lrpt, Mode::Dab];

    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "wfm" | "fm" => Some(Mode::Wfm),
            "nfm" => Some(Mode::Nfm),
            "am" => Some(Mode::Am),
            "usb" => Some(Mode::Usb),
            "lsb" => Some(Mode::Lsb),
            "cw" => Some(Mode::Cw),
            "lrpt" | "meteor" => Some(Mode::Lrpt),
            "dab" | "dab+" => Some(Mode::Dab),
            _ => None,
        }
    }
    fn index(self) -> usize {
        Mode::ALL.iter().position(|m| *m == self).unwrap()
    }
    /// Intermediate rate, a multiple of 48 kHz
    pub fn if_rate(self) -> f64 {
        match self {
            Mode::Wfm => 240_000.0,
            Mode::Lrpt => 192_000.0, // 2.67 samples per 72 ksym/s symbol
            Mode::Dab => crate::dab::ofdm::RATE,
            _ => 48_000.0,
        }
    }
    /// Half the channel width kept by the front end (the demodulator narrows it further)
    pub fn half_band(self) -> f64 {
        match self {
            Mode::Wfm => 100_000.0,
            Mode::Nfm => 8_000.0,
            Mode::Am => 10_000.0,
            Mode::Usb | Mode::Lsb | Mode::Cw => 5_000.0,
            // 72 ksym/s, roll-off 0.6: +-57.6 kHz, plus Doppler (+-3.5 kHz)
            Mode::Lrpt => 64_000.0,
            Mode::Dab => 780_000.0,
        }
    }
    fn deviation(self) -> f64 {
        match self {
            Mode::Wfm => 75_000.0,
            _ => 5_000.0,
        }
    }
    /// Default channel bandwidth (Hz), None when it is fixed (FM)
    pub fn default_bandwidth(self) -> Option<f64> {
        match self {
            Mode::Wfm | Mode::Nfm | Mode::Lrpt | Mode::Dab => None,
            Mode::Am => Some(9_000.0),
            Mode::Usb | Mode::Lsb => Some(2_700.0),
            Mode::Cw => Some(500.0),
        }
    }
    /// Accepted bandwidth range
    pub fn bandwidth_range(self) -> (f64, f64) {
        match self {
            Mode::Am => (2_000.0, 16_000.0),
            Mode::Usb | Mode::Lsb => (500.0, 4_000.0),
            Mode::Cw => (50.0, 1_500.0),
            _ => (0.0, 0.0),
        }
    }
    fn sideband(self) -> Option<Sideband> {
        match self {
            Mode::Usb => Some(Sideband::Upper),
            Mode::Lsb => Some(Sideband::Lower),
            Mode::Cw => Some(Sideband::Cw),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum EngineCtl {
    /// Tuned frequency (Hz, absolute)
    Tune(f64),
    Mode(Mode),
    /// Channel bandwidth of the current mode (Hz; AM, SSB, CW)
    Bandwidth(f64),
    /// CW tone pitch (Hz)
    CwPitch(f64),
    Volume(f32),
    /// Broadcast FM stereo (falls back to mono without pilot anyway)
    Stereo(bool),
    /// Impulse noise blanker on the raw IQ; optional threshold (x average magnitude)
    Blanker(bool, Option<f32>),
    /// Audio filter on/off (id from AudioChain)
    Filter(String, bool),
    /// Audio filter parameter
    Param(String, String, f32),
    /// Print the signal meter every second on stderr
    Meter(bool),
    /// RTTY decoder on the demodulated audio (USB / LSB)
    Rtty(bool),
    /// RTTY parameter: shift, baud, center, rev, ita2
    RttyParam(String, f64),
    /// CW skimmer setting: "enabled" (whole band), "followVfo" (tuned frequency in CW mode)
    Cw(String, f64),
    /// Print the CW skimmer's decoded lines
    CwList,
    /// FT8/FT4 receiver setting: "enabled", "ft8", "ft4", "clear", "lat", "lon"
    Ft8(String, f64),
    /// Recorder setting: "mode" (0 baseband, 1 audio), "stereo", "start", "stop"
    Rec(String, f64),
    /// Recorder folder
    RecPath(String),
    /// SSTV setting: "enabled", "start" (mode index, for a missed VIS), "stop"
    Sstv(String, f64),
    /// LRPT: write the pictures so far (file prefix, or a dated name)
    LrptSave(Option<String>),
    /// LRPT: OQPSK (Meteor-M N2-3 / N2-4) or QPSK
    LrptOqpsk(bool),
    /// DAB: play a service (SId in hex or the start of its name)
    DabSelect(String),
    /// DAB: print the services of the ensemble
    DabList,
    /// Print the state on stderr
    Status,
    Stop,
}

/// Rebuilt on every sample-rate change
struct Front {
    rate: f64,
    decim: DecimFir<C32>,
    frac: Interp<C32>,
}

impl Front {
    fn new(rate: f64, mode: Mode) -> Front {
        let ifr = mode.if_rate();
        let d = ((rate / ifr).floor() as usize).max(1);
        let mid = rate / d as f64;
        // Pass up to the channel edge, stop where the IF would alias into the channel
        let pass = mode.half_band();
        let stop = (ifr - pass).max(pass * 1.2).min(rate / 2.0);
        // Input already narrower than the IF (e.g. a recording): nothing to filter
        let taps = if stop <= pass * 1.05 || rate <= ifr * 1.001 { vec![1.0] } else { lowpass((pass + stop) / 2.0 / rate, (stop - pass) / rate) };
        Front { rate, decim: DecimFir::new(taps, d), frac: Interp::new(mid, ifr) }
    }
}

/// Demodulation, kept across rate changes
enum Back {
    Wfm { fm: FmDemod, stereo: StereoDecoder, deemph: [Deemphasis; 2], rds: Box<RdsDecoder> },
    Nfm { fm: FmDemod, audio: DecimFir<f32> },
    Am(AmDemod),
    Ssb(SsbDemod),
    /// Silent: `rem` carries the IF samples not yet turned into (silent) audio frames
    Lrpt { dec: Box<LrptDecoder>, rem: usize },
    Dab(Box<DabReceiver>),
}

impl Back {
    fn new(mode: Mode, stereo: bool, bw: f64, pitch: f64) -> Back {
        let ifr = mode.if_rate();
        match mode {
            Mode::Wfm => {
                let mut st = StereoDecoder::new(ifr, (ifr / AUDIO_RATE) as usize);
                st.enabled = stereo;
                Back::Wfm {
                    fm: FmDemod::new(ifr, mode.deviation()),
                    stereo: st,
                    deemph: [Deemphasis::new(AUDIO_RATE, 50e-6), Deemphasis::new(AUDIO_RATE, 50e-6)],
                    rds: Box::new(RdsDecoder::new(ifr)),
                }
            }
            Mode::Nfm => Back::Nfm {
                fm: FmDemod::new(ifr, mode.deviation()),
                audio: DecimFir::new(lowpass(3_900.0 / ifr, 900.0 / ifr), 1),
            },
            Mode::Am => Back::Am(AmDemod::new(bw)),
            Mode::Usb | Mode::Lsb | Mode::Cw => Back::Ssb(SsbDemod::new(mode.sideband().unwrap(), bw, pitch)),
            Mode::Lrpt => Back::Lrpt { dec: Box::new(LrptDecoder::new(ifr, true)), rem: 0 },
            Mode::Dab => Back::Dab(Box::new(DabReceiver::new())),
        }
    }

    fn process(&mut self, iq: &[C32], out: &mut Vec<Frame>) {
        let mut demod = Vec::with_capacity(iq.len());
        match self {
            Back::Wfm { fm, stereo, deemph, rds } => {
                fm.process(iq, &mut demod);
                rds.process(&demod);
                let start = out.len();
                stereo.process(&demod, out);
                // De-emphasis per channel, after the stereo matrix
                let (mut l, mut r): (Vec<f32>, Vec<f32>) = out[start..].iter().map(|f| (f.l, f.r)).unzip();
                deemph[0].process(&mut l);
                deemph[1].process(&mut r);
                for (f, (l, r)) in out[start..].iter_mut().zip(l.into_iter().zip(r)) {
                    *f = Frame::new(l, r);
                }
            }
            Back::Nfm { fm, audio } => {
                fm.process(iq, &mut demod);
                let mut a = Vec::with_capacity(demod.len());
                audio.process(&demod, &mut a);
                out.extend(a.into_iter().map(Frame::mono));
            }
            Back::Am(d) => d.process(iq, out),
            Back::Ssb(d) => d.process(iq, out),
            Back::Dab(rx) => rx.process(iq, out),
            Back::Lrpt { dec, rem } => {
                dec.process(iq);
                // Keep the sound output running (silence) at exactly 48 kHz
                let per = (Mode::Lrpt.if_rate() / AUDIO_RATE) as usize;
                *rem += iq.len();
                out.extend(std::iter::repeat(Frame::default()).take(*rem / per));
                *rem %= per;
            }
        }
    }
}

pub struct Stats {
    pub audio_samples: u64,
    pub rebuilds: u32,
    pub last_rebuild_us: u128,
}

pub struct Engine {
    mode: Mode,
    freq: f64,
    offset: f64,
    bandwidth: [f64; 8], // per mode, remembered across mode changes
    pitch: f64,
    volume: f32,
    stereo: bool,
    nco: Nco,
    blanker: Option<NoiseBlanker>,
    blank_threshold: f32,
    front: Option<Front>,
    back: Back,
    pub fx: AudioChain,
    /// Confirm every command on stderr (command-line use)
    pub echo: bool,
    meter: Meter,
    pub meter_echo: bool,
    pub rtty: Option<Rtty>,
    /// Morse skimmer (whole CW segment + the tuned frequency)
    pub cw: crate::cw::CwSkimmer,
    /// Analog SSTV pictures from the demodulated sound
    pub sstv: crate::sstv::Decoder,
    /// Sound or baseband IQ to WAV
    pub rec: crate::recorder::Recorder,
    /// FT8 / FT4 on the standard dial frequencies in view
    pub ft8: crate::ft8::Ft8Receiver,
    /// DAB service asked for before DAB mode was on
    pending_service: Option<String>,
    rtty_cfg: Vec<(String, f64)>, // kept so the decoder comes back as it was
    delay: VecDeque<Frame>,
    mute: usize,    // audio samples still muted
    fade_in: usize, // ramp position after a mute (0 = none)
    pub stats: Stats,
}

impl Engine {
    pub fn new(mode: Mode, freq: f64) -> Engine {
        let bandwidth = Mode::ALL.map(|m| m.default_bandwidth().unwrap_or(0.0));
        let pitch = 700.0;
        Engine {
            mode,
            freq,
            offset: 0.0,
            bandwidth,
            pitch,
            volume: 0.8,
            stereo: true,
            nco: Nco::new(),
            blanker: None,
            blank_threshold: 6.0,
            front: None,
            back: Back::new(mode, true, bandwidth[mode.index()], pitch),
            fx: AudioChain::new(),
            echo: false,
            meter: Meter::new(),
            meter_echo: false,
            rtty: None,
            cw: crate::cw::CwSkimmer::new(),
            sstv: crate::sstv::Decoder::new(),
            rec: crate::recorder::Recorder::new(),
            ft8: crate::ft8::Ft8Receiver::new(),
            pending_service: None,
            rtty_cfg: Vec::new(),
            delay: VecDeque::with_capacity(DELAY * 4),
            mute: 0,
            fade_in: 0,
            stats: Stats { audio_samples: 0, rebuilds: 0, last_rebuild_us: 0 },
        }
    }

    /// Fades the newest samples of the delay line to silence, then mutes `ms` of output.
    fn soft_switch(&mut self, ms: f64) {
        let n = self.delay.len();
        let f = FADE.min(n);
        for k in 0..f {
            let g = 1.0 - (k + 1) as f32 / f as f32;
            self.delay[n - f + k] = self.delay[n - f + k] * g;
        }
        self.mute = self.mute.max((ms * AUDIO_RATE / 1000.0) as usize);
        self.fade_in = 1;
    }

    /// Applies a command and confirms it on stderr (`echo`), with the value actually used
    pub fn control(&mut self, c: &EngineCtl) {
        self.apply(c);
        if !self.echo {
            return;
        }
        let msg = match c {
            EngineCtl::Volume(_) => format!("volume {:.2}", self.volume),
            EngineCtl::Mode(_) => match self.mode.default_bandwidth() {
                Some(_) => format!("mode {:?}, bandwidth {:.0} Hz", self.mode, self.bandwidth[self.mode.index()]),
                None => format!("mode {:?}", self.mode),
            },
            EngineCtl::Bandwidth(_) if self.mode.default_bandwidth().is_some() => {
                format!("bandwidth {:.0} Hz", self.bandwidth[self.mode.index()])
            }
            EngineCtl::CwPitch(_) => format!("CW pitch {:.0} Hz", self.pitch),
            EngineCtl::Cw(..) => format!(
                "CW skimmer {}, tuned frequency {}",
                if self.cw.enabled { "on (whole CW segment of the band in view)" } else { "off" },
                if self.cw.follow_vfo { "decoded in CW mode (text to stdout)" } else { "not decoded" }
            ),
            EngineCtl::CwList => self.cw_lines(),
            EngineCtl::Rec(..) | EngineCtl::RecPath(_) => {
                if !self.rec.error.is_empty() {
                    format!("recorder: {}", self.rec.error)
                } else if self.rec.recording() {
                    format!("recording -> {}", self.rec.file())
                } else {
                    format!("not recording ({} mode, IQ width {:?}, folder {})", if self.rec.mode == crate::recorder::RecMode::Audio { "audio" } else { "baseband" }, self.rec.bb_width, self.rec.path.display())
                }
            }
            EngineCtl::Ft8(..) => {
                if !self.ft8.enabled {
                    "FT8/FT4 off".into()
                } else {
                    let d = |p: usize| match self.ft8.dial(p) {
                        0.0 => "no channel in view".to_string(),
                        f => format!("dial {:.3} MHz", f / 1e6),
                    };
                    format!("FT8 {}, FT4 {} (decodes printed at the end of each slot)", d(0), d(1))
                }
            }
            EngineCtl::Sstv(..) => {
                let st = self.sstv.status();
                if !self.sstv.enabled {
                    "SSTV off".into()
                } else if st.receiving {
                    format!("SSTV: receiving {} ({} rows)", st.mode, st.line)
                } else {
                    "SSTV on, waiting for a VIS header (pictures saved as sstv-*.png)".into()
                }
            }
            EngineCtl::Meter(on) => format!("meter {}", if *on { "on" } else { "off" }),
            EngineCtl::Rtty(_) | EngineCtl::RttyParam(..) => match (&self.rtty, c) {
                (Some(r), _) => format!("{} (decoded text goes to stdout)", r.describe()),
                (None, EngineCtl::RttyParam(k, v)) => format!("RTTY {k} {v}: kept for when RTTY is on"),
                (None, _) => "RTTY off".into(),
            },
            EngineCtl::Stereo(_) => format!("stereo {}", if self.stereo { "on" } else { "off" }),
            EngineCtl::Blanker(..) => match &self.blanker {
                Some(b) => format!("noise blanker on (x{:.0})", b.threshold),
                None => "noise blanker off".into(),
            },
            EngineCtl::Filter(id, _) | EngineCtl::Param(id, ..) => match self.fx.stages.iter().find(|s| s.id == id) {
                Some(s) => format!("{} {}  {}", s.label, if s.enabled { "ON" } else { "off" }, s.describe()),
                None => return,
            },
            _ => return,
        };
        eprintln!("[engine] {msg}");
    }

    fn apply(&mut self, c: &EngineCtl) {
        match c {
            EngineCtl::Tune(f) => {
                if (*f - self.freq).abs() > 50e3 {
                    if let Back::Wfm { rds, .. } = &mut self.back {
                        rds.reset(); // another station
                    }
                }
                self.freq = *f;
            }
            EngineCtl::Volume(v) => self.volume = v.clamp(0.0, 2.0),
            EngineCtl::Mode(m) if *m != self.mode && *m == Mode::Dab => {
                self.mode = *m;
                let mut rx = DabReceiver::new();
                if let Some(w) = self.pending_service.take() {
                    rx.select(&w);
                }
                self.back = Back::Dab(Box::new(rx));
                if let Some(f) = &self.front {
                    self.front = Some(Front::new(f.rate, *m));
                }
                self.soft_switch(5.0);
            }
            EngineCtl::Mode(m) if *m != self.mode => {
                if self.mode == Mode::Lrpt {
                    self.save_lrpt(None); // the pictures would be lost with the decoder
                }
                let same_front = m.if_rate() == self.mode.if_rate() && m.half_band() == self.mode.half_band();
                self.mode = *m;
                match (&mut self.back, m.sideband()) {
                    // USB <-> LSB <-> CW: same demodulator, only its band moves; no gap
                    (Back::Ssb(d), Some(sb)) => {
                        d.set_sideband(sb);
                        d.set_bandwidth(self.bandwidth[m.index()]);
                    }
                    _ => {
                        self.back = Back::new(*m, self.stereo, self.bandwidth[m.index()], self.pitch);
                        self.soft_switch(5.0);
                    }
                }
                if let Some(r) = &mut self.rtty {
                    r.set_sideband(*m != Mode::Lsb); // mark = higher RF frequency
                }
                if let (Some(f), false) = (&self.front, same_front) {
                    self.front = Some(Front::new(f.rate, *m));
                }
            }
            EngineCtl::Bandwidth(bw) => {
                let (lo, hi) = self.mode.bandwidth_range();
                if hi == 0.0 {
                    eprintln!("[engine] the bandwidth is fixed in {:?}", self.mode);
                    return;
                }
                let bw = bw.clamp(lo, hi);
                self.bandwidth[self.mode.index()] = bw;
                match &mut self.back {
                    Back::Am(d) => d.set_bandwidth(bw),
                    Back::Ssb(d) => d.set_bandwidth(bw),
                    _ => {}
                }
            }
            EngineCtl::CwPitch(hz) => {
                self.pitch = hz.clamp(200.0, 1500.0);
                if let Back::Ssb(d) = &mut self.back {
                    d.set_pitch(self.pitch);
                }
            }
            EngineCtl::Stereo(on) => {
                self.stereo = *on;
                if let Back::Wfm { stereo, .. } = &mut self.back {
                    stereo.enabled = *on; // the decoder fades between mono and stereo
                }
            }
            EngineCtl::Blanker(on, thr) => {
                if let Some(t) = thr {
                    self.blank_threshold = t.clamp(2.0, 50.0);
                }
                self.blanker = on.then(|| NoiseBlanker::new(self.front.as_ref().map_or(2.4e6, |f| f.rate), self.blank_threshold));
            }
            EngineCtl::Filter(id, on) => {
                if !self.fx.enable(id, *on) {
                    eprintln!("[engine] unknown filter '{id}'");
                }
            }
            EngineCtl::Param(id, key, v) => {
                if !self.fx.set(id, key, *v) {
                    eprintln!("[engine] unknown parameter '{id} {key}'");
                }
            }
            EngineCtl::Meter(on) => self.meter_echo = *on,
            EngineCtl::Cw(key, v) => {
                if !self.cw.set(key, *v) {
                    eprintln!("[cw] unknown setting {key} (enabled, followVfo)");
                }
            }
            EngineCtl::CwList => {}
            EngineCtl::Rec(key, v) => {
                if !self.rec.set(key, *v) {
                    eprintln!("[rec] unknown setting {key} (mode, stereo, start, stop)");
                }
            }
            EngineCtl::RecPath(p) => self.rec.set_path(p),
            EngineCtl::Ft8(key, v) => {
                if !self.ft8.set(key, *v) {
                    eprintln!("[ft8] unknown setting {key} (enabled, ft8, ft4, clear, lat, lon)");
                }
            }
            EngineCtl::Sstv(key, v) => {
                if !self.sstv.set(key, *v) {
                    eprintln!("[sstv] unknown setting {key} (enabled, start, stop)");
                }
            }
            EngineCtl::Rtty(on) => {
                self.rtty = on.then(|| {
                    let mut r = Rtty::new();
                    for (k, v) in &self.rtty_cfg {
                        r.set(k, *v);
                    }
                    r.set_sideband(self.mode != Mode::Lsb);
                    r
                });
            }
            EngineCtl::RttyParam(k, v) => {
                self.rtty_cfg.retain(|(key, _)| key != k);
                self.rtty_cfg.push((k.clone(), *v));
                if let Some(r) = &mut self.rtty {
                    if !r.set(k, *v) {
                        eprintln!("[engine] unknown RTTY parameter '{k}' (shift, baud, center, rev, ita2, sql)");
                    }
                }
            }
            EngineCtl::LrptSave(prefix) => {
                self.save_lrpt(prefix.as_deref());
            }
            EngineCtl::LrptOqpsk(oq) => {
                if let Back::Lrpt { dec, .. } = &mut self.back {
                    dec.demod = crate::lrpt::demod::Demod::new(Mode::Lrpt.if_rate(), *oq);
                    eprintln!("[lrpt] {}", if *oq { "OQPSK" } else { "QPSK" });
                }
            }
            EngineCtl::DabSelect(what) => match &mut self.back {
                Back::Dab(rx) => {
                    rx.select(what);
                    self.pending_service = None;
                }
                _ => self.pending_service = Some(what.clone()),
            },
            EngineCtl::DabList => match &self.back {
                Back::Dab(rx) => {
                    let list = rx.services();
                    if list.is_empty() {
                        eprintln!("[dab] no ensemble yet");
                    }
                    for (i, sv) in list.iter().enumerate() {
                        let sub = sv.subch.and_then(|c| rx.fic.ensemble.subchannels.get(&c));
                        eprintln!(
                            "[dab] {:2}  {:04X}  {:16}  {}",
                            i + 1,
                            sv.sid,
                            sv.label,
                            match (sv.ascty, sub.and_then(|c| c.bitrate())) {
                                (63, Some(b)) => format!("DAB+ {b} kbit/s"),
                                (0, _) => "DAB (MP2, not supported)".into(),
                                _ => "data".into(),
                            }
                        );
                    }
                }
                _ => eprintln!("[dab] not in DAB mode (mode dab)"),
            },
            EngineCtl::Status => eprintln!("{}", self.status()),
            _ => {}
        }
    }

    pub fn status(&self) -> String {
        let mut s = format!("[engine] {:.4} MHz {:?}", self.freq / 1e6, self.mode);
        match &self.back {
            Back::Am(d) => s += &format!(", bandwidth {:.0} Hz", d.bandwidth()),
            Back::Ssb(d) => {
                s += &format!(", bandwidth {:.0} Hz, AGC {:+.0} dB", d.bandwidth(), d.agc_gain_db());
                if self.mode == Mode::Cw {
                    s += &format!(", pitch {:.0} Hz", self.pitch);
                }
            }
            _ => {}
        }
        if let Back::Lrpt { dec, .. } = &self.back {
            s += &format!("\n[lrpt] {}\n", dec.describe());
        }
        if let Back::Dab(rx) = &self.back {
            s += &format!("\n[dab] {}\n", rx.describe());
        }
        if let Back::Wfm { rds, .. } = &self.back {
            let i = &rds.info;
            s += &format!(
                "\n[rds] {} PI {} PS \"{}\" PTY \"{}\" RT \"{}\" ({} groups, blocks {} ok / {} bad)\n",
                if rds.active() { "synced" } else { "searching" },
                i.pi.map_or("-".into(), |p| format!("{p:04X}")),
                i.ps.trim(),
                crate::rds::PTY_NAMES[i.pty as usize & 31],
                i.rt.trim(),
                i.groups,
                i.blocks_ok,
                i.blocks_bad
            );
        }
        if let Back::Wfm { stereo, .. } = &self.back {
            s += &format!(
                ", stereo {} (pilot {:.3}, {})",
                if self.stereo { "on" } else { "off" },
                stereo.pilot_level(),
                if stereo.locked() { "locked" } else { "not locked: mono" }
            );
        }
        match &self.blanker {
            Some(b) => s += &format!(", noise blanker x{:.0} ({} samples blanked, avg {:.4})", b.threshold, b.blanked, b.avg),
            None => s += ", noise blanker off",
        }
        s += &format!(", audio latency {:.1} ms\n", self.fx.latency() as f64 / AUDIO_RATE * 1000.0);
        if let Some(r) = self.reading() {
            s += &format!("[meter] {}\n", Self::format_reading(&r));
        }
        if let Some(r) = &self.rtty {
            s += &format!("[rtty] {}\n", r.describe());
        }
        s + &self.fx.describe()
    }

    /// Channel limits relative to the tuned frequency (Hz)
    fn channel(&self) -> (f64, f64) {
        let bw = self.bandwidth[self.mode.index()];
        match self.mode {
            Mode::Wfm => (-90e3, 90e3),
            Mode::Nfm => (-6.25e3, 6.25e3),
            Mode::Am | Mode::Cw => (-bw / 2.0, bw / 2.0),
            Mode::Usb => (SSB_LOW_CUT, SSB_LOW_CUT + bw),
            Mode::Lsb => (-SSB_LOW_CUT - bw, -SSB_LOW_CUT),
            Mode::Lrpt => (-57_600.0, 57_600.0),
            Mode::Dab => (-768_000.0, 768_000.0),
        }
    }

    /// Latest signal meter reading (refreshed 4 times a second)
    pub fn reading(&self) -> Option<Reading> {
        self.meter.reading
    }

    pub fn format_reading(r: &Reading) -> String {
        let mut s = format!(
            "signal {:6.1} dBFS, noise {:6.1} dBFS, SNR {:5.1} dB | ADC {:5.1} dBFS, {:.2} % clipped",
            r.signal_dbfs, r.noise_dbfs, r.snr_db, r.adc_dbfs, r.clip_pct
        );
        if let Some(a) = r.gain_advice() {
            s += &format!("  << {a}");
        }
        s
    }

    /// Processes one IQ block, appends 48 kHz stereo frames to `out`.
    pub fn process(&mut self, mut block: IqBlock, out: &mut Vec<Frame>) {
        let rate_changed = self.front.as_ref().map_or(true, |f| f.rate != block.rate);
        if rate_changed {
            let t = Instant::now();
            let first = self.front.is_none();
            self.front = Some(Front::new(block.rate, self.mode));
            if let Some(b) = &mut self.blanker {
                b.set_rate(block.rate);
            }
            self.stats.rebuilds += 1;
            self.stats.last_rebuild_us = t.elapsed().as_micros();
            if !first {
                // Filter warm-up is a few µs; the hardware may need longer (block.settle_ms)
                self.soft_switch(block.settle_ms.max(2.0));
            }
        } else if block.settle_ms > 0.0 {
            self.soft_switch(block.settle_ms);
        }
        self.offset = self.freq - block.center;
        let (lo, hi) = self.channel();
        self.meter.feed(&block.samples, block.rate, self.offset + lo, self.offset + hi);
        if let Some(b) = &mut self.blanker {
            b.process(&mut block.samples);
        }
        // The skimmer looks at the whole band, before the tuned channel is cut out
        self.cw.feed_iq(&block.samples, block.rate, block.center, self.freq);
        self.ft8.feed_iq(&block.samples, block.rate, block.center, self.freq, self.mode == Mode::Usb);
        self.rec.feed_iq(&block.samples, block.rate, block.center, self.freq, self.mode.if_rate());
        let front = self.front.as_mut().unwrap();
        self.nco.set(self.offset, block.rate);
        self.nco.mix(&mut block.samples);

        let mut dec = Vec::with_capacity(block.samples.len() / front.decim.factor() + 2);
        front.decim.process(&block.samples, &mut dec);
        let mut ifs = Vec::with_capacity(dec.len() + 4);
        front.frac.process(&dec, &mut ifs);
        let mut audio = Vec::with_capacity(ifs.len() / 4 + 2);
        self.back.process(&ifs, &mut audio);
        if let Some(r) = &mut self.rtty {
            // Decoders take the demodulated sound, before the listening filters
            let mono: Vec<f32> = audio.iter().map(|f| f.l).collect();
            r.process(&mono);
        }
        if self.sstv.enabled {
            let mono: Vec<f32> = audio.iter().map(|f| 0.5 * (f.l + f.r)).collect();
            self.sstv.feed(&mono, AUDIO_RATE);
        }
        if self.cw_focus_active() {
            let mono: Vec<f32> = audio.iter().map(|f| f.l).collect();
            self.cw.feed_focus(&mono, self.pitch);
        }
        self.fx.process(&mut audio);
        self.rec.feed_audio(&audio);

        // Volume, mute / fade-in, then the delay line
        for f in audio {
            let mut g = self.volume;
            if self.mute > 0 {
                self.mute -= 1;
                g = 0.0;
            } else if self.fade_in > 0 {
                g *= self.fade_in as f32 / FADE as f32;
                self.fade_in += 1;
                if self.fade_in > FADE {
                    self.fade_in = 0;
                }
            }
            self.delay.push_back(f * g);
        }
        while self.delay.len() > DELAY {
            out.push(self.delay.pop_front().unwrap());
        }
        self.stats.audio_samples += out.len() as u64;
    }

    /// LRPT / DAB progress line, when in those modes
    pub fn lrpt_status(&self) -> Option<String> {
        match &self.back {
            Back::Lrpt { dec, .. } => Some(dec.describe()),
            Back::Dab(rx) => Some(rx.describe()),
            _ => None,
        }
    }

    /// RDS of the current FM station, in the app's rdsJson format
    pub fn rds_json(&self) -> String {
        match &self.back {
            Back::Wfm { rds, .. } => rds.info.to_json(rds.active()),
            _ => crate::rds::RdsInfo::default().to_json(false),
        }
    }

    /// DAB: new programme text and new slideshow picture since the last call
    pub fn take_dab_news(&mut self) -> (Option<String>, Option<crate::dab::pad::Slide>, String) {
        match &mut self.back {
            Back::Dab(rx) => {
                let name = rx.playing().map(|s| s.label.clone()).unwrap_or_default();
                (rx.take_label(), rx.take_slide(), name)
            }
            _ => (None, None, String::new()),
        }
    }

    /// Writes the LRPT pictures decoded so far; the files written
    pub fn save_lrpt(&self, prefix: Option<&str>) -> Vec<String> {
        let Back::Lrpt { dec, .. } = &self.back else { return Vec::new() };
        if dec.imager.decoded == 0 {
            eprintln!("[lrpt] no picture to save yet");
            return Vec::new();
        }
        let prefix = prefix.map(str::to_string).unwrap_or_else(|| format!("meteor-{}", utc_stamp()));
        match dec.save(&prefix) {
            Ok(files) => {
                eprintln!("[lrpt] saved {}", files.join(", "));
                files
            }
            Err(e) => {
                eprintln!("[lrpt] cannot save the pictures: {e}");
                Vec::new()
            }
        }
    }

    /// Text decoded since the last call (RTTY)
    pub fn take_text(&mut self) -> String {
        let mut t = self.rtty.as_mut().map(|r| r.take_text()).unwrap_or_default();
        t.push_str(&self.cw.take_text());
        t
    }

    /// The tuned frequency is decoded (CW mode, skimmer on, follow VFO)
    pub fn cw_focus_active(&self) -> bool {
        self.mode == Mode::Cw && self.cw.enabled && self.cw.follow_vfo
    }

    /// CW skimmer state, in the app's cwJson format
    pub fn cw_json(&self) -> String {
        self.cw.json(self.cw_focus_active())
    }

    fn cw_lines(&self) -> String {
        let lines = self.cw.lines();
        if !self.cw.enabled {
            return "CW skimmer off (cw on)".into();
        }
        let mut s = format!("CW skimmer: {} channels, {} lines", self.cw.live_channels(), lines.len());
        for (f, t, _) in lines {
            let tail: String = t.chars().rev().take(70).collect::<Vec<_>>().into_iter().rev().collect();
            s += &format!("\n  {:>10.2} kHz  {tail}", f / 1e3);
        }
        s
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
}

/// Runs the engine until the source ends or `EngineCtl::Stop`.
pub fn run(blocks: Receiver<IqBlock>, ctl: Receiver<EngineCtl>, mut sink: Box<dyn AudioSink>, mut engine: Engine, verbose: bool) {
    let mut out = Vec::new();
    let mut last_report = Instant::now();
    let mut last_meter = Instant::now();
    let mut last_advice: Option<Instant> = None;
    let mut last_lrpt = Instant::now();
    let start = Instant::now();
    let (mut sstv_saved, mut sstv_announced) = (0u64, 0u64);
    loop {
        loop {
            match ctl.try_recv() {
                Ok(EngineCtl::Stop) | Err(TryRecvError::Disconnected) => {
                    engine.save_lrpt(None);
                    sink.finish();
                    return;
                }
                Ok(c) => engine.control(&c),
                Err(TryRecvError::Empty) => break,
            }
        }
        match blocks.recv_timeout(Duration::from_millis(200)) {
            Ok(b) => {
                let rate = b.rate;
                let before = engine.stats.rebuilds;
                out.clear();
                engine.process(b, &mut out);
                sink.write(&out);
                let (label, slide, service) = engine.take_dab_news();
                if let Some(l) = label {
                    eprintln!("[dab] ♪ {l}");
                }
                if let Some(sl) = slide {
                    let safe: String = service.trim().chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
                    let ext = if sl.mime == "image/png" { "png" } else { "jpg" };
                    let path = format!("dab-{}.{ext}", if safe.is_empty() { "slide".into() } else { safe });
                    match std::fs::write(&path, &sl.data) {
                        Ok(()) => eprintln!("[dab] picture \"{}\" ({} bytes) -> {path}", sl.name, sl.data.len()),
                        Err(e) => eprintln!("[dab] cannot write {path}: {e}"),
                    }
                }
                for r in engine.ft8.take_news() {
                    let secs = (r.time / 1000 - if r.mode == "FT4" { 7 } else { 15 }).rem_euclid(86_400);
                    let dist = if r.distance_km > 0.0 { format!("  {:.0} km", r.distance_km) } else { String::new() };
                    println!(
                        "{:02}{:02}{:02} {} {:>4.0} {:>5.1} {:>5.0}  {}{}",
                        secs / 3600,
                        secs % 3600 / 60,
                        secs % 60,
                        r.mode,
                        r.snr,
                        r.dt,
                        r.audio_hz,
                        r.message,
                        dist
                    );
                }
                let st = engine.sstv.status();
                if st.finished != sstv_saved {
                    sstv_saved = st.finished;
                    let path = format!("sstv-{}-{}.png", utc_stamp(), st.finished_mode.replace(' ', "").to_lowercase());
                    if let Some(png) = engine.sstv.png(1) {
                        match std::fs::write(&path, png) {
                            Ok(()) => eprintln!(
                                "[sstv] {} picture {} -> {path}",
                                st.finished_mode,
                                if st.last_complete { "complete".to_string() } else { format!("cut after {} rows", st.finished_rows) }
                            ),
                            Err(e) => eprintln!("[sstv] cannot write {path}: {e}"),
                        }
                    }
                }
                if st.receiving && st.line == 0 && st.version != sstv_announced {
                    sstv_announced = st.version;
                    eprintln!("[sstv] receiving {}", st.mode);
                }
                let text = engine.take_text();
                if !text.is_empty() {
                    use std::io::Write;
                    let mut so = std::io::stdout().lock();
                    let _ = so.write_all(text.as_bytes());
                    let _ = so.flush();
                }
                if engine.stats.rebuilds != before && before > 0 {
                    eprintln!("[engine] rate {:.0} S/s: front end rebuilt in {} µs, audio kept running", rate, engine.stats.last_rebuild_us);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                engine.save_lrpt(None);
                sink.finish();
                return;
            }
        }
        if last_meter.elapsed() >= Duration::from_secs(1) {
            last_meter = Instant::now();
            if let Some(r) = engine.reading() {
                if engine.meter_echo {
                    eprintln!("[meter] {}", Engine::format_reading(&r));
                } else if let Some(a) = r.gain_advice() {
                    // Unasked, but at most every 10 s
                    if last_advice.map_or(true, |t| t.elapsed() > Duration::from_secs(10)) {
                        last_advice = Some(Instant::now());
                        eprintln!("[engine] {a} ({:.1} % clipped, ADC {:.0} dBFS)", r.clip_pct, r.adc_dbfs);
                    }
                }
            }
        }
        if last_lrpt.elapsed() >= Duration::from_secs(10) {
            last_lrpt = Instant::now();
            if let Some(st) = engine.lrpt_status() {
                eprintln!("[{}] {st}", if engine.mode() == Mode::Dab { "dab" } else { "lrpt" });
            }
        }
        if verbose && last_report.elapsed() > Duration::from_secs(5) {
            last_report = Instant::now();
            let produced = engine.stats.audio_samples as f64 / AUDIO_RATE;
            eprintln!("[engine] {:?}, {:.1} s of audio in {:.1} s, {}", engine.mode(), produced, start.elapsed().as_secs_f64(), sink.status());
        }
    }
}

/// "20261119-0132" in UTC, for file names (no date library)
fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}", rem / 3600, rem % 3600 / 60)
}
