<!-- introduction start -->

## Benchmarks of fast_image_resize crate for x86_64 architecture

Environment:

- CPU: AMD Ryzen 9 5950X
- RAM: DDR4 4000 MHz
- Ubuntu 24.04 (linux 7.0.0)
- Rust 1.97.1
- criterion = "0.7.0"
- fast_image_resize = "6.0.1"

Other libraries used to compare of resizing speed:

- image = "0.25.6" (<https://crates.io/crates/image>)
- resize = "0.8.9" (<https://crates.io/crates/resize>, single-threaded mode)
- libvips = "8.15.1" (single-threaded mode)

Resize algorithms:

- Nearest
- Box - convolution with minimal kernel size 1x1 px
- Bilinear - convolution with minimal kernel size 2x2 px
- Bicubic (CatmullRom) - convolution with minimal kernel size 4x4 px
- Lanczos3 - convolution with minimal kernel size 6x6 px

<!-- introduction end -->

<!-- bench_compare_rgb start -->

### Resize RGB8 image (U8x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  30.36  |   -   |  80.24   | 135.29  |  176.09  |
| resize     |  8.50   | 24.37 |  48.57   |  92.43  |  137.70  |
| libvips    |  2.32   | 61.51 |   5.58   |  9.92   |  17.31   |
| fir rust   |  0.28   | 10.75 |  15.62   |  26.21  |  37.26   |
| fir sse4.1 |  0.28   | 3.82  |   5.83   |  10.34  |  15.91   |
| fir avx2   |  0.28   | 2.65  |   3.55   |  6.90   |  13.52   |

<!-- bench_compare_rgb end -->

<!-- bench_compare_rgba start -->

### Resize RGBA8 image (U8x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|            | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:------:|:--------:|:-------:|:--------:|
| resize     |  14.48  | 45.62  |  90.91   | 152.92  |  218.65  |
| libvips    |  4.94   | 169.47 |  137.95  | 231.08  |  332.42  |
| fir rust   |  0.19   | 20.81  |  26.01   |  37.99  |  50.76   |
| fir sse4.1 |  0.19   | 10.74  |  13.18   |  19.20  |  26.25   |
| fir avx2   |  0.19   |  9.38  |  10.21   |  15.20  |  24.08   |

<!-- bench_compare_rgba end -->

<!-- bench_compare_l start -->

### Resize L8 image (U8) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with one byte per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  28.88  |   -   |  60.77   |  89.05  |  116.76  |
| resize     |  7.07   | 11.83 |  21.53   |  44.31  |  69.89   |
| libvips    |  2.56   | 24.66 |   6.62   |  9.57   |  12.54   |
| fir rust   |  0.15   | 4.29  |   5.55   |  8.72   |  12.11   |
| fir sse4.1 |  0.15   | 1.64  |   2.25   |  3.63   |   5.80   |
| fir avx2   |  0.15   | 1.82  |   1.84   |  2.84   |   4.26   |

<!-- bench_compare_l end -->

<!-- bench_compare_la start -->

### Resize LA8 image (U8x2) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
  has converted into grayscale image with an alpha channel (two bytes per pixel).
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.
- The `resize` crate does not support this pixel format.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| libvips    |  3.59   | 93.88 |  77.70   | 123.69  |  166.90  |
| fir rust   |  0.17   | 18.02 |  20.36   |  26.99  |  32.65   |
| fir sse4.1 |  0.17   | 6.31  |   7.41   |  10.16  |  13.98   |
| fir avx2   |  0.17   | 4.65  |   5.16   |  6.98   |  10.21   |

<!-- bench_compare_la end -->

<!-- bench_compare_rgb16 start -->

### Resize RGB16 image (U16x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB16 image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  31.09  |   -   |  89.60   | 145.58  |  198.27  |
| resize     |  8.17   | 21.48 |  41.30   |  80.91  |  121.04  |
| libvips    |  14.00  | 96.20 |  65.80   | 131.67  |  177.22  |
| fir rust   |  0.36   | 25.70 |  39.39   |  67.16  |  96.41   |
| fir sse4.1 |  0.36   | 16.15 |  24.01   |  40.75  |  58.29   |
| fir avx2   |  0.36   | 13.22 |  17.60   |  28.08  |  36.48   |

<!-- bench_compare_rgb16 end -->

<!-- bench_compare_rgba16 start -->

### Resize RGBA16 image (U16x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|            | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:------:|:--------:|:-------:|:--------:|
| resize     |  15.49  | 45.33  |  87.35   | 153.64  |  221.48  |
| libvips    |  22.28  | 183.85 |  149.64  | 243.10  |  345.06  |
| fir rust   |  0.40   | 55.40  |  72.13   | 105.80  |  143.30  |
| fir sse4.1 |  0.40   | 34.66  |  45.67   |  69.39  |  93.68   |
| fir avx2   |  0.40   | 25.77  |  30.31   |  41.44  |  53.51   |

<!-- bench_compare_rgba16 end -->

<!-- bench_compare_l16 start -->

### Resize L16 image (U16) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  29.25  |   -   |  60.88   |  90.27  |  118.33  |
| resize     |  5.26   | 11.86 |  21.36   |  44.85  |  69.39   |
| libvips    |  5.47   | 34.60 |  23.34   |  43.35  |  59.19   |
| fir rust   |  0.17   | 13.11 |  16.81   |  28.94  |  39.37   |
| fir sse4.1 |  0.17   | 5.81  |   7.74   |  13.97  |  19.92   |
| fir avx2   |  0.17   | 5.67  |   6.36   |  9.33   |  14.07   |

<!-- bench_compare_l16 end -->

<!-- bench_compare_la16 start -->

### Resize LA16 (luma with alpha channel) image (U16x2) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
  has converted into grayscale image with an alpha channel (four bytes per pixel).
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.
- The `resize` crate does not support this pixel format.

|            | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips    |  11.87  | 105.96 |  84.97   | 132.95  |  176.89  |
| fir rust   |  0.19   | 28.66  |  34.62   |  53.18  |  72.11   |
| fir sse4.1 |  0.19   | 16.04  |  22.10   |  34.74  |  48.84   |
| fir avx2   |  0.19   | 13.72  |  16.31   |  23.55  |  31.07   |

<!-- bench_compare_la16 end -->

<!-- bench_compare_l32f start -->

### Resize L32F image (F32) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  26.69  |   -   |  54.45   |  85.39  |  113.72  |
| resize     |  5.42   | 9.50  |  14.22   |  31.29  |  46.76   |
| libvips    |  4.50   | 34.73 |  23.56   |  46.25  |  65.56   |
| fir rust   |  0.20   | 7.44  |  12.09   |  26.54  |  39.65   |
| fir sse4.1 |  0.19   | 4.80  |   7.44   |  13.22  |  19.10   |
| fir avx2   |  0.19   | 4.41  |   5.69   |  8.93   |  12.51   |

<!-- bench_compare_l32f end -->

<!-- bench_compare_la32f start -->

### Resize LA32F (luma with alpha channel) image (F32x2) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
  has converted into grayscale image with an alpha channel (two `f32` values per pixel).
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.
- The `resize` crate does not support this pixel format.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| libvips    |  12.02  | 93.60 |  75.39   | 121.86  |  163.75  |
| fir rust   |  0.42   | 30.11 |  37.10   |  55.80  |  78.18   |
| fir sse4.1 |  0.42   | 24.32 |  28.95   |  39.50  |  50.96   |
| fir avx2   |  0.44   | 24.58 |  25.93   |  32.60  |  39.07   |

<!-- bench_compare_la32f end -->

<!-- bench_compare_rgb32f start -->

### Resize RGB32F image (F32x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB32F image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|            | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image      |  28.91  |   -   |  62.84   | 102.42  |  145.36  |
| resize     |  11.57  | 15.51 |  24.37   |  48.59  |  72.48   |
| libvips    |  10.87  | 92.99 |  65.78   | 135.89  |  191.89  |
| fir rust   |  1.02   | 15.14 |  25.18   |  48.65  |  73.80   |
| fir sse4.1 |  1.03   | 12.93 |  20.47   |  35.91  |  53.21   |
| fir avx2   |  1.03   | 10.67 |  18.05   |  33.61  |  51.05   |

<!-- bench_compare_rgb32f end -->


<!-- bench_compare_rgba32f start -->

### Resize RGBA32F image (F32x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.
- The `resize` crate does not support multiplying and dividing by alpha channel
  for this pixel format.

|            | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|------------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips    |  22.43  | 157.97 |  127.17  | 205.06  |  302.74  |
| fir rust   |  1.28   | 47.68  |  54.92   |  79.85  |  103.16  |
| fir sse4.1 |  1.28   | 44.79  |  51.30   |  72.49  |  94.41   |
| fir avx2   |  1.28   | 43.12  |  44.46   |  54.10  |  66.42   |

<!-- bench_compare_rgba32f end -->