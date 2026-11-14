// The header of a PNG, as the decoders' output layout depends on it (png_info.cpp).
#pragma once
#include <cstdint>
#include <string>
#include <vector>

struct Dims {
    uint32_t width = 0, height = 0, channels = 0, bytes_per_sample = 0;
};

struct PngInfo {
    uint32_t width = 0, height = 0;
    uint8_t depth = 0, color = 0, interlace = 0;
    bool trns = false;  // a tRNS chunk before IDAT

    // The channels of the app's layout, after EXPAND (palette and tRNS to RGB(A), gray with tRNS to gray+alpha).
    uint32_t channels() const;
    uint32_t bytes_per_sample() const { return depth == 16 ? 2 : 1; }
};

// Reads IHDR and whether a tRNS chunk comes before IDAT. False, with `err` set, when the file is not a PNG.
bool parse_png_info(const std::vector<uint8_t>& png, PngInfo& info, std::string& err);
