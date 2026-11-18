// LittleSDR build of SDR++Brown's KiwiSDR source module.
// The server is normally picked on an ImGui map that LittleSDR doesn't show: this translation
// unit includes the module's main.cpp and adds an entry point to set the server from the app's
// own picker. CMakeLists.txt builds the module from this file; upstream is unmodified.
#include KIWISDR_MAIN_CPP

#include <cstring>

/** Same as choosing a server on the module's map: saved, used at the next start. */
extern "C" SDRPP_EXPORT void littlesdr_kiwi_set_site(void* instance, const char* hostPort, const char* loc) {
    auto* m = (KiwiSDRSourceModule*)instance;
    if (!m || !hostPort || !*hostPort) { return; }
    m->kiwisdrSite = hostPort;
    m->kiwisdrLoc = loc ? loc : "";
    config.acquire();
    config.conf["kiwisdr_site"] = m->kiwisdrSite;
    config.conf["kiwisdr_loc"] = m->kiwisdrLoc;
    config.release(true);
    m->kiwiSdrClient.init(m->kiwisdrSite);
    flog::info("KiwiSDR: site {}", m->kiwisdrSite);
}
