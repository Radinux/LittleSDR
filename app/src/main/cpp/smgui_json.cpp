#include "smgui_json.h"
#include <imgui.h>
#include <json.hpp>
#include <signal_path/signal_path.h>
#include <utils/flog.h>

using json = nlohmann::json;

namespace smguijson {
    namespace {
        // Visible part of an ImGui label ("Gain##rtl_gain" -> "Gain", "##x" -> "")
        std::string visible(const std::string& label) {
            auto p = label.find("##");
            return p == std::string::npos ? label : label.substr(0, p);
        }

        std::vector<std::string> splitZeros(const std::string& s) {
            std::vector<std::string> out;
            size_t start = 0;
            while (start < s.size()) {
                size_t end = s.find('\0', start);
                if (end == std::string::npos) { end = s.size(); }
                out.push_back(s.substr(start, end - start));
                start = end + 1;
            }
            return out;
        }

        const char* fmtOf(int f) {
            switch ((SmGui::FormatString)f) {
            case SmGui::FMT_STR_INT_DB:
            case SmGui::FMT_STR_FLOAT_DB_NO_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_ONE_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_TWO_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_THREE_DECIMAL: return "db";
            case SmGui::FMT_STR_NONE: return "none";
            case SmGui::FMT_STR_PLUS_INT_PERCENT: return "percent";
            case SmGui::FMT_STR_WATTS_INT: return "watts";
            default: return "num";
            }
        }

        int decimalsOf(int f) {
            switch ((SmGui::FormatString)f) {
            case SmGui::FMT_STR_FLOAT_NO_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_NO_DECIMAL: return 0;
            case SmGui::FMT_STR_FLOAT_ONE_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_ONE_DECIMAL: return 1;
            case SmGui::FMT_STR_FLOAT_TWO_DECIMAL:
            case SmGui::FMT_STR_FLOAT_DB_TWO_DECIMAL:
            case SmGui::FMT_STR_TEN_POWER_FLOAT: return 2;
            default: return 3;
            }
        }

        // Walks a recorded DrawList and turns it into flat widget objects.
        json toJson(const SmGui::DrawList& dl) {
            json out = json::array();
            const auto& e = dl.elements;
            size_t i = 0;
            int disabled = 0;
            bool sameLine = false;
            std::string pendingLabel; // LeftLabel applies to the next widget

            auto S = [&](size_t k) -> std::string { return k < e.size() ? e[k].str : std::string(); };
            auto I = [&](size_t k) -> int { return k < e.size() ? e[k].i : 0; };
            auto F = [&](size_t k) -> float { return k < e.size() ? e[k].f : 0.0f; };
            auto B = [&](size_t k) -> bool { return k < e.size() ? e[k].b : false; };

            auto push = [&](json w) {
                w["disabled"] = disabled > 0;
                w["sameLine"] = sameLine;
                if (!pendingLabel.empty() && !w.contains("title")) { w["title"] = pendingLabel; }
                pendingLabel.clear();
                sameLine = false;
                out.push_back(std::move(w));
            };

            while (i < e.size()) {
                if (e[i].type != SmGui::DRAW_LIST_ELEM_TYPE_DRAW_STEP) { i++; continue; }
                auto step = e[i].step;
                size_t a = i + 1;
                switch (step) {
                case SmGui::DRAW_STEP_FILL_WIDTH: i = a; break;
                case SmGui::DRAW_STEP_SAME_LINE: sameLine = true; i = a; break;
                case SmGui::DRAW_STEP_BEGIN_DISABLED: disabled++; i = a; break;
                case SmGui::DRAW_STEP_END_DISABLED: disabled = std::max(0, disabled - 1); i = a; break;
                case SmGui::DRAW_STEP_COMBO:
                    push({ { "t", "combo" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", I(a + 1) }, { "items", splitZeros(S(a + 2)) } });
                    i = a + 4; break;
                case SmGui::DRAW_STEP_BUTTON:
                    push({ { "t", "button" }, { "id", S(a) }, { "label", visible(S(a)) } });
                    i = a + 3; break;
                case SmGui::DRAW_STEP_COLUMNS: i = a + 3; break;
                case SmGui::DRAW_STEP_NEXT_COLUMN: sameLine = true; i = a; break;
                case SmGui::DRAW_STEP_RADIO_BUTTON:
                    push({ { "t", "radio" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", B(a + 1) } });
                    i = a + 2; break;
                case SmGui::DRAW_STEP_BEGIN_GROUP:
                case SmGui::DRAW_STEP_END_GROUP: i = a; break;
                case SmGui::DRAW_STEP_LEFT_LABEL: pendingLabel = visible(S(a)); i = a + 1; break;
                case SmGui::DRAW_STEP_SLIDER_INT:
                    push({ { "t", "slider_int" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", I(a + 1) },
                           { "min", I(a + 2) }, { "max", I(a + 3) }, { "fmt", fmtOf(I(a + 4)) } });
                    i = a + 6; break;
                case SmGui::DRAW_STEP_SLIDER_FLOAT_WITH_STEPS:
                    push({ { "t", "slider_float" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", F(a + 1) },
                           { "min", F(a + 2) }, { "max", F(a + 3) }, { "step", F(a + 4) },
                           { "fmt", fmtOf(I(a + 5)) }, { "decimals", decimalsOf(I(a + 5)) } });
                    i = a + 6; break;
                case SmGui::DRAW_STEP_INPUT_INT:
                    push({ { "t", "input_int" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", I(a + 1) }, { "step", I(a + 2) } });
                    i = a + 5; break;
                case SmGui::DRAW_STEP_CHECKBOX:
                    push({ { "t", "checkbox" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", B(a + 1) } });
                    i = a + 2; break;
                case SmGui::DRAW_STEP_SLIDER_FLOAT:
                    push({ { "t", "slider_float" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", F(a + 1) },
                           { "min", F(a + 2) }, { "max", F(a + 3) }, { "step", 0.0f },
                           { "fmt", fmtOf(I(a + 4)) }, { "decimals", decimalsOf(I(a + 4)) } });
                    i = a + 6; break;
                case SmGui::DRAW_STEP_INPUT_TEXT:
                    push({ { "t", "input_text" }, { "id", S(a) }, { "label", visible(S(a)) }, { "value", S(a + 1) }, { "maxLen", I(a + 2) } });
                    i = a + 4; break;
                case SmGui::DRAW_STEP_TEXT:
                    push({ { "t", "text" }, { "value", S(a) } });
                    i = a + 1; break;
                case SmGui::DRAW_STEP_TEXT_COLORED:
                    push({ { "t", "text" }, { "value", S(a + 4) }, { "color", { F(a), F(a + 1), F(a + 2), F(a + 3) } } });
                    i = a + 5; break;
                case SmGui::DRAW_STEP_OPEN_POPUP: i = a + 2; break;
                case SmGui::DRAW_STEP_BEGIN_POPUP: i = a + 2; break;
                case SmGui::DRAW_STEP_END_POPUP: i = a; break;
                case SmGui::DRAW_STEP_BEGIN_TABLE: i = a + 6; break;
                case SmGui::DRAW_STEP_END_TABLE: i = a; break;
                case SmGui::DRAW_STEP_TABLE_NEXT_ROW: i = a + 2; break;
                case SmGui::DRAW_STEP_TABLE_SET_COLUMN_INDEX: if (I(a) > 0) { sameLine = true; } i = a + 1; break;
                case SmGui::DRAW_STEP_SET_NEXT_ITEM_WIDTH: i = a + 1; break;
                case SmGui::DRAW_STEP_POP_STYLE_COLOR: i = a + 1; break;
                case SmGui::DRAW_STEP_PUSH_STYLE_COLOR: i = a + 5; break;
                case SmGui::DRAW_STEP_COLLAPSING_HEADER:
                    push({ { "t", "header" }, { "value", visible(S(a)) } });
                    i = a + 1; break;
                default:
                    flog::warn("smgui_json: unknown step {}", (int)step);
                    i = a; break;
                }
            }
            return out;
        }

        void drawMenu() {
            // A throwaway ImGui frame: a few menus still call ImGui directly
            // (map pickers, colored status text). Those draws go nowhere.
            ImGuiIO& io = ImGui::GetIO();
            io.DeltaTime = 1.0f / 30.0f;
            ImGui::NewFrame();
            ImGui::Begin("##littlesdr_offscreen", nullptr, ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoDecoration);
            sigpath::sourceManager.showSelectedMenu();
            ImGui::End();
            ImGui::EndFrame();
        }
    }

    std::string renderSourceMenu(const std::string& diffId, const SmGui::DrawListElem& diffValue) {
        SmGui::DrawListElem none;
        none.type = SmGui::DRAW_LIST_ELEM_TYPE_BOOL;
        none.b = false;

        if (!diffId.empty()) {
            // Apply the action, then record a clean pass (same as server.cpp renderUI)
            SmGui::setDiff(diffId, diffValue);
            drawMenu();
        }
        SmGui::DrawList dl;
        SmGui::setDiff("", none);
        SmGui::startRecord(&dl);
        drawMenu();
        SmGui::stopRecord();
        return toJson(dl).dump();
    }
}
