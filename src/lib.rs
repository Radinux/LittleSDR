//! LittleSDR engine: IQ sources, a demodulation chain that reconfigures live (sample rate,
//! mode, VFO) without stopping the stream or the sound, and audio outputs.
//! Separate from the Android app for now; the same engine is meant to drive LittleSDR on
//! Android (through JNI) and on Linux.

pub mod audio;
pub mod audio_fx;
pub mod dsp;
pub mod engine;
pub mod source;
pub mod stereo;
