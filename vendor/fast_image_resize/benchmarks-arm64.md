<!-- introduction start -->

## Benchmarks of fast_image_resize crate for arm64 architecture

Environment:

- CPU: Neoverse-N1 2GHz (Oracle Cloud Compute, VM.Standard.A1.Flex)
- Ubuntu 24.04 (linux 6.17.0)
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

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| image    |  83.47  |   -    |  161.06  | 279.08  |  393.06  |
| resize   |  17.85  | 57.03  |  99.40   | 181.91  |  270.53  |
| libvips  |  9.37   | 137.72 |  26.85   |  65.72  |  87.42   |
| fir rust |  0.89   | 21.50  |  32.95   |  84.92  |  111.98  |
| fir neon |  0.89   | 18.98  |  29.21   |  53.59  |  79.72   |

<!-- bench_compare_rgb end -->

<!-- bench_compare_rgba start -->

### Resize RGBA8 image (U8x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| resize   |  23.09  | 81.37  |  118.87  | 189.42  |  280.82  |
| libvips  |  12.82  | 326.91 |  227.36  | 460.74  |  597.53  |
| fir rust |  1.03   | 48.21  |  62.48   | 128.18  |  168.24  |
| fir neon |  1.03   | 32.04  |  45.85   |  73.75  |  102.52  |

<!-- bench_compare_rgba end -->

<!-- bench_compare_l start -->

### Resize L8 image (U8) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with one byte per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|          | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image    |  75.08  |   -   |  107.05  | 159.00  |  214.51  |
| resize   |  10.58  | 25.99 |  39.19   |  69.52  |  92.19   |
| libvips  |  5.49   | 50.94 |  14.06   |  24.23  |  30.82   |
| fir rust |  0.52   | 8.13  |  11.46   |  18.97  |  25.56   |
| fir neon |  0.52   | 5.60  |   9.08   |  16.18  |  24.53   |

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

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips  |  8.43   | 185.55 |  131.99  | 229.53  |  290.66  |
| fir rust |  0.69   | 33.51  |  39.59   |  56.26  |  69.12   |
| fir neon |  0.69   | 19.10  |  24.36   |  38.38  |  53.34   |

<!-- bench_compare_la end -->

<!-- bench_compare_rgb16 start -->

### Resize RGB16 image (U16x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB16 image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| image    |  86.18  |   -    |  164.43  | 304.94  |  438.32  |
| resize   |  19.47  | 56.22  |  96.94   | 180.85  |  265.06  |
| libvips  |  22.69  | 196.50 |  108.59  | 227.56  |  300.61  |
| fir rust |  1.55   | 45.29  |  71.02   | 123.38  |  174.23  |
| fir neon |  1.55   | 48.92  |  107.66  | 181.87  |  276.08  |

<!-- bench_compare_rgb16 end -->

<!-- bench_compare_rgba16 start -->

### Resize RGBA16 image (U16x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| resize   |  27.32  | 84.23  |  121.16  | 200.17  |  294.56  |
| libvips  |  32.84  | 329.58 |  227.05  | 456.79  |  593.13  |
| fir rust |  1.67   | 104.82 |  156.77  | 283.16  |  388.88  |
| fir neon |  1.68   | 78.52  |  121.96  | 211.76  |  305.30  |

<!-- bench_compare_rgba16 end -->

<!-- bench_compare_l16 start -->

### Resize L16 image (U16) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|          | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image    |  77.58  |   -   |  108.87  | 167.09  |  222.83  |
| resize   |  10.99  | 24.25 |  37.62   |  67.41  |  94.50   |
| libvips  |  9.11   | 69.06 |  38.11   |  76.39  |  100.01  |
| fir rust |  0.69   | 23.13 |  33.31   |  52.91  |  74.68   |
| fir neon |  0.69   | 13.94 |  19.71   |  32.53  |  46.57   |

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

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips  |  17.42  | 200.41 |  143.11  | 242.22  |  302.76  |
| fir rust |  1.05   | 54.16  |  70.81   | 113.97  |  145.58  |
| fir neon |  1.05   | 28.26  |  40.27   |  65.29  |  91.44   |

<!-- bench_compare_la16 end -->

<!-- bench_compare_l32f start -->

### Resize L32F image (F32) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|          | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image    |  44.71  |   -   |  98.13   | 168.95  |  227.68  |
| resize   |  11.77  | 23.88 |  31.81   |  54.59  |  82.45   |
| libvips  |  7.93   | 67.05 |  38.91   |  91.03  |  119.15  |
| fir rust |  1.06   | 18.57 |  30.55   |  53.83  |  77.48   |

<!-- bench_compare_l32f end -->

Note:
The `resize` crate uses `f32` for intermediate calculations.
The `fast_image_resize` uses `f64`. This is a reason why `fast_image_resize`
is slower or equal in cases with `f32`-based pixels.

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

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips  |  16.40  | 184.59 |  125.68  | 224.14  |  283.76  |
| fir rust |  1.69   | 41.65  |  66.10   | 118.19  |  164.64  |

<!-- bench_compare_la32f end -->

<!-- bench_compare_rgb32f start -->

### Resize RGB32F image (F32x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB32F image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| image    |  55.15  |   -    |  127.71  | 294.01  |  397.87  |
| resize   |  22.09  | 43.39  |  67.18   | 128.89  |  186.98  |
| libvips  |  19.33  | 202.45 |  111.67  | 274.22  |  358.29  |
| fir rust |  2.55   | 41.09  |  71.48   | 149.31  |  213.81  |

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

|          | Nearest |  Box   | Bilinear | Bicubic | Lanczos3 |
|----------|:-------:|:------:|:--------:|:-------:|:--------:|
| libvips  |  30.65  | 322.37 |  221.02  | 443.62  |  578.03  |
| fir rust |  3.29   | 71.35  |  111.05  | 210.23  |  312.22  |

<!-- bench_compare_rgba32f end -->
