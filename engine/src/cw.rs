//! CW (Morse) skimmer: decodes every keyed signal of the band's CW segment at once, plus the
//! tuned frequency in CW mode.
//!
//! - The segment is mixed down, decimated, and cut into ~50 Hz bins by an FFT every 4 ms.
//! - A scanner looks at each bin's last few seconds: a Morse signal switches between two
//!   levels, so its loud frames stand well above its quiet ones (an unkeyed carrier and the
//!   noise do not). Each such signal gets a channel.
//! - Each channel feeds its bins' envelope to a classical Morse decoder: adaptive on/off
//!   threshold, then mark/space timing against a dot length that follows the sender's speed.
//!
//! Output matches the app's cwJson (formerly TinyCW).

use crate::dsp::{lowpass, DecimFir, Nco, C32};
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

/// Decoder thresholds between the quiet and loud levels (with hysteresis)
const ON_FRAC: f32 = 0.4;
const OFF_FRAC: f32 = 0.24;

/// Typical CW segments (IARU Region 1 band plan); 40 m widened for contests
pub struct CwBand {
    pub name: &'static str,
    pub lo: f64,
    pub hi: f64,
}

pub const CW_BANDS: [CwBand; 12] = [
    CwBand { name: "160m", lo: 1_810_000.0, hi: 1_838_000.0 },
    CwBand { name: "80m", lo: 3_500_000.0, hi: 3_570_000.0 },
    CwBand { name: "60m", lo: 5_354_000.0, hi: 5_366_000.0 },
    CwBand { name: "40m", lo: 7_000_000.0, hi: 7_060_000.0 },
    CwBand { name: "30m", lo: 10_100_000.0, hi: 10_130_000.0 },
    CwBand { name: "20m", lo: 14_000_000.0, hi: 14_070_000.0 },
    CwBand { name: "17m", lo: 18_068_000.0, hi: 18_095_000.0 },
    CwBand { name: "15m", lo: 21_000_000.0, hi: 21_070_000.0 },
    CwBand { name: "12m", lo: 24_890_000.0, hi: 24_915_000.0 },
    CwBand { name: "10m", lo: 28_000_000.0, hi: 28_070_000.0 },
    CwBand { name: "6m", lo: 50_000_000.0, hi: 50_100_000.0 },
    CwBand { name: "2m", lo: 144_000_000.0, hi: 144_150_000.0 },
];

/// The band being listened to: among those in view, the one containing `pick` (the VFO),
/// else the nearest.
pub fn pick_band(pick: f64, span_lo: f64, span_hi: f64) -> Option<&'static CwBand> {
    CW_BANDS
        .iter()
        .filter(|b| b.lo < span_hi && b.hi > span_lo)
        .min_by(|a, b| {
            let d = |x: &CwBand| if pick < x.lo { x.lo - pick } else if pick > x.hi { pick - x.hi } else { 0.0 };
            d(a).total_cmp(&d(b))
        })
}

/// International Morse, with the usual punctuation and prosigns
const TABLE: [(&str, &str); 57] = [
    (".-", "A"),
    ("-...", "B"),
    ("-.-.", "C"),
    ("-..", "D"),
    (".", "E"),
    ("..-.", "F"),
    ("--.", "G"),
    ("....", "H"),
    ("..", "I"),
    (".---", "J"),
    ("-.-", "K"),
    (".-..", "L"),
    ("--", "M"),
    ("-.", "N"),
    ("---", "O"),
    (".--.", "P"),
    ("--.-", "Q"),
    (".-.", "R"),
    ("...", "S"),
    ("-", "T"),
    ("..-", "U"),
    ("...-", "V"),
    (".--", "W"),
    ("-..-", "X"),
    ("-.--", "Y"),
    ("--..", "Z"),
    ("-----", "0"),
    (".----", "1"),
    ("..---", "2"),
    ("...--", "3"),
    ("....-", "4"),
    (".....", "5"),
    ("-....", "6"),
    ("--...", "7"),
    ("---..", "8"),
    ("----.", "9"),
    (".-.-.-", "."),
    ("--..--", ","),
    ("..--..", "?"),
    ("-..-.", "/"),
    ("-...-", "="),
    (".-.-.", "+"),
    ("-....-", "-"),
    (".----.", "'"),
    ("-.--.", "("),
    ("-.--.-", ")"),
    (".-..-.", "\""),
    ("---...", ":"),
    ("-.-.-.", ";"),
    (".--.-.", "@"),
    ("-.-.--", "!"),
    ("..--.-", "_"),
    ("...-.-", "<SK>"),
    (".-...", "<AS>"),
    ("-.--.-.", "<KN>"),
    ("...-.", "<SN>"),
    ("........", "<HH>"),
];

pub fn morse(code: &str) -> Option<&'static str> {
    TABLE.iter().find(|e| e.0 == code).map(|e| e.1)
}

/// Morse elements of a character (for tests and synthetic senders)
pub fn encode(ch: char) -> Option<&'static str> {
    let up = ch.to_ascii_uppercase().to_string();
    TABLE.iter().find(|e| e.1 == up).map(|e| e.0)
}

/// Morse from an envelope sampled at a fixed frame rate (~250 Hz).
pub struct MorseDecoder {
    fs: f32,
    /// Last raw frames, for a moving average of about a third of a dot
    raw: VecDeque<f32>,
    hist: VecDeque<f32>,
    hist_len: usize,
    tick: usize,
    lo: f32,
    hi: f32,
    signal: bool,
    key: bool,
    pending: usize,
    run: usize,
    /// Dot length (frames)
    dit: f32,
    /// How much longer marks measure than they are (and spaces shorter), frames
    bias: f32,
    marks: VecDeque<f32>,
    /// Marks (true) and the spaces between them (frames) of the word being sent: it is
    /// decoded at its end, with the speed known by then
    word: Vec<(bool, f32)>,
    /// Characters decoded since the last `take`
    out: String,
    /// Marks seen (any signal at all)
    pub marks_seen: u64,
}

impl MorseDecoder {
    pub fn new(fs: f64) -> MorseDecoder {
        let fs = fs as f32;
        MorseDecoder {
            fs,
            raw: VecDeque::new(),
            hist: VecDeque::new(),
            hist_len: (4.0 * fs) as usize,
            tick: 0,
            lo: 0.0,
            hi: 0.0,
            signal: false,
            key: false,
            pending: 0,
            run: 0,
            dit: 0.06 * fs,
            bias: 0.0,
            marks: VecDeque::new(),
            word: Vec::new(),
            out: String::new(),
            marks_seen: 0,
        }
    }

    /// Words per minute (PARIS: one dot = 1.2 s / WPM)
    pub fn wpm(&self) -> f32 {
        1.2 * self.fs / self.dit
    }

    /// Levels show keying (and not just noise or a steady carrier)
    pub fn has_signal(&self) -> bool {
        self.signal
    }

    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// One frame of envelope (amplitude)
    pub fn process(&mut self, a: f32) {
        self.raw.push_front(a);
        self.raw.truncate(16);
        let l = ((self.dit / 3.0).round() as usize).clamp(1, self.raw.len());
        let s = self.raw.iter().take(l).sum::<f32>() / l as f32;
        self.hist.push_back(s);
        if self.hist.len() > self.hist_len {
            self.hist.pop_front();
        }
        self.tick += 1;
        if self.tick as f32 >= 0.1 * self.fs {
            self.tick = 0;
            self.levels();
        }
        let raw = self.signal && {
            let span = self.hi - self.lo;
            s > self.lo + if self.key { OFF_FRAC } else { ON_FRAC } * span
        };
        self.run += 1;
        if raw != self.key {
            self.pending += 1;
            if self.pending >= 2 {
                let ended = (self.run - self.pending) as f32;
                self.key = raw;
                self.run = self.pending;
                self.pending = 0;
                if !raw {
                    self.word.push((true, ended));
                } else if !self.word.is_empty() {
                    self.word.push((false, ended));
                }
            }
        } else {
            self.pending = 0;
        }
        if !self.key {
            self.space(self.run as f32);
        } else if self.run as f32 > 10.0 * self.dit {
            self.word.clear(); // a held carrier, not Morse
        }
    }

    fn levels(&mut self) {
        if (self.hist.len() as f32) < self.fs {
            self.signal = false;
            return;
        }
        let mut v: Vec<f32> = self.hist.iter().copied().collect();
        let n = v.len();
        let (i_lo, i_hi) = (n * 35 / 100, n * 92 / 100);
        let hi = *v.select_nth_unstable_by(i_hi, f32::total_cmp).1;
        let lo = *v[..i_hi].select_nth_unstable_by(i_lo, f32::total_cmp).1;
        self.lo = lo;
        self.hi = hi;
        // Noise alone reaches ~2.4x between these percentiles; hysteresis so a weak signal
        // does not drop out in the middle of a word
        let gate = if self.signal { 2.6 } else { 3.0 };
        self.signal = hi > gate * lo.max(1e-30);
    }

    fn space(&mut self, r: f32) {
        if self.word.is_empty() {
            return;
        }
        if r > 5.0 * self.dit {
            self.decode_word();
            self.out.push(' ');
        } else if self.word.len() >= 80 && r > 2.5 * self.dit {
            self.decode_word(); // no word spacing: don't hold the text back forever
        }
    }

    /// The word's marks and spaces, cleaned of noise: marks too short to be a dot become
    /// space, gaps too short to be one become mark, and neighbours of a kind are merged
    fn clean(word: &[(bool, f32)], dit: f32) -> Vec<(bool, f32)> {
        let mut v: Vec<(bool, f32)> = Vec::with_capacity(word.len());
        for (i, &(mark, d)) in word.iter().enumerate() {
            let inner = i > 0 && i + 1 < word.len();
            let kind = if d < 0.4 * dit && (mark || inner) { !mark } else { mark };
            match v.last_mut() {
                Some(last) if last.0 == kind => last.1 += d,
                _ => v.push((kind, d)),
            }
        }
        while v.first().is_some_and(|e| !e.0) {
            v.remove(0);
        }
        v
    }

    fn decode_word(&mut self) {
        let word = std::mem::take(&mut self.word);
        // Speed: from the history plus this word's raw marks (cleaning with a wrong estimate
        // would hide the right one; the fit ignores noise blips by itself)
        let marks: Vec<f32> = word.iter().filter(|e| e.0 && e.1 >= 3.0).map(|e| e.1).collect();
        if marks.is_empty() {
            return;
        }
        self.update_dit(&marks);
        let word = Self::clean(&word, self.dit);
        let (mthr, sthr) = (2.0 * self.dit + self.bias, 2.0 * self.dit - self.bias);
        let mut code = String::new();
        let flush = |code: &mut String, out: &mut String| {
            if let Some(t) = morse(code) {
                out.push_str(t);
            }
            code.clear();
        };
        for &(mark, d) in &word {
            if mark {
                self.marks_seen += 1;
                if code.len() < 8 {
                    code.push(if d > mthr { '-' } else { '.' });
                }
            } else if d > sthr {
                flush(&mut code, &mut self.out);
                if d > 5.0 * self.dit - self.bias && !self.out.ends_with(' ') {
                    self.out.push(' ');
                }
            }
        }
        flush(&mut code, &mut self.out);
        if std::env::var("CW_DEBUG").is_ok() {
            let w: Vec<String> = word.iter().map(|e| format!("{}{:.0}", if e.0 { "M" } else { "s" }, e.1)).collect();
            eprintln!("dit {:.1} bias {:.1} lo {:.3} hi {:.3} | {} -> {:?}", self.dit, self.bias, self.lo, self.hi, w.join(" "), self.out);
        }
    }

    /// Dot length: the one that best explains the recent marks as 1 or 3 dots. A slight
    /// preference for the current value settles words of dots only or dashes only.
    fn update_dit(&mut self, new_marks: &[f32]) {
        for &m in new_marks {
            self.marks.push_back(m);
        }
        while self.marks.len() > 40 {
            self.marks.pop_front();
        }
        let g = |u: f32| (-u * u / (2.0 * 0.2 * 0.2)).exp();
        let ln3 = 3f32.ln();
        let (lo, hi) = (0.025 * self.fs, 0.24 * self.fs);
        let prior = 0.1 * self.marks.len() as f32;
        let mean = self.marks.iter().sum::<f32>() / self.marks.len() as f32;
        let mut best = (f32::MIN, self.dit, 0.0);
        for i in 0..96 {
            let d = lo * (hi / lo).powf(i as f32 / 95.0);
            for j in -4..=4 {
                let b = j as f32 * 0.1 * d;
                let mut sc = prior * (g((d / self.dit).ln()) + 0.5 * g(j as f32 / 4.0));
                for &m in &self.marks {
                    // Weighted by length: noise blips are short and many
                    let x = ((m - b).max(0.1) / d).ln();
                    sc += m / mean * g(x).max(g(x - ln3));
                }
                if sc > best.0 {
                    best = (sc, d, b);
                }
            }
        }
        let k = if self.marks_seen < 12 { 1.0 } else { 0.3 };
        self.dit = (self.dit.ln() + k * (best.1.ln() - self.dit.ln())).exp().clamp(lo, hi);
        self.bias += k * (best.2 - self.bias);
    }
}

/// The tuned frequency in CW mode, from the demodulated sound (tone at the pitch)
struct Focus {
    phase: f64,
    acc: C32,
    n: usize,
    halves: [C32; 2],
    pows: [f32; 3],
    dec: MorseDecoder,
    text: String,
}

const FOCUS_HOP: usize = 192; // 4 ms at 48 kHz

impl Focus {
    fn new() -> Focus {
        Focus {
            phase: 0.0,
            acc: C32::default(),
            n: 0,
            halves: [C32::default(); 2],
            pows: [0.0; 3],
            dec: MorseDecoder::new(48_000.0 / FOCUS_HOP as f64),
            text: String::new(),
        }
    }

    fn feed(&mut self, audio: &[f32], pitch: f64) -> String {
        let step = -2.0 * std::f64::consts::PI * pitch / 48_000.0;
        let mut new = String::new();
        for &x in audio {
            let (s, c) = self.phase.sin_cos();
            self.acc += C32::new(x * c as f32, x * s as f32);
            self.phase = (self.phase + step).rem_euclid(2.0 * std::f64::consts::PI);
            self.n += 1;
            if self.n == FOCUS_HOP {
                self.n = 0;
                self.halves = [self.halves[1], self.acc];
                self.acc = C32::default();
                // 8 ms window every 4 ms, power averaged over three of them
                let p = (self.halves[0] + self.halves[1]).norm_sqr();
                self.pows = [self.pows[1], self.pows[2], p];
                self.dec.process((self.pows.iter().sum::<f32>() / 3.0).sqrt());
                new.push_str(&self.dec.take());
            }
        }
        push_text(&mut self.text, &new, 400);
        new
    }
}

fn push_text(text: &mut String, new: &str, cap: usize) {
    for c in new.chars() {
        if c == ' ' && (text.is_empty() || text.ends_with(' ')) {
            continue;
        }
        text.push(c);
    }
    if text.len() > cap {
        let cut = text.len() - cap;
        let cut = (cut..text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(text.len());
        text.drain(..cut);
    }
}

/// Smallest FFT size >= n made of factors 2, 3 and 5 (fast in rustfft)
fn smooth_size(n: usize) -> usize {
    (n.max(64)..).find(|&k| {
        let mut k = k;
        for p in [2, 3, 5] {
            while k % p == 0 {
                k /= p;
            }
        }
        k == 1
    }).unwrap()
}

/// Mixer, decimation and the FFT over the decoded range
struct Channelizer {
    rate: f64,
    lo: f64,
    hi: f64,
    band: &'static CwBand,
    mid: f64,
    nco: Nco,
    stages: Vec<DecimFir<C32>>,
    fs: f64,
    fft: Arc<dyn Fft<f32>>,
    n: usize,
    hop: usize,
    win: Vec<f32>,
    fifo: Vec<C32>,
    bin_hz: f64,
    k0: i64,
    nb: usize,
    /// Power per bin, last HIST frames (ring)
    hist: Vec<f32>,
    head: usize,
    filled: usize,
    since_scan: usize,
    scratch: Vec<Complex32>,
}

const HIST_SEC: f64 = 3.0;
/// FFT resolution: narrow bins hold less noise, but blur fast keying
const BIN_HZ: f64 = 30.0;
const SCAN_SEC: f64 = 0.5;
/// Keyed: loud frames >= this much above quiet ones, and above the band's noise
const GATE_DB: f32 = 8.0;
const MAX_CHANNELS: usize = 32;
const MIN_SPACING_HZ: f64 = 250.0;
const CHANNEL_IDLE_SEC: f64 = 15.0;
const ROW_HOLD_SEC: f64 = 24.0;
const FRESH_SEC: f64 = 2.0;

impl Channelizer {
    fn new(rate: f64, band: &'static CwBand, lo: f64, hi: f64) -> Channelizer {
        let width = hi - lo + 2_000.0; // whole channels at the edges
        let d_total = ((rate / (1.3 * width)).floor() as usize).max(1);
        let mut stages = Vec::new();
        let mut r = rate;
        let factors: Vec<usize> = if d_total >= 8 { vec![d_total / 4, 4] } else { vec![d_total] };
        for d in factors {
            if d <= 1 {
                continue;
            }
            let out = r / d as f64;
            let taps = lowpass(width / 2.0 / r, ((out - width) / r).max(0.002));
            stages.push(DecimFir::new(taps, d));
            r = out;
        }
        let n = smooth_size((r / BIN_HZ).round() as usize);
        let hop = (r / 250.0).round().max(1.0) as usize;
        let fs = r / hop as f64;
        let bin_hz = r / n as f64;
        let mid = 0.5 * (lo + hi);
        let k0 = ((lo - mid) / bin_hz).floor() as i64 - 2;
        let nb = ((hi - lo) / bin_hz).ceil() as usize + 5;
        let h = (HIST_SEC * fs) as usize;
        let win = (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect();
        Channelizer {
            rate,
            lo,
            hi,
            band,
            mid,
            nco: Nco::new(),
            stages,
            fs,
            fft: FftPlanner::new().plan_fft_forward(n),
            n,
            hop,
            win,
            fifo: Vec::new(),
            bin_hz,
            k0,
            nb,
            hist: vec![0.0; h * nb],
            head: 0,
            filled: 0,
            since_scan: 0,
            scratch: vec![Complex32::default(); n],
        }
    }

    fn frames_kept(&self) -> usize {
        self.hist.len() / self.nb
    }

    fn freq_of(&self, bin: f64) -> f64 {
        self.mid + (self.k0 as f64 + bin) * self.bin_hz
    }

    fn row(&self, back: usize) -> &[f32] {
        let h = self.frames_kept();
        let i = (self.head + h - 1 - back) % h;
        &self.hist[i * self.nb..(i + 1) * self.nb]
    }
}

struct Channel {
    bin: usize,
    freq: f64,
    dec: MorseDecoder,
    last_good: f64,
}

struct Row {
    f: f64,
    /// ASCII text, and when each character arrived
    text: String,
    stamps: VecDeque<f64>,
    last: f64,
}

impl Row {
    /// Characters of the last FRESH_SEC
    fn fresh(&self, now: f64) -> &str {
        let n = self.stamps.iter().rev().take_while(|&&t| now - t <= FRESH_SEC).count();
        &self.text[self.text.len() - n..]
    }
}

pub struct CwSkimmer {
    pub enabled: bool,
    /// Decode the tuned frequency in CW mode, besides the whole band
    pub follow_vfo: bool,
    chan: Option<Channelizer>,
    channels: Vec<Channel>,
    rows: Vec<Row>,
    focus: Focus,
    /// Seconds of band signal processed
    time: f64,
    busy: f64,
    news: String,
}

impl Default for CwSkimmer {
    fn default() -> Self {
        Self::new()
    }
}

impl CwSkimmer {
    pub fn new() -> CwSkimmer {
        CwSkimmer {
            enabled: false,
            follow_vfo: true,
            chan: None,
            channels: Vec::new(),
            rows: Vec::new(),
            focus: Focus::new(),
            time: 0.0,
            busy: 0.0,
            news: String::new(),
        }
    }

    /// App settings: "enabled", "followVfo"
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "enabled" => {
                self.enabled = v != 0.0;
                if !self.enabled {
                    self.chan = None;
                    self.channels.clear();
                }
            }
            "followVfo" => self.follow_vfo = v != 0.0,
            _ => return false,
        }
        true
    }

    /// Wideband IQ as it comes from the source
    pub fn feed_iq(&mut self, iq: &[C32], rate: f64, center: f64, vfo: f64) {
        if !self.enabled || iq.is_empty() {
            return;
        }
        let t0 = Instant::now();
        let half = 0.45 * rate;
        let range = pick_band(vfo, center - half, center + half).map(|b| (b, b.lo.max(center - half), b.hi.min(center + half)));
        let Some((band, lo, hi)) = range.filter(|r| r.2 - r.1 > 500.0) else {
            self.chan = None;
            self.channels.clear();
            return;
        };
        let same = self.chan.as_ref().is_some_and(|c| c.rate == rate && (c.lo - lo).abs() < 1.0 && (c.hi - hi).abs() < 1.0);
        if !same {
            self.chan = Some(Channelizer::new(rate, band, lo, hi));
            self.channels.clear();
        }
        let ch = self.chan.as_mut().unwrap();
        let mut buf = iq.to_vec();
        ch.nco.set(ch.mid - center, rate);
        ch.nco.mix(&mut buf);
        for st in ch.stages.iter_mut() {
            let mut out = Vec::with_capacity(buf.len() / st.factor() + 2);
            st.process(&buf, &mut out);
            buf = out;
        }
        ch.fifo.extend_from_slice(&buf);
        self.frames();
        let spent = t0.elapsed().as_secs_f64() / (iq.len() as f64 / rate);
        self.busy = 0.95 * self.busy + 0.05 * spent * 100.0;
    }

    fn frames(&mut self) {
        let Some(ch) = self.chan.as_mut() else { return };
        let mut start = 0;
        while ch.fifo.len() - start >= ch.n {
            for (k, (z, w)) in ch.fifo[start..start + ch.n].iter().zip(&ch.win).enumerate() {
                ch.scratch[k] = Complex32::new(z.re * w, z.im * w);
            }
            ch.fft.process(&mut ch.scratch);
            let h = ch.frames_kept();
            let nb = ch.nb;
            let row = &mut ch.hist[ch.head * nb..(ch.head + 1) * nb];
            for (j, p) in row.iter_mut().enumerate() {
                *p = ch.scratch[(ch.k0 + j as i64).rem_euclid(ch.n as i64) as usize].norm_sqr();
            }
            ch.head = (ch.head + 1) % h;
            ch.filled = (ch.filled + 1).min(h);
            start += ch.hop;
            self.time += 1.0 / ch.fs;
            let row = ch.row(0);
            for c in self.channels.iter_mut() {
                let p = row[c.bin];
                c.dec.process(p.sqrt());
                let new = c.dec.take();
                if !new.is_empty() {
                    add_row(&mut self.rows, c.freq, &new, self.time);
                }
            }
            ch.since_scan += 1;
            if ch.since_scan as f64 >= SCAN_SEC * ch.fs {
                ch.since_scan = 0;
                scan(ch, &mut self.channels, self.time);
            }
        }
        ch.fifo.drain(..start);
        let now = self.time;
        self.rows.retain(|r| now - r.last <= ROW_HOLD_SEC);
    }

    /// Demodulated sound (48 kHz) of the CW mode, tone at `pitch`
    pub fn feed_focus(&mut self, audio: &[f32], pitch: f64) {
        let new = self.focus.feed(audio, pitch);
        self.news.push_str(&new);
    }

    /// Focus text decoded since the last call
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.news)
    }

    pub fn focus_text(&self) -> &str {
        &self.focus.text
    }

    pub fn focus_wpm(&self) -> Option<f32> {
        self.focus.dec.has_signal().then(|| self.focus.dec.wpm())
    }

    /// Decoded rows: (frequency Hz, text, newest characters)
    pub fn lines(&self) -> Vec<(f64, &str, &str)> {
        let mut v: Vec<(f64, &str, &str)> =
            self.rows.iter().map(|r| (r.f, r.text.as_str(), r.fresh(self.time))).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    }

    pub fn live_channels(&self) -> usize {
        self.channels.len()
    }

    /// cwJson: {"ready","enabled","band","inBand","lo","hi","channels","busy",
    /// "focus":{"active","text"},"lines":[{"f","text","fresh"}]}
    pub fn json(&self, focus_active: bool) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let (band, in_band, lo, hi) = match &self.chan {
            Some(c) => (c.band.name, true, c.lo, c.hi),
            None => ("", false, 0.0, 0.0),
        };
        let lines: Vec<String> = self
            .lines()
            .iter()
            .map(|(f, t, fr)| format!("{{\"f\":{:.0},\"text\":\"{}\",\"fresh\":\"{}\"}}", f, esc(t), esc(fr)))
            .collect();
        format!(
            "{{\"ready\":true,\"enabled\":{},\"band\":\"{}\",\"inBand\":{},\"lo\":{:.0},\"hi\":{:.0},\"channels\":{},\"busy\":{:.1},\"focus\":{{\"active\":{},\"text\":\"{}\"}},\"lines\":[{}]}}",
            self.enabled,
            band,
            in_band,
            lo,
            hi,
            self.channels.len(),
            self.busy,
            focus_active,
            esc(if focus_active { &self.focus.text } else { "" }),
            lines.join(",")
        )
    }
}

fn add_row(rows: &mut Vec<Row>, f: f64, new: &str, now: f64) {
    let i = match rows.iter().position(|r| (r.f - f).abs() < 100.0) {
        Some(i) => i,
        None => {
            if new.trim().is_empty() {
                return;
            }
            rows.push(Row { f, text: String::new(), stamps: VecDeque::new(), last: now });
            rows.len() - 1
        }
    };
    let r = &mut rows[i];
    for c in new.chars().filter(|c| c.is_ascii()) {
        if c == ' ' && (r.text.is_empty() || r.text.ends_with(' ')) {
            continue;
        }
        r.text.push(c);
        r.stamps.push_back(now);
        r.last = now;
    }
    while r.text.len() > 200 {
        r.text.remove(0);
        r.stamps.pop_front();
    }
}

/// Which bins carry keyed signals; opens, re-centres and closes channels
fn scan(ch: &Channelizer, channels: &mut Vec<Channel>, now: f64) {
    let h = ch.filled;
    if (h as f64) < 0.6 * HIST_SEC * ch.fs {
        return;
    }
    let nb = ch.nb;
    let mut p_hi = vec![0.0f32; nb];
    let mut p_lo = vec![f32::MAX; nb];
    let mut series = vec![0.0f32; h];
    for j in 1..nb - 1 {
        // 16 ms moving average: steadier statistics, so a lower gate
        let mut acc = [0.0f32; 4];
        for (t, s) in series.iter_mut().enumerate() {
            acc[t % 4] = ch.row(t)[j];
            *s = acc.iter().sum();
        }
        let i_hi = h * 90 / 100;
        p_hi[j] = *series.select_nth_unstable_by(i_hi, f32::total_cmp).1;
        p_lo[j] = *series[..i_hi].select_nth_unstable_by(h * 35 / 100, f32::total_cmp).1;
    }
    let mut sorted: Vec<f32> = p_lo[1..nb - 1].to_vec();
    let mid = sorted.len() / 2;
    let floor = *sorted.select_nth_unstable_by(mid, f32::total_cmp).1;
    let gate = 10f32.powf(GATE_DB / 10.0);
    let reach = ((120.0 / ch.bin_hz).round() as usize).max(1);
    let keyed = |j: usize| p_hi[j] >= gate * p_lo[j] && p_hi[j] >= gate * floor;
    let mut cands: Vec<usize> = (1..nb - 1)
        .filter(|&j| keyed(j))
        .filter(|&j| (j.saturating_sub(reach).max(1)..=(j + reach).min(nb - 2)).all(|k| p_hi[k] <= p_hi[j]))
        .collect();
    cands.sort_by(|&a, &b| p_hi[b].total_cmp(&p_hi[a]));

    // Existing channels follow their signal by a bin or two
    for c in channels.iter_mut() {
        if let Some(&j) = cands.iter().find(|&&j| j.abs_diff(c.bin) <= 2) {
            // Move only for a clearly better bin: hopping between two equal ones changes
            // the level under the decoder
            if p_hi[j] > 1.5 * p_hi[c.bin] {
                c.bin = j;
            }
            c.last_good = now;
        } else if keyed(c.bin) {
            c.last_good = now;
        }
    }
    channels.retain(|c| now - c.last_good <= CHANNEL_IDLE_SEC);
    let spacing = (MIN_SPACING_HZ / ch.bin_hz).ceil() as usize;
    for j in cands {
        if channels.len() >= MAX_CHANNELS {
            break;
        }
        if channels.iter().any(|c| c.bin.abs_diff(j) < spacing) {
            continue;
        }
        // Peak between bins (parabola through the 3 powers)
        let (a, b, c) = (p_hi[j - 1], p_hi[j], p_hi[j + 1]);
        let den = a - 2.0 * b + c;
        let frac = if den < 0.0 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        let freq = (ch.freq_of(j as f64 + frac as f64) / 10.0).round() * 10.0;
        let mut dec = MorseDecoder::new(ch.fs);
        // Start from the recent past, so the first characters are not lost
        for t in (0..h.min((2.0 * ch.fs) as usize)).rev() {
            let r = ch.row(t);
            dec.process(r[j].sqrt());
        }
        let _ = dec.take(); // already-partial text: avoid a garbled start
        channels.push(Channel { bin: j, freq, dec, last_good: now });
    }
    for c in channels.iter_mut() {
        c.freq = (ch.freq_of(c.bin as f64) / 10.0).round() * 10.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On/off intervals (seconds) of a text at `wpm`
    fn keying(text: &str, wpm: f64) -> Vec<(bool, f64)> {
        let dit = 1.2 / wpm;
        let mut v = Vec::new();
        for (wi, word) in text.split(' ').enumerate() {
            if wi > 0 {
                v.push((false, 7.0 * dit));
            }
            for (ci, ch) in word.chars().enumerate() {
                if ci > 0 {
                    v.push((false, 3.0 * dit));
                }
                let code = encode(ch).unwrap();
                for (ei, e) in code.chars().enumerate() {
                    if ei > 0 {
                        v.push((false, dit));
                    }
                    v.push((true, if e == '.' { dit } else { 3.0 * dit }));
                }
            }
        }
        v.push((false, 14.0 * dit));
        v
    }

    /// Key state at time t (repeats the message), with 4 ms soft edges
    fn key_at(iv: &[(bool, f64)], t: f64) -> f32 {
        let total: f64 = iv.iter().map(|x| x.1).sum();
        let mut t = t % total;
        for &(on, d) in iv {
            if t < d {
                if !on {
                    return 0.0;
                }
                let e = (t.min(d - t) / 0.004).min(1.0);
                return (0.5 - 0.5 * (std::f64::consts::PI * e).cos()) as f32;
            }
            t -= d;
        }
        0.0
    }

    fn rng(seed: &mut u32) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        (*seed as f32 + 0.5) / 4_294_967_296.0
    }

    fn gauss(seed: &mut u32) -> f32 {
        let (u, v) = (rng(seed), rng(seed));
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f32::consts::PI * v).cos()
    }

    #[test]
    fn table_round_trip() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?=.,".chars() {
            assert_eq!(morse(encode(c).unwrap()), Some(c.to_string().as_str()));
        }
    }

    #[test]
    fn decodes_envelope_at_several_speeds() {
        for wpm in [12.0, 20.0, 32.0] {
            let iv = keying("CQ CQ DE F4XYZ F4XYZ PSE K", wpm);
            let total: f64 = iv.iter().map(|x| x.1).sum();
            let mut d = MorseDecoder::new(250.0);
            let mut seed = 7u32;
            let mut out = String::new();
            let mut t = 0.0;
            while t < 3.0 * total {
                let a = key_at(&iv, t) + 0.15 * gauss(&mut seed).abs();
                d.process(a);
                out.push_str(&d.take());
                t += 0.004;
            }
            assert!(out.contains("CQ DE F4XYZ F4XYZ PSE K"), "{wpm} WPM: {out:?}");
            assert!((d.wpm() - wpm as f32).abs() < 0.15 * wpm as f32, "{wpm} WPM estimated {}", d.wpm());
        }
    }

    #[test]
    fn noise_and_carrier_give_nothing() {
        let mut d = MorseDecoder::new(250.0);
        let mut seed = 3u32;
        let mut out = String::new();
        for _ in 0..250 * 20 {
            let (i, q) = (gauss(&mut seed), gauss(&mut seed));
            d.process((i * i + q * q).sqrt());
            out.push_str(&d.take());
        }
        assert!(out.trim().len() <= 2, "noise decoded as {out:?}");
    }

    /// Several stations in the 40 m segment, a steady carrier and noise
    #[test]
    fn skims_a_band() {
        let rate = 240_000.0;
        let center = 7_030_000.0;
        let stations = [
            (7_010_500.0, 18.0, 0.01, "CQ CQ DE F4ABC F4ABC K"),
            (7_021_300.0, 28.0, 0.004, "TU 5NN 14 TEST DL1XX"),
            (7_041_700.0, 14.0, 0.003, "QRZ DE YO3RAD"),
        ];
        let carrier = 7_033_000.0;
        let ivs: Vec<_> = stations.iter().map(|s| keying(s.3, s.1)).collect();
        let mut sk = CwSkimmer::new();
        sk.set("enabled", 1.0);
        let mut seed = 11u32;
        let block = 8192;
        let secs = 30.0;
        let mut n = 0usize;
        while (n as f64) < secs * rate {
            let mut buf = Vec::with_capacity(block);
            for k in 0..block {
                let t = (n + k) as f64 / rate;
                let mut z = C32::new(0.02 * gauss(&mut seed), 0.02 * gauss(&mut seed));
                for (s, iv) in stations.iter().zip(&ivs) {
                    let ph = 2.0 * std::f64::consts::PI * (s.0 - center) * t;
                    let a = s.2 * key_at(iv, t);
                    z += C32::new(a * ph.cos() as f32, a * ph.sin() as f32);
                }
                let ph = 2.0 * std::f64::consts::PI * (carrier - center) * t;
                z += C32::new(0.005 * ph.cos() as f32, 0.005 * ph.sin() as f32);
                buf.push(z);
            }
            sk.feed_iq(&buf, rate, center, 7_020_000.0);
            n += block;
        }
        let lines = sk.lines();
        let json = sk.json(false);
        assert!(json.contains("\"band\":\"40m\""), "{json}");
        for s in &stations {
            let l = lines.iter().find(|l| (l.0 - s.0).abs() < 60.0);
            let Some(l) = l else { panic!("{} not found: {lines:?}", s.0) };
            let tail: String = s.3.split(' ').skip(1).collect::<Vec<_>>().join(" ");
            assert!(l.1.contains(&tail), "{} Hz: {:?}", s.0, l.1);
        }
        assert!(!lines.iter().any(|l| (l.0 - carrier).abs() < 200.0), "carrier decoded: {lines:?}");
        assert!(lines.len() <= stations.len() + 1, "spurious lines: {lines:?}");
    }

    #[test]
    fn focus_decodes_the_tone() {
        let iv = keying("VVV DE ON4XX", 22.0);
        let total: f64 = iv.iter().map(|x| x.1).sum();
        let mut sk = CwSkimmer::new();
        let mut seed = 5u32;
        let mut t = 0.0;
        let mut buf = Vec::new();
        while t < 3.0 * total {
            let a = key_at(&iv, t) * (2.0 * std::f64::consts::PI * 650.0 * t).sin() as f32;
            buf.push(0.3 * a + 0.05 * gauss(&mut seed));
            t += 1.0 / 48_000.0;
            if buf.len() == 4800 {
                sk.feed_focus(&buf, 650.0);
                buf.clear();
            }
        }
        assert!(sk.focus_text().contains("VVV DE ON4XX"), "{:?}", sk.focus_text());
    }
}
