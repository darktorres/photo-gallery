// Checks colour.cu alone: libjpeg-turbo's own YUV planes go through it, and its RGB is compared with libjpeg-turbo's
// default RGB decode (fancy upsampling). Build from a Windows shell with the CUDA and libjpeg-turbo paths of win_build.py:
//   nvcc -std=c++17 -arch=sm_75 -Xcompiler="/EHsc /Zc:preprocessor /MT" colour_check.cu colour.cu -I<turbo include> turbojpeg-static.lib
// Result, 2026-11-14: 0 mismatched channels on 4:2:0, 4:2:2 and 4:4:4 files, odd sizes included.
#include <turbojpeg.h>
#include <cuda_runtime.h>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <vector>
#include "colour.h"

int main(int argc, char** argv) {
    for (int f = 1; f < argc; f++) {
        std::ifstream in(argv[f], std::ios::binary);
        std::vector<unsigned char> jpg((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
        tjhandle th = tjInitDecompress();
        int w = 0, hgt = 0, subsamp = 0, cs = 0;
        if (tjDecompressHeader3(th, jpg.data(), jpg.size(), &w, &hgt, &subsamp, &cs) != 0) {
            std::printf("%s: header: %s\n", argv[f], tjGetErrorStr2(th));
            return 1;
        }
        int cw = tjPlaneWidth(1, w, subsamp), ch = tjPlaneHeight(1, hgt, subsamp);
        int yw = tjPlaneWidth(0, w, subsamp), yh = tjPlaneHeight(0, hgt, subsamp);
        size_t ysz = size_t(yw) * yh, csz = size_t(cw) * ch;
        std::vector<unsigned char> yuv(ysz + 2 * csz), rgb(size_t(w) * hgt * 3);
        if (tjDecompressToYUV2(th, jpg.data(), jpg.size(), yuv.data(), w, 1, hgt, 0) != 0 ||
            tjDecompress2(th, jpg.data(), jpg.size(), rgb.data(), w, 0, hgt, TJPF_RGB, 0) != 0) {
            std::printf("%s: decode: %s\n", argv[f], tjGetErrorStr2(th));
            return 1;
        }
        int hs = (yw + cw - 1) / cw, vs = (yh + ch - 1) / ch;
        uint8_t *dy, *dcb, *dcr, *drgb;
        cudaMalloc(&dy, ysz);
        cudaMalloc(&dcb, csz);
        cudaMalloc(&dcr, csz);
        cudaMalloc(&drgb, rgb.size());
        cudaMemcpy(dy, yuv.data(), ysz, cudaMemcpyHostToDevice);
        cudaMemcpy(dcb, yuv.data() + ysz, csz, cudaMemcpyHostToDevice);
        cudaMemcpy(dcr, yuv.data() + ysz + csz, csz, cudaMemcpyHostToDevice);
        cudaError_t e = ycc_to_rgb(dy, yw, dcb, dcr, cw, w, hgt, cw, ch, hs, vs, drgb, 0);
        std::vector<unsigned char> out(rgb.size());
        if (e == cudaSuccess) e = cudaMemcpy(out.data(), drgb, out.size(), cudaMemcpyDeviceToHost);
        if (e != cudaSuccess) {
            std::printf("%s: cuda: %s\n", argv[f], cudaGetErrorString(e));
            return 1;
        }
        long long n = 0, diff = 0, sum = 0;
        int mx = 0;
        for (size_t i = 0; i < rgb.size(); i++, n++) {
            int d = std::abs(int(out[i]) - int(rgb[i]));
            diff += d != 0;
            sum += d;
            if (d > mx) mx = d;
        }
        std::printf("%s %dx%d subsamp=%d (h%d v%d): mismatched channels %.4f%%, mean %.5f, max %d\n", argv[f], w, hgt,
                    subsamp, hs, vs, 100.0 * diff / n, double(sum) / n, mx);
        cudaFree(dy); cudaFree(dcb); cudaFree(dcr); cudaFree(drgb);
        tjDestroy(th);
    }
    return 0;
}
