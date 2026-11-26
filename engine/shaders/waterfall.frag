#version 450
#extension GL_GOOGLE_include_directive : require
#include "common.glsl"
layout(std430, set = 0, binding = 0) readonly buffer Lines { float lines[]; };
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 outColor;
void main() {
    int rowOff = int(uv.y * float(pc.rows)); // newest line at the top
    if (rowOff >= pc.filled) { discard; } // not filled yet: show the background
    // A pixel row covers several history lines (1024 lines in a few hundred pixels): their
    // power average, instead of one line in three or four (each one noisy: grain)
    int nr = clamp(int(ceil(float(pc.rows) / max(pc.heightPx, 1.0))), 1, 4);
    float p = 0.0;
    float c = 0.0;
    for (int k = 0; k < nr; k++) {
        int ro = rowOff + k;
        if (ro >= pc.filled) { break; }
        int row = (pc.newest - ro + pc.rows * 4) % pc.rows;
        float v;
        SAMPLE_LINE_AVG(lines, row * pc.bins, uv.x, pc.widthPx, v)
        p += exp2(clamp(v, -200.0, 50.0) * 0.33219281);
        c += 1.0;
    }
    outColor = vec4(colormap(normDb(3.0103 * log2(max(p / max(c, 1.0), 1e-30)))), 1.0);
}
