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
use crate::linear::{AmDemod, Sideband, SsbDemod};
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
}

impl Mode {
    pub const ALL: [Mode; 6] = [Mode::Wfm, Mode::Nfm, Mode::Am, Mode::Usb, Mode::Lsb, Mode::Cw];

    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "wfm" | "fm" => Some(Mode::Wfm),
            "nfm" => Some(Mode::Nfm),
            "am" => Some(Mode::Am),
            "usb" => Some(Mode::Usb),
            "lsb" => Some(Mode::Lsb),
            "cw" => Some(Mode::Cw),
            _ => None,
        }
    }
    fn index(self) -> usize {
        Mode::ALL.iter().position(|m| *m == self).unwrap()
    }
    /// Intermediate rate, a multiple of 48 kHz
    fn if_rate(self) -> f64 {
        match self {
            Mode::Wfm => 240_000.0,
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
            Mode::Wfm | Mode::Nfm => None,
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
        let stop = (ifr - pass).max(pass * 1.2);
        let taps = lowpass((pass + stop) / 2.0 / rate, (stop - pass) / rate);
        Front { rate, decim: DecimFir::new(taps, d), frac: Interp::new(mid, ifr) }
    }
}

/// Demodulation, kept across rate changes
enum Back {
    Wfm { fm: FmDemod, stereo: StereoDecoder, deemph: [Deemphasis; 2] },
    Nfm { fm: FmDemod, audio: DecimFir<f32> },
    Am(AmDemod),
    Ssb(SsbDemod),
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
                }
            }
            Mode::Nfm => Back::Nfm {
                fm: FmDemod::new(ifr, mode.deviation()),
                audio: DecimFir::new(lowpass(3_900.0 / ifr, 900.0 / ifr), 1),
            },
            Mode::Am => Back::Am(AmDemod::new(bw)),
            Mode::Usb | Mode::Lsb | Mode::Cw => Back::Ssb(SsbDemod::new(mode.sideband().unwrap(), bw, pitch)),
        }
    }

    fn process(&mut self, iq: &[C32], out: &mut Vec<Frame>) {
        let mut demod = Vec::with_capacity(iq.len());
        match self {
            Back::Wfm { fm, stereo, deemph } => {
                fm.process(iq, &mut demod);
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
    bandwidth: [f64; 6], // per mode, remembered across mode changes
    pitch: f64,
    volume: f32,
    stereo: bool,
    nco: Nco,
    blanker: Option<NoiseBlanker>,
    blank_threshold: f32,
    front: Option<Front>,
    back: Back,
    pub fx: AudioChain,
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

    pub fn control(&mut self, c: &EngineCtl) {
        match c {
            EngineCtl::Tune(f) => self.freq = *f,
            EngineCtl::Volume(v) => self.volume = v.clamp(0.0, 2.0),
            EngineCtl::Mode(m) if *m != self.mode => {
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
        s + &format!(", audio latency {:.1} ms\n", self.fx.latency() as f64 / AUDIO_RATE * 1000.0) + &self.fx.describe()
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
        if let Some(b) = &mut self.blanker {
            b.process(&mut block.samples);
        }
        let front = self.front.as_mut().unwrap();
        self.offset = self.freq - block.center;
        self.nco.set(self.offset, block.rate);
        self.nco.mix(&mut block.samples);

        let mut dec = Vec::with_capacity(block.samples.len() / front.decim.factor() + 2);
        front.decim.process(&block.samples, &mut dec);
        let mut ifs = Vec::with_capacity(dec.len() + 4);
        front.frac.process(&dec, &mut ifs);
        let mut audio = Vec::with_capacity(ifs.len() / 4 + 2);
        self.back.process(&ifs, &mut audio);
        self.fx.process(&mut audio);

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

    pub fn mode(&self) -> Mode {
        self.mode
    }
}

/// Runs the engine until the source ends or `EngineCtl::Stop`.
pub fn run(blocks: Receiver<IqBlock>, ctl: Receiver<EngineCtl>, mut sink: Box<dyn AudioSink>, mut engine: Engine, verbose: bool) {
    let mut out = Vec::new();
    let mut last_report = Instant::now();
    let start = Instant::now();
    loop {
        loop {
            match ctl.try_recv() {
                Ok(EngineCtl::Stop) | Err(TryRecvError::Disconnected) => {
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
                if engine.stats.rebuilds != before && before > 0 {
                    eprintln!("[engine] rate {:.0} S/s: front end rebuilt in {} µs, audio kept running", rate, engine.stats.last_rebuild_us);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                sink.finish();
                return;
            }
        }
        if verbose && last_report.elapsed() > Duration::from_secs(5) {
            last_report = Instant::now();
            let produced = engine.stats.audio_samples as f64 / AUDIO_RATE;
            eprintln!("[engine] {:?}, {:.1} s of audio in {:.1} s, {}", engine.mode(), produced, start.elapsed().as_secs_f64(), sink.status());
        }
    }
}
