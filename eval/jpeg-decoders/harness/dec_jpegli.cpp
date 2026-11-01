// jpegli (github.com/google/jpegli) through its libjpeg-style API. Own file: its jpeglib.h is not libjpeg-turbo's.
#include "decoder.h"

#ifdef HAVE_JPEGLI
#include <csetjmp>
#include <cstdio>

#include "lib/jpegli/decode.h"

namespace {

struct Err {
    jpeg_error_mgr mgr;
    jmp_buf jump;
    char msg[JMSG_LENGTH_MAX];
};

void on_error(j_common_ptr c) {
    auto* e = reinterpret_cast<Err*>(c->err);
    (*c->err->format_message)(c, e->msg);
    longjmp(e->jump, 1);
}

struct Jpegli : Decoder {
    std::string decode(const std::vector<uint8_t>& jpeg, Image& out) override {
        jpeg_decompress_struct d;
        Err e;
        d.err = jpegli_std_error(&e.mgr);
        e.mgr.error_exit = on_error;
        if (setjmp(e.jump)) {
            jpegli_destroy_decompress(&d);
            return e.msg;
        }
        jpegli_create_decompress(&d);
        jpegli_mem_src(&d, jpeg.data(), jpeg.size());
        jpegli_read_header(&d, TRUE);
        d.out_color_space = JCS_RGB;
        jpegli_start_decompress(&d);
        out.layout = Image::RGB;
        out.width = d.output_width;
        out.height = d.output_height;
        size_t stride = size_t(d.output_width) * 3;
        out.use_own(stride * d.output_height);
        while (d.output_scanline < d.output_height) {
            JSAMPROW row = out.own.data() + stride * d.output_scanline;
            jpegli_read_scanlines(&d, &row, 1);
        }
        jpegli_finish_decompress(&d);
        jpegli_destroy_decompress(&d);
        return {};
    }
};

}  // namespace

void add_jpegli(std::vector<DecoderSpec>& v) {
    v.push_back({"jpegli", "jpegli (libjpeg API), RGB", [] { return std::make_unique<Jpegli>(); }});
}
#else
void add_jpegli(std::vector<DecoderSpec>&) {}
#endif
