//! Mode S messages: CRC-24, single-bit repair, field decoding (DF 0/4/5/11/16/17/18/20/21).
//!
//! References: ICAO Annex 10 vol. IV, DO-260B, "The 1090 MHz Riddle" (Junzi Sun), dump1090's
//! Gillham (Mode C) and ID13 tables.

use std::collections::HashMap;
use std::sync::OnceLock;

const POLY: u32 = 0xFFF409;

fn table() -> &'static [u32; 256] {
    static T: OnceLock<[u32; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let mut c = (i as u32) << 16;
            for _ in 0..8 {
                c = if c & 0x80_0000 != 0 { (c << 1) ^ POLY } else { c << 1 };
            }
            *v = c & 0xFF_FFFF;
        }
        t
    })
}

/// Remainder of the whole message: 0 for a clean DF11 (IID 0) / 17 / 18; the address for the
/// address/parity formats (DF 0, 4, 5, 16, 20, 21)
pub fn syndrome(msg: &[u8]) -> u32 {
    let t = table();
    let n = msg.len();
    let mut c = 0u32;
    for &b in &msg[..n - 3] {
        c = ((c << 8) ^ t[(((c >> 16) ^ b as u32) & 0xFF) as usize]) & 0xFF_FFFF;
    }
    c ^ ((msg[n - 3] as u32) << 16 | (msg[n - 2] as u32) << 8 | msg[n - 1] as u32)
}

/// Syndrome of each single-bit error of a 112-bit message (the DF field excluded) -> bit
fn one_bit_112() -> &'static HashMap<u32, usize> {
    static M: OnceLock<HashMap<u32, usize>> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = HashMap::new();
        for bit in 5..112 {
            let mut msg = [0u8; 14];
            msg[bit / 8] = 0x80 >> (bit % 8);
            m.insert(syndrome(&msg), bit);
        }
        m
    })
}

/// Repairs one flipped bit of an extended squitter (DF17/18); true if the message is now clean
pub fn fix_one_bit(msg: &mut [u8; 14]) -> bool {
    let s = syndrome(msg);
    if s == 0 {
        return true;
    }
    match one_bit_112().get(&s) {
        Some(&bit) => {
            msg[bit / 8] ^= 0x80 >> (bit % 8);
            true
        }
        None => false,
    }
}

pub fn len_bits(df: u8) -> usize {
    if df >= 16 {
        112
    } else {
        56
    }
}

/// 13-bit identity / altitude field -> "hex Gillham" layout 0xABCD (A4A2A1 in 0x4000..0x1000,
/// B 0x0400..0x0100, C 0x0040..0x0010, D 0x0004..0x0001)
fn id13_to_gillham(f: u32) -> u32 {
    const MAP: [(u32, u32); 12] = [
        (0x1000, 0x0010), // C1
        (0x0800, 0x1000), // A1
        (0x0400, 0x0020), // C2
        (0x0200, 0x2000), // A2
        (0x0100, 0x0040), // C4
        (0x0080, 0x4000), // A4
        (0x0020, 0x0100), // B1
        (0x0010, 0x0001), // D1 (Q)
        (0x0008, 0x0200), // B2
        (0x0004, 0x0002), // D2
        (0x0002, 0x0400), // B4
        (0x0001, 0x0004), // D4
    ];
    MAP.iter().filter(|(b, _)| f & b != 0).fold(0, |a, (_, g)| a | g)
}

/// Squawk (4 octal digits written as hex: 0x7700 = "7700") from a 13-bit identity field
pub fn squawk(id13: u32) -> u16 {
    id13_to_gillham(id13) as u16
}

/// Gillham (Mode C) code -> altitude in feet
fn gillham_alt(a: u32) -> Option<i32> {
    if a & 0xFFFF_8889 != 0 || a & 0xF0 == 0 {
        return None;
    }
    let mut hundreds = 0u32;
    for (bit, x) in [(0x0010, 7), (0x0020, 3), (0x0040, 1)] {
        if a & bit != 0 {
            hundreds ^= x;
        }
    }
    if hundreds & 5 == 5 {
        hundreds ^= 2;
    }
    if hundreds > 5 {
        return None;
    }
    let mut fives = 0u32;
    for (bit, x) in [(0x0002, 0xFF), (0x0004, 0x7F), (0x1000, 0x3F), (0x2000, 0x1F), (0x4000, 0x0F), (0x0100, 0x07), (0x0200, 0x03), (0x0400, 0x01)] {
        if a & bit != 0 {
            fives ^= x;
        }
    }
    if fives & 1 != 0 {
        hundreds = 6 - hundreds;
    }
    Some((fives as i32 * 5 + hundreds as i32 - 13) * 100)
}

/// AC13 altitude field (DF 0, 4, 16, 20), feet
pub fn ac13_alt(f: u32) -> Option<i32> {
    if f == 0 || f & 0x40 != 0 {
        return None; // unknown, or metric (M bit): not seen in practice
    }
    if f & 0x10 != 0 {
        let n = ((f & 0x1F80) >> 2) | ((f & 0x20) >> 1) | (f & 0x0F);
        return Some(n as i32 * 25 - 1000);
    }
    gillham_alt(id13_to_gillham(f))
}

/// AC12 altitude field (airborne position), feet
pub fn ac12_alt(f: u32) -> Option<i32> {
    if f == 0 {
        return None;
    }
    if f & 0x10 != 0 {
        let n = ((f & 0x0FE0) >> 1) | (f & 0x0F);
        return Some(n as i32 * 25 - 1000);
    }
    // Insert the M bit (0) to make it a 13-bit field
    gillham_alt(id13_to_gillham(((f & 0x0FC0) << 1) | (f & 0x3F)))
}

const CHARSET: &[u8; 64] = b"#ABCDEFGHIJKLMNOPQRSTUVWXYZ##### ###############0123456789######";

/// 8 characters of 6 bits from 6 bytes (identification, BDS 2,0)
pub fn callsign(b: &[u8]) -> Option<String> {
    let v = b[..6].iter().fold(0u64, |a, &x| a << 8 | x as u64);
    let s: String = (0..8).map(|i| CHARSET[((v >> (42 - 6 * i)) & 0x3F) as usize] as char).collect();
    let s = s.trim_end().to_string();
    (!s.is_empty() && !s.contains('#')).then_some(s)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// TC 1-4: callsign, emitter category ("A3"...)
    Ident { callsign: String, category: String },
    /// TC 9-18 / 20-22: CPR position and altitude
    Airborne { alt: Option<i32>, gnss: bool, odd: bool, lat: u32, lon: u32 },
    /// TC 5-8
    Surface { speed: Option<f64>, track: Option<f64>, odd: bool, lat: u32, lon: u32 },
    /// TC 19: ground speed (kt) and track, or airspeed and heading; vertical rate (ft/min)
    Velocity { speed: Option<f64>, track: Option<f64>, heading: Option<f64>, airspeed: bool, vrate: Option<i32> },
    /// Altitude reply (DF 0, 4, 16, 20)
    Altitude(Option<i32>),
    /// Identity reply (DF 5, 21) or emergency status (TC 28)
    Squawk { code: u16, emergency: u8 },
    /// DF11 all-call reply, other ADS-B types
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub df: u8,
    pub icao: u32,
    /// DF18 with a non-ICAO (anonymous / TIS-B) address
    pub non_icao: bool,
    /// Ground / airborne from the capability or flight status field, when given
    pub on_ground: Option<bool>,
    pub content: Vec<Content>,
}

fn u(b: &[u8], from: usize, n: usize) -> u32 {
    // bits [from, from+n) of the message, 0 = MSB of the first byte
    let mut v = 0u32;
    for k in from..from + n {
        v = v << 1 | ((b[k / 8] >> (7 - k % 8)) & 1) as u32;
    }
    v
}

/// Decodes a CRC-checked message. `icao` is the address (from AA, or recovered from the parity).
pub fn decode(msg: &[u8], icao: u32) -> Message {
    let df = msg[0] >> 3;
    let mut m = Message { df, icao, non_icao: false, on_ground: None, content: Vec::new() };
    match df {
        0 | 16 => {
            m.on_ground = Some(u(msg, 5, 1) == 1); // VS
            m.content.push(Content::Altitude(ac13_alt(u(msg, 19, 13))));
        }
        4 | 20 | 5 | 21 => {
            let fs = u(msg, 5, 3);
            m.on_ground = match fs {
                1 | 3 => Some(true),
                0 | 2 => Some(false),
                _ => None,
            };
            let f13 = u(msg, 19, 13);
            if df == 4 || df == 20 {
                m.content.push(Content::Altitude(ac13_alt(f13)));
            } else {
                m.content.push(Content::Squawk { code: squawk(f13), emergency: 0 });
            }
            if df >= 20 && msg[4] == 0x20 {
                // Comm-B, BDS 2,0: aircraft identification
                if let Some(cs) = callsign(&msg[5..11]) {
                    m.content.push(Content::Ident { callsign: cs, category: String::new() });
                }
            }
        }
        11 => {
            let ca = msg[0] & 7;
            m.on_ground = match ca {
                4 => Some(true),
                5 => Some(false),
                _ => None,
            };
            m.content.push(Content::Other);
        }
        17 | 18 => {
            let ca = msg[0] & 7;
            if df == 17 {
                m.on_ground = match ca {
                    4 => Some(true),
                    5 => Some(false),
                    _ => None,
                };
            } else {
                m.non_icao = ca != 0 && ca != 6;
            }
            m.content.push(decode_me(&msg[4..11]));
            if let Some(Content::Surface { .. }) = m.content.last() {
                m.on_ground = Some(true);
            }
            if let Some(Content::Airborne { .. }) = m.content.last() {
                m.on_ground = Some(false);
            }
        }
        _ => {}
    }
    m
}

/// Ground speed of a surface position's movement field (kt)
fn movement(mv: u32) -> Option<f64> {
    let v = mv as f64;
    Some(match mv {
        0 | 125..=127 => return None,
        1 => 0.0,
        2..=8 => 0.125 + (v - 2.0) * 0.125,
        9..=12 => 1.0 + (v - 9.0) * 0.25,
        13..=38 => 2.0 + (v - 13.0) * 0.5,
        39..=93 => 15.0 + (v - 39.0),
        94..=108 => 70.0 + (v - 94.0) * 2.0,
        109..=123 => 100.0 + (v - 109.0) * 5.0,
        _ => 175.0,
    })
}

/// The 56-bit ME field of an extended squitter
pub fn decode_me(me: &[u8]) -> Content {
    let tc = me[0] >> 3;
    let b = |from: usize, n: usize| u(me, from, n);
    match tc {
        1..=4 => {
            let cat = me[0] & 7;
            let category = if cat == 0 { String::new() } else { format!("{}{}", (b'A' + (4 - tc)) as char, cat) };
            match callsign(&me[1..7]) {
                Some(callsign) => Content::Ident { callsign, category },
                None => Content::Other,
            }
        }
        5..=8 => {
            let trk_ok = b(12, 1) == 1;
            Content::Surface {
                speed: movement(b(5, 7)),
                track: trk_ok.then(|| b(13, 7) as f64 * 360.0 / 128.0),
                odd: b(21, 1) == 1,
                lat: b(22, 17),
                lon: b(39, 17),
            }
        }
        9..=18 | 20..=22 => {
            let f = b(8, 12);
            let alt = if tc >= 20 {
                // GNSS height in metres
                (f != 0).then(|| (f as f64 * 3.28084).round() as i32)
            } else {
                ac12_alt(f)
            };
            Content::Airborne { alt, gnss: tc >= 20, odd: b(21, 1) == 1, lat: b(22, 17), lon: b(39, 17) }
        }
        19 => {
            let st = b(5, 3);
            let vr = b(37, 9);
            let vrate = (vr != 0).then(|| (vr as i32 - 1) * 64 * if b(36, 1) == 1 { -1 } else { 1 });
            match st {
                1 | 2 => {
                    let k = if st == 2 { 4.0 } else { 1.0 };
                    let (vew, vns) = (b(14, 10), b(25, 10));
                    if vew == 0 || vns == 0 {
                        return Content::Velocity { speed: None, track: None, heading: None, airspeed: false, vrate };
                    }
                    let vx = (vew as f64 - 1.0) * k * if b(13, 1) == 1 { -1.0 } else { 1.0 };
                    let vy = (vns as f64 - 1.0) * k * if b(24, 1) == 1 { -1.0 } else { 1.0 };
                    let track = vx.atan2(vy).to_degrees().rem_euclid(360.0);
                    Content::Velocity { speed: Some(vx.hypot(vy)), track: Some(track), heading: None, airspeed: false, vrate }
                }
                3 | 4 => {
                    let k = if st == 4 { 4.0 } else { 1.0 };
                    let heading = (b(13, 1) == 1).then(|| b(14, 10) as f64 * 360.0 / 1024.0);
                    let asv = b(25, 10);
                    let speed = (asv != 0).then_some((asv as f64 - 1.0) * k);
                    Content::Velocity { speed, track: None, heading, airspeed: true, vrate }
                }
                _ => Content::Other,
            }
        }
        28 if b(5, 3) == 1 => Content::Squawk { code: squawk(b(11, 13)), emergency: b(8, 3) as u8 },
        _ => Content::Other,
    }
}

pub fn hex(msg: &[u8]) -> String {
    msg.iter().map(|b| format!("{b:02X}")).collect()
}

pub fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_fields() {
        // Examples of "The 1090 MHz Riddle"
        let m = from_hex("8D4840D6202CC371C32CE0576098");
        assert_eq!(syndrome(&m), 0);
        let d = decode(&m, 0x4840D6);
        assert_eq!(d.content[0], Content::Ident { callsign: "KLM1023".into(), category: String::new() });

        let v = decode_me(&from_hex("8D485020994409940838175B284F")[4..11]);
        let Content::Velocity { speed: Some(s), track: Some(t), vrate, .. } = v else { panic!("{v:?}") };
        assert!((s - 159.2).abs() < 0.1 && (t - 182.88).abs() < 0.01, "{s} {t}");
        assert_eq!(vrate, Some(-832));

        let p = decode_me(&from_hex("8D40621D58C382D690C8AC2863A7")[4..11]);
        assert_eq!(p, Content::Airborne { alt: Some(38000), gnss: false, odd: false, lat: 93000, lon: 51372 });

        // Airspeed + heading (subtype 3)
        let v = decode_me(&from_hex("8DA05F219B06B6AF189400CBC33F")[4..11]);
        let Content::Velocity { speed: Some(s), heading: Some(h), airspeed: true, vrate, .. } = v else { panic!("{v:?}") };
        assert!((s - 375.0).abs() < 0.5 && (h - 243.98).abs() < 0.05, "{s} {h}");
        assert_eq!(vrate, Some(-2304));
    }

    #[test]
    fn one_bit_repair() {
        let good = from_hex("8D4840D6202CC371C32CE0576098");
        for bit in [5, 40, 77, 111] {
            let mut m: [u8; 14] = good.clone().try_into().unwrap();
            m[bit / 8] ^= 0x80 >> (bit % 8);
            assert!(fix_one_bit(&mut m));
            assert_eq!(m.to_vec(), good);
        }
    }

    #[test]
    fn gillham_and_squawk() {
        // Squawk 7500: A=7 B=5 C=0 D=0 -> ID13 bits A4 A2 A1 B4 B1
        let id13 = 0x0800 | 0x0200 | 0x0080 | 0x0002 | 0x0020;
        assert_eq!(squawk(id13), 0x7500);
        // 25 ft steps: Q bit
        assert_eq!(ac12_alt(0xC38), Some(38000));
        // Gillham round trip: 500 ft steps in Gray code (D2 D4 A1 A2 A4 B1 B2 B4), 100 ft
        // steps in the reflected C1 C2 C4 code
        for alt in (-1200..=60_000).step_by(100) {
            let m = ((alt + 1200) / 500) as u32;
            let mut r = ((alt + 1200) % 500 / 100) as usize;
            if m & 1 == 1 {
                r = 4 - r;
            }
            let g = m ^ (m >> 1);
            let fbits = [0x0002, 0x0004, 0x1000, 0x2000, 0x4000, 0x0100, 0x0200, 0x0400];
            let mut code = (0..8).filter(|k| g >> (7 - k) & 1 == 1).fold(0, |a, k| a | fbits[k]);
            let c = [0b001, 0b011, 0b010, 0b110, 0b100][r];
            for (k, bit) in [0x0010, 0x0020, 0x0040].iter().enumerate() {
                if c >> (2 - k) & 1 == 1 {
                    code |= bit;
                }
            }
            assert_eq!(gillham_alt(code), Some(alt), "{alt} ft, code {code:04X}");
        }
    }
}
