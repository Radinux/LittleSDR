#!/usr/bin/env python3
"""Fake rtl_tcp server: plays a WAV file as an NFM signal, for testing decoders without radio.

    tools/fake_rtl_tcp.py audio.wav [--freq 145.8e6] [--port 1234] [--dev 5000] [--gap 8]

The WAV is frequency-modulated at --freq (5 kHz deviation, light noise) and looped with --gap
seconds of noise in between; it follows the center frequency and sample rate the client sets.
From an Android emulator: `adb reverse tcp:1234 tcp:1234`, then RTL-TCP on localhost:1234.
"""
import argparse, socket, struct, threading, time, wave
import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument('wav'); ap.add_argument('--freq', type=float, default=145.8e6)
ap.add_argument('--port', type=int, default=1234); ap.add_argument('--dev', type=float, default=5000)
ap.add_argument('--gap', type=float, default=8.0); ap.add_argument('--snr', type=float, default=30.0)
args = ap.parse_args()

w = wave.open(args.wav)
assert w.getsampwidth() == 2, '16-bit WAV expected'
audio = np.frombuffer(w.readframes(w.getnframes()), np.int16).astype(np.float32) / 32768
if w.getnchannels() > 1: audio = audio.reshape(-1, w.getnchannels()).mean(1)
afs = w.getframerate()
audio = np.concatenate([np.zeros(int(args.gap * afs), np.float32), audio])  # gap first, then the image
audio /= max(1e-6, np.abs(audio).max())
print(f'{len(audio)/afs:.0f} s loop at {args.freq/1e6:.3f} MHz, port {args.port}')

def serve(conn):
    state = {'center': 145.8e6, 'rate': 2.4e6}
    def commands():
        try:
            while True:
                d = conn.recv(5)
                if len(d) < 5: break
                cmd, val = struct.unpack('>BI', d)
                if cmd == 1: state['center'] = float(val); print('center', val)
                elif cmd == 2: state['rate'] = float(val); print('rate', val)
        except OSError: pass
    threading.Thread(target=commands, daemon=True).start()
    conn.sendall(b'RTL0' + struct.pack('>II', 5, 29))  # R820T, 29 gains
    rng = np.random.default_rng()
    phase, apos, t_next = 0.0, 0.0, time.monotonic()
    chunk_s = 0.05
    try:
        while True:
            sr = state['rate']; n = int(sr * chunk_s)
            # Audio at the IQ rate (linear interpolation), looped
            t = apos + np.arange(n) * (afs / sr)
            a = np.interp(t % len(audio), np.arange(len(audio)), audio)
            apos = (apos + n * afs / sr) % len(audio)
            off = args.freq - state['center']
            if abs(off) < sr / 2:
                inst = 2 * np.pi * (off + args.dev * a) / sr
                ph = phase + np.cumsum(inst); phase = float(ph[-1] % (2 * np.pi))
                iq = np.exp(1j * ph) * 0.5
            else:
                iq = np.zeros(n, complex)
            sigma = 0.5 / np.sqrt(2) / 10 ** (args.snr / 20) * np.sqrt(sr / 12500)  # SNR in 12.5 kHz
            iq = iq + (rng.normal(0, sigma, n) + 1j * rng.normal(0, sigma, n))
            out = np.empty(2 * n, np.uint8)
            out[0::2] = np.clip(127.5 + 127 * iq.real, 0, 255)
            out[1::2] = np.clip(127.5 + 127 * iq.imag, 0, 255)
            conn.sendall(out.tobytes())
            t_next += chunk_s
            time.sleep(max(0.0, t_next - time.monotonic()))
    except OSError:
        print('client gone')

srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(('127.0.0.1', args.port)); srv.listen(1)
while True:
    c, addr = srv.accept(); print('client', addr)
    threading.Thread(target=serve, args=(c,), daemon=True).start()
