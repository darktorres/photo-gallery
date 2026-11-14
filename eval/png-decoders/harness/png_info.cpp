#include "png_info.h"

#include <cstring>

namespace {

uint32_t be32(const uint8_t* p) {
    return uint32_t(p[0]) << 24 | uint32_t(p[1]) << 16 | uint32_t(p[2]) << 8 | uint32_t(p[3]);
}

}  // namespace

uint32_t PngInfo::channels() const {
    switch (color) {
        case 0: return trns ? 2 : 1;  // gray
        case 2: return trns ? 4 : 3;  // RGB
        case 3: return trns ? 4 : 3;  // palette
        case 4: return 2;             // gray + alpha
        default: return 4;            // RGBA
    }
}

bool parse_png_info(const std::vector<uint8_t>& png, PngInfo& info, std::string& err) {
    static const uint8_t signature[8] = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};
    if (png.size() < 33 || std::memcmp(png.data(), signature, 8) != 0) {
        err = "not a PNG";
        return false;
    }
    // IHDR is the first chunk: length 13 at offset 8, type at 12, data at 16.
    if (be32(&png[8]) != 13 || std::memcmp(&png[12], "IHDR", 4) != 0) {
        err = "no IHDR chunk first";
        return false;
    }
    const uint8_t* ihdr = &png[16];
    info.width = be32(ihdr);
    info.height = be32(ihdr + 4);
    info.depth = ihdr[8];
    info.color = ihdr[9];
    info.interlace = ihdr[12];
    info.trns = false;
    for (size_t pos = 8; pos + 12 <= png.size();) {
        const uint32_t len = be32(&png[pos]);
        if (pos + 12 + len > png.size() || std::memcmp(&png[pos + 4], "IDAT", 4) == 0) break;
        if (std::memcmp(&png[pos + 4], "tRNS", 4) == 0) info.trns = true;
        pos += 12 + len;
    }
    return true;
}
