#pragma once
#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

namespace engine {
    // ------------------------------------------------------------------ FFT
    // Ring buffer of FFT lines (dB), filled by the DSP thread, read by the renderer.
    struct FftHistory {
        std::mutex mtx;
        int bins = 0;
        int rows = 0;
        std::vector<float> lines;  // rows * bins, ring
        std::vector<float> latest; // smoothed last line (spectrum trace)
        std::vector<float> peak;   // peak-hold line (valid when peakHold)
        bool peakHold = false;
        uint64_t written = 0;      // total lines written since (re)allocation
        uint64_t generation = 0;   // bumps when bins/rows change
    };
    FftHistory& fftHistory();

    // ------------------------------------------------------------------ lifecycle
    bool init(const std::string& rootDir, const std::string& cacheDir, int fftSize);
    bool isInitialized();

    // ------------------------------------------------------------------ sources
    std::vector<std::string> sourceNames();
    std::string selectedSource();
    void selectSource(const std::string& name);
    // SmGui menu of the selected source as JSON. If diffId is non-empty, the action is applied first.
    std::string sourceMenuJson(const std::string& diffId, int type, bool b, int i, float f, const std::string& s);

    // ------------------------------------------------------------------ playback
    bool start();
    void stop();
    bool isPlaying();

    // ------------------------------------------------------------------ tuning
    void tune(double freqHz);        // move the VFO (and the center if needed)
    void tuneCenter(double freqHz);  // move the hardware center frequency, VFO offset kept at 0
    double vfoFrequency();
    double centerFrequency();
    double sampleRate();             // effective (displayed) bandwidth in Hz
    double vfoOffset();
    double vfoBandwidth();
    void setView(double viewBandwidth, double viewOffset); // keep core tuner logic aware of the zoom

    // ------------------------------------------------------------------ demod
    void setMode(int mode);
    int mode();
    void setBandwidth(float hz);
    void setSquelch(bool enabled, float levelDb);
    bool squelchEnabled();
    float squelchLevel();
    void setVolume(float v);
    float volume();
    float signalLevelDb(); // peak dB inside the VFO passband
    // Occupied bandwidth of the signal under the VFO (FM), in [minBw, maxBw]; 0 if no signal
    float estimateBandwidth(double minBw, double maxBw);

    // ------------------------------------------------------------------ advanced options
    // All advanced options as one JSON object: { input:{...}, fft:{...}, audio:{...}, demod:{...} }
    std::string optionsJson();
    // Set one option, key like "input.decimation", "fft.size", "audio.latency", "demod.nb"
    void setOption(const std::string& key, double value);
    // Text options: "operator.location" (Maidenhead locator), "operator.callsign"
    void setOptionString(const std::string& key, const std::string& value);

    // ------------------------------------------------------------------ FT8 / FT4
    // { enabled, ft8:{processing,onFrequency,dial,...}, ft4:{...}, results:[...] }
    std::string ft8Json();
    // "enabled" (decoder module on/off), "ft8"/"ft4" (processing), "clear"
    void ft8Set(const std::string& key, double value);
    // Country position (CTY database) for each callsign: lat, lon pairs (NaN when unknown)
    std::vector<double> ctyLatLon(const std::vector<std::string>& calls);
}
