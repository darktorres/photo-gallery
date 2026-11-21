/*
 * encode_gpu.h - batched baseline JPEG encoder (SOF0, 8-bit, YCbCr, JFIF) on CUDA.
 *
 * Input is one device buffer of interleaved RGB8 holding every image of a batch,
 * plus one descriptor per image. Output is a complete JPEG file per image, held in
 * host memory owned by the context until the next egpu_encode_batch call.
 *
 * Batches larger than the context's pixel budget are split internally into several
 * sub-batches; each image is encoded independently, so the bytes do not depend on
 * how the batch was split.
 */
#ifndef ENCODE_GPU_H
#define ENCODE_GPU_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Largest accepted width or height. */
#define EGPU_MAX_DIM 65500u

typedef enum EgpuStatus {
    EGPU_OK = 0,
    EGPU_ERR_ARG = 1,     /* null pointer, zero count, bad option, index out of range */
    EGPU_ERR_SIZE = 2,    /* a width or height is outside 1..65500 */
    EGPU_ERR_RANGE = 3,   /* an image's pixels fall outside the device RGB buffer */
    EGPU_ERR_MEMORY = 4,  /* a device or host allocation failed */
    EGPU_ERR_CUDA = 5     /* a CUDA runtime call failed; see egpu_last_error() */
} EgpuStatus;

typedef struct EgpuContext EgpuContext;

typedef struct EgpuOptions {
    int quality;            /* 1..100; out-of-range values are clamped */
    int chroma_420;         /* nonzero: 4:2:0 (16x16 MCU); zero: 4:4:4 (8x8 MCU) */
    int device;             /* CUDA device ordinal */
    uint64_t pixel_budget;  /* max pixels encoded per internal sub-batch; 0 = 64 Mi pixels (2^26 = 67,108,864) */
    int keep_coefficients;  /* nonzero: keep quantized coefficients for egpu_coefficients() */
} EgpuOptions;

typedef struct EgpuImage {
    uint64_t offset;   /* byte offset of the image's first pixel in the device RGB buffer */
    uint32_t width;
    uint32_t height;   /* row stride is width * 3 bytes, no padding */
} EgpuImage;

/* Stage times in milliseconds, summed over all sub-batches of the last egpu_encode_batch call. */
typedef struct EgpuTimings {
    float colour_ms;   /* pad, RGB to YCbCr, 4:2:0 downsampling */
    float dct_ms;      /* forward DCT and quantization */
    float sizes_ms;    /* per-block bit counts */
    float scan_ms;     /* prefix sums, per-image layout, host read-back of sizes */
    float pack_ms;     /* zero the word buffers, Huffman coding and bit packing */
    float pad_ms;      /* 1-bit padding, 0xFF counts */
    float stuff_ms;    /* headers, byte stuffing, EOI */
    float copy_ms;     /* output read-back (and coefficient read-back if kept) */
    float total_ms;    /* host wall clock for the whole call */
} EgpuTimings;

/* Creates a context. On failure *out is left NULL and egpu_last_error() describes why. */
int egpu_create(const EgpuOptions* options, EgpuContext** out);
void egpu_destroy(EgpuContext* ctx);

/* Thread-local message describing the most recent failure on this thread. */
const char* egpu_last_error(void);

/* Encodes count images from the device buffer d_rgb (d_rgb_bytes long). Validates every
 * descriptor before any device work. Replaces the outputs of the previous call. */
int egpu_encode_batch(EgpuContext* ctx, const uint8_t* d_rgb, uint64_t d_rgb_bytes,
                      const EgpuImage* images, uint32_t count);

/* Output JPEG of image index from the last call. The pointer is valid until the next
 * egpu_encode_batch or egpu_destroy. */
int egpu_output(const EgpuContext* ctx, uint32_t index, const uint8_t** data, uint64_t* size);

/* Quantized coefficients of image index from the last call (requires keep_coefficients).
 * Blocks are in scan order (MCU raster order, then the block order of the chroma mode);
 * each block is 64 int16 values in natural (row-major) order. */
int egpu_coefficients(const EgpuContext* ctx, uint32_t index, const int16_t** coef, uint64_t* blocks);

int egpu_timings(const EgpuContext* ctx, EgpuTimings* timings);

/* Quantization tables for a quality setting (clamped to 1..100), natural order. Host only. */
void egpu_quant_tables(int quality, uint16_t luma[64], uint16_t chroma[64]);

#ifdef __cplusplus
}
#endif

#endif /* ENCODE_GPU_H */
