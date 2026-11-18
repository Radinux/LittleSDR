// LittleSDR build of SDR++Brown's recorder module (audio / baseband to WAV).
// Includes the module's main.cpp and adds C entry points for the engine: state as JSON, settings,
// recording folder. Baseband is recorded as a 500 kHz slice around the tuned frequency (the whole
// band of an RTL-SDR at 2.4 MS/s would be ~10 MB/s): a channel filter + resampler (RxVFO) sits
// between the IQ front end and the WAV writer. Upstream is unmodified.

// Same includes as the module's main.cpp, processed before the access override
#include <imgui.h>
#include <module.h>
#include <dsp/types.h>
#include <dsp/stream.h>
#include <dsp/bench/peak_level_meter.h>
#include <dsp/sink/handler_sink.h>
#include <dsp/routing/splitter.h>
#include <dsp/audio/volume.h>
#include <dsp/convert/stereo_to_mono.h>
#include <dsp/channel/rx_vfo.h>
#include <thread>
#include <ctime>
#include <gui/gui.h>
#include <filesystem>
#include <signal_path/signal_path.h>
#include <config.h>
#include <gui/style.h>
#include <gui/widgets/volume_meter.h>
#include <regex>
#include <gui/widgets/folder_select.h>
#include <recorder_interface.h>
#include <core.h>
#include <radio_interface.h>
#include <utils/optionlist.h>
#include <utils/wav.h>
#include <radio_module_interface.h>

#include <algorithm>
#include <cmath>
#include <cstring>
#include <memory>

#define private public
#include RECORDER_MAIN_CPP
#undef private

namespace {
    constexpr double BASEBAND_RATE = 500e3; // recorded IQ rate (and width) at most
    std::unique_ptr<dsp::channel::RxVFO> g_bbVfo; // set while a narrowed baseband recording runs
    double g_bbCenter = 0.0;

    // Recorded rate and absolute center frequency of a baseband recording started now
    void basebandPlan(double& rate, double& offset, double& center) {
        const double sr = sigpath::iqFrontEnd.getSampleRate();
        rate = std::min(sr, BASEBAND_RATE);
        offset = 0.0;
        auto it = gui::waterfall.vfos.find("Radio");
        if (it != gui::waterfall.vfos.end() && it->second) { offset = it->second->generalOffset; }
        const double maxOff = std::max(0.0, (sr - rate) / 2.0); // keep the slice inside the band
        offset = std::clamp(offset, -maxOff, maxOff);
        center = gui::waterfall.getCenterFrequency() + offset;
    }

    std::string stamp() {
        time_t now = time(nullptr);
        tm* t = localtime(&now);
        char buf[32];
        strftime(buf, sizeof buf, "%Y-%m-%d_%H-%M-%S", t);
        return buf;
    }

    void startBaseband(RecorderModule* m) {
        std::lock_guard<std::recursive_mutex> lck(m->recMtx);
        if (m->recording) { return; }
        double rate, offset, center;
        basebandPlan(rate, offset, center);
        if (rate <= 0) { return; }
        const double sr = sigpath::iqFrontEnd.getSampleRate();

        m->writer.setFormat(m->containers[m->containerId]);
        m->writer.setChannels(2);
        m->writer.setSampleType(m->sampleTypes[m->sampleTypeId]);
        m->writer.setSamplerate(rate);
        char name[160];
        snprintf(name, sizeof name, "/%s_baseband_%.0fHz_%.0fkHz.wav", stamp().c_str(), center, rate / 1000.0);
        std::string path = m->expandString(m->folderSelect.path + name);
        if (!m->writer.open(path)) {
            flog::error("Recorder: cannot open {}", path);
            return;
        }
        m->currentFile = path;
        m->samplerate = rate;
        g_bbCenter = center;

        m->basebandStream = new dsp::stream<dsp::complex_t>();
        if (rate < sr) {
            g_bbVfo = std::make_unique<dsp::channel::RxVFO>(m->basebandStream, sr, rate, rate * 0.9, offset);
            m->basebandSink.setInput(&g_bbVfo->out);
            g_bbVfo->start();
        }
        else {
            m->basebandSink.setInput(m->basebandStream);
        }
        m->basebandSink.start();
        sigpath::iqFrontEnd.bindIQStream(m->basebandStream);
        m->recording = true;
        flog::info("Recorder: baseband {} Hz, {} S/s -> {}", center, rate, path);
    }

    void stopRecording(RecorderModule* m) {
        std::lock_guard<std::recursive_mutex> lck(m->recMtx);
        if (!m->recording) { return; }
        if (m->recMode != RECORDER_MODE_BASEBAND || !g_bbVfo) {
            m->stop();
            return;
        }
        sigpath::iqFrontEnd.unbindIQStream(m->basebandStream);
        g_bbVfo->stop();
        m->basebandSink.stop();
        g_bbVfo.reset();
        delete m->basebandStream;
        m->basebandStream = nullptr;
        m->writer.close();
        m->recording = false;
    }

    std::string safeDump(const json& j) { return j.dump(-1, ' ', false, json::error_handler_t::replace); }
}

/**
 * {"recording", "mode" (0 baseband, 1 audio), "stereo", "file", "seconds", "bytes", "samplerate",
 *  "level": [l, r] dB, "path", "bbRate", "bbCenter"}
 */
extern "C" SDRPP_EXPORT int littlesdr_rec_json(void* instance, char* out, int cap) {
    auto* m = (RecorderModule*)instance;
    json j;
    {
        std::lock_guard<std::recursive_mutex> lck(m->recMtx);
        j["recording"] = m->recording;
        j["mode"] = m->recMode;
        j["stereo"] = m->stereo;
        j["path"] = m->folderSelect.path;
        j["file"] = m->recording ? m->currentFile : "";
        const double sr = m->samplerate > 0 ? (double)m->samplerate : 1.0;
        const size_t samples = m->recording ? m->writer.getSamplesWritten() : 0;
        j["seconds"] = samples / sr;
        j["samplerate"] = m->recording ? (double)m->samplerate : 0.0;
        std::error_code ec;
        j["bytes"] = m->recording ? (double)std::filesystem::file_size(m->currentFile, ec) : 0.0;
        if (ec) { j["bytes"] = 0.0; }
    }
    double rate, offset, center;
    basebandPlan(rate, offset, center);
    j["bbRate"] = rate;
    j["bbCenter"] = (m->recording && g_bbVfo) ? g_bbCenter : center;
    // Peak level of the audio stream since the last call
    dsp::stereo_t lvl = m->meter.getLevel();
    m->meter.resetLevel();
    auto db = [](float v) { return v > 1e-6f ? 20.0f * log10f(v) : -120.0f; };
    j["level"] = json::array({ db(lvl.l), db(lvl.r) });

    std::string s = safeDump(j);
    if ((int)s.size() >= cap) { s = "{}"; }
    memcpy(out, s.c_str(), s.size() + 1);
    return (int)s.size();
}

/** "mode" (0 baseband / 1 audio), "stereo" (audio file channels), "start", "stop". */
extern "C" SDRPP_EXPORT void littlesdr_rec_set(void* instance, const char* key, double v) {
    auto* m = (RecorderModule*)instance;
    if (!key) { return; }
    std::string k = key;
    if (k == "start") {
        if (m->recMode == RECORDER_MODE_BASEBAND) { startBaseband(m); }
        else { m->start(); }
        return;
    }
    if (k == "stop") {
        stopRecording(m);
        return;
    }
    std::lock_guard<std::recursive_mutex> lck(m->recMtx);
    if (m->recording) { return; } // fixed for the file being written
    if (k == "mode") {
        m->recMode = v != 0.0 ? RECORDER_MODE_AUDIO : RECORDER_MODE_BASEBAND;
        config.acquire();
        config.conf[m->name]["mode"] = m->recMode;
        config.release(true);
    }
    else if (k == "stereo") {
        m->stereo = v != 0.0;
        config.acquire();
        config.conf[m->name]["stereo"] = m->stereo;
        config.release(true);
    }
}

/** Folder for the recordings (the app's own storage), sortable file names. */
extern "C" SDRPP_EXPORT void littlesdr_rec_set_path(void* instance, const char* path) {
    auto* m = (RecorderModule*)instance;
    if (!path || !*path) { return; }
    std::lock_guard<std::recursive_mutex> lck(m->recMtx);
    std::filesystem::create_directories(path);
    m->folderSelect.setPath(path);
    strcpy(m->nameTemplate, "$y-$M-$d_$h-$m-$s_$t_$f_$r");
}
