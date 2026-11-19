//! 77-bit FT8 / FT4 messages (WSJT-X 2.x "77-bit" protocol): unpacking of the common types,
//! packing of standard and free-text messages (tests, synthetic signals), and the table of
//! hashed callsigns that lets "<...>" be replaced by the call once it has been heard.

use std::collections::HashMap;

const NTOKENS: u32 = 2_063_592;
const MAX22: u32 = 4_194_304;
const MAXGRID4: u32 = 32_400;

const A0: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const A1: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const A2: &[u8] = b"0123456789";
const A3: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const TEXT: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+-./?";
const C38: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ/";

fn bits_to_u64(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, &x| (a << 1) | x as u64)
}

fn bits_to_u128(b: &[u8]) -> u128 {
    b.iter().fold(0u128, |a, &x| (a << 1) | x as u128)
}

fn push_bits(out: &mut Vec<u8>, v: u128, n: usize) {
    for i in (0..n).rev() {
        out.push((v >> i) as u8 & 1);
    }
}

/// WSJT-X callsign hash on m bits (10, 12 or 22)
pub fn hash_call(call: &str, m: u32) -> u32 {
    let c: Vec<u8> = call.trim().bytes().map(|b| b.to_ascii_uppercase()).collect();
    let mut n8: u64 = 0;
    for i in 0..11 {
        let ch = c.get(i).copied().unwrap_or(b' ');
        let j = C38.iter().position(|&x| x == ch).unwrap_or(0) as u64;
        n8 = 38 * n8 + j;
    }
    (47_055_833_459u64.wrapping_mul(n8) >> (64 - m)) as u32
}

/// Calls heard recently, by hash
#[derive(Default)]
pub struct CallHashes {
    h10: HashMap<u32, String>,
    h12: HashMap<u32, String>,
    h22: HashMap<u32, String>,
}

impl CallHashes {
    pub fn add(&mut self, call: &str) {
        let call = call.trim();
        if call.len() < 3 || call.starts_with('<') || call.starts_with("CQ") || call == "QRZ" || call == "DE" {
            return;
        }
        self.h10.insert(hash_call(call, 10), call.to_string());
        self.h12.insert(hash_call(call, 12), call.to_string());
        self.h22.insert(hash_call(call, 22), call.to_string());
        if self.h22.len() > 5000 {
            self.h10.clear();
            self.h12.clear();
            self.h22.clear();
        }
    }

    fn lookup(&self, h: u32, m: u32) -> String {
        let t = match m {
            10 => &self.h10,
            12 => &self.h12,
            _ => &self.h22,
        };
        t.get(&h).map_or("<...>".to_string(), |c| format!("<{c}>"))
    }
}

/// A decoded message, with what the receiver lists need
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub text: String,
    /// Station that sent it (second call of a standard message, the CQ caller)
    pub from: String,
    /// Locator in the message, if any
    pub grid: String,
}

fn unpack28(n28: u32, hashes: &CallHashes) -> String {
    if n28 < NTOKENS {
        return match n28 {
            0 => "DE".into(),
            1 => "QRZ".into(),
            2 => "CQ".into(),
            3..=1002 => format!("CQ {:03}", n28 - 3),
            1003..=532_443 => {
                let mut n = n28 - 1003;
                let mut s = [b' '; 4];
                for k in (0..4).rev() {
                    s[k] = A3[(n % 27) as usize];
                    n /= 27;
                }
                format!("CQ {}", String::from_utf8_lossy(&s).trim())
            }
            _ => "<?>".into(),
        };
    }
    let n = n28 - NTOKENS;
    if n < MAX22 {
        return hashes.lookup(n, 22);
    }
    let mut n = n - MAX22;
    let mut c = [b' '; 6];
    c[5] = A3[(n % 27) as usize];
    n /= 27;
    c[4] = A3[(n % 27) as usize];
    n /= 27;
    c[3] = A3[(n % 27) as usize];
    n /= 27;
    c[2] = A2[(n % 10) as usize];
    n /= 10;
    c[1] = A1[(n % 36) as usize];
    n /= 36;
    c[0] = A0[(n % 37) as usize];
    let s = String::from_utf8_lossy(&c).trim().to_string();
    // Calls that don't fit the pattern are sent with a substitution
    if let Some(rest) = s.strip_prefix("3D0") {
        format!("3DA0{rest}")
    } else if s.len() >= 2 && s.starts_with('Q') && s.as_bytes()[1].is_ascii_alphabetic() {
        format!("3X{}", &s[1..])
    } else {
        s
    }
}

fn grid4(g: u32) -> String {
    let (a, r) = (g / 1800, g % 1800);
    let (b, r) = (r / 100, r % 100);
    format!("{}{}{}{}", (b'A' + a as u8) as char, (b'A' + b as u8) as char, r / 10, r % 10)
}

/// Unpacks 77 bits. `None` for types this receiver does not handle.
pub fn unpack(b: &[u8], hashes: &mut CallHashes) -> Option<Message> {
    let i3 = bits_to_u64(&b[74..77]) as u32;
    let n3 = bits_to_u64(&b[71..74]) as u32;
    match (i3, n3) {
        (0, 0) => {
            // Free text: 13 characters in base 42
            let mut n = bits_to_u128(&b[..71]);
            let mut s = [b' '; 13];
            for k in (0..13).rev() {
                s[k] = TEXT[(n % 42) as usize];
                n /= 42;
            }
            let text = String::from_utf8_lossy(&s).trim().to_string();
            Some(Message { text, ..Default::default() })
        }
        (0, 1) => {
            // DXpedition: "K1ABC RR73; W9XYZ <KH1/KH7Z> -08"
            let c1 = unpack28(bits_to_u64(&b[0..28]) as u32, hashes);
            let c2 = unpack28(bits_to_u64(&b[28..56]) as u32, hashes);
            let h10 = bits_to_u64(&b[56..66]) as u32;
            let r = bits_to_u64(&b[66..71]) as i32 * 2 - 30;
            let dx = hashes.lookup(h10, 10);
            let text = format!("{c1} RR73; {c2} {dx} {r:+03}");
            Some(Message { from: dx.trim_matches(|c| c == '<' || c == '>').to_string(), text, grid: String::new() })
        }
        (0, 5) => {
            // Telemetry: 71 bits as hex
            let n = bits_to_u128(&b[..71]);
            let s = format!("{n:018X}");
            Some(Message { text: s.trim_start_matches('0').to_string(), ..Default::default() })
        }
        (1 | 2, _) => {
            let c1 = unpack28(bits_to_u64(&b[0..28]) as u32, hashes);
            let p1 = b[28] == 1;
            let c2 = unpack28(bits_to_u64(&b[29..57]) as u32, hashes);
            let p2 = b[57] == 1;
            let ir = b[58] == 1;
            let g = bits_to_u64(&b[59..74]) as u32;
            let suffix = if i3 == 1 { "/R" } else { "/P" };
            let call1 = if p1 { format!("{c1}{suffix}") } else { c1.clone() };
            let call2 = if p2 { format!("{c2}{suffix}") } else { c2.clone() };
            let (extra, grid) = if g <= MAXGRID4 {
                let gr = grid4(g);
                (if ir { format!("R {gr}") } else { gr.clone() }, gr)
            } else {
                let rp = g - MAXGRID4;
                let e = match rp {
                    1 => String::new(),
                    2 => "RRR".into(),
                    3 => "RR73".into(),
                    4 => "73".into(),
                    _ => {
                        let snr = rp as i32 - 35;
                        if ir {
                            format!("R{snr:+03}")
                        } else {
                            format!("{snr:+03}")
                        }
                    }
                };
                (e, String::new())
            };
            let mut text = format!("{call1} {call2}");
            if !extra.is_empty() {
                text = format!("{text} {extra}");
            }
            for c in [&c1, &c2] {
                if !c.starts_with('<') {
                    hashes.add(c);
                }
            }
            let from = c2.clone(); // the sender is the second call (also after CQ / QRZ / DE)
            Some(Message { text, from: from.trim_matches(|c| c == '<' || c == '>').to_string(), grid })
        }
        (4, _) => {
            // One non-standard call (up to 11 characters) and one hashed call
            let h12 = bits_to_u64(&b[0..12]) as u32;
            let mut n = bits_to_u64(&b[12..70]);
            let mut s = [b' '; 11];
            for k in (0..11).rev() {
                s[k] = C38[(n % 38) as usize];
                n /= 38;
            }
            let c11 = String::from_utf8_lossy(&s).trim().to_string();
            let flip = b[70] == 1;
            let rpt = bits_to_u64(&b[71..73]);
            let cq = b[73] == 1;
            hashes.add(&c11);
            let hashed = hashes.lookup(h12, 12);
            if cq {
                return Some(Message { text: format!("CQ {c11}"), from: c11, grid: String::new() });
            }
            let (a, z) = if flip { (c11.clone(), hashed.clone()) } else { (hashed.clone(), c11.clone()) };
            let tail = ["", " RRR", " RR73", " 73"][rpt as usize];
            Some(Message { text: format!("{a} {z}{tail}"), from: z.trim_matches(|c| c == '<' || c == '>').to_string(), grid: String::new() })
        }
        _ => None,
    }
}

fn is_grid4(g: &str) -> bool {
    let b = g.as_bytes();
    b.len() == 4 && (b'A'..=b'R').contains(&b[0]) && (b'A'..=b'R').contains(&b[1]) && b[2].is_ascii_digit() && b[3].is_ascii_digit()
}

fn pack28(call: &str) -> Option<u32> {
    let c = call.trim().to_ascii_uppercase();
    match c.as_str() {
        "DE" => return Some(0),
        "QRZ" => return Some(1),
        "CQ" => return Some(2),
        _ => {}
    }
    if let Some(m) = c.strip_prefix("CQ ") {
        if m.len() == 3 && m.bytes().all(|b| b.is_ascii_digit()) {
            return Some(3 + m.parse::<u32>().ok()?);
        }
        if (1..=4).contains(&m.len()) && m.bytes().all(|b| b.is_ascii_alphabetic()) {
            let mut n = 0u32;
            let padded = format!("{m:>4}");
            for ch in padded.bytes() {
                n = 27 * n + A3.iter().position(|&x| x == ch)? as u32;
            }
            return Some(1003 + n);
        }
        return None;
    }
    // Standard call: the digit lands in position 2 ("K1ABC" -> " K1ABC")
    let b = c.as_bytes();
    let c6: Vec<u8> = if b.len() >= 2 && b[1].is_ascii_digit() && b.len() <= 5 {
        std::iter::once(b' ').chain(b.iter().copied()).collect()
    } else {
        b.to_vec()
    };
    if c6.len() > 6 || c6.len() < 3 || !c6[2].is_ascii_digit() {
        return None;
    }
    let mut c6 = c6;
    c6.resize(6, b' ');
    let idx = |set: &[u8], ch: u8| set.iter().position(|&x| x == ch).map(|v| v as u32);
    let mut n = idx(A0, c6[0])?;
    n = n * 36 + idx(A1, c6[1])?;
    n = n * 10 + idx(A2, c6[2])?;
    n = n * 27 + idx(A3, c6[3])?;
    n = n * 27 + idx(A3, c6[4])?;
    n = n * 27 + idx(A3, c6[5])?;
    Some(NTOKENS + MAX22 + n)
}

/// Packs a standard message ("CQ F4ABC JN05", "F4ABC DL1XX -12", "DL1XX F4ABC R-05",
/// "F4ABC DL1XX RR73") or free text (up to 13 characters). 77 bits.
pub fn pack(msg: &str) -> Option<Vec<u8>> {
    let msg = msg.trim().to_ascii_uppercase();
    let w: Vec<&str> = msg.split_whitespace().collect();
    let std = (|| {
        let (c1, rest) = if w.len() >= 2 && w[0] == "CQ" && w.len() >= 3 && pack28(&format!("CQ {}", w[1])).is_some() && pack28(w[2]).is_some() {
            (format!("CQ {}", w[1]), &w[2..])
        } else {
            (w.first()?.to_string(), &w[1..])
        };
        let n1 = pack28(&c1)?;
        let n2 = pack28(rest.first()?)?;
        let extra: Vec<&str> = rest[1..].to_vec();
        let (ir, g) = match extra.as_slice() {
            [] => (0, MAXGRID4 + 1),
            ["RRR"] => (0, MAXGRID4 + 2),
            ["RR73"] => (0, MAXGRID4 + 3),
            ["73"] => (0, MAXGRID4 + 4),
            ["R", gr] | [gr] if is_grid4(gr) && *gr != "RR73" => {
                let gb = gr.as_bytes();
                let v = (gb[0] - b'A') as u32 * 1800 + (gb[1] - b'A') as u32 * 100 + (gb[2] - b'0') as u32 * 10 + (gb[3] - b'0') as u32;
                ((extra.len() == 2) as u32, v)
            }
            [r] => {
                let (ir, num) = match r.strip_prefix('R') {
                    Some(n) => (1, n),
                    None => (0, *r),
                };
                let v: i32 = num.parse().ok()?;
                (ir, MAXGRID4 + (v.clamp(-30, 32) + 35) as u32)
            }
            _ => return None,
        };
        let mut b = Vec::with_capacity(77);
        push_bits(&mut b, n1 as u128, 28);
        b.push(0);
        push_bits(&mut b, n2 as u128, 28);
        b.push(0);
        b.push(ir as u8);
        push_bits(&mut b, g as u128, 15);
        push_bits(&mut b, 1, 3);
        Some(b)
    })();
    if std.is_some() {
        return std;
    }
    if msg.len() > 13 {
        return None;
    }
    let mut n: u128 = 0;
    for ch in format!("{msg:>13}").bytes() {
        n = 42 * n + TEXT.iter().position(|&x| x == ch)? as u128;
    }
    let mut b = Vec::with_capacity(77);
    push_bits(&mut b, n, 71);
    push_bits(&mut b, 0, 6);
    Some(b)
}

/// Maidenhead locator (4 or 6 characters) to latitude / longitude (degrees, centre)
pub fn grid_latlon(g: &str) -> Option<(f64, f64)> {
    let b = g.as_bytes();
    if b.len() < 4 || !(b'A'..=b'R').contains(&b[0].to_ascii_uppercase()) || !(b'A'..=b'R').contains(&b[1].to_ascii_uppercase()) || !b[2].is_ascii_digit() || !b[3].is_ascii_digit() {
        return None;
    }
    let mut lon = (b[0].to_ascii_uppercase() - b'A') as f64 * 20.0 + (b[2] - b'0') as f64 * 2.0 - 180.0;
    let mut lat = (b[1].to_ascii_uppercase() - b'A') as f64 * 10.0 + (b[3] - b'0') as f64 - 90.0;
    if b.len() >= 6 && b[4].is_ascii_alphabetic() && b[5].is_ascii_alphabetic() {
        lon += ((b[4].to_ascii_uppercase() - b'A') as f64 + 0.5) / 12.0;
        lat += ((b[5].to_ascii_uppercase() - b'A') as f64 + 0.5) / 24.0;
    } else {
        lon += 1.0;
        lat += 0.5;
    }
    Some((lat, lon))
}

/// Great-circle distance (km)
pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut h = CallHashes::default();
        for m in ["CQ F4ABC JN05", "CQ DX K1ABC FN42", "F4ABC DL1XX -12", "DL1XX F4ABC R-05", "F4ABC DL1XX RR73", "DL1XX F4ABC 73", "F4ABC YO3RAD RRR", "YO3RAD G4XYZ R JO01", "TNX BOB 73 GL"] {
            let b = pack(m).unwrap_or_else(|| panic!("cannot pack {m}"));
            assert_eq!(b.len(), 77);
            let u = unpack(&b, &mut h).unwrap();
            assert_eq!(u.text, m);
        }
        let u = unpack(&pack("CQ F4ABC JN05").unwrap(), &mut h).unwrap();
        assert_eq!((u.from.as_str(), u.grid.as_str()), ("F4ABC", "JN05"));
    }

    #[test]
    fn hashed_calls_are_filled_in() {
        let mut h = CallHashes::default();
        h.add("PJ4/K1ABC");
        assert_eq!(h.lookup(hash_call("PJ4/K1ABC", 12), 12), "<PJ4/K1ABC>");
        assert_eq!(h.lookup(hash_call("W9XYZ", 12), 12), "<...>");
    }

    #[test]
    fn locator_distance() {
        let paris = grid_latlon("JN18").unwrap();
        let bucharest = grid_latlon("KN34").unwrap();
        let d = distance_km(paris, bucharest);
        assert!((1700.0..2000.0).contains(&d), "{d}");
    }
}
