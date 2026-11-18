// Classic (analog) SSTV decoder: Martin, Scottie, Robot and PD modes, VIS auto-detection.
// Fed with demodulated audio (any sample rate), it produces an ARGB image line by line.
// Written for LittleSDR: SDR++Brown has no analog SSTV decoder.
#pragma once
#include <complex>
#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

namespace sstv {

    enum class Color {
        GBR,     // Martin / Scottie: green, blue, red scans
        Robot36, // Y, then R-Y or B-Y on alternate lines (4:2:0)
        Robot72, // Y, R-Y, B-Y on every line (4:2:2)
        PD,      // Y(even row), R-Y, B-Y, Y(odd row): one scan line = two image rows
    };

    struct Mode {
        const char* name;
        int vis;
        int width, height; // image size (PD: height = 2 x scan lines)
        double lineMs;     // one scan line
        double syncMs;     // sync pulse (1200 Hz) length
        double syncAtMs;   // sync start within the scan line
        double startMs;    // first scan line start, after the VIS stop bit
        Color color;
        int segs;
        double segStart[4]; // ms from the scan line start
        double segLen[4];   // ms
        int scanLines() const { return color == Color::PD ? height / 2 : height; }
    };

    const std::vector<Mode>& modes();

    struct Status {
        bool receiving = false;
        std::string mode;     // current or last image
        int line = 0;         // decoded image rows
        int height = 0, width = 0;
        uint64_t version = 0; // +1 whenever the current image changes
        uint64_t finished = 0; // +1 per finished image (complete or signal lost)
        int finishedRows = 0; // rows of the last finished image
        std::string finishedMode;
        bool lastComplete = false;
        double freq = 0.0;    // smoothed audio frequency (Hz), tuning aid
        double slantPpm = 0.0; // measured clock deviation of the current image
    };

    class Decoder {
    public:
        Decoder();

        /** Mono audio at sampleRate Hz (audio thread). */
        void feed(const float* in, int count, double sampleRate);

        /** Forget everything (decoder switched off). */
        void reset();
        /** Start an image now in a given mode (index in modes()), for a missed VIS. */
        void startManual(int modeIndex);
        /** End the current image now (kept as finished if it has enough rows). */
        void finishNow();

        Status status();
        /** ARGB pixels of the current image (which = 0) or the last finished one (1). */
        std::vector<uint32_t> image(int which, int& w, int& h);

    private:
        // ---- front end: complex mix to baseband, low-pass, decimate, FM discriminator
        void configure(double sampleRate);
        void pushFreq(float f);

        // ---- frequency history (decimated rate), with running sums for O(1) averages
        double mean(int64_t a, int64_t b) const; // [a, b) absolute indices
        int64_t ms(double m) const { return (int64_t)(m * rate_ / 1000.0 + 0.5); }

        // ---- VIS
        void detectVis();
        bool visAt(int64_t end, int& code, double& err) const;

        // ---- image
        void beginImage(int modeIndex, double t0, bool acquire);
        void decodeReady();
        void decodeLine(int n);
        bool findSync(int n, double& start);
        void refit();
        void endImage(bool complete);
        double pixel(double t0Samples, double startMs, double lenMs, int x, int w) const;
        void renderRow(int row);

        std::mutex mtx_;

        double fs_ = 0.0, rate_ = 0.0;
        int decim_ = 1, phaseCount_ = 0;
        double oscPhase_ = 0.0, oscStep_ = 0.0;
        std::vector<float> taps_;
        std::vector<std::complex<float>> hist_;
        int histPos_ = 0;
        std::complex<float> prevZ_{ 1.0f, 0.0f };

        static constexpr double HIST_SEC = 8.0;
        std::vector<float> freq_;
        std::vector<double> cum_; // cum_[i % cap] = sum of freq_ up to absolute index i (inclusive)
        int64_t count_ = 0;       // absolute index of the next sample
        double smooth_ = 1900.0;

        // VIS search
        int64_t visBestEnd_ = -1, visCandidateUntil_ = -1, visIgnoreUntil_ = 0;
        int visBestCode_ = -1;
        double visBestErr_ = 1e9;
        int64_t lastVisCheck_ = 0;

        // Current image
        int modeIdx_ = -1;
        bool acquire_ = false;
        double t0_ = 0.0, period_ = 0.0, nominalPeriod_ = 0.0;
        int nextLine_ = 0, missed_ = 0, decodedRows_ = 0;
        std::vector<std::pair<int, double>> syncs_; // scan line, measured line start (samples)
        std::vector<uint32_t> img_, done_;
        int doneW_ = 0, doneH_ = 0;
        // Per scan line channel values (0-255), for the YUV modes
        std::vector<std::vector<uint8_t>> chan_[4];
        std::vector<int> robotChroma_; // Robot36: 0 = R-Y, 1 = B-Y, -1 unknown

        Status st_;
    };
}
