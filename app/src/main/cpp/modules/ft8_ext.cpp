// LittleSDR build of SDR++Brown's FT8/FT4 decoder module.
// FT8DecoderModule is defined inside the module's main.cpp (no header), so this translation
// unit includes it and adds C entry points for the engine (results as JSON, commands).
// CMakeLists.txt builds the module from this file instead of main.cpp; upstream is unmodified.
// Same macros and includes as the module's main.cpp, processed before the access override
#define IMGUI_DEFINE_MATH_OPERATORS
#define _USE_MATH_DEFINES
#include <json.hpp>
#include <algorithm>
#include <cstring>
#include <imgui.h>
#include <config.h>
#include <core.h>
#include <gui/style.h>
#include <gui/gui.h>
#include <gui/widgets/waterfall.h>
#include <signal_path/signal_path.h>
#include <module.h>
#include <filesystem>
#include <dsp/stream.h>
#include <dsp/types.h>
#include <gui/widgets/folder_select.h>
#include <fstream>
#include <chrono>
#include <future>
#include <unordered_map>
#include <unordered_set>
#include <utils/kmeans.h>
#include <utils/cty.h>
#include "ft8_decoder.h"
#include "../../radio/src/demodulators/usb.h"
#include "module_interface.h"
#include "ft8_etc/gen_ft8.h"

// FT8DecoderModule's decoders are declared before any access specifier (private by default):
// "class" is only used for that class in main.cpp, so read it as a struct.
#define class struct
#define private public
#define protected public
#include "main.cpp"
#undef protected
#undef private
#undef class

namespace {
    nlohmann::json decoderJson(SingleDecoder& d) {
        nlohmann::json j;
        j["processing"] = d.processingEnabled;
        j["onFrequency"] = d.onTheFrequency;
        auto vfo = d.calculateVFOCenterOffsetForMode(gui::waterfall.getCenterFrequency(), gui::waterfall.getBandwidth());
        j["dial"] = vfo.first == INVALID_OFFSET ? 0 : (long long)(vfo.second - USB_BANDWIDTH);
        j["lastDecodeCount"] = d.lastDecodeCount.load();
        j["lastDecodeTime"] = d.lastDecodeTime.load();
        j["decoding"] = d.blockProcessorsRunning.load() > 0;
        j["error"] = std::string(d.decodeError);
        return j;
    }
}

// JSON: { enabled, ft8:{...}, ft4:{...}, results:[...] } newest first, at most 400 results.
extern "C" SDRPP_EXPORT int littlesdr_ft8_json(void* instance, char* out, int cap) {
    auto m = static_cast<FT8DecoderModule*>(static_cast<ModuleManager::Instance*>(instance));
    nlohmann::json j;
    j["enabled"] = m->enabled;
    j["ft8"] = decoderJson(m->ft8decoder);
    j["ft4"] = decoderJson(m->ft4decoder);
    std::vector<DecodedResult> copy;
    {
        std::lock_guard<std::mutex> g(m->decodedResultsLock);
        copy = m->decodedResults;
    }
    std::sort(copy.begin(), copy.end(), [](const DecodedResult& a, const DecodedResult& b) {
        return a.decodeEndTimestamp > b.decodeEndTimestamp;
    });
    if (copy.size() > 400) { copy.erase(copy.begin() + 400, copy.end()); }
    j["results"] = nlohmann::json::array();
    for (auto& r : copy) {
        j["results"].push_back({
            { "mode", r.mode == DM_FT4 ? "FT4" : "FT8" },
            { "time", r.decodeEndTimestamp },          // ms since epoch (end of the slot)
            { "audioHz", r.frequency },                // offset inside the 3 kHz USB passband
            { "band", r.frequencyBand },               // MHz
            { "call", r.shortString },
            { "message", r.detailedString },
            { "snr", r.strengthRaw },
            { "distanceKm", r.distance },
            { "country", r.qth },
        });
    }
    std::string s = j.dump(-1, ' ', false, nlohmann::json::error_handler_t::replace);
    int n = (int)std::min<size_t>(s.size(), (size_t)cap - 1);
    memcpy(out, s.data(), n);
    out[n] = 0;
    return n;
}

// Commands: "ft8"/"ft4" = processing on/off, "clear" = forget all results
extern "C" SDRPP_EXPORT void littlesdr_ft8_set(void* instance, const char* key, double v) {
    auto m = static_cast<FT8DecoderModule*>(static_cast<ModuleManager::Instance*>(instance));
    std::string k = key;
    if (k == "ft8") { m->ft8decoder.processingEnabled = v != 0.0; }
    else if (k == "ft4") { m->ft4decoder.processingEnabled = v != 0.0; }
    else if (k == "clear") {
        m->clearDecodedResults(DM_FT8);
        m->clearDecodedResults(DM_FT4);
    }
}
