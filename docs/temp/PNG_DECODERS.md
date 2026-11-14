# PNG decoders

Which PNG decoder the app should use: the `png` fork that `pg-codec` calls, against the same crate from crates.io
(0.18.1), zune-png 0.5.2, libpng 1.6.59, libspng 0.7.4 and Wuffs (d28ecd4). The inputs are 25 PNGs: seven from the
library, seven made for the cases the library lacks (16-bit, Adam7-interlaced, grayscale, animated), five with a
tRNS transparency chunk (four synthetic, one a real 16-bit photograph), two with filter 0 on every row, and four real
16-bit photographs. Pillow's PNG reader is a second opinion outside Rust. It is the PNG counterpart of
[JPEG_DECODERS.md](../reference/JPEG_DECODERS.md). Everything runs natively on Windows (`eval/png-decoders`). Wall time, heap traffic
and the output are measured. No instruction counts (callgrind doesn't run here).

Four timing runs are used, and they differ in load. The **quiet pass 1** (2026-11-14, 20–32% CPU load) gives the first
table of the seven library files and the seven generated ones. The **quiet pass 2** (2026-11-14, 17–40% load) gives
all 23 files in one run, and the colour-key comparison. The **high-load pass** (83–99% load, other sessions running)
gives the rest, and its absolute times are unreliable. The **streamed-inflate runs** (7–28% load) time the inflate of
each file's stream alone, in both libraries. Within each run the decoders ran back to back, so the ratios inside a run
are the useful part. Absolute times move by 20–40% between runs, and the 116 MP file moved 24% (988.77 ms in pass 1,
1229.55 ms in pass 2).

## Verdict

- **Output:** every decoder matches the app's output sample for sample on every file, with two exceptions. Wuffs
  writes the colour of a fully transparent colour-key pixel as black, where the app keeps the key colour; the alpha is
  the same. On the 16-bit colour-key photograph, zune-png gets one sample wrong (the alpha of the key pixel): it compares the key with its samples while they are still big-endian, using native-endian bytes, so a key whose bytes differ never matches. libspng's 16-bit output was wrong because of the harness, not libspng: libspng writes RGBA16 in host order, and the harness read it as big-endian. The harness is fixed, and libspng now matches the app on that photograph.
  Wuffs' 16-bit output is repacked to the app's layout, inside the timed decode, so it is compared on 16-bit files too.
  Pillow matches the app on every 8-bit file, including the tRNS ones, and keeps the key colour as the app does.
- **Speed:** the fork and png 0.18 are within 10% of each other on every file in quiet pass 1, and on 18 of the 23
  files in quiet pass 2. The exceptions in pass 2 are the 116 MP file (png 0.18 61% slower), the 2.46 MP RGBA file
  (png 0.18 26% faster; it was tied in pass 1), and the two 8-bit colour-key files (png 0.18 17–18% faster) and the
  filter-0 file (png 0.18 13% slower). The fork is not slower than the crate it's forked from, apart from the
  colour-key cases and the two files above. The other decoders are slower: libpng 1.3–3.0× the fork on the unkeyed files in pass 2 (1.5–2.1× on 8-bit
  RGB, 2.4–3.0× on the 16-bit synthetics, 1.5–1.7× on the photographs); libspng 1.4–2.5× on 8-bit, 2.6–4.8× on the
  16-bit synthetics, and 1.9–3.1× on the photographs.
- **Colour keys (confirmed in pass 2):** the fork decodes an 8-bit RGB file with a colour key 2.8× slower than the same
  image without one, and a grey file with a key 5.1× slower. On the 16-bit photograph the fork is 1.3× slower with its
  key. Palette tRNS costs nothing. libpng pays 1.1–1.3× on the 8-bit files and 1.05× on the photograph. The inflate is
  not the cause: the key adds a chunk and leaves the compressed stream alone, and the inflate of the keyed and unkeyed
  photograph measured the same (79.3 and 79.4 ms). The cost is in the expansion. The fork's expansion (`expand_trns_line`
  in `src/decoder/transform.rs` at `f7129d3`, read but not profiled) compares each pixel's colour with the key one
  pixel at a time. png 0.18 expands the keys faster than the fork (25.1 against 30.5 ms on the RGB file). This is the
  one result in the study that points to a change in the app. The fix has not been designed, because no profile has
  been taken.
- **Inflate:** zlib (libpng and libspng) inflates the same stream 1.3–2.0× more slowly than fdeflate (the fork's
  inflater). On the 8-bit files that difference accounts for 68–99% of libpng's gap to the fork; on the photographs
  for 41–61%. On the photographs the rest is libpng's own per-row work, at 1.7–4.3× the fork's. libspng's non-inflate
  work is 5–12× the fork's on the photographs. Neither is profiled. See the inflate section. The app's fork skips the zlib checksum that libpng and zlib check; without it, zlib's inflate is 1.3–2.2× fdeflate's, and Wuffs' is about fdeflate's.
- **Heap traffic:** the fork makes no heap calls per decode once its buffers are at size (its `Scratch` holds the
  inflate state and the row buffers). png 0.18 makes 11–20 calls (about 400 KiB, its inflate state) per decode. The C
  decoders make 5–12 calls (45–120 KiB), and Wuffs one 44 KiB call. zune-png hands back a new output buffer each
  decode, so it requests the whole image every time: 1.2 GB on the 116 MP file.
- **Nothing to change yet.** The colour-key expansion is the only finding that points at the app's code, and it is
  confirmed in cost, not in cause. The other gaps are between libraries, not in the app.

## What was measured

Each decoder decodes the file's bytes to packed samples on one thread, full size. The output buffer is reused across
decodes, as the app reuses its output. Each time is the median over at least five decodes, after two warm-up decodes,
for at least one or two seconds.

- **png (app fork):** the fork at `f7129d3`, decoded as `Codecs::png` does: its scratch kept between files, text and
  ICC chunks skipped, EXPAND (16-bit kept), the app's 512 MiB limit.
- **png 0.18:** crates.io's 0.18.1 with the same options and limit, and no scratch. It uses the app's patched
  `fdeflate` (the root `Cargo.toml` patches every user of fdeflate, and the fork needs it), so it isn't the stock
  crate.
- **zune-png 0.5.2:** the zune-image family's decoder, with zune-core 0.5.3 from crates.io. Its height cap is raised
  to 2^20. Its API returns a new buffer each decode, so its time includes that allocation.
- **libpng 1.6.59** (static, with zlib 1.4): the standard read API, EXPAND-equivalent transforms, 16-bit kept. It
  creates its read structures on each decode, which is how libpng is normally used. Text chunks are skipped.
- **libspng 0.7.4** (static, with the same zlib 1.4): its raw output with the palette and tRNS expansions. It creates
  its context on each decode. It cannot skip text chunks, so the library files were run as copies with their tEXt,
  zTXt, iTXt and iCCP chunks removed (`make_variants.py`), for every decoder. The pixels are unchanged, and the fork
  and libpng skip those chunks anyway.
- **Wuffs d28ecd4:** `wuffs_aux::DecodeImage`, to the nearest of Wuffs' output formats. For 8-bit gray, RGB and RGBA
  that is the app's layout. For 8-bit gray+alpha (Wuffs' PNG decoder won't write YA_NONPREMUL for it) it decodes to
  RGBA and keeps the gray and alpha. For 16-bit it decodes to BGRA_NONPREMUL_4X16LE, its own 16-bit format, and
  repacks: samples swapped to big-endian and reordered, the alpha dropped for RGB. The repacking is in the timed decode,
  since that is what the app's layout costs with Wuffs. Wuffs recreates its decoder object on each call (about 44 KiB),
  and keeps its pixel and work buffers.
- **Pillow 12.2.0 (second opinion, `pillow_bench.py`):** Pillow's own PNG reader, a `zip` decoder over zlib-ng 1.3.1,
  not libpng. Its time covers `Image.open`, `load` and any conversion, so it includes Pillow's object overhead as well
  as the decode. A grey or RGB colour key is converted to LA or RGBA, as the app gives.

## Inputs

**The library's sample** (294 PNGs in `datasets/pinterest` and `equiv/src`, with copies). All of them are 8-bit: 188
RGB, 104 RGBA and 2 palette. The seven below are the nearest to the targets by megapixels, with the largest library
PNG as the last one.

| Input | Kind | Size | File |
|---|---|---|---|
| `rgb-1mp` | RGB 8-bit | 1.01 MP, 1.7 MB | `pinterest_1141451468122730916` |
| `rgb-3mp` | RGB 8-bit | 2.79 MP, 1.0 MB | `pinterest_1141451468123671931` |
| `rgb-4mp` | RGB 8-bit | 4.28 MP, 5.8 MB | `pinterest_1141451468122225667` |
| `rgba-1mp` | RGBA 8-bit | 0.96 MP, 1.5 MB | `pinterest_1141451468122732060` |
| `rgba-2mp` | RGBA 8-bit | 2.46 MP, 3.0 MB | `pinterest_1141451468123671747` |
| `palette-2mp` | palette | 2.07 MP, 0.8 MB | `pinterest_1141451468123718688` |
| `rgb-116mp` | RGB 8-bit | 116.14 MP, 142 MB | `_incoming/db4f698cb287297b07d5a501a6595f51` |

**Made for the cases the sample lacks** (`eval/png-decoders/make_inputs.py`, written under `$PG_ROOT\eval\png-decoders\inputs`).
The source is a library image, so the pixels are the library's. The encoder is ours because Pillow can't write 16-bit
RGB(A), interlaced or APNG files. Each row uses the least-residual filter, at zlib level 6.

| Input | Kind | Size | Made from |
|---|---|---|---|
| `rgb16-3mp` | RGB 16-bit | 2.79 MP, 1.3 MB | `rgb-3mp`, × 257 |
| `rgba16-2mp` | RGBA 16-bit | 2.46 MP, 4.0 MB | `rgba-2mp`, × 257 |
| `rgb16-interlaced-3mp` | RGB 16-bit, Adam7 | 2.79 MP, 1.9 MB | `rgb-3mp`, × 257 |
| `rgb8-interlaced-3mp` | RGB 8-bit, Adam7 | 2.79 MP, 1.5 MB | `rgb-3mp` |
| `rgba8-interlaced-2mp` | RGBA 8-bit, Adam7 | 2.46 MP, 3.8 MB | `rgba-2mp` |
| `gray8-2mp` | grayscale 8-bit | 2.79 MP, 0.4 MB | `rgb-3mp`, luminance (0.299, 0.587, 0.114) |
| `apng-rgb8-3frames` | RGB 8-bit, APNG | 3 frames of 2.79 MP, 3.1 MB | `rgb-3mp`; frame 0 is the default image, frames 1–2 are shifted and flipped copies |

The 16-bit inputs written here are synthetic: each sample is the 8-bit value × 257, so its high and low bytes are
equal. That compresses better than real 16-bit data, so their inflate is cheaper than a photograph's (see the real
photographs below).

**Chunk variants** (`make_variants.py`, under `$PG_ROOT\eval\png-decoders\inputs-variants`). The pixels are never
re-encoded; a chunk is added or the file is copied.

| Input | Kind | Size | Made from |
|---|---|---|---|
| `palette-2mp-trns` | palette, tRNS | 2.07 MP, 0.8 MB | `palette-2mp`, one alpha per palette entry (they vary) |
| `rgb8-3mp-trns` | RGB 8-bit, colour key | 2.79 MP, 1.0 MB | `rgb-3mp`, key = the colour of pixel (0, 0) |
| `gray8-2mp-trns` | grayscale 8-bit, key | 2.79 MP, 0.4 MB | `gray8-2mp`, key = the grey of pixel (0, 0) |
| `gray16-2mp-trns` | grayscale 16-bit, key | 2.79 MP, 0.6 MB | `gray8-2mp` × 257, key = pixel (0, 0)'s grey |
| `rgb8-3mp-filter0` | RGB 8-bit, filter 0 | 2.79 MP | `rgb-3mp`, filter type 0 on every row |
| `rgb8-4mp-filter0` | RGB 8-bit, filter 0 | 4.28 MP | `rgb-4mp`, filter type 0 on every row |
| `real-flower_foveon-16bit-trns` | RGB 16-bit, colour key | 3.43 MP, 15.6 MB | `real-flower_foveon`, key = the colour of pixel (0, 0), as 16-bit big-endian R, G, B |

The filter-0 variants make unfiltering a copy, so a decoder's whole time on them is almost all inflate. They are the
check on the inflate question below.

**Real photographs** (`make_real16.py`, under `$PG_ROOT\eval\png-decoders\inputs-real16`). The 16-bit RGB set of
[imagecompression.info](https://www.imagecompression.info/test_images), "The New Image Compression Test Set" (Sachin
Garg, 2008), whose terms allow redistribution with attribution. The archive holds 16-bit PPMs, which were read from the
zip and written as 16-bit PNGs with the same encoder. The four chosen span 3.4 to 27.7 MP.

| Input | Size | PNG size |
|---|---|---|
| `real-flower_foveon` | 2268 × 1512 (3.43 MP) | 15.6 MB |
| `real-leaves_iso_200` | 3008 × 2000 (6.02 MP) | 31.0 MB |
| `real-nightshot_iso_1600` | 3136 × 2352 (7.38 MP) | 37.7 MB |
| `real-big_tree` | 6088 × 4550 (27.70 MP) | 145.5 MB |

These are the inputs the synthetic 16-bit files stood in for. Their PNGs are about 10× the synthetic 16-bit files' size
per megapixel (about 5 MB per MP against 0.5 MB per MP), which is why their inflate cost differs from those.

**Text-free library copies** (`inputs-notext`): the seven library files with their text chunks removed, used for all
decoders in the matrix (see libspng above).

## Decode time

### Quiet pass 1 (the seven library files and the seven generated ones)

Median ms per decode, one thread, full size. The Rust and C columns come from one run each, back to back. Pillow is
from its own run, in the same session. Machine load 20–32%.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs | Pillow 12 |
|---|---:|---:|---:|---:|---:|---:|---:|
| rgb-1mp | **9.44** | 9.53 | 13.69 | 18.64 | 18.94 | 19.24 | 23.26 |
| rgb-3mp | **11.02** | 11.39 | 15.77 | 22.21 | 21.86 | 18.72 | 24.86 |
| rgb-4mp | 41.14 | 40.26 | 62.29 | 69.60 | 66.07 | 67.92 | 79.16 |
| rgba-1mp | 10.64 | 10.49 | 12.49 | 20.31 | 19.92 | 22.04 | 25.73 |
| rgba-2mp | **20.15** | 20.56 | 25.64 | 40.45 | 40.88 | 39.59 | 50.07 |
| palette-2mp | **5.62** | 5.61 | 7.63 | 12.32 | 11.41 | 5.85 | 11.32 |
| rgb-116mp | **988.77** | 1012.37 | 1335.29 | 1815.83 | 1829.23 | 1965.26 | 2336.43 |
| rgb16-3mp | **15.97** | 17.18 | 21.06 | 47.11 | 74.42 | 118.34* | n/a |
| rgba16-2mp | **28.05** | 28.91 | 32.52 | 71.61 | 133.81 | 177.80* | n/a |
| rgb16-interlaced-3mp | **26.59** | 29.04 | 43.05 | 78.06 | 77.60 | 129.69* | n/a |
| rgb8-interlaced-3mp | **23.42** | 24.34 | 31.45 | 42.34 | 30.68 | 38.40 | 38.66 |
| rgba8-interlaced-2mp | 33.62 | 32.58 | 41.89 | 80.36 | 60.57 | 55.06 | 59.64 |
| gray8-2mp | **5.90** | 5.79 | 8.38 | 10.48 | 14.41 | 8.50 | 9.07 |
| apng-rgb8-3frames | **12.13** | 12.07 | 17.05 | 24.91 | 23.88 | 18.76 | 24.58 |

Bold: the fastest of the Rust decoders, or tied within 1%. n/a: Pillow reads 16-bit RGB(A) at 8 bits. \*: the Wuffs
figures for the 16-bit files were taken before its repacking was in the timed decode, so they are from the high-load
pass and carry its load; quiet pass 2 has the repacked figures.

### Quiet pass 2 (all 23 files, one run, 17–40% load)

Median ms per decode, one thread, full size, every decoder back to back on each file. Taken from
`quiet_table.md`. Pillow was not run in this pass.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs |
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
| gray8-2mp-trns | 29.50 | 24.47 | 9.21 | 12.67 | 21.99 | 29.08 |
| palette-2mp-trns | 5.47 | 5.38 | 7.67 | 12.23 | 11.47 | 5.78 |
| rgb8-3mp-filter0 | 8.83 | 9.96 | 11.40 | 14.88 | 14.60 | 9.06 |
| rgb8-3mp-trns | 30.51 | 25.13 | 18.15 | 24.97 | 37.48 | 35.15 |
| rgb8-4mp-filter0 | 49.76 | 48.43 | 48.74 | 68.64 | 67.73 | 47.26 |
| real-big_tree | 689.27 | 645.30 | 675.52 | 1041.11 | 1969.38 | 1914.94 |
| real-flower_foveon | 94.27 | 88.47 | 92.15 | 139.92 | 176.85 | 166.98 |
| real-leaves_iso_200 | 143.90 | 140.12 | 147.31 | 246.75 | 446.07 | 415.47 |
| real-nightshot_iso_1600 | 170.25 | 171.15 | 172.02 | 294.23 | 453.32 | 435.86 |

The 116 MP file moved 24% from pass 1 to pass 2 for the fork, and the 2.46 MP RGBA file moved 44%. Single-file figures
carry about that much uncertainty. Ratios between decoders on one file within one pass are the reliable part.

### High-load pass (the rest of the inputs, and the same files again)

Load 83–99% for the whole pass, so the absolute times are higher than the quiet passes' (the fork's 1 MP decode reads
15.7 ms here and 9.4 ms there). Within a file the decoders ran back to back, so the ratios are the useful part.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs |
|---|---:|---:|---:|---:|---:|---:|
| rgb-1mp (for reference) | 15.65 | 19.58 | 23.77 | 29.33 | 27.01 | 27.96 |
| rgb-3mp (for reference) | 20.75 | 27.31 | 30.47 | 29.06 | 29.24 | 26.67 |
| rgb-4mp (for reference) | 73.04 | 69.51 | 101.50 | 95.53 | 105.43 | 92.61 |
| palette-2mp (for reference) | 10.81 | 9.42 | 13.53 | 18.65 | 16.24 | 9.57 |
| rgb8-3mp-filter0 | 13.48 | 14.33 | 20.41 | 20.06 | 19.23 | 13.10 |
| rgb8-4mp-filter0 | 75.32 | 76.64 | 87.38 | 90.22 | 90.44 | 66.93 |
| palette-2mp-trns | 13.10 | 9.34 | 16.46 | 16.52 | 15.34 | 9.05 |
| gray8-2mp-trns | 50.74 | 35.85 | 16.07 | 17.60 | 31.71 | 44.53 |
| rgb8-3mp-trns | 45.04 | 47.99 | 36.48 | 32.68 | 58.87 | 53.52 |
| real-flower_foveon (3.43 MP, 16-bit) | 130.94 | 141.81 | 140.38 | 192.99 | 307.73 | 221.21 |
| real-leaves_iso_200 (6.02 MP, 16-bit) | 241.77 | 281.06 | 310.29 | 329.38 | 662.46 | 500.83 |
| real-nightshot_iso_1600 (7.38 MP, 16-bit) | 314.38 | 322.72 | 429.68 | 430.37 | 681.37 | 575.00 |
| real-big_tree (27.70 MP, 16-bit) | 1136.46 | 1087.93 | 1083.64 | 1547.85 | 2730.67 | 2656.91 |

The palette and gray tRNS rows here are not trusted: the palette-trns fork figure (13.10 ms) is 2.4× the same file's
time in quiet pass 2 (5.47 ms), which is the sort of error the load causes. The high-load pass is kept for correctness
and as a second look at the ratios.

### Colour-key transparency (quiet pass 2, same run, and the 16-bit photograph)

Each keyed file was timed with its unkeyed sibling in the same run. The first table's rows come from quiet pass 2; the
photograph's come from its own run, with the unkeyed photograph in the same run.

| File pair | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 |
|---|---:|---:|---:|---:|
| `rgb-3mp` → `rgb8-3mp-trns` (RGB colour key) | 10.99 → 30.51 (2.8×) | 10.87 → 25.13 (2.3×) | 15.38 → 18.15 (1.2×) | 22.61 → 24.97 (1.1×) |
| `gray8-2mp` → `gray8-2mp-trns` (grey colour key) | 5.79 → 29.50 (5.1×) | 5.72 → 24.47 (4.3×) | 8.26 → 9.21 (1.1×) | 10.03 → 12.67 (1.3×) |
| `palette-2mp` → `palette-2mp-trns` (palette alphas) | 5.56 → 5.47 (1.0×) | 5.57 → 5.38 (1.0×) | 8.33 → 7.67 (0.9×) | 11.73 → 12.23 (1.0×) |
| `real-flower_foveon` → `…-trns` (16-bit colour key) | 85.04 → 109.35 (1.3×) | 88.60 → 131.55 (1.5×) | 92.06 → 96.02 (1.0×) | 135.41 → 141.69 (1.05×) |

The inflate is not the difference. The key adds a chunk and leaves the IDAT stream alone, so the same bytes are inflated.
The streamed fdeflate measured them at 5.49 ms (keyed) and 5.39 ms (unkeyed) on the RGB file, and 79.27 and 79.42 ms
on the photograph. So the extra time is the expansion. The fork's expansion (`expand_trns_line` in
`src/decoder/transform.rs` at `f7129d3`) was read, not profiled: it handles one pixel at a time, with a slice copy and a
key comparison. Palette tRNS is a lookup table (`palette.rs`), which is why it costs nothing. The rgb8 key in the fork
costs about 19.5 ms on 2.79 MP, which is roughly 7 ns a pixel; the gray key about 24 ms, roughly 8.5 ns a pixel. Those
are arithmetic on the table, not a measurement of the loop.

Two results here are in the decoders, not the timing. zune-png leaves the key pixel opaque on the photograph, which its key compare explains (see Output), so its key time (1.04×) is close to unkeyed. libpng and libspng keep the key colour as the fork does; Wuffs writes it black.

Rerun on a quiet machine (15–17% load, 5 s): fork 86.95 → 102.56 ms (1.18×), png 0.18 84.28 → 105.52 (1.25×), zune-png 89.87 → 95.57 (1.06×), libpng 131.01 → 139.80 (1.07×), libspng 173.54 → 208.31 (1.20×), Wuffs 159.41 → 168.19 (1.06×). The earlier quiet pass gave the fork 1.29×, so the fork's cost on this photograph is 1.2–1.3× across the two runs.

## Inflate (fdeflate and zlib on the same streams)

The question was whether zlib's inflate, which libpng and libspng use, explains their gap to the fork's fdeflate. It
is measured two ways, and the streamed one is the one used here. The earlier single-call numbers (one call over the
whole stream into a buffer of the whole image) did not match the fork's whole decode and are superseded.

The **streamed inflate** (`inflate-stream` in both harnesses) feeds the IDAT stream in 16 KiB pieces and inflates into
a 96 KiB window, which is rolled back to its last 32 KiB of history when it fills (fdeflate needs that history; zlib
keeps its own). Each library reuses its state across decodes. The median ms per decode, reused state:

**8-bit and synthetic files** (runs at 2 s and 5 s; the 2 s run covers all the rows, the 5 s run the ones marked):

| Input | fdeflate, 2 s | zlib, 2 s | zlib ÷ fdeflate, 2 s | fdeflate, 5 s | zlib, 5 s | zlib ÷ fdeflate, 5 s |
|---|---:|---:|---:|---:|---:|---:|
| 116 MP RGB | 606.93 | 1541.53 | 2.54 | 573.09 | 1124.00 | 1.96 |
| rgb-4mp | 27.12 | 64.62 | 2.38 | 30.72 | 49.86 | 1.62 |
| rgb-1mp | 6.17 | 13.16 | 2.13 | 6.21 | 12.26 | 1.97 |
| rgba-1mp | 7.52 | 15.37 | 2.04 | 7.46 | 14.33 | 1.92 |
| rgba-2mp | 14.43 | 34.96 | 2.42 | 14.23 | 30.82 | 2.17 |
| rgb-3mp | 5.39 | 15.32 | 2.84 | 5.18 | 16.69 | 3.22 |
| palette-2mp | 4.28 | 7.87 | 1.84 | 4.08 | 8.30 | 2.03 |
| apng-rgb8-3frames | 6.27 | 16.65 | 2.66 | | | |
| gray8-2mp | 2.57 | 5.81 | 2.26 | | | |
| rgb16-3mp | 8.53 | 26.94 | 3.16 | | | |
| rgba16-2mp | 22.17 | 56.28 | 2.54 | | | |
| rgb8-3mp-filter0 | 8.88 | 15.19 | 1.71 | 7.83 | 13.00 | 1.66 |
| rgb8-4mp-filter0 | 62.65 | 61.54 | 0.98 | 44.47 | 61.88 | 1.39 |
| rgb8-3mp-trns (key) | 5.49 | 14.33 | 2.61 | 5.30 | 14.23 | 2.68 |
| palette-2mp-trns | 4.10 | 7.67 | 1.87 | | | |
| gray8-2mp-trns | 2.52 | 5.66 | 2.25 | | | |

**The four real photographs** (6 s runs, the ones the 2 s run also covered in the last two columns):

| Input | fdeflate, 6 s | zlib, 6 s | zlib ÷ fdeflate, 6 s | fdeflate, 2 s | zlib, 2 s | zlib ÷ fdeflate, 2 s |
|---|---:|---:|---:|---:|---:|---:|
| real-big_tree (27.7 MP) | 503.02 | 716.36 | 1.42 | 633.51 | 694.79 | 1.10 |
| real-flower_foveon (3.4 MP) | 79.42 | 104.22 | 1.31 | 104.91 | 107.48 | 1.02 |
| real-leaves_iso_200 (6.0 MP) | 110.44 | 152.73 | 1.38 | 147.16 | 159.94 | 1.09 |
| real-nightshot_iso_1600 (7.4 MP) | 148.98 | 201.74 | 1.35 | 223.78 | 211.25 | 0.94 |
| real-flower_foveon-16bit-trns (key) | 79.27 | 105.47 | 1.33 | 84.13 | 178.60 | 2.12 |

**How much of each gap is inflate.** Quiet pass 2's full decodes, minus the 5 s streamed inflate of the same file,
for the 8-bit files. The gap is libpng's full decode minus the fork's. The inflate part is zlib's minus fdeflate's:

| Input | fork − fdeflate | libpng − zlib | gap (libpng − fork) | of which inflate (zlib − fdeflate) |
|---|---:|---:|---:|---:|
| rgb-1mp | 9.46 − 6.21 = 3.3 | 17.73 − 12.26 = 5.5 | 8.3 | 6.1 (73%) |
| rgb-3mp | 10.99 − 5.18 = 5.8 | 22.61 − 16.69 = 5.9 | 11.6 | 11.5 (99%) |
| palette-2mp | 5.56 − 4.08 = 1.5 | 11.73 − 8.30 = 3.4 | 6.2 | 4.2 (68%) |
| rgb-4mp | 42.30 − 30.72 = 11.6 | 62.96 − 49.86 = 13.1 | 20.7 | 19.1 (93%) |
| rgba-1mp | 11.08 − 7.46 = 3.6 | 18.38 − 14.33 = 4.1 | 7.3 | 6.9 (94%) |

For the photographs the same comparison, with the 6 s inflate and quiet pass 2's decodes:

| Input | libpng gap | of which inflate | libpng non-inflate ÷ fork non-inflate | libspng non-inflate ÷ fork non-inflate |
|---|---:|---:|---:|---:|
| real-flower_foveon | 45.7 ms | 24.8 ms (54%) | 35.7 ÷ 14.9 = 2.4 | 72.6 ÷ 14.9 = 4.9 |
| real-leaves_iso_200 | 102.9 ms | 42.3 ms (41%) | 94.0 ÷ 33.5 = 2.8 | 293.3 ÷ 33.5 = 8.8 |
| real-nightshot_iso_1600 | 123.9 ms | 52.8 ms (43%) | 92.5 ÷ 21.3 = 4.3 | 251.6 ÷ 21.3 = 11.8 |
| real-big_tree | 352 ms | 213 ms (61%) | 325 ÷ 186 = 1.7 | 1253 ÷ 186 = 6.7 |

Non-inflate is each decoder's quiet-pass-2 decode minus its streamed inflate, so it includes unfiltering, row handling
and the output, and it is a difference of two runs. Treat the ratios as indicative.

What this supports:

- **zlib inflates 1.3–2.0× more slowly than fdeflate on the same stream** (median 1.97 across the ten 8-bit rows at 5 s;
  median 1.35 across the photographs at 6 s). Those ratios are the inflate result, and it is not a single-file claim.
- **On the 8-bit files, the inflate difference is most of libpng's gap to the fork** (68–99%). rgb-3mp is the clearest:
  the inflate difference of 10–12 ms covers the whole 11.6 ms gap. So for those files the libpng gap is mainly zlib.
- **On the photographs, inflate is 41–61% of libpng's gap.** The rest is libpng's own non-inflate work, at 1.7–4.3× the
  fork's. This is a 16-bit effect: it is not in the 8-bit ratios.
- **libspng's extra cost is not in its inflate.** It uses the same zlib 1.4, so its inflate matches libpng's, and its
  non-inflate work is 5–12× the fork's on the photographs. The cause was not located; it is outside zlib. Its 16-bit
  synthetic files (2.6–4.8× the fork) point the same way.

The 2 s runs are the least reliable. fdeflate was 21–33% slower on the photographs in the 2 s run than in the 6 s run,
while zlib was within 5%, which is why the 2 s photograph ratios were 0.9–1.1 and the 6 s ones 1.3–1.4. On the 4.28 MP
filter-0 file fdeflate moved from 62.7 ms (2 s) to 44.5 ms (5 s). A single ratio from one run carries about ±30%; the
medians across files are the usable figures.

**Filter-0 files and Wuffs.** The filter-0 files make unfiltering a copy, so their whole decode is almost all inflate.
On the 2.79 MP file, the fork decodes the filter-0 version in 8.83 ms, against its adaptive version's 10.99 ms; the
fork's streamed inflate of the two streams is 7.83 ms (filter 0) and 5.18 ms (adaptive), so the fork's unfiltering of
the adaptive rows is about 6 ms. Wuffs decodes the filter-0 file in 9.06 ms and the adaptive one in 18.43 ms. Its own inflate, measured below,
is about fdeflate's, so its unfiltering on the adaptive rows is about 11 ms, roughly 2.4× the fork's (about 5 ms). Its RGB output path is not the cause. Decoding RGB as
RGBA changes Wuffs' time by 3–10% (18.86 → 19.53 ms on 1.01 MP, 18.57 → 20.52 on 2.79 MP, 65.43 → 67.59 on 4.28 MP;
the RGBA output is 33% larger).

**The checksum, and Wuffs' inflate.** zlib checks the zlib Adler-32 on every stream. The app's fork and png 0.18 skip
it (png's `ignore_adler32` is on by default), and Wuffs' PNG decoder sets its checksum quirk to ignore it too. The
inflate numbers above include the checksum for zlib and for Wuffs, so they overstate what the app pays. Re-measured with
`PNG_INFLATE_NO_CHECKSUM=1` (the app's way), in one window at 8–23% load, 5 s, and the decodes below from the same run:

| Input | zlib, checksum on / off | Wuffs, streamed, on / off | fdeflate, on / off |
|---|---:|---:|---:|
| rgb-3mp (adaptive, 8-bit) | 14.09 / 11.41 | 8.87 / 6.40 | 5.16 / 5.18 |
| rgb8-3mp-filter0 | 13.04 / 10.37 | 10.20 / 7.73 | 7.74 / 7.77 |
| real-flower_foveon (16-bit, colour key) | 104.41 / 97.85 | 82.44 / 76.48 | 76.36 / 76.66 |

Without the checksum, zlib's inflate is 2.2× fdeflate's on the adaptive file, 1.3× on the filter-0 file and 1.3× on the
photograph (2.7×, 1.7× and 1.4× with it). The checksum is about a fifth of zlib's time on the 8-bit files and 6% on the
photograph. fdeflate's checksum costs nothing measurable. Wuffs' inflate is close to fdeflate's: 1.2×, 1.0× and 1.0×.

The decodes in the same window (libpng, libspng and Wuffs, the full decode) are the reference for the Wuffs unfiltering
figure in the filter-0 paragraph above. Wuffs' decode of the adaptive file (18.30 ms) less its inflate (6.40 ms) leaves
about 12 ms. The filter-0 file's decode (8.45 ms) less its inflate (7.73 ms) leaves about 1 ms. So its unfiltering of the
adaptive rows is about 11 ms. The fork's unfiltering, from quiet pass 2 and fdeflate's inflate, is 10.99 − 5.18 = 5.8 ms on
the adaptive file and 8.83 − 7.77 = 1.1 ms on filter 0, so about 4.7 ms, and Wuffs' is about 2.4× that. The decodes come
from a different run than the inflates, so this is indicative.

## Heap traffic

Each decoder, one decode after two warm-ups, so the buffers it keeps are at their size. "Calls" are heap calls made
during the decode; "KiB" is the bytes they requested. The Rust numbers come from a counting global allocator over
the whole process. The C numbers come from hooks on each library's allocator (libpng's user allocator, which zlib's
calls also go through; libspng's context allocator; a counting `operator new` for the C++ side).

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

The zune-png call counts in this table come from the high-load pass and differ from the quiet pass's (18 against 11 on
the 1 MP file, say), while its requested bytes match. Its call count isn't stable from run to run. The fork's and
png 0.18's counts were the same in both passes. The libpng count on the palette and tRNS files (10–12 calls, against
8) is libpng's tRNS and palette transforms allocating their own buffers. The keyed photograph was not counted.

What the numbers include:

- **Fork:** no calls in steady state. Its one call on the palette and tRNS files is a 1 KiB buffer.
- **png 0.18:** its inflate state and row buffers, about 400 KiB, allocated again each decode.
- **zune-png:** the output buffer plus its own working memory, all allocated each decode. Peak above the start was
  942 MB on the 116 MP file.
- **libpng:** the read and info structures, zlib's inflate state and window (32 KiB), the 8 KiB zbuf, row buffers and
  the keep list. Every allocation goes through libpng's user allocator, zlib included.
- **libspng:** the context, zlib's state and window, and its text-chunk storage. Its output buffer is reused. The
  library files here are text-free, so their text storage is absent.
- **Wuffs:** one 44 KiB decoder object per call. Its pixel, work and repacking buffers are reused, so they add nothing
  after warm-up.
- **Pillow:** not measured. Its image buffers are allocated in C, outside any hook this harness can count.

## Output

Every decoder's output was compared with the app's fork's, sample by sample, for every file (24 of them):

- **8-bit files** (the eleven library and generated ones, plus the palette, gray and RGB tRNS files and the two filter-0
  files): libpng, libspng, both Rust crates, and Wuffs are identical to the app, except Wuffs on the two colour-key
  files. Pillow is identical on every 8-bit file, the tRNS ones included.
- **Colour-key difference (Wuffs only, 8-bit):** on `rgb8-3mp-trns` (1,341,222 of 11,141,120 samples) and `gray8-2mp-trns`
  (448,299 of 5,570,560), Wuffs differs from the app. The difference is the colour of the fully transparent key pixels,
  which Wuffs writes as black (0) where the app keeps the key colour (251 in these files). The alpha is identical in
  every sample. This is in the output, so `compare` reports it.
- **16-bit files** (the synthetic ones and the four real photographs): the app's output is the source × 257 exactly for
  the synthetic files (checked by `make_inputs.py --verify-dump`), and the fork's dump equals the photographs' PPM
  samples exactly. Every decoder is identical to the app on every 16-bit file without a key, Wuffs included since its
  repacking. Pillow reads 16-bit RGB(A) at 8 bits, so it is not compared.
- **16-bit colour-key photograph** (`real-flower_foveon-16bit-trns`, 13,716,864 samples):
  - libpng: identical to the app.
  - Wuffs: 3 samples differ (the key pixel's colour, written as black; largest 12201). Alpha is identical.
  - zune-png 0.5: 1 sample differs, the alpha of the key pixel (largest 65535). zune-png 0.5.2 expands 16-bit tRNS (`expand_trns::<true>`, `src/utils.rs`) while its samples are still big-endian; the conversion to native order comes after the expansion (`src/decoder.rs`). The expansion compares the samples with the key's native-endian bytes (`to_ne_bytes()`), so on a little-endian host a key matches only if both bytes of every channel are equal. The flower's key (10887, 12201, 10673) has unequal bytes in all three channels, so pixel (0, 0) stays opaque; it is the only pixel equal to the key. This is an upstream zune-png behaviour, not an app one.
  - libspng: 9,343,104 samples differed (largest 64515), and the fault was in the harness, now fixed. libspng writes `SPNG_FMT_RGBA16` (and `GA16`) in host order: it converts each 16-bit scanline to host order before those formats' conversion (`spng.c`, line 3247, for every format but RAW). The harness read the output as big-endian, so the colour samples came out byte-swapped. Alpha survived, since 0x0000 and 0xFFFF are unchanged by a swap. The count agrees exactly: a swap leaves 944,544 of the 10,287,648 colour samples equal, so 9,343,104 differ. The harness now swaps those outputs to big-endian inside the timed decode (`harness/dec_libspng.cpp`), and libspng is identical to the app on the photograph. The earlier libspng colour-key timing (181.99 → 205.80 ms) was of the wrong output and is superseded.
- **16-bit grey colour key** (`gray16-2mp-trns`, 5,570,560 samples): libpng and libspng are identical to the app, the
  libspng GA16 path included (its output is host-ordered, and the harness now swaps it). Wuffs differs in 448,299
  samples, the same count as its 8-bit grey key: the key pixels are written black. The Rust decoders are identical,
  zune-png too, because each sample is the 8-bit value times 257, so the key's two bytes are equal and the byte-order
  fault cannot show.
- **APNG:** the decoders output the first frame, which is the library's image exactly (checked on the app's output).

The comparison was checked against a deliberately altered reference: one 8-bit sample gives "1 of … samples differ,
largest 1", and one 16-bit sample gives "largest 256", which confirms big-endian reading.

## Tooling

- `eval/png-decoders/src/` (Rust): the fork, png 0.18 and zune-png, with the counting allocator (`allocs`), `inflate`
  (fdeflate, one call over the whole stream), and `inflate-stream` (fdeflate, streamed in 16 KiB pieces into a 96 KiB
  rolled window).
- `eval/png-decoders/harness/` (C++, MSVC): libpng 1.6.59, libspng 0.7.4, Wuffs d28ecd4, zlib 1.4 (static, shared by
  libpng and libspng). Its commands are `time`, `compare`, `allocs`, `inflate` (zlib, one call), `inflate-stream` (zlib,
  streamed the same way as the Rust one) and `dump`. Built with `eval\png-decoders\win_build.py`
  (`$env:PG_ROOT='D:\PhotoGallery'`).
- `eval/png-decoders/make_inputs.py`: writes the seven generated inputs, checks Pillow's decode of the 8-bit ones
  against the source, and checks the app's 16-bit dump with `--verify-dump`.
- `eval/png-decoders/make_variants.py`: the tRNS variants (including the real-photo key), the filter-0 variants and the
  text-free library copies.
- `eval/png-decoders/make_real16.py`: the four real photographs, from the imagecompression.info zip.
- `eval/png-decoders/pillow_bench.py`: Pillow's reader, timed and compared against the app's dump.
- `eval/png-decoders/quiet_table.md`: the quiet pass 2 table, generated from the timing files.

## Next

- **Colour-key expansion in the fork.** The cost (2.8× on RGB, 5.1× on gray, 1.3× on the 16-bit photograph) is
  confirmed in one run, and the inflate is ruled out by the identical streams. The mechanism is read from
  `expand_trns_line` but not profiled. A profiler trace of the keyed and unkeyed file, once a profiler is set up on
  Windows, would say where the time goes before anything is changed.
- **libpng's and libspng's non-inflate work on the photographs.** libpng's is 1.7–4.3× the fork's, libspng's 5–12×.
  Neither was profiled. The libspng part is the larger, and its cause is not located.
- **libspng's 16-bit RGBA and GA16 output.** The fault was in the harness, and it is fixed (see Output). The keyed photograph is timed above on a quiet machine. The GA16 path is tested on `gray16-2mp-trns` and matches the app.
- **Wuffs' unfiltering.** Measured (see the checksum paragraph in the inflate section): about 2.4× the fork's on adaptive
  rows. Profiling it would name the code; no profiler is set up yet.
- **zune-png's 16-bit colour key.** Explained (see Output): it compares native-endian key bytes with big-endian samples. Decide whether to report it upstream. The app's fork is not affected.
- **The fork against png 0.18 on two files.** The 116 MP file (fork 1229.55, png 0.18 1983.78 in pass 2; 988.77 and 1012.37
  in pass 1) and the 2.46 MP RGBA file (29.03 against 21.45 in pass 2; 20.15 against 20.56 in pass 1) vary by more than
  the other files. Rerun both on a quiet machine.
- **Real photographs with an 8-bit tRNS chunk.** Only the 16-bit photograph has a key. An 8-bit photograph with a key
  would show whether the key cost on real data matches the synthetic files'.
- **Done since the last round:** the libspng 16-bit output cause (a harness byte-order fault, fixed); the zune-png 16-bit key cause; the keyed photograph re-timed on a quiet machine, in the colour-key section; the GA16 test file (`gray16-2mp-trns`); Wuffs' inflate measured, with the zlib checksum found to be a fifth of zlib's time on the 8-bit files.
- **Done in this round:** the quiet-machine rerun (quiet pass 2, all 23 files in one run); the colour-key finding, confirmed
  and its cost measured on RGB, gray, palette and a 16-bit photograph; the inflate split by streamed inflate, on the 8-bit
  and photographic streams, at 2, 5 and 6 s; the Wuffs RGB-to-RGBA swizzle check (ruled out as the cost of its RGB path);
  libspng on the real photographs (confirmed 1.9–3.1× the fork, with its non-inflate work the larger part); the 16-bit colour-key
  photograph, with its results in the output table.
- **Done in the earlier round:** the four real 16-bit photographs; the three synthetic tRNS inputs, with Wuffs' colour-key
  difference documented; Wuffs' 16-bit output, now repacked to the app's layout and identical on every 16-bit file;
  libspng's text chunks, removed from the library copies for every decoder; the libpng and libspng C-side decoders;
  the Wuffs decoder.
