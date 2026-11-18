//! rtl_tcp client (network RTL-SDR, or tools/fake_rtl_tcp.py for tests).
//!
//! The server changes the rate while streaming, but data already in the network buffers is at
//! the old rate and nothing marks the switch: the first block after a change asks the engine
//! to mute for the in-flight time.

use super::{u8_to_iq, IqBlock, Source, SourceConfig, SourceCtl};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::time::Duration;

pub struct RtlTcp {
    pub addr: String,
}

fn cmd(s: &mut TcpStream, code: u8, value: u32) -> std::io::Result<()> {
    let mut b = [0u8; 5];
    b[0] = code;
    b[1..].copy_from_slice(&value.to_be_bytes());
    s.write_all(&b)
}

fn gain(s: &mut TcpStream, g: Option<f32>) -> std::io::Result<()> {
    match g {
        None => cmd(s, 0x03, 0),
        Some(db) => {
            cmd(s, 0x03, 1)?;
            cmd(s, 0x04, (db * 10.0) as u32)
        }
    }
}

impl Source for RtlTcp {
    fn name(&self) -> String {
        format!("rtl_tcp {}", self.addr)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let mut s = TcpStream::connect(&self.addr).map_err(|e| format!("connect: {e}"))?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok();
        s.set_nodelay(true).ok();
        let mut header = [0u8; 12];
        s.read_exact(&mut header).map_err(|e| format!("header: {e}"))?;
        if &header[..4] != b"RTL0" {
            return Err("not an rtl_tcp server".into());
        }
        let (mut rate, mut center) = (cfg.rate, cfg.center);
        let io = |e: std::io::Error| e.to_string();
        cmd(&mut s, 0x02, rate).map_err(io)?;
        cmd(&mut s, 0x01, center as u32).map_err(io)?;
        gain(&mut s, cfg.gain).map_err(io)?;
        if cfg.direct_sampling > 0 {
            cmd(&mut s, 0x09, cfg.direct_sampling as u32).map_err(io)?;
        }

        let mut buf = vec![0u8; 64 * 1024];
        let mut odd: Option<u8> = None; // a sample split across two reads
        let mut settle_ms = 0.0;
        loop {
            loop {
                match ctl.try_recv() {
                    Ok(SourceCtl::Rate(r)) if r != rate => {
                        rate = r;
                        cmd(&mut s, 0x02, rate).map_err(io)?;
                        settle_ms = 80.0; // server + kernel buffers, measured on localhost
                    }
                    Ok(SourceCtl::Rate(_)) => {}
                    Ok(SourceCtl::Center(f)) => {
                        center = f;
                        cmd(&mut s, 0x01, center as u32).map_err(io)?;
                    }
                    Ok(SourceCtl::Gain(g)) => gain(&mut s, g).map_err(io)?,
                    Ok(SourceCtl::DirectSampling(m)) => {
                        cmd(&mut s, 0x09, m as u32).map_err(io)?;
                        cmd(&mut s, 0x01, center as u32).map_err(io)?;
                    }
                    Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Err(TryRecvError::Empty) => break,
                }
            }
            let n = s.read(&mut buf).map_err(|e| format!("read: {e}"))?;
            if n == 0 {
                return Err("server closed the connection".into());
            }
            let mut bytes = Vec::with_capacity(n + 1);
            if let Some(b) = odd.take() {
                bytes.push(b);
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() % 2 == 1 {
                odd = bytes.pop();
            }
            let mut samples = Vec::with_capacity(bytes.len() / 2);
            u8_to_iq(&bytes, &mut samples);
            let block = IqBlock { samples, rate: rate as f64, center: center as f64, settle_ms };
            settle_ms = 0.0;
            if out.send(block).is_err() {
                return Ok(());
            }
        }
    }
}
