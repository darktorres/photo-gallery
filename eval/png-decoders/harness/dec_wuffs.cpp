// Wuffs at d28ecd4 through wuffs_aux::DecodeImage, as the JPEG harness uses it. Its PNG output formats are fixed per
// pixel format, and the app's layout has one for each file the 14 inputs contain, except 16-bit colour: Wuffs has
// Y_16BE but no 16-bit big-endian RGB or RGBA (RGBA_NONPREMUL_4X16LE is little-endian), so those files are
// "not comparable" and carry no time.
//
// Allocations: the pixel and work buffers belong to this decoder and are kept across decodes (Callbacks below).
// aux's DecodeImage makes one decoder object per call (wuffs_png__decoder__alloc: calloc, freed with free), so that
// object's allocation is made and freed per decode, and counted by alloc_count.h: the C allocator names are routed
// there for this translation unit only.
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

// The Wuffs format of the app's layout for this file, or why there is none.
bool wuffs_format(const PngInfo& info, wuffs_base__pixel_format& format, std::string& why) {
    if (info.bytes_per_sample() == 1) {
        switch (info.channels()) {
            case 1: format = wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__Y); return true;
            case 2: format = wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__YA_NONPREMUL); return true;
            case 3: format = wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGB); return true;
            default: format = wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGBA_NONPREMUL); return true;
        }
    }
    if (info.channels() == 1) {
        format = wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__Y_16BE);
        return true;
    }
    why = "no 16-bit big-endian output for " + std::to_string(info.channels()) +
          " channels (Wuffs' 16-bit RGB(A) is little-endian)";
    return false;
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
        wuffs_base__pixel_format format{};
        std::string why;
        if (!wuffs_format(info, format, why)) return {Status::Unsupported, why};
        cb_.pixels = &out_;
        cb_.want = format;
        wuffs_aux::sync_io::MemoryInput in(png.data(), png.size());
        auto r = wuffs_aux::DecodeImage(cb_, in);
        if (!r.error_message.empty()) return {Status::Failed, r.error_message};
        dims_ = {r.pixbuf.pixcfg.width(), r.pixbuf.pixcfg.height(), info.channels(), info.bytes_per_sample()};
        return {};
    }

private:
    Callbacks cb_;
};

}  // namespace

void add_wuffs(std::vector<DecoderSpec>& v) {
    v.push_back({"wuffs", [] { return std::make_unique<Wuffs>(); }});
}
