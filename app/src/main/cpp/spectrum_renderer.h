#pragma once
#include <android/native_window.h>
#include <atomic>
#include <mutex>
#include <thread>

// Vulkan renderer for the spectrum trace + waterfall, drawing into an Android Surface.
// Runs its own render thread; reads FFT lines from engine::fftHistory().
namespace spectrum {
    struct ViewParams {
        double viewBandwidth = 0; // Hz shown across the width (0 = full span)
        double viewOffset = 0;    // Hz, center of the view relative to the center frequency
        float dbMin = -120.0f;
        float dbMax = -20.0f;
        float spectrumFraction = 0.35f; // height share of the spectrum trace (top)
        float fillColor[4] = { 0.55f, 0.75f, 1.0f, 0.35f };
        float lineColor[4] = { 0.80f, 0.90f, 1.0f, 1.0f };
        float background[4] = { 0.07f, 0.07f, 0.09f, 1.0f };
        float lineWidthPx = 3.0f;
        int palette = 0; // see colormap() in shaders/common.glsl
    };

    bool attach(ANativeWindow* window); // takes ownership of one window reference
    void resize(int width, int height);
    void detach();
    void setView(const ViewParams& p);
    ViewParams view();
}
