//! Meteor-M LRPT weather satellite reception (137 MHz): QPSK / OQPSK demodulation, CCSDS
//! frames (Viterbi, Reed-Solomon), packets and MSU-MR image decompression.
//!
//! Meteor-M N2-4 (checked on a real pass): 72 ksym/s OQPSK with differential coding; the
//! frame sync finds the coding by itself, QPSK or OQPSK is a setting.

pub mod demod;
pub mod fec;
pub mod frame;
pub mod image;

use crate::dsp::C32;
use demod::Demod;
use frame::{Framer, Vcdu};
use image::{Imager, Packet, PacketAssembler};
use std::collections::HashMap;

pub struct LrptDecoder {
    pub demod: Demod,
    pub framer: Framer,
    assemblers: HashMap<u8, PacketAssembler>,
    pub imager: Imager,
    soft: Vec<i8>,
    vcdus: Vec<Vcdu>,
    packets: Vec<Packet>,
}

impl LrptDecoder {
    pub fn new(rate: f64, oqpsk: bool) -> LrptDecoder {
        LrptDecoder {
            demod: Demod::new(rate, oqpsk),
            framer: Framer::new(),
            assemblers: HashMap::new(),
            imager: Imager::new(),
            soft: Vec::new(),
            vcdus: Vec::new(),
            packets: Vec::new(),
        }
    }

    pub fn process(&mut self, iq: &[C32]) {
        self.soft.clear();
        self.demod.process(iq, &mut self.soft);
        self.vcdus.clear();
        self.framer.push(&self.soft, &mut self.vcdus);
        if !self.vcdus.is_empty() {
            self.demod.confirm_lock();
        }
        for v in &self.vcdus {
            let vcid = v.data[1] & 0x3F;
            if v.data[0] >> 6 == 0 || vcid == 0 || vcid == 63 {
                continue; // fill frames
            }
            self.packets.clear();
            self.assemblers.entry(vcid).or_default().push(&v.data, &mut self.packets);
            for p in &self.packets {
                self.imager.push(p);
            }
        }
    }

    pub fn describe(&self) -> String {
        let f = &self.framer.stats;
        format!(
            "LRPT: carrier {:+.0} Hz, {}, eye {:.2} | frames {} ok / {} failed{} | {} image packets, channels {:?}, {} lines",
            self.demod.freq_hz(),
            if self.framer.locked() { "frames locked" } else { "searching" },
            self.demod.eye(),
            f.rs_ok,
            f.rs_fail,
            match f.diff {
                Some(true) => ", differential",
                _ => "",
            },
            self.imager.decoded,
            self.imager.channels(),
            self.imager.channel(self.imager.channels().first().copied().unwrap_or(64)).map_or(0, |c| c.1 / 8),
        )
    }

    /// Writes one PNG per channel, plus a 221 colour composite when channels 64 and 65 are
    /// there. Returns the files written.
    pub fn save(&self, prefix: &str) -> std::io::Result<Vec<String>> {
        let mut files = Vec::new();
        for apid in self.imager.channels() {
            if let Some((w, h, px)) = self.imager.channel(apid) {
                let name = format!("{prefix}_{apid}.png");
                std::fs::write(&name, image::png(w, h, 1, &px))?;
                files.push(name);
            }
        }
        if let (Some((w, h, b)), Some((_, _, g))) = (self.imager.channel(64), self.imager.channel(65)) {
            let rgb: Vec<u8> = g.iter().zip(&b).flat_map(|(&g, &b)| [g, g, b]).collect();
            let name = format!("{prefix}_221.png");
            std::fs::write(&name, image::png(w, h, 3, &rgb))?;
            files.push(name);
        }
        Ok(files)
    }
}
