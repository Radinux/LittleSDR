//! Prints every MSG of a KiwiSDR sound connection (reference client's sequence).
use littlesdr::source::ws::{Message, WebSocket};
use std::time::{Duration, Instant};
fn main() {
    let site = std::env::args().nth(1).unwrap_or("sk6ag1.ddns.net:8071".into());
    let (h, p) = site.rsplit_once(':').unwrap();
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() & 0xffff_ffff;
    let mut ws = WebSocket::connect(h, p.parse().unwrap(), &format!("/kiwi/{ts}/SND"), Duration::from_secs(8)).unwrap();
    ws.send_text("SET auth t=kiwi p=#").unwrap();
    let t0 = Instant::now();
    let mut last_ka = 0u64;
    let mut snd = 0;
    loop {
        match ws.recv() {
            Ok(Message::Binary(b)) if b.starts_with(b"MSG") => {
                let t = String::from_utf8_lossy(&b[4..]).chars().take(160).collect::<String>();
                if !t.starts_with("load_") && !t.starts_with("extint") {
                    println!("{:5.1}s MSG {t}", t0.elapsed().as_secs_f64());
                }
                for kv in t.split(' ') {
                    if let Some(v) = kv.strip_prefix("audio_rate=") {
                        ws.send_text(&format!("SET AR OK in={v} out=44100")).unwrap();
                    }
                    if kv.starts_with("sample_rate=") {
                        for c in ["SET squelch=0 max=0", "SET genattn=0", "SET gen=0 mix=-1", "SET ident_user=LittleSDR", "SET mod=iq low_cut=-6000 high_cut=6000 freq=6030.000", "SET agc=1 hang=0 thresh=-100 slope=6 decay=1000 manGain=50", "SET keepalive"] {
                            ws.send_text(c).unwrap();
                        }
                    }
                }
            }
            Ok(Message::Binary(b)) => {
                snd += 1;
                let s = t0.elapsed().as_secs();
                if b.starts_with(b"SND") && s != last_ka {
                    ws.send_text("SET keepalive").unwrap();
                    last_ka = s;
                }
            }
            Ok(Message::Text(t)) => println!("TEXT {t}"),
            Err(e) => {
                println!("{:5.1}s closed after {snd} packets: {e}", t0.elapsed().as_secs_f64());
                return;
            }
        }
        if t0.elapsed() > Duration::from_secs(20) {
            println!("still connected after 20 s, {snd} packets");
            return;
        }
    }
}
