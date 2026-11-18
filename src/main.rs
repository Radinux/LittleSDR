//! Command-line LittleSDR: listen with the engine, change settings live.
//!
//!   littlesdr [--usb [N] | --rtltcp HOST:PORT] [--freq 96.5M] [--rate 2.4M] [--mode wfm|nfm]
//!             [--gain DB|auto] [--wav out.wav] [--no-audio] [--script "3s rate 1.024M; 6s quit"]
//!
//! Live commands on stdin: freq <f>, rate <r>, mode wfm|nfm, gain <dB>|auto, vol <0-2>,
//!   stereo on|off, nb on|off [threshold], fx <filter> on|off, fx <filter> <param> <value>,
//!   status, quit. Filters: hp lp cw notch anotch nr eq agc limit

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

/// Hardware center kept 250 kHz below the tuned frequency: the RTL-SDR's DC spike stays out
/// of the channel. Returns (center, VFO offset).
fn plan(freq: f64, rate: f64) -> (u64, f64) {
    let offset = if rate >= 1e6 { 250e3 } else { 0.0 };
    ((freq - offset).round() as u64, offset)
}

enum Cmd {
    Line(String),
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut usb: Option<u32> = Some(0);
    let mut tcp: Option<String> = None;
    let mut freq = 96.5e6;
    let mut rate = 2.4e6;
    let mut mode = Mode::Wfm;
    let mut gain: Option<f32> = None;
    let mut wav: Option<String> = None;
    let mut audio = cfg!(feature = "audio");
    let mut script = String::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--usb" => usb = Some(args.next().and_then(|v| v.parse().ok()).unwrap_or(0)),
            "--rtltcp" => {
                tcp = args.next();
                usb = None;
            }
            "--freq" => freq = args.next().and_then(|v| parse_hz(&v)).expect("--freq"),
            "--rate" => rate = args.next().and_then(|v| parse_hz(&v)).expect("--rate"),
            "--mode" => mode = args.next().and_then(|v| Mode::parse(&v)).expect("--mode wfm|nfm"),
            "--gain" => gain = args.next().and_then(|v| v.parse().ok()),
            "--wav" => wav = args.next(),
            "--no-audio" => audio = false,
            "--script" => script = args.next().unwrap_or_default(),
            "-h" | "--help" => {
                println!("{}", include_str!("main.rs").lines().take(9).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
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
    let (center, offset) = plan(freq, rate);
    let (iq_tx, iq_rx) = mpsc::sync_channel(64);
    let src = source::spawn(src, SourceConfig { rate: rate as u32, center, gain }, iq_tx);

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
        .spawn(move || engine::run(iq_rx, eng_rx, sink, Engine::new(mode, offset), true))
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
    drop(cmd_tx);

    let mut cur_freq = freq;
    let mut cur_rate = rate;
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
            "status" | "s" => {
                let _ = eng_tx.send(EngineCtl::Status);
            }
            "freq" | "f" => {
                if let Some(f) = parse_hz(v) {
                    cur_freq = f;
                    let (center, offset) = plan(cur_freq, cur_rate);
                    let _ = src.ctl.send(SourceCtl::Center(center));
                    let _ = eng_tx.send(EngineCtl::Offset(offset));
                    eprintln!("[littlesdr] {:.3} MHz", f / 1e6);
                }
            }
            "rate" | "r" => {
                if let Some(r) = parse_hz(v) {
                    cur_rate = r;
                    let (center, offset) = plan(cur_freq, cur_rate);
                    let _ = eng_tx.send(EngineCtl::Offset(offset));
                    let _ = src.ctl.send(SourceCtl::Center(center));
                    let _ = src.ctl.send(SourceCtl::Rate(r as u32));
                    eprintln!("[littlesdr] rate -> {:.0} S/s", r);
                }
            }
            "mode" | "m" => {
                if let Some(m) = Mode::parse(v) {
                    let _ = eng_tx.send(EngineCtl::Mode(m));
                }
            }
            "gain" | "g" => {
                let _ = src.ctl.send(SourceCtl::Gain(v.parse().ok()));
            }
            "vol" => {
                if let Ok(x) = v.parse() {
                    let _ = eng_tx.send(EngineCtl::Volume(x));
                }
            }
            "quit" | "q" | "exit" => break,
            "" => {}
            other => eprintln!("commands: freq | rate | mode | gain | vol | stereo | nb | fx | status | quit  ({other}?)"),
        }
    }
    let _ = src.ctl.send(SourceCtl::Stop);
    let _ = eng_tx.send(EngineCtl::Stop);
    let _ = eng.join();
}
