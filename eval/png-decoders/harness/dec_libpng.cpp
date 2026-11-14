// libpng 1.6.59 (pnggroup/libpng) over zlib 1.4, set up as the app's png fork is: text and ICC chunks skipped, EXPAND,
// 16-bit kept. Its read structs are created and destroyed per decode, which is how libpng is used; the row pointers
// png_read_image takes are kept across decodes.
//
// Skipping: the chunk names listed to png_set_keep_unknown_chunks with PNG_HANDLE_CHUNK_NEVER are handled as unknown
// chunks and discarded (pngrutil.c png_handle_unknown: png_crc_finish), so tEXt, zTXt, iTXt and iCCP are never parsed.
//
// Allocations: every libpng allocation, zlib's included (png_zalloc -> png_malloc), goes through the user allocator
// below, which is alloc_count.h's malloc and free.
#include <csetjmp>
#include <cstring>
#include <string>
#include <vector>

#include <png.h>

#include "alloc_count.h"
#include "decoder.h"

namespace {

void* user_malloc(png_structp, png_alloc_size_t size) { return heap::malloc_counted(size); }
void user_free(png_structp, png_voidp ptr) { heap::free_counted(ptr); }

// The error function records the message and jumps back to decode()'s setjmp. Warnings are dropped.
void on_error(png_structp p, png_const_charp message) {
    *static_cast<std::string*>(png_get_error_ptr(p)) = message;
    longjmp(png_jmpbuf(p), 1);
}
void on_warning(png_structp, png_const_charp) {}

struct Source {
    const uint8_t* data;
    size_t size;
    size_t pos;
};

void read_from(png_structp p, png_bytep out, size_t n) {
    Source* src = static_cast<Source*>(png_get_io_ptr(p));
    if (n > src->size - src->pos) png_error(p, "read past the end of the file");
    std::memcpy(out, src->data + src->pos, n);
    src->pos += n;
}

class Libpng final : public Decoder {
public:
    Result decode(const std::vector<uint8_t>& png, const PngInfo&) override {
        Source src{png.data(), png.size(), 0};
        std::string error;
        png_structp p = png_create_read_struct_2(PNG_LIBPNG_VER_STRING, &error, on_error, on_warning, nullptr, user_malloc,
                                                 user_free);
        if (!p) return {Status::Failed, "png_create_read_struct_2"};
        png_infop info = png_create_info_struct(p);
        if (!info) {
            png_destroy_read_struct(&p, nullptr, nullptr);
            return {Status::Failed, "png_create_info_struct"};
        }
        if (setjmp(png_jmpbuf(p))) {
            png_destroy_read_struct(&p, &info, nullptr);
            return {Status::Failed, error};
        }
        png_set_read_fn(p, &src, read_from);

        // Text and ICC skipped: the listed names are 5 bytes each (4 name bytes and a pad), as libpng's own table has them.
        static const png_byte skipped[] = {'t', 'E', 'X', 't', 0, 'z', 'T', 'X', 't', 0,
                                           'i', 'T', 'X', 't', 0, 'i', 'C', 'C', 'P', 0};
        png_set_keep_unknown_chunks(p, PNG_HANDLE_CHUNK_NEVER, skipped, 4);
        png_read_info(p, info);
        png_set_expand(p);              // palette to RGB, tRNS to alpha, gray below 8 bits to 8 (16-bit kept)
        png_set_interlace_handling(p);  // Adam7: the passes of png_read_image
        png_read_update_info(p, info);

        const png_uint_32 width = png_get_image_width(p, info), height = png_get_image_height(p, info);
        const size_t row_bytes = png_get_rowbytes(p, info);
        const uint32_t channels = png_get_channels(p, info);
        const uint32_t bytes_per_sample = png_get_bit_depth(p, info) / 8;
        out_.resize(row_bytes * height);
        rows_.resize(height);
        for (png_uint_32 y = 0; y < height; y++) rows_[y] = out_.data() + size_t(y) * row_bytes;
        png_read_image(p, rows_.data());
        png_read_end(p, nullptr);
        png_destroy_read_struct(&p, &info, nullptr);

        dims_ = {width, height, channels, bytes_per_sample};
        return {};
    }

private:
    std::vector<png_bytep> rows_;  // kept across decodes
};

}  // namespace

void add_libpng(std::vector<DecoderSpec>& v) {
    v.push_back({"libpng", [] { return std::make_unique<Libpng>(); }});
}
