# JPEG decoder comparison

`wsl.py` runs the comparison under WSL. Its `inputs` step reads one `<name>.jpg` per entry of `NAMES` from the
directory in `JPEG_SRC_DIR`. `win_build.py` builds the decoders on Windows.

## Third-party patches

`patches/` holds local patches to two upstream projects. `wsl_build.py` and `win_build.py` apply each one to a
clone of its upstream. Each patch modifies its upstream project, so it stays under that project's license. The
upstream license text is saved next to the patch:

- `patches/jpeggpu.patch`: [nolmoonen/jpeggpu](https://github.com/nolmoonen/jpeggpu), a low-latency CUDA JPEG decoder.
  License: GPL-3.0, text in `patches/LICENSE-jpeggpu.txt`.
- `patches/parajpeg.patch`: [anniigr/parajpeg](https://github.com/anniigr/parajpeg), a parallel JPEG decoder.
  License: MIT, text in `patches/LICENSE-parajpeg.txt`.
