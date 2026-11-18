// LittleSDR build of SDR++'s SpyServer source module.
// The module class lives in its main.cpp (no header), so this translation unit includes it and
// adds what LittleSDR needs; CMakeLists.txt builds the module from this file instead of main.cpp.
// Upstream is unmodified.
//
//  - The "Connected (device)" status is drawn with raw ImGui::TextColored, which never reaches
//    SmGui's draw list (no GUI here): it is redirected to SmGui::TextColored.
//  - littlesdr_spyserver_adjust(): first connection to a server picks an internet-friendly
//    stream (<= 400 kS/s, 100 kS/s for an Airspy HF+, 8-bit) instead of the full rate (10 MS/s on an Airspy = ~40 MB/s).

// Same includes as the module's main.cpp, processed before the overrides below
#include <spyserver_client.h>
#include <imgui.h>
#include <utils/flog.h>
#include <module.h>
#include <gui/gui.h>
#include <signal_path/signal_path.h>
#include <core.h>
#include <gui/style.h>
#include <config.h>
#include <gui/widgets/stepped_slider.h>
#include <gui/smgui.h>
#include <cstdarg>
#include <cstdio>

namespace ImGui {
    inline void LittleSdrTextColored(const ImVec4& col, const char* fmt, ...) {
        char buf[512];
        va_list args;
        va_start(args, fmt);
        vsnprintf(buf, sizeof buf, fmt, args);
        va_end(args);
        SmGui::TextColored(col, buf);
    }
}

#define TextColored LittleSdrTextColored
#define private public
#include SPYSERVER_MAIN_CPP
#undef private
#undef TextColored

namespace {
    // ~0.6 MB/s in 8-bit: what a phone on an ordinary Wi-Fi sustains (measured: a phone got
    // 0.6 MB/s where a PC on the same network got the 1.25 MB/s of 625 kS/s). Still fits a WFM station.
    constexpr double NET_MAX_RATE = 400e3;
    // Airspy HF+ servers are mostly used on HF (AM/SSB/CW) and often sit on modest home uplinks:
    // measured, several public ones only sustain ~0.2 MB/s, i.e. 96 kS/s in 8-bit
    constexpr double NET_MAX_RATE_HF = 100e3;
    constexpr int NET_IQ_TYPE = 0; // UInt8
    // Bumped when the defaults change, so they are applied again once
    constexpr int NET_DEFAULTS_VERSION = 4;
    const char* const NET_DEFAULTS_KEY = "littlesdrDefaults";

    bool defaultsApplied(json& dev) {
        return dev.contains(NET_DEFAULTS_KEY) && dev[NET_DEFAULTS_KEY].is_number() && dev[NET_DEFAULTS_KEY].get<int>() >= NET_DEFAULTS_VERSION;
    }
}

/**
 * 1: the stream settings were changed. -1: they need changing but the stream is running (the
 * caller stops playback, calls again, restarts). 0: nothing to do.
 * Called after each source-menu render and when playback starts.
 */
extern "C" SDRPP_EXPORT int littlesdr_spyserver_adjust(void* instance) {
    auto* m = (SpyServerSourceModule*)instance;
    if (!m || !m->client || !m->client->isOpen() || m->devRef.empty() || m->sampleRates.empty()) {
        return 0;
    }
    if (m->running) {
        config.acquire();
        bool done = defaultsApplied(config.conf["devices"][m->devRef]);
        config.release(false);
        return done ? 0 : -1;
    }
    config.acquire();
    json& dev = config.conf["devices"][m->devRef];
    bool done = defaultsApplied(dev);
    if (!done) {
        dev[NET_DEFAULTS_KEY] = NET_DEFAULTS_VERSION;
        double maxRate = m->client->devInfo.DeviceType == SPYSERVER_DEVICE_AIRSPY_HF ? NET_MAX_RATE_HF : NET_MAX_RATE;
        // Highest rate that fits (rates are listed from the fastest)
        int id = (int)m->sampleRates.size() - 1;
        for (int i = 0; i < (int)m->sampleRates.size(); i++) {
            if (m->sampleRates[i] <= maxRate) { id = i; break; }
        }
        m->srId = id;
        m->sampleRate = m->sampleRates[id];
        m->iqType = NET_IQ_TYPE;
        dev["sampleRateId"] = m->srId;
        dev["sampleBitDepthId"] = m->iqType;
    }
    config.release(!done);
    if (done) { return 0; }
    core::setInputSampleRate(m->sampleRate);
    flog::info("SpyServer: network defaults {} S/s, 8-bit", m->sampleRate);
    return 1;
}

namespace {
    // Moves to another entry of the rate list (stream stopped), saved for this server
    int setRateIndex(SpyServerSourceModule* m, int id) {
        if (!m || m->running || !m->client || m->devRef.empty() || m->sampleRates.empty()) { return 0; }
        if (id < 0 || id >= (int)m->sampleRates.size() || id == m->srId) { return 0; }
        m->srId = id;
        m->sampleRate = m->sampleRates[id];
        config.acquire();
        config.conf["devices"][m->devRef]["sampleRateId"] = m->srId;
        config.release(true);
        core::setInputSampleRate(m->sampleRate);
        flog::info("SpyServer: rate {} S/s", m->sampleRate);
        return 1;
    }
}

/** Server too slow (see engine::spyServerMonitor): next lower rate. Stream must be stopped. */
extern "C" SDRPP_EXPORT int littlesdr_spyserver_step_down(void* instance) {
    auto* m = (SpyServerSourceModule*)instance;
    return m ? setRateIndex(m, m->srId + 1) : 0;
}

/** Stream fine for a while after a step down: try the next higher rate. Stream must be stopped. */
extern "C" SDRPP_EXPORT int littlesdr_spyserver_step_up(void* instance) {
    auto* m = (SpyServerSourceModule*)instance;
    return m ? setRateIndex(m, m->srId - 1) : 0;
}

/** Identifies the server the module points at (host:port), to reset the engine's monitor. */
extern "C" SDRPP_EXPORT uint64_t littlesdr_spyserver_key(void* instance) {
    auto* m = (SpyServerSourceModule*)instance;
    if (!m) { return 0; }
    return (uint64_t)std::hash<std::string>()(std::string(m->hostname) + ":" + std::to_string(m->port));
}
