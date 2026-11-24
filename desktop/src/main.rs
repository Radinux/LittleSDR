//! LittleSDR on the Linux desktop (prototype): the same Rust engine as the Android app,
//! through its service layer, with an egui interface. Spectrum + waterfall (click to tune),
//! source settings built from the engine's own menu, modes, DAB+ stations, RDS, DRM.
//! Sound through cpal (PipeWire / ALSA).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use littlesdr::service::Service;
use serde_json::Value;
use std::time::{Duration, Instant};

const MODES: [&str; 11] = ["NFM", "WFM", "AM", "DSB", "USB", "CW", "LSB", "RAW", "DAB+", "DRM", "LRPT"];
const WF_ROWS: usize = 360;

/// Band III DAB channels (name, kHz)
const DAB_CHANNELS: [(&str, u32); 38] = [
    ("5A", 174_928), ("5B", 176_640), ("5C", 178_352), ("5D", 180_064), ("6A", 181_936), ("6B", 183_648), ("6C", 185_360),
    ("6D", 187_072), ("7A", 188_928), ("7B", 190_640), ("7C", 192_352), ("7D", 194_064), ("8A", 195_936), ("8B", 197_648),
    ("8C", 199_360), ("8D", 201_072), ("9A", 202_928), ("9B", 204_640), ("9C", 206_352), ("9D", 208_064), ("10A", 209_936),
    ("10B", 211_648), ("10C", 213_360), ("10D", 215_072), ("11A", 216_928), ("11B", 218_640), ("11C", 220_352),
    ("11D", 222_064), ("12A", 223_936), ("12B", 225_648), ("12C", 227_360), ("12D", 229_072), ("13A", 230_784),
    ("13B", 232_496), ("13C", 234_208), ("13D", 235_776), ("13E", 237_488), ("13F", 239_200),
];

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
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 760.0]).with_title("LittleSDR"),
        ..Default::default()
    };
    eframe::run_native("LittleSDR", options, Box::new(move |cc| Ok(Box::new(App::new(cc, svc, args.shot.clone())))))
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

struct App {
    svc: &'static Service,
    wf: Vec<Color32>,
    wf_width: usize,
    wf_head: usize,
    wf_tex: Option<TextureHandle>,
    seen: u64,
    trace: Vec<f32>,
    db_min: f32,
    db_max: f32,
    freq_text: String,
    menu: Vec<Value>,
    menu_at: Instant,
    texts: std::collections::HashMap<String, String>,
    digital: Value,
    rds: Value,
    info_at: Instant,
    error: Option<String>,
    shot: Option<String>,
    started: Instant,
    shot_asked: bool,
}

impl App {
    fn new(_cc: &eframe::CreationContext, svc: &'static Service, shot: Option<String>) -> App {
        let st = svc.state();
        App {
            svc,
            wf: Vec::new(),
            wf_width: 0,
            wf_head: 0,
            wf_tex: None,
            seen: 0,
            trace: Vec::new(),
            db_min: -110.0,
            db_max: -30.0,
            freq_text: format!("{:.6}", st[0] / 1e6),
            menu: Vec::new(),
            menu_at: Instant::now() - Duration::from_secs(10),
            texts: Default::default(),
            digital: Value::Null,
            rds: Value::Null,
            info_at: Instant::now(),
            error: None,
            shot,
            started: Instant::now(),
            shot_asked: false,
        }
    }

    fn refresh_menu(&mut self, action: Option<(&str, bool, i32, f32, &str)>) {
        let (id, b, i, f, s) = action.unwrap_or(("", false, 0, 0.0, ""));
        let json = self.svc.source_menu(id, b, i, f, s);
        self.menu = serde_json::from_str::<Value>(&json).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        self.menu_at = Instant::now();
    }

    /// New FFT lines -> waterfall rows (the whole span across the width)
    fn update_display(&mut self, width: usize) {
        let h = self.svc.history().lock().unwrap();
        if h.bins == 0 {
            return;
        }
        if width != self.wf_width {
            self.wf_width = width;
            self.wf = vec![Color32::BLACK; width * WF_ROWS];
            self.wf_head = 0;
            self.seen = h.written.saturating_sub(WF_ROWS as u64);
        }
        let resample = |line: &[f32], out: &mut Vec<f32>| {
            out.clear();
            let n = line.len();
            for x in 0..width {
                let (a, b) = (x * n / width, ((x + 1) * n / width).max(x * n / width + 1).min(n));
                out.push(line[a..b].iter().copied().fold(f32::MIN, f32::max));
            }
        };
        let mut row = Vec::with_capacity(width);
        let first = self.seen.max(h.written.saturating_sub(h.rows as u64));
        for r in first..h.written {
            let off = (r % h.rows as u64) as usize * h.bins;
            resample(&h.lines[off..off + h.bins], &mut row);
            self.wf_head = (self.wf_head + WF_ROWS - 1) % WF_ROWS;
            let base = self.wf_head * width;
            let span = (self.db_max - self.db_min).max(1.0);
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

    fn top_bar(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        ui.horizontal_wrapped(|ui| {
            let playing = st[7] != 0.0;
            if ui.add_sized([90.0, 30.0], egui::Button::new(if playing { "⏹ Arrêter" } else { "▶ Écouter" })).clicked() {
                if playing {
                    self.svc.stop();
                } else if let Err(e) = self.svc.try_start() {
                    self.error = Some(e);
                }
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
        let level = st[5];
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} · centre {:.4} MHz · span {:.0} kHz · bande {:.1} kHz · {:.0} dB",
                MODES.get(st[6] as usize).unwrap_or(&"?"),
                st[1] / 1e6,
                st[2] / 1e3,
                st[4] / 1e3,
                level
            ));
            ui.separator();
            ui.label("Échelle dB");
            ui.add(egui::DragValue::new(&mut self.db_min).range(-160.0..=self.db_max - 10.0).speed(1.0));
            ui.add(egui::DragValue::new(&mut self.db_max).range(self.db_min + 10.0..=20.0).speed(1.0));
        });
    }

    fn spectrum(&mut self, ui: &mut egui::Ui, st: &[f64; 11]) {
        let avail = ui.available_size();
        let width = avail.x.max(100.0) as usize;
        self.update_display(width);
        let spec_h = (avail.y * 0.35).max(80.0);
        let (resp, painter) = ui.allocate_painter(Vec2::new(avail.x, spec_h), Sense::click());
        let rect = resp.rect;
        painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 15, 24));
        let span = (self.db_max - self.db_min).max(1.0);
        for db in ((self.db_min as i32 / 10 * 10)..=(self.db_max as i32)).step_by(10) {
            let y = rect.bottom() - (db as f32 - self.db_min) / span * rect.height();
            if y > rect.top() && y < rect.bottom() {
                painter.hline(rect.x_range(), y, Stroke::new(1.0, Color32::from_gray(40)));
                painter.text(Pos2::new(rect.left() + 4.0, y - 2.0), egui::Align2::LEFT_BOTTOM, format!("{db}"), egui::FontId::monospace(10.0), Color32::GRAY);
            }
        }
        if !self.trace.is_empty() {
            let pts: Vec<Pos2> = self
                .trace
                .iter()
                .enumerate()
                .map(|(x, v)| Pos2::new(rect.left() + x as f32 * rect.width() / self.trace.len() as f32, rect.bottom() - ((v - self.db_min) / span).clamp(0.0, 1.0) * rect.height()))
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.4, Color32::from_rgb(246, 179, 85))));
        }
        // Channel and VFO
        let (center, rate, freq, bw) = (st[1], st[2].max(1.0), st[0], st[4]);
        let fx = |f: f64| rect.left() + ((f - (center - rate / 2.0)) / rate) as f32 * rect.width();
        let mode = st[6] as usize;
        let (lo, hi) = match mode {
            4 => (freq, freq + bw), // USB: passband above the dial
            6 => (freq - bw, freq), // LSB
            _ => (freq - bw / 2.0, freq + bw / 2.0),
        };
        painter.rect_filled(Rect::from_x_y_ranges(fx(lo)..=fx(hi), rect.y_range()), 0.0, Color32::from_rgba_unmultiplied(246, 179, 85, 28));
        painter.vline(fx(freq), rect.y_range(), Stroke::new(1.5, Color32::from_rgb(246, 179, 85)));
        // Waterfall
        let wf_h = ui.available_height().max(50.0);
        let (wresp, wpainter) = ui.allocate_painter(Vec2::new(avail.x, wf_h), Sense::click());
        let img = self.waterfall_image();
        match &mut self.wf_tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.wf_tex = Some(ui.ctx().load_texture("waterfall", img, TextureOptions::LINEAR)),
        }
        if let Some(t) = &self.wf_tex {
            wpainter.image(t.id(), wresp.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        wpainter.vline(fx(freq), wresp.rect.y_range(), Stroke::new(1.0, Color32::from_rgba_unmultiplied(246, 179, 85, 160)));
        // Click to tune (rounded to 100 Hz, 100 kHz in WFM)
        for r in [&resp, &wresp] {
            if let Some(p) = r.interact_pointer_pos().filter(|_| r.clicked()) {
                let f = center - rate / 2.0 + ((p.x - rect.left()) / rect.width()) as f64 * rate;
                let step = if mode == 1 { 100e3 } else { 100.0 };
                self.svc.tune((f / step).round() * step);
            }
        }
    }

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
                ui.label(egui::RichText::new(if ps.is_empty() { "—" } else { ps }).size(22.0).strong());
                ui.label(rt);
            }
            8 => {
                ui.heading("DAB+");
                let here = DAB_CHANNELS.iter().find(|c| (c.1 as f64 * 1e3 - st[0]).abs() < 1e3).map(|c| c.0).unwrap_or("—");
                egui::ComboBox::from_label("Canal").selected_text(here).show_ui(ui, |ui| {
                    for (n, khz) in DAB_CHANNELS {
                        if ui.selectable_label(n == here, n).clicked() {
                            self.svc.tune(khz as f64 * 1e3);
                        }
                    }
                });
                let d = &self.digital;
                ui.label(egui::RichText::new(d["ensemble"].as_str().unwrap_or("")).italics());
                let playing = d["playing"].as_str().unwrap_or("");
                ui.label(egui::RichText::new(if playing.is_empty() { "Choisis une station" } else { playing }).size(22.0).strong());
                ui.label(d["text"].as_str().unwrap_or(""));
                ui.separator();
                let mut services: Vec<(String, String, bool)> = d["services"]
                    .as_array()
                    .map(|a| a.iter().map(|s| (s["sid"].as_str().unwrap_or("").to_string(), s["label"].as_str().unwrap_or("").to_string(), s["dabPlus"].as_bool().unwrap_or(true))).collect())
                    .unwrap_or_default();
                services.sort_by_key(|s| s.1.to_lowercase());
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for (sid, label, plus) in services {
                        if ui.add_enabled(plus, egui::SelectableLabel::new(label == playing, &label)).clicked() {
                            self.svc.dab_select(&sid);
                        }
                    }
                });
            }
            9 => {
                ui.heading("DRM");
                let d = &self.digital;
                ui.label(egui::RichText::new(d["station"].as_str().unwrap_or("Recherche…")).size(22.0).strong());
                ui.label(d["info"].as_str().unwrap_or(""));
                ui.label(d["text"].as_str().unwrap_or(""));
            }
            _ => {}
        }
        if matches!(st[6] as usize, 8 | 9) {
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
        if self.info_at.elapsed() > Duration::from_millis(700) {
            self.info_at = Instant::now();
            self.digital = serde_json::from_str(&self.svc.digital_json()).unwrap_or(Value::Null);
            self.rds = serde_json::from_str(&self.svc.rds_json()).unwrap_or(Value::Null);
            let e = self.svc.source_error();
            if !e.is_empty() && st[7] == 0.0 {
                self.error = Some(e);
            }
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui, &st));
        egui::SidePanel::right("side").default_width(300.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.source_panel(ui);
                ui.separator();
                self.info_panel(ui, &st);
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| self.spectrum(ui, &st));
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
