// The PNG decoders behind one interface, for the C side of the PNG decoder comparison. Every decoder gives the app's
// layout (the png fork's EXPAND output: palette and tRNS to RGB(A), gray below 8 bits to 8, 16-bit kept as big-endian
// samples, packed row by row, no padding), one thread, full size, into an output buffer it keeps across decodes.
#pragma once
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "png_info.h"

// Unsupported: the library cannot give the app's layout for this file, so its output is not compared. Failed: an error.
enum class Status { Ok, Unsupported, Failed };

struct Result {
    Status status = Status::Ok;
    std::string why;  // the reason, when not Ok
};

class Decoder {
public:
    virtual ~Decoder() = default;

    // Decodes the PNG whose bytes are `png` into out(). `info` is the file's header (png_info.h): a decoder may use it
    // to choose its output format, never to produce pixels.
    virtual Result decode(const std::vector<uint8_t>& png, const PngInfo& info) = 0;

    const std::vector<uint8_t>& out() const { return out_; }
    const Dims& dims() const { return dims_; }

protected:
    std::vector<uint8_t> out_;  // kept across decodes, as the app's fork keeps its output
    Dims dims_;
};

struct DecoderSpec {
    std::string name;
    std::function<std::unique_ptr<Decoder>()> make;
};

// Each dec_*.cpp adds its decoder.
void add_libpng(std::vector<DecoderSpec>&);
void add_libspng(std::vector<DecoderSpec>&);
void add_wuffs(std::vector<DecoderSpec>&);
