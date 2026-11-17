// Minimal AAudio playback sink for LittleSDR.
// Replaces SDR++Brown's android_audio_sink, which depends on NativeActivity
// (microphone/TX, Bluetooth SCO via JNI, ImGui menu). Registered as "Audio".
#include <module.h>
#include <signal_path/signal_path.h>
#include <signal_path/sink.h>
#include <dsp/buffer/packer.h>
#include <utils/flog.h>
#include <aaudio/AAudio.h>
#include <core.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <mutex>
#include <set>
#include <thread>

SDRPP_MOD_INFO{
    /* Name:            */ "audio_sink",
    /* Description:     */ "AAudio playback sink (LittleSDR)",
    /* Author:          */ "LittleSDR",
    /* Version:         */ 0, 1, 0,
    /* Max instances    */ 1
};

// Settings shared by all sink instances, driven by the engine through the module interface
namespace {
    std::atomic<int> g_deviceId{ AAUDIO_UNSPECIFIED }; // 0 = system default route
    std::atomic<int> g_bursts{ 4 };                    // buffer size in bursts (latency vs. dropouts)
}
class AAudioSink;
static std::mutex g_sinksMtx;
static std::set<AAudioSink*> g_sinks;

class AAudioSink : SinkManager::Sink {
public:
    AAudioSink(SinkManager::Stream* stream, std::string streamName) : _stream(stream), _streamName(streamName) {
        packer.init(_stream->sinkOut, 480); // 10 ms at 48 kHz
        _stream->setSampleRate(sampleRate);
        std::lock_guard<std::mutex> l(g_sinksMtx);
        g_sinks.insert(this);
    }

    ~AAudioSink() {
        {
            std::lock_guard<std::mutex> l(g_sinksMtx);
            g_sinks.erase(this);
        }
        stop();
    }

    /** Settings changed: reopen the stream from the worker thread on the next buffer. */
    void requestReopen() { reopenRequested = true; }

    void start() override {
        if (running) { return; }
        running = true;
        openStream();
        packer.start();
        worker = std::thread(&AAudioSink::workerLoop, this);
    }

    void stop() override {
        if (!running) { return; }
        running = false;
        packer.stop();
        packer.out.stopReader();
        if (worker.joinable()) { worker.join(); }
        packer.out.clearReadStop();
        closeStream();
    }

    void menuHandler() override {}

private:
    void openStream() {
        std::lock_guard<std::mutex> lck(streamMtx);
        AAudioStreamBuilder* b = nullptr;
        if (AAudio_createStreamBuilder(&b) != AAUDIO_OK) { flog::error("AAudio: builder failed"); return; }
        AAudioStreamBuilder_setDirection(b, AAUDIO_DIRECTION_OUTPUT);
        AAudioStreamBuilder_setSharingMode(b, AAUDIO_SHARING_MODE_SHARED);
        AAudioStreamBuilder_setPerformanceMode(b, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
        AAudioStreamBuilder_setUsage(b, AAUDIO_USAGE_MEDIA);
        AAudioStreamBuilder_setContentType(b, AAUDIO_CONTENT_TYPE_SPEECH);
        AAudioStreamBuilder_setSampleRate(b, (int32_t)sampleRate);
        AAudioStreamBuilder_setChannelCount(b, 2);
        AAudioStreamBuilder_setFormat(b, AAUDIO_FORMAT_PCM_FLOAT);
        AAudioStreamBuilder_setErrorCallback(b, &AAudioSink::onError, this);
        if (g_deviceId.load() > 0) { AAudioStreamBuilder_setDeviceId(b, g_deviceId.load()); }
        aaudio_result_t r = AAudioStreamBuilder_openStream(b, &stream);
        AAudioStreamBuilder_delete(b);
        if (r != AAUDIO_OK) {
            flog::error("AAudio: open failed: {}", AAudio_convertResultToText(r));
            stream = nullptr;
            return;
        }
        // ~4 bursts of buffering: low latency without underruns from the DSP thread jitter
        int32_t burst = AAudioStream_getFramesPerBurst(stream);
        AAudioStream_setBufferSizeInFrames(stream, burst * g_bursts.load());
        r = AAudioStream_requestStart(stream);
        if (r != AAUDIO_OK) {
            flog::error("AAudio: start failed: {}", AAudio_convertResultToText(r));
            AAudioStream_close(stream);
            stream = nullptr;
            return;
        }
        flog::info("AAudio: playback started ({} Hz, burst {})", AAudioStream_getSampleRate(stream), burst);
    }

    void closeStream() {
        std::lock_guard<std::mutex> lck(streamMtx);
        if (!stream) { return; }
        AAudioStream_requestStop(stream);
        AAudioStream_close(stream);
        stream = nullptr;
    }

    void workerLoop() {
        while (true) {
            int count = packer.out.read();
            if (count < 0) { return; }
            {
                std::lock_guard<std::mutex> lck(streamMtx);
                if (stream) {
                    AAudioStream_write(stream, packer.out.readBuf, count, 100 * 1000 * 1000LL);
                }
            }
            packer.out.flush();
            if (reopenRequested.exchange(false)) {
                closeStream();
                openStream();
            }
        }
    }

    // Called on an AAudio thread: headphones plugged/unplugged, BT changes...
    static void onError(AAudioStream*, void* user, aaudio_result_t error) {
        auto _this = (AAudioSink*)user;
        flog::warn("AAudio error: {}", AAudio_convertResultToText(error));
        if (error == AAUDIO_ERROR_DISCONNECTED) { _this->reopenRequested = true; }
    }

    SinkManager::Stream* _stream;
    std::string _streamName;
    dsp::buffer::Packer<dsp::stereo_t> packer;
    std::thread worker;
    std::mutex streamMtx;
    AAudioStream* stream = nullptr;
    std::atomic<bool> reopenRequested{ false };
    double sampleRate = 48000.0;
    bool running = false;
};

class AAudioSinkModule : public ModuleManager::Instance {
public:
    explicit AAudioSinkModule(std::string name) : name(name) {
        provider.create = [](SinkManager::Stream* stream, std::string streamName, void*) -> SinkManager::Sink* {
            return (SinkManager::Sink*)(new AAudioSink(stream, streamName));
        };
        provider.ctx = this;
        sigpath::sinkManager.registerSinkProvider("Audio", provider);
        core::modComManager.registerInterface("audio_sink", name, &AAudioSinkModule::interfaceHandler, this);
    }
    ~AAudioSinkModule() {
        core::modComManager.unregisterInterface(name);
        sigpath::sinkManager.unregisterSinkProvider("Audio");
    }

    // 0: set output device id (int, 0 = default), 1: set buffer size in bursts (int)
    static void interfaceHandler(int code, void* in, void*, void*) {
        if (!in) { return; }
        int v = *(int*)in;
        if (code == 0) { g_deviceId = v > 0 ? v : AAUDIO_UNSPECIFIED; }
        else if (code == 1) { g_bursts = std::clamp(v, 2, 16); }
        else { return; }
        std::lock_guard<std::mutex> l(g_sinksMtx);
        for (auto s : g_sinks) { s->requestReopen(); }
    }
    void postInit() override {}
    void enable() override { enabled = true; }
    void disable() override { enabled = false; }
    bool isEnabled() override { return enabled; }

private:
    std::string name;
    bool enabled = true;
    SinkManager::SinkProvider provider;
};

MOD_EXPORT void _INIT_() {}
MOD_EXPORT void* _CREATE_INSTANCE_(std::string name) { return new AAudioSinkModule(name); }
MOD_EXPORT void _DELETE_INSTANCE_(void* instance) { delete (AAudioSinkModule*)instance; }
MOD_EXPORT void _END_() {}
