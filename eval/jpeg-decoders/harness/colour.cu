// The colour stage after jpeggpu's planes: libjpeg-turbo's default ("fancy") chroma upsampling, jdsample.c's h1v2, h2v1
// and h2v2 triangle filters, and its YCbCr -> RGB fixed-point tables (jdcolor.c, SCALEBITS 16). With the same planes it
// reproduces libjpeg-turbo's RGB to the level, so its output can be compared with the reference decode directly.
//
// The filters are written per output pixel. For h2v2, a chroma column sum is 3 * (this row) + (the row above or below),
// and the output pixel is (3 * this column sum + the neighbouring column sum + bias) >> 4, with bias 8 on the left
// output of a pair and 7 on the right. Out-of-image neighbours repeat the edge sample (libjpeg's edge replication).
#include "colour.h"

namespace {

// libjpeg's FIX(x): x * 2^16, rounded.
constexpr int fix(double x) { return int(x * 65536.0 + 0.5); }
constexpr int kOneHalf = 1 << 15;
constexpr int kCrR = fix(1.40200);
constexpr int kCbB = fix(1.77200);
constexpr int kCrG = fix(0.71414);
constexpr int kCbG = fix(0.34414);

__device__ __forceinline__ int clampi(int v, int lo, int hi) { return v < lo ? lo : (v > hi ? hi : v); }

__device__ __forceinline__ uint8_t clamp8(int v) { return uint8_t(v < 0 ? 0 : (v > 255 ? 255 : v)); }

// Chroma value at luma pixel (x, y) of a plane of cw x ch samples.
__device__ __forceinline__ int chroma_at(const uint8_t* p, int pitch, int cw, int ch, int x, int y, int hs, int vs) {
    if (hs == 1 && vs == 1) return p[y * pitch + x];
    if (hs == 1) {  // h1v2 (4:4:0): (3 * this row + the row above or below + bias) >> 2, bias 1 on a pair's top row, 2 on the bottom
        const int j = y >> 1, b = y & 1;
        const int jn = clampi(b ? j + 1 : j - 1, 0, ch - 1);
        return (3 * p[j * pitch + x] + p[jn * pitch + x] + (b ? 2 : 1)) >> 2;
    }
    int i = x >> 1, a = x & 1;
    if (vs == 1) {  // h2v1: (3 * this + neighbour + bias) >> 2, bias 1 on the left, 2 on the right
        int self = 3 * p[y * pitch + i];
        int nb = p[y * pitch + clampi(a ? i + 1 : i - 1, 0, cw - 1)];
        return a ? (self + nb + 2) >> 2 : (self + nb + 1) >> 2;
    }
    // h2v2: column sums over the two chroma rows nearest the pixel, the row above for the top output row
    int j = y >> 1, b = y & 1;
    int jn = clampi(b ? j + 1 : j - 1, 0, ch - 1);
    int in = clampi(a ? i + 1 : i - 1, 0, cw - 1);
    int self = 3 * p[j * pitch + i] + p[jn * pitch + i];
    int nb = 3 * p[j * pitch + in] + p[jn * pitch + in];
    return a ? (3 * self + nb + 7) >> 4 : (3 * self + nb + 8) >> 4;
}

__global__ void ycc_to_rgb_kernel(const uint8_t* y, int ypitch, const uint8_t* cb, const uint8_t* cr, int cpitch,
                                  int w, int h, int cw, int ch, int hs, int vs, uint8_t* rgb) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int py = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= w || py >= h) return;
    int yy = y[py * ypitch + x];
    int xb = chroma_at(cb, cpitch, cw, ch, x, py, hs, vs) - 128;
    int xr = chroma_at(cr, cpitch, cw, ch, x, py, hs, vs) - 128;
    int r = yy + ((kCrR * xr + kOneHalf) >> 16);
    int g = yy + ((-kCbG * xb + kOneHalf - kCrG * xr) >> 16);
    int b = yy + ((kCbB * xb + kOneHalf) >> 16);
    uint8_t* o = rgb + (size_t(py) * w + x) * 3;
    o[0] = clamp8(r);
    o[1] = clamp8(g);
    o[2] = clamp8(b);
}

}  // namespace

cudaError_t ycc_to_rgb(const uint8_t* y, int ypitch, const uint8_t* cb, const uint8_t* cr, int cpitch, int w, int h,
                       int cw, int ch, int hs, int vs, uint8_t* rgb, cudaStream_t stream) {
    dim3 block(16, 16);
    dim3 grid((unsigned(w) + 15) / 16, (unsigned(h) + 15) / 16);
    ycc_to_rgb_kernel<<<grid, block, 0, stream>>>(y, ypitch, cb, cr, cpitch, w, h, cw, ch, hs, vs, rgb);
    return cudaGetLastError();
}
