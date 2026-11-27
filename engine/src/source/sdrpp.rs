//! SDR++ server (`sdrpp --server`, default port 5259): the receiver of a remote SDR++, whatever
//! it is (RX888, LimeSDR, bladeRF, USRP...), streamed over TCP. Works with SDR++ and SDR++Brown
//! (core/src/server_protocol.h).
//!
//! Packets: a header (type u32, total size u32, little endian), then for commands a command
//! u32 and its arguments. Baseband packets carry 2 bytes of compression type, 2 of sample type
//! (0 int8, 1 int16, 2 float32), a float scale, then the I/Q pairs. The server's own UI (its
//! receiver, gain, sample rate) is not drawn here: those are set on the server.

use super::{IqBlock, Source, SourceConfig, SourceCtl};
use crate::dsp::C32;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 5259;

const PACKET_COMMAND: u32 = 0;
const PACKET_COMMAND_ACK: u32 = 1;
const PACKET_BASEBAND: u32 = 2;
const PACKET_ERROR: u32 = 6;

const CMD_GET_UI: u32 = 0x00;
const CMD_START: u32 = 0x02;
const CMD_STOP: u32 = 0x03;
const CMD_SET_FREQUENCY: u32 = 0x04;
const CMD_GET_SAMPLERATE: u32 = 0x05;
const CMD_SET_SAMPLE_TYPE: u32 = 0x06;
const CMD_SET_COMPRESSION: u32 = 0x07;
/// SDR++Brown: its own lossy FFT compression, on by default (SDR++ answers "invalid command")
const CMD_SET_FFTZSTD_COMPRESSION: u32 = 0x38;
const CMD_SET_SAMPLERATE: u32 = 0x80;
/// SDR++: 0x81; SDR++Brown: 0xA5
const CMD_DISCONNECT: [u32; 2] = [0x81, 0xA5];
/// SDR++Brown with a password
const CMD_SECURE_CHALLENGE: u32 = 0xA4;

/// Sample types the server can send, lightest first
pub const SAMPLE_TYPES: [&str; 3] = ["8 bits (le plus léger)", "16 bits", "32 bits flottants"];

#[derive(Default, Clone)]
pub struct SdrppStatus {
    pub message: String,
    /// The server's sample rate, once it told it
    pub rate: f64,
}

pub struct SdrppServer {
    pub addr: String,
    /// 0 int8, 1 int16, 2 float32
    pub sample_type: u8,
    pub status: Arc<Mutex<SdrppStatus>>,
}

impl SdrppServer {
    pub fn new(addr: &str, sample_type: u8) -> SdrppServer {
        SdrppServer { addr: addr.trim().to_string(), sample_type: sample_type.min(2), status: Arc::default() }
    }

    fn say(&self, m: impl Into<String>) {
        self.status.lock().unwrap().message = m.into();
    }
}

fn command(cmd: u32, args: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + args.len());
    p.extend_from_slice(&PACKET_COMMAND.to_le_bytes());
    p.extend_from_slice(&((12 + args.len()) as u32).to_le_bytes());
    p.extend_from_slice(&cmd.to_le_bytes());
    p.extend_from_slice(args);
    p
}

/// The I/Q pairs of a baseband payload
pub fn decode_baseband(p: &[u8]) -> Option<Vec<C32>> {
    if p.len() < 8 {
        return None;
    }
    let kind = u16::from_le_bytes([p[2], p[3]]);
    let scale = f32::from_le_bytes([p[4], p[5], p[6], p[7]]);
    let d = &p[8..];
    Some(match kind {
        0 => {
            let k = scale / 128.0;
            d.as_chunks::<2>().0.iter().map(|c| C32::new(c[0] as i8 as f32 * k, c[1] as i8 as f32 * k)).collect()
        }
        1 => {
            let k = scale / 32768.0;
            d.as_chunks::<4>()
                .0
                .iter()
                .map(|c| C32::new(i16::from_le_bytes([c[0], c[1]]) as f32 * k, i16::from_le_bytes([c[2], c[3]]) as f32 * k))
                .collect()
        }
        2 => d
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| C32::new(f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])))
            .collect(),
        _ => return None,
    })
}

impl Source for SdrppServer {
    fn name(&self) -> String {
        format!("SDR++ {}", self.addr)
    }

    fn run(self: Box<Self>, cfg: SourceConfig, out: SyncSender<IqBlock>, ctl: Receiver<SourceCtl>) -> Result<(), String> {
        let addr = if self.addr.contains(':') { self.addr.clone() } else { format!("{}:{DEFAULT_PORT}", self.addr) };
        self.say(format!("Connexion à {addr}…"));
        let sa = addr.to_socket_addrs().map_err(|e| format!("adresse {addr} : {e}"))?.next().ok_or(format!("adresse {addr} introuvable"))?;
        let mut sock = TcpStream::connect_timeout(&sa, Duration::from_secs(5)).map_err(|e| format!("serveur SDR++ {addr} injoignable ({e})"))?;
        let _ = sock.set_nodelay(true);
        sock.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e| e.to_string())?;
        let send = |s: &mut TcpStream, p: Vec<u8>| s.write_all(&p).map_err(|e| format!("serveur SDR++ : {e}"));

        let mut center = cfg.center as f64;
        send(&mut sock, command(CMD_GET_UI, &[]))?;
        send(&mut sock, command(CMD_SET_SAMPLE_TYPE, &[self.sample_type]))?;
        send(&mut sock, command(CMD_SET_COMPRESSION, &[0]))?;
        send(&mut sock, command(CMD_SET_FFTZSTD_COMPRESSION, &[0]))?;
        send(&mut sock, command(CMD_SET_FREQUENCY, &center.to_le_bytes()))?;
        send(&mut sock, command(CMD_GET_SAMPLERATE, &[]))?;
        send(&mut sock, command(CMD_START, &[]))?;
        self.say(format!("Connecté à {addr}"));

        let mut rate = 0.0f64;
        let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
        let mut tmp = vec![0u8; 1 << 16];
        let mut idle = 0u32;
        loop {
            match ctl.try_recv() {
                Ok(SourceCtl::Center(f)) => {
                    center = f as f64;
                    send(&mut sock, command(CMD_SET_FREQUENCY, &center.to_le_bytes()))?;
                }
                Ok(SourceCtl::Stop) | Err(TryRecvError::Disconnected) => {
                    let _ = send(&mut sock, command(CMD_STOP, &[]));
                    return Ok(());
                }
                Ok(_) | Err(TryRecvError::Empty) => {}
            }
            match sock.read(&mut tmp) {
                Ok(0) => return Err("le serveur SDR++ a fermé la connexion".into()),
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    idle = 0;
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    idle += 1;
                    // 10 s without anything: the server's receiver is stopped or gone
                    if idle == 100 {
                        self.say("Le serveur ne renvoie rien : son récepteur est-il démarré ?");
                    }
                    continue;
                }
                Err(e) => return Err(format!("serveur SDR++ : {e}")),
            }
            // Every complete packet received so far
            let mut at = 0;
            while buf.len() - at >= 8 {
                let kind = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
                let size = u32::from_le_bytes(buf[at + 4..at + 8].try_into().unwrap()) as usize;
                if !(8..=64 << 20).contains(&size) {
                    return Err(format!("serveur SDR++ : paquet invalide (taille {size})"));
                }
                if buf.len() - at < size {
                    break;
                }
                let p = &buf[at + 8..at + size];
                at += size;
                match kind {
                    PACKET_COMMAND if p.len() >= 4 => {
                        let cmd = u32::from_le_bytes(p[..4].try_into().unwrap());
                        let a = &p[4..];
                        if cmd == CMD_SET_SAMPLERATE && a.len() >= 8 {
                            rate = f64::from_le_bytes(a[..8].try_into().unwrap());
                            let mut st = self.status.lock().unwrap();
                            st.rate = rate;
                            st.message = format!("Connecté à {addr} · {:.3} MS/s", rate / 1e6);
                        } else if cmd == CMD_SET_FREQUENCY && a.len() >= 8 {
                            // The server's receiver moved by itself (file source...)
                            center = f64::from_le_bytes(a[..8].try_into().unwrap());
                        } else if CMD_DISCONNECT.contains(&cmd) {
                            return Err("le serveur SDR++ est occupé par un autre client".into());
                        } else if cmd == CMD_SECURE_CHALLENGE {
                            return Err("ce serveur SDR++ demande un mot de passe (pas encore pris en charge)".into());
                        }
                    }
                    PACKET_BASEBAND => {
                        if rate <= 0.0 {
                            continue;
                        }
                        if let Some(samples) = decode_baseband(p) {
                            let block = IqBlock { samples, rate, center, settle_ms: 0.0, time: None };
                            if out.send(block).is_err() {
                                return Ok(());
                            }
                        }
                    }
                    PACKET_ERROR => {
                        self.say(format!("Erreur du serveur SDR++ ({})", p.first().copied().unwrap_or(0)));
                    }
                    PACKET_COMMAND_ACK | PACKET_COMMAND => {}
                    // Compressed or other formats are never asked for
                    _ => {}
                }
            }
            buf.drain(..at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three sample types of SDR++'s SampleStreamCompressor, scaled back
    #[test]
    fn baseband_formats() {
        let mut p = vec![0, 0, 0, 0];
        p.extend_from_slice(&0.5f32.to_le_bytes());
        p.extend_from_slice(&[64u8, (-128i8) as u8]);
        let s = decode_baseband(&p).unwrap();
        assert_eq!(s, vec![C32::new(0.25, -0.5)]);

        let mut p = vec![0, 0, 1, 0];
        p.extend_from_slice(&2.0f32.to_le_bytes());
        p.extend_from_slice(&16384i16.to_le_bytes());
        p.extend_from_slice(&(-32768i16).to_le_bytes());
        assert_eq!(decode_baseband(&p).unwrap(), vec![C32::new(1.0, -2.0)]);

        let mut p = vec![0, 0, 2, 0, 0, 0, 0, 0];
        p.extend_from_slice(&0.75f32.to_le_bytes());
        p.extend_from_slice(&(-0.25f32).to_le_bytes());
        assert_eq!(decode_baseband(&p).unwrap(), vec![C32::new(0.75, -0.25)]);
    }

    /// A command packet: header (type 0, total size) then the command and its arguments
    #[test]
    fn command_layout() {
        let p = command(CMD_SET_FREQUENCY, &100e6f64.to_le_bytes());
        assert_eq!(p.len(), 20);
        assert_eq!(&p[..4], &0u32.to_le_bytes());
        assert_eq!(&p[4..8], &20u32.to_le_bytes());
        assert_eq!(&p[8..12], &4u32.to_le_bytes());
        assert_eq!(f64::from_le_bytes(p[12..20].try_into().unwrap()), 100e6);
    }
}
