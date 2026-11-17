#version 450
#extension GL_GOOGLE_include_directive : require
#include "common.glsl"
layout(location = 0) in float vTopY;
layout(location = 1) in float vT;
layout(location = 0) out vec4 outColor;
void main() {
    float d = (gl_FragCoord.y - pc.offY) - vTopY;           // distance below the trace
    float line = 1.0 - smoothstep(pc.lineW * 0.5, pc.lineW * 0.5 + 1.0, abs(d));
    vec4 fill = vec4(pc.fillColor.rgb, pc.fillColor.a * (0.15 + 0.85 * vT));
    outColor = mix(fill, pc.lineColor, line);
}
