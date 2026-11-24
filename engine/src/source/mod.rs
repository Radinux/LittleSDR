//! IQ sources. Each runs on its own thread, sends `IqBlock`s and takes `SourceCtl` commands
//! while streaming: the stream itself never stops for a setting change.

use crate::dsp::C32;
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

pub mod airspy;
pub mod kiwisdr;
pub mod sdrplay;
pub mod rtltcp;
pub mod spyserver;
pub mod wavfile;
pub mod ws;
#[cfg(feature = "rtlsdr")]
pub mod rtlsdr;

/// A block of IQ samples and the format they were produced in.
pub struct IqBlock {
    pub samples: Vec<C32>,
    /// Sample rate of these samples (Hz)
    pub rate: f64,
    /// Hardware center frequency (Hz)
    pub center: f64,
    /// Set on the first block after a sample-rate change: how long (ms of signal) the samples
    /// may still be mislabeled (data that was in flight when the rate changed), to be muted.
    pub settle_ms: f64,
    /// UTC time of the first sample (s since the epoch) when the source knows it better than
    /// the arrival time: a recording played back
    pub time: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum SourceCtl {
    Rate(u32),
    Center(u64),
    /// Tuner gain in dB, None = automatic
    Gain(Option<f32>),
    /// RTL-SDR direct sampling for HF on dongles without upconverter: 0 off, 1 I, 2 Q branch
    DirectSampling(u8),
    /// 4.5 V on the antenna input, for active antennas (loops, LNAs)
    BiasTee(bool),
    /// Files: go to this many seconds from the start
    Seek(f64),
    /// Files: pause / resume
    Pause(bool),
    /// Files: start again at the end
    Loop(bool),
    Stop,
}

/// Settings a source starts with.
#[derive(Clone, Debug)]
pub struct SourceConfig {
    pub rate: u32,
    pub center: u64,
    pub gain: Option<f32>,
    pub direct_sampling: u8,
    pub bias_tee: bool,
}

pub struct SourceHandle {
    pub ctl: Sender<SourceCtl>,
    pub thread: JoinHandle<()>,
    /// Why the source stopped by itself (server refusal, unplugged...), once it has
    pub error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

pub trait Source: Send + 'static {
    fn name(&self) -> String;
    /// Runs the source until `SourceCtl::Stop` or an error.
    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String>;
}

pub fn spawn(source: Box<dyn Source>, cfg: SourceConfig, out: SyncSender<IqBlock>) -> SourceHandle {
    let (tx, rx) = std::sync::mpsc::channel();
    let name = source.name();
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let err = error.clone();
    let thread = std::thread::Builder::new()
        .name(format!("source {name}"))
        .spawn(move || {
            if let Err(e) = source.run(cfg, out, rx) {
                eprintln!("[source] {name}: {e}");
                *err.lock().unwrap() = Some(e.to_string());
            }
        })
        .expect("spawn source thread");
    SourceHandle { ctl: tx, thread, error }
}

/// RTL-SDR bytes (unsigned 8-bit I, Q) to complex samples.
pub fn u8_to_iq(bytes: &[u8], out: &mut Vec<C32>) {
    out.extend(bytes.chunks_exact(2).map(|p| C32::new((p[0] as f32 - 127.5) / 127.5, (p[1] as f32 - 127.5) / 127.5)));
}
