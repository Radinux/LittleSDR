// Shared by all LittleSDR shaders (included with #include "common.glsl")
layout(push_constant) uniform PC {
    float binStart;   // first visible FFT bin (fractional)
    float binCount;   // number of visible bins
    float dbMin;
    float dbMax;
    int bins;         // bins per FFT line
    int rows;         // waterfall history rows
    int newest;       // index of the newest row in the ring
    int filled;       // number of valid rows
    float widthPx;    // viewport size in pixels
    float heightPx;
    float offY;       // viewport top in framebuffer pixels
    float lineW;      // spectrum trace thickness in pixels
    vec4 fillColor;
    vec4 lineColor;
    int palette;      // 0 Turbo, 1 Viridis, 2 Inferno, 3 Magma, 4 Classic, 5 Grayscale
    int lineOffset;   // spectrum pass: 0 = smoothed trace, bins = peak-hold line
} pc;

// Sample one FFT line at horizontal position x in [0,1] of the visible span:
// linear interpolation when zoomed in, peak-hold over the covered bins when zoomed out
// (keeps narrow carriers visible at any zoom).
#define SAMPLE_LINE(BUF, BASE, X, PIXELS, OUT)                                   \
    {                                                                            \
        float b0_ = pc.binStart + (X) * pc.binCount;                             \
        float span_ = pc.binCount / max(PIXELS, 1.0);                            \
        if (span_ <= 1.0) {                                                      \
            float bf_ = b0_ - 0.5;                                               \
            int i0_ = clamp(int(floor(bf_)), 0, pc.bins - 1);                    \
            int i1_ = min(i0_ + 1, pc.bins - 1);                                 \
            OUT = mix(BUF[(BASE) + i0_], BUF[(BASE) + i1_], fract(bf_));         \
        } else {                                                                 \
            int i0_ = int(b0_);                                                  \
            int n_ = int(ceil(span_));                                           \
            int st_ = max(1, n_ / 32);                                           \
            OUT = -1000.0;                                                       \
            for (int k_ = 0; k_ < n_; k_ += st_) {                               \
                OUT = max(OUT, BUF[(BASE) + clamp(i0_ + k_, 0, pc.bins - 1)]);   \
            }                                                                    \
        }                                                                        \
    }

// Waterfall version: zoomed out, the average POWER of the bins over two pixels instead of
// the highest bin of one. The highest of ~30 noise bins is a random bright speckle: the zoomed-out waterfall
// turns to sand. The power average is smooth on noise and still shows a carrier (one bin 30 dB
// up among 32 lifts the average by ~15 dB).
#define SAMPLE_LINE_AVG(BUF, BASE, X, PIXELS, OUT)                               \
    {                                                                            \
        float b0_ = pc.binStart + (X) * pc.binCount;                             \
        float span_ = pc.binCount / max(PIXELS, 1.0);                            \
        if (span_ <= 1.0) {                                                      \
            float bf_ = b0_ - 0.5;                                               \
            int i0_ = clamp(int(floor(bf_)), 0, pc.bins - 1);                    \
            int i1_ = min(i0_ + 1, pc.bins - 1);                                 \
            OUT = mix(BUF[(BASE) + i0_], BUF[(BASE) + i1_], fract(bf_));         \
        } else {                                                                 \
            /* two pixels wide, centred: half the resolution, much less grain */ \
            int i0_ = int(b0_ - 0.5 * span_);                                    \
            int n_ = int(ceil(2.0 * span_));                                     \
            int st_ = max(1, n_ / 16);                                           \
            float p_ = 0.0;                                                      \
            float c_ = 0.0;                                                      \
            for (int k_ = 0; k_ < n_; k_ += st_) {                               \
                float d_ = BUF[(BASE) + clamp(i0_ + k_, 0, pc.bins - 1)];        \
                p_ += exp2(clamp(d_, -200.0, 50.0) * 0.33219281);                \
                c_ += 1.0;                                                       \
            }                                                                    \
            OUT = 3.0103 * log2(max(p_ / c_, 1e-30));                            \
        }                                                                        \
    }

float normDb(float v) { return clamp((v - pc.dbMin) / max(pc.dbMax - pc.dbMin, 1.0), 0.0, 1.0); }

// Google "Turbo" colormap, polynomial approximation (Anton Mikhailov, Apache-2.0)
vec3 turbo(float x) {
    const vec4 kR4 = vec4(0.13572138, 4.61539260, -42.66032258, 132.13108234);
    const vec4 kG4 = vec4(0.09140261, 2.19418839, 4.84296658, -14.18503333);
    const vec4 kB4 = vec4(0.10667330, 12.64194608, -60.58204836, 110.36276771);
    const vec2 kR2 = vec2(-152.94239396, 59.28637943);
    const vec2 kG2 = vec2(4.27729857, 2.82956604);
    const vec2 kB2 = vec2(-89.90310912, 27.34824973);
    x = clamp(x, 0.0, 1.0);
    vec4 v4 = vec4(1.0, x, x * x, x * x * x);
    vec2 v2 = v4.zw * v4.z;
    return vec3(dot(v4, kR4) + dot(v2, kR2), dot(v4, kG4) + dot(v2, kG2), dot(v4, kB4) + dot(v2, kB2));
}

// Viridis / Inferno / Magma: polynomial fits by Matt Zucker (CC0, shadertoy WlfXRN)
vec3 poly6(float t, vec3 c0, vec3 c1, vec3 c2, vec3 c3, vec3 c4, vec3 c5, vec3 c6) {
    return c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * (c5 + t * c6)))));
}
vec3 viridis(float t) {
    return poly6(t, vec3(0.2777273272234177, 0.005407344544966578, 0.3340998053353061),
                    vec3(0.1050930431085774, 1.404613529898575, 1.384590162594685),
                    vec3(-0.3308618287255563, 0.214847559468213, 0.09509516302823659),
                    vec3(-4.634230498983486, -5.799100973351585, -19.33244095627987),
                    vec3(6.228269936347081, 14.17993336680509, 56.69055260068105),
                    vec3(4.776384997670288, -13.74514537774601, -65.35303263337234),
                    vec3(-5.435455855934631, 4.645852612178535, 26.3124352495832));
}
vec3 inferno(float t) {
    return poly6(t, vec3(0.0002189403691192265, 0.001651004631001012, -0.01948089843709184),
                    vec3(0.1065134194856116, 0.5639564367884091, 3.932712388889277),
                    vec3(11.60249308247187, -3.972853965665698, -15.9423941062914),
                    vec3(-41.70399613139459, 17.43639888205313, 44.35414519872813),
                    vec3(77.162935699427, -33.40235894210092, -81.80730925738993),
                    vec3(-71.31942824499214, 32.62606426397723, 73.20951985803202),
                    vec3(25.13112622477341, -12.24266895238567, -23.07032500287172));
}
vec3 magma(float t) {
    return poly6(t, vec3(-0.002136485053939582, -0.000749655052795221, -0.005386127855323933),
                    vec3(0.2516605407371642, 0.6775232436837668, 2.494026599312351),
                    vec3(8.353717279216625, -3.577719514958484, 0.3144679030132573),
                    vec3(-27.66873308576866, 14.26473078096533, -13.64921318813922),
                    vec3(52.17613981234068, -27.94360607168351, 12.94416944238394),
                    vec3(-50.76852536473588, 29.04658282127291, 4.23415299384598),
                    vec3(18.65570506591883, -11.48977351997711, -5.601961508734096));
}
// Classic receiver look: deep blue -> blue -> cyan -> yellow -> red
vec3 classic(float t) {
    const vec3 k[5] = vec3[](vec3(0.0, 0.0, 0.12), vec3(0.0, 0.25, 1.0), vec3(0.0, 1.0, 1.0),
                             vec3(1.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0));
    float x = clamp(t, 0.0, 1.0) * 4.0;
    int i = min(int(x), 3);
    return mix(k[i], k[i + 1], x - float(i));
}
vec3 colormap(float t) {
    t = clamp(t, 0.0, 1.0);
    if (pc.palette == 1) return clamp(viridis(t), 0.0, 1.0);
    if (pc.palette == 2) return clamp(inferno(t), 0.0, 1.0);
    if (pc.palette == 3) return clamp(magma(t), 0.0, 1.0);
    if (pc.palette == 4) return classic(t);
    if (pc.palette == 5) return vec3(t);
    return clamp(turbo(t), 0.0, 1.0);
}
