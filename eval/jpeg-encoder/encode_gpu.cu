// encode_gpu.cu - batched baseline JPEG encoder on CUDA. See encode_gpu.h.
//
// Pipeline per sub-batch (one launch set per stage for the whole sub-batch):
//   1. colour  : pad by edge replication, RGB -> YCbCr (JFIF), 4:2:0 2x2 downsampling
//   2. dct     : level shift, float separable 8x8 DCT, quantize -> int16 natural order.
//                Blocks wholly outside the image's block grid ("dummy" blocks) are all-zero
//                AC with DC equal to the nearest preceding real block's DC of the same
//                component (k_dummy_dc), as libjpeg does when transcoding coefficients.
//   3. sizes   : per-block bit count (DC diff, AC run/size, ZRL, EOB)
//   4. scan    : exclusive prefix sum of block bit counts
//   5. pack    : zero word buffers, Huffman-code every block, atomicOr bit packing
//   6. pad     : fill each image's final byte with 1-bits; count 0xFF bytes
//   7. stuff   : host headers, stuffed entropy bytes, EOI
// All block offsets are relative to their image's first block, and every image
// starts on a fresh word region, so an image's bytes do not depend on the batch.

#include "encode_gpu.h"

#include <cuda_runtime.h>
#include <thrust/device_ptr.h>
#include <thrust/execution_policy.h>
#include <thrust/scan.h>

#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <new>
#include <vector>

namespace {

// ---------------------------------------------------------------- errors

thread_local char g_error[320] = "ok";

void set_error(const char* msg) { snprintf(g_error, sizeof g_error, "%s", msg); }

void set_cuda_error(const char* what, cudaError_t e) {
    snprintf(g_error, sizeof g_error, "%s failed: %s", what, cudaGetErrorString(e));
    cudaGetLastError();  // clear the sticky-free error state so later calls start clean
}

#define CUDA_TRY(expr)                                               \
    do {                                                             \
        cudaError_t e_ = (expr);                                     \
        if (e_ != cudaSuccess) {                                     \
            set_cuda_error(#expr, e_);                               \
            return EGPU_ERR_CUDA;                                    \
        }                                                            \
    } while (0)

// ---------------------------------------------------------------- Annex K tables
// Copied from libjpeg-turbo 3.1.0 (jcparam.c std_*_quant_tbl, jstdhuff.c std_huff_tables,
// jutils.c jpeg_natural_order). Quant tables are in natural (row-major) order.

const unsigned kStdLumQ[64] = {
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55,
    14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62,
    18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92,
    49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99};

const unsigned kStdChrQ[64] = {
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99,
    24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99};

// Zigzag position k -> natural index.
const uint8_t kZigzag[64] = {
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5,
    12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63};

// Counts of codes per length 1..16, then the symbol values in order.
const uint8_t kDcLumCounts[16] = {0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0};
const uint8_t kDcLumVals[12] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
const uint8_t kDcChrCounts[16] = {0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0};
const uint8_t kDcChrVals[12] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
const uint8_t kAcLumCounts[16] = {0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d};
const uint8_t kAcLumVals[162] = {
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa};
const uint8_t kAcChrCounts[16] = {0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77};
const uint8_t kAcChrVals[162] = {
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa};

// Huffman table indices into c_hcode / c_hlen.
enum { HT_DC_LUM = 0, HT_AC_LUM = 1, HT_DC_CHR = 2, HT_AC_CHR = 3 };

// ---------------------------------------------------------------- device constants

__constant__ uint32_t c_hcode[4][256];
__constant__ uint32_t c_hlen[4][256];
__constant__ uint8_t c_zz[64];      // zigzag k -> natural index
__constant__ float c_q[2][64];      // [0] luma, [1] chroma, natural order
__constant__ float c_cos[64];       // c_cos[u*8+x] = cos((2x+1) u pi / 16)

constexpr float kInvSqrt2 = 0.70710678118654752f;
constexpr uint32_t kNone = 0xFFFFFFFFu;
constexpr unsigned kTpb = 256;

// Per-image geometry, uploaded for each sub-batch.
struct ImgInfo {
    uint64_t rgb_off;   // byte offset of the first pixel in the device RGB buffer
    uint32_t w, h;      // image size
    uint32_t pw, ph;    // padded luma size (whole MCUs)
    uint32_t cw, ch;    // chroma plane size (equal to pw, ph for 4:4:4)
    uint32_t mcus_x, mcus_y;
    uint32_t pad;
};

// Largest index i in [0, n) with base[i] <= x. base has n+1 strictly increasing entries.
__device__ __forceinline__ uint32_t find_index(const uint64_t* base, uint32_t n, uint64_t x) {
    uint32_t lo = 0, hi = n;
    while (hi - lo > 1) {
        const uint32_t mid = (lo + hi) >> 1;
        if (base[mid] <= x) lo = mid; else hi = mid;
    }
    return lo;
}

// ---------------------------------------------------------------- stage 1: colour

__device__ __forceinline__ void rgb_at(const uint8_t* rgb, const ImgInfo& m, uint32_t x, uint32_t y,
                                       float& R, float& G, float& B) {
    const uint32_t xx = x < m.w ? x : m.w - 1;   // edge replication to the padded grid
    const uint32_t yy = y < m.h ? y : m.h - 1;
    const uint8_t* p = rgb + m.rgb_off + ((uint64_t)yy * m.w + xx) * 3;
    R = (float)p[0];
    G = (float)p[1];
    B = (float)p[2];
}

__device__ __forceinline__ uint8_t clamp_u8(float v) {
    v = roundf(v);
    if (v < 0.f) v = 0.f;
    if (v > 255.f) v = 255.f;
    return (uint8_t)v;
}

// comp 0 = Cb, 1 = Cr (JFIF, full range).
__device__ __forceinline__ uint8_t chroma_u8(uint32_t comp, float R, float G, float B) {
    if (comp == 0) return clamp_u8(-0.168736f * R - 0.331264f * G + 0.5f * B + 128.f);
    return clamp_u8(0.5f * R - 0.418688f * G - 0.081312f * B + 128.f);
}

// One thread per plane sample across all images. Plane layout per image: Y, then Cb, then Cr.
__global__ void k_colour(const uint8_t* __restrict__ rgb, const ImgInfo* info, const uint64_t* pbase,
                         uint32_t n, uint8_t* planes, uint64_t total, int c420) {
    const uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    for (uint64_t idx = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x; idx < total; idx += stride) {
        const uint32_t i = find_index(pbase, n, idx);
        const ImgInfo m = info[i];
        const uint64_t o = idx - pbase[i];
        const uint64_t ysz = (uint64_t)m.pw * m.ph;
        if (o < ysz) {
            float R, G, B;
            rgb_at(rgb, m, (uint32_t)(o % m.pw), (uint32_t)(o / m.pw), R, G, B);
            planes[idx] = clamp_u8(0.299f * R + 0.587f * G + 0.114f * B);
            continue;
        }
        const uint64_t csz = (uint64_t)m.cw * m.ch;
        const uint64_t o2 = o - ysz;
        const uint32_t comp = (uint32_t)(o2 / csz);
        const uint64_t p = o2 - comp * csz;
        const uint32_t cx = (uint32_t)(p % m.cw), cy = (uint32_t)(p / m.cw);
        if (!c420) {
            float R, G, B;
            rgb_at(rgb, m, cx, cy, R, G, B);
            planes[idx] = chroma_u8(comp, R, G, B);
        } else {
            // libjpeg h2v2_downsample: each Cb/Cr sample is 8-bit before averaging,
            // sum + bias (1,2,1,2,... across the row), shift right by 2.
            uint32_t sum = 0;
            for (uint32_t dy = 0; dy < 2; ++dy) {
                for (uint32_t dx = 0; dx < 2; ++dx) {
                    float R, G, B;
                    rgb_at(rgb, m, 2 * cx + dx, 2 * cy + dy, R, G, B);
                    sum += chroma_u8(comp, R, G, B);
                }
            }
            sum += 1u + (cx & 1u);
            planes[idx] = (uint8_t)(sum >> 2);
        }
    }
}

// ---------------------------------------------------------------- stage 2: DCT + quantize

// True when the block lies within the image's block grid for its component. Blocks outside it
// (the padding at the right and bottom edges of partial MCUs) are dummies, as in libjpeg.
__device__ __forceinline__ bool block_is_real(const ImgInfo& m, uint32_t comp, uint32_t mcu, uint32_t k,
                                              int c420) {
    const uint32_t mx = mcu % m.mcus_x, my = mcu / m.mcus_x;
    const bool luma420 = c420 && comp == 0;
    const uint32_t gcol = luma420 ? mx * 2 + (k & 1u) : mx;
    const uint32_t grow = luma420 ? my * 2 + (k >> 1) : my;
    const uint32_t maxh = c420 ? 2u : 1u;
    const uint32_t hsc = luma420 ? 2u : 1u;
    const uint32_t rw = ((m.w * hsc + maxh - 1) / maxh + 7) / 8;
    const uint32_t rh = ((m.h * hsc + maxh - 1) / maxh + 7) / 8;
    return gcol < rw && grow < rh;
}

// One CUDA block (64 threads) per 8x8 block, in the chunk's global block order.
__global__ void k_dct(const uint8_t* __restrict__ planes, const ImgInfo* info, const uint64_t* pbase,
                      const uint64_t* bbase, uint32_t n, int16_t* coef, int c420) {
    __shared__ float ct[64];
    __shared__ float s[64];
    __shared__ float tmp[64];

    const uint64_t g = blockIdx.x;
    const int t = threadIdx.x;
    ct[t] = c_cos[t];

    const uint32_t i = find_index(bbase, n, g);
    const ImgInfo m = info[i];
    const uint64_t local = g - bbase[i];
    const uint32_t bpm = c420 ? 6u : 3u;
    const uint32_t mcu = (uint32_t)(local / bpm);
    const uint32_t k = (uint32_t)(local % bpm);
    const uint32_t mx = mcu % m.mcus_x, my = mcu / m.mcus_x;
    const uint64_t ysz = (uint64_t)m.pw * m.ph, csz = (uint64_t)m.cw * m.ch;
    const uint8_t* base = planes + pbase[i];

    const uint8_t* src;
    uint32_t stride, x0, y0, comp;
    if (!c420) {
        comp = k;
        src = base + comp * ysz;
        stride = m.pw;
        x0 = mx * 8;
        y0 = my * 8;
    } else if (k < 4) {
        comp = 0;
        src = base;
        stride = m.pw;
        x0 = mx * 16 + (k & 1u) * 8;
        y0 = my * 16 + (k >> 1) * 8;
    } else {
        comp = k - 3;
        src = base + ysz + (uint64_t)(comp - 1) * csz;
        stride = m.cw;
        x0 = mx * 8;
        y0 = my * 8;
    }

    // Dummy blocks: all AC zero. Their DC is filled in by k_dummy_dc, once the real DCs exist.
    if (!block_is_real(m, comp, mcu, k, c420)) {
        coef[g * 64 + t] = 0;
        return;
    }

    s[t] = (float)src[(uint64_t)(y0 + (t >> 3)) * stride + x0 + (t & 7)] - 128.0f;
    __syncthreads();

    // Row pass: tmp[y][u] = sum_x s[y][x] cos((2x+1) u pi/16)
    {
        const int y = t >> 3, u = t & 7;
        float a = 0.f;
        for (int x = 0; x < 8; ++x) a += s[y * 8 + x] * ct[u * 8 + x];
        tmp[y * 8 + u] = a;
    }
    __syncthreads();

    // Column pass and scaling: F(u,v) = 1/4 C(u) C(v) sum_y tmp[y][u] cos((2y+1) v pi/16)
    {
        const int v = t >> 3, u = t & 7;
        float a = 0.f;
        for (int y = 0; y < 8; ++y) a += tmp[y * 8 + u] * ct[v * 8 + y];
        const float F = 0.25f * (u == 0 ? kInvSqrt2 : 1.f) * (v == 0 ? kInvSqrt2 : 1.f) * a;
        const int nat = v * 8 + u;
        float q = roundf(F / c_q[comp ? 1 : 0][nat]);   // half away from zero
        if (q < -32768.f) q = -32768.f;
        if (q > 32767.f) q = 32767.f;
        coef[g * 64 + nat] = (int16_t)q;
    }
}

// ---------------------------------------------------------------- stages 3 and 5: entropy

__device__ __forceinline__ uint32_t nbits_of(uint32_t v) { return v ? 32u - __clz(v) : 0u; }

__device__ __forceinline__ uint32_t comp_of(uint32_t k, int c420) {
    return c420 ? (k < 4u ? 0u : k - 3u) : k;
}

// Local index of the previous block of the same component in scan order, or kNone.
__device__ __forceinline__ uint32_t prev_same_comp(uint32_t mcu, uint32_t k, int c420) {
    const uint32_t bpm = c420 ? 6u : 3u;
    if (c420 && k < 4u && k > 0u) return mcu * bpm + k - 1u;
    if (mcu == 0u) return kNone;
    if (c420 && k < 4u) return (mcu - 1u) * bpm + 3u;
    return (mcu - 1u) * bpm + k;
}

// ORs the value's bits into the word buffer at word wbase + bitpos/32, MSB first.
// Writes span at most two words; each write is a single atomicOr.
__device__ __forceinline__ void put_bits(uint32_t* words, uint64_t wbase, uint64_t bitpos,
                                         uint32_t value, uint32_t nbits) {
    const uint64_t w = wbase + (bitpos >> 5);
    const uint32_t o = (uint32_t)(bitpos & 31u);
    const uint64_t big = ((uint64_t)value) << (64u - o - nbits);
    const uint32_t hi = (uint32_t)(big >> 32), lo = (uint32_t)big;
    if (hi) atomicOr(&words[w], hi);
    if (lo) atomicOr(&words[w + 1], lo);
}

// Emits Huffman code sym from table tbl, followed by s magnitude bits of mag.
template <bool W>
__device__ __forceinline__ void put_symbol(uint32_t* words, uint64_t wbase, uint64_t& pos, int tbl,
                                           uint32_t sym, int mag, uint32_t s) {
    const uint32_t len = c_hlen[tbl][sym];
    const uint32_t code = c_hcode[tbl][sym];
    const uint32_t mb = mag < 0 ? (uint32_t)(mag + (1 << s) - 1) : (uint32_t)mag;
    const uint32_t n = len + s;
    if (W && n) put_bits(words, wbase, pos, (code << s) | mb, n);
    pos += n;
}

// Codes one block in scan order. W = false only counts bits; W = true also writes them.
template <bool W>
__device__ void encode_block(const int16_t* blk, uint32_t comp, int has_pred, int pred,
                             uint32_t* words, uint64_t wbase, uint64_t& pos) {
    const int dct = comp ? HT_DC_CHR : HT_DC_LUM;
    const int act = comp ? HT_AC_CHR : HT_AC_LUM;

    const int diff = (int)blk[0] - (has_pred ? pred : 0);
    const uint32_t ad = (uint32_t)(diff < 0 ? -diff : diff);
    const uint32_t s = nbits_of(ad);
    put_symbol<W>(words, wbase, pos, dct, s, diff, s);

    int run = 0;
    for (int k = 1; k < 64; ++k) {
        int v = (int)blk[c_zz[k]];
        if (v == 0) {
            ++run;
            continue;
        }
        while (run > 15) {
            put_symbol<W>(words, wbase, pos, act, 0xF0u, 0, 0);
            run -= 16;
        }
        if (v > 1023) v = 1023;
        if (v < -1023) v = -1023;
        const uint32_t av = (uint32_t)(v < 0 ? -v : v);
        const uint32_t sz = nbits_of(av);
        put_symbol<W>(words, wbase, pos, act, ((uint32_t)run << 4) | sz, v, sz);
        run = 0;
    }
    if (run > 0) put_symbol<W>(words, wbase, pos, act, 0x00u, 0, 0);   // EOB
}

// Dummy blocks take the DC of the nearest preceding real block of the same component in scan
// order (libjpeg's convention), so their DC difference is zero. Reads only real blocks' DCs.
__global__ void k_dummy_dc(int16_t* coef, const ImgInfo* info, const uint64_t* bbase, uint32_t n,
                           uint64_t nblocks, int c420) {
    const uint64_t g = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    if (g >= nblocks) return;
    const uint32_t i = find_index(bbase, n, g);
    const ImgInfo& m = info[i];
    const uint64_t local = g - bbase[i];
    const uint32_t bpm = c420 ? 6u : 3u;
    uint32_t mcu = (uint32_t)(local / bpm);
    uint32_t k = (uint32_t)(local % bpm);
    if (block_is_real(m, comp_of(k, c420), mcu, k, c420)) return;
    int dc = 0;
    for (;;) {
        const uint32_t p = prev_same_comp(mcu, k, c420);
        if (p == kNone) break;
        mcu = p / bpm;
        k = p % bpm;
        if (block_is_real(m, comp_of(k, c420), mcu, k, c420)) {
            dc = (int)coef[(bbase[i] + p) * 64];
            break;
        }
    }
    coef[g * 64] = (int16_t)dc;
}

__global__ void k_sizes(const int16_t* coef, const uint64_t* bbase, uint32_t n, uint64_t* bits,
                        uint64_t nblocks, int c420) {
    const uint64_t g = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    if (g >= nblocks) return;
    const uint32_t i = find_index(bbase, n, g);
    const uint64_t local = g - bbase[i];
    const uint32_t bpm = c420 ? 6u : 3u;
    const uint32_t mcu = (uint32_t)(local / bpm), k = (uint32_t)(local % bpm);
    const uint32_t comp = comp_of(k, c420);
    const uint32_t prev = prev_same_comp(mcu, k, c420);
    const int has = prev != kNone;
    const int pred = has ? (int)coef[(bbase[i] + prev) * 64] : 0;
    uint64_t pos = 0;
    encode_block<false>(coef + g * 64, comp, has, pred, nullptr, 0, pos);
    bits[g] = pos;
}

__global__ void k_pack(const int16_t* coef, const uint64_t* bbase, uint32_t n, const uint64_t* off,
                       const uint64_t* wbase, uint32_t* words, uint64_t nblocks, int c420) {
    const uint64_t g = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    if (g >= nblocks) return;
    const uint32_t i = find_index(bbase, n, g);
    const uint64_t local = g - bbase[i];
    const uint32_t bpm = c420 ? 6u : 3u;
    const uint32_t mcu = (uint32_t)(local / bpm), k = (uint32_t)(local % bpm);
    const uint32_t comp = comp_of(k, c420);
    const uint32_t prev = prev_same_comp(mcu, k, c420);
    const int has = prev != kNone;
    const int pred = has ? (int)coef[(bbase[i] + prev) * 64] : 0;
    // Bit offset relative to the image's first block, so the image's bytes are batch-independent.
    uint64_t pos = off[g] - off[bbase[i]];
    encode_block<true>(coef + g * 64, comp, has, pred, words, wbase[i], pos);
}

// Fills the padding bits of each image's last byte with 1s.
__global__ void k_pad(uint32_t* words, const uint64_t* wbase, const uint64_t* ibits, uint32_t n) {
    const uint32_t i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    const uint64_t bits = ibits[i];
    const uint64_t end = (bits + 7u) & ~7ull;
    for (uint64_t p = bits; p < end; ++p) {
        atomicOr(&words[wbase[i] + (p >> 5)], 1u << (31u - (uint32_t)(p & 31u)));
    }
}

// FF bytes in each word (bytes beyond an image's data are zero, so they never count).
__global__ void k_wcount(const uint32_t* words, uint32_t* wff, uint64_t W) {
    const uint64_t w = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    if (w >= W) return;
    const uint32_t v = words[w];
    uint32_t c = 0;
    c += (v >> 24) == 0xFFu;
    c += ((v >> 16) & 0xFFu) == 0xFFu;
    c += ((v >> 8) & 0xFFu) == 0xFFu;
    c += (v & 0xFFu) == 0xFFu;
    wff[w] = c;
}

__global__ void k_gather64(const uint64_t* src, const uint64_t* idx, uint64_t cnt, uint64_t* dst) {
    const uint64_t i = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;  // 64-bit, like the counts and tables it indexes
    if (i < cnt) dst[i] = src[idx[i]];
}

// Writes stuffed entropy bytes. One thread per 32-bit word; positions come from the
// exclusive prefix sum of per-word FF counts, so no per-byte scan is needed.
__global__ void k_stuff(const uint32_t* words, const uint64_t* wpref, const uint64_t* wbase, uint32_t n,
                        const uint64_t* nbytes, const uint64_t* outent, uint64_t W, uint8_t* out) {
    const uint64_t w = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    if (w >= W) return;
    const uint32_t i = find_index(wbase, n, w);
    const uint64_t k = w - wbase[i];
    const uint64_t fbefore = wpref[w] - wpref[wbase[i]];
    uint64_t dst = outent[i] + 4ull * k + fbefore;
    const uint32_t v = words[w];
    for (uint32_t t = 0; t < 4; ++t) {
        const uint64_t j = 4ull * k + t;
        if (j >= nbytes[i]) break;
        const uint8_t b = (uint8_t)(v >> (24 - 8 * t));
        out[dst] = b;
        if (b == 0xFF) {
            out[dst + 1] = 0x00;
            dst += 2;
        } else {
            dst += 1;
        }
    }
}

__global__ void k_eoi(uint8_t* out, const uint64_t* eoi, uint32_t n) {
    const uint32_t i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    out[eoi[i]] = 0xFF;
    out[eoi[i] + 1] = 0xD9;
}

// ---------------------------------------------------------------- host helpers

struct Buf {
    void* p = nullptr;
    size_t cap = 0;
};

int ensure(Buf& b, size_t bytes) {
    if (bytes <= b.cap) return EGPU_OK;
    if (b.p) {
        cudaFree(b.p);
        b.p = nullptr;
        b.cap = 0;
    }
    if (bytes == 0) bytes = 1;
    cudaError_t e = cudaMalloc(&b.p, bytes);
    if (e != cudaSuccess) {
        set_cuda_error("cudaMalloc", e);
        return EGPU_ERR_MEMORY;
    }
    b.cap = bytes;
    return EGPU_OK;
}

#define ALLOC(buf, bytes)                       \
    do {                                        \
        int rc_ = ensure((buf), (bytes));       \
        if (rc_) return rc_;                    \
    } while (0)

template <class T>
T* dev(const Buf& b) {
    return static_cast<T*>(b.p);
}

unsigned grid_exact(uint64_t items) {
    uint64_t g = (items + kTpb - 1) / kTpb;
    return g ? (unsigned)g : 1u;
}

unsigned grid_colour(uint64_t items) {
    uint64_t g = (items + kTpb - 1) / kTpb;
    if (g > (1u << 22)) g = 1u << 22;   // grid-stride loop covers the rest
    return g ? (unsigned)g : 1u;
}

enum Stage { ST_COLOUR, ST_DCT, ST_SIZES, ST_SCAN, ST_PACK, ST_PAD, ST_STUFF, ST_COPY, ST_COUNT };

struct Span {
    int stage;
    cudaEvent_t a, b;
};

// Timing events only; a failure to create them leaves the stage time at zero rather than failing the batch.
int open_span(std::vector<Span>& v, int stage) {
    Span s{stage, nullptr, nullptr};
    cudaEventCreate(&s.a);
    cudaEventCreate(&s.b);
    cudaEventRecord(s.a);
    v.push_back(s);
    return (int)v.size() - 1;
}

cudaError_t close_span(std::vector<Span>& v, int idx) { return cudaEventRecord(v[idx].b); }

void destroy_spans(std::vector<Span>& v) {
    for (Span& s : v) {
        if (s.a) cudaEventDestroy(s.a);
        if (s.b) cudaEventDestroy(s.b);
    }
    v.clear();
}

// Builds canonical Huffman codes (T.81 Annex C) for a table given as counts and values.
void build_huff(const uint8_t counts[16], const uint8_t* vals, uint32_t* code, uint32_t* len) {
    uint32_t c = 0;
    int k = 0;
    for (int l = 1; l <= 16; ++l) {
        for (int j = 0; j < counts[l - 1]; ++j) {
            const uint8_t sym = vals[k++];
            code[sym] = c;
            len[sym] = (uint32_t)l;
            ++c;
        }
        c <<= 1;
    }
}

void append_u16(std::vector<uint8_t>& v, uint32_t x) {
    v.push_back((uint8_t)(x >> 8));
    v.push_back((uint8_t)(x & 0xFF));
}

// Headers: SOI, APP0 JFIF, DQT (zigzag, both tables), SOF0, DHT (DC0, AC0, DC1, AC1), SOS.
void build_header(const EgpuContext* c, uint32_t w, uint32_t h, std::vector<uint8_t>& out);

}  // namespace

// ---------------------------------------------------------------- context

struct EgpuContext {
    int device = 0;
    int quality = 75;
    int c420 = 1;
    uint64_t budget = 0;
    int keep = 0;
    uint16_t qluma[64] = {};
    uint16_t qchroma[64] = {};

    Buf info, pbase, bbase, planes, coef, bits, off, tmp64, words, wff, wpref, wbase, ibits,
        nbytes, outent, eoi, out;

    std::vector<Span> spans;
    EgpuTimings timings{};

    std::vector<uint8_t> host_out;
    std::vector<uint64_t> out_begin, out_len;
    std::vector<std::vector<int16_t>> coef_host;
};

namespace {

void build_header(const EgpuContext* c, uint32_t w, uint32_t h, std::vector<uint8_t>& out) {
    out.clear();
    // SOI
    out.push_back(0xFF);
    out.push_back(0xD8);
    // APP0 JFIF 1.02, no density unit, 1:1 aspect, no thumbnail
    const uint8_t app0[] = {0xFF, 0xE0, 0x00, 0x10, 'J', 'F', 'I', 'F', 0x00, 0x01, 0x02,
                            0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00};
    out.insert(out.end(), app0, app0 + sizeof app0);
    // DQT: table 0 and table 1 in one segment, zigzag order
    out.push_back(0xFF);
    out.push_back(0xDB);
    append_u16(out, 132);
    out.push_back(0x00);
    for (int k = 0; k < 64; ++k) out.push_back((uint8_t)c->qluma[kZigzag[k]]);
    out.push_back(0x01);
    for (int k = 0; k < 64; ++k) out.push_back((uint8_t)c->qchroma[kZigzag[k]]);
    // SOF0: 8-bit, height, width, three components
    out.push_back(0xFF);
    out.push_back(0xC0);
    append_u16(out, 17);
    out.push_back(8);
    append_u16(out, h);
    append_u16(out, w);
    out.push_back(3);
    const uint8_t ys = c->c420 ? 0x22 : 0x11;
    out.push_back(1); out.push_back(ys); out.push_back(0);
    out.push_back(2); out.push_back(0x11); out.push_back(1);
    out.push_back(3); out.push_back(0x11); out.push_back(1);
    // DHT: four tables, length 418
    out.push_back(0xFF);
    out.push_back(0xC4);
    append_u16(out, 418);
    auto put_table = [&](uint8_t tc_th, const uint8_t* counts, const uint8_t* vals, int nv) {
        out.push_back(tc_th);
        for (int l = 0; l < 16; ++l) out.push_back(counts[l]);
        for (int j = 0; j < nv; ++j) out.push_back(vals[j]);
    };
    put_table(0x00, kDcLumCounts, kDcLumVals, 12);
    put_table(0x10, kAcLumCounts, kAcLumVals, 162);
    put_table(0x01, kDcChrCounts, kDcChrVals, 12);
    put_table(0x11, kAcChrCounts, kAcChrVals, 162);
    // SOS: three components, Ss=0, Se=63, Ah=Al=0
    out.push_back(0xFF);
    out.push_back(0xDA);
    append_u16(out, 12);
    out.push_back(3);
    out.push_back(1); out.push_back(0x00);
    out.push_back(2); out.push_back(0x11);
    out.push_back(3); out.push_back(0x11);
    out.push_back(0x00); out.push_back(0x3F); out.push_back(0x00);
}

// Encodes one sub-batch (images [0, n) of imgs) and appends its outputs at global index gbase.
int encode_chunk(EgpuContext* c, const uint8_t* d_rgb, const EgpuImage* imgs, uint32_t n, uint32_t gbase) {
    const int c420 = c->c420;
    const uint32_t mcu = c420 ? 16u : 8u;
    const uint32_t bpm = c420 ? 6u : 3u;

    // Host geometry: plane and block bases per image.
    std::vector<ImgInfo> hinfo(n);
    std::vector<uint64_t> hpb(n + 1), hbb(n + 1);
    uint64_t plane_total = 0, blk_total = 0;
    for (uint32_t i = 0; i < n; ++i) {
        const EgpuImage& im = imgs[i];
        ImgInfo& m = hinfo[i];
        std::memset(&m, 0, sizeof m);
        m.rgb_off = im.offset;
        m.w = im.width;
        m.h = im.height;
        m.mcus_x = (im.width + mcu - 1) / mcu;
        m.mcus_y = (im.height + mcu - 1) / mcu;
        m.pw = m.mcus_x * mcu;
        m.ph = m.mcus_y * mcu;
        m.cw = c420 ? m.pw / 2 : m.pw;
        m.ch = c420 ? m.ph / 2 : m.ph;
        hpb[i] = plane_total;
        plane_total += (uint64_t)m.pw * m.ph + 2ull * m.cw * m.ch;
        hbb[i] = blk_total;
        blk_total += (uint64_t)m.mcus_x * m.mcus_y * bpm;
    }
    hpb[n] = plane_total;
    hbb[n] = blk_total;
    const uint64_t nb = blk_total;
    // k_dct launches one block per JPEG block (`<<<(unsigned)nb, 64>>>`): the count must fit that unsigned grid. The size
    // check in the batch entry keeps any real input far below it; this makes the cast checked rather than assumed.
    if (nb > 0xFFFFFFFFull) {
        snprintf(g_error, sizeof g_error, "%llu JPEG blocks: more than one launch's grid holds", (unsigned long long)nb);
        return EGPU_ERR_SIZE;
    }

    ALLOC(c->info, n * sizeof(ImgInfo));
    CUDA_TRY(cudaMemcpy(c->info.p, hinfo.data(), n * sizeof(ImgInfo), cudaMemcpyHostToDevice));
    ALLOC(c->pbase, (n + 1) * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->pbase.p, hpb.data(), (n + 1) * 8, cudaMemcpyHostToDevice));
    ALLOC(c->bbase, (n + 1) * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->bbase.p, hbb.data(), (n + 1) * 8, cudaMemcpyHostToDevice));

    // Stage 1: colour and downsampling.
    ALLOC(c->planes, plane_total);
    int sp = open_span(c->spans, ST_COLOUR);
    k_colour<<<grid_colour(plane_total), kTpb>>>(d_rgb, dev<ImgInfo>(c->info), dev<uint64_t>(c->pbase), n,
                                                 dev<uint8_t>(c->planes), plane_total, c420);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(close_span(c->spans, sp));

    // Stage 2: DCT and quantize.
    ALLOC(c->coef, nb * 64 * sizeof(int16_t));
    sp = open_span(c->spans, ST_DCT);
    k_dct<<<(unsigned)nb, 64>>>(dev<uint8_t>(c->planes), dev<ImgInfo>(c->info), dev<uint64_t>(c->pbase),
                                dev<uint64_t>(c->bbase), n, dev<int16_t>(c->coef), c420);
    CUDA_TRY(cudaGetLastError());
    k_dummy_dc<<<grid_exact(nb), kTpb>>>(dev<int16_t>(c->coef), dev<ImgInfo>(c->info), dev<uint64_t>(c->bbase),
                                         n, nb, c420);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(close_span(c->spans, sp));

    // Stage 3: per-block bit counts. bits[nb] is the zero sentinel for the scan.
    ALLOC(c->bits, (nb + 1) * sizeof(uint64_t));
    ALLOC(c->off, (nb + 1) * sizeof(uint64_t));
    sp = open_span(c->spans, ST_SIZES);
    CUDA_TRY(cudaMemset(dev<uint64_t>(c->bits) + nb, 0, sizeof(uint64_t)));
    k_sizes<<<grid_exact(nb), kTpb>>>(dev<int16_t>(c->coef), dev<uint64_t>(c->bbase), n,
                                      dev<uint64_t>(c->bits), nb, c420);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(close_span(c->spans, sp));

    // Stage 4: exclusive scan over the batch, then per-image bit counts read back.
    sp = open_span(c->spans, ST_SCAN);
    {
        thrust::device_ptr<uint64_t> in(dev<uint64_t>(c->bits));
        thrust::device_ptr<uint64_t> outp(dev<uint64_t>(c->off));
        thrust::exclusive_scan(thrust::device, in, in + nb + 1, outp, (uint64_t)0);
    }
    ALLOC(c->tmp64, (n + 1) * sizeof(uint64_t));
    k_gather64<<<grid_exact(n + 1), kTpb>>>(dev<uint64_t>(c->off), dev<uint64_t>(c->bbase), n + 1,
                                            dev<uint64_t>(c->tmp64));
    CUDA_TRY(cudaGetLastError());
    std::vector<uint64_t> g1(n + 1);
    CUDA_TRY(cudaMemcpy(g1.data(), c->tmp64.p, (n + 1) * 8, cudaMemcpyDeviceToHost));
    std::vector<uint64_t> hbits(n), hwb(n + 1, 0);
    for (uint32_t i = 0; i < n; ++i) {
        hbits[i] = g1[i + 1] - g1[i];
        hwb[i + 1] = hwb[i] + (hbits[i] + 31) / 32 + 1;   // +1 spare word per image
    }
    const uint64_t W = hwb[n];
    ALLOC(c->wbase, (n + 1) * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->wbase.p, hwb.data(), (n + 1) * 8, cudaMemcpyHostToDevice));
    ALLOC(c->ibits, n * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->ibits.p, hbits.data(), n * 8, cudaMemcpyHostToDevice));
    CUDA_TRY(close_span(c->spans, sp));

    // Stage 5: clear word buffers (finishes before the pack kernel), then pack.
    ALLOC(c->words, W * sizeof(uint32_t));
    sp = open_span(c->spans, ST_PACK);
    CUDA_TRY(cudaMemset(c->words.p, 0, W * sizeof(uint32_t)));
    k_pack<<<grid_exact(nb), kTpb>>>(dev<int16_t>(c->coef), dev<uint64_t>(c->bbase), n,
                                     dev<uint64_t>(c->off), dev<uint64_t>(c->wbase),
                                     dev<uint32_t>(c->words), nb, c420);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(close_span(c->spans, sp));

    // Stage 6: 1-bit padding, FF counts per word, prefix sum, per-image FF counts.
    sp = open_span(c->spans, ST_PAD);
    k_pad<<<(n + 31) / 32, 32>>>(dev<uint32_t>(c->words), dev<uint64_t>(c->wbase), dev<uint64_t>(c->ibits), n);
    CUDA_TRY(cudaGetLastError());
    ALLOC(c->wff, (W + 1) * sizeof(uint32_t));
    ALLOC(c->wpref, (W + 1) * sizeof(uint64_t));
    CUDA_TRY(cudaMemset(dev<uint32_t>(c->wff) + W, 0, sizeof(uint32_t)));
    k_wcount<<<grid_exact(W), kTpb>>>(dev<uint32_t>(c->words), dev<uint32_t>(c->wff), W);
    CUDA_TRY(cudaGetLastError());
    {
        thrust::device_ptr<uint32_t> in(dev<uint32_t>(c->wff));
        thrust::device_ptr<uint64_t> outp(dev<uint64_t>(c->wpref));
        thrust::exclusive_scan(thrust::device, in, in + W + 1, outp, (uint64_t)0);
    }
    k_gather64<<<grid_exact(n + 1), kTpb>>>(dev<uint64_t>(c->wpref), dev<uint64_t>(c->wbase), n + 1,
                                            dev<uint64_t>(c->tmp64));
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(cudaMemcpy(g1.data(), c->tmp64.p, (n + 1) * 8, cudaMemcpyDeviceToHost));
    CUDA_TRY(close_span(c->spans, sp));

    // Host: output layout. Each image is header + entropy bytes + stuffing + EOI.
    std::vector<std::vector<uint8_t>> hdrs(n);
    std::vector<uint64_t> hnbytes(n), hout_base(n + 1, 0), hent(n), heoi(n);
    for (uint32_t i = 0; i < n; ++i) {
        build_header(c, imgs[i].width, imgs[i].height, hdrs[i]);
        hnbytes[i] = (hbits[i] + 7) / 8;
        const uint64_t ff = g1[i + 1] - g1[i];
        const uint64_t len = hdrs[i].size() + hnbytes[i] + ff + 2;
        hout_base[i + 1] = hout_base[i] + len;
        hent[i] = hout_base[i] + hdrs[i].size();
        heoi[i] = hent[i] + hnbytes[i] + ff;
    }
    const uint64_t out_total = hout_base[n];

    ALLOC(c->nbytes, n * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->nbytes.p, hnbytes.data(), n * 8, cudaMemcpyHostToDevice));
    ALLOC(c->outent, n * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->outent.p, hent.data(), n * 8, cudaMemcpyHostToDevice));
    ALLOC(c->eoi, n * sizeof(uint64_t));
    CUDA_TRY(cudaMemcpy(c->eoi.p, heoi.data(), n * 8, cudaMemcpyHostToDevice));
    ALLOC(c->out, out_total);

    // Stage 7: headers, stuffed entropy, EOI.
    sp = open_span(c->spans, ST_STUFF);
    for (uint32_t i = 0; i < n; ++i) {
        CUDA_TRY(cudaMemcpy(dev<uint8_t>(c->out) + hout_base[i], hdrs[i].data(), hdrs[i].size(),
                            cudaMemcpyHostToDevice));
    }
    k_stuff<<<grid_exact(W), kTpb>>>(dev<uint32_t>(c->words), dev<uint64_t>(c->wpref), dev<uint64_t>(c->wbase), n,
                                     dev<uint64_t>(c->nbytes), dev<uint64_t>(c->outent), W,
                                     dev<uint8_t>(c->out));
    CUDA_TRY(cudaGetLastError());
    k_eoi<<<(n + 63) / 64, 64>>>(dev<uint8_t>(c->out), dev<uint64_t>(c->eoi), n);
    CUDA_TRY(cudaGetLastError());
    CUDA_TRY(close_span(c->spans, sp));

    // Copy-out: outputs, and coefficients if requested.
    sp = open_span(c->spans, ST_COPY);
    const size_t host_base = c->host_out.size();
    c->host_out.resize(host_base + out_total);
    CUDA_TRY(cudaMemcpy(c->host_out.data() + host_base, c->out.p, out_total, cudaMemcpyDeviceToHost));
    for (uint32_t i = 0; i < n; ++i) {
        c->out_begin[gbase + i] = host_base + hout_base[i];
        c->out_len[gbase + i] = hout_base[i + 1] - hout_base[i];
        if (c->keep) {
            const uint64_t blocks = hbb[i + 1] - hbb[i];
            std::vector<int16_t>& dst = c->coef_host[gbase + i];
            dst.resize(blocks * 64);
            CUDA_TRY(cudaMemcpy(dst.data(), dev<int16_t>(c->coef) + hbb[i] * 64, blocks * 64 * sizeof(int16_t),
                                cudaMemcpyDeviceToHost));
        }
    }
    CUDA_TRY(close_span(c->spans, sp));
    return EGPU_OK;
}

int encode_all(EgpuContext* c, const uint8_t* d_rgb, const EgpuImage* images, uint32_t count) {
    CUDA_TRY(cudaSetDevice(c->device));
    destroy_spans(c->spans);
    c->timings = EgpuTimings{};
    c->host_out.clear();
    c->out_begin.assign(count, 0);
    c->out_len.assign(count, 0);
    c->coef_host.clear();
    if (c->keep) c->coef_host.resize(count);

    const auto t0 = std::chrono::steady_clock::now();
    uint32_t i0 = 0;
    while (i0 < count) {
        // Greedy sub-batch under the pixel budget; a single oversize image forms its own sub-batch.
        uint64_t px = 0;
        uint32_t i1 = i0;
        while (i1 < count) {
            const uint64_t p = (uint64_t)images[i1].width * images[i1].height;
            if (i1 > i0 && px + p > c->budget) break;
            px += p;
            ++i1;
        }
        const int rc = encode_chunk(c, d_rgb, images + i0, i1 - i0, i0);
        if (rc) return rc;
        i0 = i1;
    }
    CUDA_TRY(cudaDeviceSynchronize());

    for (const Span& s : c->spans) {
        float ms = 0.f;
        CUDA_TRY(cudaEventElapsedTime(&ms, s.a, s.b));
        float* f = nullptr;
        switch (s.stage) {
            case ST_COLOUR: f = &c->timings.colour_ms; break;
            case ST_DCT: f = &c->timings.dct_ms; break;
            case ST_SIZES: f = &c->timings.sizes_ms; break;
            case ST_SCAN: f = &c->timings.scan_ms; break;
            case ST_PACK: f = &c->timings.pack_ms; break;
            case ST_PAD: f = &c->timings.pad_ms; break;
            case ST_STUFF: f = &c->timings.stuff_ms; break;
            case ST_COPY: f = &c->timings.copy_ms; break;
            default: break;
        }
        if (f) *f += ms;
    }
    destroy_spans(c->spans);
    c->timings.total_ms =
        (float)std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
    return EGPU_OK;
}

}  // namespace

// ---------------------------------------------------------------- public C API

extern "C" void egpu_quant_tables(int quality, uint16_t luma[64], uint16_t chroma[64]) {
    int q = quality < 1 ? 1 : (quality > 100 ? 100 : quality);
    const int scale = q < 50 ? 5000 / q : 200 - 2 * q;
    for (int i = 0; i < 64; ++i) {
        int a = ((int)kStdLumQ[i] * scale + 50) / 100;
        int b = ((int)kStdChrQ[i] * scale + 50) / 100;
        a = a < 1 ? 1 : (a > 255 ? 255 : a);
        b = b < 1 ? 1 : (b > 255 ? 255 : b);
        luma[i] = (uint16_t)a;
        chroma[i] = (uint16_t)b;
    }
}

extern "C" const char* egpu_last_error(void) { return g_error; }

extern "C" int egpu_create(const EgpuOptions* o, EgpuContext** out) {
    if (!o || !out) {
        set_error("egpu_create: null argument");
        return EGPU_ERR_ARG;
    }
    *out = nullptr;
    EgpuContext* c = new (std::nothrow) EgpuContext();
    if (!c) {
        set_error("egpu_create: out of host memory");
        return EGPU_ERR_MEMORY;
    }
    c->device = o->device;
    c->quality = o->quality < 1 ? 1 : (o->quality > 100 ? 100 : o->quality);
    c->c420 = o->chroma_420 ? 1 : 0;
    c->budget = o->pixel_budget ? o->pixel_budget : (64ull << 20);
    c->keep = o->keep_coefficients ? 1 : 0;
    egpu_quant_tables(c->quality, c->qluma, c->qchroma);

    float qf[2][64];
    for (int i = 0; i < 64; ++i) {
        qf[0][i] = (float)c->qluma[i];
        qf[1][i] = (float)c->qchroma[i];
    }
    float cs[64];
    for (int u = 0; u < 8; ++u) {
        for (int x = 0; x < 8; ++x) {
            cs[u * 8 + x] = (float)std::cos((2.0 * x + 1.0) * u * 3.14159265358979323846 / 16.0);
        }
    }
    uint32_t hc[4][256] = {}, hl[4][256] = {};
    build_huff(kDcLumCounts, kDcLumVals, hc[HT_DC_LUM], hl[HT_DC_LUM]);
    build_huff(kAcLumCounts, kAcLumVals, hc[HT_AC_LUM], hl[HT_AC_LUM]);
    build_huff(kDcChrCounts, kDcChrVals, hc[HT_DC_CHR], hl[HT_DC_CHR]);
    build_huff(kAcChrCounts, kAcChrVals, hc[HT_AC_CHR], hl[HT_AC_CHR]);

    cudaError_t e = cudaSetDevice(c->device);
    if (e == cudaSuccess) e = cudaMemcpyToSymbol(c_q, qf, sizeof qf);
    if (e == cudaSuccess) e = cudaMemcpyToSymbol(c_zz, kZigzag, sizeof kZigzag);
    if (e == cudaSuccess) e = cudaMemcpyToSymbol(c_cos, cs, sizeof cs);
    if (e == cudaSuccess) e = cudaMemcpyToSymbol(c_hcode, hc, sizeof hc);
    if (e == cudaSuccess) e = cudaMemcpyToSymbol(c_hlen, hl, sizeof hl);
    if (e != cudaSuccess) {
        set_cuda_error("egpu_create", e);
        delete c;
        return EGPU_ERR_CUDA;
    }
    *out = c;
    return EGPU_OK;
}

extern "C" void egpu_destroy(EgpuContext* c) {
    if (!c) return;
    cudaSetDevice(c->device);
    destroy_spans(c->spans);
    Buf* bufs[] = {&c->info, &c->pbase, &c->bbase, &c->planes, &c->coef, &c->bits, &c->off, &c->tmp64,
                   &c->words, &c->wff, &c->wpref, &c->wbase, &c->ibits, &c->nbytes, &c->outent,
                   &c->eoi, &c->out};
    for (Buf* b : bufs) {
        if (b->p) cudaFree(b->p);
        b->p = nullptr;
        b->cap = 0;
    }
    delete c;
}

extern "C" int egpu_encode_batch(EgpuContext* c, const uint8_t* d_rgb, uint64_t d_rgb_bytes,
                                 const EgpuImage* images, uint32_t count) {
    set_error("ok");  // the last call's message is not this call's: a call that succeeds leaves "ok"
    if (!c) {
        set_error("egpu_encode_batch: null context");
        return EGPU_ERR_ARG;
    }
    if (count == 0 || !images || !d_rgb) {
        set_error("egpu_encode_batch: empty batch or null buffer");
        return EGPU_ERR_ARG;
    }
    // Validate every descriptor before any device work.
    for (uint32_t i = 0; i < count; ++i) {
        const EgpuImage& im = images[i];
        if (im.width == 0 || im.height == 0 || im.width > EGPU_MAX_DIM || im.height > EGPU_MAX_DIM) {
            snprintf(g_error, sizeof g_error, "image %u: size %ux%u outside 1..%u", i, im.width, im.height,
                     EGPU_MAX_DIM);
            return EGPU_ERR_SIZE;
        }
        const uint64_t need = (uint64_t)im.width * im.height * 3ull;
        if (im.offset > d_rgb_bytes || d_rgb_bytes - im.offset < need) {
            snprintf(g_error, sizeof g_error, "image %u: pixels [%llu, +%llu) outside the %llu-byte buffer", i,
                     (unsigned long long)im.offset, (unsigned long long)need, (unsigned long long)d_rgb_bytes);
            return EGPU_ERR_RANGE;
        }
    }
    try {
        return encode_all(c, d_rgb, images, count);
    } catch (const std::bad_alloc&) {
        set_error("egpu_encode_batch: host allocation failed");
        return EGPU_ERR_MEMORY;
    } catch (...) {
        set_error("egpu_encode_batch: unexpected exception");
        return EGPU_ERR_CUDA;
    }
}

extern "C" int egpu_output(const EgpuContext* c, uint32_t index, const uint8_t** data, uint64_t* size) {
    if (!c || !data || !size) {
        set_error("egpu_output: null argument");
        return EGPU_ERR_ARG;
    }
    if (index >= c->out_begin.size()) {
        set_error("egpu_output: index out of range for the last batch");
        return EGPU_ERR_ARG;
    }
    *data = c->host_out.data() + c->out_begin[index];
    *size = c->out_len[index];
    return EGPU_OK;
}

extern "C" int egpu_coefficients(const EgpuContext* c, uint32_t index, const int16_t** coef, uint64_t* blocks) {
    if (!c || !coef || !blocks) {
        set_error("egpu_coefficients: null argument");
        return EGPU_ERR_ARG;
    }
    if (!c->keep) {
        set_error("egpu_coefficients: context was created without keep_coefficients");
        return EGPU_ERR_ARG;
    }
    if (index >= c->coef_host.size()) {
        set_error("egpu_coefficients: index out of range for the last batch");
        return EGPU_ERR_ARG;
    }
    *coef = c->coef_host[index].data();
    *blocks = c->coef_host[index].size() / 64;
    return EGPU_OK;
}

extern "C" int egpu_timings(const EgpuContext* c, EgpuTimings* t) {
    if (!c || !t) {
        set_error("egpu_timings: null argument");
        return EGPU_ERR_ARG;
    }
    *t = c->timings;
    return EGPU_OK;
}
