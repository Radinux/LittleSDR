//! Command-line LittleSDR: listen with the engine, change settings live.
//!
//!   littlesdr [--usb [N] | --rtltcp HOST:PORT] [--freq 96.5M] [--rate 2.4M]
//!             [--mode wfm|nfm|am|usb|lsb|cw] [--gain DB|auto] [--ds 0|1|2] [--bias-tee]
//!             [--wav out.wav] [--no-audio] [--verbose] [--script "3s rate 1.024M; 6s quit"]
//!
//! Live commands on stdin: freq <f> (or +1k / -500 to step), rate <r>, mode <m>, bw <Hz>,
//!   pitch <Hz>, gain <dB>|auto, vol <0-2>, ds 0|1|2, bt on|off (bias-tee), stereo on|off,
//!   nb on|off [threshold], fx <filter> on|off, fx <filter> <param> <value>, meter on|off,
//!   rtty on|off, rtty shift|baud|center <v>, rtty rev|ita2|sql on|off, status, quit.
//!   Filters: hp lp cw notch anotch nr eq agc limit

use littlesdr::audio::{AudioSink, WavSink};
use littlesdr::engine::{self, Engine, EngineCtl, Mode};
use littlesdr::source::{self, Source, SourceConfig, SourceCtl};
use std::io::BufRead;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// "96.5M", "2.4M", "1024k", "145.8e6" -> Hz
fn parse_hz(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, mul) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1e3),
        'M' | 'm' => (&s[..s.len() - 1], 1e6),
        'G' | 'g' => (&s[..s.len() - 1], 1e9),
        _ => (s, 1.0),
    };
    num.parse::<f64>().ok().map(|v| v * mul)
}

/// Hardware center for a new tuning: 250 kHz below the tuned frequency, so the RTL-SDR's DC
/// spike stays out of the channel.
fn plan(freq: f64, rate: f64) -> u64 {
    let offset = if rate >= 1e6 { 250e3 } else { 0.0 };
    (freq - offset).round() as u64
}

/// Whether the hardware must move to receive `freq`: the channel must fit in the clean part
/// of the captured band and stay clear of the DC spike. Otherwise only the VFO moves.
fn needs_retune(freq: f64, center: u64, rate: f64, mode: Mode) -> bool {
    let d = (freq - center as f64).abs();
    let edge = rate * 0.45 - mode.half_band();
    let guard = if rate >= 1e6 { mode.half_band() + 5e3 } else { 0.0 };
    d > edge || d < guard
}

enum Cmd {
    Line(String),
}

/// Ctrl+C / kill: set here, turned into a clean "quit" (the source then switches the
/// bias-tee off and closes the dongle instead of leaving it powered).
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_signal(_: i32) {
    INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut usb: Option<u32> = Some(0);
    let mut tcp: Option<String> = None;
    let mut freq = 96.5e6;
    let mut rate = 2.4e6;
    let mut mode = Mode::Wfm;
    let mut gain: Option<f32> = None;
    let mut ds: u8 = 0;
    let mut bias_tee = false;
    let mut wav: Option<String> = None;
    let mut audio = cfg!(feature = "audio");
    let mut script = String::new();
    let mut verbose = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--usb" => usb = Some(args.next().and_then(|v| v.parse().ok()).unwrap_or(0)),
            "--rtltcp" => {
                tcp = args.next();
                usb = None;
            }
            "--freq" => freq = args.next().and_then(|v| parse_hz(&v)).expect("--freq"),
            "--rate" => rate = args.next().and_then(|v| parse_hz(&v)).expect("--rate"),
            "--mode" => mode = args.next().and_then(|v| Mode::parse(&v)).expect("--mode wfm|nfm|am|usb|lsb|cw"),
            "--gain" => gain = args.next().and_then(|v| v.parse().ok()),
            "--bias-tee" => bias_tee = true,
            "--ds" => ds = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--wav" => wav = args.next(),
            "--no-audio" => audio = false,
            "--verbose" | "-v" => verbose = true,
            "--script" => script = args.next().unwrap_or_default(),
            "-h" | "--help" => {
                println!("{}", include_str!("main.rs").lines().take(11).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
                return;
            }
            other => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
        }
    }

    // Source
    let src: Box<dyn Source> = match (&tcp, usb) {
        (Some(addr), _) => Box::new(source::rtltcp::RtlTcp { addr: addr.clone() }),
        #[cfg(feature = "rtlsdr")]
        (None, Some(i)) => {
            let devs = source::rtlsdr::list();
            if devs.is_empty() {
                eprintln!("no RTL-SDR found");
                std::process::exit(1);
            }
            eprintln!("[source] RTL-SDR #{i}: {}", devs.get(i as usize).cloned().unwrap_or_default());
            Box::new(source::rtlsdr::RtlSdr { index: i })
        }
        _ => {
            eprintln!("no source (--usb or --rtltcp)");
            std::process::exit(2);
        }
    };
    let center = plan(freq, rate);
    let (iq_tx, iq_rx) = mpsc::sync_channel(64);
    let src = source::spawn(src, SourceConfig { rate: rate as u32, center, gain, direct_sampling: ds, bias_tee }, iq_tx);

    // Output
    let sink: Box<dyn AudioSink> = match (&wav, audio) {
        (Some(p), _) => Box::new(WavSink::create(p).expect("wav file")),
        #[cfg(feature = "audio")]
        (None, true) => match littlesdr::audio::CpalSink::open() {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("[audio] {e}");
                std::process::exit(1);
            }
        },
        _ => Box::new(WavSink::create("/dev/null").unwrap()),
    };

    let (eng_tx, eng_rx) = mpsc::channel();
    let eng = std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            let mut e = Engine::new(mode, freq);
            e.echo = true;
            engine::run(iq_rx, eng_rx, sink, e, verbose)
        })
        .unwrap();
    eprintln!("[littlesdr] {:.3} MHz {:?}, {:.0} S/s", freq / 1e6, mode, rate);

    // Commands: stdin and the optional script ("<seconds>s <command>; ...")
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    {
        let tx = cmd_tx.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(Cmd::Line(line)).is_err() {
                    break;
                }
            }
        });
    }
    if !script.is_empty() {
        let tx = cmd_tx.clone();
        let t0 = Instant::now();
        std::thread::spawn(move || {
            for item in script.split(';') {
                let item = item.trim();
                let Some((t, c)) = item.split_once(' ') else { continue };
                let at = t.trim_end_matches('s').parse::<f64>().unwrap_or(0.0);
                let wait = Duration::from_secs_f64(at).saturating_sub(t0.elapsed());
                std::thread::sleep(wait);
                if tx.send(Cmd::Line(c.to_string())).is_err() {
                    break;
                }
            }
        });
    }
    {
        unsafe {
            signal(2, on_signal); // SIGINT
            signal(15, on_signal); // SIGTERM
        }
        let tx = cmd_tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            if INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = tx.send(Cmd::Line("quit".into()));
                break;
            }
        });
    }
    drop(cmd_tx);

    let mut cur_freq = freq;
    let mut cur_rate = rate;
    let mut cur_center = center;
    let mut cur_mode = mode;
    // Moves the hardware only if the channel no longer fits where it is
    let follow = |f: f64, r: f64, m: Mode, c: &mut u64| -> bool {
        if needs_retune(f, *c, r, m) {
            *c = plan(f, r);
            let _ = src.ctl.send(SourceCtl::Center(*c));
            true
        } else {
            false
        }
    };
    let on = |v: &str| matches!(v, "on" | "1" | "yes");
    for Cmd::Line(line) in cmd_rx {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (c, v) = (words.first().copied().unwrap_or(""), words.get(1).copied().unwrap_or(""));
        match c {
            "stereo" => {
                let _ = eng_tx.send(EngineCtl::Stereo(on(v)));
            }
            "nb" => {
                let _ = eng_tx.send(EngineCtl::Blanker(on(v), words.get(2).and_then(|t| t.parse().ok())));
            }
            "fx" => match (words.get(2), words.get(3)) {
                (Some(&state), None) => {
                    let _ = eng_tx.send(EngineCtl::Filter(v.to_string(), on(state)));
                }
                (Some(&key), Some(val)) => match val.parse() {
                    Ok(x) => {
                        let _ = eng_tx.send(EngineCtl::Param(v.to_string(), key.to_string(), x));
                    }
                    Err(_) => eprintln!("fx {v} {key} <number>"),
                },
                _ => eprintln!("fx <filter> on|off  |  fx <filter> <param> <value>"),
            },
            "rtty" => match (v, words.get(2)) {
                ("on" | "off" | "", None) => {
                    let _ = eng_tx.send(EngineCtl::Rtty(v != "off"));
                }
                (key, Some(val)) => {
                    let x = match *val {
                        "on" | "yes" => Some(1.0),
                        "off" | "no" => Some(0.0),
                        n => n.parse().ok(),
                    };
                    match x {
                        Some(x) => {
                            let _ = eng_tx.send(EngineCtl::RttyParam(key.to_string(), x));
                        }
                        None => eprintln!("rtty {key} <number|on|off>"),
                    }
                }
                _ => eprintln!("rtty on|off  |  rtty shift 170 | baud 45.45 | center 2210 | rev on | ita2 on | sql off"),
            },
            "meter" | "sm" => {
                let _ = eng_tx.send(EngineCtl::Meter(v.is_empty() || on(v)));
            }
            "status" | "s" => {
                let _ = eng_tx.send(EngineCtl::Status);
            }
            "freq" | "f" => {
                let step = v.starts_with('+') || v.starts_with('-');
                if let Some(f) = parse_hz(v.trim_start_matches('+')) {
                    cur_freq = if step { cur_freq + f } else { f };
                    let _ = eng_tx.send(EngineCtl::Tune(cur_freq));
                    let moved = follow(cur_freq, cur_rate, cur_mode, &mut cur_center);
                    eprintln!("[littlesdr] {:.4} MHz{}", cur_freq / 1e6, if moved { " (hardware retuned)" } else { "" });
                }
            }
            "rate" | "r" => {
                if let Some(r) = parse_hz(v) {
                    cur_rate = r;
                    follow(cur_freq, cur_rate, cur_mode, &mut cur_center);
                    let _ = src.ctl.send(SourceCtl::Rate(r as u32));
                    eprintln!("[littlesdr] rate -> {:.0} S/s", r);
                }
            }
            "mode" | "m" => {
                if let Some(m) = Mode::parse(v) {
                    cur_mode = m;
                    let _ = eng_tx.send(EngineCtl::Mode(m));
                    follow(cur_freq, cur_rate, cur_mode, &mut cur_center);
                }
            }
            "bw" => {
                if let Some(b) = parse_hz(v) {
                    let _ = eng_tx.send(EngineCtl::Bandwidth(b));
                }
            }
            "pitch" => {
                if let Some(p) = parse_hz(v) {
                    let _ = eng_tx.send(EngineCtl::CwPitch(p));
                }
            }
            "bt" => {
                let _ = src.ctl.send(SourceCtl::BiasTee(on(v)));
                eprintln!("[littlesdr] bias-tee {}", if on(v) { "on (4.5 V on the antenna)" } else { "off" });
            }
            "ds" => {
                let m: u8 = v.parse().unwrap_or(0);
                let _ = src.ctl.send(SourceCtl::DirectSampling(m));
                eprintln!("[littlesdr] direct sampling {}", ["off", "I branch", "Q branch"].get(m as usize).unwrap_or(&"?"));
            }
            "gain" | "g" => {
                let g: Option<f32> = v.parse().ok();
                let _ = src.ctl.send(SourceCtl::Gain(g));
                match g {
                    Some(db) => eprintln!("[littlesdr] gain {db} dB (nearest step of the tuner)"),
                    None => eprintln!("[littlesdr] gain auto"),
                }
            }
            "vol" => {
                if let Ok(x) = v.parse() {
                    let _ = eng_tx.send(EngineCtl::Volume(x));
                }
            }
            "quit" | "q" | "exit" => break,
            "" => {}
            other => eprintln!("commands: freq | rate | mode | bw | pitch | gain | vol | ds | bt | meter | rtty | stereo | nb | fx | status | quit  ({other}?)"),
        }
    }
    let _ = src.ctl.send(SourceCtl::Stop);
    let _ = eng_tx.send(EngineCtl::Stop);
    let _ = eng.join();
    let _ = src.thread.join(); // dongle released (bias-tee off) before we exit
}
