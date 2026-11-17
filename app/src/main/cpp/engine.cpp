// Headless SDR++Brown engine: same init sequence as MainWindow::init(), minus
// everything that draws. The Kotlin UI drives it through jni_bridge.cpp.
#include "engine.h"
#include "smgui_json.h"

#include <core.h>
#include <config.h>
#include <module.h>
#include <imgui.h>
#include <gui/gui.h>
#include <gui/main_window.h>
#include <gui/tuner.h>
#include <gui/smgui.h>
#include <signal_path/signal_path.h>
#include <utils/flog.h>
#include <radio_interface.h>

#include <dlfcn.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <filesystem>

extern char* sdrppResourcesDirectory;
extern char* sdrppModulesDirectory;

namespace engine {
    namespace {
        constexpr const char* VFO_NAME = "Radio";
        constexpr int HISTORY_ROWS = 512; // ~20 s at 25 lines/s, 16 MB at 8192 bins

        std::recursive_mutex g_mtx; // serializes every control call ("UI thread")

        // FFT presentation options (DSP thread reads them in releaseFFTBuffer)
        std::atomic<float> g_smoothing{ 0.4f };   // weight of the previous trace (0 = none)
        std::atomic<float> g_peakDecayDb{ 10.0f }; // peak-hold fall, dB per second
        std::atomic<float> g_fftRate{ 25.0f };
        std::atomic<bool> g_initialized{ false };
        FftHistory g_fft;
        std::vector<float> g_fftScratch;
        dsp::stream<dsp::complex_t> g_dummyStream;
        EventHandler<VFOManager::VFO*> g_vfoCreatedHandler;

        struct ModuleDef { const char* instance; const char* module; };
        const ModuleDef MODULES[] = {
            { "RTL-SDR Source", "rtl_sdr_source" },
            { "Airspy Source", "airspy_source" },
            { "AirspyHF+ Source", "airspyhf_source" },
            { "HackRF Source", "hackrf_source" },
            { "RTL-TCP Source", "rtl_tcp_source" },
            { "SpyServer Source", "spyserver_source" },
            { "KiwiSDR Source", "kiwisdr_source" },
            { "Audio Sink", "audio_sink" },
            { "Radio", "radio" },
        };

        // ---------------------------------------------------------- FFT callbacks (DSP thread)
        // The FFT mutex is held from acquire to release (same DSP thread), so the buffers
        // can be resized (FFT size change) without the DSP writing into freed memory.
        float* acquireFFTBuffer(void*) {
            g_fft.mtx.lock();
            return g_fftScratch.data();
        }

        void releaseFFTBuffer(void*) {
            std::unique_lock<std::mutex> lck(g_fft.mtx, std::adopt_lock);
            if (g_fft.bins <= 0) { return; }
            const int bins = g_fft.bins;
            float* row = &g_fft.lines[(g_fft.written % g_fft.rows) * bins];
            memcpy(row, g_fftScratch.data(), bins * sizeof(float));
            // Exponential smoothing for the spectrum trace
            float* lat = g_fft.latest.data();
            const float a = g_smoothing.load();
            if (g_fft.written == 0 || a <= 0.0f) {
                memcpy(lat, row, bins * sizeof(float));
            }
            else {
                const float b = 1.0f - a;
                for (int i = 0; i < bins; i++) { lat[i] = lat[i] * a + row[i] * b; }
            }
            // Peak hold: rises instantly, falls at peakDecay dB/s
            if (g_fft.peakHold) {
                float* pk = g_fft.peak.data();
                const float fall = g_peakDecayDb.load() / std::max(1.0f, g_fftRate.load());
                for (int i = 0; i < bins; i++) { pk[i] = std::max(lat[i], pk[i] - fall); }
            }
            g_fft.written++;
        }

        void allocFft(int bins) {
            std::lock_guard<std::mutex> lck(g_fft.mtx);
            g_fft.bins = bins;
            g_fft.rows = HISTORY_ROWS;
            g_fft.lines.assign((size_t)bins * HISTORY_ROWS, -150.0f);
            g_fft.latest.assign(bins, -150.0f);
            g_fft.peak.assign(bins, -150.0f);
            g_fft.written = 0;
            g_fft.generation++;
            g_fftScratch.assign(bins, -150.0f);
        }

        // ---------------------------------------------------------- helpers
        ImGui::WaterfallVFO* radioVfo() {
            auto it = gui::waterfall.vfos.find(VFO_NAME);
            return it == gui::waterfall.vfos.end() ? nullptr : it->second;
        }

        void saveTuning() {
            core::configManager.acquire();
            core::configManager.conf["frequency"] = gui::waterfall.getCenterFrequency();
            if (auto vfo = radioVfo()) {
                core::configManager.conf["vfoOffsets"][VFO_NAME] = vfo->centerOffset;
            }
            core::configManager.release(true);
        }

        void onVfoCreated(VFOManager::VFO* vfo, void*) {
            std::string name = vfo->getName();
            core::configManager.acquire();
            bool has = core::configManager.conf["vfoOffsets"].contains(name);
            double offset = has ? (double)core::configManager.conf["vfoOffsets"][name] : 0.0;
            core::configManager.release();
            if (has) { sigpath::vfoManager.setCenterOffset(name, offset); }
        }

        json defaultConfig(const std::string& root) {
            json c;
            c["frequency"] = 100000000.0;
            c["source"] = "";
            c["decimation"] = 1;
            c["iqCorrection"] = false;
            c["invertIQ"] = false;
            c["offsetMode"] = 0;
            c["offset"] = 0.0;
            c["centerTuning"] = false;
            c["vfoOffsets"] = json::object();
            c["streams"]["Radio"]["muted"] = false;
            c["streams"]["Radio"]["sink"] = "Audio";
            c["streams"]["Radio"]["volume"] = 1.0f;
            c["modules"] = json::array();
            c["moduleInstances"] = json::object();
            c["modulesDirectory"] = root + "/modules";
            c["resourcesDirectory"] = root + "/res";
            c["operatorCallsign"] = "";
            c["operatorLocation"] = "";
            return c;
        }

        void setServerFlag(bool on) {
            core::args["server"] = CLIArg('s', "Run in server mode", on);
        }
    }

    void restoreOptions();
    FftHistory& fftHistory() { return g_fft; }
    bool isInitialized() { return g_initialized; }

    bool init(const std::string& rootDir, const std::string& cacheDir, int fftSize) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (g_initialized) { return true; }
        flog::info("LittleSDR engine init (root={}, fft={})", rootDir, fftSize);

        // Command line (root/temp dirs are read by modules through core::args)
        core::args.defineAll();
        std::string a0 = "littlesdr", a1 = "--root", a3 = "--temp";
        std::vector<char*> argv = { a0.data(), a1.data(), (char*)rootDir.c_str(), a3.data(), (char*)cacheDir.c_str() };
        if (core::args.parse((int)argv.size(), argv.data()) < 0) { flog::error("args parse failed"); return false; }
        std::filesystem::create_directories(rootDir);

        // Config
        json def = defaultConfig(rootDir);
        core::configManager.setPath(rootDir + "/config.json");
        core::configManager.load(def);
        core::configManager.enableAutoSave();
        core::configManager.acquire();
        for (auto const& item : def.items()) {
            if (!core::configManager.conf.contains(item.key())) { core::configManager.conf[item.key()] = item.value(); }
        }
        core::configManager.conf["modules"] = json::array();
        core::configManager.conf["moduleInstances"] = json::object();
        for (auto& m : MODULES) {
            core::configManager.conf["modules"].push_back(std::string(m.module) + ".so");
            core::configManager.conf["moduleInstances"][m.instance]["module"] = m.module;
            core::configManager.conf["moduleInstances"][m.instance]["enabled"] = true;
        }
        core::configManager.conf["streams"]["Radio"]["sink"] = "Audio";
        std::string resDir = core::configManager.conf["resourcesDirectory"];
        std::string modDir = core::configManager.conf["modulesDirectory"];
        double frequency = core::configManager.conf["frequency"];
        std::string savedSource = core::configManager.conf["source"];
        core::configManager.release(true);
        sdrppResourcesDirectory = strdup(resDir.c_str());
        sdrppModulesDirectory = strdup(modDir.c_str());

        // ImGui context without renderer: absorbs the few direct ImGui calls some
        // source menus still make (see smgui_json.cpp), never displayed.
        ImGui::CreateContext();
        ImGuiIO& io = ImGui::GetIO();
        io.IniFilename = nullptr;
        io.DisplaySize = ImVec2(1080, 2000);
        unsigned char* px; int w, h;
        io.Fonts->GetTexDataAsRGBA32(&px, &w, &h);

        // Source menus record into SmGui draw lists instead of drawing
        SmGui::init(true);

        // Waterfall object is only used as data (VFO list, center/bandwidth for the tuner)
        gui::waterfall.setBandwidth(DEFAULT_SAMPLE_RATE);
        gui::waterfall.setViewBandwidth(DEFAULT_SAMPLE_RATE);

        allocFft(fftSize);
        sigpath::iqFrontEnd.init(&g_dummyStream, DEFAULT_SAMPLE_RATE, true, 1, false, fftSize, 25.0,
                                 IQFrontEnd::FFTWindow::NUTTALL, acquireFFTBuffer, releaseFFTBuffer, nullptr);
        sigpath::iqFrontEnd.start();

        g_vfoCreatedHandler.handler = onVfoCreated;
        g_vfoCreatedHandler.ctx = nullptr;
        sigpath::vfoManager.onVfoCreated.bindHandler(&g_vfoCreatedHandler);

        // Modules
        for (auto& m : MODULES) {
            std::string file = std::string(m.module) + ".so";
            flog::info("Loading {}", file);
            core::moduleManager.loadModule(file);
        }
        for (auto& m : MODULES) {
            // rtl_sdr_source picks its SmGui gain slider (instead of raw ImGui) when
            // created in server mode. The flag must be off otherwise: it reroutes the IQ stream.
            bool rtl = strcmp(m.module, "rtl_sdr_source") == 0;
            if (rtl) { setServerFlag(true); }
            core::moduleManager.createInstance(m.instance, m.module);
            if (rtl) { setServerFlag(false); }
        }
        core::moduleManager.doPostInitAll();

        // Restore source and tuning
        auto names = sigpath::sourceManager.getSourceNames();
        if (!names.empty()) {
            std::string sel = std::find(names.begin(), names.end(), savedSource) != names.end() ? savedSource : names[0];
            sigpath::sourceManager.selectSource(sel);
        }
        gui::waterfall.setCenterFrequency(frequency);
        sigpath::sourceManager.tune(frequency);
        gui::waterfall.selectFirstVFO();

        restoreOptions();

        g_initialized = true;
        flog::info("LittleSDR engine ready, {} sources", (int)names.size());
        return true;
    }

    // ------------------------------------------------------------------ sources
    std::vector<std::string> sourceNames() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        return sigpath::sourceManager.getSourceNames();
    }

    std::string selectedSource() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        return sigpath::sourceManager.getSelectedName();
    }

    void selectSource(const std::string& name) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (gui::mainWindow.isPlaying()) { return; }
        sigpath::sourceManager.selectSource(name);
        core::configManager.acquire();
        core::configManager.conf["source"] = name;
        core::configManager.release(true);
    }

    std::string sourceMenuJson(const std::string& diffId, int type, bool b, int i, float f, const std::string& s) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        SmGui::DrawListElem diff;
        diff.type = (SmGui::DrawListElemType)type;
        diff.b = b; diff.i = i; diff.f = f; diff.str = s;
        return smguijson::renderSourceMenu(diffId, diff);
    }

    // ------------------------------------------------------------------ playback
    bool start() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (sigpath::sourceManager.getSelectedName().empty()) { return false; }
        gui::mainWindow.setPlayState(true);
        return gui::mainWindow.isPlaying();
    }

    void stop() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        gui::mainWindow.setPlayState(false);
        saveTuning();
    }

    bool isPlaying() { return gui::mainWindow.isPlaying(); }

    // ------------------------------------------------------------------ tuning
    // SDR++ normal tuning: at the band edge the hardware recenters and the VFO continues
    // from the other side (kept on purpose: the joystick makes this jump harmless).
    void tune(double freqHz) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        tuner::tune(tuner::TUNER_MODE_NORMAL, VFO_NAME, freqHz);
        saveTuning();
    }

    void tuneCenter(double freqHz) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        double off = vfoOffset();
        gui::waterfall.setCenterFrequency(freqHz);
        sigpath::sourceManager.tune(freqHz);
        if (radioVfo()) { sigpath::vfoManager.setOffset(VFO_NAME, off); }
        saveTuning();
    }

    double vfoFrequency() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        auto vfo = radioVfo();
        return gui::waterfall.getCenterFrequency() + (vfo ? vfo->generalOffset : 0.0);
    }

    double centerFrequency() { return gui::waterfall.getCenterFrequency(); }
    double sampleRate() { return gui::waterfall.getBandwidth(); }

    double vfoOffset() {
        auto vfo = radioVfo();
        return vfo ? vfo->generalOffset : 0.0;
    }

    double vfoBandwidth() {
        auto vfo = radioVfo();
        return vfo ? vfo->bandwidth : 0.0;
    }

    void setView(double viewBandwidth, double viewOffset) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        double bw = gui::waterfall.getBandwidth();
        viewBandwidth = std::clamp(viewBandwidth, 1000.0, bw);
        gui::waterfall.setViewBandwidth(viewBandwidth);
        gui::waterfall.setViewOffset(viewOffset);
    }

    // ------------------------------------------------------------------ demod
    void setMode(int m) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_MODE, &m, nullptr);
    }

    int mode() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        int m = -1;
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_GET_MODE, nullptr, &m);
        return m;
    }

    void setBandwidth(float hz) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_BANDWIDTH, &hz, nullptr);
    }

    void setSquelch(bool enabled, float levelDb) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_SQUELCH_ENABLED, &enabled, nullptr);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_SQUELCH_LEVEL, &levelDb, nullptr);
    }

    bool squelchEnabled() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        bool e = false;
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_GET_SQUELCH_ENABLED, nullptr, &e);
        return e;
    }

    float squelchLevel() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        float l = -100.0f;
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_GET_SQUELCH_LEVEL, nullptr, &l);
        return l;
    }

    void setVolume(float v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        auto it = sigpath::sinkManager.streams.find(VFO_NAME);
        if (it != sigpath::sinkManager.streams.end()) { it->second->setVolume(std::clamp(v, 0.0f, 1.0f)); }
    }

    float volume() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        auto it = sigpath::sinkManager.streams.find(VFO_NAME);
        return it != sigpath::sinkManager.streams.end() ? it->second->getVolume() : 0.0f;
    }

    float estimateBandwidth(double minBw, double maxBw) {
        double sr = gui::waterfall.getBandwidth();
        auto vfo = radioVfo();
        if (!vfo || sr <= 0) { return 0.0f; }
        std::vector<float> line;
        {
            std::lock_guard<std::mutex> lck(g_fft.mtx);
            if (g_fft.bins <= 0 || g_fft.written == 0) { return 0.0f; }
            line = g_fft.latest;
        }
        const int bins = (int)line.size();
        const double binHz = sr / bins;

        // Noise floor: 20th percentile of the whole span (robust to strong carriers)
        std::vector<float> sorted(line);
        std::nth_element(sorted.begin(), sorted.begin() + bins / 5, sorted.end());
        const float noise = sorted[bins / 5];

        // FM: the tuned frequency is the middle of the channel
        const int c = std::clamp((int)((vfo->generalOffset / sr + 0.5) * bins), 0, bins - 1);
        const int minHalf = std::max(1, (int)(minBw / 2.0 / binHz));
        const int maxHalf = std::max(minHalf, (int)(maxBw / 2.0 / binHz));

        float peak = -1e9f;
        for (int i = std::max(0, c - minHalf); i <= std::min(bins - 1, c + minHalf); i++) { peak = std::max(peak, line[i]); }
        if (peak - noise < 8.0f) { return 0.0f; } // nothing worth following

        const float thr = std::max(noise + 6.0f, peak - 30.0f);
        const int quietRun = std::max(2, (int)(1000.0 / binHz)); // ~1 kHz below threshold ends the signal
        auto edge = [&](int dir) {
            int quiet = 0, last = 0;
            for (int k = 1; k <= maxHalf; k++) {
                int i = c + dir * k;
                if (i < 0 || i >= bins) { break; }
                if (line[i] < thr) {
                    if (++quiet >= quietRun) { break; }
                }
                else {
                    quiet = 0;
                    last = k;
                }
            }
            return last;
        };
        const int half = std::max(edge(-1), edge(1)); // VFO stays centered: symmetric width
        double bw = 2.0 * (half + 1) * binHz * 1.15;
        return (float)std::clamp(bw, minBw, maxBw);
    }

    float signalLevelDb() {
        double sr = gui::waterfall.getBandwidth();
        auto vfo = radioVfo();
        if (!vfo || sr <= 0) { return -150.0f; }
        double lo = vfo->lowerOffset, hi = vfo->upperOffset;
        std::lock_guard<std::mutex> lck(g_fft.mtx);
        const int bins = g_fft.bins;
        if (bins <= 0 || g_fft.written == 0) { return -150.0f; }
        int b0 = std::clamp((int)((lo / sr + 0.5) * bins), 0, bins - 1);
        int b1 = std::clamp((int)((hi / sr + 0.5) * bins), 0, bins - 1);
        float peak = -150.0f;
        for (int b = b0; b <= b1; b++) { peak = std::max(peak, g_fft.latest[b]); }
        return peak;
    }
    // ================================================================== advanced options
    namespace {
        // Our options live under "littlesdr" in the core config (decimation/iqCorrection/
        // invertIQ keep their upstream keys so the core reads them the usual way)
        json& opts() { return core::configManager.conf["littlesdr"]; }

        template <class T>
        T optGet(const char* key, T def) {
            json& o = opts();
            return (o.is_object() && o.contains(key)) ? o[key].get<T>() : def;
        }

        void optPut(const char* key, double v) {
            core::configManager.acquire();
            if (!core::configManager.conf["littlesdr"].is_object()) { core::configManager.conf["littlesdr"] = json::object(); }
            core::configManager.conf["littlesdr"][key] = v;
            core::configManager.release(true);
        }

        // ---- radio extension (modules/radio_ext.cpp, compiled into radio.so)
        using RadioOptionsFn = int (*)(void*, char*, int);
        using RadioSetFn = void (*)(void*, const char*, double);

        void* radioInstance() {
            auto it = core::moduleManager.instances.find(VFO_NAME);
            return it == core::moduleManager.instances.end() ? nullptr : (void*)it->second.instance;
        }

        void* radioSymbol(const char* sym) {
            auto it = core::moduleManager.modules.find("radio");
            if (it == core::moduleManager.modules.end() || !it->second.handle) { return nullptr; }
            return dlsym(it->second.handle, sym);
        }

        // ---- our audio sink (modules/aaudio_sink): interface "Audio Sink"
        enum { AUDIO_CMD_SET_DEVICE = 0, AUDIO_CMD_SET_LATENCY = 1 };

        int g_fftSize = 8192;
        int g_fftWindow = 2; // NUTTALL

        void applyFftSize(int size) {
            size = std::clamp(size, 1024, 65536);
            {
                // Allocate the new buffers before the frontend produces lines of the new size
                std::lock_guard<std::mutex> lck(g_fft.mtx);
                g_fftScratch.assign(std::max<size_t>(g_fftScratch.size(), (size_t)size), -150.0f);
            }
            sigpath::iqFrontEnd.setFFTSize(size);
            allocFft(size);
            g_fftSize = size;
        }
    }

    void restoreOptions() {
        core::configManager.acquire();
        double offset = optGet<double>("offset", 0.0);
        int fftSize = optGet<int>("fftSize", g_fftSize);
        float fftRate = optGet<float>("fftRate", 25.0f);
        int fftWindow = optGet<int>("fftWindow", 2);
        float smoothing = optGet<float>("smoothing", 0.4f);
        bool peakHold = optGet<double>("peakHold", 0.0) != 0.0;
        float peakDecay = optGet<float>("peakDecay", 10.0f);
        int audioDevice = optGet<int>("audioDevice", 0);
        int audioLatency = optGet<int>("audioLatency", 4);
        int decimation = core::configManager.conf["decimation"];
        bool dc = core::configManager.conf["iqCorrection"];
        bool inv = core::configManager.conf["invertIQ"];
        core::configManager.release();

        sigpath::iqFrontEnd.setDCBlocking(dc);
        sigpath::iqFrontEnd.setInvertIQ(inv);
        if (decimation > 1) { sigpath::iqFrontEnd.setDecimation(decimation); }
        sigpath::sourceManager.setTuningOffset(offset);
        if (fftSize != g_fftSize) { applyFftSize(fftSize); }
        g_fftRate = fftRate;
        sigpath::iqFrontEnd.setFFTRate(fftRate);
        g_fftWindow = fftWindow;
        sigpath::iqFrontEnd.setFFTWindow((IQFrontEnd::FFTWindow)fftWindow);
        g_smoothing = smoothing;
        g_peakDecayDb = peakDecay;
        {
            std::lock_guard<std::mutex> lck(g_fft.mtx);
            g_fft.peakHold = peakHold;
        }
        core::modComManager.callInterface("Audio Sink", AUDIO_CMD_SET_DEVICE, &audioDevice, nullptr);
        core::modComManager.callInterface("Audio Sink", AUDIO_CMD_SET_LATENCY, &audioLatency, nullptr);
    }

    std::string optionsJson() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        json j;
        core::configManager.acquire();
        j["input"]["decimation"] = (int)core::configManager.conf["decimation"];
        j["input"]["dcBlocking"] = (bool)core::configManager.conf["iqCorrection"];
        j["input"]["invertIQ"] = (bool)core::configManager.conf["invertIQ"];
        j["input"]["offset"] = optGet<double>("offset", 0.0);
        j["fft"]["size"] = g_fftSize;
        j["fft"]["rate"] = g_fftRate.load();
        j["fft"]["window"] = g_fftWindow;
        j["fft"]["smoothing"] = g_smoothing.load();
        j["fft"]["peakDecay"] = g_peakDecayDb.load();
        j["audio"]["device"] = optGet<int>("audioDevice", 0);
        j["audio"]["latency"] = optGet<int>("audioLatency", 4);
        core::configManager.release();
        {
            std::lock_guard<std::mutex> l2(g_fft.mtx);
            j["fft"]["peakHold"] = g_fft.peakHold;
        }
        j["demod"] = json::object();
        if (auto fn = (RadioOptionsFn)radioSymbol("littlesdr_radio_options")) {
            if (void* inst = radioInstance()) {
                std::vector<char> buf(8192);
                fn(inst, buf.data(), (int)buf.size());
                j["demod"] = json::parse(buf.data(), nullptr, false);
            }
        }
        return j.dump();
    }

    void setOption(const std::string& key, double v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (key == "input.decimation") {
            int d = 1;
            while (d < 64 && d < (int)v) { d *= 2; } // powers of two only
            sigpath::iqFrontEnd.setDecimation(d);
            core::configManager.acquire();
            core::configManager.conf["decimation"] = d;
            core::configManager.release(true);
        }
        else if (key == "input.dcBlocking") {
            sigpath::iqFrontEnd.setDCBlocking(v != 0.0);
            core::configManager.acquire();
            core::configManager.conf["iqCorrection"] = v != 0.0;
            core::configManager.release(true);
        }
        else if (key == "input.invertIQ") {
            sigpath::iqFrontEnd.setInvertIQ(v != 0.0);
            core::configManager.acquire();
            core::configManager.conf["invertIQ"] = v != 0.0;
            core::configManager.release(true);
        }
        else if (key == "input.offset") {
            sigpath::sourceManager.setTuningOffset(v);
            sigpath::sourceManager.tune(gui::waterfall.getCenterFrequency());
            optPut("offset", v);
        }
        else if (key == "fft.size") {
            applyFftSize((int)v);
            optPut("fftSize", g_fftSize);
        }
        else if (key == "fft.rate") {
            g_fftRate = std::clamp((float)v, 5.0f, 60.0f);
            sigpath::iqFrontEnd.setFFTRate(g_fftRate);
            optPut("fftRate", g_fftRate);
        }
        else if (key == "fft.window") {
            g_fftWindow = std::clamp((int)v, 0, 2);
            sigpath::iqFrontEnd.setFFTWindow((IQFrontEnd::FFTWindow)g_fftWindow);
            optPut("fftWindow", g_fftWindow);
        }
        else if (key == "fft.smoothing") {
            g_smoothing = std::clamp((float)v, 0.0f, 0.95f);
            optPut("smoothing", g_smoothing);
        }
        else if (key == "fft.peakHold") {
            {
                std::lock_guard<std::mutex> l2(g_fft.mtx);
                g_fft.peakHold = v != 0.0;
                std::fill(g_fft.peak.begin(), g_fft.peak.end(), -150.0f);
            }
            optPut("peakHold", v != 0.0 ? 1.0 : 0.0);
        }
        else if (key == "fft.peakDecay") {
            g_peakDecayDb = std::clamp((float)v, 1.0f, 60.0f);
            optPut("peakDecay", g_peakDecayDb);
        }
        else if (key == "audio.device") {
            int id = (int)v;
            core::modComManager.callInterface("Audio Sink", AUDIO_CMD_SET_DEVICE, &id, nullptr);
            optPut("audioDevice", id);
        }
        else if (key == "audio.latency") {
            int bursts = std::clamp((int)v, 2, 16);
            core::modComManager.callInterface("Audio Sink", AUDIO_CMD_SET_LATENCY, &bursts, nullptr);
            optPut("audioLatency", bursts);
        }
        else if (key.rfind("demod.", 0) == 0) {
            if (auto fn = (RadioSetFn)radioSymbol("littlesdr_radio_set")) {
                if (void* inst = radioInstance()) { fn(inst, key.c_str() + 6, v); }
            }
        }
    }

}
