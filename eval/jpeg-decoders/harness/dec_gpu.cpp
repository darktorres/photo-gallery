// GPU decoders: jpeggpu (github.com/nolmoonen/jpeggpu), GPUJPEG (github.com/CESNET/GPUJPEG) and nvJPEG (CUDA
// toolkit). Each decodes one image per call, copies the result to a pinned host buffer and waits for it, so the
// time is what CPU code after the decode would see. Input and device buffers are kept between calls (grow-only).
#include "decoder.h"

#ifdef HAVE_CUDA
#include <cuda_runtime.h>

#include <cstring>

namespace {

#define CU(x)                                                                     \
    do {                                                                          \
        cudaError_t e_ = (x);                                                     \
        if (e_ != cudaSuccess) return std::string(#x ": ") + cudaGetErrorString(e_); \
    } while (0)

struct Pinned {
    uint8_t* p = nullptr;
    size_t n = 0;
    bool ensure(size_t want) {
        if (want <= n) return true;
        if (p) cudaFreeHost(p);
        n = 0;
        if (cudaMallocHost(reinterpret_cast<void**>(&p), want) != cudaSuccess) return p = nullptr, false;
        n = want;
        return true;
    }
    ~Pinned() { if (p) cudaFreeHost(p); }
};

struct DevBuf {
    uint8_t* p = nullptr;
    size_t n = 0;
    bool ensure(size_t want) {
        if (want <= n) return true;
        if (p) cudaFree(p);
        n = 0;
        if (cudaMalloc(reinterpret_cast<void**>(&p), want) != cudaSuccess) return p = nullptr, false;
        n = want;
        return true;
    }
    ~DevBuf() { if (p) cudaFree(p); }
};

}  // namespace

void gpu_init() { cudaSetDeviceFlags(cudaDeviceScheduleBlockingSync); }

// ---- jpeggpu: planes on the GPU (no upsampling, no colour conversion), copied back as planes ----
#ifdef HAVE_JPEGGPU
#include <jpeggpu/jpeggpu.h>

namespace {

struct JpegGpu : Decoder {
    jpeggpu_decoder_t d = nullptr;
    cudaStream_t s = nullptr;
    Pinned in, host;
    DevBuf tmp, plane[3];
    std::string init;
    JpegGpu() {
        if (jpeggpu_decoder_startup(&d) != JPEGGPU_SUCCESS) init = "jpeggpu_decoder_startup failed";
        cudaStreamCreate(&s);
    }
    ~JpegGpu() override {
        if (d) jpeggpu_decoder_cleanup(d);
        cudaStreamDestroy(s);
    }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        if (!init.empty()) return init;
        if (!in.ensure(jpeg.size())) return "cudaMallocHost";
        std::memcpy(in.p, jpeg.data(), jpeg.size());
        jpeggpu_img_info info;
        auto st = jpeggpu_decoder_parse_header(d, &info, in.p, jpeg.size());
        if (st != JPEGGPU_SUCCESS) return jpeggpu_get_status_string(st);
        size_t tmp_size = 0;
        jpeggpu_decoder_get_buffer_size(d, &tmp_size);
        if (!tmp.ensure(tmp_size)) return "cudaMalloc tmp";
        st = jpeggpu_decoder_transfer(d, tmp.p, tmp_size, s);
        if (st != JPEGGPU_SUCCESS) return jpeggpu_get_status_string(st);
        jpeggpu_img img{};
        size_t total = 0, off[3];
        int n = info.num_components < 3 ? info.num_components : 3;
        for (int c = 0; c < n; c++) {
            int pitch = (info.sizes_x[c] + 7) / 8 * 8;
            if (!plane[c].ensure(size_t(pitch) * info.sizes_y[c])) return "cudaMalloc plane";
            img.image[c] = plane[c].p;
            img.pitch[c] = pitch;
            off[c] = total;
            total += size_t(info.sizes_x[c]) * info.sizes_y[c];
        }
        st = jpeggpu_decoder_decode(d, &img, tmp.p, tmp_size, s);
        if (st != JPEGGPU_SUCCESS) return jpeggpu_get_status_string(st);
        if (!host.ensure(total)) return "cudaMallocHost";
        for (int c = 0; c < n; c++)
            CU(cudaMemcpy2DAsync(host.p + off[c], size_t(info.sizes_x[c]), plane[c].p, size_t(img.pitch[c]),
                                 size_t(info.sizes_x[c]), size_t(info.sizes_y[c]), cudaMemcpyDeviceToHost, s));
        CU(cudaStreamSynchronize(s));
        out.layout = Image::Planar;
        out.width = uint32_t(info.sizes_x[0]);
        out.height = uint32_t(info.sizes_y[0]);
        out.planes = n;
        for (int c = 0; c < n; c++) out.pw[c] = uint32_t(info.sizes_x[c]), out.ph[c] = uint32_t(info.sizes_y[c]);
        out.data = host.p;
        out.size = total;
        return {};
    }
};

}  // namespace
#endif

// ---- GPUJPEG: RGB into its own host buffer ----
#ifdef HAVE_GPUJPEG
#include <libgpujpeg/gpujpeg_decoder.h>

namespace {

struct GpuJpeg : Decoder {
    gpujpeg_decoder* d = nullptr;
    GpuJpeg() {
        auto p = gpujpeg_decoder_default_init_parameters();
        p.verbose = -1;
        d = gpujpeg_decoder_create_with_params(&p);
        if (d) gpujpeg_decoder_set_output_format(d, GPUJPEG_RGB, GPUJPEG_444_U8_P012);
    }
    ~GpuJpeg() override {
        if (d) gpujpeg_decoder_destroy(d);
    }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        if (!d) return "gpujpeg_decoder_create failed";
        gpujpeg_decoder_output o;
        gpujpeg_decoder_output_set_default(&o);
        if (gpujpeg_decoder_decode(d, const_cast<uint8_t*>(jpeg.data()), jpeg.size(), &o) != 0)
            return "gpujpeg_decoder_decode failed";
        out.layout = Image::RGB;
        out.width = uint32_t(o.param_image.width);
        out.height = uint32_t(o.param_image.height);
        out.data = o.data;
        out.size = o.data_size;
        return {};
    }
};

}  // namespace
#endif

// ---- nvJPEG: the simple API (default backend: Huffman on the CPU) and the decoupled API with GPU_HYBRID ----
#ifdef HAVE_NVJPEG
#include <nvjpeg.h>

namespace {

#define NV(x)                                                                   \
    do {                                                                        \
        nvjpegStatus_t s_ = (x);                                                \
        if (s_ != NVJPEG_STATUS_SUCCESS) return std::string(#x ": status ") + std::to_string(int(s_)); \
    } while (0)

// Smooth chroma upsampling, as libjpeg-turbo's fancy upsampling (the default replicates chroma samples).
constexpr unsigned kFlags = NVJPEG_FLAGS_UPSAMPLING_WITH_INTERPOLATION;

struct NvBase : Decoder {
    nvjpegHandle_t h = nullptr;
    cudaStream_t s = nullptr;
    DevBuf dev;
    Pinned host;
    std::string init;
    NvBase() {
        if (nvjpegCreateEx(NVJPEG_BACKEND_DEFAULT, nullptr, nullptr, kFlags, &h) != NVJPEG_STATUS_SUCCESS)
            init = "nvjpegCreateEx failed";
        cudaStreamCreate(&s);
    }
    ~NvBase() override {
        cudaStreamDestroy(s);
        if (h) nvjpegDestroy(h);
    }
    std::string target(const std::vector<uint8_t>& jpeg, Image& out, nvjpegImage_t& im) {
        int nc = 0;
        nvjpegChromaSubsampling_t ss;
        int w[NVJPEG_MAX_COMPONENT], ht[NVJPEG_MAX_COMPONENT];
        NV(nvjpegGetImageInfo(h, jpeg.data(), jpeg.size(), &nc, &ss, w, ht));
        size_t n = size_t(w[0]) * ht[0] * 3;
        if (!dev.ensure(n) || !host.ensure(n)) return "buffer allocation";
        im = {};
        im.channel[0] = dev.p;
        im.pitch[0] = size_t(w[0]) * 3;
        out.layout = Image::RGB;
        out.width = uint32_t(w[0]);
        out.height = uint32_t(ht[0]);
        out.data = host.p;
        out.size = n;
        return {};
    }
    std::string finish(Image& out) {
        CU(cudaMemcpyAsync(host.p, dev.p, out.size, cudaMemcpyDeviceToHost, s));
        CU(cudaStreamSynchronize(s));
        return {};
    }
};

struct NvSimple : NvBase {
    nvjpegJpegState_t st = nullptr;
    NvSimple() {
        if (h && nvjpegJpegStateCreate(h, &st) != NVJPEG_STATUS_SUCCESS) init = "nvjpegJpegStateCreate failed";
    }
    ~NvSimple() override {
        if (st) nvjpegJpegStateDestroy(st);
    }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        if (!init.empty()) return init;
        nvjpegImage_t im;
        if (auto e = target(jpeg, out, im); !e.empty()) return e;
        NV(nvjpegDecode(h, st, jpeg.data(), jpeg.size(), NVJPEG_OUTPUT_RGBI, &im, s));
        return finish(out);
    }
};

struct NvDecoupled : NvBase {
    nvjpegJpegDecoder_t dec = nullptr;
    nvjpegJpegState_t st = nullptr;
    nvjpegBufferPinned_t pb = nullptr;
    nvjpegBufferDevice_t db = nullptr;
    nvjpegJpegStream_t js = nullptr;
    nvjpegDecodeParams_t prm = nullptr;
    explicit NvDecoupled(nvjpegBackend_t backend) {
        if (!h) return;
        bool ok = nvjpegDecoderCreate(h, backend, &dec) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegDecoderStateCreate(h, dec, &st) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegBufferPinnedCreate(h, nullptr, &pb) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegBufferDeviceCreate(h, nullptr, &db) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegStateAttachPinnedBuffer(st, pb) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegStateAttachDeviceBuffer(st, db) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegJpegStreamCreate(h, &js) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegDecodeParamsCreate(h, &prm) == NVJPEG_STATUS_SUCCESS &&
                  nvjpegDecodeParamsSetOutputFormat(prm, NVJPEG_OUTPUT_RGBI) == NVJPEG_STATUS_SUCCESS;
        if (!ok) init = "nvJPEG decoupled setup failed";
    }
    ~NvDecoupled() override {
        if (prm) nvjpegDecodeParamsDestroy(prm);
        if (js) nvjpegJpegStreamDestroy(js);
        if (st) nvjpegJpegStateDestroy(st);
        if (db) nvjpegBufferDeviceDestroy(db);
        if (pb) nvjpegBufferPinnedDestroy(pb);
        if (dec) nvjpegDecoderDestroy(dec);
    }
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        if (!init.empty()) return init;
        nvjpegImage_t im;
        if (auto e = target(jpeg, out, im); !e.empty()) return e;
        NV(nvjpegJpegStreamParse(h, jpeg.data(), jpeg.size(), 0, 0, js));
        NV(nvjpegDecodeJpegHost(h, dec, st, prm, js));
        NV(nvjpegDecodeJpegTransferToDevice(h, dec, st, js, s));
        NV(nvjpegDecodeJpegDevice(h, dec, st, &im, s));
        return finish(out);
    }
};

}  // namespace
#endif

void add_gpu(std::vector<DecoderSpec>& v) {
#ifdef HAVE_NVJPEG
    v.push_back({"nvjpeg", "nvJPEG simple API, default backend (Huffman on the CPU), RGB to pinned host",
                 [] { return std::make_unique<NvSimple>(); }, true});
    v.push_back({"nvjpeg-gpuhuff", "nvJPEG decoupled API, GPU_HYBRID (Huffman on the GPU), RGB to pinned host",
                 [] { return std::make_unique<NvDecoupled>(NVJPEG_BACKEND_GPU_HYBRID); }, true});
#endif
#ifdef HAVE_JPEGGPU
    v.push_back({"jpeggpu", "jpeggpu, planes to pinned host (no upsampling, no colour conversion)",
                 [] { return std::make_unique<JpegGpu>(); }, true});
#endif
#ifdef HAVE_GPUJPEG
    v.push_back({"gpujpeg", "GPUJPEG, RGB to its host buffer (GPU Huffman only with restart markers)",
                 [] { return std::make_unique<GpuJpeg>(); }, true});
#endif
}
#else
void gpu_init() {}
void add_gpu(std::vector<DecoderSpec>&) {}
#endif
