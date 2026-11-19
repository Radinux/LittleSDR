//! Programme Associated Data of DAB+ (ETSI EN 300 401 clause 7, TS 102 980, EN 301 234):
//! the text sent with the programme (Dynamic Label: title, artist, show) and the MOT
//! slideshow (cover art, station pictures).
//!
//! Each access unit may carry X-PAD bytes, stored back to front, cut into subfields by
//! content indicators (CI). Subfields of one kind join into data groups (each with a CRC):
//! DL segments, then the MOT data groups (header and body segments of each picture).

use super::fec::crc16_ok;
use std::collections::HashMap;

const CI_LENS: [usize; 8] = [4, 6, 8, 12, 16, 24, 32, 48];
const T_DGLI: u8 = 1;
const T_DL_START: u8 = 2;
const T_DL_CONT: u8 = 3;
/// X-PAD application type of the MOT slideshow (start; +1 = continuation)
const T_MOT_START: u8 = 12;
const T_MOT_CONT: u8 = 13;

/// A picture of the slideshow
#[derive(Clone, Debug)]
pub struct Slide {
    pub name: String,
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// Data group being assembled from subfields; complete once `need` bytes are there
#[derive(Default)]
struct Group {
    buf: Vec<u8>,
    need: Option<usize>,
    active: bool,
}

impl Group {
    fn start(&mut self) {
        self.buf.clear();
        self.need = None;
        self.active = true;
    }
    fn add(&mut self, data: &[u8]) {
        if self.active {
            self.buf.extend_from_slice(data);
            if self.buf.len() > 16_384 {
                self.active = false; // runaway
            }
        }
    }
}

#[derive(Default)]
struct MotObject {
    header: Option<Vec<u8>>,
    header_segs: HashMap<u16, Vec<u8>>,
    header_last: Option<u16>,
    body_segs: HashMap<u16, Vec<u8>>,
    body_last: Option<u16>,
    done: bool,
}

#[derive(Default)]
pub struct PadDecoder {
    /// Content indicator of the last subfield, continued when an AU has no CI list
    last_type: Option<u8>,
    dgli: Group,
    dgli_len: usize,
    dl: Group,
    dl_segs: [Option<Vec<u8>>; 8],
    dl_toggle: Option<bool>,
    dl_charset: u8,
    mot: Group,
    mot_len: usize,
    objects: HashMap<u16, MotObject>,
    /// Latest complete text, and whether it changed since it was last taken
    pub label: Option<String>,
    pub label_new: bool,
    pub slide: Option<Slide>,
    pub slide_new: bool,
    pub stats: PadStats,
}

#[derive(Default, Debug, Clone)]
pub struct PadStats {
    pub xpad: u64,
    pub dl_groups: u64,
    pub mot_groups: u64,
    pub crc_errors: u64,
    pub slides: u64,
}

impl PadDecoder {
    pub fn new() -> PadDecoder {
        PadDecoder::default()
    }

    /// One DAB+ access unit (without its CRC): finds the PAD in its data stream element
    pub fn process_au(&mut self, au: &[u8]) {
        if au.len() < 3 || au[0] >> 5 != 4 {
            return; // no data stream element: no PAD
        }
        let (mut start, mut len) = (2usize, au[1] as usize);
        if len == 255 {
            len += au[2] as usize;
            start = 3;
        }
        if len < 2 || au.len() < start + len {
            return;
        }
        let xpad = &au[start..start + len - 2];
        let fpad = &au[start + len - 2..start + len];
        self.process(xpad, fpad);
    }

    fn process(&mut self, xpad_raw: &[u8], fpad: &[u8]) {
        let fpad_type = fpad[0] >> 6;
        let xpad_ind = (fpad[0] >> 4) & 3;
        let ci_flag = fpad[1] & 0x02 != 0;
        if fpad_type != 0 || xpad_ind == 0 || xpad_raw.is_empty() {
            return;
        }
        self.stats.xpad += 1;
        // X-PAD bytes come last-first
        let xpad: Vec<u8> = xpad_raw.iter().rev().copied().collect();
        // Subfields: (type, length)
        let mut subs: Vec<(u8, usize)> = Vec::new();
        let mut off = 0usize;
        if ci_flag {
            if xpad_ind == 1 {
                // Short X-PAD: one CI byte, 3 data bytes
                let t = xpad[0] & 0x1F;
                off = 1;
                if t != 0 {
                    subs.push((t, 3));
                }
            } else {
                for i in 0..4.min(xpad.len()) {
                    off += 1;
                    let t = xpad[i] & 0x1F;
                    if t == 0 {
                        break; // end marker
                    }
                    subs.push((t, CI_LENS[(xpad[i] >> 5) as usize]));
                }
            }
        } else if let Some(t) = self.last_type {
            subs.push((t, xpad.len())); // continuation of the previous subfield
        }
        let mut last = None;
        for (t, len) in subs {
            let end = (off + len).min(xpad.len());
            if off >= end {
                break;
            }
            let data = &xpad[off..end];
            off = end;
            match t {
                T_DGLI => {
                    self.dgli.start();
                    self.dgli.add(data);
                    self.finish_dgli();
                    last = Some(T_DGLI);
                }
                T_DL_START | T_DL_CONT => {
                    if t == T_DL_START {
                        self.dl.start();
                    }
                    self.dl.add(data);
                    self.finish_dl();
                    last = Some(T_DL_CONT);
                }
                T_MOT_START | T_MOT_CONT => {
                    if t == T_MOT_START {
                        self.mot.start();
                        self.mot_len = std::mem::take(&mut self.dgli_len);
                    }
                    self.mot.add(data);
                    self.finish_mot();
                    last = Some(T_MOT_CONT);
                }
                _ => last = None,
            }
        }
        // A new CI list sets what an AU without one continues
        if ci_flag {
            self.last_type = last;
        }
    }

    fn finish_dgli(&mut self) {
        let g = &mut self.dgli;
        if g.buf.len() >= 4 {
            if crc16_ok(&g.buf[..4]) {
                self.dgli_len = ((g.buf[0] as usize & 0x3F) << 8) | g.buf[1] as usize;
            }
            g.active = false;
        }
    }

    fn finish_dl(&mut self) {
        let g = &mut self.dl;
        if !g.active || g.buf.len() < 2 {
            return;
        }
        let command = g.buf[0] & 0x10 != 0;
        let field = if command { 0 } else { (g.buf[0] & 0x0F) as usize + 1 };
        let total = 2 + field + 2;
        if g.buf.len() < total {
            return;
        }
        g.active = false;
        let grp = g.buf[..total].to_vec();
        if !crc16_ok(&grp) {
            self.stats.crc_errors += 1;
            return;
        }
        self.stats.dl_groups += 1;
        if command {
            if grp[0] & 0x0F == 1 {
                self.label = None; // "remove label"
            }
            return;
        }
        let toggle = grp[0] & 0x80 != 0;
        let first = grp[0] & 0x40 != 0;
        let last = grp[0] & 0x20 != 0;
        let seg = if first { 0 } else { ((grp[1] >> 4) & 7) as usize };
        if self.dl_toggle != Some(toggle) {
            self.dl_segs = Default::default(); // a new label
            self.dl_toggle = Some(toggle);
        }
        if first {
            self.dl_charset = grp[1] >> 4;
        }
        self.dl_segs[seg] = Some(grp[2..2 + field].to_vec());
        if last {
            // Complete when all segments up to the last one are there
            let n = seg + 1;
            if self.dl_segs[..n].iter().all(|s| s.is_some()) {
                let raw: Vec<u8> = self.dl_segs[..n].iter().flat_map(|s| s.clone().unwrap()).collect();
                let text = decode_text(&raw, self.dl_charset);
                if self.label.as_deref() != Some(text.as_str()) {
                    self.label = Some(text);
                    self.label_new = true;
                }
            }
        }
    }

    fn finish_mot(&mut self) {
        let g = &mut self.mot;
        if !g.active || self.mot_len < 4 || g.buf.len() < self.mot_len {
            return;
        }
        g.active = false;
        let grp = g.buf[..self.mot_len].to_vec();
        if !crc16_ok(&grp) {
            self.stats.crc_errors += 1;
            return;
        }
        self.stats.mot_groups += 1;
        self.mot_group(&grp[..grp.len() - 2]);
    }

    /// One MSC data group (EN 300 401 5.3.3) carrying a MOT segment
    fn mot_group(&mut self, d: &[u8]) {
        if d.len() < 2 {
            return;
        }
        let ext = d[0] & 0x80 != 0;
        let seg_flag = d[0] & 0x20 != 0;
        let ua_flag = d[0] & 0x10 != 0;
        let dg_type = d[0] & 0x0F;
        let mut i = 2 + if ext { 2 } else { 0 };
        let (mut last, mut segnum) = (false, 0u16);
        if seg_flag {
            if d.len() < i + 2 {
                return;
            }
            last = d[i] & 0x80 != 0;
            segnum = u16::from_be_bytes([d[i] & 0x7F, d[i + 1]]);
            i += 2;
        }
        let mut tid = None;
        if ua_flag {
            if d.len() < i + 1 {
                return;
            }
            let tid_flag = d[i] & 0x10 != 0;
            let len = (d[i] & 0x0F) as usize;
            if tid_flag && len >= 2 && d.len() >= i + 3 {
                tid = Some(u16::from_be_bytes([d[i + 1], d[i + 2]]));
            }
            i += 1 + len;
        }
        let Some(tid) = tid else { return };
        // Segmentation header: repetition count (3) + segment size (13)
        if d.len() < i + 2 {
            return;
        }
        let size = (u16::from_be_bytes([d[i] & 0x1F, d[i + 1]])) as usize;
        i += 2;
        let seg = d[i..(i + size).min(d.len())].to_vec();
        if self.objects.len() > 32 && !self.objects.contains_key(&tid) {
            self.objects.clear(); // keep memory bounded on long listening
        }
        let o = self.objects.entry(tid).or_default();
        match dg_type {
            3 => {
                o.header_segs.insert(segnum, seg);
                if last {
                    o.header_last = Some(segnum);
                }
                if let Some(l) = o.header_last {
                    if (0..=l).all(|n| o.header_segs.contains_key(&n)) {
                        o.header = Some((0..=l).flat_map(|n| o.header_segs[&n].clone()).collect());
                    }
                }
            }
            4 => {
                o.body_segs.insert(segnum, seg);
                if last {
                    o.body_last = Some(segnum);
                }
            }
            _ => return,
        }
        // Complete object?
        let (Some(h), Some(l)) = (&o.header, o.body_last) else { return };
        if o.done || !(0..=l).all(|n| o.body_segs.contains_key(&n)) || h.len() < 7 {
            return;
        }
        let body: Vec<u8> = (0..=l).flat_map(|n| o.body_segs[&n].clone()).collect();
        let body_size = (u32::from_be_bytes([h[0], h[1], h[2], h[3]]) >> 4) as usize;
        let ctype = (h[5] >> 1) & 0x3F;
        let subtype = (((h[5] & 1) as u16) << 8) | h[6] as u16;
        if body.len() < body_size {
            return;
        }
        o.done = true;
        let name = mot_name(&h[7..]).unwrap_or_else(|| format!("{tid:04X}"));
        let mime = match (ctype, subtype) {
            (2, 1) => "image/jpeg",
            (2, 3) => "image/png",
            (2, 0) => "image/gif",
            _ => return,
        };
        let data = body[..body_size].to_vec();
        // Carousels send the same picture again under new ids: only a different one is news
        if self.slide.as_ref().is_some_and(|s| s.data == data) {
            return;
        }
        self.stats.slides += 1;
        self.slide = Some(Slide { name, mime, data });
        self.slide_new = true;
    }
}

/// ContentName (parameter 0x0C) from a MOT header extension
fn mot_name(mut p: &[u8]) -> Option<String> {
    while !p.is_empty() {
        let pli = p[0] >> 6;
        let id = p[0] & 0x3F;
        let (len, hdr) = match pli {
            0 => (0, 1),
            1 => (1, 1),
            2 => (4, 1),
            _ => {
                if p.len() < 2 {
                    return None;
                }
                if p[1] & 0x80 != 0 {
                    if p.len() < 3 {
                        return None;
                    }
                    ((((p[1] & 0x7F) as usize) << 8) | p[2] as usize, 3)
                } else {
                    ((p[1] & 0x7F) as usize, 2)
                }
            }
        };
        if p.len() < hdr + len {
            return None;
        }
        if id == 0x0C && len > 1 {
            // First byte: charset
            return Some(decode_text(&p[hdr + 1..hdr + len], p[hdr] >> 4));
        }
        p = &p[hdr + len..];
    }
    None
}

/// Text in a DAB charset: 0 = EBU Latin, 4 = ISO 8859-1, 15 = UTF-8
pub fn decode_text(raw: &[u8], charset: u8) -> String {
    let s: String = match charset {
        15 => String::from_utf8_lossy(raw).into_owned(),
        4 => raw.iter().map(|&b| b as char).collect(),
        _ => raw.iter().map(|&b| super::fic::ebu_char(b)).collect(),
    };
    s.trim().to_string()
}
