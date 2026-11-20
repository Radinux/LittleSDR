//! Minimal WebSocket client (RFC 6455) over plain TCP: enough for KiwiSDR. No TLS.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct WebSocket {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    seed: u32,
}

pub enum Message {
    Text(String),
    Binary(Vec<u8>),
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

impl WebSocket {
    /// Connects to ws://host:port/path
    pub fn connect(host: &str, port: u16, path: &str, timeout: Duration) -> Result<WebSocket, String> {
        use std::net::ToSocketAddrs;
        let addr = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.next().ok_or(format!("{host}: no address"))?;
        let s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{host}:{port}: {e}"))?;
        s.set_nodelay(true).ok();
        s.set_read_timeout(Some(timeout)).ok();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() | 1);
        let mut ws = WebSocket { reader: BufReader::new(s.try_clone().map_err(|e| e.to_string())?), writer: s, seed };
        let key: Vec<u8> = (0..16).map(|_| ws.rand() as u8).collect();
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: LittleSDR\r\n\r\n",
            base64(&key)
        );
        ws.writer.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut status = String::new();
        ws.reader.read_line(&mut status).map_err(|e| e.to_string())?;
        if !status.contains(" 101") {
            return Err(format!("server refused the connection: {}", status.trim()));
        }
        loop {
            let mut l = String::new();
            ws.reader.read_line(&mut l).map_err(|e| e.to_string())?;
            if l == "\r\n" || l.is_empty() {
                break;
            }
        }
        Ok(ws)
    }

    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    fn send_frame(&mut self, opcode: u8, data: &[u8]) -> Result<(), String> {
        let mut f = vec![0x80 | opcode];
        match data.len() {
            n if n < 126 => f.push(0x80 | n as u8),
            n if n < 65536 => {
                f.push(0x80 | 126);
                f.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                f.push(0x80 | 127);
                f.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        let mask = self.rand().to_be_bytes();
        f.extend_from_slice(&mask);
        f.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.writer.write_all(&f).map_err(|e| e.to_string())
    }

    pub fn send_text(&mut self, s: &str) -> Result<(), String> {
        self.send_frame(1, s.as_bytes())
    }

    /// Next message (pings answered on the way). Err on close or network error.
    pub fn recv(&mut self) -> Result<Message, String> {
        let mut payload = Vec::new();
        let mut kind = 0u8;
        loop {
            let mut h = [0u8; 2];
            self.reader.read_exact(&mut h).map_err(|e| e.to_string())?;
            let (fin, op) = (h[0] & 0x80 != 0, h[0] & 0x0F);
            let masked = h[1] & 0x80 != 0;
            let mut len = (h[1] & 0x7F) as u64;
            if len == 126 {
                let mut b = [0u8; 2];
                self.reader.read_exact(&mut b).map_err(|e| e.to_string())?;
                len = u16::from_be_bytes(b) as u64;
            } else if len == 127 {
                let mut b = [0u8; 8];
                self.reader.read_exact(&mut b).map_err(|e| e.to_string())?;
                len = u64::from_be_bytes(b);
            }
            if len > 16 << 20 {
                return Err("frame too large".into());
            }
            let mut mask = [0u8; 4];
            if masked {
                self.reader.read_exact(&mut mask).map_err(|e| e.to_string())?;
            }
            let mut data = vec![0u8; len as usize];
            self.reader.read_exact(&mut data).map_err(|e| e.to_string())?;
            if masked {
                data.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
            }
            match op {
                8 => {
                    let _ = self.send_frame(8, &[]);
                    let why = if data.len() >= 2 {
                        format!(" (code {}{})", u16::from_be_bytes([data[0], data[1]]), String::from_utf8_lossy(&data[2..]).trim().chars().take(80).map(|c| c.to_string()).collect::<String>().split_whitespace().next().map_or(String::new(), |_| format!(": {}", String::from_utf8_lossy(&data[2..]).trim())))
                    } else {
                        String::new()
                    };
                    return Err(format!("connection closed by the server{why}"));
                }
                9 => {
                    self.send_frame(10, &data)?;
                    continue;
                }
                10 => continue,
                0 => payload.extend(data),
                o => {
                    kind = o;
                    payload = data;
                }
            }
            if fin {
                return Ok(if kind == 1 { Message::Text(String::from_utf8_lossy(&payload).into_owned()) } else { Message::Binary(payload) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc() {
        assert_eq!(super::base64(b"the sample nonce"), "dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(super::base64(b"foob"), "Zm9vYg==");
    }
}
