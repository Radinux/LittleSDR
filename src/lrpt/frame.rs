//! Frame synchronisation and error correction: soft symbols -> corrected VCDUs.
//!
//! Each CADU is 1024 bytes: the sync marker 1ACFFC1D then 1020 scrambled bytes (a VCDU with
//! its Reed-Solomon parity), convolutionally encoded: 16384 soft symbols. The encoded marker
//! is searched under the 8 ways the demodulator may hand the symbols over (4 carrier phases,
//! mirrored or not), at any offset, on the plain stream and on the differentially decoded one
//! (OQPSK satellites send differential coding). Once found, the next ones are expected one
//! CADU later, with a little slack.

use super::fec::{conv_encode, pn_sequence, ReedSolomon, Viterbi};

pub const ASM: u32 = 0x1ACF_FC1D;
pub const CADU_SYMBOLS: usize = 1024 * 8 * 2;
const TAIL: usize = 64; // extra symbols decoded past the frame (into the next marker)
const FIND: i32 = 46; // marker agreement (out of 64) to acquire...
const CONFIRM: i32 = 42; // ...confirmed by a second marker one frame later
const KEEP: i32 = 42; // to stay locked, same orientation, near the expected place
const SLACK: usize = 4;
const MAX_FAILS: u32 = 4; // consecutive frames failing Reed-Solomon before resync

/// Corrected VCDU (1020 bytes minus the 128 parity bytes = 892) and its quality
pub struct Vcdu {
    pub data: Vec<u8>,
    pub rs_errors: usize,
    pub viterbi: f32,
}

#[derive(Default, Clone, Debug)]
pub struct FrameStats {
    pub frames: u64,
    pub rs_ok: u64,
    pub rs_fail: u64,
    pub resyncs: u64,
    pub diff: Option<bool>,
    pub viterbi: f32,
}

pub struct Framer {
    viterbi: Viterbi,
    rs: ReedSolomon,
    pn: [u8; 255],
    /// Expected signs of the encoded marker under each transform
    patterns: [[i8; 64]; 8],
    raw: Vec<i8>,
    dif: Vec<i8>,
    prev: (i32, i32),
    /// Locked: (use the differential stream, transform, position of the next frame)
    lock: Option<(bool, usize, usize)>,
    /// Differential mode, once a frame has decoded (None = still trying both)
    mode: Option<bool>,
    pub stats: FrameStats,
    /// Last frame as decoded by the Viterbi (marker included), for diagnostics
    pub last_raw: Vec<u8>,
    fails: u32,
}

impl Default for Framer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pair transforms: optional swap, then a rotation by t % 4 quarter turns
#[inline]
fn transform(a: i32, b: i32, t: usize) -> (i32, i32) {
    let (a, b) = if t >= 4 { (b, a) } else { (a, b) };
    match t % 4 {
        0 => (a, b),
        1 => (b, -a),
        2 => (-a, -b),
        _ => (-b, a),
    }
}

/// Undoes `transform(.., t)`
#[inline]
fn untransform(x: i32, y: i32, t: usize) -> (i32, i32) {
    let (a, b) = match t % 4 {
        0 => (x, y),
        1 => (-y, x),
        2 => (-x, -y),
        _ => (y, -x),
    };
    if t >= 4 {
        (b, a)
    } else {
        (a, b)
    }
}

impl Framer {
    pub fn new() -> Framer {
        let mut st = 0;
        let bits = conv_encode(&ASM.to_be_bytes(), &mut st);
        // The register before the marker is unknown: its first 6 encoded pairs depend on the
        // previous frame's last bits. Only the last 26 bits are fully determined, but in
        // practice the previous bits are the end of RS parity: matching all 64 still works
        // with the threshold below (as other decoders do).
        let e: Vec<i32> = bits.iter().map(|&b| if b == 1 { -1 } else { 1 }).collect();
        let mut patterns = [[0i8; 64]; 8];
        for (t, p) in patterns.iter_mut().enumerate() {
            for k in 0..32 {
                let (x, y) = transform(e[2 * k], e[2 * k + 1], t);
                p[2 * k] = x as i8;
                p[2 * k + 1] = y as i8;
            }
        }
        Framer {
            viterbi: Viterbi::new(),
            rs: ReedSolomon::new(),
            pn: pn_sequence(),
            patterns,
            raw: Vec::new(),
            dif: Vec::new(),
            prev: (0, 0),
            lock: None,
            mode: None,
            stats: FrameStats::default(),
            last_raw: Vec::new(),
            fails: 0,
        }
    }

    pub fn locked(&self) -> bool {
        self.lock.is_some()
    }

    fn corr(&self, s: &[i8], at: usize, t: usize) -> i32 {
        let p = &self.patterns[t];
        let mut c = 0;
        for k in 0..64 {
            let v = s[at + k];
            if v != 0 && (v > 0) == (p[k] > 0) {
                c += 1;
            }
        }
        c
    }

    /// Best (agreement, position, transform) in `s[from..to]`
    fn search(&self, s: &[i8], from: usize, to: usize) -> (i32, usize, usize) {
        let mut best = (0, 0, 0);
        for at in from..to.min(s.len().saturating_sub(64)) {
            for t in 0..8 {
                let c = self.corr(s, at, t);
                if c > best.0 {
                    best = (c, at, t);
                }
            }
        }
        best
    }

    pub fn push(&mut self, soft: &[i8], out: &mut Vec<Vcdu>) {
        // Differential decoding of the pairs: d[k] = s[k] * s[k-1] per branch, one inverted
        for pair in soft.chunks_exact(2) {
            let (i, q) = (pair[0] as i32, pair[1] as i32);
            let a = signsqrt(i * self.prev.0);
            let b = signsqrt(-q * self.prev.1);
            self.dif.push(a);
            self.dif.push(b);
            self.prev = (i, q);
        }
        self.raw.extend_from_slice(soft);

        loop {
            let need = CADU_SYMBOLS + TAIL + SLACK + 64;
            match self.lock {
                Some((d, t, pos)) => {
                    let s = if d { &self.dif } else { &self.raw };
                    if s.len() < pos + need {
                        break;
                    }
                    // Re-find the marker around where it should be (clock slips), keeping the
                    // orientation: a random match elsewhere must not steer the lock
                    let lo = pos.saturating_sub(SLACK);
                    let (c, at) = (lo..=pos + SLACK).map(|a| (self.corr(s, a, t), a)).max_by_key(|x| x.0).unwrap();
                    if c < KEEP || self.fails >= MAX_FAILS {
                        self.lock = None;
                        self.fails = 0;
                        self.stats.resyncs += 1;
                        self.discard(pos.saturating_sub(SLACK));
                        continue;
                    }
                    let ok = self.decode(d, t, at, out);
                    self.fails = if ok { 0 } else { self.fails + 1 };
                    self.lock = Some((d, t, at + CADU_SYMBOLS));
                    if ok && self.mode.is_none() {
                        self.mode = Some(d);
                        self.stats.diff = Some(d);
                    }
                    // Keep the buffers short
                    if at > 4 * CADU_SYMBOLS {
                        let cut = at - CADU_SYMBOLS;
                        self.raw.drain(..cut);
                        self.dif.drain(..cut);
                        self.lock = Some((d, t, at + CADU_SYMBOLS - cut));
                    }
                }
                None => {
                    // One period to search, plus one more frame to confirm the hit
                    if self.raw.len() < 2 * CADU_SYMBOLS + 2 * SLACK + TAIL + 64 {
                        break;
                    }
                    let span = CADU_SYMBOLS + 1;
                    let mut best = (0, 0, 0, false);
                    for d in [false, true] {
                        if self.mode.is_some_and(|m| m != d) {
                            continue;
                        }
                        let s = if d { &self.dif } else { &self.raw };
                        let (c, at, t) = self.search(s, 0, span);
                        if c > best.0 {
                            best = (c, at, t, d);
                        }
                    }
                    let (c, at, t, d) = best;
                    let s = if d { &self.dif } else { &self.raw };
                    let next = at + CADU_SYMBOLS;
                    let confirmed = c >= FIND
                        && (next.saturating_sub(SLACK)..=next + SLACK).any(|a| self.corr(s, a, t) >= CONFIRM);
                    if confirmed {
                        self.lock = Some((d, t, at));
                    } else {
                        self.discard(span);
                    }
                }
            }
        }
    }

    fn discard(&mut self, n: usize) {
        let n = n.min(self.raw.len());
        self.raw.drain(..n);
        self.dif.drain(..n);
    }

    /// Viterbi, descrambling and Reed-Solomon of the frame starting at `at`
    fn decode(&mut self, d: bool, t: usize, at: usize, out: &mut Vec<Vcdu>) -> bool {
        let s = if d { &self.dif } else { &self.raw };
        let win = &s[at..at + CADU_SYMBOLS + TAIL];
        let mut soft = Vec::with_capacity(win.len());
        for p in win.chunks_exact(2) {
            let (x, y) = untransform(p[0] as i32, p[1] as i32, t);
            soft.push(x.clamp(-127, 127) as i8);
            soft.push(y.clamp(-127, 127) as i8);
        }
        let (bytes, q) = self.viterbi.decode(&soft);
        if std::env::var_os("LRPT_DEBUG").is_some() {
            eprintln!("decode d={d} t={t} at={at} q={q:.2} head {:02X?}", &bytes[..4]);
        }
        self.stats.frames += 1;
        self.stats.viterbi = q;
        self.last_raw = bytes.clone();
        let mut frame: Vec<u8> = bytes[4..1024].to_vec();
        for (i, b) in frame.iter_mut().enumerate() {
            *b ^= self.pn[i % 255];
        }
        match self.rs.decode_interleaved(&mut frame) {
            Some(n) => {
                self.stats.rs_ok += 1;
                frame.truncate(892);
                out.push(Vcdu { data: frame, rs_errors: n, viterbi: q });
                true
            }
            None => {
                self.stats.rs_fail += 1;
                false
            }
        }
    }
}

#[inline]
fn signsqrt(x: i32) -> i8 {
    let r = (x.unsigned_abs() as f32).sqrt().min(127.0) as i8;
    if x >= 0 {
        r
    } else {
        -r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    /// Builds CADUs from VCDU payloads: RS parity, scrambling, marker, then encoding
    pub(crate) fn make_cadus(payloads: &[Vec<u8>]) -> Vec<u8> {
        let rs = ReedSolomon::new();
        let pn = pn_sequence();
        let mut bytes = Vec::new();
        for p in payloads {
            assert_eq!(p.len(), 892);
            let mut frame = vec![0u8; 1020];
            for i in 0..4 {
                let mut msg = [0u8; 223];
                for j in 0..223 {
                    msg[j] = p[j * 4 + i];
                }
                let par = rs.encode(&msg);
                for j in 0..223 {
                    frame[j * 4 + i] = msg[j];
                }
                for j in 0..32 {
                    frame[(223 + j) * 4 + i] = par[j];
                }
            }
            for (i, b) in frame.iter_mut().enumerate() {
                *b ^= pn[i % 255];
            }
            bytes.extend_from_slice(&ASM.to_be_bytes());
            bytes.extend_from_slice(&frame);
        }
        let mut st = 0;
        conv_encode(&bytes, &mut st)
    }

    #[test]
    fn finds_frames_under_any_phase_and_corrects_them() {
        let mut seed = 5u32;
        let payloads: Vec<Vec<u8>> = (0..6).map(|_| (0..892).map(|_| rng(&mut seed) as u8).collect()).collect();
        let bits = make_cadus(&payloads);
        for t in [0usize, 3, 6] {
            // Soft symbols, rotated / mirrored, some noise and errors, an odd start offset
            let mut soft: Vec<i8> = (0..777).map(|_| (rng(&mut seed) % 200) as i8).collect();
            for pair in bits.chunks_exact(2) {
                let e = |b: u8| if b == 1 { -50 } else { 50 };
                let (x, y) = transform(e(pair[0]), e(pair[1]), t);
                for v in [x, y] {
                    let noise = (rng(&mut seed) % 61) as i32 - 30;
                    let flip = rng(&mut seed) % 50 == 0;
                    soft.push((if flip { -v } else { v + noise }).clamp(-127, 127) as i8);
                }
            }
            soft.extend((0..40_000).map(|_| (rng(&mut seed) % 200) as i8));
            let mut f = Framer::new();
            let mut out = Vec::new();
            for ch in soft.chunks(3000) {
                f.push(ch, &mut out);
            }
            assert!(out.len() >= 5, "transform {t}: {} frames ({:?})", out.len(), f.stats);
            for v in &out {
                assert!(payloads.contains(&v.data), "transform {t}: wrong payload");
            }
        }
    }
}
