// The C side of docs/temp/PNG_DECODERS.md: libpng, libspng and Wuffs, measured as eval/png-decoders' Rust harness
// (src/main.rs) measures the app's fork.
//
//   png-c-harness time [--secs S] FILE...     median ms per decode, one thread, full size; counting off
//   png-c-harness compare DIR FILE...         each decoder's output against DIR/NAME.raw (the app's), sample by sample
//   png-c-harness allocs FILE...              one decode's heap calls after two warm-ups; counting on
//
// The time and compare commands run with the counting hooks inert (heap::enable is off). Only allocs turns them on.
// Each decoder is created once per command and used for every file, as the Rust harness's Decoders is.
#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <memory>
#include <string>
#include <vector>

#include "alloc_count.h"
#include "decoder.h"
#include "png_info.h"

namespace {

volatile size_t sink = 0;  // the decoders' output sizes are read here, so no decode is optimised away

double now_ms() {
    using clock = std::chrono::steady_clock;
    return std::chrono::duration<double, std::milli>(clock::now().time_since_epoch()).count();
}

bool read_file(const std::string& path, std::vector<uint8_t>& bytes) {
    std::ifstream f(path, std::ios::binary | std::ios::ate);
    if (!f) return false;
    const std::streamsize size = f.tellg();
    bytes.resize(size_t(size));
    f.seekg(0);
    return bool(f.read(reinterpret_cast<char*>(bytes.data()), size));
}

std::string stem(const std::string& path) {
    size_t slash = path.find_last_of("/\\");
    std::string name = slash == std::string::npos ? path : path.substr(slash + 1);
    size_t dot = name.find_last_of('.');
    return dot == std::string::npos ? name : name.substr(0, dot);
}

std::vector<std::unique_ptr<Decoder>> make_all() {
    std::vector<DecoderSpec> specs;
    add_libpng(specs);
    add_libspng(specs);
    add_wuffs(specs);
    std::vector<std::unique_ptr<Decoder>> decoders;
    for (const auto& spec : specs) decoders.push_back(spec.make());
    return decoders;
}

const std::vector<std::string>& names() {
    static const std::vector<std::string> n = {"libpng", "libspng", "wuffs"};
    return n;
}

// The row label as the Rust harness writes it: megapixels, channels and bits.
std::string label(const PngInfo& info) {
    char buf[64];
    std::snprintf(buf, sizeof buf, "%5.2f MP  %u ch  %2u-bit", double(info.width) * info.height / 1e6, info.channels(),
                  info.bytes_per_sample() * 8);
    return buf;
}

// The file's bytes and header; false (with a message) when it is not a PNG.
bool load(const std::string& path, std::vector<uint8_t>& bytes, PngInfo& info) {
    std::string err;
    if (!read_file(path, bytes)) {
        std::fprintf(stderr, "%s: cannot read\n", path.c_str());
        return false;
    }
    if (!parse_png_info(bytes, info, err)) {
        std::fprintf(stderr, "%s: %s\n", path.c_str(), err.c_str());
        return false;
    }
    return true;
}

int time_mode(const std::vector<std::string>& args) {
    double budget = 1.0;
    size_t first = 0;
    if (args.size() >= 2 && args[0] == "--secs") {
        budget = std::atof(args[1].c_str());
        first = 2;
    }
    auto decoders = make_all();
    size_t width = 0;
    for (size_t i = first; i < args.size(); i++) width = std::max(width, stem(args[i]).size());
    std::printf("%*s  %26s", int(width), "", "");
    for (const auto& n : names()) std::printf(" %15s", n.c_str());
    std::printf("   (median ms per decode)\n");
    for (size_t i = first; i < args.size(); i++) {
        std::vector<uint8_t> bytes;
        PngInfo info;
        if (!load(args[i], bytes, info)) return 1;
        std::printf("%-*s  %s", int(width), stem(args[i]).c_str(), label(info).c_str());
        for (size_t d = 0; d < decoders.size(); d++) {
            Decoder& dec = *decoders[d];
            Result r = dec.decode(bytes, info);
            if (r.status == Status::Unsupported) {
                std::printf(" %15s", "n/c");
                continue;
            }
            if (r.status == Status::Failed) {
                std::printf(" %15s", "failed");
                continue;
            }
            for (int w = 0; w < 2; w++) sink = sink + dec.decode(bytes, info).why.size() + dec.out().size();
            std::vector<double> samples;
            const double start = now_ms();
            while (samples.size() < 5 || now_ms() - start < budget * 1e3) {
                const double t = now_ms();
                Result again = dec.decode(bytes, info);
                samples.push_back(now_ms() - t);
                sink = sink + again.why.size() + dec.out().size();
                if (again.status != Status::Ok) break;
            }
            std::sort(samples.begin(), samples.end());
            std::printf(" %15.2f", samples[samples.size() / 2]);
        }
        std::printf("\n");
        std::fflush(stdout);
    }
    return 0;
}

// The app's output (NAME.raw, samples packed, 16-bit big-endian) and its "width height channels bytes-per-sample".
bool load_reference(const std::string& dir, const std::string& name, std::vector<uint8_t>& raw, Dims& dims) {
    if (!read_file(dir + "/" + name + ".raw", raw)) return false;
    std::vector<uint8_t> txt;
    if (!read_file(dir + "/" + name + ".txt", txt)) return false;
    unsigned w = 0, h = 0, c = 0, b = 0;
    if (std::sscanf(reinterpret_cast<const char*>(txt.data()), "%u %u %u %u", &w, &h, &c, &b) != 4) return false;
    dims = {w, h, c, b};
    return true;
}

// Samples that differ between `a` and `b` (bytes per sample `bps`, big-endian for 16-bit): how many, and the largest
// difference.
void sample_diff(const std::vector<uint8_t>& a, const std::vector<uint8_t>& b, uint32_t bps, size_t& differ,
                 uint32_t& largest) {
    differ = 0;
    largest = 0;
    auto value = [bps](const uint8_t* s) {
        uint32_t v = 0;
        for (uint32_t k = 0; k < bps; k++) v = (v << 8) | s[k];
        return v;
    };
    for (size_t i = 0; i + bps <= a.size(); i += bps) {
        const uint32_t x = value(&a[i]), y = value(&b[i]);
        const uint32_t d = x > y ? x - y : y - x;
        if (d != 0) {
            differ++;
            largest = std::max(largest, d);
        }
    }
}

int compare_mode(const std::vector<std::string>& args) {
    if (args.size() < 2) {
        std::fprintf(stderr, "usage: png-c-harness compare DIR FILE...\n");
        return 2;
    }
    const std::string dir = args[0];
    auto decoders = make_all();
    std::printf("Against the app's png fork (DIR/NAME.raw): identical samples, or how many samples differ and the largest "
                "difference.\n");
    for (size_t i = 1; i < args.size(); i++) {
        const std::string name = stem(args[i]);
        std::vector<uint8_t> bytes, reference;
        PngInfo info;
        Dims want{};
        if (!load(args[i], bytes, info)) return 1;
        if (!load_reference(dir, name, reference, want)) {
            std::fprintf(stderr, "%s: no reference in %s\n", name.c_str(), dir.c_str());
            return 1;
        }
        std::printf("%s  %s", name.c_str(), label(info).c_str());
        for (size_t d = 0; d < decoders.size(); d++) {
            Decoder& dec = *decoders[d];
            const std::string who = names()[d];
            Result r = dec.decode(bytes, info);
            if (r.status == Status::Unsupported) {
                std::printf("  %s: not comparable (%s)", who.c_str(), r.why.c_str());
                continue;
            }
            if (r.status == Status::Failed) {
                std::printf("  %s: failed (%s)", who.c_str(), r.why.c_str());
                continue;
            }
            const Dims& got = dec.dims();
            if (got.width != want.width || got.height != want.height || got.channels != want.channels ||
                got.bytes_per_sample != want.bytes_per_sample || dec.out().size() != reference.size()) {
                std::printf("  %s: channel/size mismatch: %ux%u %u ch %u-byte (the app's %ux%u %u ch %u-byte)", who.c_str(),
                            got.width, got.height, got.channels, got.bytes_per_sample, want.width, want.height,
                            want.channels, want.bytes_per_sample);
                continue;
            }
            if (dec.out() == reference) {
                std::printf("  %s: identical", who.c_str());
                continue;
            }
            size_t differ = 0;
            uint32_t largest = 0;
            sample_diff(dec.out(), reference, want.bytes_per_sample, differ, largest);
            std::printf("  %s: %zu of %zu samples differ, largest %u", who.c_str(), differ,
                        dec.out().size() / want.bytes_per_sample, largest);
        }
        std::printf("\n");
        std::fflush(stdout);
    }
    return 0;
}

int allocs_mode(const std::vector<std::string>& args) {
    auto decoders = make_all();
    std::printf("Heap calls per decode, steady state (after two warm-ups): calls, KiB requested, peak KiB above the "
                "live bytes at the start.\n");
    size_t width = 0;
    for (const auto& a : args) width = std::max(width, stem(a).size());
    for (const auto& path : args) {
        std::vector<uint8_t> bytes;
        PngInfo info;
        if (!load(path, bytes, info)) return 1;
        std::printf("%-*s", int(width), stem(path).c_str());
        for (size_t d = 0; d < decoders.size(); d++) {
            Decoder& dec = *decoders[d];
            const std::string who = names()[d];
            Result r = dec.decode(bytes, info);
            if (r.status != Status::Ok) {
                std::printf("  %s: %s", who.c_str(), r.status == Status::Failed ? "failed" : "not comparable");
                continue;
            }
            for (int w = 0; w < 2; w++) sink = sink + dec.decode(bytes, info).why.size();
            heap::reset();
            const int64_t start = heap::totals().live;
            Result measured = dec.decode(bytes, info);
            const heap::Totals t = heap::totals();
            sink = sink + measured.why.size();
            std::printf("  %s: %llu calls, %.0f KiB, peak %.0f KiB", who.c_str(), static_cast<unsigned long long>(t.calls),
                        double(t.requested) / 1024.0, double(t.peak - start) / 1024.0);
        }
        std::printf("\n");
        std::fflush(stdout);
    }
    return 0;
}

}  // namespace

int main(int argc, char** argv) {
    std::vector<std::string> args(argv + 1, argv + argc);
    if (args.empty()) {
        std::fprintf(stderr, "usage: png-c-harness time [--secs S] FILE... | compare DIR FILE... | allocs FILE...\n");
        return 2;
    }
    const std::string cmd = args[0];
    args.erase(args.begin());
    heap::enable(cmd == "allocs");  // before any decode: only the allocs command counts
    if (cmd == "time") return time_mode(args);
    if (cmd == "compare") return compare_mode(args);
    if (cmd == "allocs") return allocs_mode(args);
    std::fprintf(stderr, "unknown command %s\n", cmd.c_str());
    return 2;
}
