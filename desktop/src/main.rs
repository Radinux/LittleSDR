//! LittleSDR on the Linux desktop (prototype): the same Rust engine as the Android app,
//! through its service layer, with an egui interface. Spectrum + waterfall (click to tune,
//! wheel to zoom, drag to pan), source settings built from the engine's own menu, modes, a
//! DAB+ car radio (display, ⏮ ▶ ⏭, multiplex scan, stations, search by name), RDS, DRM.
//! Sound through cpal (PipeWire / ALSA).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use littlesdr::service::Service;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const MODES: [&str; 11] = ["NFM", "WFM", "AM", "DSB", "USB", "CW", "LSB", "RAW", "DAB+", "DRM", "LRPT"];
const DAB: usize = 8;
const WF_ROWS: usize = 360;
const AMBER: Color32 = Color32::from_rgb(246, 179, 85);

/// Band III DAB channels (name, kHz)
const DAB_CHANNELS: [(&str, u32); 38] = [
    ("5A", 174_928), ("5B", 176_640), ("5C", 178_352), ("5D", 180_064), ("6A", 181_936), ("6B", 183_648), ("6C", 185_360),
    ("6D", 187_072), ("7A", 188_928), ("7B", 190_640), ("7C", 192_352), ("7D", 194_064), ("8A", 195_936), ("8B", 197_648),
    ("8C", 199_360), ("8D", 201_072), ("9A", 202_928), ("9B", 204_640), ("9C", 206_352), ("9D", 208_064), ("10A", 209_936),
    ("10B", 211_648), ("10C", 213_360), ("10D", 215_072), ("11A", 216_928), ("11B", 218_640), ("11C", 220_352),
    ("11D", 222_064), ("12A", 223_936), ("12B", 225_648), ("12C", 227_360), ("12D", 229_072), ("13A", 230_784),
    ("13B", 232_496), ("13C", 234_208), ("13D", 235_776), ("13E", 237_488), ("13F", 239_200),
];

fn channel_at(hz: f64) -> Option<usize> {
    DAB_CHANNELS.iter().position(|c| (c.1 as f64 * 1e3 - hz).abs() < 1e3)
}

/// Command line: [--file iq.wav] [--mode N] [--no-audio] [--shot out.png]
#[derive(Default, Clone)]
struct Args {
    file: Option<String>,
    mode: Option<i32>,
    /// Save a screenshot of the window after a few seconds, then quit (tests; silent)
    shot: Option<String>,
    /// No sound output at all
    no_audio: bool,
}

fn args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--file" => a.file = it.next(),
            "--mode" => a.mode = it.next().and_then(|v| v.parse().ok()),
            "--shot" => a.shot = it.next(),
            "--no-audio" => a.no_audio = true,
            "-h" | "--help" => {
                println!("littlesdr-desktop [--file iq.wav] [--mode 0-10] [--no-audio] [--shot screenshot.png]");
                std::process::exit(0);
            }
            other => eprintln!("unknown option {other}"),
        }
    }
    a
}

fn main() -> eframe::Result<()> {
    let args = args();
    let root = std::env::var("XDG_DATA_HOME")
        .map(|d| format!("{d}/littlesdr"))
        .unwrap_or_else(|_| format!("{}/.local/share/littlesdr", std::env::var("HOME").unwrap_or_default()));
    let svc = Service::init(&root, 8192);
    // Tests (--shot) never play through the speakers
    let _audio = if args.no_audio || args.shot.is_some() { None } else { start_audio(svc) };
    if let Some(f) = &args.file {
        // An IQ recording as the receiver, played at once
        svc.select_source("Fichier IQ");
        svc.source_menu("file_path", false, 0, 0.0, f);
        if let Some(m) = args.mode {
            svc.set_mode(m);
        }
        if let Err(e) = svc.try_start() {
            eprintln!("{f}: {e}");
        }
    } else if let Some(m) = args.mode {
        svc.set_mode(m);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 780.0]).with_title("LittleSDR"),
        ..Default::default()
    };
    let shot = args.shot.clone();
    eframe::run_native("LittleSDR", options, Box::new(move |cc| Ok(Box::new(App::new(cc, svc, root, shot)))))
}

/// 48 kHz stereo output reading the engine's ring (it is resampled to 48 kHz already)
fn start_audio(svc: &'static Service) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let dev = host.default_output_device()?;
    let cfg = cpal::StreamConfig { channels: 2, sample_rate: cpal::SampleRate(48_000), buffer_size: cpal::BufferSize::Default };
    let ring = svc.audio();
    let stream = dev
        .build_output_stream(&cfg, move |data: &mut [f32], _| ring.read(data, 2), |e| eprintln!("audio: {e}"), None)
        .map_err(|e| eprintln!("audio output: {e}"))
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

/// Turbo colormap (Anton Mikhailov's polynomial, Apache-2.0)
fn turbo(x: f32) -> Color32 {
    let x = x.clamp(0.0, 1.0);
    let (x2, x3, x4, x5) = (x * x, x * x * x, x * x * x * x, x * x * x * x * x);
    let r = 0.135_721_38 + 4.615_392_6 * x - 42.660_32 * x2 + 132.131_08 * x3 - 152.942_39 * x4 + 59.286_38 * x5;
    let g = 0.091_402_61 + 2.194_188_4 * x + 4.842_966_6 * x2 - 14.185_033 * x3 + 4.277_298_6 * x4 + 2.829_566 * x5;
    let b = 0.106_673_3 + 12.641_946 * x - 60.582_05 * x2 + 110.362_77 * x3 - 89.903_11 * x4 + 27.348_25 * x5;
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgb(c(r), c(g), c(b))
}

/// Multiplex scan in progress: one channel at a time
struct Scan {
    idx: usize,
    since: Instant,
    synced_at: Option<Instant>,
    found: BTreeMap<String, String>,
    catalog: BTreeMap<String, Vec<(String, String)>>,
}

#[derive(PartialEq, Clone, Copy)]
enum DabView {
    Car,
    Spectrum,
}

struct App {
    svc: &'static Service,
    root: String,
    // waterfall
    wf: Vec<Color32>,
    wf_width: usize,
    wf_head: usize,
    wf_tex: Option<TextureHandle>,
    wf_key: (usize, u64, u64, u64),
    seen: u64,
    trace: Vec<f32>,
    db_min: f32,
    db_max: f32,
    /// Zoom: Hz shown across the width (0 = whole span), centre offset from the hardware centre
    view_bw: f64,
    view_off: f64,
    // controls
    freq_text: String,
    menu: Vec<Value>,
    menu_at: Instant,
    texts: std::collections::HashMap<String, String>,
    digital: Value,
    rds: Value,
    info_at: Instant,
    error: Option<String>,
    // DAB car radio
    dab_view: DabView,
    scan: Option<Scan>,
    found: BTreeMap<String, String>,
    catalog: BTreeMap<String, Vec<(String, String)>>,
    search: String,
    slide_id: u64,
    slide_tex: Option<TextureHandle>,
    // tests
    shot: Option<String>,
    started: Instant,
    shot_asked: bool,
}

impl App {
    fn new(_cc: &eframe::CreationContext, svc: &'static Service, root: String, shot: Option<String>) -> App {
        let st = svc.state();
        let saved: Value = std::fs::read_to_string(format!("{root}/desktop_dab.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let found = serde_json::from_value(saved["found"].clone()).unwrap_or_default();
        let catalog = serde_json::from_value(saved["catalog"].clone()).unwrap_or_default();
        App {
            svc,
            root,
            wf: Vec::new(),
            wf_width: 0,
            wf_head: 0,
            wf_tex: None,
            wf_key: (0, 0, 0, 0),
            seen: 0,
            trace: Vec::new(),
            db_min: -110.0,
            db_max: -30.0,
            view_bw: 0.0,
            view_off: 0.0,
            freq_text: format!("{:.6}", st[0] / 1e6),
            menu: Vec::new(),
            menu_at: Instant::now() - Duration::from_secs(10),
            texts: Default::default(),
            digital: Value::Null,
            rds: Value::Null,
            info_at: Instant::now(),
            error: None,
            dab_view: DabView::Car,
            scan: None,
            found,
            catalog,
            search: String::new(),
            slide_id: 0,
            slide_tex: None,
            shot,
            started: Instant::now(),
            shot_asked: false,
        }
    }

    fn save_dab(&self) {
        let v = json!({"found": self.found, "catalog": self.catalog});
        let _ = std::fs::create_dir_all(&self.root);
        let _ = std::fs::write(format!("{}/desktop_dab.json", self.root), v.to_string());
    }

    fn refresh_menu(&mut self, action: Option<(&str, bool, i32, f32, &str)>) {
        let (id, b, i, f, s) = action.unwrap_or(("", false, 0, 0.0, ""));
        let json = self.svc.source_menu(id, b, i, f, s);
        self.menu = serde_json::from_str::<Value>(&json).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        self.menu_at = Instant::now();
    }

    // ------------------------------------------------------------------ zoom

    /// Visible window (left Hz, width Hz) for a span and centre
    fn window(&self, center: f64, rate: f64) -> (f64, f64) {
        let vb = if self.view_bw > 0.0 { self.view_bw.min(rate) } else { rate };
        (center + self.view_off - vb / 2.0, vb)
    }

    fn clamp_view(&mut self, rate: f64) {
        if rate <= 0.0 {
            return;
        }
        if self.view_bw <= 0.0 || self.view_bw >= rate {
            self.view_bw = 0.0;
            self.view_off = 0.0;
            return;
        }
        let max_off = (rate - self.view_bw) / 2.0;
        self.view_off = self.view_off.clamp(-max_off, max_off);
    }

    /// Zoom by `factor` (< 1: in) keeping the frequency `at` at the same place on screen
    fn zoom(&mut self, factor: f64, at: f64, st: &[f64; 11]) {
        let (center, rate) = (st[1], st[2].max(1.0));
        let (left, vb) = self.window(center, rate);
        let frac = ((at - left) / vb).clamp(0.0, 1.0);
        let nvb = (vb * factor).clamp(2_000.0, rate);
        self.view_bw = nvb;
        self.view_off = at - frac * nvb + nvb / 2.0 - center;
        self.clamp_view(rate);
    }

    // ------------------------------------------------------------------ display data

    /// New FFT lines -> waterfall rows over the visible window; the whole waterfall is redone
    /// from the history when the window changes (zoom stays sharp)
    fn update_display(&mut self, width: usize, st: &[f64; 11]) {
        let (center, rate) = (st[1], st[2].max(1.0));
        let (left, vb) = self.window(center, rate);
        let h = self.svc.history().lock().unwrap();
        if h.bins == 0 {
            return;
        }
        let key = (width, h.generation, vb.to_bits(), (left - center).to_bits());
        if key != self.wf_key || width != self.wf_width {
            self.wf_key = key;
            self.wf_width = width;
            self.wf = vec![Color32::BLACK; width * WF_ROWS];
            self.wf_head = 0;
            self.seen = h.written.saturating_sub(WF_ROWS.min(h.rows) as u64);
        }
        let n = h.bins as f64;
        let b0 = (left - (center - rate / 2.0)) / rate * n;
        let b1 = b0 + vb / rate * n;
        let resample = |line: &[f32], out: &mut Vec<f32>| {
            out.clear();
            let last = line.len() - 1;
            for x in 0..width {
                let a = b0 + (b1 - b0) * x as f64 / width as f64;
                let b = b0 + (b1 - b0) * (x + 1) as f64 / width as f64;
                if b - a < 1.0 {
                    // zoomed in: interpolate between bins
                    let c = ((a + b) / 2.0 - 0.5).clamp(0.0, last as f64);
                    let (i, t) = (c.floor() as usize, (c - c.floor()) as f32);
                    let j = (i + 1).min(last);
                    out.push(line[i] * (1.0 - t) + line[j] * t);
                } else {
                    let (i, j) = ((a.floor() as usize).min(last), (b.ceil() as usize).clamp(1, line.len()));
                    out.push(line[i..j.max(i + 1)].iter().copied().fold(f32::MIN, f32::max));
                }
            }
        };
        let mut row = Vec::with_capacity(width);
        let first = self.seen.max(h.written.saturating_sub(h.rows as u64));
        let span = (self.db_max - self.db_min).max(1.0);
        for r in first..h.written {
            let off = (r % h.rows as u64) as usize * h.bins;
            resample(&h.lines[off..off + h.bins], &mut row);
            self.wf_head = (self.wf_head + WF_ROWS - 1) % WF_ROWS;
            let base = self.wf_head * width;
            for (x, v) in row.iter().enumerate() {
                self.wf[base + x] = turbo((v - self.db_min) / span);
            }
        }
        self.seen = h.written;
        resample(&h.latest, &mut self.trace);
    }

    fn waterfall_image(&self) -> ColorImage {
        let w = self.wf_width.max(1);
        let mut px = Vec::with_capacity(w * WF_ROWS);
        for i in 0..WF_ROWS {
            let r = (self.wf_head + i) % WF_ROWS;
            px.extend_from_slice(&self.wf[r * w..(r + 1) * w]);
        }
        ColorImage { size: [w, WF_ROWS], pixels: px }
    }

    // ------------------------------------------------------------------ top bar

    fn top_bar(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        ui.horizontal_wrapped(|ui| {
            let playing = st[7] != 0.0;
            if ui.add_sized([90.0, 30.0], egui::Button::new(if playing { "⏹ Arrêter" } else { "▶ Écouter" })).clicked() {
                self.toggle_play(playing);
            }
            ui.separator();
            let sel = self.svc.selected_source();
            egui::ComboBox::from_id_salt("source").selected_text(&sel).show_ui(ui, |ui| {
                for name in self.svc.source_names() {
                    if ui.selectable_label(name == sel, &name).clicked() {
                        self.svc.select_source(&name);
                        self.menu_at = Instant::now() - Duration::from_secs(10);
                    }
                }
            });
            ui.separator();
            ui.label("MHz");
            let r = ui.add(egui::TextEdit::singleline(&mut self.freq_text).desired_width(110.0).font(egui::TextStyle::Monospace));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(mhz) = self.freq_text.replace(',', ".").trim().parse::<f64>() {
                    self.svc.tune(mhz * 1e6);
                }
            }
            if !r.has_focus() {
                self.freq_text = format!("{:.6}", st[0] / 1e6);
            }
            ui.separator();
            let mode = st[6] as usize;
            for (i, m) in MODES.iter().enumerate() {
                if ui.selectable_label(mode == i, *m).clicked() {
                    self.svc.set_mode(i as i32);
                }
            }
            ui.separator();
            let mut vol = st[10] as f32;
            ui.label("🔊");
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.5).show_value(false)).changed() {
                self.svc.set_volume(vol);
            }
        });
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} · centre {:.4} MHz · span {:.0} kHz · bande {:.1} kHz · {:.0} dB",
                MODES.get(st[6] as usize).unwrap_or(&"?"),
                st[1] / 1e6,
                st[2] / 1e3,
                st[4] / 1e3,
                st[5]
            ));
            ui.separator();
            ui.label("Échelle dB");
            ui.add(egui::DragValue::new(&mut self.db_min).range(-160.0..=self.db_max - 10.0).speed(1.0));
            ui.add(egui::DragValue::new(&mut self.db_max).range(self.db_min + 10.0..=20.0).speed(1.0));
            ui.separator();
            ui.label("Zoom");
            if ui.button("➕").on_hover_text("Zoomer (ou molette)").clicked() {
                self.zoom(0.5, st[0], st);
            }
            if ui.button("➖").on_hover_text("Dézoomer").clicked() {
                self.zoom(2.0, st[0], st);
            }
            if ui.button("⤢").on_hover_text("Toute la bande").clicked() {
                self.view_bw = 0.0;
                self.view_off = 0.0;
            }
            let rate = st[2].max(1.0);
            let shown = if self.view_bw > 0.0 { self.view_bw.min(rate) } else { rate };
            ui.label(format!("{:.0} kHz affichés", shown / 1e3));
        });
    }

    fn toggle_play(&mut self, playing: bool) {
        if playing {
            self.scan = None;
            self.svc.stop();
        } else if let Err(e) = self.svc.try_start() {
            self.error = Some(e);
        }
    }

    // ------------------------------------------------------------------ spectrum

    fn spectrum(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        let avail = ui.available_size();
        let width = avail.x.max(100.0) as usize;
        self.clamp_view(st[2]);
        self.update_display(width, st);
        let spec_h = (avail.y * 0.35).max(80.0);
        let (resp, painter) = ui.allocate_painter(Vec2::new(avail.x, spec_h), Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 15, 24));
        let span = (self.db_max - self.db_min).max(1.0);
        for db in ((self.db_min as i32 / 10 * 10)..=(self.db_max as i32)).step_by(10) {
            let y = rect.bottom() - (db as f32 - self.db_min) / span * rect.height();
            if y > rect.top() && y < rect.bottom() {
                painter.hline(rect.x_range(), y, Stroke::new(1.0_f32, Color32::from_gray(40)));
                painter.text(Pos2::new(rect.left() + 4.0, y - 2.0), egui::Align2::LEFT_BOTTOM, format!("{db}"), egui::FontId::monospace(10.0), Color32::GRAY);
            }
        }
        let (center, rate, freq, bw) = (st[1], st[2].max(1.0), st[0], st[4]);
        let (left, vb) = self.window(center, rate);
        let fx = |f: f64| rect.left() + ((f - left) / vb) as f32 * rect.width();
        let xf = |x: f32| left + ((x - rect.left()) / rect.width()) as f64 * vb;
        // Frequency scale: a round step for the width shown
        let step = [1e3, 2e3, 5e3, 10e3, 20e3, 50e3, 100e3, 200e3, 500e3, 1e6].into_iter().find(|s| vb / s <= 10.0).unwrap_or(2e6);
        let mut f = (left / step).ceil() * step;
        while f < left + vb {
            let x = fx(f);
            painter.vline(x, rect.y_range(), Stroke::new(1.0_f32, Color32::from_gray(30)));
            painter.text(Pos2::new(x + 2.0, rect.bottom() - 2.0), egui::Align2::LEFT_BOTTOM, format!("{:.3}", f / 1e6), egui::FontId::monospace(10.0), Color32::GRAY);
            f += step;
        }
        if !self.trace.is_empty() {
            let pts: Vec<Pos2> = self
                .trace
                .iter()
                .enumerate()
                .map(|(x, v)| Pos2::new(rect.left() + x as f32 * rect.width() / self.trace.len() as f32, rect.bottom() - ((v - self.db_min) / span).clamp(0.0, 1.0) * rect.height()))
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.4_f32, AMBER)));
        }
        let mode = st[6] as usize;
        let (lo, hi) = match mode {
            4 => (freq, freq + bw), // USB: passband above the dial
            6 => (freq - bw, freq), // LSB
            _ => (freq - bw / 2.0, freq + bw / 2.0),
        };
        let band = Rect::from_x_y_ranges(fx(lo).max(rect.left())..=fx(hi).min(rect.right()), rect.y_range());
        if band.width() > 0.0 {
            painter.rect_filled(band, 0.0, Color32::from_rgba_unmultiplied(246, 179, 85, 28));
        }
        painter.vline(fx(freq), rect.y_range(), Stroke::new(1.5_f32, AMBER));
        // Waterfall
        let wf_h = ui.available_height().max(50.0);
        let (wresp, wpainter) = ui.allocate_painter(Vec2::new(avail.x, wf_h), Sense::click_and_drag());
        let img = self.waterfall_image();
        match &mut self.wf_tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.wf_tex = Some(ui.ctx().load_texture("waterfall", img, TextureOptions::LINEAR)),
        }
        if let Some(t) = &self.wf_tex {
            wpainter.image(t.id(), wresp.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        wpainter.vline(fx(freq), wresp.rect.y_range(), Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(246, 179, 85, 160)));
        // Mouse: click tunes, wheel zooms around the pointer, drag pans
        for r in [&resp, &wresp] {
            if let Some(p) = r.interact_pointer_pos().filter(|_| r.clicked()) {
                let step = if mode == 1 { 100e3 } else { 100.0 };
                self.svc.tune((xf(p.x) / step).round() * step);
            }
            if r.dragged() {
                self.view_off -= r.drag_delta().x as f64 / rect.width() as f64 * vb;
                if self.view_bw <= 0.0 {
                    self.view_bw = rate; // panning needs a window
                }
            }
            if r.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.5 {
                    if let Some(p) = r.hover_pos() {
                        self.zoom((0.9f64).powf(scroll as f64 / 20.0), xf(p.x), st);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ DAB car radio

    fn dab_services(&self) -> Vec<(String, String, bool)> {
        let mut v: Vec<(String, String, bool)> = self.digital["services"]
            .as_array()
            .map(|a| a.iter().map(|s| (s["sid"].as_str().unwrap_or("").to_string(), s["label"].as_str().unwrap_or("").to_string(), s["dabPlus"].as_bool().unwrap_or(true))).collect())
            .unwrap_or_default();
        v.sort_by_key(|s| s.1.to_lowercase());
        v
    }

    /// ⏮ ⏭: previous / next station of the multiplex
    fn step_station(&self, dir: i32) {
        let list: Vec<_> = self.dab_services().into_iter().filter(|s| s.2).collect();
        if list.is_empty() {
            return;
        }
        let playing = self.digital["playing"].as_str().unwrap_or("");
        let i = list.iter().position(|s| s.1 == playing);
        let n = list.len() as i32;
        let next = match i {
            Some(i) => ((i as i32 + dir).rem_euclid(n)) as usize,
            None if dir > 0 => 0,
            None => list.len() - 1,
        };
        self.svc.dab_select(&list[next].0);
    }

    /// Next / previous channel: among the multiplexes found once there are some
    fn step_channel(&self, st: &[f64; 11], dir: i32) {
        let here = channel_at(st[0]);
        let pool: Vec<usize> = if self.found.is_empty() {
            (0..DAB_CHANNELS.len()).collect()
        } else {
            DAB_CHANNELS.iter().enumerate().filter(|(_, c)| self.found.contains_key(c.0)).map(|(i, _)| i).collect()
        };
        let pos = here.and_then(|h| pool.iter().position(|&p| p == h));
        let n = pool.len() as i32;
        let next = match pos {
            Some(p) => pool[((p as i32 + dir).rem_euclid(n)) as usize],
            None => pool[if dir > 0 { 0 } else { pool.len() - 1 }],
        };
        self.svc.tune(DAB_CHANNELS[next].1 as f64 * 1e3);
    }

    /// The scan, one step per poll
    fn run_scan(&mut self) {
        let Some(s) = &mut self.scan else { return };
        let synced = self.digital["synced"].as_bool().unwrap_or(false);
        let ensemble = self.digital["ensemble"].as_str().unwrap_or("").to_string();
        let mut advance = false;
        if s.synced_at.is_none() && s.since.elapsed() > Duration::from_millis(1300) {
            if synced {
                s.synced_at = Some(Instant::now());
            } else {
                advance = true;
            }
        }
        if let Some(t) = s.synced_at {
            if !ensemble.is_empty() && t.elapsed() > Duration::from_millis(1500) {
                let ch = DAB_CHANNELS[s.idx].0.to_string();
                s.found.insert(ch.clone(), ensemble);
                let svcs = self.digital["services"]
                    .as_array()
                    .map(|a| a.iter().map(|x| (x["sid"].as_str().unwrap_or("").to_string(), x["label"].as_str().unwrap_or("").to_string())).collect())
                    .unwrap_or_default();
                s.catalog.insert(ch, svcs);
                advance = true;
            } else if t.elapsed() > Duration::from_secs(5) {
                advance = true;
            }
        }
        if !advance {
            return;
        }
        s.idx += 1;
        s.since = Instant::now();
        s.synced_at = None;
        if s.idx < DAB_CHANNELS.len() {
            self.svc.tune(DAB_CHANNELS[s.idx].1 as f64 * 1e3);
            return;
        }
        let done = self.scan.take().unwrap();
        if done.found.is_empty() {
            self.error = Some("Aucun multiplex DAB trouvé".into());
            return;
        }
        // A scan replaces the list: no multiplex from another region left
        self.found = done.found;
        self.catalog = done.catalog;
        self.save_dab();
        if let Some(c) = DAB_CHANNELS.iter().find(|c| self.found.contains_key(c.0)) {
            self.svc.tune(c.1 as f64 * 1e3);
        }
    }

    fn start_scan(&mut self, st: &[f64; 11]) {
        if self.scan.is_some() {
            self.scan = None;
            return;
        }
        if st[7] == 0.0 {
            if let Err(e) = self.svc.try_start() {
                self.error = Some(e);
                return;
            }
        }
        self.svc.tune(DAB_CHANNELS[0].1 as f64 * 1e3);
        self.scan = Some(Scan { idx: 0, since: Instant::now(), synced_at: None, found: BTreeMap::new(), catalog: BTreeMap::new() });
    }

    /// Keeps the list of multiplexes up to date while listening
    fn learn_multiplex(&mut self, st: &[f64; 11]) {
        if self.scan.is_some() || st[6] as usize != DAB || !self.digital["synced"].as_bool().unwrap_or(false) {
            return;
        }
        let Some(ch) = channel_at(st[0]).map(|i| DAB_CHANNELS[i].0.to_string()) else { return };
        let ens = self.digital["ensemble"].as_str().unwrap_or("").to_string();
        let svcs: Vec<(String, String)> = self.dab_services().into_iter().map(|s| (s.0, s.1)).collect();
        if ens.is_empty() || svcs.is_empty() {
            return;
        }
        if self.found.get(&ch) != Some(&ens) || self.catalog.get(&ch) != Some(&svcs) {
            self.found.insert(ch.clone(), ens);
            self.catalog.insert(ch, svcs);
            self.save_dab();
        }
    }

    fn update_slide(&mut self, ctx: &egui::Context) {
        let id = self.digital["slide"].as_u64().unwrap_or(0);
        if id == self.slide_id {
            return;
        }
        self.slide_id = id;
        if id == 0 {
            self.slide_tex = None;
            return;
        }
        let bytes = self.svc.dab_slide();
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgba = img.to_rgba8();
            let ci = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            self.slide_tex = Some(ctx.load_texture("slide", ci, TextureOptions::LINEAR));
        }
    }

    fn car_radio(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        let playing = st[7] != 0.0;
        let d = self.digital.clone();
        let station = d["playing"].as_str().unwrap_or("").to_string();
        let here = channel_at(st[0]);
        ui.columns(2, |cols| {
            // ----- the head unit
            let ui = &mut cols[0];
            egui::Frame::default().fill(ui.visuals().extreme_bg_color).corner_radius(18.0).inner_margin(18.0).show(ui, |ui| {
                ui.set_min_height((ui.available_height() - 40.0).max(200.0));
                ui.horizontal(|ui| {
                    let where_ = format!("{} · {}", here.map(|i| DAB_CHANNELS[i].0).unwrap_or("—"), d["ensemble"].as_str().filter(|s| !s.is_empty()).unwrap_or("…"));
                    ui.label(RichText::new(where_).size(16.0).color(Color32::GRAY));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let snr = d["snr"].as_f64().unwrap_or(0.0);
                        let bars = if !playing || !d["synced"].as_bool().unwrap_or(false) {
                            0
                        } else if snr < 6.0 {
                            1
                        } else if snr < 10.0 {
                            2
                        } else if snr < 15.0 {
                            3
                        } else {
                            4
                        };
                        let (r, p) = ui.allocate_painter(Vec2::new(34.0, 22.0), Sense::hover());
                        for i in 0..4 {
                            let h = 6.0 + i as f32 * 5.0;
                            let x = r.rect.left() + i as f32 * 8.0;
                            p.rect_filled(Rect::from_min_max(Pos2::new(x, r.rect.bottom() - h), Pos2::new(x + 5.0, r.rect.bottom())), 1.5, if i < bars { AMBER } else { Color32::from_gray(70) });
                        }
                        r.on_hover_text(format!("SNR {snr:.1} dB\n{}", d["status"].as_str().unwrap_or("")));
                    });
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let side = 180.0;
                    let (r, p) = ui.allocate_painter(Vec2::splat(side), Sense::hover());
                    p.rect_filled(r.rect, 16.0, Color32::from_gray(45));
                    if let Some(t) = &self.slide_tex {
                        let s = t.size_vec2();
                        let k = (side / s.x).min(side / s.y);
                        let img = Rect::from_center_size(r.rect.center(), s * k);
                        p.image(t.id(), img, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    } else {
                        p.text(r.rect.center(), egui::Align2::CENTER_CENTER, "📻", egui::FontId::proportional(64.0), AMBER);
                    }
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        let title = if !playing {
                            "Radio à l'arrêt".to_string()
                        } else if station.is_empty() {
                            "Choisis une station".to_string()
                        } else {
                            station.clone()
                        };
                        ui.label(RichText::new(title).size(34.0).strong());
                        if playing {
                            ui.label(RichText::new(d["text"].as_str().unwrap_or("")).size(18.0).color(AMBER));
                        }
                        if let Some(s) = &self.scan {
                            ui.label(RichText::new(format!("Recherche : canal {} ({}/{})", DAB_CHANNELS[s.idx].0, s.idx + 1, DAB_CHANNELS.len())).color(AMBER));
                        }
                    });
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let big = |t: &str| egui::Button::new(RichText::new(t).size(30.0)).min_size(Vec2::new(70.0, 56.0)).corner_radius(28.0);
                    if ui.add_enabled(playing, big("⏮")).on_hover_text("Station précédente").clicked() {
                        self.step_station(-1);
                    }
                    if ui.add(big(if playing { "⏹" } else { "▶" }).fill(AMBER.gamma_multiply(0.85))).clicked() {
                        self.toggle_play(playing);
                    }
                    if ui.add_enabled(playing, big("⏭")).on_hover_text("Station suivante").clicked() {
                        self.step_station(1);
                    }
                });
            });

            // ----- multiplex bar, search, stations
            let ui = &mut cols[1];
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("◀").size(20.0)).min_size(Vec2::splat(40.0))).on_hover_text("Multiplex précédent").clicked() {
                    self.step_channel(st, -1);
                }
                let label = here.map(|i| DAB_CHANNELS[i].0).unwrap_or("—");
                let ens = here.and_then(|i| self.found.get(DAB_CHANNELS[i].0)).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("channel").selected_text(RichText::new(format!("{label}  {ens}")).size(18.0)).width(220.0).show_ui(ui, |ui| {
                    for (n, khz) in DAB_CHANNELS {
                        let t = match self.found.get(n) {
                            Some(e) => format!("{n} · {e}"),
                            None => n.to_string(),
                        };
                        if ui.selectable_label(Some(n) == here.map(|i| DAB_CHANNELS[i].0), t).clicked() {
                            self.svc.tune(khz as f64 * 1e3);
                        }
                    }
                });
                if ui.add(egui::Button::new(RichText::new("▶").size(20.0)).min_size(Vec2::splat(40.0))).on_hover_text("Multiplex suivant").clicked() {
                    self.step_channel(st, 1);
                }
                let scan_label = if self.scan.is_some() { "⏹ Arrêter" } else { "📡 Rechercher" };
                if ui.add(egui::Button::new(RichText::new(scan_label).size(16.0)).min_size(Vec2::new(120.0, 40.0))).on_hover_text("Recherche des multiplex, canal par canal").clicked() {
                    self.start_scan(st);
                }
            });
            ui.add_space(6.0);
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Chercher une station…").desired_width(f32::INFINITY));
            ui.add_space(6.0);
            let q = self.search.trim().to_lowercase();
            let here_name = here.map(|i| DAB_CHANNELS[i].0.to_string());
            // (sid, name, channel when elsewhere, DAB+)
            let mut tiles: Vec<(String, String, Option<String>, bool)> = Vec::new();
            for (sid, label, plus) in self.dab_services() {
                if q.is_empty() || label.to_lowercase().contains(&q) {
                    tiles.push((sid, label, None, plus));
                }
            }
            if !q.is_empty() {
                for (ch, list) in &self.catalog {
                    if Some(ch) == here_name.as_ref() {
                        continue;
                    }
                    for (sid, label) in list {
                        if label.to_lowercase().contains(&q) {
                            tiles.push((sid.clone(), label.clone(), Some(ch.clone()), true));
                        }
                    }
                }
            } else if tiles.is_empty() {
                // stopped or not synced yet: the stations already known on this channel
                if let Some(list) = here_name.as_ref().and_then(|c| self.catalog.get(c)) {
                    tiles.extend(list.iter().map(|(s, l)| (s.clone(), l.clone(), here_name.clone(), true)));
                }
            }
            if tiles.is_empty() {
                let hint = if !q.is_empty() {
                    "Aucune station de ce nom dans les multiplex trouvés"
                } else if playing {
                    "Lecture de la liste des stations…"
                } else {
                    "Appuie sur ▶, ou lance une recherche"
                };
                ui.label(RichText::new(hint).color(Color32::GRAY));
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(8.0);
                    for (sid, name, other, plus) in tiles {
                        let elsewhere = other.as_ref().filter(|c| Some(*c) != here_name.as_ref());
                        let current = elsewhere.is_none() && name == station;
                        let text = match elsewhere {
                            Some(ch) => format!("{name}\n{ch} · {}", self.found.get(ch).cloned().unwrap_or_default()),
                            None => name.clone(),
                        };
                        let b = egui::Button::new(RichText::new(text).size(16.0).strong()).min_size(Vec2::new(170.0, 64.0)).corner_radius(14.0);
                        let b = if current { b.fill(AMBER.gamma_multiply(0.55)) } else { b };
                        if ui.add_enabled(plus, b).clicked() {
                            self.svc.dab_select(&sid);
                            if let Some(ch) = elsewhere.and_then(|c| DAB_CHANNELS.iter().find(|x| x.0 == c.as_str())) {
                                self.svc.tune(ch.1 as f64 * 1e3);
                            }
                            if st[7] == 0.0 {
                                self.toggle_play(false);
                            }
                            self.search.clear();
                        }
                    }
                });
            });
        });
    }

    // ------------------------------------------------------------------ side panel

    /// The source's settings, built from the engine's menu (combo, checkbox, slider, text)
    fn source_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(format!("Récepteur : {}", self.svc.selected_source()));
        let mut action: Option<(String, bool, i32, f32, String)> = None;
        for w in self.menu.clone() {
            let id = w["id"].as_str().unwrap_or("").to_string();
            let label = w["label"].as_str().unwrap_or("");
            match w["t"].as_str().unwrap_or("") {
                "combo" => {
                    let items: Vec<String> = w["items"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                    let cur = w["value"].as_i64().unwrap_or(0) as usize;
                    egui::ComboBox::from_label(label).selected_text(items.get(cur).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                        for (i, it) in items.iter().enumerate() {
                            if ui.selectable_label(i == cur, it).clicked() {
                                action = Some((id.clone(), false, i as i32, 0.0, String::new()));
                            }
                        }
                    });
                }
                "checkbox" => {
                    let mut v = w["value"].as_bool().unwrap_or(false);
                    if ui.checkbox(&mut v, label).changed() {
                        action = Some((id, v, 0, 0.0, String::new()));
                    }
                }
                "slider_float" => {
                    let mut v = w["value"].as_f64().unwrap_or(0.0) as f32;
                    let (lo, hi) = (w["min"].as_f64().unwrap_or(0.0) as f32, w["max"].as_f64().unwrap_or(1.0) as f32);
                    let dis = w["disabled"].as_bool().unwrap_or(false);
                    ui.label(label);
                    if ui.add_enabled(!dis, egui::Slider::new(&mut v, lo..=hi)).drag_stopped() {
                        action = Some((id, false, 0, v, String::new()));
                    }
                }
                "input_text" => {
                    let entry = self.texts.entry(id.clone()).or_insert_with(|| w["value"].as_str().unwrap_or("").to_string());
                    ui.label(label);
                    let r = ui.text_edit_singleline(entry);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = Some((id.clone(), false, 0, 0.0, entry.clone()));
                    }
                }
                "text" => {
                    ui.small(w["value"].as_str().unwrap_or(""));
                }
                _ => {}
            }
        }
        if let Some((id, b, i, f, s)) = action {
            self.refresh_menu(Some((&id, b, i, f, &s)));
        }
    }

    fn info_panel(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        match st[6] as usize {
            1 => {
                ui.heading("RDS");
                let ps = self.rds["ps"].as_str().unwrap_or("");
                let rt = self.rds["rt"].as_str().unwrap_or("");
                ui.label(RichText::new(if ps.is_empty() { "—" } else { ps }).size(22.0).strong());
                ui.label(rt);
            }
            9 => {
                ui.heading("DRM");
                let d = &self.digital;
                ui.label(RichText::new(d["station"].as_str().unwrap_or("Recherche…")).size(22.0).strong());
                ui.label(d["info"].as_str().unwrap_or(""));
                ui.label(d["text"].as_str().unwrap_or(""));
            }
            _ => {}
        }
        if matches!(st[6] as usize, DAB | 9) {
            if let Some(s) = self.digital["status"].as_str() {
                ui.separator();
                ui.small(s);
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let st = self.svc.state();
        // A recording plays its own band: a frequency outside it comes back to its centre
        if st[7] != 0.0 && st[2] > 0.0 && (st[0] - st[1]).abs() > st[2] / 2.0 && self.svc.selected_source() == "Fichier IQ" {
            self.svc.tune(st[1]);
        }
        if self.menu_at.elapsed() > Duration::from_secs(2) {
            self.refresh_menu(None);
        }
        let every = if self.scan.is_some() { 200 } else { 700 };
        if self.info_at.elapsed() > Duration::from_millis(every) {
            self.info_at = Instant::now();
            self.digital = serde_json::from_str(&self.svc.digital_json()).unwrap_or(Value::Null);
            self.rds = serde_json::from_str(&self.svc.rds_json()).unwrap_or(Value::Null);
            let e = self.svc.source_error();
            if !e.is_empty() && st[7] == 0.0 {
                self.error = Some(e);
            }
            self.run_scan();
            self.learn_multiplex(&st);
            self.update_slide(ctx);
        }
        let dab = st[6] as usize == DAB;
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui, &st));
        let car = dab && self.dab_view == DabView::Car;
        if !car {
            egui::SidePanel::right("side").default_width(300.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.source_panel(ui);
                    ui.separator();
                    self.info_panel(ui, &st);
                });
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if dab {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.dab_view, DabView::Car, RichText::new("🚗 Autoradio").size(16.0));
                    ui.selectable_value(&mut self.dab_view, DabView::Spectrum, RichText::new("〰 Spectre").size(16.0));
                });
                ui.add_space(6.0);
            }
            if car {
                self.car_radio(ui, &st);
            } else {
                self.spectrum(ui, &st);
            }
        });
        if let Some(e) = self.error.clone() {
            egui::Window::new("Réception").collapsible(false).show(ctx, |ui| {
                ui.label(e);
                if ui.button("OK").clicked() {
                    self.error = None;
                }
            });
        }
        // --shot: photograph the window after 6 s, save it as PNG, quit
        if let Some(path) = self.shot.clone() {
            if !self.shot_asked && self.started.elapsed() > Duration::from_secs(6) {
                self.shot_asked = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let img = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = img {
                let rgb: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b()]).collect();
                let _ = std::fs::write(&path, littlesdr::lrpt::image::png(img.size[0], img.size[1], 3, &rgb));
                self.svc.stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
