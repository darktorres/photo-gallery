// libjpeg-turbo through the TurboJPEG 3 API, as the app decodes (defaults: accurate DCT, fancy upsampling). The
// reference for every comparison; `turbo-yuv` (planes, no upsampling or colour conversion) is jpeggpu's reference.
#include <turbojpeg.h>

#include "decoder.h"

namespace {

struct Turbo : Decoder {
    bool planar;
    tjhandle h = tj3Init(TJINIT_DECOMPRESS);
    explicit Turbo(bool planar) : planar(planar) {}
    ~Turbo() override { tj3Destroy(h); }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        if (tj3DecompressHeader(h, jpeg.data(), jpeg.size()) != 0) return tj3GetErrorStr(h);
        int w = tj3Get(h, TJPARAM_JPEGWIDTH), ht = tj3Get(h, TJPARAM_JPEGHEIGHT);
        int ss = tj3Get(h, TJPARAM_SUBSAMP);
        out.width = uint32_t(w);
        out.height = uint32_t(ht);
        if (!planar) {
            out.layout = Image::RGB;
            out.use_own(size_t(w) * ht * 3);
            if (tj3Decompress8(h, jpeg.data(), jpeg.size(), out.own.data(), 0, TJPF_RGB) != 0) return tj3GetErrorStr(h);
            return {};
        }
        out.layout = Image::Planar;
        int n = ss == TJSAMP_GRAY ? 1 : 3;
        size_t total = 0, off[3];
        int strides[3];
        out.planes = n;
        for (int c = 0; c < n; c++) {
            off[c] = total;
            strides[c] = tj3YUVPlaneWidth(c, w, ss);
            out.pw[c] = uint32_t(strides[c]);
            out.ph[c] = uint32_t(tj3YUVPlaneHeight(c, ht, ss));
            total += size_t(out.pw[c]) * out.ph[c];
        }
        out.use_own(total);
        unsigned char* planes[3];
        for (int c = 0; c < n; c++) planes[c] = out.own.data() + off[c];
        if (tj3DecompressToYUVPlanes8(h, jpeg.data(), jpeg.size(), planes, strides) != 0) return tj3GetErrorStr(h);
        return {};
    }
};

}  // namespace

void add_turbo(std::vector<DecoderSpec>& v) {
    v.push_back({"turbo", "libjpeg-turbo 3.1.0 (vendored), RGB", [] { return std::make_unique<Turbo>(false); }});
    v.push_back({"turbo-yuv", "libjpeg-turbo, planes only (no upsampling, no colour conversion)",
                 [] { return std::make_unique<Turbo>(true); }});
}
