# LittleSDR engine

The SDR engine of LittleSDR, in Rust — separate from the Android app for now. The same engine
is meant to drive LittleSDR on Android (through JNI) and on Linux.

**Goal:** change receiver settings (sample rate, mode, frequency) while listening, without
stopping the dongle or cutting the sound.

## What works

- Sources: RTL-SDR over USB (system `librtlsdr`, asynchronous transfers, Blog V4 OK), rtl_tcp
- WFM with stereo (pilot PLL, falls back to mono when the pilot is weak, 50 µs de-emphasis per
  channel), NFM, AM, USB, LSB and CW, 48 kHz stereo audio
- AM / SSB / CW bandwidth adjustable live (crossfaded filter swap, no click); SSB and CW run
  their channel filter and AGC (look-ahead, hang) at 12 kHz; CW pitch adjustable
- Tuning by absolute frequency: inside the captured band only the VFO moves, the dongle is
  retuned only when the channel leaves the clean part of the band or nears the DC spike
- HF: RTL-SDR Blog V4 upconverter handled by the driver; direct sampling (`--ds 2`) for others
- RTTY decoder on USB / LSB audio: any baud rate and shift (45.45 Bd / 170 Hz amateur,
  50 Bd / 450 Hz weather), polarity found automatically (or forced), US or ITA2 figures,
  follows a mistuned signal, squelch (framing
  quality + energy on the tones) so noise prints nothing; text on stdout
- Meteor-M LRPT weather pictures: QPSK / OQPSK demodulator (carrier from the 4th power of
  the signal, Costas loop, symbol clock), frame sync under any phase, Viterbi, CCSDS
  descrambling, Reed-Solomon, packets, MSU-MR JPEG decompression, PNG per channel + colour
  composite. Checked on a real Meteor-M2-4 pass (OQPSK + differential coding): 96 % of the
  frames, 11 min 44 s decoded in 18 s (`cargo run --release --example lrpt_file -- pass.wav`)
- Signal meter: channel power and noise floor (dBFS), SNR, ADC clipping; warns unasked when
  the gain is clearly too high (ADC clipping) or too low (`meter on` prints it every second)
- Bias-tee for active antennas (`--bias-tee`, `bt on|off`), always switched off on exit (Ctrl+C included)
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
./target/release/littlesdr --freq 7.1M --mode lsb --gain 40 --bias-tee   # 40 m, active loop
```

Live commands on stdin: `freq 101.1M` (`freq +500` / `freq -1k` to step), `rate 1.024M`,
`mode wfm|nfm|am|usb|lsb|cw`, `bw 2.4k`, `pitch 650`, `gain 30` / `gain auto`, `ds 0|1|2`,
`vol 1.2`, `stereo on|off`, `nb on|off [threshold]`, `fx nr on`, `fx notch hz 1200`, `status`,
`meter on|off`, `rtty on|off`, `rtty shift 450`, `rtty baud 50`, `rtty ita2 on`, `rtty rev on|off|auto`,
`quit`. `--script "5s rate 1.024M; 10s quit"` runs commands at given times,
`--wav out.wav` records instead of playing.

## Layout

- `src/dsp.rs` — FIR design, decimator, crossfading filter swap, polyphase upsampler, resampler,
  NCO, FM discriminator, de-emphasis, noise blanker (+ tests)
- `src/rtty.rs` — RTTY decoder (+ tests, including through the whole engine)
- `src/lrpt/` — Meteor LRPT: demodulator, FEC (Viterbi, Reed-Solomon), frames, images (+ tests)
- `src/meter.rs` — signal meter and ADC overload detection (+ tests)
- `src/linear.rs` — AM, SSB and CW demodulators, IF AGC (+ tests)
- `src/stereo.rs` — broadcast FM stereo decoder
- `src/audio_fx.rs` — switchable audio filter chain (+ tests)
- `src/engine.rs` — the demodulation chain and its live reconfiguration
- `src/source/` — RTL-SDR USB, rtl_tcp
- `src/audio.rs` — sound card and WAV outputs
