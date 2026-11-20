#version 450
#extension GL_GOOGLE_include_directive : require
#include "common.glsl"
// Triangle strip: 2 vertices per pixel column (trace point, bottom of the viewport)
layout(std430, set = 0, binding = 1) readonly buffer Latest { float latest[]; };
layout(location = 0) out float vTopY; // trace height in pixels from viewport top
layout(location = 1) out float vT;    // 1 on the trace, 0 at the bottom
void main() {
    int col = gl_VertexIndex / 2;
    bool top = (gl_VertexIndex % 2) == 0;
    float cols = max(pc.widthPx, 2.0);
    float x = float(col) / (cols - 1.0);
    float v;
    SAMPLE_LINE(latest, pc.lineOffset, x, cols, v)
    float t = normDb(v);
    vTopY = (1.0 - t) * pc.heightPx;
    vT = top ? 1.0 : 0.0;
    gl_Position = vec4(x * 2.0 - 1.0, top ? (1.0 - 2.0 * t) : 1.0, 0.0, 1.0);
}
