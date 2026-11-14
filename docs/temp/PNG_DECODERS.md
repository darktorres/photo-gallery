# PNG decoders

Which PNG decoder the app should use: the `png` fork that `pg-codec` calls, against the same crate from crates.io
(0.18.1), zune-png 0.5.2, libpng 1.6.59, libspng 0.7.4 and Wuffs (d28ecd4), on fourteen PNGs: seven from the library,
seven made for the cases the library lacks (16-bit, Adam7-interlaced, grayscale, animated). Pillow's PNG reader is a
second opinion outside Rust. It is the PNG counterpart of [JPEG_DECODERS.md](JPEG_DECODERS.md). Everything runs
natively on Windows (`eval/png-decoders`, 2026-11-14). Wall time, heap traffic and the output are measured. No
instruction counts (callgrind doesn't run here).

## Verdict: keep the app's png fork

- **Output:** every decoder matches the app's output sample for sample on every file it can produce. PNG is lossless,
  so this is the bar. Wuffs can't produce the app's 16-bit layout (its 16-bit RGB(A) is little-endian), so those three
  files are not comparable for it.
- **Speed:** the fork is the fastest on 10 of the 14 files. png 0.18 is 0.5–3% faster on the other four (rgb-4mp,
  rgba-1mp, apng, rgba8-interlaced), which is within noise on the small ones. The gap to the others is large: zune-png
  is 1.2–1.5× the fork, Pillow 1.5–2.5×, libpng and libspng about 2× on 8-bit RGB(A) and about 2.5–3× on 16-bit RGB.
  Wuffs is level on the palette file (5.85 vs 5.62 ms), 1.4–2.1× on the other 8-bit files, and 2.0× on the 116 MP file.
- **Heap traffic:** the fork makes no heap calls per decode once its buffers are at size (its `Scratch` holds the
  inflate state and the row buffers). png 0.18 makes 11–19 calls (about 400 KiB, its inflate state) per decode. The
  C decoders make 1–10 calls (44–67 KiB). zune-png hands back a new output buffer each decode, so it requests the
  whole image every time: 1.2 GB on the 116 MP file.
- **Nothing to change.** A faster decoder would have to match every pixel, and none is faster.

## What was measured

Each decoder decodes the file's bytes to packed samples on one thread, full size. The output buffer is reused across
decodes, as the app reuses its output. Each time is the median over at least five decodes, after two warm-up decodes,
for at least two seconds.

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
  its context on each decode. **It cannot skip text chunks** (6 of the 7 library files have tEXt, 1–3 each), so its
  time includes reading them.
- **Wuffs d28ecd4:** `wuffs_aux::DecodeImage` to the matching pixel format (RGB, RGBA, gray). It recreates its decoder
  object on each call (about 44 KiB), but keeps its pixel and work buffers.
- **Pillow 12.2.0 (second opinion, `pillow_bench.py`):** Pillow's own PNG reader, a `zip` decoder over zlib-ng 1.3.1,
  not libpng. Its time covers `Image.open`, `load` and any conversion, so it includes Pillow's object overhead as well
  as the decode.

The timed runs (`time --secs 2`) were done back to back on the same machine, with the machine at 20–32% CPU load
(sampled before and after). The Rust run agrees with an earlier pass within 4%, and the C run with its earlier pass
within about 5% on the large files. The small files carry more noise: a few milliseconds each.

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

Two caveats on the 16-bit inputs. Each sample is the 8-bit value × 257, so its high and low bytes are equal. That
compresses better than a camera's 16-bit file would, so inflate is cheaper here than on real 16-bit data. And the
inputs are synthetic, so the grayscale and APNG cases are a check on the code paths, not a sample of real files.

No input is a camera or scanner file, and no input carries tRNS.

## Decode time

Median ms per decode, one thread, full size. The Rust and C columns come from one run each, back to back. Pillow is
from its own run, in the same session.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs | Pillow 12 |
|---|---:|---:|---:|---:|---:|---:|---:|
| rgb-1mp | **9.44** | 9.53 | 13.69 | 18.64 | 18.94 | 19.24 | 23.26 |
| rgb-3mp | **11.02** | 11.39 | 15.77 | 22.21 | 21.86 | 18.72 | 24.86 |
| rgb-4mp | 41.14 | 40.26 | 62.29 | 69.60 | 66.07 | 67.92 | 79.16 |
| rgba-1mp | 10.64 | 10.49 | 12.49 | 20.31 | 19.92 | 22.04 | 25.73 |
| rgba-2mp | **20.15** | 20.56 | 25.64 | 40.45 | 40.88 | 39.59 | 50.07 |
| palette-2mp | **5.62** | 5.61 | 7.63 | 12.32 | 11.41 | 5.85 | 11.32 |
| rgb-116mp | **988.77** | 1012.37 | 1335.29 | 1815.83 | 1829.23 | 1965.26 | 2336.43 |
| rgb16-3mp | **15.97** | 17.18 | 21.06 | 47.11 | 74.42 | n/c | n/a |
| rgba16-2mp | **28.05** | 28.91 | 32.52 | 71.61 | 133.81 | n/c | n/a |
| rgb16-interlaced-3mp | **26.59** | 29.04 | 43.05 | 78.06 | 77.60 | n/c | n/a |
| rgb8-interlaced-3mp | **23.42** | 24.34 | 31.45 | 42.34 | 30.68 | 38.40 | 38.66 |
| rgba8-interlaced-2mp | 33.62 | 32.58 | 41.89 | 80.36 | 60.57 | 55.06 | 59.64 |
| gray8-2mp | **5.90** | 5.79 | 8.38 | 10.48 | 14.41 | 8.50 | 9.07 |
| apng-rgb8-3frames | 12.13 | 12.07 | 17.05 | 24.91 | 23.88 | 18.76 | 24.58 |

n/c: not comparable (Wuffs writes 16-bit RGB(A) little-endian only). n/a: Pillow reads 16-bit RGB(A) at 8 bits.

The gap to libpng and libspng on 8-bit RGB is about 2×, and on 16-bit RGB about 2.5–3× (libspng is slower still on
16-bit, at 74 and 134 ms). Their inflate is zlib's; the fork's is fdeflate. That's a likely cause, but it is not
profiled here. Interlacing costs the fork about 2× (rgb-3mp 11.0 → 23.4 ms), and the C decoders 1.4–1.9×.

## Heap traffic

Each decoder, one decode after two warm-ups, so the buffers it keeps are at their size. "Calls" are heap calls made
during the decode; "KiB" is the bytes they requested. The Rust numbers come from a counting global allocator over
the whole process. The C numbers come from hooks on each library's allocator (libpng's user allocator, which zlib's
calls also go through; libspng's context allocator; a counting `operator new` for the C++ side).

| Input | png (app fork) | png 0.18 | zune-png 0.5 | libpng 1.6 | libspng 0.7 | Wuffs |
|---|---|---|---|---|---|---|
| rgb-1mp | 0 | 16 / 404 KiB | 18 / 9,996 KiB | 8 / 54 KiB | 5 / 48 KiB | 1 / 44 KiB |
| rgb-3mp | 0 | 15 / 404 KiB | 17 / 18,343 KiB | 8 / 57 KiB | 5 / 51 KiB | 1 / 44 KiB |
| rgb-4mp | 0 | 17 / 404 KiB | 29 / 41,416 KiB | 8 / 60 KiB | 7 / 55 KiB | 1 / 44 KiB |
| rgba-1mp | 0 | 18 / 404 KiB | 18 / 11,569 KiB | 8 / 55 KiB | 5 / 50 KiB | 1 / 44 KiB |
| rgba-2mp | 0 | 17 / 404 KiB | 19 / 27,426 KiB | 8 / 58 KiB | 5 / 53 KiB | 1 / 44 KiB |
| palette-2mp | 1 / 1 KiB | 19 / 407 KiB | 20 / 14,176 KiB | 10 / 57 KiB | 5 / 45 KiB | 1 / 44 KiB |
| rgb-116mp | 0 | 16 / 404 KiB | 13 / 1,204,074 KiB | 8 / 56 KiB | 5 / 50 KiB | 1 / 44 KiB |
| rgb16-3mp | 0 | 13 / 404 KiB | 5 / 33,908 KiB | 8 / 65 KiB | 5 / 59 KiB | n/c |
| rgba16-2mp | 0 | 11 / 403 KiB | 5 / 42,421 KiB | 8 / 67 KiB | 5 / 62 KiB | n/c |
| rgb16-interlaced-3mp | 0 | 14 / 412 KiB | 6 / 50,787 KiB | 8 / 65 KiB | 6 / 67 KiB | n/c |
| rgb8-interlaced-3mp | 0 | 15 / 408 KiB | 6 / 25,932 KiB | 8 / 57 KiB | 6 / 55 KiB | 1 / 44 KiB |
| rgba8-interlaced-2mp | 0 | 14 / 408 KiB | 6 / 32,623 KiB | 8 / 58 KiB | 6 / 57 KiB | 1 / 44 KiB |
| gray8-2mp | 0 | 14 / 404 KiB | 6 / 11,306 KiB | 8 / 51 KiB | 5 / 46 KiB | 1 / 44 KiB |
| apng-rgb8-3frames | 0 | 16 / 422 KiB | 7 / 19,362 KiB | 8 / 57 KiB | 5 / 51 KiB | 1 / 44 KiB |

What the numbers include:

- **Fork:** no calls in steady state. Its one call on palette-2mp is a 1 KiB buffer.
- **png 0.18:** its inflate state and row buffers, about 400 KiB, allocated again each decode.
- **zune-png:** the output buffer plus its own working memory, all allocated each decode. Peak above the start was
  942 MB on the 116 MP file.
- **libpng:** the read and info structures, zlib's inflate state and window (32 KiB), the 8 KiB zbuf, row buffers and
  the keep list. Every allocation goes through libpng's user allocator, zlib included.
- **libspng:** the context, zlib's state and window, and its text-chunk storage (it can't skip text). Its output
  buffer is reused.
- **Wuffs:** one 44 KiB decoder object per call. Its pixel and work buffers are reused, so they add nothing after
  warm-up. Its inflate state lives in that decoder object, so it is not like-for-like with the fork, which keeps its
  inflate state.
- **Pillow:** not measured. Its image buffers are allocated in C, outside any hook this harness can count.

## Output

Every decoder's output was compared with the app's fork's, sample by sample, for every file it can produce:

- **8-bit files** (eleven of them, the grayscale, APNG and both 8-bit interlaced files among them): the two Rust
  decoders, libpng, libspng, Wuffs and Pillow are all identical to the app.
- **16-bit files:** the app's output is the source × 257 exactly (checked by `make_inputs.py --verify-dump`). The three
  Rust decoders, libpng and libspng are identical to it. Wuffs is not comparable. Pillow reads 16-bit RGB(A) at 8 bits,
  so it is not compared.
- **APNG:** the decoders output the first frame, which is the library's image exactly (checked on the app's output).

The comparison was also checked against a deliberately altered reference: one 8-bit sample gives "1 of … samples
differ, largest 1", and one 16-bit sample gives "largest 256", which confirms big-endian reading.

## Tooling

- `eval/png-decoders/src/` (Rust): the fork, png 0.18 and zune-png, with the counting allocator (`allocs`).
- `eval/png-decoders/harness/` (C++, MSVC): libpng 1.6.59, libspng 0.7.4, Wuffs d28ecd4, zlib 1.4 (static, shared by
  libpng and libspng). Built with `eval\png-decoders\win_build.py` (`$env:PG_ROOT='D:\PhotoGallery'`).
- `eval/png-decoders/make_inputs.py`: writes the seven generated inputs, checks Pillow's decode of the 8-bit ones
  against the source, and checks the app's 16-bit dump with `--verify-dump`.
- `eval/png-decoders/pillow_bench.py`: Pillow's reader, timed and compared against the app's dump.

## Next

- **Find the cause of the 2× gap** on 8-bit RGB for the zlib-based decoders: profile libpng's inflate against the
  fork's fdeflate (the claim above, that the gap is inflate, is a guess until then).
- **Wuffs on 16-bit:** either byte-swap its little-endian output inside the timed decode (and count it), or find a
  big-endian 16-bit format. Until then it's not comparable on 16-bit.
- **libspng can't skip text chunks**, so its time carries them. Either measure libspng with text stripped from the
  inputs, or accept the difference and say so.
- **Real 16-bit files:** the 16-bit inputs are synthetic (8-bit × 257). A camera or scanner 16-bit file would show
  whether the inflate advantage holds on data with real low bytes.
- **Untested paths:** palette and RGB or gray with tRNS. The C and Rust code handles them, but no input has tRNS, so
  they are implemented and not measured.
- **Quiet-machine rerun:** the final timing ran at 20–32% CPU load. The small files (1–2 MP) carry the most noise.
