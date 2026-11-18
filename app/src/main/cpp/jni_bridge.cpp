// JNI surface of the native engine, mirrored by app.littlesdr.engine.NativeBridge (Kotlin).
#include <jni.h>
#include <android/native_window_jni.h>
#include <cstdint>
#include <string>
#include <vector>

#include "engine.h"
#include "headless_backend.h"
#include "spectrum_renderer.h"

#define JNI_FN(name) Java_app_littlesdr_engine_NativeBridge_##name

namespace {
    // Java strings <-> standard UTF-8. JNI's *StringUTF* functions use "modified" UTF-8, where a
    // character outside the BMP (emoji, e.g. in KiwiSDR names) is two 3-byte surrogates: invalid
    // UTF-8 for nlohmann::json (a config save then throws and aborts the process), and CheckJNI
    // aborts on a real 4-byte sequence passed to NewStringUTF. Convert from/to UTF-16 instead.
    void putUtf8(std::string& r, uint32_t cp) {
        if (cp < 0x80) { r += (char)cp; }
        else if (cp < 0x800) { r += (char)(0xC0 | (cp >> 6)); r += (char)(0x80 | (cp & 0x3F)); }
        else if (cp < 0x10000) {
            r += (char)(0xE0 | (cp >> 12)); r += (char)(0x80 | ((cp >> 6) & 0x3F)); r += (char)(0x80 | (cp & 0x3F));
        }
        else {
            r += (char)(0xF0 | (cp >> 18)); r += (char)(0x80 | ((cp >> 12) & 0x3F));
            r += (char)(0x80 | ((cp >> 6) & 0x3F)); r += (char)(0x80 | (cp & 0x3F));
        }
    }

    std::string str(JNIEnv* env, jstring s) {
        if (!s) { return ""; }
        jsize n = env->GetStringLength(s);
        const jchar* c = env->GetStringChars(s, nullptr);
        std::string r;
        r.reserve(n);
        for (jsize i = 0; i < n; i++) {
            uint32_t cp = c[i];
            if (cp >= 0xD800 && cp <= 0xDBFF && i + 1 < n && c[i + 1] >= 0xDC00 && c[i + 1] <= 0xDFFF) {
                cp = 0x10000 + ((cp - 0xD800) << 10) + (c[++i] - 0xDC00);
            }
            else if (cp >= 0xD800 && cp <= 0xDFFF) { cp = 0xFFFD; } // lone surrogate
            putUtf8(r, cp);
        }
        env->ReleaseStringChars(s, c);
        return r;
    }

    jstring jstr(JNIEnv* env, const std::string& s) {
        std::u16string u;
        u.reserve(s.size());
        const auto* p = (const unsigned char*)s.data();
        size_t n = s.size(), i = 0;
        while (i < n) {
            uint32_t cp = p[i];
            int len = cp < 0x80 ? 1 : (cp >> 5) == 0x6 ? 2 : (cp >> 4) == 0xE ? 3 : (cp >> 3) == 0x1E ? 4 : 0;
            bool ok = len > 0 && i + len <= n;
            if (ok && len > 1) {
                cp &= 0xFF >> (len + 1);
                for (int k = 1; k < len; k++) {
                    if ((p[i + k] & 0xC0) != 0x80) { ok = false; break; }
                    cp = (cp << 6) | (p[i + k] & 0x3F);
                }
            }
            if (!ok) { u += (char16_t)0xFFFD; i++; continue; }
            i += len;
            if (cp >= 0x10000) {
                cp -= 0x10000;
                u += (char16_t)(0xD800 + (cp >> 10));
                u += (char16_t)(0xDC00 + (cp & 0x3FF));
            }
            else { u += (char16_t)cp; }
        }
        return env->NewString((const jchar*)u.data(), (jsize)u.size());
    }

    void argbToFloat(jint c, float out[4]) {
        out[0] = ((c >> 16) & 0xFF) / 255.0f;
        out[1] = ((c >> 8) & 0xFF) / 255.0f;
        out[2] = (c & 0xFF) / 255.0f;
        out[3] = ((c >> 24) & 0xFF) / 255.0f;
    }
}

extern "C" {

JNIEXPORT jint JNI_OnLoad(JavaVM* vm, void*) {
    JNIEnv* env = nullptr;
    if (vm->GetEnv((void**)&env, JNI_VERSION_1_6) != JNI_OK) { return JNI_ERR; }
    jclass cls = env->FindClass("app/littlesdr/engine/NativeBridge");
    if (!cls) { return JNI_ERR; }
    headless::setJavaVM(vm, env->NewGlobalRef(cls));
    return JNI_VERSION_1_6;
}

// ---------------------------------------------------------------- lifecycle
JNIEXPORT jboolean JNICALL JNI_FN(nativeInit)(JNIEnv* env, jclass, jstring root, jstring cache, jint fftSize) {
    return engine::init(str(env, root), str(env, cache), fftSize);
}

JNIEXPORT void JNICALL JNI_FN(setUsbDevice)(JNIEnv*, jclass, jint fd, jint vid, jint pid) {
    headless::setUsbDevice(fd, vid, pid);
}

JNIEXPORT void JNICALL JNI_FN(clearUsbDevice)(JNIEnv*, jclass) { headless::clearUsbDevice(); }

// ---------------------------------------------------------------- sources
JNIEXPORT jobjectArray JNICALL JNI_FN(sourceNames)(JNIEnv* env, jclass) {
    auto names = engine::sourceNames();
    jobjectArray arr = env->NewObjectArray((jsize)names.size(), env->FindClass("java/lang/String"), nullptr);
    for (size_t i = 0; i < names.size(); i++) {
        jstring s = jstr(env, names[i]);
        env->SetObjectArrayElement(arr, (jsize)i, s);
        env->DeleteLocalRef(s);
    }
    return arr;
}

JNIEXPORT jstring JNICALL JNI_FN(selectedSource)(JNIEnv* env, jclass) {
    return jstr(env, engine::selectedSource());
}

JNIEXPORT void JNICALL JNI_FN(selectSource)(JNIEnv* env, jclass, jstring name) {
    engine::selectSource(str(env, name));
}

JNIEXPORT jstring JNICALL JNI_FN(sourceMenu)(JNIEnv* env, jclass, jstring diffId, jint type, jboolean b, jint i, jfloat f, jstring s) {
    return jstr(env, engine::sourceMenuJson(str(env, diffId), type, b, i, f, str(env, s)));
}

// ---------------------------------------------------------------- playback / tuning
JNIEXPORT jboolean JNICALL JNI_FN(start)(JNIEnv*, jclass) { return engine::start(); }
JNIEXPORT void JNICALL JNI_FN(stop)(JNIEnv*, jclass) { engine::stop(); }
JNIEXPORT void JNICALL JNI_FN(tune)(JNIEnv*, jclass, jdouble hz) { engine::tune(hz); }
JNIEXPORT void JNICALL JNI_FN(tuneCenter)(JNIEnv*, jclass, jdouble hz) { engine::tuneCenter(hz); }

// One call for everything the UI polls (avoids a dozen JNI round trips per frame)
JNIEXPORT jdoubleArray JNICALL JNI_FN(state)(JNIEnv* env, jclass) {
    engine::syncVfo(); // keep audio on the displayed frequency (see engine::syncVfo)
    engine::spyServerMonitor(); // SpyServer: lower the rate when the server can't keep up
    double v[11] = {
        engine::vfoFrequency(),
        engine::centerFrequency(),
        engine::sampleRate(),
        engine::vfoOffset(),
        engine::vfoBandwidth(),
        engine::signalLevelDb(),
        (double)engine::mode(),
        engine::isPlaying() ? 1.0 : 0.0,
        engine::squelchEnabled() ? 1.0 : 0.0,
        engine::squelchLevel(),
        engine::volume(),
    };
    jdoubleArray arr = env->NewDoubleArray(11);
    env->SetDoubleArrayRegion(arr, 0, 11, v);
    return arr;
}

// ---------------------------------------------------------------- demod
JNIEXPORT void JNICALL JNI_FN(setMode)(JNIEnv*, jclass, jint m) { engine::setMode(m); }
JNIEXPORT void JNICALL JNI_FN(setBandwidth)(JNIEnv*, jclass, jfloat hz) { engine::setBandwidth(hz); }
JNIEXPORT void JNICALL JNI_FN(setSquelch)(JNIEnv*, jclass, jboolean en, jfloat lvl) { engine::setSquelch(en, lvl); }
JNIEXPORT jfloat JNICALL JNI_FN(estimateBandwidth)(JNIEnv*, jclass, jdouble minBw, jdouble maxBw) {
    return engine::estimateBandwidth(minBw, maxBw);
}
JNIEXPORT jstring JNICALL JNI_FN(options)(JNIEnv* env, jclass) {
    return jstr(env, engine::optionsJson());
}
JNIEXPORT void JNICALL JNI_FN(setOption)(JNIEnv* env, jclass, jstring key, jdouble value) {
    engine::setOption(str(env, key), value);
}
JNIEXPORT void JNICALL JNI_FN(setOptionString)(JNIEnv* env, jclass, jstring key, jstring value) {
    engine::setOptionString(str(env, key), str(env, value));
}
JNIEXPORT jstring JNICALL JNI_FN(ft8Json)(JNIEnv* env, jclass) {
    return jstr(env, engine::ft8Json());
}
JNIEXPORT void JNICALL JNI_FN(ft8Set)(JNIEnv* env, jclass, jstring key, jdouble value) {
    engine::ft8Set(str(env, key), value);
}
JNIEXPORT void JNICALL JNI_FN(kiwiSetSite)(JNIEnv* env, jclass, jstring hostPort, jstring loc) {
    engine::kiwiSetSite(str(env, hostPort), str(env, loc));
}
JNIEXPORT jstring JNICALL JNI_FN(cwJson)(JNIEnv* env, jclass) {
    return jstr(env, engine::cwJson());
}
JNIEXPORT void JNICALL JNI_FN(cwSet)(JNIEnv* env, jclass, jstring key, jdouble value) {
    engine::cwSet(str(env, key), value);
}
JNIEXPORT jstring JNICALL JNI_FN(recJson)(JNIEnv* env, jclass) {
    return jstr(env, engine::recJson());
}
JNIEXPORT void JNICALL JNI_FN(recSet)(JNIEnv* env, jclass, jstring key, jdouble value) {
    engine::recSet(str(env, key), value);
}
JNIEXPORT jstring JNICALL JNI_FN(sstvJson)(JNIEnv* env, jclass) {
    return jstr(env, engine::sstvJson());
}
JNIEXPORT void JNICALL JNI_FN(sstvSet)(JNIEnv* env, jclass, jstring key, jdouble value) {
    engine::sstvSet(str(env, key), value);
}
JNIEXPORT jintArray JNICALL JNI_FN(sstvImage)(JNIEnv* env, jclass, jint which) {
    int w = 0, h = 0;
    auto px = engine::sstvImage(which, w, h);
    // [width, height, ARGB pixels...]
    jintArray arr = env->NewIntArray((jsize)px.size() + 2);
    jint dims[2] = { w, h };
    env->SetIntArrayRegion(arr, 0, 2, dims);
    if (!px.empty()) { env->SetIntArrayRegion(arr, 2, (jsize)px.size(), (const jint*)px.data()); }
    return arr;
}
JNIEXPORT jdoubleArray JNICALL JNI_FN(ctyLatLon)(JNIEnv* env, jclass, jobjectArray calls) {
    std::vector<std::string> v;
    jsize n = env->GetArrayLength(calls);
    for (jsize i = 0; i < n; i++) {
        auto s = (jstring)env->GetObjectArrayElement(calls, i);
        v.push_back(str(env, s));
        env->DeleteLocalRef(s);
    }
    auto r = engine::ctyLatLon(v);
    jdoubleArray arr = env->NewDoubleArray((jsize)r.size());
    env->SetDoubleArrayRegion(arr, 0, (jsize)r.size(), r.data());
    return arr;
}
JNIEXPORT jstring JNICALL JNI_FN(rdsJson)(JNIEnv* env, jclass) {
    return jstr(env, engine::rdsJson());
}
JNIEXPORT void JNICALL JNI_FN(setVolume)(JNIEnv*, jclass, jfloat v) { engine::setVolume(v); }

// ---------------------------------------------------------------- display
JNIEXPORT void JNICALL JNI_FN(setView)(JNIEnv*, jclass, jdouble viewBw, jdouble viewOffset) {
    engine::setView(viewBw, viewOffset);
    auto p = spectrum::view();
    p.viewBandwidth = viewBw;
    p.viewOffset = viewOffset;
    spectrum::setView(p);
}

JNIEXPORT void JNICALL JNI_FN(setDisplay)(JNIEnv*, jclass, jfloat dbMin, jfloat dbMax, jfloat specFraction,
                                          jint fill, jint line, jint background, jfloat lineWidthPx, jint palette) {
    auto p = spectrum::view();
    p.dbMin = dbMin;
    p.dbMax = dbMax;
    p.spectrumFraction = specFraction;
    argbToFloat(fill, p.fillColor);
    argbToFloat(line, p.lineColor);
    argbToFloat(background, p.background);
    p.lineWidthPx = lineWidthPx;
    p.palette = palette;
    spectrum::setView(p);
}

JNIEXPORT void JNICALL JNI_FN(surfaceAttach)(JNIEnv* env, jclass, jobject surface) {
    ANativeWindow* w = ANativeWindow_fromSurface(env, surface);
    if (w) { spectrum::attach(w); }
}

JNIEXPORT void JNICALL JNI_FN(surfaceResize)(JNIEnv*, jclass, jint w, jint h) { spectrum::resize(w, h); }
JNIEXPORT void JNICALL JNI_FN(surfaceDetach)(JNIEnv*, jclass) { spectrum::detach(); }

} // extern "C"
