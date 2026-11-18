#include "sstv_decoder.h"

#include <algorithm>
#include <cmath>

namespace sstv {

    namespace {
        constexpr double PI = 3.14159265358979323846;
        constexpr double CENTER_HZ = 1900.0; // mixing frequency: middle of the 1100-2300 Hz SSTV range
        constexpr double TARGET_RATE = 12000.0;
        constexpr int TAPS = 65;
        constexpr double CUTOFF_HZ = 1300.0; // keeps 600-3200 Hz of audio
        constexpr uint32_t BLANK = 0xFF15171Cu;

        Mode martin(const char* name, int vis, double scan) {
            const double sync = 4.862, gap = 0.572;
            Mode m{ name, vis, 320, 256, sync + gap + 3 * (scan + gap), sync, 0.0, 0.0, Color::GBR, 3, {}, {} };
            for (int i = 0; i < 3; i++) {
                m.segStart[i] = sync + gap + i * (scan + gap);
                m.segLen[i] = scan;
            }
            return m;
        }

        // Sync pulse in the middle of the line, before the red scan; a 9 ms "starting sync"
        // follows the VIS, so the first line begins 9 ms after it
        Mode scottie(const char* name, int vis, double scan) {
            const double sep = 1.5, sync = 9.0;
            Mode m{ name, vis, 320, 256, 3 * scan + 3 * sep + sync, sync, 2 * sep + 2 * scan, sync, Color::GBR, 3, {}, {} };
            m.segStart[0] = sep;
            m.segStart[1] = 2 * sep + scan;
            m.segStart[2] = 2 * sep + 2 * scan + sync + sep;
            for (int i = 0; i < 3; i++) { m.segLen[i] = scan; }
            return m;
        }

        Mode pd(const char* name, int vis, int w, int h, double pixelMs) {
            const double sync = 20.0, porch = 2.08, scan = w * pixelMs;
            Mode m{ name, vis, w, h, sync + porch + 4 * scan, sync, 0.0, 0.0, Color::PD, 4, {}, {} };
            for (int i = 0; i < 4; i++) {
                m.segStart[i] = sync + porch + i * scan;
                m.segLen[i] = scan;
            }
            return m;
        }

        uint8_t clamp8(double v) { return (uint8_t)std::clamp(v, 0.0, 255.0); }

        uint32_t rgb(double r, double g, double b) {
            return 0xFF000000u | ((uint32_t)clamp8(r) << 16) | ((uint32_t)clamp8(g) << 8) | clamp8(b);
        }

        // ITU-R 601 studio range (Y 16-235), as MMSSTV encodes and QSSTV / Robot36 decode
        uint32_t yuv(double y, double cr, double cb) {
            const double l = 1.164 * (y - 16.0);
            return rgb(l + 1.596 * (cr - 128.0), l - 0.813 * (cr - 128.0) - 0.391 * (cb - 128.0), l + 2.018 * (cb - 128.0));
        }

        int findMode(int vis) {
            auto& ms = modes();
            for (int i = 0; i < (int)ms.size(); i++) {
                if (ms[i].vis == vis) { return i; }
            }
            return -1;
        }
    }

    const std::vector<Mode>& modes() {
        static const std::vector<Mode> list = [] {
            std::vector<Mode> v;
            v.push_back(martin("Martin M1", 44, 146.432));
            v.push_back(martin("Martin M2", 40, 73.216));
            v.push_back(scottie("Scottie S1", 60, 138.24));
            v.push_back(scottie("Scottie S2", 56, 88.064));
            v.push_back(scottie("Scottie DX", 76, 345.6));
            // Robot 36: sync 9, porch 3, Y 88, separator 4.5 (1500 Hz: R-Y, 2300 Hz: B-Y), porch 1.5, chroma 44
            v.push_back(Mode{ "Robot 36", 8, 320, 240, 150.0, 9.0, 0.0, 0.0, Color::Robot36, 2, { 12.0, 106.0 }, { 88.0, 44.0 } });
            // Robot 72: sync 9, porch 3, Y 138, sep 4.5 + porch 1.5, R-Y 69, sep 4.5 + porch 1.5, B-Y 69
            v.push_back(Mode{ "Robot 72", 12, 320, 240, 300.0, 9.0, 0.0, 0.0, Color::Robot72, 3, { 12.0, 156.0, 231.0 }, { 138.0, 69.0, 69.0 } });
            v.push_back(pd("PD 50", 93, 320, 256, 0.286));
            v.push_back(pd("PD 90", 99, 320, 256, 0.532));
            v.push_back(pd("PD 120", 95, 640, 496, 0.19));
            v.push_back(pd("PD 160", 98, 512, 400, 0.382));
            v.push_back(pd("PD 180", 96, 640, 496, 0.286));
            v.push_back(pd("PD 240", 97, 640, 496, 0.382));
            v.push_back(pd("PD 290", 94, 800, 616, 0.286));
            return v;
        }();
        return list;
    }

    Decoder::Decoder() {}

    // ------------------------------------------------------------------ front end
    void Decoder::configure(double sampleRate) {
        fs_ = sampleRate;
        decim_ = std::max(1, (int)std::lround(sampleRate / TARGET_RATE));
        rate_ = sampleRate / decim_;
        oscStep_ = 2.0 * PI * CENTER_HZ / sampleRate;
        oscPhase_ = 0.0;
        phaseCount_ = 0;

        // Windowed-sinc low-pass (Hamming), unity DC gain
        taps_.assign(TAPS, 0.0f);
        const double fc = CUTOFF_HZ / sampleRate;
        double sum = 0.0;
        for (int i = 0; i < TAPS; i++) {
            const double k = i - (TAPS - 1) / 2.0;
            const double sinc = k == 0 ? 2.0 * fc : std::sin(2.0 * PI * fc * k) / (PI * k);
            const double w = 0.54 - 0.46 * std::cos(2.0 * PI * i / (TAPS - 1));
            taps_[i] = (float)(sinc * w);
            sum += taps_[i];
        }
        for (auto& t : taps_) { t = (float)(t / sum); }
        hist_.assign(TAPS, { 0.0f, 0.0f });
        histPos_ = 0;
        prevZ_ = { 1.0f, 0.0f };

        const size_t cap = (size_t)(HIST_SEC * rate_);
        freq_.assign(cap, (float)CENTER_HZ);
        cum_.assign(cap, 0.0);
        count_ = 0;
        lastVisCheck_ = 0;
        visIgnoreUntil_ = 0;
        visCandidateUntil_ = -1;
        visBestErr_ = 1e9;
        // The time base changed: an image in progress cannot continue
        if (modeIdx_ >= 0) { endImage(false); }
    }

    void Decoder::pushFreq(float f) {
        f = std::clamp(f, 900.0f, 2700.0f);
        const size_t cap = freq_.size();
        const size_t i = (size_t)(count_ % (int64_t)cap);
        const double prev = count_ > 0 ? cum_[(size_t)((count_ - 1) % (int64_t)cap)] : 0.0;
        freq_[i] = f;
        cum_[i] = prev + f;
        count_++;
        smooth_ += (f - smooth_) * 0.002;
    }

    double Decoder::mean(int64_t a, int64_t b) const {
        const int64_t cap = (int64_t)freq_.size();
        if (cap == 0 || b <= a || b > count_ || a - 1 < count_ - cap || a < 0) { return CENTER_HZ; }
        const double hi = cum_[(size_t)((b - 1) % cap)];
        const double lo = a > 0 ? cum_[(size_t)((a - 1) % cap)] : 0.0;
        return (hi - lo) / (double)(b - a);
    }

    void Decoder::feed(const float* in, int count, double sampleRate) {
        std::lock_guard<std::mutex> lck(mtx_);
        if (sampleRate <= 0) { return; }
        if (std::fabs(sampleRate - fs_) > 0.5) { configure(sampleRate); }
        const double toHz = rate_ / (2.0 * PI);
        for (int n = 0; n < count; n++) {
            const float x = in[n];
            hist_[histPos_] = { (float)(x * std::cos(oscPhase_)), (float)(-x * std::sin(oscPhase_)) };
            histPos_ = (histPos_ + 1) % TAPS;
            oscPhase_ += oscStep_;
            if (oscPhase_ > 2.0 * PI) { oscPhase_ -= 2.0 * PI; }
            if (++phaseCount_ < decim_) { continue; }
            phaseCount_ = 0;
            std::complex<float> z{ 0.0f, 0.0f };
            for (int k = 0; k < TAPS; k++) { z += taps_[k] * hist_[(histPos_ + k) % TAPS]; }
            const std::complex<float> d = z * std::conj(prevZ_);
            prevZ_ = z;
            pushFreq((float)(CENTER_HZ + std::arg(d) * toHz));
        }
        detectVis();
        decodeReady();
    }

    // ------------------------------------------------------------------ VIS
    // Leader 1900 Hz (300 ms), start bit 1200 Hz, 7 data bits LSB first + even parity
    // (1100 Hz = 1, 1300 Hz = 0), stop bit 1200 Hz; 30 ms per bit. "end" = end of the stop bit.
    // Tolerant on purpose: only the last 190 ms of leader are checked, and some encoders send the
    // start bit at 1900 Hz (e.g. leader, 10 ms break, 30 ms of 1900 Hz, then the data bits).
    // Parity, stop bit and the list of known modes keep false detections out.
    bool Decoder::visAt(int64_t end, int& code, double& err) const {
        const int64_t s = end - ms(300);
        if (s - ms(190) < count_ - (int64_t)freq_.size() + 1 || s - ms(190) < 0) { return false; }
        auto m = [&](double a, double b) { return mean(s + ms(a), s + ms(b)); };
        err = 0.0;
        for (double a : { -190.0, -100.0 }) {
            const double v = m(a, a + 80.0);
            if (std::fabs(v - 1900.0) > 80.0) { return false; }
            err += std::fabs(v - 1900.0);
        }
        const double start = m(5, 25), stop = m(275, 295);
        const double startDev = std::min(std::fabs(start - 1200.0), std::fabs(start - 1900.0));
        if (startDev > 100.0 || std::fabs(stop - 1200.0) > 100.0) { return false; }
        err += startDev + std::fabs(stop - 1200.0);
        int bits = 0, ones = 0;
        for (int k = 0; k < 8; k++) {
            const double v = m(35.0 + 30.0 * k, 55.0 + 30.0 * k);
            if (v < 1000.0 || v > 1400.0 || std::fabs(v - 1200.0) < 25.0) { return false; }
            const bool one = v < 1200.0;
            err += std::fabs(v - (one ? 1100.0 : 1300.0));
            if (k < 7) {
                bits |= (one ? 1 : 0) << k;
                ones += one;
            }
            else if ((ones & 1) != (one ? 1 : 0)) { return false; } // even parity
        }
        code = bits;
        return true;
    }

    void Decoder::detectVis() {
        const int64_t step = std::max<int64_t>(1, ms(1.0));
        for (int64_t e = lastVisCheck_ + step; e <= count_; e += step) {
            lastVisCheck_ = e;
            if (e < visIgnoreUntil_) { continue; }
            int code = -1;
            double err = 0.0;
            if (visAt(e, code, err) && findMode(code) >= 0) {
                // Keep the best alignment over the next 15 ms
                if (visCandidateUntil_ < 0) { visCandidateUntil_ = e + ms(15); }
                if (err < visBestErr_) {
                    visBestErr_ = err;
                    visBestEnd_ = e;
                    visBestCode_ = code;
                }
            }
            if (visCandidateUntil_ >= 0 && e >= visCandidateUntil_) {
                const int mi = findMode(visBestCode_);
                // A VIS inside a clean image is a false alarm; after lost syncs, a new transmission
                if (mi >= 0 && (modeIdx_ < 0 || missed_ >= 3)) {
                    if (modeIdx_ >= 0) { endImage(false); }
                    beginImage(mi, (double)visBestEnd_ + modes()[mi].startMs * rate_ / 1000.0, false);
                }
                visIgnoreUntil_ = visBestEnd_ + ms(1000);
                visCandidateUntil_ = -1;
                visBestErr_ = 1e9;
            }
        }
    }

    // ------------------------------------------------------------------ image
    void Decoder::beginImage(int mi, double t0, bool acquire) {
        const Mode& m = modes()[mi];
        modeIdx_ = mi;
        acquire_ = acquire;
        t0_ = t0;
        nominalPeriod_ = period_ = m.lineMs * rate_ / 1000.0;
        nextLine_ = missed_ = decodedRows_ = 0;
        syncs_.clear();
        img_.assign((size_t)m.width * m.height, BLANK);
        const int lines = m.scanLines();
        for (auto& c : chan_) { c.clear(); }
        if (m.color == Color::Robot36) {
            chan_[0].assign(lines, std::vector<uint8_t>(m.width, 0));
            chan_[1].assign(lines, std::vector<uint8_t>(m.width, 128));
            robotChroma_.assign(lines, -1);
        }
        st_.receiving = true;
        st_.mode = m.name;
        st_.width = m.width;
        st_.height = m.height;
        st_.line = 0;
        st_.slantPpm = 0.0;
        st_.version++;
    }

    void Decoder::decodeReady() {
        if (modeIdx_ < 0) { return; }
        const Mode& m = modes()[modeIdx_];
        const int lines = m.scanLines();
        while (modeIdx_ >= 0 && nextLine_ < lines) {
            // Whole line + sync search margin received (first line of a manual start: two lines)
            const double need = t0_ + (nextLine_ + (acquire_ ? 2 : 1)) * period_ + ms(m.syncMs + 12.0);
            if ((double)count_ < need) { break; }
            decodeLine(nextLine_++);
            // Long silence or noise: nothing left to decode
            if (missed_ >= 12) {
                endImage(false);
                return;
            }
        }
        if (modeIdx_ >= 0 && nextLine_ >= lines) { endImage(true); }
    }

    bool Decoder::findSync(int n, double& start) {
        const Mode& m = modes()[modeIdx_];
        const int64_t len = std::max<int64_t>(1, ms(m.syncMs));
        const double expected = t0_ + n * period_ + m.syncAtMs * rate_ / 1000.0;
        const int64_t w = acquire_ ? (int64_t)period_ : ms(6.0);
        const int64_t c = (int64_t)std::llround(expected);
        double best = 1e9;
        int64_t bestP = c;
        for (int64_t p = c - (acquire_ ? 0 : w); p <= c + w; p++) {
            const double v = mean(p, p + len);
            if (v < best) {
                best = v;
                bestP = p;
            }
        }
        if (best > 1350.0) { return false; }
        start = (double)bestP - m.syncAtMs * rate_ / 1000.0; // line start
        return true;
    }

    void Decoder::refit() {
        // Robust line fit of the measured line starts: start(n) = t0 + n * period
        const double maxDev = nominalPeriod_ * 0.01;
        auto fit = [&](const std::vector<std::pair<int, double>>& pts, double& t0, double& per) {
            const int lo = pts.front().first, hi = pts.back().first;
            if (pts.size() < 3 || hi - lo < 6) {
                per = period_;
                double s = 0.0;
                for (auto& p : pts) { s += p.second - p.first * per; }
                t0 = s / pts.size();
                return;
            }
            double sn = 0, sy = 0, snn = 0, sny = 0;
            for (auto& p : pts) {
                sn += p.first;
                sy += p.second;
                snn += (double)p.first * p.first;
                sny += p.first * p.second;
            }
            const double k = (double)pts.size();
            const double den = k * snn - sn * sn;
            per = den != 0.0 ? (k * sny - sn * sy) / den : period_;
            per = std::clamp(per, nominalPeriod_ - maxDev, nominalPeriod_ + maxDev);
            double s = 0.0;
            for (auto& p : pts) { s += p.second - p.first * per; }
            t0 = s / k;
        };
        double t0 = t0_, per = period_;
        fit(syncs_, t0, per);
        std::vector<std::pair<int, double>> inl;
        for (auto& p : syncs_) {
            if (std::fabs(p.second - (t0 + p.first * per)) < ms(1.5)) { inl.push_back(p); }
        }
        if (inl.size() >= 2) { fit(inl, t0, per); }
        t0_ = t0;
        period_ = per;
    }

    double Decoder::pixel(double base, double startMs, double lenMs, int x, int w) const {
        const double pd = lenMs / w;
        const double c = startMs + (x + 0.5) * pd;
        const double k = rate_ / 1000.0;
        const int64_t a = (int64_t)std::floor(base + (c - pd / 2) * k + 0.5);
        const int64_t b = std::max(a + 1, (int64_t)std::floor(base + (c + pd / 2) * k + 0.5));
        return std::clamp((mean(a, b) - 1500.0) / 800.0 * 255.0, 0.0, 255.0);
    }

    void Decoder::renderRow(int row) {
        // Robot 36: rows 2k and 2k+1 share the R-Y of one line and the B-Y of the other
        const Mode& m = modes()[modeIdx_];
        if (row < 0 || row >= nextLine_ || row >= m.height) { return; }
        const int b = row & ~1;
        const std::vector<uint8_t>* cr = nullptr;
        const std::vector<uint8_t>* cb = nullptr;
        for (int l = b; l <= b + 1 && l < nextLine_; l++) {
            if (robotChroma_[l] == 0) { cr = &chan_[1][l]; }
            else if (robotChroma_[l] == 1) { cb = &chan_[1][l]; }
        }
        for (int x = 0; x < m.width; x++) {
            img_[(size_t)row * m.width + x] = yuv(chan_[0][row][x], cr ? (*cr)[x] : 128, cb ? (*cb)[x] : 128);
        }
    }

    void Decoder::decodeLine(int n) {
        const Mode& m = modes()[modeIdx_];
        const double k = rate_ / 1000.0;
        double s = 0.0;
        // Martin, Robot, PD: the first sync directly follows the 1200 Hz VIS stop bit, so no edge
        // tells where it starts. The VIS timing is exact there; syncs are measured from line 1.
        const bool measure = acquire_ || n > 0 || m.syncAtMs > 0.0;
        if (!measure) {}
        else if (findSync(n, s)) {
            if (acquire_) {
                // Manual start: lock the time base on the first sync found
                t0_ = s - n * period_;
                acquire_ = false;
                syncs_.push_back({ n, s });
                missed_ = 0;
            }
            else {
                const double predicted = t0_ + n * period_;
                if (syncs_.size() < 6 || std::fabs(s - predicted) < ms(2.5)) {
                    syncs_.push_back({ n, s });
                    refit();
                    missed_ = 0;
                }
                else { missed_++; }
            }
        }
        else {
            missed_++;
            acquire_ = false;
        }

        const double base = t0_ + n * period_;
        const int w = m.width;
        switch (m.color) {
        case Color::GBR:
            for (int x = 0; x < w; x++) {
                const double g = pixel(base, m.segStart[0], m.segLen[0], x, w);
                const double bl = pixel(base, m.segStart[1], m.segLen[1], x, w);
                const double r = pixel(base, m.segStart[2], m.segLen[2], x, w);
                img_[(size_t)n * w + x] = rgb(r, g, bl);
            }
            decodedRows_ = n + 1;
            break;
        case Color::Robot72:
            for (int x = 0; x < w; x++) {
                img_[(size_t)n * w + x] = yuv(pixel(base, m.segStart[0], m.segLen[0], x, w),
                                              pixel(base, m.segStart[1], m.segLen[1], x, w),
                                              pixel(base, m.segStart[2], m.segLen[2], x, w));
            }
            decodedRows_ = n + 1;
            break;
        case Color::Robot36: {
            for (int x = 0; x < w; x++) {
                chan_[0][n][x] = (uint8_t)pixel(base, m.segStart[0], m.segLen[0], x, w);
                chan_[1][n][x] = (uint8_t)pixel(base, m.segStart[1], m.segLen[1], x, w);
            }
            // Separator after Y: 1500 Hz before R-Y, 2300 Hz before B-Y (line parity otherwise)
            const double sep = mean((int64_t)(base + 100.5 * k), (int64_t)(base + 103.5 * k));
            robotChroma_[n] = std::fabs(sep - 1900.0) > 250.0 ? (sep > 1900.0 ? 1 : 0) : (n & 1);
            decodedRows_ = n + 1;
            renderRow(n & ~1); // nextLine_ is already n + 1 here
            renderRow((n & ~1) + 1);
            break;
        }
        case Color::PD:
            for (int x = 0; x < w; x++) {
                const double cr = pixel(base, m.segStart[1], m.segLen[1], x, w);
                const double cb = pixel(base, m.segStart[2], m.segLen[2], x, w);
                img_[(size_t)(2 * n) * w + x] = yuv(pixel(base, m.segStart[0], m.segLen[0], x, w), cr, cb);
                img_[(size_t)(2 * n + 1) * w + x] = yuv(pixel(base, m.segStart[3], m.segLen[3], x, w), cr, cb);
            }
            decodedRows_ = 2 * n + 2;
            break;
        }
        st_.line = decodedRows_;
        st_.slantPpm = (period_ / nominalPeriod_ - 1.0) * 1e6;
        st_.version++;
    }

    void Decoder::endImage(bool complete) {
        if (modeIdx_ < 0) { return; }
        const Mode& m = modes()[modeIdx_];
        // A lost signal also ends with lines of noise decoded before giving up: drop them
        int rows = decodedRows_;
        if (!complete) {
            const int perLine = m.color == Color::PD ? 2 : 1;
            rows = std::max(0, rows - missed_ * perLine);
        }
        if (complete || rows >= 16) {
            done_ = img_;
            doneW_ = m.width;
            doneH_ = m.height;
            st_.finished++;
            st_.finishedRows = complete ? m.height : rows;
            st_.finishedMode = m.name;
            st_.lastComplete = complete;
        }
        st_.receiving = false;
        st_.version++;
        modeIdx_ = -1;
        acquire_ = false;
        visIgnoreUntil_ = std::max(visIgnoreUntil_, count_ + ms(500));
    }

    // ------------------------------------------------------------------ control
    void Decoder::reset() {
        std::lock_guard<std::mutex> lck(mtx_);
        modeIdx_ = -1;
        acquire_ = false;
        fs_ = 0.0; // reconfigure on the next block
        st_.receiving = false;
        st_.line = 0;
        st_.version++;
    }

    void Decoder::startManual(int mi) {
        std::lock_guard<std::mutex> lck(mtx_);
        if (mi < 0 || mi >= (int)modes().size() || rate_ <= 0) { return; }
        if (modeIdx_ >= 0) { endImage(false); }
        beginImage(mi, (double)count_, true);
    }

    void Decoder::finishNow() {
        std::lock_guard<std::mutex> lck(mtx_);
        if (modeIdx_ < 0) { return; }
        missed_ = 0; // keep every decoded row
        endImage(nextLine_ >= modes()[modeIdx_].scanLines());
    }

    Status Decoder::status() {
        std::lock_guard<std::mutex> lck(mtx_);
        Status s = st_;
        s.freq = smooth_;
        return s;
    }

    std::vector<uint32_t> Decoder::image(int which, int& w, int& h) {
        std::lock_guard<std::mutex> lck(mtx_);
        if (which == 1) {
            w = doneW_;
            h = doneH_;
            return done_;
        }
        w = st_.width;
        h = st_.height;
        return img_;
    }
}
