// One interface for every decoder the harness measures (docs/JPEG_DECODERS.md).
#pragma once
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

// A decoded image in host memory. `data` points at `own` for CPU decoders and at a pinned staging buffer (owned by
// the decoder, valid until its next decode) for GPU decoders, which is where CPU code after the decode would read.
struct Image {
    enum Layout { RGB, RGBX, Planar } layout = RGB;  // Planar: Y, Cb, Cr planes back to back, unpadded, not upsampled
    uint32_t width = 0, height = 0;
    int planes = 0;                                // Planar only: plane c is pw[c] x ph[c], tightly packed
    uint32_t pw[3] = {}, ph[3] = {};
    const uint8_t* data = nullptr;
    size_t size = 0;
    std::vector<uint8_t> own;
    void use_own(size_t n) {
        if (own.size() < n) own.resize(n);
        data = own.data();
        size = n;
    }
};

struct Decoder {
    virtual ~Decoder() = default;
    // Decode one JPEG fully (no DCT scaling) into `out`. Returns an error message, empty on success.
    virtual std::string decode(const std::vector<uint8_t>& jpeg, Image& out) = 0;
};

struct DecoderSpec {
    std::string name;
    std::string about;  // one line: library, backend, output
    std::function<std::unique_ptr<Decoder>()> make;
    bool gpu = false;
};

// Each dec_*.cpp adds its decoders; the ones not compiled in are absent.
void add_turbo(std::vector<DecoderSpec>&);
void add_parajpeg(std::vector<DecoderSpec>&);
void add_jpegli(std::vector<DecoderSpec>&);
void add_wuffs(std::vector<DecoderSpec>&);
void add_gpu(std::vector<DecoderSpec>&);  // jpeggpu, GPUJPEG, nvJPEG
void gpu_init();  // blocking sync before any CUDA context exists, so CPU time is real work, not spinning
