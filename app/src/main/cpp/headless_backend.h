#pragma once
#include <jni.h>
#include <string>
#include <module.h>

// Headless replacement for SDR++Brown's Android backend: no NativeActivity,
// no ImGui rendering. The Kotlin app owns the UI and feeds us the USB device.
namespace headless {
    SDRPP_EXPORT void setJavaVM(JavaVM* vm, jobject bridgeClassGlobalRef);
    SDRPP_EXPORT void setUsbDevice(int fd, int vid, int pid);
    SDRPP_EXPORT void clearUsbDevice();
    SDRPP_EXPORT JNIEnv* attachEnv(bool& didAttach);
    SDRPP_EXPORT void detachEnv(bool didAttach);
}
