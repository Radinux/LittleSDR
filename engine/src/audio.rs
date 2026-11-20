//! Audio outputs: the sound card (cpal) with clock-drift compensation, or a WAV file.

use crate::dsp::Frame;
use crate::engine::AUDIO_RATE;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};

pub trait AudioSink: Send {
    /// Stereo frames at 48 kHz
    fn write(&mut self, frames: &[Frame]);
    fn finish(&mut self) {}
    fn status(&self) -> String {
        String::new()
    }
}

/// 16-bit stereo WAV at 48 kHz (sizes patched on finish).
pub struct WavSink {
    w: BufWriter<File>,
    samples: u32,
}

impl WavSink {
    pub fn create(path: &str) -> std::io::Result<WavSink> {
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(&Self::header(0))?;
        Ok(WavSink { w, samples: 0 })
    }

    fn header(frames: u32) -> Vec<u8> {
        let data = frames * 4;
        let rate = AUDIO_RATE as u32;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&2u16.to_le_bytes()); // stereo
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * 4).to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        h
    }
}

impl AudioSink for WavSink {
    fn write(&mut self, frames: &[Frame]) {
        for f in frames {
            for s in [f.l, f.r] {
                let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                let _ = self.w.write_all(&v.to_le_bytes());
            }
        }
        self.samples += frames.len() as u32;
    }

    fn finish(&mut self) {
        let _ = self.w.flush();
        let f = self.w.get_mut();
        if f.seek(SeekFrom::Start(0)).is_ok() {
            let _ = f.write_all(&Self::header(self.samples));
        }
        let _ = f.flush();
    }

    fn status(&self) -> String {
        format!("wav {:.1} s", self.samples as f64 / AUDIO_RATE)
    }
}

#[cfg(feature = "audio")]
pub use cpal_sink::CpalSink;

#[cfg(feature = "audio")]
mod cpal_sink {
    use super::AudioSink;
    use crate::dsp::{Frame, Interp};
    use crate::engine::AUDIO_RATE;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex};

    struct Shared {
        ring: Mutex<VecDeque<Frame>>,
        underruns: AtomicU64,
        /// Samples to buffer before playing (and again after running dry)
        prime: usize,
        primed: AtomicBool,
    }

    /// Default sound card. The radio and the sound card have different clocks: the ring buffer
    /// level steers a tiny resampling ratio trim (±0.5 %) so it never runs dry or overflows.
    pub struct CpalSink {
        shared: Arc<Shared>,
        interp: Interp<Frame>,
        base_step: f64,
        target: usize,
        _stop: mpsc::Sender<()>,
        device: String,
        dev_rate: f64,
    }

    impl CpalSink {
        pub fn open() -> Result<CpalSink, String> {
            let (ready_tx, ready_rx) = mpsc::channel();
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            // The shared buffer is created by the audio thread once the device rate is known
            let (shared_tx, shared_rx) = mpsc::channel::<Arc<Shared>>();
            // cpal streams are not Send: the stream lives on its own thread
            std::thread::Builder::new()
                .name("audio".into())
                .spawn(move || {
                    let r = (|| -> Result<(cpal::Stream, String, f64), String> {
                        let host = cpal::default_host();
                        let dev = host.default_output_device().ok_or("no output device")?;
                        let name = dev.name().unwrap_or_default();
                        let sup = dev.default_output_config().map_err(|e| e.to_string())?;
                        let fmt = sup.sample_format();
                        let cfg: cpal::StreamConfig = sup.into();
                        let rate = cfg.sample_rate.0 as f64;
                        // Prime to the target margin (120 ms)
                        let sh = Arc::new(Shared {
                            ring: Mutex::new(VecDeque::new()),
                            underruns: AtomicU64::new(0),
                            prime: (rate * 0.12) as usize,
                            primed: AtomicBool::new(false),
                        });
                        let _ = shared_tx.send(sh.clone());
                        let stream = match fmt {
                            cpal::SampleFormat::F32 => build::<f32>(&dev, &cfg, sh),
                            cpal::SampleFormat::I16 => build::<i16>(&dev, &cfg, sh),
                            cpal::SampleFormat::U16 => build::<u16>(&dev, &cfg, sh),
                            other => return Err(format!("unsupported sample format {other:?}")),
                        }?;
                        stream.play().map_err(|e| e.to_string())?;
                        Ok((stream, name, rate))
                    })();
                    match r {
                        Ok((stream, name, rate)) => {
                            let _ = ready_tx.send(Ok((name, rate)));
                            let _ = stop_rx.recv(); // keep the stream alive until dropped
                            drop(stream);
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            let (device, dev_rate) = ready_rx.recv().map_err(|e| e.to_string())??;
            let shared = shared_rx.recv().map_err(|e| e.to_string())?;
            let base_step = AUDIO_RATE / dev_rate;
            Ok(CpalSink {
                shared,
                interp: Interp::new(AUDIO_RATE, dev_rate),
                base_step,
                target: (dev_rate * 0.12) as usize, // 120 ms of margin
                _stop: stop_tx,
                device,
                dev_rate,
            })
        }
    }

    fn build<T>(dev: &cpal::Device, cfg: &cpal::StreamConfig, sh: Arc<Shared>) -> Result<cpal::Stream, String>
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let ch = cfg.channels as usize;
        dev.build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut ring = sh.ring.lock().unwrap();
                // Start (or restart after running dry) only with a full margin: a steady
                // stream instead of crackles while the buffer is still filling
                if !sh.primed.load(Ordering::Relaxed) {
                    if ring.len() < sh.prime.max(data.len() / ch) {
                        data.iter_mut().for_each(|x| *x = T::from_sample(0.0f32));
                        return;
                    }
                    sh.primed.store(true, Ordering::Relaxed);
                }
                let mut dry = false;
                for frame in data.chunks_mut(ch) {
                    let s = ring.pop_front().unwrap_or_else(|| {
                        dry = true;
                        Frame::default()
                    });
                    // Left / right on the first two channels, their mix on any others
                    for (i, x) in frame.iter_mut().enumerate() {
                        *x = T::from_sample(match (i, ch) {
                            (_, 1) => (s.l + s.r) * 0.5,
                            (0, _) => s.l,
                            (1, _) => s.r,
                            _ => (s.l + s.r) * 0.5,
                        });
                    }
                }
                if dry {
                    sh.underruns.fetch_add(1, Ordering::Relaxed);
                    sh.primed.store(false, Ordering::Relaxed);
                }
            },
            |e| eprintln!("[audio] {e}"),
            None,
        )
        .map_err(|e| e.to_string())
    }

    impl AudioSink for CpalSink {
        fn write(&mut self, samples: &[Frame]) {
            let fill = self.shared.ring.lock().unwrap().len();
            // Too full: read the input faster (bigger step), too empty: slower
            let err = (fill as f64 - self.target as f64) / self.target as f64;
            self.interp.set_step(self.base_step * (1.0 + (err * 0.002).clamp(-0.005, 0.005)));
            let mut out = Vec::with_capacity(samples.len() + 8);
            self.interp.process(samples, &mut out);
            let mut ring = self.shared.ring.lock().unwrap();
            ring.extend(out);
            let max = self.target * 4;
            if ring.len() > max {
                let n = ring.len() - max;
                ring.drain(..n);
            }
        }

        fn status(&self) -> String {
            let fill = self.shared.ring.lock().unwrap().len();
            format!(
                "sound card '{}' {} Hz, buffer {:.0} ms, underruns {}",
                self.device,
                self.dev_rate,
                fill as f64 / self.dev_rate * 1000.0,
                self.shared.underruns.load(Ordering::Relaxed)
            )
        }
    }
}

/// Sound for an output driven from elsewhere (Android AAudio's callback): the engine writes,
/// the callback reads. Same adaptive resampling as the cpal sink, so the source's clock and
/// the sound card's never drift apart into underruns or a growing delay.
pub struct RingSink {
    pub shared: std::sync::Arc<RingShared>,
    interp: crate::dsp::Interp<Frame>,
    base_step: f64,
    target: usize,
}

pub struct RingShared {
    pub ring: std::sync::Mutex<std::collections::VecDeque<Frame>>,
    pub primed: std::sync::atomic::AtomicBool,
    pub prime: usize,
    pub underruns: std::sync::atomic::AtomicU64,
}

impl RingSink {
    /// `latency_ms`: buffer kept between the engine and the output
    pub fn new(dev_rate: f64, latency_ms: f64) -> RingSink {
        let target = (dev_rate * latency_ms / 1000.0) as usize;
        RingSink {
            shared: std::sync::Arc::new(RingShared {
                ring: std::sync::Mutex::new(std::collections::VecDeque::new()),
                primed: std::sync::atomic::AtomicBool::new(false),
                prime: target,
                underruns: std::sync::atomic::AtomicU64::new(0),
            }),
            interp: crate::dsp::Interp::new(48_000.0, dev_rate),
            base_step: 48_000.0 / dev_rate,
            target: target.max(256),
        }
    }
}

impl RingShared {
    /// Fills `out` (interleaved, `channels` per frame); silence while (re)filling
    pub fn read(&self, out: &mut [f32], channels: usize) {
        use std::sync::atomic::Ordering;
        let ch = channels.max(1);
        let mut ring = self.ring.lock().unwrap();
        if !self.primed.load(Ordering::Relaxed) {
            if ring.len() < self.prime.max(out.len() / ch) {
                out.iter_mut().for_each(|x| *x = 0.0);
                return;
            }
            self.primed.store(true, Ordering::Relaxed);
        }
        let mut dry = false;
        for frame in out.chunks_mut(ch) {
            let s = ring.pop_front().unwrap_or_else(|| {
                dry = true;
                Frame::default()
            });
            for (i, x) in frame.iter_mut().enumerate() {
                *x = match (i, ch) {
                    (_, 1) => (s.l + s.r) * 0.5,
                    (0, _) => s.l,
                    (1, _) => s.r,
                    _ => (s.l + s.r) * 0.5,
                };
            }
        }
        if dry {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            self.primed.store(false, Ordering::Relaxed);
        }
    }

    pub fn clear(&self) {
        self.ring.lock().unwrap().clear();
        self.primed.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl AudioSink for RingSink {
    fn write(&mut self, samples: &[Frame]) {
        let fill = self.shared.ring.lock().unwrap().len();
        let err = (fill as f64 - self.target as f64) / self.target as f64;
        self.interp.set_step(self.base_step * (1.0 + (err * 0.002).clamp(-0.005, 0.005)));
        let mut out = Vec::with_capacity(samples.len() + 8);
        self.interp.process(samples, &mut out);
        let mut ring = self.shared.ring.lock().unwrap();
        ring.extend(out);
        let max = self.target * 4;
        if ring.len() > max {
            let n = ring.len() - max;
            ring.drain(..n);
        }
    }
}
