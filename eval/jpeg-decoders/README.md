# JPEG decoder comparison

`linux.py` runs the comparison on Linux, in the container (`just linux-eval eval/jpeg-decoders/linux.py STEP...`) or natively. Its `inputs` step reads one `<name>.jpg` per entry of `NAMES` from the
directory in `JPEG_SRC_DIR` (the source photos, read-only). The benches read their working copies from `JPEG_INPUTS` (default `~/jpeg-inputs`), which the inputs step fills from `JPEG_SRC_DIR`. `win_build.py` builds the decoders on Windows.

## Third-party patches

`patches/` holds local patches to two upstream projects. `linux_build.py` and `win_build.py` apply each one to a
clone of its upstream. Each patch modifies its upstream project, so it stays under that project's license. The
upstream license text is saved next to the patch:

- `patches/jpeggpu.patch`: [nolmoonen/jpeggpu](https://github.com/nolmoonen/jpeggpu), a low-latency CUDA JPEG decoder.
  License: GPL-3.0, text in `patches/LICENSE-jpeggpu.txt`.
- `patches/parajpeg.patch`: [anniigr/parajpeg](https://github.com/anniigr/parajpeg), a parallel JPEG decoder.
  License: MIT, text in `patches/LICENSE-parajpeg.txt`.

The other upstreams are not in this repository. `sources.py` (`PINS`) fetches each at its pinned commit when a build
runs: mozjpeg, jpegli, guetzli, wuffs, GPUJPEG and GPU-JPEG-Decoder, and the rust-turbojpeg fork named by
`tools/build_jpeg_turbo.py`. Nothing from them is copied here, and the licence each upstream publishes applies to its
own checkout.
