// parajpeg (github.com/anniigr/parajpeg): AVX2 + threads within one image, and its CUDA backend when built with it.
#include "decoder.h"

#ifdef HAVE_PARAJPEG
#include <parajpeg/parajpeg.h>

namespace {

struct Para : Decoder {
    parajpeg::DecodeOptions o;
    Para(parajpeg::Backend b, int threads) {
        o.backend = b;
        o.threads = threads;
        o.format = parajpeg::PixelFormat::RGB;
    }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        parajpeg::ImageInfo info;
        auto s = parajpeg::read_header(jpeg.data(), jpeg.size(), info);
        if (!s) return s.message;
        out.layout = Image::RGB;
        out.width = info.width;
        out.height = info.height;
        out.use_own(size_t(info.width) * info.height * 3);
        s = parajpeg::decode_into(jpeg.data(), jpeg.size(), o, out.own.data(), size_t(info.width) * 3);
        return s ? std::string() : s.message;
    }
};

}  // namespace

void add_parajpeg(std::vector<DecoderSpec>& v) {
    using parajpeg::Backend;
    for (int t : {1, 6, 12})
        v.push_back({"parajpeg-t" + std::to_string(t), "parajpeg CPU (AVX2), " + std::to_string(t) + " thread(s), RGB",
                     [t] { return std::make_unique<Para>(Backend::Cpu, t); }});
    if (parajpeg::backend_available(Backend::Cuda))
        v.push_back({"parajpeg-cuda", "parajpeg CUDA backend, RGB to host",
                     [] { return std::make_unique<Para>(Backend::Cuda, 0); }, true});
}
#else
void add_parajpeg(std::vector<DecoderSpec>&) {}
#endif
