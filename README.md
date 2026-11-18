# LittleSDR engine

The SDR engine of LittleSDR, in Rust — separate from the Android app for now. The same engine
is meant to drive LittleSDR on Android (through JNI) and on Linux.

**Goal:** change receiver settings (sample rate, mode, frequency) while listening, without
stopping the dongle or cutting the sound.

## What works

- Sources: RTL-SDR over USB (system `librtlsdr`, asynchronous transfers, Blog V4 OK), rtl_tcp
- WFM with stereo (pilot PLL, falls back to mono when the pilot is weak, 50 µs de-emphasis per
  channel) and NFM, 48 kHz stereo audio
- Impulse noise blanker on the raw IQ (ignition, electric fences)
- Switchable audio filters: high-pass, low-pass, CW peak, manual notch, automatic notch (LMS),
  noise reduction (spectral), equalizer, audio AGC, limiter. Every filter runs all the time and
  its switch crossfades over 10 ms: no click, and adaptive filters are already converged
- Live sample-rate change: only the front end is rebuilt (tens of µs); demodulator, filters and
  sound output keep running. The few ms of samples in flight at the old rate are faded out.
- Sound card output (cpal) with clock-drift compensation, or WAV output for tests

## Try it

```
cargo build --release
./target/release/littlesdr --freq 96.5M            # RTL-SDR on USB, WFM
./target/release/littlesdr --rtltcp 127.0.0.1:1234 --freq 145.8M --mode nfm
```

Live commands on stdin: `freq 101.1M`, `rate 1.024M`, `mode nfm`, `gain 30` / `gain auto`,
`vol 1.2`, `stereo on|off`, `nb on|off [threshold]`, `fx nr on`, `fx notch hz 1200`, `status`,
`quit`. `--script "5s rate 1.024M; 10s quit"` runs commands at given times,
`--wav out.wav` records instead of playing.

## Layout

- `src/dsp.rs` — FIR design, decimator, resampler, NCO, FM discriminator, de-emphasis (+ tests)
- `src/stereo.rs` — broadcast FM stereo decoder
- `src/audio_fx.rs` — switchable audio filter chain (+ tests)
- `src/engine.rs` — the demodulation chain and its live reconfiguration
- `src/source/` — RTL-SDR USB, rtl_tcp
- `src/audio.rs` — sound card and WAV outputs
