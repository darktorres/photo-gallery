// libspng 0.7.4 (randy408/libspng) over zlib 1.4 (its zlib build, not miniz). The format per file gives the app's
// layout: SPNG_FMT_RAW (the PNG's own samples, 16-bit big-endian) where nothing changes; palette to RGB8, or RGBA8 with
// tRNS; tRNS to alpha on RGB (RGBA8/RGBA16) and gray (GA8/GA16); gray below 8 bits to G8.
//
// The context is created and freed per decode, as libspng is used. Nothing is kept between decodes but the output
// buffer, which the harness owns. spng_decode_chunks reads every chunk up to IDAT, and libspng has no option that
// skips text or ICC chunks, so it parses (and, for zTXt and iCCP, inflates) them on every decode; the app's skip is
// not matched here.
//
// Allocations: every libspng allocation, zlib's included (spng__zalloc -> ctx->alloc.calloc_fn), goes through the
// allocator given to spng_ctx_new2, which is alloc_count.h's.
#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include <spng.h>

#include "alloc_count.h"
#include "decoder.h"

namespace {

void* lib_malloc(size_t size) { return heap::malloc_counted(size); }
void* lib_realloc(void* ptr, size_t size) { return heap::realloc_counted(ptr, size); }
void* lib_calloc(size_t count, size_t size) { return heap::calloc_counted(count, size); }
void lib_free(void* ptr) { heap::free_counted(ptr); }

Result failed(int err) { return {Status::Failed, spng_strerror(err)}; }

class Libspng final : public Decoder {
public:
    Result decode(const std::vector<uint8_t>& png, const PngInfo&) override {
        spng_alloc alloc{lib_malloc, lib_realloc, lib_calloc, lib_free};
        spng_ctx* ctx = spng_ctx_new2(&alloc, 0);
        if (!ctx) return {Status::Failed, "spng_ctx_new2"};
        Result result = decode_with(ctx, png);
        spng_ctx_free(ctx);
        return result;
    }

private:
    Result decode_with(spng_ctx* ctx, const std::vector<uint8_t>& png) {
        int ret = spng_set_png_buffer(ctx, png.data(), png.size());  // the file's bytes, not a copy
        if (ret) return failed(ret);
        spng_ihdr ihdr;
        if ((ret = spng_get_ihdr(ctx, &ihdr))) return failed(ret);
        // Every chunk up to IDAT is read here, text and ICC included (see the file's comment).
        if ((ret = spng_decode_chunks(ctx))) return failed(ret);
        spng_trns trns;
        const bool has_trns = spng_get_trns(ctx, &trns) == 0;

        const bool gray = ihdr.color_type == SPNG_COLOR_TYPE_GRAYSCALE;
        const bool rgb = ihdr.color_type == SPNG_COLOR_TYPE_TRUECOLOR;
        const bool sixteen = ihdr.bit_depth == 16;
        int fmt = SPNG_FMT_RAW, flags = 0;
        uint32_t channels = 0, bytes_per_sample = sixteen ? 2 : 1;
        if (ihdr.color_type == SPNG_COLOR_TYPE_INDEXED) {  // palette: RGB, or RGBA with tRNS
            fmt = has_trns ? SPNG_FMT_RGBA8 : SPNG_FMT_RGB8;
            flags = has_trns ? SPNG_DECODE_TRNS : 0;
            channels = has_trns ? 4 : 3;
        } else if (gray && has_trns) {  // gray + alpha (GA8 is for 8 bits and below)
            fmt = sixteen ? SPNG_FMT_GA16 : SPNG_FMT_GA8;
            flags = SPNG_DECODE_TRNS;
            channels = 2;
        } else if (rgb && has_trns) {  // RGBA
            fmt = sixteen ? SPNG_FMT_RGBA16 : SPNG_FMT_RGBA8;
            flags = SPNG_DECODE_TRNS;
            channels = 4;
        } else if (gray && !sixteen && ihdr.bit_depth < 8) {  // 1, 2 and 4-bit gray expanded to 8
            fmt = SPNG_FMT_G8;
            channels = 1;
        } else {  // the PNG's own samples: no conversion
            channels = ihdr.color_type == SPNG_COLOR_TYPE_GRAYSCALE ? 1
                       : ihdr.color_type == SPNG_COLOR_TYPE_GRAYSCALE_ALPHA ? 2
                       : ihdr.color_type == SPNG_COLOR_TYPE_TRUECOLOR ? 3
                                                                      : 4;
        }
        if (fmt == SPNG_FMT_GA16 || fmt == SPNG_FMT_RGBA16) bytes_per_sample = 2;
        else if (fmt != SPNG_FMT_RAW) bytes_per_sample = 1;

        size_t size = 0;
        if ((ret = spng_decoded_image_size(ctx, fmt, &size))) return failed(ret);
        out_.resize(size);
        if ((ret = spng_decode_image(ctx, out_.data(), out_.size(), fmt, flags))) return failed(ret);
        // libspng writes SPNG_FMT_RGBA16 and GA16 samples in host order (its scanline is converted to host order before
        // these formats' conversion, spng.c), where the app's layout and the other decoders' is big-endian. Swap them
        // here, in the timed decode, as Wuffs' repacking is. The harness is little-endian (Windows), so swap every pair.
        if (fmt == SPNG_FMT_RGBA16 || fmt == SPNG_FMT_GA16) {
            for (size_t i = 0; i + 1 < out_.size(); i += 2) std::swap(out_[i], out_[i + 1]);
        }
        dims_ = {ihdr.width, ihdr.height, channels, bytes_per_sample};
        return {};
    }
};

}  // namespace

void add_libspng(std::vector<DecoderSpec>& v) {
    v.push_back({"libspng", [] { return std::make_unique<Libspng>(); }});
}
