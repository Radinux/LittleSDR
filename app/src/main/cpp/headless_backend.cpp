// Implements the backend:: API that SDR++Brown's core and modules expect,
// without any window or GL context.
#include "headless_backend.h"
#include <backend.h>
#include <android_backend.h>
#include <utils/flog.h>
#include <atomic>
#include <mutex>

namespace {
    JavaVM* g_vm = nullptr;
    jclass g_bridge = nullptr; // app.littlesdr.engine.NativeBridge (global ref)

    std::mutex g_usbMtx;
    int g_fd = -1, g_vid = -1, g_pid = -1;

    std::string callStaticString(const char* method, const std::string* arg) {
        bool attached = false;
        JNIEnv* env = headless::attachEnv(attached);
        if (!env || !g_bridge) { return ""; }
        std::string result;
        jmethodID mid = arg
            ? env->GetStaticMethodID(g_bridge, method, "(Ljava/lang/String;)Ljava/lang/String;")
            : env->GetStaticMethodID(g_bridge, method, "()Ljava/lang/String;");
        if (mid) {
            jstring jarg = arg ? env->NewStringUTF(arg->c_str()) : nullptr;
            auto jres = (jstring)(arg ? env->CallStaticObjectMethod(g_bridge, mid, jarg)
                                      : env->CallStaticObjectMethod(g_bridge, mid));
            if (env->ExceptionCheck()) { env->ExceptionDescribe(); env->ExceptionClear(); }
            if (jres) {
                const char* s = env->GetStringUTFChars(jres, nullptr);
                result = s;
                env->ReleaseStringUTFChars(jres, s);
                env->DeleteLocalRef(jres);
            }
            if (jarg) { env->DeleteLocalRef(jarg); }
        }
        else {
            env->ExceptionClear();
            flog::error("NativeBridge.{} not found", method);
        }
        headless::detachEnv(attached);
        return result;
    }
}

namespace headless {
    void setJavaVM(JavaVM* vm, jobject bridgeClassGlobalRef) {
        g_vm = vm;
        g_bridge = (jclass)bridgeClassGlobalRef;
    }

    void setUsbDevice(int fd, int vid, int pid) {
        std::lock_guard<std::mutex> lck(g_usbMtx);
        g_fd = fd; g_vid = vid; g_pid = pid;
        flog::info("USB device set: fd={} vid={:04x} pid={:04x}", fd, vid, pid);
    }

    void clearUsbDevice() {
        std::lock_guard<std::mutex> lck(g_usbMtx);
        g_fd = -1; g_vid = -1; g_pid = -1;
    }

    JNIEnv* attachEnv(bool& didAttach) {
        didAttach = false;
        if (!g_vm) { return nullptr; }
        JNIEnv* env = nullptr;
        jint r = g_vm->GetEnv((void**)&env, JNI_VERSION_1_6);
        if (r == JNI_EDETACHED) {
            if (g_vm->AttachCurrentThread(&env, nullptr) != JNI_OK) { return nullptr; }
            didAttach = true;
        }
        else if (r != JNI_OK) {
            return nullptr;
        }
        return env;
    }

    void detachEnv(bool didAttach) {
        if (didAttach && g_vm) { g_vm->DetachCurrentThread(); }
    }
}

namespace backend {
    // ---- Rendering API: unused, the UI is Compose + our Vulkan renderer
    int init(std::string) { return 0; }
    void beginFrame() {}
    void render(bool) {}
    void getMouseScreenPos(double& x, double& y) { x = 0; y = 0; }
    void setMouseScreenPos(double, double) {}
    int renderLoop() { return 0; }
    int end() { return 0; }

    // ---- Android helpers used by modules
    int getDeviceFD(int& vid, int& pid, const std::vector<DevVIDPID>& allowedVidPids) {
        std::lock_guard<std::mutex> lck(g_usbMtx);
        vid = g_vid;
        pid = g_pid;
        if (g_fd < 0) { return -1; }
        if (allowedVidPids.empty()) { return g_fd; }
        for (auto const& vp : allowedVidPids) {
            if (vp.vid == g_vid && vp.pid == g_pid) { return g_fd; }
        }
        return -1;
    }

    std::string httpGet(const std::string& url) { return callStaticString("httpGet", &url); }
    std::string getAudioSinkIds() { return ""; }
    std::string getAudioSourceIds() { return ""; }
    std::string scanAudioDevices() { return ""; }
    std::string startBtSco() { return ""; }
    std::string stopBtSco() { return ""; }
    void doPermissionsDialogs() {}
    std::string getBatteryLevel() { return callStaticString("batteryLevel", nullptr); }
    void androidHapticFeedback() {}

    const std::vector<DevVIDPID> AIRSPY_VIDPIDS = { { 0x1d50, 0x60a1 } };
    const std::vector<DevVIDPID> AIRSPYHF_VIDPIDS = { { 0x03EB, 0x800C } };
    const std::vector<DevVIDPID> HACKRF_VIDPIDS = { { 0x1d50, 0x604b }, { 0x1d50, 0x6089 }, { 0x1d50, 0xcc15 } };
    const std::vector<DevVIDPID> HYDRASDR_VIDPIDS = { { 0x1d50, 0x60a1 }, { 0x38af, 0x0001 } };
    const std::vector<DevVIDPID> RTL_SDR_VIDPIDS = {
        { 0x0bda, 0x2832 }, { 0x0bda, 0x2838 }, { 0x0413, 0x6680 }, { 0x0413, 0x6f0f },
        { 0x0458, 0x707f }, { 0x0ccd, 0x00a9 }, { 0x0ccd, 0x00b3 }, { 0x0ccd, 0x00b4 },
        { 0x0ccd, 0x00b5 }, { 0x0ccd, 0x00b7 }, { 0x0ccd, 0x00b8 }, { 0x0ccd, 0x00b9 },
        { 0x0ccd, 0x00c0 }, { 0x0ccd, 0x00c6 }, { 0x0ccd, 0x00d3 }, { 0x0ccd, 0x00d7 },
        { 0x0ccd, 0x00e0 }, { 0x1554, 0x5020 }, { 0x15f4, 0x0131 }, { 0x15f4, 0x0133 },
        { 0x185b, 0x0620 }, { 0x185b, 0x0650 }, { 0x185b, 0x0680 }, { 0x1b80, 0xd393 },
        { 0x1b80, 0xd394 }, { 0x1b80, 0xd395 }, { 0x1b80, 0xd397 }, { 0x1b80, 0xd398 },
        { 0x1b80, 0xd39d }, { 0x1b80, 0xd3a4 }, { 0x1b80, 0xd3a8 }, { 0x1b80, 0xd3af },
        { 0x1b80, 0xd3b0 }, { 0x1d19, 0x1101 }, { 0x1d19, 0x1102 }, { 0x1d19, 0x1103 },
        { 0x1d19, 0x1104 }, { 0x1f4d, 0xa803 }, { 0x1f4d, 0xb803 }, { 0x1f4d, 0xc803 },
        { 0x1f4d, 0xd286 }, { 0x1f4d, 0xd803 }
    };
}
