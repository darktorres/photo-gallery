// The colour stage jpeggpu leaves out: chroma upsampling and YCbCr -> RGB on the GPU (colour.cu).
#pragma once
#include <cstdint>
#include <cuda_runtime.h>

// Planes as jpeggpu returns them: Y is w x h, Cb and Cr are cw x ch, each row `pitch` bytes apart. hs and vs are the
// luma-to-chroma factors (1 or 2). Writes packed RGB (w * h * 3 bytes, `rgb` tightly packed) on `stream`, and returns
// the first CUDA error (cudaSuccess otherwise).
cudaError_t ycc_to_rgb(const uint8_t* y, int ypitch, const uint8_t* cb, const uint8_t* cr, int cpitch, int w, int h,
                       int cw, int ch, int hs, int vs, uint8_t* rgb, cudaStream_t stream);
