//! FT8 / FT4 receiver: a USB channel of its own on the standard dial frequency in view
//! (independent of the tuned frequency, like the app's former SDR++ module), cut into UTC
//! slots, each slot decoded in the background. Results in the app's ft8Json format.

pub mod decode;
pub mod ldpc;
pub mod message;
mod tables;

use crate::dsp::{lowpass, DecimFir, Nco, C32};
use decode::{Protocol, RATE};
use message::CallHashes;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FT8_DIALS: [(&str, f64); 12] = [
    ("160 m", 1_840_000.0),
    ("80 m", 3_573_000.0),
    ("60 m", 5_357_000.0),
    ("40 m", 7_074_000.0),
    ("30 m", 10_136_000.0),
    ("20 m", 14_074_000.0),
    ("17 m", 18_100_000.0),
    ("15 m", 21_074_000.0),
    ("12 m", 24_915_000.0),
    ("10 m", 28_074_000.0),
    ("6 m", 50_323_000.0),
    ("2 m", 144_174_000.0),
];

pub const FT4_DIALS: [(&str, f64); 6] =
    [("40 m", 7_047_500.0), ("30 m", 10_140_000.0), ("20 m", 14_080_000.0), ("17 m", 18_104_000.0), ("15 m", 21_140_000.0), ("10 m", 28_180_000.0)];

#[derive(Clone, Debug)]
pub struct Ft8Result {
    pub mode: &'static str,
    /// End of the slot, ms since the epoch (UTC)
    pub time: i64,
    pub audio_hz: f64,
    /// Dial frequency in MHz, "14.074"
    pub band: String,
    pub call: String,
    pub message: String,
    pub snr: f64,
    pub dt: f64,
    pub distance_km: f64,
    pub country: String,
}

/// USB audio (12 kHz) of a dial frequency, from the wideband IQ
struct Channel {
    dial: f64,
    rate: f64,
    nco: Nco,
    stages: Vec<DecimFir<C32>>,
    up: Nco,
    /// Audio and the UTC time (s) of its first sample
    audio: VecDeque<f32>,
    t0: f64,
}

impl Channel {
    fn new(dial: f64, rate: f64) -> Option<Channel> {
        // Complex baseband centred on dial + 1500 Hz, +-1500 Hz, down to 12 kHz, then
        // shifted back up so the real part is the USB audio (0..3 kHz)
        let total = rate / RATE;
        if (total - total.round()).abs() > 1e-6 || total < 1.0 {
            return None; // needs an integer ratio (2.4 MS/s, 2.048, 1.024, 240k, 48k ...)
        }
        let total = total.round() as usize;
        let mut factors = Vec::new();
        let mut left = total;
        for f in [5, 4, 2, 3] {
            while left.is_multiple_of(f) && left > 1 {
                factors.push(f);
                left /= f;
            }
        }
        if left > 1 {
            factors.push(left);
        }
        let mut stages = Vec::new();
        let mut r = rate;
        for f in factors {
            let out = r / f as f64;
            let last = (out - RATE).abs() < 1.0;
            let (cut, tw) = if last { (1_550.0, 800.0) } else { (1_600.0, out - 3_200.0) };
            stages.push(DecimFir::new(lowpass(cut / r, (tw / r).max(0.002)), f));
            r = out;
        }
        Some(Channel { dial, rate, nco: Nco::new(), stages, up: Nco::new(), audio: VecDeque::new(), t0: 0.0 })
    }

    fn feed(&mut self, iq: &[C32], center: f64, now: f64) {
        let mut buf = iq.to_vec();
        self.nco.set(self.dial + 1_500.0 - center, self.rate);
        self.nco.mix(&mut buf);
        for st in &mut self.stages {
            let mut out = Vec::with_capacity(buf.len() / st.factor() + 2);
            st.process(&buf, &mut out);
            buf = out;
        }
        self.up.set(-1_500.0, RATE);
        self.up.mix(&mut buf);
        // Time base from the wall clock: the block just arrived, so it ends now
        let end_expected = self.t0 + (self.audio.len() + buf.len()) as f64 / RATE;
        if self.audio.is_empty() || (end_expected - now).abs() > 0.5 {
            self.audio.clear();
            self.t0 = now - buf.len() as f64 / RATE;
        }
        self.audio.extend(buf.iter().map(|z| z.re));
    }
}

#[derive(Default)]
struct ProtoState {
    processing: bool,
    dial: f64,
    on_frequency: bool,
    last_slot: i64,
    error: String,
}

pub struct Ft8Receiver {
    pub enabled: bool,
    ch: [Option<Channel>; 2],
    st: [ProtoState; 2],
    hashes: Arc<Mutex<CallHashes>>,
    results: Arc<Mutex<Vec<Ft8Result>>>,
    busy: Arc<Mutex<[bool; 2]>>,
    counts: Arc<Mutex<[usize; 2]>>,
    /// Receiver position, for distances
    pub home: Option<(f64, f64)>,
    news: Arc<Mutex<Vec<Ft8Result>>>,
}

impl Default for Ft8Receiver {
    fn default() -> Self {
        Self::new()
    }
}

fn now_s() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

const PROTOS: [Protocol; 2] = [Protocol::Ft8, Protocol::Ft4];

impl Ft8Receiver {
    pub fn new() -> Ft8Receiver {
        Ft8Receiver {
            enabled: false,
            ch: [None, None],
            st: [ProtoState { processing: true, ..Default::default() }, ProtoState { processing: true, ..Default::default() }],
            hashes: Arc::default(),
            results: Arc::default(),
            busy: Arc::default(),
            counts: Arc::default(),
            home: None,
            news: Arc::default(),
        }
    }

    /// App settings: "enabled", "ft8" / "ft4" (processing on/off), "clear", "lat" / "lon"
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "enabled" => {
                self.enabled = v != 0.0;
                if !self.enabled {
                    self.ch = [None, None];
                }
            }
            "ft8" => self.st[0].processing = v != 0.0,
            "ft4" => self.st[1].processing = v != 0.0,
            "clear" => self.results.lock().unwrap().clear(),
            "lat" => self.home = Some((v, self.home.map_or(0.0, |h| h.1))),
            "lon" => self.home = Some((self.home.map_or(0.0, |h| h.0), v)),
            "status" => {}
            _ => return false,
        }
        true
    }

    /// Wideband IQ; `vfo` = tuned frequency (to pick the band and report onFrequency)
    pub fn feed_iq(&mut self, iq: &[C32], rate: f64, center: f64, vfo: f64, usb: bool) {
        if !self.enabled || iq.is_empty() {
            return;
        }
        let now = now_s();
        let half = 0.45 * rate;
        for p in 0..2 {
            let dials: &[(&str, f64)] = if p == 0 { &FT8_DIALS } else { &FT4_DIALS };
            let dial = dials
                .iter()
                .filter(|d| d.1 > center - half && d.1 + 3_000.0 < center + half)
                .min_by(|a, b| (a.1 - vfo).abs().total_cmp(&(b.1 - vfo).abs()))
                .map(|d| d.1);
            let st = &mut self.st[p];
            st.dial = dial.unwrap_or(0.0);
            st.on_frequency = dial.is_some_and(|d| usb && vfo >= d - 100.0 && vfo <= d + 3_000.0);
            let Some(dial) = dial.filter(|_| st.processing) else {
                self.ch[p] = None;
                continue;
            };
            if self.ch[p].as_ref().is_none_or(|c| c.dial != dial || c.rate != rate) {
                self.ch[p] = Channel::new(dial, rate);
                if self.ch[p].is_none() {
                    st.error = format!("sample rate {rate} is not a multiple of 12 kHz");
                    continue;
                }
                st.error.clear();
            }
            let ch = self.ch[p].as_mut().unwrap();
            ch.feed(iq, center, now);
            self.slot_ready(p);
        }
    }

    /// 12 kHz audio whose first sample is at UTC time `t0` (files, tests)
    pub fn feed_audio(&mut self, p: usize, audio: &[f32], t0: f64, dial: f64) {
        let ch = self.ch[p].get_or_insert_with(|| Channel { dial, rate: RATE, nco: Nco::new(), stages: Vec::new(), up: Nco::new(), audio: VecDeque::new(), t0 });
        if ch.audio.is_empty() {
            ch.t0 = t0;
        }
        ch.audio.extend(audio);
        self.st[p].dial = dial;
        self.slot_ready(p);
    }

    /// Hands a complete slot to a decoding thread
    fn slot_ready(&mut self, p: usize) {
        let proto = PROTOS[p];
        let Some(ch) = self.ch[p].as_mut() else { return };
        let (slot, lead, tail) = match proto {
            Protocol::Ft8 => (15.0, 1.5, 14.6),
            Protocol::Ft4 => (7.5, 1.0, 7.3),
        };
        let end_t = ch.t0 + ch.audio.len() as f64 / RATE;
        // The latest slot whose decoding window has fully arrived
        let k = ((end_t - tail) / slot).floor();
        let slot_start = k * slot;
        let a = slot_start - lead; // window start (UTC s)
        let from = ((a - ch.t0) * RATE).round();
        let n = ((tail + lead) * RATE) as usize;
        let slot_id = k as i64;
        let mut ready = None;
        if slot_id != self.st[p].last_slot && from >= 0.0 && from as usize + n <= ch.audio.len() {
            self.st[p].last_slot = slot_id;
            let from = from as usize;
            let audio: Vec<f32> = ch.audio.range(from..from + n).copied().collect();
            ready = Some((audio, ch.dial));
        }
        // Keep two slots of audio
        let keep = ((2.0 * slot + lead) * RATE) as usize;
        if ch.audio.len() > keep {
            let drop = ch.audio.len() - keep;
            ch.audio.drain(..drop);
            ch.t0 += drop as f64 / RATE;
        }
        if let Some((audio, dial)) = ready {
            // The decoder wants the time from the window start to the nominal signal start
            self.spawn(p, audio, lead + proto.start(), slot_start, dial);
        }
    }

    fn spawn(&mut self, p: usize, audio: Vec<f32>, lead: f64, slot_start: f64, dial: f64) {
        let proto = PROTOS[p];
        let (hashes, results, busy, counts, news) = (self.hashes.clone(), self.results.clone(), self.busy.clone(), self.counts.clone(), self.news.clone());
        let home = self.home;
        busy.lock().unwrap()[p] = true;
        std::thread::spawn(move || {
            let decodes = {
                let mut h = hashes.lock().unwrap();
                decode::decode(proto, &audio, lead, &mut h, 150.0, 3_000.0)
            };
            let mut list: Vec<Ft8Result> = decodes
                .into_iter()
                .map(|d| {
                    let distance_km = match (home, message::grid_latlon(&d.msg.grid)) {
                        (Some(h), Some(g)) => message::distance_km(h, g).round(),
                        _ => 0.0,
                    };
                    Ft8Result {
                        mode: proto.name(),
                        time: ((slot_start + proto.slot()) * 1000.0) as i64,
                        audio_hz: d.freq.round(),
                        band: format!("{:.3}", dial / 1e6),
                        call: d.msg.from.clone(),
                        message: d.msg.text.clone(),
                        snr: d.snr,
                        dt: d.dt,
                        distance_km,
                        country: String::new(),
                    }
                })
                .collect();
            list.sort_by(|a, b| a.audio_hz.total_cmp(&b.audio_hz));
            counts.lock().unwrap()[p] = list.len();
            news.lock().unwrap().extend(list.iter().cloned());
            let mut r = results.lock().unwrap();
            r.extend(list);
            let over = r.len().saturating_sub(1000);
            r.drain(..over);
            busy.lock().unwrap()[p] = false;
        });
    }

    /// Decodes finished since the last call
    pub fn take_news(&mut self) -> Vec<Ft8Result> {
        std::mem::take(&mut *self.news.lock().unwrap())
    }

    pub fn results(&self) -> Vec<Ft8Result> {
        self.results.lock().unwrap().clone()
    }

    pub fn dial(&self, p: usize) -> f64 {
        self.st[p].dial
    }

    /// ft8Json
    pub fn json(&self) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let busy = *self.busy.lock().unwrap();
        let counts = *self.counts.lock().unwrap();
        let dec = |p: usize| {
            let s = &self.st[p];
            format!(
                "{{\"processing\":{},\"onFrequency\":{},\"dial\":{:.0},\"lastDecodeCount\":{},\"decoding\":{},\"error\":\"{}\"}}",
                s.processing,
                s.on_frequency,
                if self.enabled { s.dial } else { 0.0 },
                counts[p],
                busy[p],
                esc(&s.error)
            )
        };
        let res: Vec<String> = self
            .results
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                format!(
                    "{{\"mode\":\"{}\",\"time\":{},\"audioHz\":{:.0},\"band\":\"{}\",\"call\":\"{}\",\"message\":\"{}\",\"snr\":{:.0},\"distanceKm\":{:.0},\"country\":\"{}\"}}",
                    r.mode,
                    r.time,
                    r.audio_hz,
                    r.band,
                    esc(&r.call),
                    esc(&r.message),
                    r.snr,
                    r.distance_km,
                    esc(&r.country)
                )
            })
            .collect();
        format!("{{\"enabled\":{},\"ft8\":{},\"ft4\":{},\"results\":[{}]}}", self.enabled, dec(0), dec(1), res.join(","))
    }
}
