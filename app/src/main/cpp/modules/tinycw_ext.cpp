// LittleSDR build of SDR++Brown's TinyCW decoder module (neural CW/Morse decoder).
// The module class lives in its main.cpp (no header), so this translation unit includes it and
// adds C entry points for the engine (results as JSON, settings). CMakeLists.txt builds the
// module from this file instead of main.cpp; upstream is unmodified.

// Same macros and includes as the module's main.cpp, processed before the access override
#ifndef NOMINMAX
#define NOMINMAX
#endif
#define IMGUI_DEFINE_MATH_OPERATORS
#ifndef GImGui
#define GImGui (ImGui::GetCurrentContext())
#endif
#include <imgui.h>
#include <imgui_internal.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <map>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <config.h>
#include <gui/tuner.h>
#include <gui/brown/mobile_main_window.h>
#include <signal_path/vfo_manager.h>
#include <core.h>
#include <gui/gui.h>
#include <gui/style.h>
#include <gui/widgets/waterfall.h>
#include <module.h>
#include <signal_path/signal_path.h>
#include <utils/flog.h>

#include "tinycw_adapt.h"
#include "tinycw_anchor.h"
#include "tinycw_bands.h"
#include "tinycw_engine.h"
#include "tinycw_interface.h"
#include "tinycw_model.h"
#include "tinycw_textlayout.h"

#define private public
#include TINYCW_MAIN_CPP
#undef private

namespace {
    // Decoded text is plain ASCII in practice; never let a stray byte abort the process
    std::string safeDump(const json& j) { return j.dump(-1, ' ', false, json::error_handler_t::replace); }
}

/**
 * {"ready", "enabled", "band", "inBand", "lo", "hi", "channels", "busy",
 *  "focus": {"active", "text"}, "lines": [{"f", "text", "fresh"}]}
 */
extern "C" SDRPP_EXPORT int littlesdr_cw_json(void* instance, char* out, int cap) {
    auto* m = (TinyCWDecoderModule*)instance;
    json j = json::object();
    if (!m) { j["ready"] = false; }
    else {
        j["ready"] = m->model_.ready();
        j["enabled"] = m->enabled;
        j["channels"] = m->engine_.liveChannels();
        j["busy"] = m->engine_.busyPercent();

        json lines = json::array();
        {
            std::lock_guard<std::mutex> lck(m->uiMtx_);
            j["band"] = m->range_.bandName;
            j["inBand"] = m->range_.valid;
            j["lo"] = m->range_.loHz;
            j["hi"] = m->range_.hiHz;
            for (auto& l : m->uiDecodes_) {
                if (l.text.empty()) { continue; }
                json e;
                e["f"] = l.freqHz;
                e["text"] = l.text;
                e["fresh"] = l.fresh;
                lines.push_back(e);
            }
        }
        j["lines"] = lines;

        json focus = json::object();
        focus["active"] = m->engine_.focusActive();
        std::string text;
        std::vector<float> conf;
        if (m->engine_.focusActive()) { m->engine_.focusTextConf(text, conf); }
        focus["text"] = text;
        j["focus"] = focus;
    }
    std::string s = safeDump(j);
    if ((int)s.size() >= cap) { s = "{}"; }
    std::memcpy(out, s.c_str(), s.size() + 1);
    return (int)s.size();
}

/** "followVfo" (0/1): decode the tuned frequency in CW mode, in addition to the whole band. */
extern "C" SDRPP_EXPORT void littlesdr_cw_set(void* instance, const char* key, double v) {
    auto* m = (TinyCWDecoderModule*)instance;
    if (!m || !key) { return; }
    if (std::strcmp(key, "followVfo") == 0) { m->followVfo_ = v != 0.0; }
}
