// Wuffs (github.com/google/wuffs) through wuffs_aux::DecodeImage, to RGBA: its AVX2 YCbCr path writes 4-byte pixels
// (3-byte formats take the generic per-pixel path). Pixel and work buffers are reused across decodes, as the other
// decoders reuse theirs.
#include "decoder.h"

#ifdef HAVE_WUFFS
#define WUFFS_IMPLEMENTATION
#define WUFFS_CONFIG__MODULES
#define WUFFS_CONFIG__MODULE__AUX__BASE
#define WUFFS_CONFIG__MODULE__AUX__IMAGE
#define WUFFS_CONFIG__MODULE__BASE
#define WUFFS_CONFIG__MODULE__JPEG
#include WUFFS_SOURCE

namespace {

struct Callbacks : wuffs_aux::DecodeImageCallbacks {
    std::vector<uint8_t>* pixels;
    std::vector<uint8_t> work;
    wuffs_base__pixel_format SelectPixfmt(const wuffs_base__image_config&) override {
        return wuffs_base__make_pixel_format(WUFFS_BASE__PIXEL_FORMAT__RGBA_NONPREMUL);  // RGBX: not accepted
    }
    AllocPixbufResult AllocPixbuf(const wuffs_base__image_config& ic, bool) override {
        uint64_t len = ic.pixcfg.pixbuf_len();
        if (pixels->size() < len) pixels->resize(size_t(len));
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

struct Wuffs : Decoder {
    Callbacks cb;
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        cb.pixels = &out.own;
        wuffs_aux::sync_io::MemoryInput in(jpeg.data(), jpeg.size());
        auto r = wuffs_aux::DecodeImage(cb, in);
        if (!r.error_message.empty()) return r.error_message;
        out.layout = Image::RGBX;
        out.width = r.pixbuf.pixcfg.width();
        out.height = r.pixbuf.pixcfg.height();
        out.data = out.own.data();
        out.size = size_t(out.width) * out.height * 4;
        return {};
    }
};

}  // namespace

void add_wuffs(std::vector<DecoderSpec>& v) {
    v.push_back({"wuffs", "Wuffs (wuffs_aux::DecodeImage), RGBA (opaque)", [] { return std::make_unique<Wuffs>(); }});
}
#else
void add_wuffs(std::vector<DecoderSpec>&) {}
#endif
