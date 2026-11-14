# PNG decoders

Which PNG decoder the app should use. The candidates are the `png` fork that `pg-codec` calls (pinned at `2f176ed`), the
same crate from crates.io (png 0.18.1), zune-png 0.5.2, libpng 1.6.59, libspng 0.7.4 and Wuffs (d28ecd4). Pillow 12.2.0
was run once, as a second opinion. Everything runs natively on Windows in `eval/png-decoders`. Wall time, heap traffic
and output are measured. Instruction counts are not (callgrind doesn't run here).

The study is complete except one question: a keyed 8-bit photograph from a second source (see Next).

## Decision

- **Keep the fork, now at `2f176ed`.** Before the fix the fork was 1.2× slower than png 0.18 on the keyed 8-bit
  files, and its keyed decode cost 2.8–5.1× its own unkeyed time, because its tRNS expansion made library calls for
  every pixel. The fix (`expand_trns_fixed`, pushed to `darktorres/image-png` and pinned) removes those calls. Measured
  after it, in one run per file, the fork is the fastest decoder on most files and within 7% of the fastest on all of
  them. The closest are the 116 MP and 2.46 MP RGBA files (tied with png 0.18 within 4%) and the interlaced 8-bit RGB
  file (png 0.18 7% faster).
- **No other decoder is worth switching to.** libpng and libspng are 1.3–3× the fork's time on most files. zune-png is
  slower on every unkeyed file in quiet pass 2, and it returns a new buffer on each decode (1.2 GB on the 116 MP file).
  Wuffs is within about 5–25% of the fork on the 8-bit RGB files in quiet pass 4 (its AVX2 build), and 1.8–4.7× the
  fork's time on the 16-bit files in quiet pass 2 (its scalar build; the AVX2 build changes nothing there). It also
  writes the colour of a transparent key pixel as black, where the app keeps it, so it would change the output.
- **The only app-side finding was the fork's colour-key expansion.** It is fixed. Nothing else in the study points at
  the app's code: the other gaps are between libraries.

## Decoders

- **png (app fork), `2f176ed`:** decoded as `Codecs::png` does. Its scratch is kept between files, text and ICC chunks
  are skipped, EXPAND is on with 16-bit kept, and the app's 512 MiB limit applies. Before `2f176ed` it was pinned at
  `f7129d3`; the figures marked "pre-fix" below are from that pin.
- **png 0.18:** crates.io's 0.18.1 with the same options and limit, and no scratch. It uses the app's patched
  `fdeflate`, which the root `Cargo.toml` patches for every user of fdeflate, so it isn't the stock crate.
- **zune-png 0.5.2** with zune-core 0.5.3. Its height cap is raised to 2^20. It returns a new buffer each decode, so its
  time includes that allocation.
- **libpng 1.6.59** (static, with zlib 1.4): the standard read API with EXPAND-equivalent transforms, 16-bit kept. It
  creates its read structures on each decode, as libpng is normally used. Text chunks are skipped.
- **libspng 0.7.4** (static, with the same zlib 1.4): raw output with the palette and tRNS expansions, one context per
  decode. It can't skip text chunks, so the library files were run as copies with their tEXt, zTXt, iTXt and iCCP
  chunks removed (`make_variants.py`), for every decoder. The pixels are unchanged.
- **Wuffs d28ecd4:** `wuffs_aux::DecodeImage`, to the nearest output format. For 8-bit gray, RGB and RGBA that is the
  app's layout. For 8-bit gray+alpha (Wuffs' PNG decoder won't write YA_NONPREMUL) it decodes to RGBA and keeps the gray
  and alpha. For 16-bit it decodes to BGRA_NONPREMUL_4X16LE and repacks to the app's layout (samples big-endian,
  reordered, alpha dropped for RGB), inside the timed decode. The decoder object is recreated per call (about 44 KiB).
  The harness passes `/arch:AVX2` to Wuffs' translation unit only, so its SIMD unfilter is compiled in. Quiet pass 1 and
  2 used the scalar build; quiet pass 3 measures both.
- **Pillow 12.2.0** (`pillow_bench.py`): its own `zip` decoder over zlib-ng 1.3.1, not libpng. Its time includes
  `Image.open`, `load` and any conversion. A grey or RGB key is converted to LA or RGBA, as the app gives. It was run
  once, in quiet pass 1, and skips 16-bit files (Pillow reads them at 8 bits).

## Inputs

**The library's sample.** 294 PNGs in `datasets/pinterest` and `equiv/src`, all 8-bit: 188 RGB, 104 RGBA and 2
palette. Seven are used, the nearest to the target sizes, plus the largest:

| Input | Kind | Size | File |
|---|---|---|---|
| `rgb-1mp` | RGB 8-bit | 1.01 MP, 1.7 MB | `pinterest_1141451468122730916` |
| `rgb-3mp` | RGB 8-bit | 2.79 MP, 1.0 MB | `pinterest_1141451468123671931` |
| `rgb-4mp` | RGB 8-bit | 4.28 MP, 5.8 MB | `pinterest_1141451468122225667` |
| `rgba-1mp` | RGBA 8-bit | 0.96 MP, 1.5 MB | `pinterest_1141451468122732060` |
| `rgba-2mp` | RGBA 8-bit | 2.46 MP, 3.0 MB | `pinterest_1141451468123671747` |
| `palette-2mp` | palette | 2.07 MP, 0.8 MB | `pinterest_1141451468123718688` |
| `rgb-116mp` | RGB 8-bit | 116.14 MP, 142 MB | `_incoming/db4f698cb287297b07d5a501a6595f51` |

**Made for the cases the sample lacks** (`make_inputs.py`). The pixels are the library's. The encoder is ours, because
Pillow can't write 16-bit RGB(A), interlaced or APNG files. Each row uses the least-residual filter, at zlib level 6.

| Input | Kind | Size | Made from |
|---|---|---|---|
| `rgb16-3mp` | RGB 16-bit | 2.79 MP, 1.3 MB | `rgb-3mp` × 257 |
| `rgba16-2mp` | RGBA 16-bit | 2.46 MP, 4.0 MB | `rgba-2mp` × 257 |
| `rgb16-interlaced-3mp` | RGB 16-bit, Adam7 | 2.79 MP, 1.9 MB | `rgb-3mp` × 257 |
| `rgb8-interlaced-3mp` | RGB 8-bit, Adam7 | 2.79 MP, 1.5 MB | `rgb-3mp` |
| `rgba8-interlaced-2mp` | RGBA 8-bit, Adam7 | 2.46 MP, 3.8 MB | `rgba-2mp` |
| `gray8-2mp` | grayscale 8-bit | 2.79 MP, 0.4 MB | `rgb-3mp`, luminance (0.299, 0.587, 0.114) |
| `apng-rgb8-3frames` | RGB 8-bit, APNG | 3 frames of 2.79 MP, 3.1 MB | `rgb-3mp`; frames 1–2 are shifted and flipped copies |

The synthetic 16-bit samples are the 8-bit value × 257, so their high and low bytes are equal. That compresses better
than real 16-bit data, so their inflate is cheaper than a photograph's.

**Chunk variants** (`make_variants.py`). The pixels are never re-encoded; a chunk is added or the file is copied.

| Input | Kind | Size | Made from |
|---|---|---|---|
| `palette-2mp-trns` | palette, tRNS | 2.07 MP, 0.8 MB | `palette-2mp`, one alpha per entry (they vary) |
| `rgb8-3mp-trns` | RGB 8-bit, colour key | 2.79 MP, 1.0 MB | `rgb-3mp`, key = pixel (0, 0)'s colour |
| `gray8-2mp-trns` | grayscale 8-bit, key | 2.79 MP, 0.4 MB | `gray8-2mp`, key = pixel (0, 0)'s grey |
| `gray16-2mp-trns` | grayscale 16-bit, key | 2.79 MP, 0.6 MB | `gray8-2mp` × 257, key = pixel (0, 0)'s grey |
| `rgb8-3mp-filter0` | RGB 8-bit, filter 0 | 2.79 MP | `rgb-3mp`, filter type 0 on every row |
| `rgb8-4mp-filter0` | RGB 8-bit, filter 0 | 4.28 MP | `rgb-4mp`, filter type 0 on every row |
| `real-flower_foveon-16bit-trns` | RGB 16-bit, colour key | 3.43 MP, 15.6 MB | `real-flower_foveon`, key = pixel (0, 0), as 16-bit big-endian R, G, B |

The filter-0 variants make unfiltering a copy, so their decode time is almost all inflate. They are the check on the
inflate question below. `rgb8-3mp-trns` is already a real 8-bit photograph: its source, `rgb-3mp`, is a Pinterest photograph. A second keyed
copy of it (`real-pinterest-8bit-trns`) decodes identically and was made in error; it is not a separate test.

**Real photographs** (`make_real16.py`). The 16-bit RGB set of
[imagecompression.info](https://www.imagecompression.info/test_images), "The New Image Compression Test Set" (Sachin
Garg, 2008), whose terms allow redistribution with attribution. The archive holds 16-bit PPMs, read from the zip and
written as 16-bit PNGs with the same encoder.

| Input | Size | PNG size |
|---|---|---|
| `real-flower_foveon` | 2268 × 1512 (3.43 MP) | 15.6 MB |
| `real-leaves_iso_200` | 3008 × 2000 (6.02 MP) | 31.0 MB |
| `real-nightshot_iso_1600` | 3136 × 2352 (7.38 MP) | 37.7 MB |
| `real-big_tree` | 6088 × 4550 (27.70 MP) | 145.5 MB |

Their PNGs are about 5 MB per megapixel, against 0.5 MB per megapixel for the synthetic 16-bit files, so their inflate
costs differ from the synthetic files'.

**Text-free library copies** (`inputs-notext`): the seven library files with their text chunks removed, used for every
decoder.

## Method and reliability

Each decoder decodes the file's bytes to packed samples on one thread, full size. The output buffer is reused across
decodes, as the app reuses its output. Each time is the median over at least five decodes, after two warm-ups, for a
budget of 2 s (or as stated). The fork skips the zlib checksum, as the app does. png's `ignore_adler32` is on by default,
and Wuffs' PNG decoder sets its checksum quirk to ignore it too.

Runs and their load:

- **Quiet pass 1** (20–32% load): the seven library files and the seven generated ones, with Pillow.
- **Quiet pass 2** (17–40% load): all 23 files in one run, every decoder back to back on each file. This is the main
  table for the Rust and C decoders, with the scalar Wuffs build.
- **Quiet pass 3** (7–27% load): 24 files, with Wuffs' scalar and AVX2 builds back to back on each file.
- **Quiet pass 4** (load not checked during the run): eleven files, with the Rust harness (the fork, png 0.18, zune-png)
  and the C harness (libpng, libspng, Wuffs AVX2) back to back on each file.
- **Streamed-inflate runs** (7–28% load): the inflate of each file's stream alone, in fdeflate and zlib.
- **A high-load pass** (83–99% load, other sessions running) was discarded. Its absolute times were 1.7–2.4× the quiet
  passes' for the same files, and it is not used.

Absolute times move by 20–40% between runs, and the 116 MP file moved 24% between passes 1 and 2. Ratios between
decoders measured back to back within one run are the reliable figures. A ratio of 1.2× or less between two runs is not
a result on its own.

## Results

### Speed

Quiet pass 2, all 23 files, one run. Median ms per decode.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs (scalar) |
|---|---:|---:|---:|---:|---:|---:|
| rgb-116mp | 1229.55 | 1983.78 | 1743.65 | 1751.92 | 1724.14 | 1924.05 |
| rgb-4mp | 42.30 | 39.48 | 64.47 | 62.96 | 62.49 | 65.03 |
| rgb-1mp | 9.46 | 9.64 | 15.29 | 17.73 | 17.45 | 19.02 |
| rgba-1mp | 11.08 | 10.65 | 17.06 | 18.38 | 18.05 | 20.67 |
| rgba-2mp | 29.03 | 21.45 | 25.27 | 38.06 | 37.52 | 38.34 |
| rgb-3mp | 10.99 | 10.87 | 15.38 | 22.61 | 21.33 | 18.43 |
| palette-2mp | 5.56 | 5.57 | 8.33 | 11.73 | 11.27 | 5.85 |
| apng-rgb8-3frames | 14.21 | 13.26 | 16.78 | 24.30 | 23.39 | 18.71 |
| gray8-2mp | 5.79 | 5.72 | 8.26 | 10.03 | 14.19 | 8.53 |
| rgb16-3mp | 15.89 | 15.71 | 21.24 | 48.07 | 76.95 | 74.04 |
| rgb16-interlaced-3mp | 29.01 | 30.22 | 40.25 | 68.16 | 74.91 | 70.41 |
| rgb8-interlaced-3mp | 21.00 | 22.81 | 30.95 | 43.87 | 30.39 | 39.21 |
| rgba16-2mp | 29.49 | 30.18 | 34.23 | 72.98 | 110.33 | 93.80 |
| rgba8-interlaced-2mp | 33.60 | 36.52 | 40.97 | 63.78 | 53.22 | 54.56 |
| gray8-2mp-trns (pre-fix fork) | 29.50 | 24.47 | 9.21 | 12.67 | 21.99 | 29.08 |
| palette-2mp-trns | 5.47 | 5.38 | 7.67 | 12.23 | 11.47 | 5.78 |
| rgb8-3mp-filter0 | 8.83 | 9.96 | 11.40 | 14.88 | 14.60 | 9.06 |
| rgb8-3mp-trns (pre-fix fork) | 30.51 | 25.13 | 18.15 | 24.97 | 37.48 | 35.15 |
| rgb8-4mp-filter0 | 49.76 | 48.43 | 48.74 | 68.64 | 67.73 | 47.26 |
| real-big_tree | 689.27 | 645.30 | 675.52 | 1041.11 | 1969.38 | 1914.94 |
| real-flower_foveon | 94.27 | 88.47 | 92.15 | 139.92 | 176.85 | 166.98 |
| real-leaves_iso_200 | 143.90 | 140.12 | 147.31 | 246.75 | 446.07 | 415.47 |
| real-nightshot_iso_1600 | 170.25 | 171.15 | 172.02 | 294.23 | 453.32 | 435.86 |

Read from this table:

- In pass 2 the fork and png 0.18 are within 10% of each other on 18 of the 23 files. The five beyond 10% are the 116 MP
  file (png 0.18 61% slower), the 2.46 MP RGBA file (png 0.18 26% faster), the two pre-fix colour-key files, and the
  filter-0 file (png 0.18 13% slower). Pass 4 does not reproduce the 116 MP or the 2.46 MP gaps (below).
- libpng is 1.3–3.0× the fork on the unkeyed files, and libspng 1.3–2.5× on 8-bit and 2.6–4.8× on the 16-bit
  synthetics. Both are slower on the photographs too (libspng 1.9–3.1×).
- Wuffs (scalar) is 1.8–4.7× the fork on the 16-bit files, where its repacking and generic unfilter cost the most.
  Its AVX2 build (pass 3) closes most of the 8-bit gap, below.
- Quiet pass 1 (with Pillow): the fork and png 0.18 are within 10% of each other on every file, and Pillow is 1.5–2.5× the
  fork's time. Pass 2 supersedes it for every decoder but Pillow.

Pillow, quiet pass 1: 23.26 (rgb-1mp), 24.86 (rgb-3mp), 79.16 (rgb-4mp), 25.73 (rgba-1mp), 50.07 (rgba-2mp), 11.32
(palette-2mp), 2336.43 (116 MP), 24.58 (apng), 38.66 (rgb8-interlaced-3mp), 59.64 (rgba8-interlaced-2mp), 9.07 (gray8-2mp).
Pillow reads 16-bit RGB(A) at 8 bits, so it has no 16-bit figures.

### Wuffs' SIMD unfilter (quiet pass 3)

The same 24 files with Wuffs' scalar and AVX2 builds, back to back on each file. libpng and libspng are the same code in
both binaries, so they are shown as a check on the run.

| Input | libpng 1.6 | libspng 0.7 | Wuffs, scalar | Wuffs, AVX2 | scalar ÷ AVX2 |
|---|---:|---:|---:|---:|---:|
| `db4f698cb287295b07d5a501a6595f51` (116 MP) | 1749.93 | 1674.27 | 1854.34 | 1037.24 | 1.79× |
| `pinterest_1141451468122225667` (4.28 MP) | 63.28 | 61.91 | 62.38 | 44.09 | 1.41× |
| `pinterest_1141451468122730916` (1.01 MP) | 17.32 | 17.21 | 19.06 | 11.55 | 1.65× |
| `pinterest_1141451468122732060` (0.96 MP RGBA) | 18.23 | 17.93 | 20.20 | 11.97 | 1.69× |
| `pinterest_1141451468123671747` (2.46 MP RGBA) | 38.16 | 38.67 | 36.72 | 23.00 | 1.60× |
| `pinterest_1141451468123671931` (2.79 MP) | 21.30 | 21.05 | 18.25 | 11.09 | 1.65× |
| `pinterest_1141451468123718688` (2.07 MP palette) | 11.27 | 10.95 | 5.57 | 5.53 | 1.01× |
| `apng-rgb8-3frames` | 24.09 | 23.34 | 18.26 | 11.19 | 1.63× |
| `gray8-2mp` | 9.99 | 14.44 | 7.78 | 8.21 | 0.95× |
| `rgb16-3mp` | 45.43 | 73.09 | 62.73 | 62.21 | 1.01× |
| `rgb16-interlaced-3mp` | 62.12 | 75.22 | 64.81 | 65.31 | 0.99× |
| `rgb8-interlaced-3mp` | 39.45 | 28.54 | 34.74 | 29.07 | 1.20× |
| `rgba16-2mp` | 68.15 | 109.69 | 84.41 | 87.33 | 0.97× |
| `rgba8-interlaced-2mp` | 62.10 | 50.90 | 49.71 | 39.85 | 1.25× |
| `gray8-2mp-trns` | 12.73 | 22.30 | 26.93 | 26.63 | 1.01× |
| `palette-2mp-trns` | 12.22 | 11.44 | 5.59 | 5.55 | 1.01× |
| `rgb8-3mp-filter0` | 14.92 | 14.71 | 8.42 | 8.29 | 1.02× |
| `rgb8-3mp-trns` | 24.77 | 35.64 | 33.56 | 26.10 | 1.29× |
| `rgb8-4mp-filter0` | 68.10 | 66.49 | 45.15 | 44.88 | 1.01× |
| `real-big_tree-16bit` | 1040.10 | 2001.55 | 1787.13 | 1816.36 | 0.98× |
| `real-flower_foveon-16bit` | 133.14 | 173.75 | 160.48 | 159.04 | 1.01× |
| `real-leaves_iso_200-16bit` | 235.81 | 437.03 | 371.45 | 373.90 | 0.99× |
| `real-nightshot_iso_1600-16bit` | 273.75 | 434.48 | 401.27 | 393.10 | 1.02× |
| `real-flower_foveon-16bit-trns` | 140.28 | 213.30 | 160.80 | 162.97 | 0.99× |

The AVX2 unfilter is 1.4–1.8× faster on the 8-bit RGB and RGBA files whose rows are filtered, and 1.2–1.3× on the
interlaced 8-bit files and the 8-bit colour-key RGB file. It changes nothing where the pixels don't reach its SIMD
unfilter: the 16-bit files (6-byte pixels take the generic filter), the 8-bit grey files (1-byte pixels), the palette
file (a lookup), and the filter-0 files (their unfilter is a copy).

### The fork against Wuffs and png 0.18 (quiet pass 4)

Median ms per decode. Each file ran the Rust harness, then the C harness, back to back.

| Input | fork (app) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs, AVX2 | fork ÷ Wuffs |
|---|---:|---:|---:|---:|---:|---:|---:|
| 116 MP | 979.99 | 940.54 | 1210.07 | 1742.92 | 1764.64 | 1030.39 | 0.95× |
| RGBA 2.46 MP | 19.14 | 19.22 | 24.65 | 38.81 | 38.39 | 22.94 | 0.83× |
| RGB 4.28 MP | 35.92 | 35.97 | 59.63 | 64.15 | 64.11 | 43.83 | 0.82× |
| RGB 2.79 MP (unkeyed sibling of the key photo) | 10.54 | 10.64 | 15.03 | 21.67 | 21.34 | 11.10 | 0.95× |
| RGB 2.07 MP | 5.63 | 5.73 | 7.60 | 11.67 | 11.17 | 5.52 | 1.02× |
| RGB 1.01 MP | 9.08 | 9.11 | 13.56 | 18.03 | 17.72 | 11.46 | 0.79× |
| RGBA 0.96 MP | 10.00 | 9.93 | 12.70 | 18.85 | 18.50 | 11.93 | 0.84× |
| `rgb8-3mp-trns` (8-bit key) | 13.30 | 26.57 | 18.21 | 25.93 | 32.67 | 26.16 | 0.51× |
| `rgb8-interlaced-3mp` | 21.74 | 20.31 | 29.91 | 41.36 | 29.31 | 29.39 | 0.74× |
| `rgb8-4mp-filter0` | 44.56 | 44.36 | 47.97 | 71.57 | 120.46 | 78.81 | 0.57× |

- The 116 MP and 2.46 MP RGBA files are tied with png 0.18 within 4%.
- The 8-bit keyed file (`rgb8-3mp-trns`) costs the fork 13.30 ms, against 10.54 ms unkeyed in the same run: about 26%.
  Wuffs writes the key pixels black: 1,341,222 samples differ.
- The fork is faster than Wuffs (AVX2) on every 8-bit RGB and RGBA file here except the 2.07 MP one, where they tie.

### Colour keys

**Before the fix** (quiet pass 2, same run as the unkeyed sibling; pre-fix fork at `f7129d3`):

| File pair | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 |
|---|---:|---:|---:|---:|
| `rgb-3mp` → `rgb8-3mp-trns` (RGB key) | 10.99 → 30.51 (2.8×) | 10.87 → 25.13 (2.3×) | 15.38 → 18.15 (1.2×) | 22.61 → 24.97 (1.1×) |
| `gray8-2mp` → `gray8-2mp-trns` (grey key) | 5.79 → 29.50 (5.1×) | 5.72 → 24.47 (4.3×) | 8.26 → 9.21 (1.1×) | 10.03 → 12.67 (1.3×) |
| `palette-2mp` → `palette-2mp-trns` (palette alphas) | 5.56 → 5.47 (1.0×) | 5.57 → 5.38 (1.0×) | 8.33 → 7.67 (0.9×) | 11.73 → 12.23 (1.0×) |
| `real-flower_foveon` → `…-trns` (16-bit key) | 85.04 → 109.35 (1.3×) | 88.60 → 131.55 (1.5×) | 92.06 → 96.02 (1.0×) | 135.41 → 141.69 (1.05×) |

The inflate is not the difference: the key adds a chunk and leaves the IDAT stream alone. The streamed fdeflate measured
the same bytes at 5.49 ms (keyed) and 5.39 ms (unkeyed) on the RGB file, and 79.27 and 79.42 ms on the photograph.

The cost is in the expansion. The fork's expansion (`expand_trns_line` and `expand_trns_line16` in
`src/decoder/transform.rs` at `f7129d3`) handled one pixel at a time, with a slice copy and a key comparison. Each copy
and comparison was a library call (`copy_from_slice` to `memmove`/`memcpy`, slice `==` to `memcmp`). The saved VTune
profiles of the fork alone (`png-decoders profile`, 10 s a file) give:

| Profile | Expansion function, self | libc memory calls (`memmove`, `memcmp`, `memcpy`, `memset`) |
|---|---:|---:|
| keyed RGB 8-bit (`fork-rgb-trns`) | 2.8% | 45% |
| unkeyed RGB 8-bit (`fork-rgb`) | 0% | 5.5% |
| keyed grey 8-bit (`gray-trns`) | 4.2% | 50% |
| keyed 16-bit photograph (`flower-trns`) | 3.1% | 15% |

Shares are from one run each. Their absolute times were noisy, so only the shares are used. Palette tRNS is a lookup
table (`palette.rs`), which is why it costs nothing.

**The fix** (`2f176ed`, on `darktorres/image-png` `master` on top of `f7129d3`). `expand_trns_fixed<N, A>` reads the key once
per row as `[u8; N]` and copies and compares each pixel with fixed-size operations, so the loop makes no library calls.
The app's `Cargo.toml`, `fuzz/Cargo.toml` and the eval harness's `Cargo.toml` pin `2f176ed`; their lock files change
only that hash. The fork's own unit tests: 75 pass, and eight fail as they did before, on `tests/` fixtures the cargo
checkout doesn't include. `pg-codec` type-checks against it and its PNG tests pass.

The output is unchanged. The patched build matches the reference dumps on the 18 inputs that have one, the four tRNS
files included. The two 8-bit colour-key files that Wuffs differs on are unaffected: that difference is Wuffs' output.

Decodes in a fixed 4 s budget (`profile`, one file at a time, before and after; load not checked):

| File | Before | After |
|---|---:|---:|
| `real-flower_foveon-16bit-trns` (16-bit key) | 37, 38, 41 | 44, 45, 45 |
| `rgb8-3mp-trns` (8-bit key) | 172 | 307 |
| `gray16-2mp-trns` (16-bit grey key) | 176 | 364 |
| `gray8-2mp-trns` (8-bit grey key) | 206 | 553 |
| `rgb8-3mp-filter0` (no key: control) | 474 | 470 |

Under VTune, on the 8-bit RGB key the expansion and its libc calls fell from about 39% of samples to about 6% (2.27 s of
5.86 s against 0.35 s). On the 16-bit photograph they fell from about 14% to about 2.5%.

The same-run comparison after the fix (the eval crate on `2f176ed`, `time --secs 2`, load not checked):

| File | fork | png 0.18 | zune-png 0.5 |
|---|---:|---:|---:|
| `rgb8-3mp-trns` | 13.15 | 26.00 | 17.81 |
| `gray8-2mp-trns` | 7.07 | 19.90 | 8.98 |
| 16-bit colour-key photograph | 87.64 | 101.93 | 94.38 |
| `rgb8-3mp-filter0` (control) | 8.36 | 8.48 | 10.93 |

### Inflate

The question was whether zlib's inflate, which libpng and libspng use, explains their gap to the fork's fdeflate. The
streamed inflate (`inflate-stream` in both harnesses) feeds the IDAT stream in 16 KiB pieces into a 96 KiB window, which
is rolled back to its last 32 KiB when it fills. Each library reuses its state across decodes. An earlier single-call
measurement did not match the decodes and is superseded.

**8-bit and synthetic files, 5 s runs** (median ms per decode):

| Input | fdeflate | zlib | zlib ÷ fdeflate |
|---|---:|---:|---:|
| 116 MP RGB | 573.09 | 1124.00 | 1.96 |
| rgb-4mp | 30.72 | 49.86 | 1.62 |
| rgb-1mp | 6.21 | 12.26 | 1.97 |
| rgba-1mp | 7.46 | 14.33 | 1.92 |
| rgba-2mp | 14.23 | 30.82 | 2.17 |
| rgb-3mp | 5.18 | 16.69 | 3.22 |
| palette-2mp | 4.08 | 8.30 | 2.03 |
| rgb8-3mp-filter0 | 7.83 | 13.00 | 1.66 |
| rgb8-4mp-filter0 | 44.47 | 61.88 | 1.39 |
| rgb8-3mp-trns (key) | 5.30 | 14.23 | 2.68 |

The median across these ten rows is 1.97, with a range of 1.39–3.22.

**The four real photographs, 6 s runs:**

| Input | fdeflate | zlib | zlib ÷ fdeflate |
|---|---:|---:|---:|
| real-big_tree (27.7 MP) | 503.02 | 716.36 | 1.42 |
| real-flower_foveon (3.4 MP) | 79.42 | 104.22 | 1.31 |
| real-leaves_iso_200 (6.0 MP) | 110.44 | 152.73 | 1.38 |
| real-nightshot_iso_1600 (7.4 MP) | 148.98 | 201.74 | 1.35 |
| real-flower_foveon-16bit-trns (key) | 79.27 | 105.47 | 1.33 |

The median is 1.35. The earlier 2 s runs are not used: fdeflate was 21–33% slower in them on the photographs, which is
why their photograph ratios were 0.9–1.4.

**How much of libpng's gap is inflate** (8-bit files: quiet pass 2 decodes minus the 5 s streamed inflate):

| Input | fork − fdeflate | libpng − zlib | gap (libpng − fork) | of which inflate (zlib − fdeflate) |
|---|---:|---:|---:|---:|
| rgb-1mp | 9.46 − 6.21 = 3.3 | 17.73 − 12.26 = 5.5 | 8.3 | 6.1 (73%) |
| rgb-3mp | 10.99 − 5.18 = 5.8 | 22.61 − 16.69 = 5.9 | 11.6 | 11.5 (99%) |
| palette-2mp | 5.56 − 4.08 = 1.5 | 11.73 − 8.30 = 3.4 | 6.2 | 4.2 (68%) |
| rgb-4mp | 42.30 − 30.72 = 11.6 | 62.96 − 49.86 = 13.1 | 20.7 | 19.1 (93%) |
| rgba-1mp | 11.08 − 7.46 = 3.6 | 18.38 − 14.33 = 4.1 | 7.3 | 6.9 (94%) |

For the photographs (6 s inflate, quiet pass 2 decodes):

| Input | libpng gap | of which inflate | libpng non-inflate ÷ fork non-inflate | libspng non-inflate ÷ fork non-inflate |
|---|---:|---:|---:|---:|
| real-flower_foveon | 45.7 ms | 24.8 ms (54%) | 2.4 | 4.9 |
| real-leaves_iso_200 | 102.9 ms | 42.3 ms (41%) | 2.8 | 8.8 |
| real-nightshot_iso_1600 | 123.9 ms | 52.8 ms (43%) | 4.3 | 11.8 |
| real-big_tree | 352 ms | 213 ms (61%) | 1.7 | 6.7 |

Non-inflate is each decoder's decode minus its streamed inflate, so it includes unfiltering, row handling and output. It
is a difference of two runs, so the ratios are indicative.

What this supports:

- zlib inflates 1.4–3.2× more slowly than fdeflate on the 8-bit files (median 1.97) and 1.3–1.4× on the photographs
  (median 1.35), on the same stream.
- On the 8-bit files the inflate difference is most of libpng's gap to the fork (68–99%). On the photographs it is 41–61%.
  The rest of the photograph gap is libpng's own non-inflate work, at 1.7–4.3× the fork's. That is a 16-bit effect.
- libspng's extra cost is not in its inflate. It uses the same zlib 1.4, and its non-inflate work is 5–12× the fork's on
  the photographs. Its 16-bit synthetic files (2.6–4.8× the fork) point the same way.

**Filter-0 files and Wuffs.** On the 2.79 MP file the fork decodes the filter-0 version in 8.83 ms and the adaptive one
in 10.99 ms. Its streamed inflate is 7.83 ms (filter 0) and 5.18 ms (adaptive), so the fork's unfiltering of the adaptive
rows is about 6 ms. Wuffs' scalar build decodes the filter-0 file in 9.06 ms and the adaptive one in 18.43 ms. Its
inflate is close to fdeflate's, so its unfiltering on the adaptive rows is about 11 ms. That was the scalar build, and it
is revised below (the AVX2 build). Decoding RGB as RGBA changes Wuffs' time by 3–10%, and the RGBA output is 33% larger.

**The checksum.** zlib checks the zlib Adler-32 on every stream. The app's fork and png 0.18 skip it, and so does
Wuffs' PNG decoder. The inflate numbers above include it for zlib and Wuffs, so they overstate what the app pays.
Re-measured with `PNG_INFLATE_NO_CHECKSUM=1` (5 s, 8–23% load, the decodes from the same window):

| Input | zlib, checksum on / off | Wuffs, streamed, on / off | fdeflate, on / off |
|---|---:|---:|---:|
| rgb-3mp (adaptive, 8-bit) | 14.09 / 11.41 | 8.87 / 6.40 | 5.16 / 5.18 |
| rgb8-3mp-filter0 | 13.04 / 10.37 | 10.20 / 7.73 | 7.74 / 7.77 |
| real-flower_foveon (16-bit, colour key) | 104.41 / 97.85 | 82.44 / 76.48 | 76.36 / 76.66 |

Without the checksum zlib's inflate is 2.2× fdeflate's on the adaptive file, 1.3× on the filter-0 file and 1.3× on the
photograph (2.7×, 1.7× and 1.4× with it). The checksum is about a fifth of zlib's time on the 8-bit files and 6% on the
photograph. fdeflate's checksum costs nothing measurable, and Wuffs' inflate is 1.2×, 1.0× and 1.0× fdeflate's.

### Where the time goes (profiles)

VTune 2024.3, 10 s a file, one decoder at a time (`profile` in each harness). These are shares of each run's CPU time,
not times. The C decoders are profiled in a build with PDBs, with zlib, libpng and libspng also built with debug info
(`build\harness-prof` and `deps-prof`). The AVX2 Wuffs profile uses `build\harness-avx2`.

| Decoder, file | Inflate | Unfilter | Other decoder code | C runtime |
|---|---:|---:|---:|---:|
| fork, 2.79 MP RGB, adaptive | 41% | 36% | 15% | 8% |
| libpng, same | 44% | 49% | 3% | 4% |
| libspng, same | 46% | 50% | 1% | 3% |
| Wuffs, scalar build, same | 35% | 60% | 1% | 5% |
| Wuffs, AVX2 build, same | 56% | 28% | 9% | 8% |
| fork, 3.43 MP 16-bit photo | 79% | 6% | 12% | 2% |
| libpng, same | 82% | 12% | 2% | 4% |
| libspng, same | 66% | 34% | 0% | 1% |
| Wuffs, scalar build, same (plus 7% in the harness's repacking) | 42% | 36% | 10% | 5% |
| Wuffs, scalar build, filter-0 RGB | 86% | 1% | 0% | 13% |

- **libspng's 16-bit gap is its unfilter.** On the photo, libspng's paeth and row defilter take a third of its run (about
  60 ms of 177 ms), and libpng's take about 16 ms of 140. libspng has fast paths only for 3- and 4-byte pixels
  (`defilter_paeth3`, `defilter_sub4` and the like in `spng.c`). A 16-bit RGB pixel is 6 bytes, so it runs the generic
  per-byte paeth. This is the upstream library's behaviour, like zune-png's key.
- **Wuffs' unfilter.** The scalar build runs `filter_4_distance_3_fallback` (5.35 s of the scalar run). With the flag it
  runs `filter_4_distance_3_x86_sse42` (1.95 s), and the unfilter share falls from 60% to 28%, below the fork's 36%.
  Quiet pass 3 measures the same effect as 1.2–1.8× on the files with filtered 8-bit rows.
- **The 16-bit photo has no SIMD unfilter in either Wuffs build.** Its 6-byte pixels take the generic `filter_4` and
  `filter_and_swizzle_tricky` path, so its unfilter share is 36% in both.
- **libpng's photo unfilter is about twice the fork's share** (12% against 6%).

### Heap traffic

Each decoder, one decode after two warm-ups, so the buffers it keeps are at their size. "Calls" are heap calls during the
decode; "KiB" is the bytes they requested. The Rust numbers come from a counting global allocator over the process. The C
numbers come from hooks on each library's allocator (libpng's user allocator, which zlib's calls also go through; libspng's
context allocator; a counting `operator new` for the C++ side). The table is from the quiet passes except as noted.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs |
|---|---|---|---|---|---|---|
| rgb-1mp | 0 | 16 / 404 KiB | 11 / 9,996 KiB | 8 / 54 KiB | 5 / 48 KiB | 1 / 44 KiB |
| rgb-3mp | 0 | 15 / 404 KiB | 10 / 18,342 KiB | 8 / 57 KiB | 5 / 51 KiB | 1 / 44 KiB |
| rgb-4mp | 0 | 17 / 404 KiB | 13 / 41,415 KiB | 8 / 60 KiB | 7 / 55 KiB | 1 / 44 KiB |
| rgba-1mp | 0 | 18 / 404 KiB | 11 / 11,569 KiB | 8 / 55 KiB | 5 / 50 KiB | 1 / 44 KiB |
| rgba-2mp | 0 | 17 / 404 KiB | 12 / 27,425 KiB | 8 / 58 KiB | 5 / 53 KiB | 1 / 44 KiB |
| palette-2mp | 1 / 1 KiB | 19 / 407 KiB | 13 / 14,176 KiB | 10 / 57 KiB | 5 / 45 KiB | 1 / 44 KiB |
| rgb-116mp | 0 | 16 / 404 KiB | 13 / 1,204,074 KiB | 8 / 56 KiB | 5 / 50 KiB | 1 / 44 KiB |
| rgb16-3mp | 0 | 13 / 404 KiB | 5 / 33,908 KiB | 8 / 65 KiB | 5 / 59 KiB | 1 / 44 KiB |
| rgba16-2mp | 0 | 11 / 403 KiB | 5 / 42,421 KiB | 8 / 67 KiB | 5 / 62 KiB | 1 / 44 KiB |
| rgb16-interlaced-3mp | 0 | 14 / 412 KiB | 6 / 50,787 KiB | 8 / 65 KiB | 6 / 67 KiB | 1 / 44 KiB |
| rgb8-interlaced-3mp | 0 | 15 / 408 KiB | 6 / 25,932 KiB | 8 / 57 KiB | 6 / 55 KiB | 1 / 44 KiB |
| rgba8-interlaced-2mp | 0 | 14 / 408 KiB | 6 / 32,623 KiB | 8 / 58 KiB | 6 / 57 KiB | 1 / 44 KiB |
| gray8-2mp | 0 | 14 / 404 KiB | 6 / 11,306 KiB | 8 / 51 KiB | 5 / 46 KiB | 1 / 44 KiB |
| apng-rgb8-3frames | 0 | 16 / 422 KiB | 7 / 19,362 KiB | 8 / 57 KiB | 5 / 51 KiB | 1 / 44 KiB |
| palette-2mp-trns | 1 / 1 KiB | 20 / 407 KiB | 20 / 16,202 KiB | 12 / 59 KiB | 5 / 45 KiB | 1 / 44 KiB |
| gray8-2mp-trns | 1 / 0 KiB | 16 / 404 KiB | 7 / 14,029 KiB | 10 / 55 KiB | 5 / 46 KiB | 1 / 44 KiB |
| rgb8-3mp-trns | 1 / 0 KiB | 17 / 404 KiB | 18 / 21,068 KiB | 10 / 60 KiB | 5 / 51 KiB | 1 / 44 KiB |
| rgb8-3mp-filter0 | 0 | 13 / 404 KiB | 5 / 17,509 KiB | 8 / 57 KiB | 5 / 51 KiB | 1 / 44 KiB |
| rgb8-4mp-filter0 | 0 | 14 / 403 KiB | 5 / 34,910 KiB | 8 / 60 KiB | 5 / 54 KiB | 1 / 44 KiB |
| real-flower_foveon | 0 | 11 / 403 KiB | 5 / 55,387 KiB | 8 / 75 KiB | 5 / 70 KiB | 1 / 44 KiB |
| real-leaves_iso_200 | 0 | 12 / 403 KiB | 5 / 100,734 KiB | 8 / 84 KiB | 5 / 79 KiB | 1 / 44 KiB |
| real-nightshot_iso_1600 | 0 | 11 / 403 KiB | 5 / 123,213 KiB | 8 / 85 KiB | 5 / 80 KiB | 1 / 44 KiB |
| real-big_tree | 0 | 11 / 403 KiB | 5 / 466,743 KiB | 8 / 120 KiB | 5 / 115 KiB | 1 / 44 KiB |

- **Fork:** no calls in steady state. Its one call on the palette and tRNS files is a 1 KiB buffer.
- **png 0.18:** its inflate state and row buffers, about 400 KiB, allocated again each decode.
- **zune-png:** the output buffer plus its working memory, allocated each decode. Peak above the start was 942 MB on the
  116 MP file. Its call counts vary between runs (18 against 11 on the 1 MP file, say), while its bytes match. The
  figures here are from the high-load pass, so only the bytes are used.
- **libpng:** its read and info structures, zlib's inflate state and 32 KiB window, the 8 KiB zbuf, row buffers and the
  keep list. The extra calls on the palette and tRNS files (10–12, against 8) are its transform buffers.
- **libspng:** its context, zlib's state and window, and its text-chunk storage. The library files here are text-free, so
  the text storage is absent.
- **Wuffs:** one 44 KiB decoder object per call. Its pixel, work and repacking buffers are reused.
- **Pillow:** not measured. Its image buffers are allocated in C, outside any hook this harness can count.

### Output

Every decoder's output was compared with the app's fork, sample by sample, on every file. The reference is the fork's own
dump (`dump`), and the comparison is `compare` in each harness.

- **8-bit files** (the library, generated, palette, grey, RGB tRNS and filter-0 files): libpng, libspng, both Rust crates
  and Wuffs match the app, except Wuffs on the colour-key files. Pillow matches on every 8-bit file, the tRNS ones included.
- **Wuffs' colour-key difference.** On `rgb8-3mp-trns` (1,341,222 of 11,141,120 samples), `gray8-2mp-trns` (448,299 of
  5,570,560), Wuffs writes the colour of each fully transparent
  key pixel as black (0), where the app keeps the key colour (251 in these files). The alpha is the same in every sample.
- **16-bit files without a key** (the synthetic ones and the four photographs): the synthetic outputs are the source × 257
  exactly (checked by `make_inputs.py --verify-dump`), and the fork's dump equals the photographs' PPM samples. Every
  decoder matches the app on them, Wuffs included since its repacking. Pillow reads 16-bit at 8 bits, so it isn't compared.
- **16-bit colour-key photograph** (`real-flower_foveon-16bit-trns`, 13,716,864 samples):
  - libpng: identical to the app.
  - Wuffs: 3 samples differ (the key pixel's colour, written black; largest 12201). The alpha is identical.
  - zune-png 0.5: 1 sample differs, the alpha of the key pixel (largest 65535). zune-png expands 16-bit tRNS
    (`expand_trns::<true>`, `src/utils.rs`) while its samples are still big-endian, and converts to native order after
    (`src/decoder.rs`). The expansion compares the samples with the key's native-endian bytes (`to_ne_bytes()`), so on a
    little-endian host a key matches only if both bytes of every channel are equal. The flower's key (10887, 12201,
    10673) has unequal bytes in all three channels, so pixel (0, 0) stays opaque. This is upstream zune-png behaviour.
  - libspng: 9,343,104 samples differed, from a harness fault that is fixed. libspng writes `SPNG_FMT_RGBA16` (and `GA16`)
    in host order: it converts each 16-bit scanline to host order before those formats' conversion (`spng.c`, line 3247,
    for every format but RAW). The harness read the output as big-endian, so the colour samples came out byte-swapped.
    Alpha survived, since 0x0000 and 0xFFFF are unchanged by a swap. The count agrees exactly: a swap leaves 944,544 of the
    10,287,648 colour samples equal, so 9,343,104 differ. The harness now swaps those outputs to big-endian inside the
    timed decode (`harness/dec_libspng.cpp`), and libspng matches the app on the photograph.
- **16-bit grey colour key** (`gray16-2mp-trns`, 5,570,560 samples): libpng and libspng match, the GA16 path included.
  Wuffs differs in 448,299 samples (the key pixels, black). The Rust decoders match, zune-png too, because each sample is
  the 8-bit value times 257, so its key's two bytes are equal and the byte-order fault can't show.
- **APNG:** the decoders output the first frame, which is the library's image exactly (checked on the app's output).

The comparison was checked against a deliberately altered reference: one 8-bit sample gave "1 of … samples differ,
largest 1", and one 16-bit sample gave "largest 256", which confirms big-endian reading.

## Tooling

- `eval/png-decoders/src/` (Rust): the fork, png 0.18 and zune-png, with the counting allocator (`allocs`), `inflate`
  (fdeflate, one call over the whole stream), `inflate-stream` (streamed in 16 KiB pieces into a 96 KiB window), `time`,
  `compare`, `dump`, and `profile` (the fork alone, for a profiler).
- `eval/png-decoders/harness/` (C++, MSVC): libpng 1.6.59, libspng 0.7.4, Wuffs d28ecd4, zlib 1.4 (static, shared by libpng
  and libspng). Commands: `time`, `compare`, `allocs`, `inflate`, `inflate-stream`, `inflate-wuffs`, `dump`, and `profile`
  (one decoder, for a profiler). Built with `eval\png-decoders\win_build.py` and `cmake`; the Wuffs translation unit gets
  `/arch:AVX2` (its CMakeLists).
- `pillow_bench.py`: Pillow's reader, timed and compared against the app's dump.
- `make_inputs.py`, `make_variants.py` (the tRNS variants, the filter-0 variants, the text-free copies, the real-photo
  keys), `make_real16.py` (the four real photographs from the imagecompression.info zip).
- VTune: `amplxe-cl -collect hotspots` and `-report hotspots -format=csv`, user-mode sampling, no admin needed.

## Next

Nothing is open in this study.

Possible follow-ups, none needed for the decision:

- Move this report to `docs/reference` as a finished study, as the repo does for finished reports. The
  `JPEG_DECODERS.md` link above already points there. Not done, pending your go-ahead.
- A keyed 8-bit photograph from a source other than `rgb-3mp`. Every 8-bit keyed file so far is built from that one
  photograph, so a second source is needed to test the key cost on other real data. The 16-bit keyed photograph is
  the only other real keyed image.
