//! Real-signal test: the DAB capture in testdata/ (channel 11D, 15 s) must give the ensemble
//! and clean DAB+ audio. Skipped when the capture is not there.
use littlesdr::dab::DabReceiver;
use littlesdr::source::u8_to_iq;

#[test]
fn decodes_the_11d_capture() {
    let Ok(raw) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/dab_11D_2048k_g40.u8")) else {
        eprintln!("capture missing: skipped");
        return;
    };
    let mut rx = DabReceiver::new();
    rx.select("radio classique");
    let mut out = Vec::new();
    for ch in raw.chunks(65536) {
        let mut s = Vec::new();
        u8_to_iq(ch, &mut s);
        rx.process(&s, &mut out);
    }
    assert_eq!(rx.fic.ensemble.label, "Métropolitain 1");
    assert_eq!(rx.fic.ensemble.services.len(), 13);
    assert_eq!(rx.fic.stats.crc_errors, 0);
    assert_eq!(rx.playing().map(|s| s.label.as_str()), Some("RADIO CLASSIQUE"));
    assert!(out.len() as f64 / 48_000.0 > 13.0, "{} s of audio", out.len() as f64 / 48_000.0);
    let rms = (out.iter().map(|f| f.l * f.l).sum::<f32>() / out.len() as f32).sqrt();
    assert!(rms > 0.05, "silent? rms {rms}");
}
