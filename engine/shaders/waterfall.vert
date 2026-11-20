#version 450
// Full-viewport quad; the viewport itself places the waterfall on screen.
layout(location = 0) out vec2 uv;
void main() {
    vec2 p[6] = vec2[](vec2(0, 0), vec2(1, 0), vec2(0, 1), vec2(1, 0), vec2(1, 1), vec2(0, 1));
    uv = p[gl_VertexIndex];
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}
