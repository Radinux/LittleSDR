#version 450
#extension GL_GOOGLE_include_directive : require
#include "common.glsl"
layout(std430, set = 0, binding = 0) readonly buffer Lines { float lines[]; };
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 outColor;
void main() {
    int rowOff = int(uv.y * float(pc.rows)); // newest line at the top
    if (rowOff >= pc.filled) { discard; } // not filled yet: show the background
    int row = (pc.newest - rowOff + pc.rows * 4) % pc.rows;
    float v;
    SAMPLE_LINE_AVG(lines, row * pc.bins, uv.x, pc.widthPx, v)
    outColor = vec4(colormap(normDb(v)), 1.0);
}
