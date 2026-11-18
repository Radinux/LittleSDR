// LittleSDR extension compiled INTO SDR++Brown's radio.so (see CMakeLists.txt).
// The radio module only exposes mode/bandwidth/squelch through its interface; its other
// options (de-emphasis, FM IF noise reduction, noise blanker, AGC, snap) are private and
// drawn with raw ImGui. This file reaches them through two C entry points that the engine
// looks up with dlsym, without modifying the upstream sources.

// Pull every dependency in first so the access override below only affects the module classes
#include <imgui.h>
#include <module.h>
#include <gui/gui.h>
#include <gui/style.h>
#include <signal_path/signal_path.h>
#include <config.h>
#include <core.h>
#include <utils/optionlist.h>
#include <json.hpp>
#include <algorithm>
#include <cstring>
#include <string>

#define private public
#define protected public
#include <radio_module.h>
#undef protected
#undef private

using json = nlohmann::json;

namespace {
    // RDS text uses the EBU Latin character set (EN 50067, annex E), not UTF-8: 0x80-0xFF are
    // accented letters (0x98 = û, 0x82 = é...). Passing them raw to the JSON serializer throws.
    const char* const EBU_HIGH[128] = {
        "á","à","é","è","í","ì","ó","ò","ú","ù","Ñ","Ç","Ş","ß","¡","Ĳ",
        "â","ä","ê","ë","î","ï","ô","ö","û","ü","ñ","ç","ş","ğ","ı","ĳ",
        "ª","α","©","‰","Ğ","ě","ň","ő","π","€","£","$","←","↑","→","↓",
        "º","¹","²","³","±","İ","ń","ű","µ","¿","÷","°","¼","½","¾","§",
        "Á","À","É","È","Í","Ì","Ó","Ò","Ú","Ù","Ř","Č","Š","Ž","Đ","Ŀ",
        "Â","Ä","Ê","Ë","Î","Ï","Ô","Ö","Û","Ü","ř","č","š","ž","đ","ŀ",
        "Ã","Å","Æ","Œ","ŷ","Ý","Õ","Ø","Þ","Ŋ","Ŕ","Ć","Ś","Ź","Ŧ","ð",
        "ã","å","æ","œ","ŵ","ý","õ","ø","þ","ŋ","ŕ","ć","ś","ź","ŧ"," ",
    };

    std::string ebuToUtf8(const std::string& in) {
        std::string out;
        out.reserve(in.size());
        for (unsigned char c : in) {
            if (c >= 0x80) { out += EBU_HIGH[c - 0x80]; }
            else if (c >= 0x20 && c != 0x7F) { out += (char)c; }
            else { out += ' '; } // control codes (e.g. 0x0D end of RadioText)
        }
        return out;
    }

    // Never let a stray byte abort the process: invalid UTF-8 is replaced
    std::string safeDump(const json& j) { return j.dump(-1, ' ', false, json::error_handler_t::replace); }
}

namespace {
    // AGC lives in the AM / SSB / CW demodulators, all with the same member names
    template <class D>
    bool agcGet(demod::Demodulator* d, json& out) {
        auto x = dynamic_cast<D*>(d);
        if (!x) { return false; }
        out["agc"] = true;
        out["agcAttack"] = x->agcAttack;
        out["agcDecay"] = x->agcDecay;
        return true;
    }

    template <class D>
    bool agcSet(demod::Demodulator* d, bool attack, float v) {
        auto x = dynamic_cast<D*>(d);
        if (!x) { return false; }
        if (attack) {
            x->agcAttack = std::clamp(v, 1.0f, 200.0f);
            x->demod.setAGCAttack(x->agcAttack / x->getIFSampleRate());
        }
        else {
            x->agcDecay = std::clamp(v, 1.0f, 20.0f);
            x->demod.setAGCDecay(x->agcDecay / x->getIFSampleRate());
        }
        x->_config->acquire();
        x->_config->conf[x->name][x->getName()][attack ? "agcAttack" : "agcDecay"] = attack ? x->agcAttack : x->agcDecay;
        x->_config->release(true);
        return true;
    }

    bool anyAgcGet(demod::Demodulator* d, json& out) {
        return agcGet<demod::AM>(d, out) || agcGet<demod::USB>(d, out) || agcGet<demod::LSB>(d, out) ||
               agcGet<demod::DSB>(d, out) || agcGet<demod::CW>(d, out);
    }

    bool anyAgcSet(demod::Demodulator* d, bool attack, float v) {
        return agcSet<demod::AM>(d, attack, v) || agcSet<demod::USB>(d, attack, v) || agcSet<demod::LSB>(d, attack, v) ||
               agcSet<demod::DSB>(d, attack, v) || agcSet<demod::CW>(d, attack, v);
    }
}

// Writes the current demodulator options as JSON into out (NUL-terminated). Returns the length.
extern "C" SDRPP_EXPORT int littlesdr_radio_options(void* instance, char* out, int cap) {
    auto r = static_cast<RadioModule*>(static_cast<ModuleManager::Instance*>(instance));
    json j;
    if (r->selectedDemod) {
        j["demod"] = r->selectedDemod->getName();
        j["deempAllowed"] = r->deempAllowed;
        j["deemp"] = r->deempId;
        j["deempModes"] = json::array();
        for (int i = 0; i < r->deempModes.size(); i++) { j["deempModes"].push_back(r->deempModes.key(i)); }
        j["ifnrAllowed"] = r->FMIFNRAllowed;
        j["ifnr"] = r->FMIFNREnabled;
        j["ifnrPreset"] = r->fmIFPresetId;
        j["ifnrPresets"] = json::array();
        for (int i = 0; i < r->ifnrPresets.size(); i++) { j["ifnrPresets"].push_back(r->ifnrPresets.key(i)); }
        j["nbAllowed"] = r->nbAllowed;
        j["nb"] = r->nbEnabled;
        j["nbLevel"] = r->nbLevel;
        j["snapInterval"] = r->snapInterval;
        j["agc"] = false;
        anyAgcGet(r->selectedDemod, j);
    }
    std::string s = safeDump(j);
    int n = (int)std::min<size_t>(s.size(), (size_t)cap - 1);
    memcpy(out, s.data(), n);
    out[n] = 0;
    return n;
}

extern "C" SDRPP_EXPORT void littlesdr_radio_set(void* instance, const char* key, double v) {
    auto r = static_cast<RadioModule*>(static_cast<ModuleManager::Instance*>(instance));
    if (!r->selectedDemod) { return; }
    std::string k = key;
    if (k == "deemp" && r->deempAllowed) {
        int id = std::clamp((int)v, 0, r->deempModes.size() - 1);
        r->setDeemphasisMode(r->deempModes.value(id));
    }
    else if (k == "ifnr" && r->FMIFNRAllowed) {
        r->setFMIFNREnabled(v != 0.0);
    }
    else if (k == "ifnrPreset" && r->FMIFNRAllowed) {
        int id = std::clamp((int)v, 0, r->ifnrPresets.size() - 1);
        r->setIFNRPreset(r->ifnrPresets.value(id));
    }
    else if (k == "nb" && r->nbAllowed) {
        r->setNBEnabled(v != 0.0);
    }
    else if (k == "nbLevel" && r->nbAllowed) {
        r->setNBLevel(std::clamp((float)v, (float)r->MIN_NB, (float)r->MAX_NB));
    }
    else if (k == "snapInterval") {
        r->snapInterval = std::max(1, (int)v);
        r->vfo->setSnapInterval(r->snapInterval);
        config.acquire();
        config.conf[r->name][r->selectedDemod->getName()]["snapInterval"] = r->snapInterval;
        config.release(true);
    }
    else if (k == "agcAttack") {
        anyAgcSet(r->selectedDemod, true, (float)v);
    }
    else if (k == "agcDecay") {
        anyAgcSet(r->selectedDemod, false, (float)v);
    }
}

// RDS of the WFM demodulator as JSON: {active, ps, rt, pi, callsign, pty}. Turns RDS decoding on
// when the WFM demodulator is selected (it is off by default upstream).
extern "C" SDRPP_EXPORT int littlesdr_radio_rds(void* instance, char* out, int cap) {
    auto r = static_cast<RadioModule*>(static_cast<ModuleManager::Instance*>(instance));
    json j;
    j["active"] = false;
    auto w = r->selectedDemod ? dynamic_cast<demod::WFM*>(r->selectedDemod) : nullptr;
    if (w) {
        if (!w->_rds) {
            w->_rds = true;
            w->demod.setRDSOut(true);
            w->_config->acquire();
            w->_config->conf[w->name][w->getName()]["rds"] = true;
            w->_config->release(true);
        }
        auto& d = w->rdsDecode;
        j["active"] = true;
        auto trim = [](std::string s) {
            s.erase(s.find_last_not_of(' ') + 1);
            s.erase(0, std::min(s.find_first_not_of(' '), s.size()));
            return s;
        };
        if (d.PSNameValid()) { j["ps"] = trim(ebuToUtf8(d.getPSName(false))); }
        if (d.radioTextValid()) { j["rt"] = trim(ebuToUtf8(d.getRadioText(false))); }
        if (d.piCodeValid()) {
            char pi[8];
            snprintf(pi, sizeof pi, "%04X", d.getPICode());
            j["pi"] = pi;
            j["callsign"] = ebuToUtf8(d.getCallsign());
        }
        if (d.programTypeValid()) {
            int pty = (int)d.getProgramType();
            bool us = w->rdsRegions.size() > 0 && w->rdsRegions.key(w->rdsRegionId) == "na";
            if (pty > 0 && pty < 32) { j["pty"] = us ? rds::PROGRAM_TYPE_US_TO_STR[pty] : rds::PROGRAM_TYPE_EU_TO_STR[pty]; }
        }
    }
    std::string s = safeDump(j);
    int n = (int)std::min<size_t>(s.size(), (size_t)cap - 1);
    memcpy(out, s.data(), n);
    out[n] = 0;
    return n;
}
