// Headless SDR++Brown engine: same init sequence as MainWindow::init(), minus
// everything that draws. The Kotlin UI drives it through jni_bridge.cpp.
#include "engine.h"
#include "smgui_json.h"
#include "sstv/sstv_decoder.h"

#include <core.h>
#include <config.h>
#include <module.h>
#include <imgui.h>
#include <gui/gui.h>
#include <gui/main_window.h>
#include <gui/tuner.h>
#include <gui/smgui.h>
#include <gui/style.h>
#include <signal_path/signal_path.h>
#include <utils/flog.h>
#include <chrono>
#include <utils/cty.h>
#include <radio_interface.h>

#include <dlfcn.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <thread>

extern char* sdrppResourcesDirectory;
extern char* sdrppModulesDirectory;

namespace engine {
    namespace {
        constexpr const char* VFO_NAME = "Radio";
        constexpr int HISTORY_ROWS = 512; // ~20 s at 25 lines/s, 16 MB at 8192 bins

        std::recursive_mutex g_mtx; // serializes every control call ("UI thread")

        // Frequency the user tuned to (absolute, Hz). SDR++ clamps the VFO into the band whenever the
        // source sample rate changes (e.g. RTL-SDR selected before its USB device is available, so
        // with a near-zero band): we re-tune to this after such changes. Only user tuning updates it.
        double g_wantedVfo = 0.0;
        double g_lastBandwidth = 0.0;

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
            { "FT8/FT4 Decoder", "ft8_decoder" }, // created disabled, enabled from the FT8 screen
            { "TinyCW Decoder", "tinycw_decoder" }, // created disabled, enabled from the Morse tab
            { "Recorder", "recorder" },
        };
        constexpr const char* FT8_INSTANCE = "FT8/FT4 Decoder";
        constexpr const char* CW_INSTANCE = "TinyCW Decoder";
        // Decoders cost CPU during reception: always created off, turned on explicitly
        bool startsDisabled(const char* instance) {
            return strcmp(instance, FT8_INSTANCE) == 0 || strcmp(instance, CW_INSTANCE) == 0;
        }

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
            if (g_wantedVfo > 0) {
                if (!core::configManager.conf["littlesdr"].is_object()) { core::configManager.conf["littlesdr"] = json::object(); }
                core::configManager.conf["littlesdr"]["vfoFrequency"] = g_wantedVfo;
            }
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
        // The FT8 decoder always starts off: it costs CPU during reception, so it is only
        // turned on explicitly from the FT8 screen (never silently at app start).
        const bool ft8Enabled = false;
        for (auto& m : MODULES) {
            core::configManager.conf["modules"].push_back(std::string(m.module) + ".so");
            core::configManager.conf["moduleInstances"][m.instance]["module"] = m.module;
            core::configManager.conf["moduleInstances"][m.instance]["enabled"] =
                startsDisabled(m.instance) ? ft8Enabled : true;
        }
        core::configManager.conf["streams"]["Radio"]["sink"] = "Audio";
        std::string resDir = core::configManager.conf["resourcesDirectory"];
        std::string modDir = core::configManager.conf["modulesDirectory"];
        double frequency = core::configManager.conf["frequency"];
        std::string savedSource = core::configManager.conf["source"];
        std::string operatorLocation = core::configManager.conf["operatorLocation"];
        std::string operatorCallsign = core::configManager.conf["operatorCallsign"];
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
        // SDR++'s fonts are never loaded here (no GUI); some module menus still read them, e.g.
        // KiwiSDR's map button uses style::baseFont->FontSize. Point them all at the default font.
        ImFont* defaultFont = io.Fonts->Fonts.empty() ? nullptr : io.Fonts->Fonts[0];
        style::notificationFont = style::tinyFont = style::baseFont = defaultFont;
        style::bigFont = style::mediumFont = style::hugeFont = defaultFont;

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

        // Country database (FT8 decodes) and operator position (distances)
        if (std::filesystem::exists(resDir + "/cty/cty.dat")) {
            core::configManager.acquire();
            utils::loadAllCty();
            core::configManager.release();
        }
        sigpath::iqFrontEnd.operatorLocation = operatorLocation;
        sigpath::iqFrontEnd.operatorCallsign = operatorCallsign;

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
            if (startsDisabled(m.instance)) { core::moduleManager.disableInstance(m.instance); }
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
        core::configManager.acquire();
        {
            auto& o = core::configManager.conf["littlesdr"];
            g_wantedVfo = (o.is_object() && o.contains("vfoFrequency")) ? (double)o["vfoFrequency"] : 0.0;
        }
        core::configManager.release();
        if (g_wantedVfo <= 0) { g_wantedVfo = vfoFrequency(); }
        g_lastBandwidth = 0.0; // force the first syncVfo() to check the restored VFO
        syncVfo(); // the radio module set its reference/bandwidth after the VFO offset was restored

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

    bool spyServerAdjust();

    std::string sourceMenuJson(const std::string& diffId, int type, bool b, int i, float f, const std::string& s) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        SmGui::DrawListElem diff;
        diff.type = (SmGui::DrawListElemType)type;
        diff.b = b; diff.i = i; diff.f = f; diff.str = s;
        std::string out = smguijson::renderSourceMenu(diffId, diff);
        if (spyServerAdjust()) {
            SmGui::DrawListElem none;
            out = smguijson::renderSourceMenu("", none);
        }
        return out;
    }

    // SDR++ changes some VFO offsets on the waterfall side only (reference / bandwidth changes,
    // e.g. switching to USB) and lets the GUI loop copy them to the DSP every frame through
    // updateFromWaterfall(). There is no such loop here, so without this the display (waterfall
    // data) and the audio (DSP VFO) drift apart, and the drift survives restarts.
    void spyServerMonitor();

    void syncVfo() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        sigpath::vfoManager.updateFromWaterfall(&gui::waterfall);

        // Band changed (sample rate, source, USB device appearing): put the VFO back where the user left it
        double bw = gui::waterfall.getBandwidth();
        if (bw != g_lastBandwidth) {
            g_lastBandwidth = bw;
            if (g_wantedVfo > 0 && radioVfo() && std::abs(vfoFrequency() - g_wantedVfo) > 1.0) {
                tuner::tune(tuner::TUNER_MODE_NORMAL, VFO_NAME, g_wantedVfo);
                sigpath::vfoManager.updateFromWaterfall(&gui::waterfall);
                saveTuning();
            }
        }
    }

    // ------------------------------------------------------------------ playback
    // SpyServer (modules/spyserver_ext.cpp): the first connection to a server switches to an
    // internet-friendly stream; when that happens while playing, playback is restarted.
    bool spyServerAdjust() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (sigpath::sourceManager.getSelectedName() != "SpyServer Source") { return false; }
        auto mod = core::moduleManager.modules.find("spyserver_source");
        auto inst = core::moduleManager.instances.find("SpyServer Source");
        if (mod == core::moduleManager.modules.end() || !mod->second.handle || inst == core::moduleManager.instances.end()) { return false; }
        using AdjustFn = int (*)(void*);
        auto adjust = (AdjustFn)dlsym(mod->second.handle, "littlesdr_spyserver_adjust");
        if (!adjust) { return false; }
        int r = adjust(inst->second.instance);
        if (r == -1) {
            gui::mainWindow.setPlayState(false);
            r = adjust(inst->second.instance);
            gui::mainWindow.setPlayState(true);
        }
        return r != 0;
    }

    // SpyServer automatic rate. When the server (or the network) delivers less than the chosen
    // sample rate, FFT lines arrive slower than g_fftRate (the frontend makes them from the
    // samples): two consecutive 3 s windows below 75 % -> next lower rate, playback restarted.
    // Cautious way back up: after g_spyUpDelay seconds of clean stream, try one rate higher
    // (only to undo our own steps down). A step up that fails again doubles the delay.
    namespace {
        constexpr double SPY_WINDOW_S = 3.0;
        constexpr double SPY_UP_DELAY_S = 60.0;
        constexpr double SPY_UP_DELAY_MAX_S = 600.0;
        constexpr double SPY_UP_PROBATION_S = 30.0;

        std::chrono::steady_clock::time_point g_spyWindowStart;
        uint64_t g_spyWindowLines = 0;
        int g_spySlowWindows = 0;
        bool g_spyWarmup = true;
        int g_spyStepsDown = 0;          // our own steps below the server's saved rate
        double g_spyCleanSecs = 0.0;     // clean stream since the last change
        double g_spyUpDelay = SPY_UP_DELAY_S;
        bool g_spyOnProbation = false;   // just stepped up: a quick failure means "not yet"
        uint64_t g_spyServerKey = 0;

        void* spySymbol(const char* name) {
            auto mod = core::moduleManager.modules.find("spyserver_source");
            if (mod == core::moduleManager.modules.end() || !mod->second.handle) { return nullptr; }
            return dlsym(mod->second.handle, name);
        }

        void* spyInstance() {
            auto inst = core::moduleManager.instances.find("SpyServer Source");
            return inst == core::moduleManager.instances.end() ? nullptr : (void*)inst->second.instance;
        }
    }

    void spyServerResetWindow() {
        g_spyWindowStart = std::chrono::steady_clock::now();
        { std::lock_guard<std::mutex> l(g_fft.mtx); g_spyWindowLines = g_fft.written; }
        g_spySlowWindows = 0;
        g_spyCleanSecs = 0.0;
        g_spyWarmup = true; // the first window after a (re)start includes connection and buffering
    }

    // Changes the rate with the stream stopped (the sample rate can't change while running)
    bool spyServerChangeRate(const char* fn) {
        auto change = (int (*)(void*))spySymbol(fn);
        void* inst = spyInstance();
        if (!change || !inst) { return false; }
        gui::mainWindow.setPlayState(false);
        bool changed = change(inst) != 0;
        gui::mainWindow.setPlayState(true);
        spyServerResetWindow();
        return changed;
    }

    void spyServerMonitor() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (!gui::mainWindow.isPlaying() || sigpath::sourceManager.getSelectedName() != "SpyServer Source") {
            g_spyWindowStart = {};
            return;
        }
        // Another server: our steps down don't apply to it
        auto keyFn = (uint64_t (*)(void*))spySymbol("littlesdr_spyserver_key");
        uint64_t key = (keyFn && spyInstance()) ? keyFn(spyInstance()) : 0;
        if (key != g_spyServerKey) {
            g_spyServerKey = key;
            g_spyStepsDown = 0;
            g_spyUpDelay = SPY_UP_DELAY_S;
            g_spyOnProbation = false;
        }

        auto now = std::chrono::steady_clock::now();
        if (g_spyWindowStart == std::chrono::steady_clock::time_point{}) { spyServerResetWindow(); return; }
        double secs = std::chrono::duration<double>(now - g_spyWindowStart).count();
        if (secs < SPY_WINDOW_S) { return; }
        uint64_t written;
        { std::lock_guard<std::mutex> l(g_fft.mtx); written = g_fft.written; }
        double ratio = written >= g_spyWindowLines
            ? (double)(written - g_spyWindowLines) / secs / std::max(1.0f, g_fftRate.load()) : 1.0;
        g_spyWindowStart = now;
        g_spyWindowLines = written;
        if (g_spyWarmup) { g_spyWarmup = false; return; }

        if (ratio < 0.75) {
            g_spyCleanSecs = 0.0;
            if (++g_spySlowWindows < 2) { return; }
            flog::warn("SpyServer: receiving {}% of the stream", (int)(ratio * 100));
            if (g_spyOnProbation) {
                // The higher rate still doesn't fit: wait longer before the next try
                g_spyUpDelay = std::min(g_spyUpDelay * 2.0, SPY_UP_DELAY_MAX_S);
                g_spyOnProbation = false;
            }
            if (spyServerChangeRate("littlesdr_spyserver_step_down")) { g_spyStepsDown++; }
            return;
        }
        g_spySlowWindows = 0;
        if (ratio < 0.95) { return; } // borderline: neither worse nor clean
        g_spyCleanSecs += secs;
        if (g_spyOnProbation && g_spyCleanSecs >= SPY_UP_PROBATION_S) {
            g_spyOnProbation = false; // the step up held
        }
        if (g_spyStepsDown > 0 && !g_spyOnProbation && g_spyCleanSecs >= g_spyUpDelay) {
            if (spyServerChangeRate("littlesdr_spyserver_step_up")) {
                g_spyStepsDown--;
                g_spyOnProbation = true;
            }
        }
    }

    bool start() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (sigpath::sourceManager.getSelectedName().empty()) { return false; }
        gui::mainWindow.setPlayState(true);
        spyServerAdjust(); // playing without "Connect" first connects here
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
        g_wantedVfo = freqHz;
        tuner::tune(tuner::TUNER_MODE_NORMAL, VFO_NAME, freqHz);
        syncVfo();
        saveTuning();
    }

    void tuneCenter(double freqHz) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        double off = vfoOffset();
        gui::waterfall.setCenterFrequency(freqHz);
        sigpath::sourceManager.tune(freqHz);
        if (radioVfo()) { sigpath::vfoManager.setOffset(VFO_NAME, off); }
        syncVfo();
        g_wantedVfo = vfoFrequency();
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
    int mode() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        int m = -1;
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_GET_MODE, nullptr, &m);
        return m;
    }

    namespace {
        // TinyCW follows the tuned frequency only when the main window says "CW": that window is
        // not used here, so mirror the radio's mode into it
        void syncMainWindowMode() {
            int m = mode();
            const char* name = m == 5 ? "CW" : (m == 0 || m == 1) ? "FM" : (m == 4 || m == 6) ? "SSB" : "AM";
            if (gui::mainWindow.getCurrentMode() != name) { gui::mainWindow.setCurrentMode(name); }
        }
    }

    void setMode(int m) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_MODE, &m, nullptr);
        syncVfo();
        // Every mode change, Morse tab shown or not: TinyCW starts/stops the tuned-frequency decoder
        syncMainWindowMode();
    }

    void setBandwidth(float hz) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        core::modComManager.callInterface(VFO_NAME, RADIO_IFACE_CMD_SET_BANDWIDTH, &hz, nullptr);
        syncVfo();
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
        j["operator"]["location"] = (std::string)core::configManager.conf["operatorLocation"];
        j["operator"]["callsign"] = (std::string)core::configManager.conf["operatorCallsign"];
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
        return j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace);
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

    // ================================================================== text options
    namespace { void* moduleSymbol(const char* module, const char* sym); }

    void setOptionString(const std::string& key, const std::string& value) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (key == "operator.location") {
            sigpath::iqFrontEnd.operatorLocation = value;
            core::configManager.acquire();
            core::configManager.conf["operatorLocation"] = value;
            core::configManager.release(true);
        }
        else if (key == "recorder.path") {
            using Fn = void (*)(void*, const char*);
            auto it = core::moduleManager.instances.find("Recorder");
            if (auto fn = (Fn)moduleSymbol("recorder", "littlesdr_rec_set_path")) {
                if (it != core::moduleManager.instances.end()) { fn(it->second.instance, value.c_str()); }
            }
        }
        else if (key == "operator.callsign") {
            sigpath::iqFrontEnd.operatorCallsign = value;
            core::configManager.acquire();
            core::configManager.conf["operatorCallsign"] = value;
            core::configManager.release(true);
        }
    }

    // ================================================================== FT8 / FT4
    namespace {
        void* moduleSymbol(const char* module, const char* sym) {
            auto it = core::moduleManager.modules.find(module);
            if (it == core::moduleManager.modules.end() || !it->second.handle) { return nullptr; }
            return dlsym(it->second.handle, sym);
        }

        void* ft8Instance() {
            auto it = core::moduleManager.instances.find(FT8_INSTANCE);
            return it == core::moduleManager.instances.end() ? nullptr : (void*)it->second.instance;
        }
    }

    std::string ft8Json() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        using Fn = int (*)(void*, char*, int);
        auto fn = (Fn)moduleSymbol("ft8_decoder", "littlesdr_ft8_json");
        void* inst = ft8Instance();
        if (!fn || !inst) { return "{}"; }
        std::vector<char> buf(256 * 1024);
        fn(inst, buf.data(), (int)buf.size());
        return std::string(buf.data());
    }

    void ft8Set(const std::string& key, double v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (key == "enabled") {
            bool on = v != 0.0;
            if (on) { core::moduleManager.enableInstance(FT8_INSTANCE); }
            else { core::moduleManager.disableInstance(FT8_INSTANCE); }
            return;
        }
        using Fn = void (*)(void*, const char*, double);
        if (auto fn = (Fn)moduleSymbol("ft8_decoder", "littlesdr_ft8_set")) {
            if (void* inst = ft8Instance()) { fn(inst, key.c_str(), v); }
        }
    }

    // ------------------------------------------------------------------ Morse (TinyCW)
    namespace {
        void* cwInstance() {
            auto it = core::moduleManager.instances.find(CW_INSTANCE);
            return it == core::moduleManager.instances.end() ? nullptr : (void*)it->second.instance;
        }

    }

    std::string cwJson() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        syncMainWindowMode();
        using Fn = int (*)(void*, char*, int);
        auto fn = (Fn)moduleSymbol("tinycw_decoder", "littlesdr_cw_json");
        void* inst = cwInstance();
        if (!fn || !inst) { return "{}"; }
        std::vector<char> buf(256 * 1024);
        fn(inst, buf.data(), (int)buf.size());
        return std::string(buf.data());
    }

    void cwSet(const std::string& key, double v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (key == "enabled") {
            if (v != 0.0) { syncMainWindowMode(); core::moduleManager.enableInstance(CW_INSTANCE); }
            else { core::moduleManager.disableInstance(CW_INSTANCE); }
            return;
        }
        using Fn = void (*)(void*, const char*, double);
        if (auto fn = (Fn)moduleSymbol("tinycw_decoder", "littlesdr_cw_set")) {
            if (void* inst = cwInstance()) { fn(inst, key.c_str(), v); }
        }
    }

    // ------------------------------------------------------------------ KiwiSDR
    void kiwiSetSite(const std::string& hostPort, const std::string& loc) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        using Fn = void (*)(void*, const char*, const char*);
        auto fn = (Fn)moduleSymbol("kiwisdr_source", "littlesdr_kiwi_set_site");
        auto it = core::moduleManager.instances.find("KiwiSDR Source");
        if (!fn || it == core::moduleManager.instances.end()) { return; }
        // The client reconnects only on start: restart when listening to KiwiSDR
        bool restart = gui::mainWindow.isPlaying() && sigpath::sourceManager.getSelectedName() == "KiwiSDR";
        if (restart) { gui::mainWindow.setPlayState(false); }
        fn(it->second.instance, hostPort.c_str(), loc.c_str());
        if (restart) { gui::mainWindow.setPlayState(true); }
    }

    // ------------------------------------------------------------------ Recorder
    std::string recJson() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        using Fn = int (*)(void*, char*, int);
        auto fn = (Fn)moduleSymbol("recorder", "littlesdr_rec_json");
        auto it = core::moduleManager.instances.find("Recorder");
        if (!fn || it == core::moduleManager.instances.end()) { return "{}"; }
        std::vector<char> buf(8192);
        fn(it->second.instance, buf.data(), (int)buf.size());
        return std::string(buf.data());
    }

    void recSet(const std::string& key, double v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        using Fn = void (*)(void*, const char*, double);
        auto fn = (Fn)moduleSymbol("recorder", "littlesdr_rec_set");
        auto it = core::moduleManager.instances.find("Recorder");
        if (fn && it != core::moduleManager.instances.end()) { fn(it->second.instance, key.c_str(), v); }
    }

    // ------------------------------------------------------------------ SSTV
    // Decoded from the radio's demodulated audio, taken before the volume (works muted)
    namespace {
        sstv::Decoder g_sstv;
        dsp::stream<dsp::stereo_t>* g_sstvStream = nullptr;
        std::thread g_sstvThread;

        void sstvLoop(dsp::stream<dsp::stereo_t>* s) {
            std::vector<float> mono;
            float rate = sigpath::sinkManager.getStreamSampleRate(VFO_NAME);
            auto lastRate = std::chrono::steady_clock::now();
            while (true) {
                int n = s->read();
                if (n < 0) { break; }
                mono.resize(n);
                for (int i = 0; i < n; i++) { mono[i] = (s->readBuf[i].l + s->readBuf[i].r) * 0.5f; }
                s->flush(); // release the splitter before decoding
                auto now = std::chrono::steady_clock::now();
                if (now - lastRate > std::chrono::seconds(1)) {
                    rate = sigpath::sinkManager.getStreamSampleRate(VFO_NAME);
                    lastRate = now;
                }
                g_sstv.feed(mono.data(), n, rate);
            }
        }
    }

    std::string sstvJson() {
        auto st = g_sstv.status();
        json j;
        j["enabled"] = g_sstvStream != nullptr;
        j["receiving"] = st.receiving;
        j["mode"] = st.mode;
        j["line"] = st.line;
        j["width"] = st.width;
        j["height"] = st.height;
        j["version"] = st.version;
        j["finished"] = st.finished;
        j["finishedRows"] = st.finishedRows;
        j["finishedMode"] = st.finishedMode;
        j["lastComplete"] = st.lastComplete;
        j["freq"] = st.freq;
        j["slantPpm"] = st.slantPpm;
        j["modes"] = json::array();
        for (auto& m : sstv::modes()) { j["modes"].push_back(m.name); }
        return j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace);
    }

    void sstvSet(const std::string& key, double v) {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        if (key == "enabled") {
            if (v != 0.0 && !g_sstvStream) {
                g_sstvStream = sigpath::sinkManager.bindStream(VFO_NAME);
                if (g_sstvStream) { g_sstvThread = std::thread(sstvLoop, g_sstvStream); }
            }
            else if (v == 0.0 && g_sstvStream) {
                // Reader first: unbinding stops the splitter's writer, then deletes the stream
                g_sstvStream->stopReader();
                if (g_sstvThread.joinable()) { g_sstvThread.join(); }
                sigpath::sinkManager.unbindStream(VFO_NAME, g_sstvStream);
                g_sstvStream = nullptr;
                g_sstv.reset();
            }
        }
        else if (key == "start") { g_sstv.startManual((int)v); }
        else if (key == "stop") { g_sstv.finishNow(); }
    }

    std::vector<uint32_t> sstvImage(int which, int& w, int& h) { return g_sstv.image(which, w, h); }

    std::vector<double> ctyLatLon(const std::vector<std::string>& calls) {
        std::vector<double> out;
        out.reserve(calls.size() * 2);
        for (auto& c : calls) {
            auto cs = utils::globalCty.findCallsign(c);
            if (cs.ll.isValid() && !(cs.ll.lat == 0 && cs.ll.lon == 0)) {
                out.push_back(cs.ll.lat);
                out.push_back(cs.ll.lon);
            }
            else {
                out.push_back(std::nan(""));
                out.push_back(std::nan(""));
            }
        }
        return out;
    }

    std::string rdsJson() {
        std::lock_guard<std::recursive_mutex> lck(g_mtx);
        using Fn = int (*)(void*, char*, int);
        auto fn = (Fn)moduleSymbol("radio", "littlesdr_radio_rds");
        auto it = core::moduleManager.instances.find(VFO_NAME);
        if (!fn || it == core::moduleManager.instances.end()) { return "{}"; }
        char buf[2048];
        fn(it->second.instance, buf, sizeof buf);
        return buf;
    }

}
