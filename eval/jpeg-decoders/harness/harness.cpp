// JPEG decoder harness for docs/JPEG_DECODERS.md: the same measurement for every decoder, CPU and GPU.
//
//   harness list
//   harness time [--secs S] [--dec a,b,...] FILE...   per file and decoder: median wall ms per decode, process CPU
//                                                     ms per decode (all threads), and the output against
//                                                     libjpeg-turbo (mean |diff|, max, PSNR; planar decoders
//                                                     against libjpeg-turbo's planes)
//   harness repeat N DEC FILE                         N decodes, nothing else (callgrind: repeat 3 minus repeat 1)
//   harness dump DEC FILE OUT.ppm                     one decode written as PPM (references for other tools)
//   harness throughput [--secs S] --workers W DEC FILE...
//                                                     W threads, each with its own decoder, one image at a time:
//                                                     images per second and CPU ms per image
//
// Timing follows the Rust harness (src/main.rs): warm-up decodes, then samples until both 5 samples and the budget
// (default 1 s) are reached; the median. GPU decoders get more warm-up (context, buffers) and blocking sync.
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iterator>
#include <string>
#include <thread>
#include <vector>

#ifdef _WIN32
#define NOMINMAX
#include <windows.h>
#else
#include <ctime>
#endif

#include "decoder.h"

namespace {

double cpu_seconds() {
#ifdef _WIN32
    FILETIME c, e, k, u;
    GetProcessTimes(GetCurrentProcess(), &c, &e, &k, &u);
    auto t = [](FILETIME f) { return (double(f.dwHighDateTime) * 4294967296.0 + f.dwLowDateTime) * 1e-7; };
    return t(k) + t(u);
#else
    timespec ts;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &ts);
    return double(ts.tv_sec) + ts.tv_nsec * 1e-9;
#endif
}

double now() { return std::chrono::duration<double>(std::chrono::steady_clock::now().time_since_epoch()).count(); }

std::vector<uint8_t> read_file(const std::string& p) {
    std::ifstream f(p, std::ios::binary);
    return {std::istreambuf_iterator<char>(f), {}};
}

std::string base(const std::string& p) {
    auto s = p.find_last_of("/\\");
    return s == std::string::npos ? p : p.substr(s + 1);
}

std::vector<DecoderSpec> all_decoders() {
    std::vector<DecoderSpec> v;
    add_turbo(v);
    add_parajpeg(v);
    add_jpegli(v);
    add_wuffs(v);
    add_gpu(v);
    return v;
}

struct Diff {
    std::string note;  // set when the images can't be compared
    double mean = 0, psnr = 0;
    int max = 0;
};

void accumulate(const uint8_t* a, const uint8_t* b, uint64_t& sum, uint64_t& sq, int& mx) {
    int d = std::abs(int(*a) - int(*b));
    sum += uint64_t(d);
    sq += uint64_t(d) * uint64_t(d);
    mx = std::max(mx, d);
}

Diff finish(uint64_t sum, uint64_t sq, int mx, double n) {
    Diff r;
    r.mean = sum / n;
    r.max = mx;
    r.psnr = sq == 0 ? INFINITY : 10.0 * std::log10(255.0 * 255.0 / (double(sq) / n));
    return r;
}

Diff compare(const Image& out, const Image& rgb, const Image& yuv) {
    uint64_t sum = 0, sq = 0;
    int mx = 0;
    double n = 0;
    if (out.layout == Image::Planar) {
        if (out.planes != yuv.planes) return {"planes " + std::to_string(out.planes)};
        const uint8_t *a = out.data, *b = yuv.data;
        for (int c = 0; c < out.planes; c++) {
            uint32_t w = std::min(out.pw[c], yuv.pw[c]), h = std::min(out.ph[c], yuv.ph[c]);
            for (uint32_t y = 0; y < h; y++)
                for (uint32_t x = 0; x < w; x++, n++)
                    accumulate(a + size_t(y) * out.pw[c] + x, b + size_t(y) * yuv.pw[c] + x, sum, sq, mx);
            a += size_t(out.pw[c]) * out.ph[c];
            b += size_t(yuv.pw[c]) * yuv.ph[c];
        }
        return finish(sum, sq, mx, n);
    }
    if (out.width != rgb.width || out.height != rgb.height)
        return {std::to_string(out.width) + "x" + std::to_string(out.height)};
    int step = out.layout == Image::RGBX ? 4 : 3;
    size_t px = size_t(out.width) * out.height;
    for (size_t i = 0; i < px; i++)
        for (int c = 0; c < 3; c++, n++) accumulate(out.data + i * step + c, rgb.data + i * 3 + c, sum, sq, mx);
    return finish(sum, sq, mx, n);
}

int time_mode(std::vector<std::string> args) {
    double budget = 1.0;
    std::string only;
    while (!args.empty() && args[0].rfind("--", 0) == 0) {
        if (args.size() < 2) return std::fprintf(stderr, "%s needs a value\n", args[0].c_str()), 2;
        if (args[0] == "--secs") budget = std::atof(args[1].c_str());
        else if (args[0] == "--dec") only = "," + args[1] + ",";
        args.erase(args.begin(), args.begin() + 2);
    }
    auto decs = all_decoders();
    std::printf("%-34s %-16s %10s %10s %6s   %s\n", "file", "decoder", "wall ms", "cpu ms", "n",
                "vs libjpeg-turbo: mean |diff| / max / PSNR");
    for (const auto& path : args) {
        auto jpeg = read_file(path);
        Image rgb, yuv;
        auto ref = decs[0].make();  // turbo
        auto refy = decs[1].make();  // turbo-yuv
        std::string e1 = ref->decode(jpeg, rgb), e2 = refy->decode(jpeg, yuv);
        for (const auto& spec : decs) {
            if (!only.empty() && only.find("," + spec.name + ",") == std::string::npos) continue;
            std::printf("%-34s %-16s ", base(path).c_str(), spec.name.c_str());
            std::fflush(stdout);
            auto dec = spec.make();
            Image out;
            std::string err;
            for (int i = 0; i < (spec.gpu ? 5 : 2) && err.empty(); i++) err = dec->decode(jpeg, out);
            if (!err.empty()) {
                std::printf("error: %s\n", err.c_str());
                continue;
            }
            Diff d = e1.empty() ? compare(out, rgb, yuv) : Diff{"no reference: " + e1};
            std::vector<double> samples;
            double c0 = cpu_seconds(), t0 = now();
            while (samples.size() < 5 || now() - t0 < budget) {
                double t = now();
                err = dec->decode(jpeg, out);
                samples.push_back((now() - t) * 1e3);
                if (!err.empty()) break;
            }
            double cpu = (cpu_seconds() - c0) * 1e3 / double(samples.size());
            if (!err.empty()) {
                std::printf("error while timing: %s\n", err.c_str());
                continue;
            }
            std::sort(samples.begin(), samples.end());
            std::printf("%10.3f %10.3f %6zu   ", samples[samples.size() / 2], cpu, samples.size());
            if (!d.note.empty()) std::printf("%s\n", d.note.c_str());
            else std::printf("%.3f / %d / %.1f dB\n", d.mean, d.max, d.psnr);
            std::fflush(stdout);
        }
        (void)e2;
    }
    return 0;
}

const DecoderSpec* find(const std::vector<DecoderSpec>& v, const std::string& name) {
    for (const auto& s : v)
        if (s.name == name) return &s;
    std::fprintf(stderr, "unknown decoder %s (harness list)\n", name.c_str());
    return nullptr;
}

int repeat_mode(const std::vector<std::string>& a) {
    if (a.size() != 3) return std::fprintf(stderr, "repeat N DEC FILE\n"), 2;
    auto decs = all_decoders();
    auto* spec = find(decs, a[1]);
    if (!spec) return 2;
    auto jpeg = read_file(a[2]);
    auto dec = spec->make();
    Image out;
    for (int i = 0; i < std::atoi(a[0].c_str()); i++)
        if (auto e = dec->decode(jpeg, out); !e.empty()) return std::fprintf(stderr, "%s\n", e.c_str()), 1;
    return 0;
}

int dump_mode(const std::vector<std::string>& a) {  // dump DEC FILE OUT.ppm: RGB decoders only
    if (a.size() != 3) return std::fprintf(stderr, "dump DEC FILE OUT.ppm\n"), 2;
    auto decs = all_decoders();
    auto* spec = find(decs, a[0]);
    if (!spec) return 2;
    auto jpeg = read_file(a[1]);
    Image out;
    if (auto e = spec->make()->decode(jpeg, out); !e.empty()) return std::fprintf(stderr, "%s\n", e.c_str()), 1;
    if (out.layout == Image::Planar) return std::fprintf(stderr, "planar output\n"), 1;
    std::ofstream f(a[2], std::ios::binary);
    f << "P6\n" << out.width << " " << out.height << "\n255\n";
    int step = out.layout == Image::RGBX ? 4 : 3;
    for (size_t i = 0; i < size_t(out.width) * out.height; i++) f.write(reinterpret_cast<const char*>(out.data + i * step), 3);
    return 0;
}

int throughput_mode(std::vector<std::string> args) {
    double budget = 2.0;
    int workers = 1;
    while (!args.empty() && args[0].rfind("--", 0) == 0) {
        if (args.size() < 2) return std::fprintf(stderr, "%s needs a value\n", args[0].c_str()), 2;
        if (args[0] == "--secs") budget = std::atof(args[1].c_str());
        else if (args[0] == "--workers") workers = std::atoi(args[1].c_str());
        args.erase(args.begin(), args.begin() + 2);
    }
    if (args.empty()) return std::fprintf(stderr, "throughput [--secs S] [--workers N] DEC FILE...\n"), 2;
    auto decs = all_decoders();
    auto* spec = find(decs, args[0]);
    if (!spec) return 2;
    for (size_t f = 1; f < args.size(); f++) {
        auto jpeg = read_file(args[f]);
        std::vector<std::unique_ptr<Decoder>> ds;
        std::vector<Image> outs(static_cast<size_t>(workers));
        std::string err;
        for (int w = 0; w < workers && err.empty(); w++) {
            ds.push_back(spec->make());
            for (int i = 0; i < (spec->gpu ? 5 : 2) && err.empty(); i++) err = ds.back()->decode(jpeg, outs[size_t(w)]);
        }
        if (!err.empty()) {  // this file is not for this decoder: say so and go on with the next
            std::printf("%-34s %-16s error: %s\n", base(args[f]).c_str(), spec->name.c_str(), err.c_str());
            continue;
        }
        std::atomic<long> done{0};
        std::atomic<bool> stop{false}, failed{false};
        double c0 = cpu_seconds(), t0 = now();
        std::vector<std::thread> th;
        for (int w = 0; w < workers; w++)
            th.emplace_back([&, w] {
                while (!stop) {
                    if (!ds[size_t(w)]->decode(jpeg, outs[size_t(w)]).empty()) failed = true, stop = true;
                    done++;
                }
            });
        std::this_thread::sleep_for(std::chrono::duration<double>(budget));
        stop = true;
        for (auto& t : th) t.join();
        double secs = now() - t0, cpu = cpu_seconds() - c0;
        std::printf("%-34s %-16s workers %2d: %9.1f images/s  %8.3f ms/image  cpu %8.3f ms/image%s\n",
                    base(args[f]).c_str(), spec->name.c_str(), workers, double(done) / secs,
                    secs * 1e3 / double(done), cpu * 1e3 / double(done), failed ? "  (errors)" : "");
        std::fflush(stdout);
    }
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    std::vector<std::string> a(argv + 1, argv + argc);
    gpu_init();
    if (a.empty()) return std::fprintf(stderr, "harness list | time | repeat | throughput (see harness.cpp)\n"), 2;
    std::string mode = a[0];
    a.erase(a.begin());
    if (mode == "list") {
        for (const auto& s : all_decoders()) std::printf("%-16s %s\n", s.name.c_str(), s.about.c_str());
        return 0;
    }
    if (mode == "time") return time_mode(a);
    if (mode == "repeat") return repeat_mode(a);
    if (mode == "throughput") return throughput_mode(a);
    if (mode == "dump") return dump_mode(a);
    std::fprintf(stderr, "unknown mode %s\n", mode.c_str());
    return 2;
}
