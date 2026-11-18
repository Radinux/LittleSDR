//! Demodulation chain with live reconfiguration.
//!
//! IQ (any rate) -> NCO (VFO offset) -> decimating FIR -> fractional resampler -> fixed IF rate
//! -> FM discriminator -> de-emphasis -> audio FIR -> 48 kHz.
//!
//! The IF rate is fixed per mode, so a new hardware sample rate only rebuilds the front end
//! (NCO step, FIR, resampler ratio). The demodulator, de-emphasis, audio filter and the sound
//! output keep their state and keep running: no gap, no click. Data that may be mislabeled
//! around the switch (rtl_tcp in-flight buffers) is faded out / in over a few ms.

use crate::audio::AudioSink;
use crate::dsp::{lowpass, Deemphasis, DecimFir, FmDemod, Interp, Nco, C32};
use crate::source::IqBlock;
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
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "wfm" | "fm" => Some(Mode::Wfm),
            "nfm" => Some(Mode::Nfm),
            _ => None,
        }
    }
    /// Intermediate rate, a multiple of 48 kHz
    fn if_rate(self) -> f64 {
        match self {
            Mode::Wfm => 240_000.0,
            Mode::Nfm => 48_000.0,
        }
    }
    /// Half the channel width kept by the front end
    fn half_band(self) -> f64 {
        match self {
            Mode::Wfm => 100_000.0,
            Mode::Nfm => 8_000.0,
        }
    }
    fn deviation(self) -> f64 {
        match self {
            Mode::Wfm => 75_000.0,
            Mode::Nfm => 5_000.0,
        }
    }
    fn audio_cutoff(self) -> f64 {
        match self {
            Mode::Wfm => 15_000.0,
            Mode::Nfm => 3_500.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum EngineCtl {
    /// VFO offset from the hardware center (Hz)
    Offset(f64),
    Mode(Mode),
    Volume(f32),
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

/// Kept across rate changes
struct Back {
    mode: Mode,
    fm: FmDemod,
    deemph: Option<Deemphasis>,
    audio: DecimFir<f32>,
}

impl Back {
    fn new(mode: Mode) -> Back {
        let ifr = mode.if_rate();
        let cut = mode.audio_cutoff();
        let factor = (ifr / AUDIO_RATE).round() as usize;
        // WFM: the 19 kHz stereo pilot must be gone, so the stop band starts at 18.5 kHz
        let stop = (cut * 1.25).min(AUDIO_RATE / 2.0 - 500.0);
        let taps = lowpass((cut + stop) / 2.0 / ifr, (stop - cut) / ifr);
        Back {
            mode,
            fm: FmDemod::new(ifr, mode.deviation()),
            deemph: (mode == Mode::Wfm).then(|| Deemphasis::new(ifr, 50e-6)),
            audio: DecimFir::new(taps, factor),
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
    offset: f64,
    volume: f32,
    nco: Nco,
    front: Option<Front>,
    back: Back,
    delay: VecDeque<f32>,
    mute: usize,    // audio samples still muted
    fade_in: usize, // ramp position after a mute (0 = none)
    pub stats: Stats,
}

impl Engine {
    pub fn new(mode: Mode, offset: f64) -> Engine {
        Engine {
            mode,
            offset,
            volume: 0.8,
            nco: Nco::new(),
            front: None,
            back: Back::new(mode),
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
            self.delay[n - f + k] *= g;
        }
        self.mute = self.mute.max((ms * AUDIO_RATE / 1000.0) as usize);
        self.fade_in = 1;
    }

    pub fn control(&mut self, c: &EngineCtl) {
        match c {
            EngineCtl::Offset(o) => self.offset = *o,
            EngineCtl::Volume(v) => self.volume = v.clamp(0.0, 2.0),
            EngineCtl::Mode(m) if *m != self.mode => {
                self.mode = *m;
                self.back = Back::new(*m);
                if let Some(f) = &self.front {
                    self.front = Some(Front::new(f.rate, *m));
                }
                self.soft_switch(5.0);
            }
            _ => {}
        }
    }

    /// Processes one IQ block, appends 48 kHz audio to `out`.
    pub fn process(&mut self, mut block: IqBlock, out: &mut Vec<f32>) {
        let rate_changed = self.front.as_ref().map_or(true, |f| f.rate != block.rate);
        if rate_changed {
            let t = Instant::now();
            let first = self.front.is_none();
            self.front = Some(Front::new(block.rate, self.mode));
            self.stats.rebuilds += 1;
            self.stats.last_rebuild_us = t.elapsed().as_micros();
            if !first {
                // Filter warm-up is a few µs; the hardware may need longer (block.settle_ms)
                self.soft_switch(block.settle_ms.max(2.0));
            }
        } else if block.settle_ms > 0.0 {
            self.soft_switch(block.settle_ms);
        }
        let front = self.front.as_mut().unwrap();
        self.nco.set(self.offset, block.rate);
        self.nco.mix(&mut block.samples);

        let mut dec = Vec::with_capacity(block.samples.len() / front.decim.factor() + 2);
        front.decim.process(&block.samples, &mut dec);
        let mut ifs = Vec::with_capacity(dec.len() + 4);
        front.frac.process(&dec, &mut ifs);
        let mut demod = Vec::with_capacity(ifs.len());
        self.back.fm.process(&ifs, &mut demod);
        if let Some(d) = &mut self.back.deemph {
            d.process(&mut demod);
        }
        let mut audio = Vec::with_capacity(demod.len() / 4 + 2);
        self.back.audio.process(&demod, &mut audio);

        // Gain, mute / fade-in, then the delay line
        for s in audio {
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
            self.delay.push_back(s * g);
        }
        while self.delay.len() > DELAY {
            out.push(self.delay.pop_front().unwrap());
        }
        self.stats.audio_samples += out.len() as u64;
    }

    pub fn mode(&self) -> Mode {
        self.back.mode
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
