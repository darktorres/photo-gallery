// test_encoder.cpp - test driver for the batched GPU baseline JPEG encoder.
//
// Usage: test_encoder.exe [--tests 1,2,...] [--sample DIR] [--work DIR] [--ssim EXE] [--no-ssim]
// Tests: 1 entropy exactness, 2 coefficient agreement, 3 valid JPEG, 4 batch invariance,
//        5 edge sizes, 6 quality vs libjpeg-turbo, 7 throughput.

#include "encode_gpu.h"

#include <cuda_runtime.h>

#include <cstdio>   // jpeglib.h uses FILE and must see it first
#include <cstddef>

extern "C" {
#include <jpeglib.h>
#include <turbojpeg.h>
}

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <random>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include <setjmp.h>

namespace fs = std::filesystem;

// ---------------------------------------------------------------- options and reporting

static std::string g_sample_dir = "C:/Users/Torres/AppData/Local/Temp/claude/2c16f13ff155207f/1d1a3c2b-7598-4f16-b017-c47c7c340cad/scratchpad/gpu_sample";
static std::string g_work_dir = "C:/Users/Torres/AppData/Local/Temp/claude/2c16f13ff155207f/1d1a3c2b-7598-4f16-b017-c47c7c340cad/scratchpad/jpeg_encoder_work";
static std::string g_ssim_exe = "D:/PhotoGallery/tools/jpeg-eval/build/jpegli/tools/ssimulacra2.exe";
static bool g_use_ssim = true;
static int g_failures = 0;

static void report(const char* name, bool pass, const std::string& detail) {
    printf("[%s] %s\n       %s\n", pass ? "PASS" : "FAIL", name, detail.c_str());
    fflush(stdout);
    if (!pass) ++g_failures;
}

static std::string fmt(const char* f, ...) {
    char buf[1024];
    va_list ap;
    va_start(ap, f);
    vsnprintf(buf, sizeof buf, f, ap);
    va_end(ap);
    return buf;
}

// ---------------------------------------------------------------- images

struct Rgb {
    int w = 0, h = 0;
    std::vector<uint8_t> px;
};

// kind 0 gradient, 1 uniform noise, 2 flat with sparse noise (long zero runs), 3 checker edges.
static Rgb make_image(int w, int h, int kind, uint32_t seed) {
    Rgb im;
    im.w = w;
    im.h = h;
    im.px.resize((size_t)w * h * 3);
    std::mt19937 rng(seed);
    for (int y = 0; y < h; ++y) {
        for (int x = 0; x < w; ++x) {
            uint8_t* p = &im.px[((size_t)y * w + x) * 3];
            switch (kind) {
                case 0:
                    p[0] = (uint8_t)(w > 1 ? x * 255 / (w - 1) : 0);
                    p[1] = (uint8_t)(h > 1 ? y * 255 / (h - 1) : 0);
                    p[2] = (uint8_t)((x + y) & 255);
                    break;
                case 1:
                    for (int c = 0; c < 3; ++c) p[c] = (uint8_t)(rng() & 255);
                    break;
                case 2:
                    p[0] = 90; p[1] = 120; p[2] = 200;
                    if (rng() % 100 == 0) {
                        for (int c = 0; c < 3; ++c) p[c] = (uint8_t)(rng() & 255);
                    }
                    break;
                default: {
                    const uint8_t v = (((x / 8) + (y / 8)) & 1) ? 255 : 0;
                    p[0] = p[1] = p[2] = v;
                    break;
                }
            }
        }
    }
    return im;
}

// libjpeg error manager: counts warnings, longjmps on fatal errors, silences messages.
struct JerrMgr {
    jpeg_error_mgr pub;
    jmp_buf jb;
    int warnings;
};

static void jerr_exit(j_common_ptr cinfo) {
    JerrMgr* m = reinterpret_cast<JerrMgr*>(cinfo->err);
    longjmp(m->jb, 1);
}

static void jerr_emit(j_common_ptr cinfo, int level) {
    if (level < 0) reinterpret_cast<JerrMgr*>(cinfo->err)->warnings++;
}

static void jerr_output(j_common_ptr) {}

// Decodes to RGB with libjpeg. Returns false on a fatal error.
static bool decode_rgb(const uint8_t* data, size_t len, Rgb& out, int& warnings) {
    jpeg_decompress_struct d;
    JerrMgr em;
    em.warnings = 0;
    d.err = jpeg_std_error(&em.pub);
    em.pub.error_exit = jerr_exit;
    em.pub.emit_message = jerr_emit;
    em.pub.output_message = jerr_output;
    if (setjmp(em.jb)) {
        jpeg_destroy_decompress(&d);
        warnings = em.warnings;
        return false;
    }
    jpeg_create_decompress(&d);
    jpeg_mem_src(&d, data, (unsigned long)len);
    jpeg_read_header(&d, TRUE);
    d.out_color_space = JCS_RGB;
    jpeg_start_decompress(&d);
    out.w = (int)d.output_width;
    out.h = (int)d.output_height;
    out.px.resize((size_t)out.w * out.h * 3);
    while (d.output_scanline < d.output_height) {
        JSAMPROW row = out.px.data() + (size_t)d.output_scanline * out.w * 3;
        jpeg_read_scanlines(&d, &row, 1);
    }
    jpeg_finish_decompress(&d);
    warnings = em.warnings;
    jpeg_destroy_decompress(&d);
    return true;
}

static bool load_sample(int index, Rgb& out) {
    std::ifstream f(fmt("%s/%03d.jpg", g_sample_dir.c_str(), index), std::ios::binary);
    if (!f) return false;
    std::vector<uint8_t> data((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    int warn = 0;
    return decode_rgb(data.data(), data.size(), out, warn);
}

static double psnr(const Rgb& a, const Rgb& b) {
    if (a.w != b.w || a.h != b.h) return -1.0;
    double sse = 0.0;
    for (size_t i = 0; i < a.px.size(); ++i) {
        const double d = (double)a.px[i] - (double)b.px[i];
        sse += d * d;
    }
    const double mse = sse / (double)a.px.size();
    if (mse <= 0.0) return 100.0;
    return 10.0 * std::log10(255.0 * 255.0 / mse);
}

static double median(std::vector<double> v) {
    if (v.empty()) return 0.0;
    std::sort(v.begin(), v.end());
    const size_t n = v.size();
    return n % 2 ? v[n / 2] : 0.5 * (v[n / 2 - 1] + v[n / 2]);
}

// Nearest-rank percentile.
static double percentile(std::vector<double> v, double p) {
    if (v.empty()) return 0.0;
    std::sort(v.begin(), v.end());
    size_t idx = (size_t)std::ceil(p * (double)v.size());
    if (idx < 1) idx = 1;
    if (idx > v.size()) idx = v.size();
    return v[idx - 1];
}

// ---------------------------------------------------------------- GPU encode helper

struct GpuResult {
    int rc = 0;
    std::string err;
    std::vector<std::vector<uint8_t>> jpg;
    std::vector<std::vector<int16_t>> coef;
    EgpuTimings t{};
};

static GpuResult gpu_encode(const std::vector<const Rgb*>& imgs, int quality, bool c420, uint64_t budget, bool keep) {
    GpuResult r;
    EgpuOptions o;
    o.quality = quality;
    o.chroma_420 = c420 ? 1 : 0;
    o.device = 0;
    o.pixel_budget = budget;
    o.keep_coefficients = keep ? 1 : 0;
    EgpuContext* ctx = nullptr;
    if ((r.rc = egpu_create(&o, &ctx)) != EGPU_OK) {
        r.err = egpu_last_error();
        return r;
    }
    const uint32_t n = (uint32_t)imgs.size();
    std::vector<EgpuImage> desc(n);
    std::vector<uint8_t> host;
    for (uint32_t i = 0; i < n; ++i) {
        desc[i].offset = host.size();
        desc[i].width = (uint32_t)imgs[i]->w;
        desc[i].height = (uint32_t)imgs[i]->h;
        host.insert(host.end(), imgs[i]->px.begin(), imgs[i]->px.end());
    }
    uint8_t* d = nullptr;
    const size_t bytes = host.empty() ? 1 : host.size();
    if (cudaMalloc((void**)&d, bytes) != cudaSuccess) {
        r.rc = EGPU_ERR_MEMORY;
        r.err = "cudaMalloc failed in test";
        egpu_destroy(ctx);
        return r;
    }
    if (!host.empty()) cuda_ok(cudaMemcpy(d, host.data(), host.size(), cudaMemcpyHostToDevice), "cudaMemcpy");
    r.rc = egpu_encode_batch(ctx, d, host.size(), desc.data(), n);
    if (r.rc != EGPU_OK) {
        r.err = egpu_last_error();
    } else {
        r.jpg.resize(n);
        if (keep) r.coef.resize(n);
        for (uint32_t i = 0; i < n; ++i) {
            const uint8_t* p = nullptr;
            uint64_t sz = 0;
            egpu_output(ctx, i, &p, &sz);
            r.jpg[i].assign(p, p + sz);
            if (keep) {
                const int16_t* c = nullptr;
                uint64_t blocks = 0;
                egpu_coefficients(ctx, i, &c, &blocks);
                r.coef[i].assign(c, c + blocks * 64);
            }
        }
        egpu_timings(ctx, &r.t);
    }
    cudaFree(d);
    egpu_destroy(ctx);
    return r;
}

static std::vector<uint8_t> turbo_encode(const Rgb& im, int q, bool c420) {
    tjhandle h = tjInitCompress();
    unsigned char* buf = nullptr;
    unsigned long sz = 0;
    tjCompress2(h, im.px.data(), im.w, 0, im.h, TJPF_RGB, &buf, &sz, c420 ? TJSAMP_420 : TJSAMP_444, q, 0);
    std::vector<uint8_t> out(buf, buf + sz);
    tjFree(buf);
    tjDestroy(h);
    return out;
}

// ---------------------------------------------------------------- JPEG structure helpers

// Returns the entropy-coded bytes between SOS and EOI, found by walking the marker segments.
static bool scan_bytes(const std::vector<uint8_t>& j, std::vector<uint8_t>& scan) {
    if (j.size() < 4 || j[0] != 0xFF || j[1] != 0xD8) return false;
    size_t pos = 2;
    while (pos + 4 <= j.size()) {
        if (j[pos] != 0xFF) return false;
        const uint8_t marker = j[pos + 1];
        if (marker == 0xFF) {
            ++pos;
            continue;
        }
        const size_t len = ((size_t)j[pos + 2] << 8) | j[pos + 3];
        if (marker == 0xDA) {
            const size_t start = pos + 2 + len;
            if (j.size() < start + 2 || j[j.size() - 2] != 0xFF || j[j.size() - 1] != 0xD9) return false;
            scan.assign(j.begin() + start, j.end() - 2);
            return true;
        }
        pos += 2 + len;
    }
    return false;
}

// Removes the 0x00 stuffed after each 0xFF data byte.
static std::vector<uint8_t> unstuff(const std::vector<uint8_t>& s) {
    std::vector<uint8_t> out;
    out.reserve(s.size());
    for (size_t i = 0; i < s.size(); ++i) {
        if (i > 0 && s[i - 1] == 0xFF && s[i] == 0x00) continue;
        out.push_back(s[i]);
    }
    return out;
}

// Canonical Huffman codes (T.81 C.2) from a libjpeg table.
struct Codes {
    uint32_t code[256] = {};
    uint32_t len[256] = {};
};

static void make_codes(const JHUFF_TBL* t, Codes& c) {
    uint32_t code = 0;
    int k = 0;
    for (int l = 1; l <= 16; ++l) {
        for (int j = 0; j < t->bits[l]; ++j) {
            const uint8_t sym = t->huffval[k++];
            c.code[sym] = code++;
            c.len[sym] = (uint32_t)l;
        }
        code <<= 1;
    }
}

static const int kZig[64] = {0,  1,  8,  16, 9,  2,  3,  10, 17, 24, 32, 25, 18, 11, 4,  5,
                             12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6,  7,  14, 21, 28,
                             35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
                             58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63};

static int nbits(int v) {
    int a = v < 0 ? -v : v, s = 0;
    while (a) { ++s; a >>= 1; }
    return s;
}

// Bit offset of each block in the unstuffed scan, using the spec's symbol rules.
// codes: [0] DC luma, [1] AC luma, [2] DC chroma, [3] AC chroma. Block g is in scan order.
static std::vector<uint64_t> block_starts(const std::vector<int16_t>& coef, bool c420, const Codes* codes, size_t nblocks) {
    std::vector<uint64_t> start(nblocks + 1, 0);
    const size_t bpm = c420 ? 6 : 3;
    int pred[3] = {0, 0, 0};
    uint64_t pos = 0;
    for (size_t g = 0; g < nblocks; ++g) {
        const size_t k = g % bpm;
        const int comp = c420 ? (k < 4 ? 0 : (int)k - 3) : (int)k;
        const int16_t* b = &coef[g * 64];
        const Codes& dc = codes[comp ? 2 : 0];
        const Codes& ac = codes[comp ? 3 : 1];
        start[g] = pos;
        const int diff = b[0] - pred[comp];
        pred[comp] = b[0];
        const int s = nbits(diff);
        pos += dc.len[s] + s;
        int run = 0;
        for (int z = 1; z < 64; ++z) {
            int v = b[kZig[z]];
            if (v == 0) { ++run; continue; }
            while (run > 15) { pos += ac.len[0xF0]; run -= 16; }
            if (v > 1023) v = 1023;
            if (v < -1023) v = -1023;
            const int sz = nbits(v);
            pos += ac.len[(run << 4) | sz] + sz;
            run = 0;
        }
        if (run > 0) pos += ac.len[0x00];
    }
    start[nblocks] = pos;
    return start;
}

// Block-grid layout used by both libjpeg's coefficient arrays and the GPU's scan order.
struct Layout {
    int w, h, mcus_x, mcus_y;
    bool c420;
};

static Layout make_layout(int w, int h, bool c420) {
    const int mcu = c420 ? 16 : 8;
    return Layout{w, h, (w + mcu - 1) / mcu, (h + mcu - 1) / mcu, c420};
}

static int hs(const Layout& L, int comp) { return (comp == 0 && L.c420) ? 2 : 1; }
static int grid_w(const Layout& L, int comp) { return L.mcus_x * hs(L, comp); }
static int grid_h(const Layout& L, int comp) { return L.mcus_y * hs(L, comp); }

// Real (non-padding) block extent of a component, as libjpeg reports it.
static int real_w(const Layout& L, int comp) {
    const int maxh = L.c420 ? 2 : 1;
    const int dw = (L.w * hs(L, comp) + maxh - 1) / maxh;
    return (dw + 7) / 8;
}
static int real_h(const Layout& L, int comp) {
    const int maxv = L.c420 ? 2 : 1;
    const int dh = (L.h * hs(L, comp) + maxv - 1) / maxv;
    return (dh + 7) / 8;
}

// GPU scan-order block index for component comp at block-grid position (r, c).
static size_t gpu_block(const Layout& L, int comp, int r, int c) {
    if (!L.c420) return ((size_t)r * L.mcus_x + c) * 3 + comp;
    if (comp == 0) {
        const size_t mcu = (size_t)(r / 2) * L.mcus_x + c / 2;
        return mcu * 6 + (size_t)(r % 2) * 2 + (c % 2);
    }
    return ((size_t)r * L.mcus_x + c) * 6 + 3 + comp;
}

static size_t total_blocks(const Layout& L) {
    return (size_t)L.mcus_x * L.mcus_y * (L.c420 ? 6 : 3);
}

// Finds the block containing unstuffed bit position `bit`, and its component.
static size_t locate_block(const std::vector<uint64_t>& start, size_t nblocks, uint64_t bit, bool c420, int& comp) {
    size_t lo = 0, hi = nblocks;
    while (hi - lo > 1) {
        const size_t mid = (lo + hi) / 2;
        if (start[mid] <= bit) lo = mid; else hi = mid;
    }
    const size_t k = lo % (c420 ? 6 : 3);
    comp = c420 ? (k < 4 ? 0 : (int)k - 3) : (int)k;
    return lo;
}

// ---------------------------------------------------------------- libjpeg encode from coefficients

// Encodes with libjpeg's coefficient API from the given natural-order blocks (scan order).
// Also returns libjpeg's standard Huffman codes for diagnostics.
static bool libjpeg_from_coefs(const Layout& L, const uint16_t ql[64], const uint16_t qc[64],
                               const std::vector<int16_t>& coef, std::vector<uint8_t>& out, Codes* codes) {
    jpeg_compress_struct c;
    JerrMgr em;
    em.warnings = 0;
    c.err = jpeg_std_error(&em.pub);
    em.pub.error_exit = jerr_exit;
    em.pub.emit_message = jerr_emit;
    em.pub.output_message = jerr_output;
    unsigned char* buf = nullptr;
    unsigned long len = 0;
    if (setjmp(em.jb)) {
        jpeg_destroy_compress(&c);
        free(buf);
        return false;
    }
    jpeg_create_compress(&c);
    jpeg_mem_dest(&c, &buf, &len);
    c.image_width = (JDIMENSION)L.w;
    c.image_height = (JDIMENSION)L.h;
    c.input_components = 3;
    c.in_color_space = JCS_YCbCr;
    jpeg_set_defaults(&c);
    jpeg_set_colorspace(&c, JCS_YCbCr);
    c.optimize_coding = FALSE;
    for (int comp = 0; comp < 3; ++comp) {
        c.comp_info[comp].h_samp_factor = hs(L, comp);
        c.comp_info[comp].v_samp_factor = hs(L, comp);
        c.comp_info[comp].quant_tbl_no = comp ? 1 : 0;
        c.comp_info[comp].dc_tbl_no = comp ? 1 : 0;
        c.comp_info[comp].ac_tbl_no = comp ? 1 : 0;
    }
    for (int i = 0; i < 64; ++i) {
        c.quant_tbl_ptrs[0]->quantval[i] = ql[i];
        c.quant_tbl_ptrs[1]->quantval[i] = qc[i];
    }
    if (codes) {
        make_codes(c.dc_huff_tbl_ptrs[0], codes[0]);
        make_codes(c.ac_huff_tbl_ptrs[0], codes[1]);
        make_codes(c.dc_huff_tbl_ptrs[1], codes[2]);
        make_codes(c.ac_huff_tbl_ptrs[1], codes[3]);
    }
    jvirt_barray_ptr arrays[3];
    for (int comp = 0; comp < 3; ++comp) {
        const JDIMENSION bw = (JDIMENSION)grid_w(L, comp), bh = (JDIMENSION)grid_h(L, comp);
        arrays[comp] = (*c.mem->request_virt_barray)((j_common_ptr)&c, JPOOL_IMAGE, FALSE, bw, bh, bh);
    }
    jpeg_write_coefficients(&c, arrays);
    for (int comp = 0; comp < 3; ++comp) {
        const int bh = grid_h(L, comp), bw = grid_w(L, comp);
        JBLOCKARRAY rows = (*c.mem->access_virt_barray)((j_common_ptr)&c, arrays[comp], 0, (JDIMENSION)bh, TRUE);
        for (int r = 0; r < bh; ++r) {
            for (int col = 0; col < bw; ++col) {
                const size_t g = gpu_block(L, comp, r, col);
                for (int i = 0; i < 64; ++i) rows[r][col][i] = coef[g * 64 + i];
            }
        }
    }
    jpeg_finish_compress(&c);
    out.assign(buf, buf + len);
    free(buf);
    jpeg_destroy_compress(&c);
    return em.warnings == 0;
}

// Standard libjpeg encode of RGB, for the coefficient reference in test 2.
static bool libjpeg_encode_rgb(const Rgb& im, int q, bool c420, std::vector<uint8_t>& out) {
    jpeg_compress_struct c;
    JerrMgr em;
    em.warnings = 0;
    c.err = jpeg_std_error(&em.pub);
    em.pub.error_exit = jerr_exit;
    em.pub.emit_message = jerr_emit;
    em.pub.output_message = jerr_output;
    unsigned char* buf = nullptr;
    unsigned long len = 0;
    if (setjmp(em.jb)) {
        jpeg_destroy_compress(&c);
        free(buf);
        return false;
    }
    jpeg_create_compress(&c);
    jpeg_mem_dest(&c, &buf, &len);
    c.image_width = (JDIMENSION)im.w;
    c.image_height = (JDIMENSION)im.h;
    c.input_components = 3;
    c.in_color_space = JCS_RGB;
    jpeg_set_defaults(&c);
    jpeg_set_quality(&c, q, TRUE);
    jpeg_set_colorspace(&c, JCS_YCbCr);
    c.dct_method = JDCT_ISLOW;
    c.optimize_coding = FALSE;
    if (c420) {
        c.comp_info[0].h_samp_factor = 2;
        c.comp_info[0].v_samp_factor = 2;
    } else {
        c.comp_info[0].h_samp_factor = 1;
        c.comp_info[0].v_samp_factor = 1;
    }
    c.comp_info[1].h_samp_factor = c.comp_info[2].h_samp_factor = 1;
    c.comp_info[1].v_samp_factor = c.comp_info[2].v_samp_factor = 1;
    jpeg_start_compress(&c, TRUE);
    while (c.next_scanline < c.image_height) {
        JSAMPROW row = const_cast<uint8_t*>(im.px.data()) + (size_t)c.next_scanline * im.w * 3;
        jpeg_write_scanlines(&c, &row, 1);
    }
    jpeg_finish_compress(&c);
    out.assign(buf, buf + len);
    free(buf);
    jpeg_destroy_compress(&c);
    return true;
}

// Reads the coefficients back from a libjpeg file and compares them with the supplied blocks.
// Returns the number of mismatched coefficients in the real (non-padding) blocks.
static size_t roundtrip_mismatches(const std::vector<uint8_t>& jpg, const Layout& L,
                                   const std::vector<int16_t>& coef, std::string& first) {
    jpeg_decompress_struct d;
    JerrMgr em;
    em.warnings = 0;
    d.err = jpeg_std_error(&em.pub);
    em.pub.error_exit = jerr_exit;
    em.pub.emit_message = jerr_emit;
    em.pub.output_message = jerr_output;
    if (setjmp(em.jb)) {
        jpeg_destroy_decompress(&d);
        first = "read error";
        return (size_t)-1;
    }
    jpeg_create_decompress(&d);
    jpeg_mem_src(&d, jpg.data(), (unsigned long)jpg.size());
    jpeg_read_header(&d, TRUE);
    jvirt_barray_ptr* arr = jpeg_read_coefficients(&d);
    size_t mism = 0;
    for (int comp = 0; comp < 3; ++comp) {
        // Full padded grid, including the padding blocks at the right and bottom edges.
        const int rh = grid_h(L, comp), rw = grid_w(L, comp);
        for (int r = 0; r < rh; ++r) {
            JBLOCKARRAY rows = (*d.mem->access_virt_barray)((j_common_ptr)&d, arr[comp], (JDIMENSION)r, 1, FALSE);
            for (int col = 0; col < rw; ++col) {
                const size_t bi = gpu_block(L, comp, r, col);
                for (int i = 0; i < 64; ++i) {
                    if (rows[0][col][i] != coef[bi * 64 + i]) {
                        if (mism == 0) {
                            first = fmt("comp %d block (%d,%d) coef %d: supplied %d, read back %d", comp, r, col, i,
                                        (int)coef[bi * 64 + i], (int)rows[0][col][i]);
                        }
                        ++mism;
                    }
                }
            }
        }
    }
    jpeg_finish_decompress(&d);
    jpeg_destroy_decompress(&d);
    return mism;
}

// ---------------------------------------------------------------- test 1: entropy exactness

static bool test1_entropy() {
    std::vector<Rgb> imgs;
    for (int i = 1; i <= 3; ++i) {
        Rgb s;
        if (load_sample(i, s)) imgs.push_back(std::move(s));
    }
    imgs.push_back(make_image(97, 61, 0, 1));
    imgs.push_back(make_image(129, 77, 2, 2));
    imgs.push_back(make_image(64, 64, 1, 3));
    imgs.push_back(make_image(200, 40, 3, 4));

    struct Cfg { bool c420; int q; };
    const std::vector<Cfg> cfgs = {{true, 50}, {true, 75}, {true, 100}, {false, 50}, {false, 75}, {false, 100}};
    int checked = 0, identical = 0;
    std::string first_fail, first_dump;
    for (const Cfg& cfg : cfgs) {
        for (size_t ii = 0; ii < imgs.size(); ++ii) {
            const Rgb& im = imgs[ii];
            GpuResult g = gpu_encode({&im}, cfg.q, cfg.c420, 0, true);
            if (g.rc != EGPU_OK) {
                first_fail = fmt("gpu encode failed: %s", g.err.c_str());
                continue;
            }
            uint16_t ql[64], qc[64];
            egpu_quant_tables(cfg.q, ql, qc);
            const Layout L = make_layout(im.w, im.h, cfg.c420);
            std::vector<uint8_t> lj;
            Codes codes[4];
            if (!libjpeg_from_coefs(L, ql, qc, g.coef[0], lj, codes)) {
                first_fail = "libjpeg coefficient encode failed";
                continue;
            }
            std::vector<uint8_t> sg, sl;
            if (!scan_bytes(g.jpg[0], sg) || !scan_bytes(lj, sl)) {
                first_fail = "could not locate scan data";
                continue;
            }
            ++checked;
            std::string rt;
            const size_t rtm = roundtrip_mismatches(lj, L, g.coef[0], rt);
            if (sg == sl) {
                ++identical;
                if (rtm != 0) first_fail = fmt("round trip mismatches %zu (%s)", rtm, rt.c_str());
                continue;
            }
            // Compare unstuffed data so the first difference maps to a bit position.
            const std::vector<uint8_t> ug = unstuff(sg), ul = unstuff(sl);
            size_t bu = 0;
            while (bu < ug.size() && bu < ul.size() && ug[bu] == ul[bu]) ++bu;
            const size_t nblocks = total_blocks(L);
            int comp = -1;
            const auto starts = block_starts(g.coef[0], cfg.c420, codes, nblocks);
            const uint64_t expected_bits = starts[nblocks];
            const size_t blk = locate_block(starts, nblocks, (uint64_t)bu * 8, cfg.c420, comp);
            if (first_fail.empty()) {
                // Dump the failing block's nonzero coefficients and both streams' bits from its start.
                std::string dump = "\n       block coefficients (zigzag index:value):";
                const int16_t* bb = &g.coef[0][blk * 64];
                for (int z = 0; z < 64; ++z) {
                    if (bb[kZig[z]] != 0) dump += fmt(" %d:%d", z, (int)bb[kZig[z]]);
                }
                auto bits_at = [](const std::vector<uint8_t>& u, uint64_t from, int n) {
                    std::string s;
                    for (int b = 0; b < n; ++b) {
                        const uint64_t p = from + (uint64_t)b;
                        s += ((p >> 3) < u.size()) ? (((u[p >> 3] >> (7 - (p & 7))) & 1) ? '1' : '0') : '?';
                    }
                    return s;
                };
                const std::string bg = bits_at(ug, starts[blk], 160), bl = bits_at(ul, starts[blk], 160);
                size_t dbit = 0;
                while (dbit < bg.size() && bg[dbit] == bl[dbit]) ++dbit;
                dump += fmt("\n       first differing bit relative to block start: %zu (block length GPU %llu bits, next block starts at %llu)",
                            dbit, (unsigned long long)(starts[blk + 1] - starts[blk]), (unsigned long long)starts[blk + 1]);
                dump += fmt("\n       bits from block start: GPU    %s", bg.c_str());
                dump += fmt("\n       bits from block start: libjpeg %s", bl.c_str());
                first_dump = dump;
            }
            if (first_fail.empty()) first_fail = fmt("%s q%d image %zu (%dx%d): first differing unstuffed byte %zu (bit %zu of block %zu, comp %d, block starts at bit %llu); "
                             "GPU entropy %zu bytes, host-predicted %llu bits (%llu bytes), libjpeg %zu bytes; "
                             "round trip of libjpeg output: %zu mismatched coefficients (%s)",
                             cfg.c420 ? "4:2:0" : "4:4:4", cfg.q, ii, im.w, im.h, bu, (bu * 8) % 8, blk, comp,
                             (unsigned long long)starts[blk], ug.size(), (unsigned long long)expected_bits,
                             (unsigned long long)((expected_bits + 7) / 8), ul.size(), rtm, rt.c_str());
        }
    }
    const bool pass = identical == checked && checked > 0;
    std::string detail = fmt("%d of %d scan streams byte-identical to libjpeg-turbo (configs: 4:2:0 and 4:4:4 at q50/75/100, %zu images)",
                             identical, checked, imgs.size());
    if (!first_fail.empty()) detail += "; " + first_fail + first_dump;
    report("1 entropy exactness", pass, detail);
    return pass;
}

// ---------------------------------------------------------------- test 2: coefficient agreement

static bool test2_coefficients() {
    std::vector<Rgb> imgs;
    for (int i = 1; i <= 3; ++i) {
        Rgb s;
        if (load_sample(i, s)) imgs.push_back(std::move(s));
    }
    imgs.push_back(make_image(96, 80, 0, 5));
    imgs.push_back(make_image(70, 45, 2, 6));

    bool pass = true;
    for (int c420i = 0; c420i < 2; ++c420i) {
        const bool c420 = c420i == 1;
        for (int q : {50, 75, 90}) {
            size_t total = 0, within1 = 0, exact = 0;
            int maxdiff = 0;
            std::string why;   // first failure reasons, if any
            for (const Rgb& im : imgs) {
                GpuResult g = gpu_encode({&im}, q, c420, 0, true);
                if (g.rc != EGPU_OK) { pass = false; why += " gpu:" + g.err; continue; }
                std::vector<uint8_t> lj;
                if (!libjpeg_encode_rgb(im, q, c420, lj)) { pass = false; why += " libjpeg-encode"; continue; }

                jpeg_decompress_struct d;
                JerrMgr em;
                em.warnings = 0;
                d.err = jpeg_std_error(&em.pub);
                em.pub.error_exit = jerr_exit;
                em.pub.emit_message = jerr_emit;
                em.pub.output_message = jerr_output;
                if (setjmp(em.jb)) {
                    jpeg_destroy_decompress(&d);
                    pass = false;
                    why += " libjpeg-read";
                    continue;
                }
                jpeg_create_decompress(&d);
                jpeg_mem_src(&d, lj.data(), (unsigned long)lj.size());
                jpeg_read_header(&d, TRUE);
                jvirt_barray_ptr* arr = jpeg_read_coefficients(&d);
                const Layout L = make_layout(im.w, im.h, c420);
                for (int comp = 0; comp < 3; ++comp) {
                    const int rh = real_h(L, comp), rw = real_w(L, comp);
                    for (int r = 0; r < rh; ++r) {
                        // libjpeg allows at most maxaccess (v_samp_factor) rows per access, so access one row.
                        JBLOCKARRAY rows = (*d.mem->access_virt_barray)((j_common_ptr)&d, arr[comp], (JDIMENSION)r, 1, FALSE);
                        for (int col = 0; col < rw; ++col) {
                            const size_t bi = gpu_block(L, comp, r, col);
                            for (int i = 0; i < 64; ++i) {
                                const int diff = std::abs((int)g.coef[0][bi * 64 + i] - (int)rows[0][col][i]);
                                ++total;
                                if (diff <= 1) ++within1;
                                if (diff == 0) ++exact;
                                if (diff > maxdiff) maxdiff = diff;
                            }
                        }
                    }
                }
                jpeg_finish_decompress(&d);
                jpeg_destroy_decompress(&d);
            }
            const double frac = total ? (double)within1 / (double)total : 0.0;
            const bool ok = frac >= 0.99 && total > 0;
            pass = pass && ok;
            report(fmt("2 coefficient agreement %s q%d", c420 ? "4:2:0" : "4:4:4", q).c_str(), ok,
                   fmt("%.4f%% of %zu coefficients within +-1 (%.2f%% exact), max |diff| %d%s",
                       100.0 * frac, total, total ? 100.0 * exact / total : 0.0, maxdiff, why.c_str()));
        }
    }
    return pass;
}

// ---------------------------------------------------------------- edge sizes (tests 3 and 5)

struct SizeCase { int w, h; };
static const std::vector<SizeCase> kEdgeSizes = {
    {1, 1}, {7, 9}, {8, 8}, {9, 8}, {15, 17}, {16, 16}, {17, 17}, {1, 1000}, {1000, 1}, {65500, 1}};

// Encodes the edge sizes in one batch and checks each output decodes with no warnings.
static bool check_edge_outputs(bool c420, int& ok_count, int& total, std::string& detail) {
    std::vector<Rgb> imgs;
    uint32_t seed = 100;
    for (const SizeCase& s : kEdgeSizes) {
        imgs.push_back(make_image(s.w, s.h, (int)(seed % 2), seed));   // gradient or noise
        ++seed;
    }
    std::vector<const Rgb*> ptrs;
    for (const Rgb& im : imgs) ptrs.push_back(&im);
    GpuResult g = gpu_encode(ptrs, 75, c420, 0, false);
    if (g.rc != EGPU_OK) {
        detail = "batch failed: " + g.err;
        return false;
    }
    ok_count = 0;
    total = (int)imgs.size();
    std::string bad;
    for (size_t i = 0; i < imgs.size(); ++i) {
        Rgb out;
        int warn = 0;
        const bool decoded = decode_rgb(g.jpg[i].data(), g.jpg[i].size(), out, warn);
        const bool dims = decoded && out.w == imgs[i].w && out.h == imgs[i].h;
        if (decoded && dims && warn == 0) {
            ++ok_count;
        } else {
            bad += fmt(" %dx%d(warn %d)", imgs[i].w, imgs[i].h, warn);
        }
    }
    detail = fmt("%d of %d edge-size JPEGs decode with 0 warnings and matching size", ok_count, total);
    if (!bad.empty()) detail += "; failed:" + bad;
    return ok_count == total;
}

static bool test3_valid() {
    bool pass = true;
    int ok = 0, total = 0;
    std::string detail;
    for (bool c420 : {true, false}) {
        int o = 0, t = 0;
        std::string d;
        if (!check_edge_outputs(c420, o, t, d)) pass = false;
        ok += o;
        total += t;
        detail += (c420 ? "4:2:0: " : "4:4:4: ") + d + "; ";
    }
    // Sample images at two qualities and both chroma modes.
    int sample_ok = 0, sample_total = 0;
    for (int i = 1; i <= 40; ++i) {
        Rgb s;
        if (!load_sample(i, s)) continue;
        for (bool c420 : {true, false}) {
            for (int q : {50, 90}) {
                GpuResult g = gpu_encode({&s}, q, c420, 0, false);
                ++sample_total;
                Rgb out;
                int warn = 0;
                if (g.rc == EGPU_OK && decode_rgb(g.jpg[0].data(), g.jpg[0].size(), out, warn) &&
                    out.w == s.w && out.h == s.h && warn == 0) {
                    ++sample_ok;
                }
            }
        }
    }
    pass = pass && sample_ok == sample_total && sample_total > 0;
    detail += fmt("sample images: %d of %d decode with 0 warnings (q50/q90, both chroma modes)", sample_ok, sample_total);
    report("3 valid JPEG", pass, detail);
    return pass;
}

static bool test5_edges() {
    bool pass = true;
    std::string detail;
    for (bool c420 : {true, false}) {
        int ok = 0, total = 0;
        std::string d;
        const bool p = check_edge_outputs(c420, ok, total, d);
        pass = pass && p;
        detail += (c420 ? "4:2:0: " : "4:4:4: ") + d + "; ";
    }
    // Invalid descriptors must return errors, not crash.
    struct BadCase { int w, h; uint64_t extra_bytes; int expect; const char* name; };
    const std::vector<BadCase> bad = {
        {0, 5, 0, EGPU_ERR_SIZE, "0 x N"},
        {5, 0, 0, EGPU_ERR_SIZE, "N x 0"},
        {65501, 1, 0, EGPU_ERR_SIZE, "65501 x 1"},
        {1, 65501, 0, EGPU_ERR_SIZE, "1 x 65501"},
    };
    std::string errs;
    for (const BadCase& b : bad) {
        Rgb im;
        im.w = b.w;
        im.h = b.h;
        im.px.assign((size_t)std::max(0, b.w) * std::max(0, b.h) * 3, 0);
        GpuResult g = gpu_encode({&im}, 75, true, 0, false);
        const bool ok = g.rc == b.expect;
        pass = pass && ok;
        errs += fmt(" %s -> %s;", b.name, ok ? "error as expected" : "UNEXPECTED");
    }
    detail += "invalid sizes:" + errs;
    report("5 edge sizes", pass, detail);
    return pass;
}

// ---------------------------------------------------------------- test 4: batch invariance

static bool test4_invariance() {
    std::vector<Rgb> imgs;
    for (int i = 1; i <= 24; ++i) {
        Rgb s;
        if (load_sample(i, s)) imgs.push_back(std::move(s));
    }
    std::mt19937 rng(42);
    for (int i = 0; i < 8; ++i) {
        const int w = 1 + (int)(rng() % 257), h = 1 + (int)(rng() % 257);
        imgs.push_back(make_image(w, h, i % 4, 1000 + i));
    }
    const size_t n = imgs.size();

    bool pass = true;
    std::string detail;
    for (bool c420 : {true, false}) {
        for (int q : {75, 90}) {
            // Reference: every image alone.
            std::vector<std::vector<uint8_t>> alone(n);
            for (size_t i = 0; i < n; ++i) {
                GpuResult g = gpu_encode({&imgs[i]}, q, c420, 0, false);
                if (g.rc != EGPU_OK) { pass = false; continue; }
                alone[i] = g.jpg[0];
            }
            // Variants: one batch, one batch with a tiny pixel budget, shuffled batches of 5.
            std::vector<const Rgb*> all;
            for (const Rgb& im : imgs) all.push_back(&im);
            struct Variant { const char* name; std::vector<std::vector<uint8_t>> out; };
            std::vector<Variant> variants;
            {
                GpuResult g = gpu_encode(all, q, c420, 0, false);
                variants.push_back({"one batch", g.rc == EGPU_OK ? g.jpg : std::vector<std::vector<uint8_t>>(n)});
            }
            {
                GpuResult g = gpu_encode(all, q, c420, 50000, false);
                variants.push_back({"one batch, 50k-pixel budget", g.rc == EGPU_OK ? g.jpg : std::vector<std::vector<uint8_t>>(n)});
            }
            {
                std::vector<size_t> perm(n);
                for (size_t i = 0; i < n; ++i) perm[i] = i;
                std::shuffle(perm.begin(), perm.end(), std::mt19937(7));
                std::vector<std::vector<uint8_t>> out(n);
                for (size_t s = 0; s < n; s += 5) {
                    std::vector<const Rgb*> part;
                    std::vector<size_t> ids;
                    for (size_t k = s; k < std::min(n, s + 5); ++k) {
                        part.push_back(&imgs[perm[k]]);
                        ids.push_back(perm[k]);
                    }
                    GpuResult g = gpu_encode(part, q, c420, 0, false);
                    if (g.rc != EGPU_OK) { pass = false; continue; }
                    for (size_t k = 0; k < ids.size(); ++k) out[ids[k]] = g.jpg[k];
                }
                variants.push_back({"shuffled batches of 5", out});
            }
            for (const Variant& v : variants) {
                size_t same = 0;
                for (size_t i = 0; i < n; ++i) {
                    if (!v.out[i].empty() && v.out[i] == alone[i]) ++same;
                }
                const bool ok = same == n;
                pass = pass && ok;
                detail += fmt("%s %s q%d: %zu/%zu identical to alone; ", c420 ? "4:2:0" : "4:4:4", v.name, q, same, n);
            }
        }
    }
    report("4 batch invariance", pass, detail);
    return pass;
}

// ---------------------------------------------------------------- test 6: quality vs libjpeg-turbo

static bool write_ppm(const std::string& path, const Rgb& im) {
    std::ofstream f(path, std::ios::binary);
    if (!f) return false;
    f << "P6\n" << im.w << " " << im.h << "\n255\n";
    f.write(reinterpret_cast<const char*>(im.px.data()), (std::streamsize)im.px.size());
    return (bool)f;
}

// Runs ssimulacra2 on two PPM files and parses the score from its output.
static bool ssim2(const std::string& ref, const std::string& dis, double& score) {
    const std::string cmd = "\"\"" + g_ssim_exe + "\" \"" + ref + "\" \"" + dis + "\"\"";
    FILE* p = _popen(cmd.c_str(), "r");
    if (!p) return false;
    std::string outp;
    char buf[512];
    while (fgets(buf, sizeof buf, p)) outp += buf;
    _pclose(p);
    std::istringstream is(outp);
    std::string tok;
    bool found = false;
    while (is >> tok) {
        char* end = nullptr;
        const double v = strtod(tok.c_str(), &end);
        if (end != tok.c_str() && *end == '\0') {
            score = v;
            found = true;
        }
    }
    return found;
}

struct QualityRow {
    std::vector<double> psnr_g, psnr_t, ssim_g, ssim_t, ratio;
};

// Puts the PATH back as it was when the scope ends, so a DLL folder added for a test does not outlive it.
struct PathRestore {
    std::string saved;
    bool had = false;
    PathRestore() {
        if (const char* p = std::getenv("PATH")) {
            saved = p;
            had = true;
        }
    }
    ~PathRestore() { _putenv_s("PATH", had ? saved.c_str() : ""); }
    PathRestore(const PathRestore&) = delete;
    PathRestore& operator=(const PathRestore&) = delete;
};

static bool test6_quality() {
    PathRestore restore_path;
    fs::create_directories(g_work_dir);
    if (g_use_ssim) {
        // ssimulacra2.exe loads jpegli_cms.dll from ..\lib relative to its tools folder.
        const std::string dll_dir = (fs::path(g_ssim_exe).parent_path().parent_path() / "lib").string();
        const char* old_path = std::getenv("PATH");
        _putenv_s("PATH", (dll_dir + ";" + (old_path ? old_path : "")).c_str());
    }
    const int N = 300, BATCH = 50;
    struct Setting { bool c420; int q; };
    const std::vector<Setting> settings = {{true, 50}, {true, 75}, {true, 90}, {false, 50}, {false, 75}, {false, 90}};
    std::vector<QualityRow> rows(settings.size());
    int failed_loads = 0;
    const std::string ssim_ref_dir = g_work_dir;

    for (int b0 = 1; b0 <= N; b0 += BATCH) {
        std::vector<Rgb> src;
        for (int i = b0; i < b0 + BATCH && i <= N; ++i) {
            Rgb s;
            if (load_sample(i, s)) src.push_back(std::move(s));
            else ++failed_loads;
        }
        std::vector<const Rgb*> ptrs;
        for (const Rgb& s : src) ptrs.push_back(&s);
        for (size_t si = 0; si < settings.size(); ++si) {
            const Setting& st = settings[si];
            GpuResult g = gpu_encode(ptrs, st.q, st.c420, 0, false);
            if (g.rc != EGPU_OK) {
                printf("gpu batch failed: %s\n", g.err.c_str());
                continue;
            }
            for (size_t i = 0; i < src.size(); ++i) {
                const std::vector<uint8_t> tj = turbo_encode(src[i], st.q, st.c420);
                Rgb dg, dt;
                int wg = 0, wt = 0;
                if (!decode_rgb(g.jpg[i].data(), g.jpg[i].size(), dg, wg) ||
                    !decode_rgb(tj.data(), tj.size(), dt, wt)) {
                    printf("decode failed for image %d\n", b0 + (int)i);
                    ++failed_loads;  // a decode failure fails the test, as the other failures here do
                    continue;
                }
                QualityRow& row = rows[si];
                row.psnr_g.push_back(psnr(src[i], dg));
                row.psnr_t.push_back(psnr(src[i], dt));
                row.ratio.push_back((double)g.jpg[i].size() / (double)tj.size());
                if (g_use_ssim) {
                    const std::string base = fmt("%s/s%zu_%d", ssim_ref_dir.c_str(), si, b0 + (int)i);
                    write_ppm(base + "_src.ppm", src[i]);
                    write_ppm(base + "_g.ppm", dg);
                    write_ppm(base + "_t.ppm", dt);
                    double sg = 0, st2 = 0;
                    if (ssim2(base + "_src.ppm", base + "_g.ppm", sg)) row.ssim_g.push_back(sg);
                    if (ssim2(base + "_src.ppm", base + "_t.ppm", st2)) row.ssim_t.push_back(st2);
                    fs::remove(base + "_g.ppm");
                    fs::remove(base + "_t.ppm");
                    fs::remove(base + "_src.ppm");
                }
            }
        }
        printf("  ... images %d-%d done\n", b0, std::min(N, b0 + BATCH - 1));
        fflush(stdout);
    }

    bool pass = failed_loads == 0;
    for (size_t si = 0; si < settings.size(); ++si) {
        const Setting& st = settings[si];
        const QualityRow& r = rows[si];
        const double mg = median(r.psnr_g), mt = median(r.psnr_t);
        const double med_ratio = median(r.ratio);
        const bool ratio_ok = med_ratio >= 0.97 && med_ratio <= 1.03;
        // One-sided: the GPU may not fall more than 0.5 dB below turbo. A higher PSNR is reported, not failed.
        const bool psnr_ok = mg - mt >= -0.5;
        const bool ok = ratio_ok && psnr_ok;
        pass = pass && ok;
        std::string d = fmt("n=%zu  PSNR gpu med %.2f p5 %.2f | turbo med %.2f p5 %.2f (dB)",
                            r.psnr_g.size(), mg, percentile(r.psnr_g, 0.05), mt, percentile(r.psnr_t, 0.05));
        if (g_use_ssim) {
            d += fmt("\n       SSIMULACRA2 gpu med %.2f p5 %.2f | turbo med %.2f p5 %.2f (n=%zu/%zu)",
                     median(r.ssim_g), percentile(r.ssim_g, 0.05), median(r.ssim_t), percentile(r.ssim_t, 0.05),
                     r.ssim_g.size(), r.ssim_t.size());
        }
        d += fmt("\n       size ratio (gpu/turbo) median %.4f  [criterion 0.97..1.03: %s]; PSNR diff %.2f dB [criterion >= -0.5 dB: %s]",
                 med_ratio, ratio_ok ? "met" : "not met", mg - mt, psnr_ok ? "met" : "not met");
        report(fmt("6 quality %s q%d", st.c420 ? "4:2:0" : "4:4:4", st.q).c_str(), ok, d);
    }
    return pass;
}

// ---------------------------------------------------------------- test 7: throughput

static bool test7_throughput() {
    std::vector<Rgb> src;
    for (int i = 1; i <= 128; ++i) {
        Rgb s;
        if (load_sample(i, s)) src.push_back(std::move(s));
    }
    if (src.size() < 128) {
        report("7 throughput", false, "could not load 128 sample images");
        return false;
    }
    std::string detail;
    for (size_t batch : {64u, 128u}) {
        EgpuOptions o;
        o.quality = 75;
        o.chroma_420 = 1;
        o.device = 0;
        o.pixel_budget = 0;
        o.keep_coefficients = 0;
        EgpuContext* ctx = nullptr;
        if (egpu_create(&o, &ctx) != EGPU_OK) {
            report("7 throughput", false, egpu_last_error());
            return false;
        }
        std::vector<EgpuImage> desc(batch);
        std::vector<uint8_t> host;
        for (size_t i = 0; i < batch; ++i) {
            desc[i].offset = host.size();
            desc[i].width = (uint32_t)src[i].w;
            desc[i].height = (uint32_t)src[i].h;
            host.insert(host.end(), src[i].px.begin(), src[i].px.end());
        }
        uint8_t* d = nullptr;
        cuda_ok(cudaMalloc((void**)&d, host.size()), "cudaMalloc");
        cuda_ok(cudaMemcpy(d, host.data(), host.size(), cudaMemcpyHostToDevice), "cudaMemcpy");

        // Warm-up, then timed repetitions. Input stays on the device, as in the pipeline.
        for (int w = 0; w < 3; ++w) egpu_encode_batch(ctx, d, host.size(), desc.data(), (uint32_t)batch);
        std::vector<double> ms;
        EgpuTimings acc{};
        const int reps = 20;
        for (int r = 0; r < reps; ++r) {
            const auto t0 = std::chrono::steady_clock::now();
            const int rc = egpu_encode_batch(ctx, d, host.size(), desc.data(), (uint32_t)batch);
            const auto t1 = std::chrono::steady_clock::now();
            if (rc != EGPU_OK) break;
            ms.push_back(std::chrono::duration<double, std::milli>(t1 - t0).count());
            EgpuTimings t;
            egpu_timings(ctx, &t);
            acc.colour_ms += t.colour_ms; acc.dct_ms += t.dct_ms; acc.sizes_ms += t.sizes_ms;
            acc.scan_ms += t.scan_ms; acc.pack_ms += t.pack_ms; acc.pad_ms += t.pad_ms;
            acc.stuff_ms += t.stuff_ms; acc.copy_ms += t.copy_ms; acc.total_ms += t.total_ms;
        }
        const double med_ms = median(ms);
        const double ips = med_ms > 0 ? (double)batch / (med_ms / 1000.0) : 0.0;
        const double k = ms.empty() ? 1.0 : (double)ms.size();
        detail += fmt("\n       batch %zu (4:2:0 q75, %zu reps): median %.2f ms per batch, %.1f images/s; "
                       "stages (mean ms): colour %.3f dct %.3f sizes %.3f scan %.3f pack %.3f pad %.3f stuff %.3f copy %.3f",
                       batch, ms.size(), med_ms, ips, acc.colour_ms / k, acc.dct_ms / k, acc.sizes_ms / k,
                       acc.scan_ms / k, acc.pack_ms / k, acc.pad_ms / k, acc.stuff_ms / k, acc.copy_ms / k);
        cudaFree(d);
        egpu_destroy(ctx);
    }
    report("7 throughput", true, detail.substr(8));   // measurement only
    return true;
}

// ---------------------------------------------------------------- main

// ---------------------------------------------------------------- diagnostic 8: DCT method at q90
// Not a pre-registered test. It explains the q90 PSNR gap in test 6: the same images at q90 through libjpeg's three
// DCT methods (integer ISLOW, the default that turbo_encode uses; float; fast integer IFAST), and the GPU encoder.
// Prints median PSNR against the source. Run it on request with --tests 8.

// A CUDA call whose failure ends the run: the harness has no result to report without it.
static void cuda_ok(cudaError_t e, const char* what) {
    if (e != cudaSuccess) {
        fprintf(stderr, "%s failed: %s\n", what, cudaGetErrorString(e));
        exit(1);
    }
}

// libjpeg's encode with the DCT method given. A libjpeg error gives an empty result, which the caller's score() skips as a
// failed decode is skipped; the error manager longjmps as libjpeg_encode_rgb's does (libjpeg's default one exits).
static std::vector<uint8_t> libjpeg_encode(const Rgb& im, int q, bool c420, J_DCT_METHOD dct) {
    jpeg_compress_struct c;
    JerrMgr em;
    em.warnings = 0;
    c.err = jpeg_std_error(&em.pub);
    em.pub.error_exit = jerr_exit;
    em.pub.emit_message = jerr_emit;
    em.pub.output_message = jerr_output;
    unsigned char* buf = nullptr;
    unsigned long sz = 0;
    if (setjmp(em.jb)) {
        jpeg_destroy_compress(&c);
        free(buf);
        return {};
    }
    jpeg_create_compress(&c);
    jpeg_mem_dest(&c, &buf, &sz);
    c.image_width = (JDIMENSION)im.w;
    c.image_height = (JDIMENSION)im.h;
    c.input_components = 3;
    c.in_color_space = JCS_RGB;
    jpeg_set_defaults(&c);
    jpeg_set_quality(&c, q, TRUE);
    c.dct_method = dct;
    c.comp_info[0].h_samp_factor = c420 ? 2 : 1;
    c.comp_info[0].v_samp_factor = c420 ? 2 : 1;
    jpeg_start_compress(&c, TRUE);
    for (int y = 0; y < im.h; ++y) {
        JSAMPROW row = const_cast<JSAMPROW>(im.px.data() + (size_t)y * im.w * 3);
        jpeg_write_scanlines(&c, &row, 1);
    }
    jpeg_finish_compress(&c);
    std::vector<uint8_t> out(buf, buf + sz);
    free(buf);
    jpeg_destroy_compress(&c);
    return out;
}

static void test8_float_dct_diag() {
    const int N = 300, BATCH = 50;
    struct Setting { bool c420; int q; };
    const std::vector<Setting> settings = {{true, 90}, {false, 90}};
    for (const Setting& st : settings) {
        std::vector<double> pg, pislow, pfloat, pifast;
        for (int b0 = 1; b0 <= N; b0 += BATCH) {
            std::vector<Rgb> src;
            for (int i = b0; i < b0 + BATCH && i <= N; ++i) {
                Rgb s;
                if (load_sample(i, s)) src.push_back(std::move(s));
            }
            std::vector<const Rgb*> ptrs;
            for (const Rgb& s : src) ptrs.push_back(&s);
            GpuResult g = gpu_encode(ptrs, st.q, st.c420, 0, false);
            if (g.rc != EGPU_OK) {
                printf("gpu batch failed: %s\n", g.err.c_str());
                return;
            }
            for (size_t i = 0; i < src.size(); ++i) {
                Rgb d;
                int w = 0;
                auto score = [&](const std::vector<uint8_t>& j, std::vector<double>& out) {
                    if (decode_rgb(j.data(), j.size(), d, w)) out.push_back(psnr(src[i], d));
                };
                score(g.jpg[i], pg);
                score(turbo_encode(src[i], st.q, st.c420), pislow);
                score(libjpeg_encode(src[i], st.q, st.c420, JDCT_FLOAT), pfloat);
                score(libjpeg_encode(src[i], st.q, st.c420, JDCT_IFAST), pifast);
            }
        }
        printf("8 diag %s q%d (n=%zu): median PSNR  gpu %.2f | ISLOW %.2f | FLOAT %.2f | IFAST %.2f (dB)\n",
               st.c420 ? "4:2:0" : "4:4:4", st.q, pg.size(), median(pg), median(pislow), median(pfloat),
               median(pifast));
        fflush(stdout);
    }
}

int main(int argc, char** argv) {
    std::set<int> which;
    for (int i = 1; i < argc; ++i) {
        const std::string a = argv[i];
        if (a == "--sample" && i + 1 < argc) g_sample_dir = argv[++i];
        else if (a == "--work" && i + 1 < argc) g_work_dir = argv[++i];
        else if (a == "--ssim" && i + 1 < argc) g_ssim_exe = argv[++i];
        else if (a == "--no-ssim") g_use_ssim = false;
        else if (a == "--tests" && i + 1 < argc) {
            std::stringstream ss(argv[++i]);
            std::string item;
            while (std::getline(ss, item, ',')) which.insert(std::atoi(item.c_str()));
        } else {
            printf("unknown argument: %s\n", a.c_str());
            return 2;
        }
    }
    if (which.empty()) which = {1, 2, 3, 4, 5, 6, 7};

    int dev = 0;
    cudaDeviceProp prop{};
    if (cudaGetDevice(&dev) == cudaSuccess && cudaGetDeviceProperties(&prop, dev) == cudaSuccess) {
        printf("GPU: %s (sm_%d%d)\n", prop.name, prop.major, prop.minor);
    } else {
        printf("no CUDA device available\n");
        return 2;
    }

    if (which.count(1)) test1_entropy();
    if (which.count(2)) test2_coefficients();
    if (which.count(3)) test3_valid();
    if (which.count(4)) test4_invariance();
    if (which.count(5)) test5_edges();
    if (which.count(6)) test6_quality();
    if (which.count(7)) test7_throughput();
    if (which.count(8)) test8_float_dct_diag();

    printf("\n%d failure(s)\n", g_failures);
    return g_failures ? 1 : 0;
}
