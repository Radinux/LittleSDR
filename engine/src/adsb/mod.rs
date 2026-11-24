//! ADS-B / Mode S reception on 1090 MHz, at 2 MS/s.
//!
//! Pulse position modulation, 1 µs per bit: a bit is a pulse in the first (1) or second (0)
//! half of its microsecond, so 2 samples per bit at 2 MS/s. An 8 µs preamble (pulses at 0,
//! 1, 3.5 and 4.5 µs) precedes 56 or 112 bits; the last 24 are a CRC (the parity, XORed with
//! the address in the replies to radar interrogations).
//!
//! Detection: the preamble's shape on the magnitude, then the bits by comparing the two
//! halves of each microsecond, at the found position and half a sample later (pulse timing
//! is not aligned on the samples). Extended squitters (DF17/18) are repaired when one bit is
//! wrong. Replies whose CRC carries the address (altitude, squawk) are only taken from
//! aircraft already heard with a clean address, so noise does not make ghosts.
//!
//! Aircraft are kept 60 s after their last message, with their track (positions).

pub mod countries;
pub mod cpr;
pub mod modes;

use crate::dsp::C32;
use modes::{Content, Message};
use serde_json::{json, Value};
use std::collections::HashMap;

/// ADS-B frequency
pub const FREQ: f64 = 1_090_000_000.0;
/// Sample rate the engine feeds the decoder with (any rate from 2 MS/s works)
pub const RATE: f64 = 2_400_000.0;
/// Preamble (8 µs) + 112 bits (µs)
const FRAME_US: f64 = 120.0;
/// Timing hypotheses per sample when slicing the bits
const PHASES: usize = 4;
/// Forgotten after this long without a message (s)
const EXPIRE: f64 = 60.0;
/// Even / odd position frames further apart than this cannot be combined (s)
const CPR_PAIR: f64 = 10.0;
const TRAIL_MAX: usize = 400;
/// Track points sent to the app per aircraft
const TRAIL_JSON: usize = 150;

#[derive(Clone, Debug, Default)]
pub struct Aircraft {
    pub icao: u32,
    pub non_icao: bool,
    pub callsign: String,
    pub category: String,
    pub squawk: Option<u16>,
    pub emergency: u8,
    /// Barometric altitude (ft)
    pub alt: Option<i32>,
    /// GNSS height (ft)
    pub gnss_alt: Option<i32>,
    pub on_ground: Option<bool>,
    /// Ground speed (kt) and track (deg), or airspeed and heading when `airspeed`
    pub speed: Option<f64>,
    pub track: Option<f64>,
    pub heading: Option<f64>,
    pub airspeed: bool,
    /// ft/min
    pub vrate: Option<i32>,
    pub pos: Option<(f64, f64)>,
    pub pos_time: f64,
    /// Last even / odd CPR frame: (lat, lon, time)
    even: Option<(u32, u32, f64)>,
    odd: Option<(u32, u32, f64)>,
    /// Positions: (lat, lon, alt ft, time)
    pub trail: Vec<(f64, f64, Option<i32>, f64)>,
    pub first_seen: f64,
    pub last_seen: f64,
    pub messages: u64,
    /// Signal of the last message (dBFS)
    pub rssi: f32,
    /// Heard with a clean address (DF11 / 17 / 18): replies with address parity accepted
    confirmed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub preambles: u64,
    pub messages: u64,
    pub fixed: u64,
    pub by_df: [u64; 32],
    /// Messages per second, smoothed
    pub rate: f64,
}

pub struct AdsbDecoder {
    /// Samples per µs
    spu: f64,
    mag: Vec<f32>,
    /// Running sums of `mag` (f64: a block holds ~10^5 samples)
    cum: Vec<f64>,
    /// Median magnitude: the noise floor
    noise: f64,
    pub aircraft: HashMap<u32, Aircraft>,
    pub stats: Stats,
    /// Receiver position, for distance / bearing and surface positions
    pub home: Option<(f64, f64)>,
    last_rate: (f64, u64),
    last_expire: f64,
    /// Messages decoded since the last call to take_raw (hex), for the command line
    raw: Vec<String>,
    pub keep_raw: bool,
}

impl AdsbDecoder {
    /// `rate`: samples per second, 2 MS/s or more
    pub fn new(rate: f64) -> AdsbDecoder {
        AdsbDecoder {
            spu: rate / 1e6,
            mag: Vec::new(),
            cum: vec![0.0],
            noise: 0.0,
            aircraft: HashMap::new(),
            stats: Stats::default(),
            home: None,
            last_rate: (0.0, 0),
            last_expire: 0.0,
            raw: Vec::new(),
            keep_raw: false,
        }
    }

    pub fn rate(&self) -> f64 {
        self.spu * 1e6
    }

    /// IQ centred on 1090 MHz at the decoder's rate; `now` = UTC seconds of the block
    pub fn process(&mut self, iq: &[C32], now: f64) {
        let start = self.mag.len();
        self.mag.extend(iq.iter().map(|z| z.norm_sqr().sqrt()));
        let mut c = *self.cum.last().unwrap();
        for &m in &self.mag[start..] {
            c += m as f64;
            self.cum.push(c);
        }
        if iq.len() >= 64 {
            // Median of a subsample: the noise level, even with frames filling the band
            let mut sub: Vec<f32> = self.mag[start..].iter().step_by(7).copied().collect();
            let mid = sub.len() / 2;
            let med = *sub.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1 as f64;
            self.noise = if self.noise == 0.0 { med } else { 0.8 * self.noise + 0.2 * med };
        }
        let n = self.mag.len();
        let need = (FRAME_US * self.spu).ceil() as usize + 3;
        let mut j = 0;
        while j + need < n {
            match self.detect(j) {
                Some((msg, bits, sig, end)) => {
                    self.accept(&msg[..bits / 8], sig, now);
                    j = end.max(j + 1);
                }
                None => j += 1,
            }
        }
        // Keep the tail: a frame may straddle two blocks
        self.mag.drain(..j);
        let base = self.cum[j];
        self.cum.drain(..j);
        for v in &mut self.cum {
            *v -= base;
        }
        if now - self.last_expire > 5.0 {
            self.last_expire = now;
            self.aircraft.retain(|_, a| now - a.last_seen < EXPIRE);
            let (t, c) = self.last_rate;
            if t > 0.0 {
                let r = (self.stats.messages - c) as f64 / (now - t);
                self.stats.rate = if self.stats.rate == 0.0 { r } else { 0.5 * self.stats.rate + 0.5 * r };
            }
            self.last_rate = (now, self.stats.messages);
        }
    }

    /// Integral of the magnitude over [a, b) samples (fractional ends)
    fn area(&self, a: f64, b: f64) -> f64 {
        let f = |x: f64| {
            let k = x as usize;
            self.cum[k] + (x - k as f64) * self.mag[k] as f64
        };
        f(b) - f(a)
    }

    /// Preamble quality at time t (samples): mean level in the 4 pulses over the mean level
    /// in the gaps, and the pulse level
    fn preamble(&self, t: f64) -> (f64, f64) {
        let u = self.spu;
        let w = |a: f64, b: f64| self.area(t + a * u, t + b * u);
        let pulses = w(0.0, 0.5) + w(1.0, 1.5) + w(3.5, 4.0) + w(4.5, 5.0);
        // The long gaps only, edges trimmed: at 2 MS/s a sample may straddle a pulse and a
        // gap, so the short gaps between pulses can be as bright as the pulses
        let gaps = w(1.75, 3.25) + w(5.25, 7.75);
        let hi = pulses / (2.0 * u);
        let lo = gaps / (4.0 * u);
        (hi / lo.max(1e-9), hi)
    }

    /// A preamble starting in sample `j`, then a frame with a valid CRC. Returns the message,
    /// its length, its level (dBFS) and the sample after it.
    fn detect(&mut self, j: usize) -> Option<([u8; 14], usize, f32, usize)> {
        // Cheap test first: a pulse at the start, quiet 2 µs later
        let u = self.spu;
        let m0 = self.mag[j] as f64 + self.mag[j + 1] as f64;
        if m0 < 2.0 * 2.0 * self.noise {
            return None;
        }
        let q = (j as f64 + 2.0 * u) as usize;
        if (self.mag[q] as f64 + self.mag[q + 1] as f64) * 1.5 > m0 {
            return None;
        }
        // Best timing within this sample
        let (mut best, mut bt) = (0.0, 0.0);
        for k in 0..PHASES {
            let t = j as f64 + k as f64 / PHASES as f64;
            let (score, hi) = self.preamble(t);
            if score > best && hi > 2.0 * self.noise {
                (best, bt) = (score, t);
            }
        }
        if best < 3.0 {
            return None;
        }
        self.stats.preambles += 1;
        // Slice at the best timing, then a quarter sample either side
        for dt in [0.0, -0.25, 0.25, -0.5, 0.5] {
            let t0 = bt + dt + 8.0 * u;
            if t0 < 0.0 {
                continue;
            }
            let mut msg = [0u8; 14];
            let mut sig = 0.0f64;
            let mut bits = 112;
            let mut i = 0;
            while i < bits {
                let s = t0 + i as f64 * u;
                let (a, b) = (self.area(s, s + 0.5 * u), self.area(s + 0.5 * u, s + u));
                if a > b {
                    msg[i / 8] |= 0x80 >> (i % 8);
                }
                let p = a.max(b) / (0.5 * u);
                sig += p * p;
                i += 1;
                if i == 5 {
                    bits = modes::len_bits(msg[0] >> 3);
                }
            }
            let df = msg[0] >> 3;
            if !matches!(df, 0 | 4 | 5 | 11 | 16 | 17 | 18 | 20 | 21) {
                continue;
            }
            let syn = modes::syndrome(&msg[..bits / 8]);
            let ok = match df {
                17 | 18 => {
                    syn == 0 || {
                        let r = modes::fix_one_bit(&mut msg);
                        self.stats.fixed += r as u64;
                        r
                    }
                }
                11 => syn & !0x7F == 0,
                // Address parity: valid if the address is a known aircraft
                _ => self.aircraft.get(&syn).is_some_and(|a| a.confirmed),
            };
            if ok {
                let end = (t0 + bits as f64 * u) as usize;
                return Some((msg, bits, (10.0 * (sig / bits as f64).max(1e-12).log10()) as f32, end));
            }
        }
        None
    }

    fn accept(&mut self, msg: &[u8], rssi: f32, now: f64) {
        let df = msg[0] >> 3;
        let syn = modes::syndrome(msg);
        let aa = (msg[1] as u32) << 16 | (msg[2] as u32) << 8 | msg[3] as u32;
        let icao = match df {
            11 | 17 | 18 => aa,
            _ => syn,
        };
        let d = modes::decode(msg, icao);
        // DF11 with a non-zero interrogator code, or address parity: known aircraft only
        let clean = matches!(df, 17 | 18) || (df == 11 && syn == 0);
        if !clean && !self.aircraft.get(&icao).is_some_and(|a| a.confirmed) && df != 11 {
            return;
        }
        if df == 11 && syn != 0 && !self.aircraft.contains_key(&icao) {
            return;
        }
        self.stats.messages += 1;
        self.stats.by_df[df as usize] += 1;
        if self.keep_raw {
            self.raw.push(modes::hex(msg));
        }
        let home = self.home;
        let a = self.aircraft.entry(icao).or_insert_with(|| Aircraft { icao, first_seen: now, ..Default::default() });
        a.confirmed |= clean;
        a.last_seen = now;
        a.messages += 1;
        a.rssi = rssi;
        a.non_icao |= d.non_icao;
        if let Some(g) = d.on_ground {
            a.on_ground = Some(g);
        }
        update(a, &d, now, home);
    }

    /// Aircraft heard more than once (single messages are often noise), most recent first
    pub fn list(&self) -> Vec<&Aircraft> {
        let mut v: Vec<&Aircraft> = self.aircraft.values().filter(|a| a.messages >= 2 && a.confirmed).collect();
        v.sort_by(|a, b| b.last_seen.total_cmp(&a.last_seen));
        v
    }

    pub fn clear(&mut self) {
        self.aircraft.clear();
        self.stats = Stats::default();
    }

    pub fn take_raw(&mut self) -> Vec<String> {
        std::mem::take(&mut self.raw)
    }

    pub fn describe(&self) -> String {
        let s = &self.stats;
        let pos = self.list().iter().filter(|a| a.pos.is_some()).count();
        format!(
            "ADS-B: {} aircraft ({} with position), {} messages ({:.1}/s, {} repaired), {} preambles",
            self.list().len(),
            pos,
            s.messages,
            s.rate,
            s.fixed,
            s.preambles
        )
    }

    /// The app's view: aircraft with what is known of them; `trails`: their tracks too
    pub fn json(&self, now: f64, trails: bool) -> Value {
        let list: Vec<Value> = self.list().iter().map(|a| aircraft_json(a, now, self.home, trails)).collect();
        json!({
            "active": true,
            "messages": self.stats.messages,
            "rate": (self.stats.rate * 10.0).round() / 10.0,
            "repaired": self.stats.fixed,
            "home": self.home.map(|h| vec![h.0, h.1]),
            "aircraft": list,
        })
    }

    /// One line per aircraft, for the command line
    pub fn table(&self, now: f64) -> String {
        let mut s = format!("{}\n  ICAO   flight    squawk  alt ft   kt   trk   ft/min  position              dist   msgs  seen  dBFS", self.describe());
        for a in self.list() {
            let o = |v: Option<String>| v.unwrap_or_else(|| "-".into());
            let dist = match (a.pos, self.home) {
                (Some(p), Some(h)) => format!("{:.0} km", cpr::distance_bearing(h, p).0),
                _ => "-".into(),
            };
            s += &format!(
                "\n  {:06X} {:<9} {:>6}  {:>6} {:>4} {:>5} {:>7}  {:<21} {:>6} {:>5} {:>4.0}s {:>5.1}",
                a.icao,
                a.callsign,
                o(a.squawk.map(|q| format!("{q:04X}"))),
                o(a.alt.or(a.gnss_alt).map(|v| v.to_string()).or_else(|| (a.on_ground == Some(true)).then(|| "ground".into()))),
                o(a.speed.map(|v| format!("{v:.0}"))),
                o(a.track.or(a.heading).map(|v| format!("{v:.0}"))),
                o(a.vrate.map(|v| format!("{v:+}"))),
                o(a.pos.map(|p| format!("{:.4} {:.4}", p.0, p.1))),
                dist,
                a.messages,
                now - a.last_seen,
                a.rssi
            );
        }
        s
    }
}

fn update(a: &mut Aircraft, d: &Message, now: f64, home: Option<(f64, f64)>) {
    for c in &d.content {
        match c {
            Content::Ident { callsign, category } => {
                a.callsign = callsign.clone();
                if !category.is_empty() {
                    a.category = category.clone();
                }
            }
            Content::Altitude(alt) => {
                if alt.is_some() {
                    a.alt = *alt;
                }
            }
            Content::Squawk { code, emergency } => {
                a.squawk = Some(*code);
                if d.df == 17 || d.df == 18 {
                    a.emergency = *emergency;
                }
            }
            Content::Velocity { speed, track, heading, airspeed, vrate } => {
                if speed.is_some() {
                    a.speed = *speed;
                    a.airspeed = *airspeed;
                }
                if track.is_some() {
                    a.track = *track;
                }
                if heading.is_some() {
                    a.heading = *heading;
                }
                if vrate.is_some() {
                    a.vrate = *vrate;
                }
            }
            Content::Airborne { alt, gnss, odd, lat, lon } => {
                if alt.is_some() {
                    if *gnss {
                        a.gnss_alt = *alt;
                    } else {
                        a.alt = *alt;
                    }
                }
                position(a, *odd, *lat, *lon, false, now, home);
            }
            Content::Surface { speed, track, odd, lat, lon } => {
                if speed.is_some() {
                    a.speed = *speed;
                    a.airspeed = false;
                }
                if track.is_some() {
                    a.track = *track;
                }
                a.vrate = None;
                position(a, *odd, *lat, *lon, true, now, home);
            }
            Content::Other => {}
        }
    }
}

fn position(a: &mut Aircraft, odd: bool, lat: u32, lon: u32, surface: bool, now: f64, home: Option<(f64, f64)>) {
    if odd {
        a.odd = Some((lat, lon, now));
    } else {
        a.even = Some((lat, lon, now));
    }
    // Local decoding against our last position, still fresh; on the ground against the
    // receiver (surface CPR has a 90° ambiguity a global pair cannot lift without it)
    let reference = match (a.pos, surface) {
        (Some(p), _) if now - a.pos_time < 30.0 => Some(p),
        (_, true) => home,
        _ => None,
    };
    let new = match reference {
        Some(r) => Some(cpr::local(r, (lat, lon), odd, surface)),
        None if !surface => match (a.even, a.odd) {
            (Some(e), Some(o)) if (e.2 - o.2).abs() < CPR_PAIR => cpr::global_airborne((e.0, e.1), (o.0, o.1), odd),
            _ => None,
        },
        None => None,
    };
    let Some(p) = new else { return };
    // Sanity: a jump faster than ~Mach 2.5 since the last fix. (No range check against the
    // receiver: it may be far away, a network receiver, or the locator wrong; the global
    // decoding of a CRC-checked pair is unambiguous.)
    if let Some(old) = a.pos {
        let dt = (now - a.pos_time).max(1.0);
        if cpr::distance_bearing(old, p).0 > 0.85 * dt + 2.0 {
            a.pos = None; // start again from a global pair
            return;
        }
    }
    a.pos = Some(p);
    a.pos_time = now;
    let alt = a.alt.or(a.gnss_alt);
    let moved = a.trail.last().is_none_or(|l| now - l.3 > 4.0 || cpr::distance_bearing((l.0, l.1), p).0 > 0.3);
    if moved {
        a.trail.push((p.0, p.1, alt, now));
        if a.trail.len() > TRAIL_MAX {
            a.trail.remove(0);
        }
    }
}

fn round(v: f64, k: f64) -> f64 {
    (v * k).round() / k
}

fn aircraft_json(a: &Aircraft, now: f64, home: Option<(f64, f64)>, trails: bool) -> Value {
    let mut o = json!({
        "icao": format!("{:06X}", a.icao),
        "callsign": a.callsign,
        "category": a.category,
        "country": if a.non_icao { "" } else { countries::country(a.icao) },
        "squawk": a.squawk.map(|q| format!("{q:04X}")),
        "emergency": a.emergency,
        "alt": a.alt,
        "gnssAlt": a.gnss_alt,
        "ground": a.on_ground,
        "speed": a.speed.map(|v| v.round()),
        "airspeed": a.airspeed,
        "track": a.track.map(|v| round(v, 10.0)),
        "heading": a.heading.map(|v| round(v, 10.0)),
        "vrate": a.vrate,
        "lat": a.pos.map(|p| round(p.0, 1e5)),
        "lon": a.pos.map(|p| round(p.1, 1e5)),
        "seen": round(now - a.last_seen, 10.0),
        "seenPos": a.pos.map(|_| round(now - a.pos_time, 10.0)),
        "msgs": a.messages,
        "rssi": round(a.rssi as f64, 10.0),
        "nonIcao": a.non_icao,
    });
    if let (Some(p), Some(h)) = (a.pos, home) {
        let (d, b) = cpr::distance_bearing(h, p);
        o["dist"] = json!(round(d, 10.0));
        o["bearing"] = json!(b.round());
    }
    if trails {
        // The last ~10 minutes: enough for the map, keeps the app's polling light
        let from = a.trail.len().saturating_sub(TRAIL_JSON);
        o["trail"] = json!(a.trail[from..].iter().map(|t| json!([round(t.0, 1e5), round(t.1, 1e5), t.2])).collect::<Vec<_>>());
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use modes::from_hex;

    /// PPM sampled at `rate` with a timing offset (fraction of a sample), noise and a
    /// carrier offset; each sample averages the signal over its period
    fn modulate(msgs: &[&str], amp: f32, noise: f32, rate: f64, offset: f64) -> Vec<C32> {
        let mut rng = 12345u64;
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        // Half-µs chips
        let mut chips: Vec<f32> = vec![0.0; 200];
        for m in msgs {
            let b = from_hex(m);
            chips.extend([1., 0., 1., 0., 0., 0., 0., 1., 0., 1., 0., 0., 0., 0., 0., 0.]);
            for i in 0..b.len() * 8 {
                let bit = (b[i / 8] >> (7 - i % 8)) & 1 == 1;
                chips.extend(if bit { [1., 0.] } else { [0., 1.] });
            }
            chips.extend(vec![0.0; 157]);
        }
        let spu = rate / 1e6;
        let n = (chips.len() as f64 / 2.0 * spu) as usize - 2;
        (0..n)
            .map(|k| {
                let v: f32 = (0..20).map(|s| {
                    let t = (k as f64 + offset + s as f64 / 20.0) / spu; // µs
                    chips.get((2.0 * t) as usize).copied().unwrap_or(0.0)
                }).sum::<f32>() / 20.0;
                let ph = k as f32 * 0.03;
                C32::new(v * amp * ph.cos() + noise * rnd(), v * amp * ph.sin() + noise * rnd())
            })
            .collect()
    }

    #[test]
    fn decodes_synthetic_frames() {
        let msgs = [
            "8D4840D6202CC371C32CE0576098", // KLM1023
            "8D40621D58C382D690C8AC2863A7", // even position
            "8D40621D58C386435CC412692AD6", // odd position
            "8D485020994409940838175B284F", // velocity
        ];
        for (rate, off) in [(2.0e6, 0.0), (2.0e6, 0.25), (2.4e6, 0.0), (2.4e6, 0.3), (2.4e6, 0.6), (2.5e6, 0.7), (3.0e6, 0.4)] {
            let mut d = AdsbDecoder::new(rate);
            // A receiver position far away (network receiver, wrong locator) blocks nothing
            d.home = Some((39.23, -123.13));
            let iq = modulate(&msgs, 0.3, 0.06, rate, off);
            // In uneven blocks: frames straddle them
            for (k, c) in iq.chunks(777).enumerate() {
                d.process(c, 1000.0 + k as f64 * 0.001);
            }
            assert_eq!(d.stats.messages, 4, "{rate} S/s, timing +{off}: {:?}", d.stats);
            assert_eq!(d.aircraft[&0x4840D6].callsign, "KLM1023");
            let a = &d.aircraft[&0x40621D];
            let p = a.pos.expect("position");
            // the odd frame came last: its position
            assert!((p.0 - 52.26578).abs() < 1e-4 && (p.1 - 3.93891).abs() < 1e-4, "{p:?}");
            assert_eq!(a.alt, Some(38000));
            assert_eq!(d.aircraft[&0x485020].vrate, Some(-832));
        }
    }

    #[test]
    fn repairs_and_rejects() {
        // One bit wrong in a squitter: repaired; an altitude reply from an unknown address: dropped
        let mut bad = from_hex("8D4840D6202CC371C32CE0576098");
        bad[6] ^= 0x10;
        let mut d = AdsbDecoder::new(RATE);
        let iq = modulate(&[&modes::hex(&bad), "200006A2DE8B1C"], 0.3, 0.02, RATE, 0.0);
        d.process(&iq, 0.0);
        assert_eq!(d.stats.fixed, 1);
        assert_eq!(d.stats.messages, 1);
        assert_eq!(d.aircraft[&0x4840D6].callsign, "KLM1023");
    }
}
