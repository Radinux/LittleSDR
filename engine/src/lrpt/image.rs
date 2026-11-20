//! From VCDUs to pictures: CCSDS packet reassembly, MSU-MR image decompression (a reduced
//! baseline JPEG: 14 blocks of 8x8 per packet, standard luminance Huffman tables, the
//! standard quantization table scaled by a quality factor carried in each packet), and the
//! assembly of each channel into a 1568-pixel-wide image.

use std::collections::BTreeMap;

pub const MCU_PER_PACKET: usize = 14;
pub const PACKETS_PER_LINE: usize = 14;
pub const WIDTH: usize = MCU_PER_PACKET * PACKETS_PER_LINE * 8; // 1568
/// Packets per scan period: 3 imaging channels x 14, plus one calibration packet
const PERIOD: i64 = 43;

// ---------------------------------------------------------------- packets

/// CCSDS packet reassembly from the data zones of consecutive VCDUs of one virtual channel
#[derive(Default)]
pub struct PacketAssembler {
    buf: Vec<u8>,
    synced: bool,
    last_counter: Option<u32>,
    pub packets: u64,
    pub lost: u64,
}

pub struct Packet {
    pub apid: u16,
    pub seq: u16,
    pub data: Vec<u8>, // after the 6-byte primary header
}

impl PacketAssembler {
    pub fn push(&mut self, vcdu: &[u8], out: &mut Vec<Packet>) {
        let counter = (vcdu[2] as u32) << 16 | (vcdu[3] as u32) << 8 | vcdu[4] as u32;
        if self.last_counter.map_or(true, |c| (c + 1) & 0xFF_FFFF != counter) {
            if self.synced {
                self.lost += 1;
            }
            self.synced = false;
            self.buf.clear();
        }
        self.last_counter = Some(counter);
        let hdr = ((vcdu[8] as usize & 7) << 8) | vcdu[9] as usize;
        let zone = &vcdu[10..892];
        if !self.synced {
            if hdr == 0x7FF || hdr >= zone.len() {
                return; // no packet starts here
            }
            self.buf.extend_from_slice(&zone[hdr..]);
            self.synced = true;
        } else {
            self.buf.extend_from_slice(zone);
        }
        while self.buf.len() >= 6 {
            let len = ((self.buf[4] as usize) << 8 | self.buf[5] as usize) + 1;
            if len > 2048 {
                self.synced = false; // nonsense length: wait for the next header pointer
                self.buf.clear();
                break;
            }
            if self.buf.len() < 6 + len {
                break;
            }
            let apid = ((self.buf[0] as u16 & 7) << 8) | self.buf[1] as u16;
            let seq = ((self.buf[2] as u16 & 0x3F) << 8) | self.buf[3] as u16;
            let data = self.buf[6..6 + len].to_vec();
            self.buf.drain(..6 + len);
            if apid != 2047 {
                self.packets += 1;
                out.push(Packet { apid, seq, data });
            }
        }
    }
}

// ---------------------------------------------------------------- JPEG

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57,
    50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const QUANT: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22,
    37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

// Standard JPEG luminance tables (ITU T.81 K.3): code counts per length, then the symbols
const DC_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];
const AC_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91,
    0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A,
    0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53,
    0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
    0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
    0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2,
    0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// Canonical Huffman table: (code, length) -> symbol, looked up by length
struct Huffman {
    /// For each length 1..=16: first code, number of codes, index of the first symbol
    first: [u32; 17],
    count: [u32; 17],
    index: [usize; 17],
    vals: &'static [u8],
}

impl Huffman {
    fn new(bits: &[u8; 16], vals: &'static [u8]) -> Huffman {
        let (mut first, mut count, mut index) = ([0u32; 17], [0u32; 17], [0usize; 17]);
        let (mut code, mut k) = (0u32, 0usize);
        for len in 1..=16 {
            first[len] = code;
            count[len] = bits[len - 1] as u32;
            index[len] = k;
            code = (code + count[len]) << 1;
            k += count[len] as usize;
        }
        Huffman { first, count, index, vals }
    }

    fn decode(&self, r: &mut BitReader) -> Option<u8> {
        let mut code = 0u32;
        for len in 1..=16 {
            code = (code << 1) | r.bit()?;
            if code.wrapping_sub(self.first[len]) < self.count[len] {
                return Some(self.vals[self.index[len] + (code - self.first[len]) as usize]);
            }
        }
        None
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn bit(&mut self) -> Option<u32> {
        let b = *self.data.get(self.pos / 8)?;
        let v = (b >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(v as u32)
    }
    fn bits(&mut self, n: u8) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }
}

/// JPEG magnitude category value: `cat` bits, leading 0 = negative
fn extend(v: u32, cat: u8) -> i32 {
    if cat == 0 {
        0
    } else if v >> (cat - 1) == 1 {
        v as i32
    } else {
        v as i32 - ((1 << cat) - 1)
    }
}

fn quant_table(q: u8) -> [i32; 64] {
    let q = q as i32;
    let ratio = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut t = [0i32; 64];
    for (o, &b) in t.iter_mut().zip(QUANT.iter()) {
        *o = ((b as i32 * ratio / 50 + 1) / 2).max(1);
    }
    t
}

pub struct Jpeg {
    dc: Huffman,
    ac: Huffman,
    cos: [[f32; 8]; 8], // cos[x][u] * C(u) / 2
}

impl Default for Jpeg {
    fn default() -> Self {
        Self::new()
    }
}

impl Jpeg {
    pub fn new() -> Jpeg {
        let mut cos = [[0f32; 8]; 8];
        for (x, row) in cos.iter_mut().enumerate() {
            for (u, c) in row.iter_mut().enumerate() {
                let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *c = cu / 2.0 * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos();
            }
        }
        Jpeg { dc: Huffman::new(&DC_BITS, &DC_VALS), ac: Huffman::new(&AC_BITS, &AC_VALS), cos }
    }

    /// Decodes the 14 blocks of one packet into a 8 x 112 strip. Blocks the data runs out
    /// for are left black; None if the very first block is unreadable.
    pub fn decode_strip(&self, data: &[u8], q: u8) -> Option<[[u8; 8 * MCU_PER_PACKET]; 8]> {
        if q == 0 {
            return None;
        }
        let qt = quant_table(q);
        let mut r = BitReader { data, pos: 0 };
        let mut strip = [[0u8; 8 * MCU_PER_PACKET]; 8];
        let mut dc = 0i32;
        for m in 0..MCU_PER_PACKET {
            let mut coef = [0i32; 64];
            let Some(cat) = self.dc.decode(&mut r) else {
                return (m > 0).then_some(strip);
            };
            let Some(v) = r.bits(cat) else { return (m > 0).then_some(strip) };
            dc += extend(v, cat);
            coef[0] = dc;
            let mut k = 1;
            while k < 64 {
                let Some(rs) = self.ac.decode(&mut r) else { return (m > 0).then_some(strip) };
                if rs == 0x00 {
                    break; // end of block
                }
                let (run, size) = ((rs >> 4) as usize, rs & 0x0F);
                k += run;
                if k >= 64 {
                    break;
                }
                let Some(v) = r.bits(size) else { return (m > 0).then_some(strip) };
                coef[ZIGZAG[k]] = extend(v, size);
                k += 1;
            }
            // Dequantize, inverse DCT, level shift
            let mut f = [0f32; 64];
            for i in 0..64 {
                f[i] = (coef[i] * qt[i]) as f32;
            }
            let mut tmp = [0f32; 64]; // rows: horizontal frequencies -> x
            for v in 0..8 {
                for x in 0..8 {
                    let mut acc = 0.0;
                    for u in 0..8 {
                        acc += self.cos[x][u] * f[v * 8 + u];
                    }
                    tmp[v * 8 + x] = acc;
                }
            }
            for x in 0..8 {
                for y in 0..8 {
                    let mut acc = 0.0;
                    for v in 0..8 {
                        acc += self.cos[y][v] * tmp[v * 8 + x];
                    }
                    strip[y][m * 8 + x] = (acc + 128.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Some(strip)
    }
}

// ---------------------------------------------------------------- channels

/// Image strips of all channels, placed by packet counter so channels stay aligned and lost
/// packets leave black gaps instead of shifting the picture
pub struct Imager {
    jpeg: Jpeg,
    /// apid -> (line, packet in line) -> strip
    strips: BTreeMap<u16, BTreeMap<(i64, usize), Box<[[u8; 8 * MCU_PER_PACKET]; 8]>>>,
    last_seq: Option<u16>,
    unwrapped: i64,
    /// Period phase, from the calibration packet (APID 70) that closes each period
    phase: Option<i64>,
    pub decoded: u64,
    pub bad: u64,
}

impl Default for Imager {
    fn default() -> Self {
        Self::new()
    }
}

impl Imager {
    pub fn new() -> Imager {
        Imager { jpeg: Jpeg::new(), strips: BTreeMap::new(), last_seq: None, unwrapped: 0, phase: None, decoded: 0, bad: 0 }
    }

    fn unwrap(&mut self, seq: u16) -> i64 {
        if let Some(last) = self.last_seq {
            let mut d = (seq as i64 - last as i64).rem_euclid(16384);
            if d > 8192 {
                d -= 16384; // a packet from just before
            }
            self.unwrapped += d;
        }
        self.last_seq = Some(seq);
        self.unwrapped
    }

    pub fn push(&mut self, p: &Packet) {
        let s = self.unwrap(p.seq);
        if p.apid == 70 {
            self.phase.get_or_insert((s + 1).rem_euclid(PERIOD));
            return;
        }
        if !(64..=69).contains(&p.apid) || p.data.len() < 14 {
            return;
        }
        let mcu = p.data[8] as usize;
        let q = p.data[13];
        let slot = mcu / MCU_PER_PACKET;
        if slot >= PACKETS_PER_LINE {
            self.bad += 1;
            return;
        }
        let Some(strip) = self.jpeg.decode_strip(&p.data[14..], q) else {
            self.bad += 1;
            return;
        };
        // The packets of one line of a channel are consecutive: its first one is `slot`
        // packets earlier; lines are one period apart
        let start = s - slot as i64;
        let phase = *self.phase.get_or_insert(start.rem_euclid(PERIOD));
        let line = (start - phase).div_euclid(PERIOD);
        self.strips.entry(p.apid).or_default().insert((line, slot), Box::new(strip));
        self.decoded += 1;
    }

    pub fn channels(&self) -> Vec<u16> {
        self.strips.keys().copied().collect()
    }

    fn line_range(&self) -> Option<(i64, i64)> {
        let lo = self.strips.values().filter_map(|m| m.keys().next().map(|k| k.0)).min()?;
        let hi = self.strips.values().filter_map(|m| m.keys().next_back().map(|k| k.0)).max()?;
        Some((lo, hi))
    }

    /// Grayscale picture of one channel: (width, height, pixels), all channels the same size
    pub fn channel(&self, apid: u16) -> Option<(usize, usize, Vec<u8>)> {
        let (lo, hi) = self.line_range()?;
        let lines = (hi - lo + 1) as usize;
        let mut px = vec![0u8; WIDTH * lines * 8];
        for (&(line, slot), strip) in self.strips.get(&apid)? {
            let y0 = (line - lo) as usize * 8;
            for (r, row) in strip.iter().enumerate() {
                let off = (y0 + r) * WIDTH + slot * 8 * MCU_PER_PACKET;
                px[off..off + row.len()].copy_from_slice(row);
            }
        }
        Some((WIDTH, lines * 8, px))
    }
}

// ---------------------------------------------------------------- PNG

/// Minimal PNG writer (stored deflate blocks: no compression library needed).
/// `channels`: 1 = gray, 3 = RGB.
pub fn png(width: usize, height: usize, channels: usize, px: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width * channels + 1) * height);
    for row in px.chunks(width * channels) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 3 { 2 } else { 0 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal encoder for one flat block (DC only), to check the decoder end to end
    fn encode_dc_blocks(levels: &[i32], q: u8) -> Vec<u8> {
        let qt = quant_table(q);
        let mut bits: Vec<u8> = Vec::new();
        let put = |bits: &mut Vec<u8>, v: u32, n: u8| {
            for i in (0..n).rev() {
                bits.push(((v >> i) & 1) as u8);
            }
        };
        // Code for a symbol in a canonical table
        let code_of = |tbits: &[u8; 16], vals: &[u8], sym: u8| -> (u32, u8) {
            let (mut code, mut k) = (0u32, 0usize);
            for len in 1..=16u8 {
                for _ in 0..tbits[len as usize - 1] {
                    if vals[k] == sym {
                        return (code, len);
                    }
                    code += 1;
                    k += 1;
                }
                code <<= 1;
            }
            panic!("no code");
        };
        let mut prev = 0;
        for &level in levels {
            // A flat block of value `level`: DC coefficient = 8 * (level - 128) / Q[0]
            let dc = ((level - 128) * 8) / qt[0];
            let diff = dc - prev;
            prev = dc;
            let cat = if diff == 0 { 0 } else { 32 - diff.unsigned_abs().leading_zeros() } as u8;
            let (c, l) = code_of(&DC_BITS, &DC_VALS, cat);
            put(&mut bits, c, l);
            let v = if diff >= 0 { diff as u32 } else { (diff + (1 << cat) - 1) as u32 };
            put(&mut bits, v, cat);
            let (c, l) = code_of(&AC_BITS, &AC_VALS, 0x00); // EOB
            put(&mut bits, c, l);
        }
        while bits.len() % 8 != 0 {
            bits.push(1);
        }
        bits.chunks(8).map(|b| b.iter().fold(0u8, |a, &x| (a << 1) | x)).collect()
    }

    #[test]
    fn decodes_flat_blocks() {
        let levels: Vec<i32> = (0..14).map(|i| 20 + i * 15).collect();
        let data = encode_dc_blocks(&levels, 80);
        let strip = Jpeg::new().decode_strip(&data, 80).unwrap();
        let qt = quant_table(80);
        for (m, &lvl) in levels.iter().enumerate() {
            let expect = 128 + ((lvl - 128) * 8 / qt[0]) * qt[0] / 8; // quantized
            for row in &strip {
                for &p in &row[m * 8..m * 8 + 8] {
                    assert!((p as i32 - expect).abs() <= 1, "block {m}: {p} vs {expect}");
                }
            }
        }
    }

    #[test]
    fn png_is_well_formed() {
        let px: Vec<u8> = (0..64 * 70_000 / 64).map(|i| i as u8).collect();
        let p = png(100, px.len() / 100, 1, &px[..px.len() / 100 * 100]);
        assert_eq!(&p[1..4], b"PNG");
        assert!(p.windows(4).any(|w| w == b"IEND"));
    }
}
