#pragma once
#include <string>
#include <gui/smgui.h>

namespace smguijson {
    // Runs the selected source's menu like SDR++ server mode does: optionally
    // applies one user action (diffId/diffValue), then records the menu and
    // returns it as a JSON array of widgets for the Compose renderer.
    std::string renderSourceMenu(const std::string& diffId, const SmGui::DrawListElem& diffValue);
}
