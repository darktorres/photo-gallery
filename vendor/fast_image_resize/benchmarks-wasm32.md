<!-- introduction start -->

## Benchmarks of fast_image_resize crate for Wasm32 architecture

Environment:

- CPU: AMD Ryzen 9 5950X
- RAM: DDR4 4000 MHz
- Ubuntu 24.04 (linux 7.0.0)
- Rust 1.97.1
- criterion = "0.7.0"
- fast_image_resize = "6.0.1"
- wasmtime = "47.0.1"

Other libraries used to compare of resizing speed:

- image = "0.25.6" (<https://crates.io/crates/image>)
- resize = "0.8.9" (<https://crates.io/crates/resize>, single-threaded mode)

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

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  22.24  |   -   |  100.02  | 178.71  |  254.58  |
| resize      |  10.81  | 32.08 |  58.26   | 110.90  |  173.44  |
| fir rust    |  0.30   | 34.77 |  64.66   | 124.81  |  186.33  |
| fir simd128 |  0.30   | 5.77  |   8.81   |  15.43  |  22.49   |

<!-- bench_compare_rgb end -->

<!-- bench_compare_rgba start -->

### Resize RGBA8 image (U8x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| resize      |  11.41  | 38.23 |  73.39   | 138.00  |  211.01  |
| fir rust    |  0.25   | 88.42 |  129.85  | 213.80  |  298.47  |
| fir simd128 |  0.25   | 13.29 |  16.48   |  24.05  |  32.50   |

<!-- bench_compare_rgba end -->

<!-- bench_compare_l start -->

### Resize L8 image (U8) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with one byte per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  19.77  |   -   |  75.47   | 130.61  |  184.25  |
| resize      |  7.13   | 16.40 |  27.14   |  51.54  |  75.86   |
| fir rust    |  0.21   | 13.09 |  23.50   |  44.67  |  66.77   |
| fir simd128 |  0.21   | 2.63  |   3.46   |  5.46   |   8.06   |

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

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| fir rust    |  0.19   | 44.14 |  64.38   | 105.64  |  147.63  |
| fir simd128 |  0.19   | 7.67  |   9.54   |  13.86  |  18.60   |

<!-- bench_compare_la end -->

<!-- bench_compare_rgb16 start -->

### Resize RGB16 image (U16x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB16 image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  22.37  |   -   |  93.58   | 164.15  |  237.91  |
| resize      |  10.16  | 31.39 |  57.52   | 110.59  |  174.47  |
| fir rust    |  0.38   | 32.44 |  50.07   |  87.39  |  126.54  |
| fir simd128 |  0.36   | 38.94 |  68.75   | 128.67  |  188.66  |

<!-- bench_compare_rgb16 end -->

<!-- bench_compare_rgba16 start -->

### Resize RGBA16 image (U16x4) 4928x3279 => 852x567

Pipeline:

`src_image => multiply by alpha => resize => divide by alpha => dst_image`

- Source image
  [nasa-4928x3279-rgba.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279-rgba.png)
- Numbers in the table mean the duration of the image resizing in milliseconds.
- The `image` crate does not support multiplying and dividing by alpha channel.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| resize      |  12.52  | 38.62 |  72.34   | 139.18  |  206.96  |
| fir rust    |  0.47   | 85.87 |  113.63  | 170.33  |  229.21  |
| fir simd128 |  0.44   | 59.33 |  96.57   | 171.30  |  247.36  |

<!-- bench_compare_rgba16 end -->

<!-- bench_compare_l16 start -->

### Resize L16 image (U16) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  20.01  |   -   |  83.55   | 145.72  |  207.00  |
| resize      |  7.09   | 16.93 |  28.27   |  52.38  |  77.69   |
| fir rust    |  0.20   | 17.48 |  24.52   |  42.30  |  58.87   |
| fir simd128 |  0.19   | 14.29 |  22.45   |  41.29  |  60.30   |

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

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| fir rust    |  0.24   | 43.09 |  55.53   |  80.12  |  105.02  |
| fir simd128 |  0.24   | 30.67 |  49.72   |  88.30  |  128.09  |

<!-- bench_compare_la16 end -->

<!-- bench_compare_l32f start -->

### Resize L32F image (F32) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into grayscale image with two bytes per pixel.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  14.59  |   -   |  49.07   |  86.95  |  122.78  |
| resize      |  6.97   | 14.93 |  23.45   |  44.40  |  66.04   |
| fir rust    |  0.27   | 10.68 |  18.70   |  35.92  |  59.26   |
| fir simd128 |  0.25   | 10.68 |  19.01   |  35.94  |  59.39   |

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

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| fir rust    |  0.44   | 33.72 |  46.43   |  73.05  |  101.52  |
| fir simd128 |  0.44   | 33.70 |  46.40   |  73.01  |  101.54  |

<!-- bench_compare_la32f end -->

<!-- bench_compare_rgb32f start -->

### Resize RGB32F image (F32x3) 4928x3279 => 852x567

Pipeline:

`src_image => resize => dst_image`

- Source image [nasa-4928x3279.png](https://github.com/Cykooz/fast_image_resize/blob/main/data/nasa-4928x3279.png)
  has converted into RGB32F image.
- Numbers in the table mean the duration of the image resizing in milliseconds.

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| image       |  16.32  |   -   |  68.30   | 121.95  |  181.16  |
| resize      |  12.14  | 21.20 |  34.63   |  62.46  |  91.27   |
| fir rust    |  1.14   | 27.06 |  48.08   |  92.09  |  138.51  |
| fir simd128 |  1.06   | 27.05 |  48.05   |  92.08  |  138.67  |

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

|             | Nearest |  Box  | Bilinear | Bicubic | Lanczos3 |
|-------------|:-------:|:-----:|:--------:|:-------:|:--------:|
| fir rust    |  1.63   | 58.53 |  81.37   | 126.81  |  176.77  |
| fir simd128 |  1.64   | 58.50 |  81.35   | 126.79  |  176.73  |

<!-- bench_compare_rgba32f end -->