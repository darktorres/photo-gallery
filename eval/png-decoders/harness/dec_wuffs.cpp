// Wuffs at d28ecd4 through wuffs_aux::DecodeImage. Its PNG decoder gives a fixed set of output formats, and the app's
// layout isn't always one of them, so the decoder picks the nearest one and repacks inside the timed decode:
//
// - 8-bit gray: Y. 8-bit RGB: RGB. 8-bit RGBA: RGBA_NONPREMUL.
// - 8-bit gray+alpha: Wuffs' PNG decoder won't write YA_NONPREMUL for it, so RGBA, then the gray and the alpha of each
//   pixel (R and A: R = G = B for gray).
// - 16-bit gray: Y_16BE. 16-bit RGB, RGBA, gray+alpha: Wuffs has no 16-bit big-endian RGB(A), and its own 16-bit
//   format is BGRA_NONPREMUL_4X16LE (the PNG decoder accepts no other), so that, then the samples swapped to big-endian
//   and reordered to RGB(A), the alpha dropped for RGB.
//
// The repacking is part of the time and the allocations, since it is what the app's layout costs with Wuffs.
// Colour-key transparency: Wuffs writes the colour of a fully transparent (colour-key) pixel as black, where the app
// and libpng keep the key colour. The alpha is the same. This shows up in `compare` as a difference.
//
// Allocations: the output and work buffers belong to this decoder and are kept across decodes (Callbacks below), and
// so is the buffer the repacking reads from. aux's DecodeImage makes one decoder object per call
// (wuffs_png__decoder__alloc: calloc, freed with free), so that object's allocation is made and freed per decode, and
// counted by alloc_count.h: the C allocator names are routed there for this translation unit only.
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits.h>
#include <memory>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <string>
#include <utility>
#include <vector>

#include "alloc_count.h"
#include "decoder.h"

#define WUFFS_IMPLEMENTATION
#define WUFFS_CONFIG__MODULES
#define WUFFS_CONFIG__MODULE__AUX__BASE
#define WUFFS_CONFIG__MODULE__AUX__IMAGE
#define WUFFS_CONFIG__MODULE__BASE
#define WUFFS_CONFIG__MODULE__PNG
#define WUFFS_CONFIG__MODULE__ZLIB
// The modules PNG and zlib build on (PNG checks its chunks' CRCs, zlib its streams' Adler-32s, both inflate with deflate).
#define WUFFS_CONFIG__MODULE__ADLER32
#define WUFFS_CONFIG__MODULE__CRC32
#define WUFFS_CONFIG__MODULE__DEFLATE
#define malloc(n) heap::malloc_counted(n)
#define calloc(n, s) heap::calloc_counted((n), (s))
#define free(p) heap::free_counted(p)
#include WUFFS_SOURCE
#undef malloc
#undef calloc
#undef free

namespace {

// What turns Wuffs' output into the app's layout.
enum class Repack { None, GrayAlpha8, GrayAlpha16, Rgb16, Rgba16 };

// The Wuffs format to decode to for this file, and the repacking after it.
wuffs_base__pixel_format wuffs_format(const PngInfo& info, Repack& repack) {
    if (info.bytes_per_sample() == 1) {
        switch (info.channels()) {
            case 1: repack = Repack::None; return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__Y);
            case 2: repack = Repack::GrayAlpha8; return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGBA_NONPREMUL);
            case 3: repack = Repack::None; return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGB);
            default: repack = Repack::None; return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGBA_NONPREMUL);
        }
    }
    if (info.channels() == 1) {
        repack = Repack::None;
        return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__Y_16BE);
    }
    switch (info.channels()) {
        case 2: repack = Repack::GrayAlpha16; break;
        case 3: repack = Repack::Rgb16; break;
        default: repack = Repack::Rgba16; break;
    }
    return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__BGRA_NONPREMUL_4X16LE);  // Wuffs' own 16-bit format
}

// Wuffs' pixels (`in`, `pixels` of them: 4 bytes of RGBA 8-bit, or 8 bytes of BGRA 16-bit little-endian) into the app's
// layout in `out`, which keeps its size once it has it.
void repack_into(Repack repack, size_t pixels, const std::vector<uint8_t>& in, std::vector<uint8_t>& out) {
    switch (repack) {
        case Repack::None:
            return;
        case Repack::GrayAlpha8:
            out.resize(pixels * 2);
            for (size_t i = 0; i < pixels; i++) {
                out[2 * i] = in[4 * i];
                out[2 * i + 1] = in[4 * i + 3];
            }
            return;
        case Repack::GrayAlpha16:
        case Repack::Rgb16:
        case Repack::Rgba16: {
            // Wuffs' BGRA order: B (0), G (1), R (2), A (3); each sample is two bytes, little-endian in `in`,
            // big-endian in `out`.
            const int channels[] = {2, 3};  // gray+alpha: R (= gray) and A
            const int rgb[] = {2, 1, 0};
            const int rgba[] = {2, 1, 0, 3};
            const int* picks = repack == Repack::GrayAlpha16 ? channels : repack == Repack::Rgb16 ? rgb : rgba;
            const size_t n = repack == Repack::Rgb16 ? 3 : repack == Repack::Rgba16 ? 4 : 2;
            out.resize(pixels * n * 2);
            for (size_t i = 0; i < pixels; i++) {
                for (size_t k = 0; k < n; k++) {
                    const size_t src = 8 * i + 2 * size_t(picks[k]);
                    out[(i * n + k) * 2] = in[src + 1];
                    out[(i * n + k) * 2 + 1] = in[src];
                }
            }
            return;
        }
    }
}

struct Callbacks : wuffs_aux::DecodeImageCallbacks {
    std::vector<uint8_t>* pixels = nullptr;  // the decoder's output, kept across decodes
    std::vector<uint8_t> work;               // Wuffs' work buffer, kept across decodes
    wuffs_base__pixel_format want{};

    wuffs_base__pixel_format SelectPixfmt(const wuffs_base__image_config&) override { return want; }

    AllocPixbufResult AllocPixbuf(const wuffs_base__image_config& ic, bool) override {
        uint64_t len = ic.pixcfg.pixbuf_len();
        pixels->resize(size_t(len));  // no allocation once the buffer has the size
        wuffs_base__pixel_buffer pb;
        auto st = pb.set_from_slice(&ic.pixcfg, wuffs_base__make_slice_u8(pixels->data(), size_t(len)));
        if (!st.is_ok()) return AllocPixbufResult(st.message());
        return AllocPixbufResult(wuffs_aux::MemOwner(nullptr, &free), pb);
    }

    AllocWorkbufResult AllocWorkbuf(wuffs_base__range_ii_u64 r, bool) override {
        if (work.size() < r.max_incl) work.resize(size_t(r.max_incl));
        return AllocWorkbufResult(wuffs_aux::MemOwner(nullptr, &free),
                                  wuffs_base__make_slice_u8(work.data(), size_t(r.max_incl)));
    }
};

class Wuffs final : public Decoder {
public:
    Result decode(const std::vector<uint8_t>& png, const PngInfo& info) override {
        Repack repack = Repack::None;
        cb_.want = wuffs_format(info, repack);
        cb_.pixels = repack == Repack::None ? &out_ : &wout_;
        wuffs_aux::sync_io::MemoryInput in(png.data(), png.size());
        auto r = wuffs_aux::DecodeImage(cb_, in);
        if (!r.error_message.empty()) return {Status::Failed, r.error_message};
        const uint32_t w = r.pixbuf.pixcfg.width(), h = r.pixbuf.pixcfg.height();
        dims_ = {w, h, info.channels(), info.bytes_per_sample()};
        repack_into(repack, size_t(w) * h, wout_, out_);
        return {};
    }

private:
    Callbacks cb_;
    std::vector<uint8_t> wout_;  // Wuffs' own output, when the app's layout needs repacking
};

}  // namespace

void add_wuffs(std::vector<DecoderSpec>& v) {
    v.push_back({"wuffs", [] { return std::make_unique<Wuffs>(); }});
}
