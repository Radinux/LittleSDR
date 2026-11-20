//! Classic (analog) SSTV decoder: Martin, Scottie, Robot and PD modes, VIS auto-detection.
//! Fed with demodulated audio (any sample rate), it produces an ARGB image line by line.
//! Rust port of the app's C++ decoder (cpp/sstv/sstv_decoder.cpp), same behaviour and
//! same sstvJson / sstvSet / sstvImage interface.

use std::f64::consts::PI;

const CENTER_HZ: f64 = 1900.0; // mixing frequency: middle of the 1100-2300 Hz SSTV range
const TARGET_RATE: f64 = 12_000.0;
const TAPS: usize = 65;
const CUTOFF_HZ: f64 = 1300.0; // keeps 600-3200 Hz of audio
const BLANK: u32 = 0xFF15_171C;
const HIST_SEC: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Martin / Scottie: green, blue, red scans
    Gbr,
    /// Y, then R-Y or B-Y on alternate lines (4:2:0)
    Robot36,
    /// Y, R-Y, B-Y on every line (4:2:2)
    Robot72,
    /// Y(even row), R-Y, B-Y, Y(odd row): one scan line = two image rows
    Pd,
}

#[derive(Clone, Debug)]
pub struct Mode {
    pub name: &'static str,
    pub vis: i32,
    pub width: usize,
    /// Image rows (PD: 2 x scan lines)
    pub height: usize,
    pub line_ms: f64,
    /// Sync pulse (1200 Hz) length
    pub sync_ms: f64,
    /// Sync start within the scan line
    pub sync_at_ms: f64,
    /// First scan line start, after the VIS stop bit
    pub start_ms: f64,
    pub color: Color,
    pub segs: usize,
    pub seg_start: [f64; 4],
    pub seg_len: [f64; 4],
}

impl Mode {
    pub fn scan_lines(&self) -> usize {
        if self.color == Color::Pd {
            self.height / 2
        } else {
            self.height
        }
    }
}

fn martin(name: &'static str, vis: i32, scan: f64) -> Mode {
    let (sync, gap) = (4.862, 0.572);
    let mut m = Mode {
        name,
        vis,
        width: 320,
        height: 256,
        line_ms: sync + gap + 3.0 * (scan + gap),
        sync_ms: sync,
        sync_at_ms: 0.0,
        start_ms: 0.0,
        color: Color::Gbr,
        segs: 3,
        seg_start: [0.0; 4],
        seg_len: [0.0; 4],
    };
    for i in 0..3 {
        m.seg_start[i] = sync + gap + i as f64 * (scan + gap);
        m.seg_len[i] = scan;
    }
    m
}

/// Sync pulse in the middle of the line, before the red scan; a 9 ms "starting sync" follows
/// the VIS, so the first line begins 9 ms after it
fn scottie(name: &'static str, vis: i32, scan: f64) -> Mode {
    let (sep, sync) = (1.5, 9.0);
    Mode {
        name,
        vis,
        width: 320,
        height: 256,
        line_ms: 3.0 * scan + 3.0 * sep + sync,
        sync_ms: sync,
        sync_at_ms: 2.0 * sep + 2.0 * scan,
        start_ms: sync,
        color: Color::Gbr,
        segs: 3,
        seg_start: [sep, 2.0 * sep + scan, 2.0 * sep + 2.0 * scan + sync + sep, 0.0],
        seg_len: [scan, scan, scan, 0.0],
    }
}

fn pd(name: &'static str, vis: i32, w: usize, h: usize, pixel_ms: f64) -> Mode {
    let (sync, porch, scan) = (20.0, 2.08, w as f64 * pixel_ms);
    let mut m = Mode {
        name,
        vis,
        width: w,
        height: h,
        line_ms: sync + porch + 4.0 * scan,
        sync_ms: sync,
        sync_at_ms: 0.0,
        start_ms: 0.0,
        color: Color::Pd,
        segs: 4,
        seg_start: [0.0; 4],
        seg_len: [scan; 4],
    };
    for i in 0..4 {
        m.seg_start[i] = sync + porch + i as f64 * scan;
    }
    m
}

pub fn modes() -> &'static [Mode] {
    use std::sync::OnceLock;
    static LIST: OnceLock<Vec<Mode>> = OnceLock::new();
    LIST.get_or_init(|| {
        let robot = |name, vis, line_ms, color, segs, seg_start: [f64; 4], seg_len: [f64; 4]| Mode {
            name,
            vis,
            width: 320,
            height: 240,
            line_ms,
            sync_ms: 9.0,
            sync_at_ms: 0.0,
            start_ms: 0.0,
            color,
            segs,
            seg_start,
            seg_len,
        };
        vec![
            martin("Martin M1", 44, 146.432),
            martin("Martin M2", 40, 73.216),
            scottie("Scottie S1", 60, 138.24),
            scottie("Scottie S2", 56, 88.064),
            scottie("Scottie DX", 76, 345.6),
            // Robot 36: sync 9, porch 3, Y 88, separator 4.5 (1500 Hz: R-Y, 2300 Hz: B-Y), porch 1.5, chroma 44
            robot("Robot 36", 8, 150.0, Color::Robot36, 2, [12.0, 106.0, 0.0, 0.0], [88.0, 44.0, 0.0, 0.0]),
            // Robot 72: sync 9, porch 3, Y 138, sep 4.5 + porch 1.5, R-Y 69, sep 4.5 + porch 1.5, B-Y 69
            robot("Robot 72", 12, 300.0, Color::Robot72, 3, [12.0, 156.0, 231.0, 0.0], [138.0, 69.0, 69.0, 0.0]),
            pd("PD 50", 93, 320, 256, 0.286),
            pd("PD 90", 99, 320, 256, 0.532),
            pd("PD 120", 95, 640, 496, 0.19),
            pd("PD 160", 98, 512, 400, 0.382),
            pd("PD 180", 96, 640, 496, 0.286),
            pd("PD 240", 97, 640, 496, 0.382),
            pd("PD 290", 94, 800, 616, 0.286),
        ]
    })
}

fn find_mode(vis: i32) -> Option<usize> {
    modes().iter().position(|m| m.vis == vis)
}

fn clamp8(v: f64) -> u32 {
    v.clamp(0.0, 255.0) as u32
}

fn rgb(r: f64, g: f64, b: f64) -> u32 {
    0xFF00_0000 | (clamp8(r) << 16) | (clamp8(g) << 8) | clamp8(b)
}

/// ITU-R 601 studio range (Y 16-235), as MMSSTV encodes and QSSTV / Robot36 decode
fn yuv(y: f64, cr: f64, cb: f64) -> u32 {
    let l = 1.164 * (y - 16.0);
    rgb(l + 1.596 * (cr - 128.0), l - 0.813 * (cr - 128.0) - 0.391 * (cb - 128.0), l + 2.018 * (cb - 128.0))
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub receiving: bool,
    /// Current or last image
    pub mode: String,
    /// Decoded image rows
    pub line: usize,
    pub width: usize,
    pub height: usize,
    /// +1 whenever the current image changes
    pub version: u64,
    /// +1 per finished image (complete or signal lost)
    pub finished: u64,
    pub finished_rows: usize,
    pub finished_mode: String,
    pub last_complete: bool,
    /// Smoothed audio frequency (Hz), tuning aid
    pub freq: f64,
    /// Measured clock deviation of the current image
    pub slant_ppm: f64,
}

pub struct Decoder {
    pub enabled: bool,
    // Front end: complex mix to baseband, low-pass, decimate, FM discriminator
    fs: f64,
    rate: f64,
    decim: usize,
    phase_count: usize,
    osc_phase: f64,
    osc_step: f64,
    taps: Vec<f32>,
    hist: Vec<(f32, f32)>,
    hist_pos: usize,
    prev_z: (f32, f32),

    // Frequency history (decimated rate), with running sums for O(1) averages
    freq: Vec<f32>,
    cum: Vec<f64>, // cum[i % cap] = sum of freq up to absolute index i (inclusive)
    count: i64,    // absolute index of the next sample
    smooth: f64,

    // VIS search
    vis_best_end: i64,
    vis_candidate_until: i64,
    vis_ignore_until: i64,
    vis_best_code: i32,
    vis_best_err: f64,
    last_vis_check: i64,

    // Current image
    mode_idx: Option<usize>,
    acquire: bool,
    t0: f64,
    period: f64,
    nominal_period: f64,
    next_line: usize,
    missed: usize,
    decoded_rows: usize,
    syncs: Vec<(usize, f64)>, // scan line, measured line start (samples)
    img: Vec<u32>,
    done: Vec<u32>,
    done_w: usize,
    done_h: usize,
    /// Per scan line channel values (0-255), for Robot 36
    chan: [Vec<Vec<u8>>; 2],
    robot_chroma: Vec<i8>, // 0 = R-Y, 1 = B-Y, -1 unknown

    st: Status,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            enabled: false,
            fs: 0.0,
            rate: 0.0,
            decim: 1,
            phase_count: 0,
            osc_phase: 0.0,
            osc_step: 0.0,
            taps: Vec::new(),
            hist: Vec::new(),
            hist_pos: 0,
            prev_z: (1.0, 0.0),
            freq: Vec::new(),
            cum: Vec::new(),
            count: 0,
            smooth: CENTER_HZ,
            vis_best_end: -1,
            vis_candidate_until: -1,
            vis_ignore_until: 0,
            vis_best_code: -1,
            vis_best_err: 1e9,
            last_vis_check: 0,
            mode_idx: None,
            acquire: false,
            t0: 0.0,
            period: 0.0,
            nominal_period: 0.0,
            next_line: 0,
            missed: 0,
            decoded_rows: 0,
            syncs: Vec::new(),
            img: Vec::new(),
            done: Vec::new(),
            done_w: 0,
            done_h: 0,
            chan: [Vec::new(), Vec::new()],
            robot_chroma: Vec::new(),
            st: Status::default(),
        }
    }

    fn ms(&self, m: f64) -> i64 {
        (m * self.rate / 1000.0 + 0.5) as i64
    }

    // ------------------------------------------------------------------ front end
    fn configure(&mut self, sample_rate: f64) {
        self.fs = sample_rate;
        self.decim = ((sample_rate / TARGET_RATE).round() as usize).max(1);
        self.rate = sample_rate / self.decim as f64;
        self.osc_step = 2.0 * PI * CENTER_HZ / sample_rate;
        self.osc_phase = 0.0;
        self.phase_count = 0;

        // Windowed-sinc low-pass (Hamming), unity DC gain
        let fc = CUTOFF_HZ / sample_rate;
        let mut taps: Vec<f64> = (0..TAPS)
            .map(|i| {
                let k = i as f64 - (TAPS - 1) as f64 / 2.0;
                let sinc = if k == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * k).sin() / (PI * k) };
                sinc * (0.54 - 0.46 * (2.0 * PI * i as f64 / (TAPS - 1) as f64).cos())
            })
            .collect();
        let sum: f64 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        self.taps = taps.into_iter().map(|t| t as f32).collect();
        self.hist = vec![(0.0, 0.0); TAPS];
        self.hist_pos = 0;
        self.prev_z = (1.0, 0.0);

        let cap = (HIST_SEC * self.rate) as usize;
        self.freq = vec![CENTER_HZ as f32; cap];
        self.cum = vec![0.0; cap];
        self.count = 0;
        self.last_vis_check = 0;
        self.vis_ignore_until = 0;
        self.vis_candidate_until = -1;
        self.vis_best_err = 1e9;
        // The time base changed: an image in progress cannot continue
        if self.mode_idx.is_some() {
            self.end_image(false);
        }
    }

    fn push_freq(&mut self, f: f32) {
        let f = f.clamp(900.0, 2700.0);
        let cap = self.freq.len() as i64;
        let i = (self.count % cap) as usize;
        let prev = if self.count > 0 { self.cum[((self.count - 1) % cap) as usize] } else { 0.0 };
        self.freq[i] = f;
        self.cum[i] = prev + f as f64;
        self.count += 1;
        self.smooth += (f as f64 - self.smooth) * 0.002;
    }

    /// Mean frequency over [a, b) (absolute indices)
    fn mean(&self, a: i64, b: i64) -> f64 {
        let cap = self.freq.len() as i64;
        if cap == 0 || b <= a || b > self.count || a - 1 < self.count - cap || a < 0 {
            return CENTER_HZ;
        }
        let hi = self.cum[((b - 1) % cap) as usize];
        let lo = if a > 0 { self.cum[((a - 1) % cap) as usize] } else { 0.0 };
        (hi - lo) / (b - a) as f64
    }

    /// Mono audio at `sample_rate` Hz
    pub fn feed(&mut self, input: &[f32], sample_rate: f64) {
        if sample_rate <= 0.0 {
            return;
        }
        if (sample_rate - self.fs).abs() > 0.5 {
            self.configure(sample_rate);
        }
        let to_hz = self.rate / (2.0 * PI);
        for &x in input {
            let (s, c) = self.osc_phase.sin_cos();
            self.hist[self.hist_pos] = ((x as f64 * c) as f32, (-x as f64 * s) as f32);
            self.hist_pos = (self.hist_pos + 1) % TAPS;
            self.osc_phase += self.osc_step;
            if self.osc_phase > 2.0 * PI {
                self.osc_phase -= 2.0 * PI;
            }
            self.phase_count += 1;
            if self.phase_count < self.decim {
                continue;
            }
            self.phase_count = 0;
            let (mut zr, mut zi) = (0.0f32, 0.0f32);
            for k in 0..TAPS {
                let h = self.hist[(self.hist_pos + k) % TAPS];
                zr += self.taps[k] * h.0;
                zi += self.taps[k] * h.1;
            }
            // z * conj(prev)
            let (pr, pi) = self.prev_z;
            let (dr, di) = (zr * pr + zi * pi, zi * pr - zr * pi);
            self.prev_z = (zr, zi);
            self.push_freq((CENTER_HZ + di.atan2(dr) as f64 * to_hz) as f32);
        }
        self.detect_vis();
        self.decode_ready();
    }

    // ------------------------------------------------------------------ VIS
    // Leader 1900 Hz (300 ms), start bit 1200 Hz, 7 data bits LSB first + even parity
    // (1100 Hz = 1, 1300 Hz = 0), stop bit 1200 Hz; 30 ms per bit. "end" = end of the stop bit.
    // Tolerant on purpose: only the last 190 ms of leader are checked, and some encoders send
    // the start bit at 1900 Hz. Parity, stop bit and the list of known modes keep false
    // detections out.
    fn vis_at(&self, end: i64) -> Option<(i32, f64)> {
        let s = end - self.ms(300.0);
        if s - self.ms(190.0) < self.count - self.freq.len() as i64 + 1 || s - self.ms(190.0) < 0 {
            return None;
        }
        let m = |a: f64, b: f64| self.mean(s + self.ms(a), s + self.ms(b));
        let mut err = 0.0;
        for a in [-190.0, -100.0] {
            let v = m(a, a + 80.0);
            if (v - 1900.0).abs() > 80.0 {
                return None;
            }
            err += (v - 1900.0).abs();
        }
        let (start, stop) = (m(5.0, 25.0), m(275.0, 295.0));
        let start_dev = (start - 1200.0).abs().min((start - 1900.0).abs());
        if start_dev > 100.0 || (stop - 1200.0).abs() > 100.0 {
            return None;
        }
        err += start_dev + (stop - 1200.0).abs();
        let (mut bits, mut ones) = (0i32, 0i32);
        for k in 0..8 {
            let v = m(35.0 + 30.0 * k as f64, 55.0 + 30.0 * k as f64);
            if !(1000.0..=1400.0).contains(&v) || (v - 1200.0).abs() < 25.0 {
                return None;
            }
            let one = v < 1200.0;
            err += (v - if one { 1100.0 } else { 1300.0 }).abs();
            if k < 7 {
                bits |= (one as i32) << k;
                ones += one as i32;
            } else if (ones & 1) != one as i32 {
                return None; // even parity
            }
        }
        Some((bits, err))
    }

    fn detect_vis(&mut self) {
        let step = self.ms(1.0).max(1);
        let mut e = self.last_vis_check + step;
        while e <= self.count {
            self.last_vis_check = e;
            if e >= self.vis_ignore_until {
                if let Some((code, err)) = self.vis_at(e).filter(|v| find_mode(v.0).is_some()) {
                    // Keep the best alignment over the next 15 ms
                    if self.vis_candidate_until < 0 {
                        self.vis_candidate_until = e + self.ms(15.0);
                    }
                    if err < self.vis_best_err {
                        self.vis_best_err = err;
                        self.vis_best_end = e;
                        self.vis_best_code = code;
                    }
                }
                if self.vis_candidate_until >= 0 && e >= self.vis_candidate_until {
                    // A VIS inside a clean image is a false alarm; after lost syncs, a new transmission
                    if let Some(mi) = find_mode(self.vis_best_code) {
                        if self.mode_idx.is_none() || self.missed >= 3 {
                            if self.mode_idx.is_some() {
                                self.end_image(false);
                            }
                            let t0 = self.vis_best_end as f64 + modes()[mi].start_ms * self.rate / 1000.0;
                            self.begin_image(mi, t0, false);
                        }
                    }
                    self.vis_ignore_until = self.vis_best_end + self.ms(1000.0);
                    self.vis_candidate_until = -1;
                    self.vis_best_err = 1e9;
                }
            }
            e += step;
        }
    }

    // ------------------------------------------------------------------ image
    fn begin_image(&mut self, mi: usize, t0: f64, acquire: bool) {
        let m = &modes()[mi];
        self.mode_idx = Some(mi);
        self.acquire = acquire;
        self.t0 = t0;
        self.nominal_period = m.line_ms * self.rate / 1000.0;
        self.period = self.nominal_period;
        self.next_line = 0;
        self.missed = 0;
        self.decoded_rows = 0;
        self.syncs.clear();
        self.img = vec![BLANK; m.width * m.height];
        let lines = m.scan_lines();
        self.chan = [Vec::new(), Vec::new()];
        if m.color == Color::Robot36 {
            self.chan[0] = vec![vec![0; m.width]; lines];
            self.chan[1] = vec![vec![128; m.width]; lines];
            self.robot_chroma = vec![-1; lines];
        }
        self.st.receiving = true;
        self.st.mode = m.name.to_string();
        self.st.width = m.width;
        self.st.height = m.height;
        self.st.line = 0;
        self.st.slant_ppm = 0.0;
        self.st.version += 1;
    }

    fn decode_ready(&mut self) {
        let Some(mi) = self.mode_idx else { return };
        let m = &modes()[mi];
        let lines = m.scan_lines();
        while self.mode_idx.is_some() && self.next_line < lines {
            // Whole line + sync search margin received (first line of a manual start: two lines)
            let need = self.t0 + (self.next_line + if self.acquire { 2 } else { 1 }) as f64 * self.period + self.ms(m.sync_ms + 12.0) as f64;
            if (self.count as f64) < need {
                break;
            }
            let n = self.next_line;
            self.next_line += 1;
            self.decode_line(n);
            // Long silence or noise: nothing left to decode
            if self.missed >= 12 {
                self.end_image(false);
                return;
            }
        }
        if self.mode_idx.is_some() && self.next_line >= lines {
            self.end_image(true);
        }
    }

    fn find_sync(&self, n: usize) -> Option<f64> {
        let m = &modes()[self.mode_idx?];
        let len = self.ms(m.sync_ms).max(1);
        let expected = self.t0 + n as f64 * self.period + m.sync_at_ms * self.rate / 1000.0;
        let w = if self.acquire { self.period as i64 } else { self.ms(6.0) };
        let c = expected.round() as i64;
        let (mut best, mut best_p) = (1e9, c);
        for p in c - if self.acquire { 0 } else { w }..=c + w {
            let v = self.mean(p, p + len);
            if v < best {
                best = v;
                best_p = p;
            }
        }
        if best > 1350.0 {
            return None;
        }
        Some(best_p as f64 - m.sync_at_ms * self.rate / 1000.0) // line start
    }

    /// Robust line fit of the measured line starts: start(n) = t0 + n * period
    fn refit(&mut self) {
        let max_dev = self.nominal_period * 0.01;
        let (period, nominal) = (self.period, self.nominal_period);
        let fit = |pts: &[(usize, f64)]| -> (f64, f64) {
            let (lo, hi) = (pts[0].0, pts[pts.len() - 1].0);
            if pts.len() < 3 || hi - lo < 6 {
                let t0 = pts.iter().map(|p| p.1 - p.0 as f64 * period).sum::<f64>() / pts.len() as f64;
                return (t0, period);
            }
            let (mut sn, mut sy, mut snn, mut sny) = (0.0, 0.0, 0.0, 0.0);
            for &(n, y) in pts {
                let n = n as f64;
                sn += n;
                sy += y;
                snn += n * n;
                sny += n * y;
            }
            let k = pts.len() as f64;
            let den = k * snn - sn * sn;
            let per = if den != 0.0 { (k * sny - sn * sy) / den } else { period };
            let per = per.clamp(nominal - max_dev, nominal + max_dev);
            let t0 = pts.iter().map(|p| p.1 - p.0 as f64 * per).sum::<f64>() / k;
            (t0, per)
        };
        let (mut t0, mut per) = fit(&self.syncs);
        let tol = self.ms(1.5) as f64;
        let inl: Vec<(usize, f64)> = self.syncs.iter().copied().filter(|p| (p.1 - (t0 + p.0 as f64 * per)).abs() < tol).collect();
        if inl.len() >= 2 {
            (t0, per) = fit(&inl);
        }
        self.t0 = t0;
        self.period = per;
    }

    fn pixel(&self, base: f64, start_ms: f64, len_ms: f64, x: usize, w: usize) -> f64 {
        let pd = len_ms / w as f64;
        let c = start_ms + (x as f64 + 0.5) * pd;
        let k = self.rate / 1000.0;
        let a = (base + (c - pd / 2.0) * k + 0.5).floor() as i64;
        let b = (a + 1).max((base + (c + pd / 2.0) * k + 0.5).floor() as i64);
        ((self.mean(a, b) - 1500.0) / 800.0 * 255.0).clamp(0.0, 255.0)
    }

    /// Robot 36: rows 2k and 2k+1 share the R-Y of one line and the B-Y of the other
    fn render_row(&mut self, row: usize) {
        let m = &modes()[self.mode_idx.unwrap()];
        if row >= self.next_line || row >= m.height {
            return;
        }
        let b = row & !1;
        let (mut cr, mut cb) = (None, None);
        for l in b..=(b + 1).min(self.next_line - 1) {
            match self.robot_chroma[l] {
                0 => cr = Some(l),
                1 => cb = Some(l),
                _ => {}
            }
        }
        for x in 0..m.width {
            let crv = cr.map_or(128.0, |l| self.chan[1][l][x] as f64);
            let cbv = cb.map_or(128.0, |l| self.chan[1][l][x] as f64);
            self.img[row * m.width + x] = yuv(self.chan[0][row][x] as f64, crv, cbv);
        }
    }

    fn decode_line(&mut self, n: usize) {
        let m = &modes()[self.mode_idx.unwrap()];
        let k = self.rate / 1000.0;
        // Martin, Robot, PD: the first sync directly follows the 1200 Hz VIS stop bit, so no
        // edge tells where it starts. The VIS timing is exact there; syncs are measured from line 1.
        let measure = self.acquire || n > 0 || m.sync_at_ms > 0.0;
        if measure {
            match self.find_sync(n) {
                Some(s) => {
                    if self.acquire {
                        // Manual start: lock the time base on the first sync found
                        self.t0 = s - n as f64 * self.period;
                        self.acquire = false;
                        self.syncs.push((n, s));
                        self.missed = 0;
                    } else {
                        let predicted = self.t0 + n as f64 * self.period;
                        if self.syncs.len() < 6 || (s - predicted).abs() < self.ms(2.5) as f64 {
                            self.syncs.push((n, s));
                            self.refit();
                            self.missed = 0;
                        } else {
                            self.missed += 1;
                        }
                    }
                }
                None => {
                    self.missed += 1;
                    self.acquire = false;
                }
            }
        }

        let base = self.t0 + n as f64 * self.period;
        let w = m.width;
        let seg = |d: &Decoder, i: usize, x: usize| d.pixel(base, m.seg_start[i], m.seg_len[i], x, w);
        match m.color {
            Color::Gbr => {
                for x in 0..w {
                    let (g, bl, r) = (seg(self, 0, x), seg(self, 1, x), seg(self, 2, x));
                    self.img[n * w + x] = rgb(r, g, bl);
                }
                self.decoded_rows = n + 1;
            }
            Color::Robot72 => {
                for x in 0..w {
                    self.img[n * w + x] = yuv(seg(self, 0, x), seg(self, 1, x), seg(self, 2, x));
                }
                self.decoded_rows = n + 1;
            }
            Color::Robot36 => {
                for x in 0..w {
                    self.chan[0][n][x] = seg(self, 0, x) as u8;
                    self.chan[1][n][x] = seg(self, 1, x) as u8;
                }
                // Separator after Y: 1500 Hz before R-Y, 2300 Hz before B-Y (line parity otherwise)
                let sep = self.mean((base + 100.5 * k) as i64, (base + 103.5 * k) as i64);
                self.robot_chroma[n] = if (sep - 1900.0).abs() > 250.0 { (sep > 1900.0) as i8 } else { (n & 1) as i8 };
                self.decoded_rows = n + 1;
                self.render_row(n & !1); // next_line is already n + 1 here
                self.render_row((n & !1) + 1);
            }
            Color::Pd => {
                for x in 0..w {
                    let (cr, cb) = (seg(self, 1, x), seg(self, 2, x));
                    self.img[2 * n * w + x] = yuv(seg(self, 0, x), cr, cb);
                    self.img[(2 * n + 1) * w + x] = yuv(seg(self, 3, x), cr, cb);
                }
                self.decoded_rows = 2 * n + 2;
            }
        }
        self.st.line = self.decoded_rows;
        self.st.slant_ppm = (self.period / self.nominal_period - 1.0) * 1e6;
        self.st.version += 1;
    }

    fn end_image(&mut self, complete: bool) {
        let Some(mi) = self.mode_idx else { return };
        let m = &modes()[mi];
        // A lost signal also ends with lines of noise decoded before giving up: drop them
        let mut rows = self.decoded_rows;
        if !complete {
            let per_line = if m.color == Color::Pd { 2 } else { 1 };
            rows = rows.saturating_sub(self.missed * per_line);
        }
        if complete || rows >= 16 {
            self.done = self.img.clone();
            self.done_w = m.width;
            self.done_h = m.height;
            self.st.finished += 1;
            self.st.finished_rows = if complete { m.height } else { rows };
            self.st.finished_mode = m.name.to_string();
            self.st.last_complete = complete;
        }
        self.st.receiving = false;
        self.st.version += 1;
        self.mode_idx = None;
        self.acquire = false;
        self.vis_ignore_until = self.vis_ignore_until.max(self.count + self.ms(500.0));
    }

    // ------------------------------------------------------------------ control
    /// Forget everything (decoder switched off)
    pub fn reset(&mut self) {
        self.mode_idx = None;
        self.acquire = false;
        self.fs = 0.0; // reconfigure on the next block
        self.st.receiving = false;
        self.st.line = 0;
        self.st.version += 1;
    }

    /// Start an image now in a given mode (index in modes()), for a missed VIS
    pub fn start_manual(&mut self, mi: usize) {
        if mi >= modes().len() || self.rate <= 0.0 {
            return;
        }
        if self.mode_idx.is_some() {
            self.end_image(false);
        }
        self.begin_image(mi, self.count as f64, true);
    }

    /// End the current image now (kept as finished if it has enough rows)
    pub fn finish_now(&mut self) {
        let Some(mi) = self.mode_idx else { return };
        self.missed = 0; // keep every decoded row
        self.end_image(self.next_line >= modes()[mi].scan_lines());
    }

    /// App settings: "enabled", "start" (mode index), "stop"
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "enabled" => {
                let on = v != 0.0;
                if self.enabled && !on {
                    self.reset();
                }
                self.enabled = on;
            }
            "start" if v >= 0.0 => self.start_manual(v as usize),
            "stop" => self.finish_now(),
            "status" => {}
            _ => return false,
        }
        true
    }

    pub fn status(&self) -> Status {
        let mut s = self.st.clone();
        s.freq = self.smooth;
        s
    }

    /// ARGB pixels of the current image (which = 0) or the last finished one (1)
    pub fn image(&self, which: i32) -> (&[u32], usize, usize) {
        if which == 1 {
            (&self.done, self.done_w, self.done_h)
        } else {
            (&self.img, self.st.width, self.st.height)
        }
    }

    /// sstvJson
    pub fn json(&self) -> String {
        let s = self.status();
        let names: Vec<String> = modes().iter().map(|m| format!("\"{}\"", m.name)).collect();
        format!(
            "{{\"enabled\":{},\"receiving\":{},\"mode\":\"{}\",\"line\":{},\"width\":{},\"height\":{},\"version\":{},\"finished\":{},\"finishedRows\":{},\"finishedMode\":\"{}\",\"lastComplete\":{},\"freq\":{:.1},\"slantPpm\":{:.1},\"modes\":[{}]}}",
            self.enabled,
            s.receiving,
            s.mode,
            s.line,
            s.width,
            s.height,
            s.version,
            s.finished,
            s.finished_rows,
            s.finished_mode,
            s.last_complete,
            s.freq,
            s.slant_ppm,
            names.join(",")
        )
    }

    /// PNG of the current (0) or last finished (1) image
    pub fn png(&self, which: i32) -> Option<Vec<u8>> {
        let (px, w, h) = self.image(which);
        if w == 0 || h == 0 || px.len() < w * h {
            return None;
        }
        let rgb: Vec<u8> = px[..w * h].iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect();
        Some(crate::lrpt::image::png(w, h, 3, &rgb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FM audio of an SSTV transmission: (frequency Hz, duration ms) segments
    struct Tx {
        rate: f64,
        phase: f64,
        t: f64, // ms of signal so far, for exact timing without drift
        out: Vec<f32>,
    }

    impl Tx {
        fn new(rate: f64) -> Tx {
            Tx { rate, phase: 0.0, t: 0.0, out: Vec::new() }
        }
        fn tone(&mut self, f: f64, ms: f64) {
            let end = self.t + ms;
            while (self.out.len() as f64) < end * self.rate / 1000.0 {
                self.phase += 2.0 * PI * f / self.rate;
                self.out.push(0.5 * self.phase.sin() as f32);
            }
            self.t = end;
        }
        fn pixels(&mut self, vals: &[f64], ms: f64) {
            let each = ms / vals.len() as f64;
            for &v in vals {
                self.tone(1500.0 + v / 255.0 * 800.0, each);
            }
        }
        fn vis(&mut self, code: i32) {
            self.tone(1900.0, 300.0);
            self.tone(1200.0, 10.0);
            self.tone(1900.0, 300.0);
            self.tone(1200.0, 30.0);
            let mut ones = 0;
            for k in 0..7 {
                let one = code >> k & 1 == 1;
                ones += one as i32;
                self.tone(if one { 1100.0 } else { 1300.0 }, 30.0);
            }
            self.tone(if ones & 1 == 1 { 1100.0 } else { 1300.0 }, 30.0);
            self.tone(1200.0, 30.0);
        }
    }

    /// Test picture: colour bars that vary down the image
    fn picture(w: usize, h: usize) -> Vec<[f64; 3]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let bar = x * 8 / w;
                let r = if bar & 1 != 0 { 230.0 } else { 30.0 };
                let g = if bar & 2 != 0 { 220.0 } else { 40.0 };
                let b = if bar & 4 != 0 { 200.0 } else { 50.0 } * (0.5 + 0.5 * y as f64 / h as f64);
                [r, g, b]
            })
            .collect()
    }

    fn check(dec: &Decoder, pic: &[[f64; 3]], w: usize, tol: f64) -> f64 {
        let (img, iw, ih) = dec.image(1);
        assert_eq!(iw, w);
        let mut err = 0.0;
        let mut n = 0.0;
        for y in 4..ih - 4 {
            for x in 4..w - 4 {
                if (x * 8 / w) != ((x + 3) * 8 / w) || (x * 8 / w) != ((x - 3) * 8 / w) {
                    continue; // skip bar edges
                }
                let p = img[y * w + x];
                let q = pic[y * w + x];
                err += ((p >> 16 & 255) as f64 - q[0]).abs() + ((p >> 8 & 255) as f64 - q[1]).abs() + ((p & 255) as f64 - q[2]).abs();
                n += 3.0;
            }
        }
        let e = err / n;
        assert!(e < tol, "mean error {e:.1}");
        e
    }

    #[test]
    fn martin_m1_with_vis() {
        let m = &modes()[0];
        let pic = picture(m.width, m.height);
        let mut tx = Tx::new(48_000.0);
        tx.tone(1500.0, 500.0);
        tx.vis(m.vis);
        for y in 0..m.height {
            tx.tone(1200.0, 4.862);
            tx.tone(1500.0, 0.572);
            for c in [1, 2, 0] {
                let vals: Vec<f64> = (0..m.width).map(|x| pic[y * m.width + x][c]).collect();
                tx.pixels(&vals, 146.432);
                tx.tone(1500.0, 0.572);
            }
        }
        tx.tone(1500.0, 300.0);
        let mut dec = Decoder::new();
        for c in tx.out.chunks(4800) {
            dec.feed(c, 48_000.0);
        }
        let st = dec.status();
        assert_eq!(st.finished, 1);
        assert!(st.last_complete);
        assert_eq!(st.finished_mode, "Martin M1");
        check(&dec, &pic, m.width, 12.0);
        assert!(dec.json().contains("\"finishedMode\":\"Martin M1\""));
    }

    #[test]
    fn pd120_with_vis_and_clock_error() {
        let m = modes().iter().find(|m| m.name == "PD 120").unwrap();
        let pic = picture(m.width, m.height);
        // The sender's sound card runs 200 ppm slow against ours (more samples per line):
        // the slant must be measured and corrected
        let mut tx = Tx::new(48_000.0 * (1.0 + 200e-6));
        tx.tone(1500.0, 400.0);
        tx.vis(m.vis);
        let to_y = |p: [f64; 3]| 16.0 + (65.738 * p[0] + 129.057 * p[1] + 25.064 * p[2]) / 256.0;
        let to_cr = |p: [f64; 3]| 128.0 + (112.439 * p[0] - 94.154 * p[1] - 18.285 * p[2]) / 256.0;
        let to_cb = |p: [f64; 3]| 128.0 + (-37.945 * p[0] - 74.494 * p[1] + 112.439 * p[2]) / 256.0;
        let scan = m.width as f64 * 0.19;
        for l in 0..m.height / 2 {
            let (r0, r1) = (2 * l * m.width, (2 * l + 1) * m.width);
            tx.tone(1200.0, 20.0);
            tx.tone(1500.0, 2.08);
            let y0: Vec<f64> = (0..m.width).map(|x| to_y(pic[r0 + x])).collect();
            let cr: Vec<f64> = (0..m.width).map(|x| 0.5 * (to_cr(pic[r0 + x]) + to_cr(pic[r1 + x]))).collect();
            let cb: Vec<f64> = (0..m.width).map(|x| 0.5 * (to_cb(pic[r0 + x]) + to_cb(pic[r1 + x]))).collect();
            let y1: Vec<f64> = (0..m.width).map(|x| to_y(pic[r1 + x])).collect();
            tx.pixels(&y0, scan);
            tx.pixels(&cr, scan);
            tx.pixels(&cb, scan);
            tx.pixels(&y1, scan);
        }
        tx.tone(1500.0, 300.0);
        let mut dec = Decoder::new();
        for c in tx.out.chunks(4096) {
            dec.feed(c, 48_000.0);
        }
        let st = dec.status();
        assert_eq!((st.finished, st.last_complete, st.finished_mode.as_str()), (1, true, "PD 120"));
        assert!((st.slant_ppm - 200.0).abs() < 40.0, "slant {:.0} ppm", st.slant_ppm);
        check(&dec, &pic, m.width, 14.0);
    }

    #[test]
    fn noise_starts_nothing() {
        let mut dec = Decoder::new();
        let mut seed = 9u32;
        let noise: Vec<f32> = (0..48_000 * 30)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32) - 0.5
            })
            .collect();
        for c in noise.chunks(4800) {
            dec.feed(c, 48_000.0);
        }
        assert_eq!(dec.status().finished, 0);
        assert!(!dec.status().receiving);
    }
}
