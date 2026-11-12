# PNG decoders

Which PNG decoder the app should use: the `png` fork that `pg-codec` calls, against the same crate from crates.io
(0.18.1) and zune-png 0.5.2, on seven library PNGs, plus Pillow's PNG reader as a second opinion (`eval/png-decoders`,
2026-11-09). It is the PNG counterpart of [JPEG_DECODERS.md](JPEG_DECODERS.md). Unlike that run, this one is native
on Windows, with no instruction counts (callgrind doesn't run here). Only wall time and the output are measured.

## Verdict: keep the app's png fork

- **Output:** every decoder matches the app's output on all seven files, sample for sample. PNG is lossless, so this
  is the bar, and no decoder fails it.
- **Speed:** the fork and the crates.io `png` 0.18.1 take the same time on every file, within about 4% (the fork's
  buffer reuse doesn't show in wall time here). zune-png is 1.2–1.5× slower, and Pillow's reader 2.1–3.7× slower.
- **Nothing to change.** A faster decoder would have to match every pixel, and none is faster.
- **What isn't measured:** allocations, the reason the fork keeps its buffers. Wall time doesn't show them, so this
  run doesn't settle whether the fork saves memory traffic on large batches.

## What was measured

Each decoder decodes the file's bytes to 8-bit samples on one thread, full size. The output buffer is reused across
decodes, as the app reuses its output. Each time is the median over at least five decodes, after two warm-up
decodes, for at least two seconds.

- **png (app fork):** the fork at `f7129d3`, decoded as `Codecs::png` does: its scratch kept between files, text and
  ICC chunks skipped, EXPAND, the app's 512 MiB limit.
- **png 0.18:** crates.io's 0.18.1 with the same options and limit, and no scratch. It uses the app's patched
  `fdeflate` (the root `Cargo.toml` patches every user of fdeflate, and the fork needs it), so it isn't the stock
  crate.
- **zune-png 0.5.2:** the zune-image family's decoder, with zune-core 0.5.3 from crates.io. Its height cap is raised
  to 2^20, since its default is 16384 and refuses the 116 MP file. Its API returns a new buffer each decode, so its
  time includes that allocation.
- **Pillow 13.0.0 (second opinion, run from `pillow_bench.py`):** Pillow's own PNG reader, a `zip` decoder over
  zlib-ng 1.3.1. It is not libpng. Its time covers `Image.open`, `load` and any conversion, so it includes
  Pillow's object overhead as well as the decode.

**Not measured:** libpng, libspng, Wuffs, FFmpeg's PNG decoder, and stb_image. None of them was built for this run.
Wuffs and libspng are the next ones worth adding, since Wuffs is already in the JPEG harness.

## Inputs

The PNGs in `datasets/pinterest` and `equiv/src` (294 files, with copies) are all 8-bit: 188 RGB, 104 RGBA and 2
palette. None is 16-bit, interlaced or animated, so those cases aren't covered here. The seven files are the nearest
to these targets by megapixels, with the largest library PNG as the last one.

| Input | Kind | Size | File |
|---|---|---|---|
| `rgb-1mp` | RGB 8-bit | 1.01 MP, 1.7 MB | `pinterest_1141451468122730916` |
| `rgb-3mp` | RGB 8-bit | 2.79 MP, 1.0 MB | `pinterest_1141451468123671931` |
| `rgb-4mp` | RGB 8-bit | 4.28 MP, 5.8 MB | `pinterest_1141451468122225667` |
| `rgba-1mp` | RGBA 8-bit | 0.96 MP, 1.5 MB | `pinterest_1141451468122732060` |
| `rgba-2mp` | RGBA 8-bit | 2.46 MP, 3.0 MB | `pinterest_1141451468123671747` |
| `palette-2mp` | palette | 2.07 MP, 0.8 MB | `pinterest_1141451468123718688` |
| `rgb-116mp` | RGB 8-bit | 116.14 MP, 142 MB | `_incoming/db4f698cb287297b07d5a501a6595f51` |

## Decode time

Median ms per decode, one thread, full size. The first run is shown. A repeat matched it within a few percent except
the palette file (5.6 and 7.6 ms for the fork), so times under about 10 ms carry roughly ±30%.

| Input | png (app fork) | png 0.18 | zune-png 0.5 | Pillow 13 |
|---|---:|---:|---:|---:|
| rgb-1mp | **9.42** | 9.77 | 13.15 | 34.79 |
| rgb-3mp | **11.02** | 10.89 | 15.07 | 34.02 |
| rgb-4mp | **38.70** | 37.71 | 59.01 | 82.44 |
| rgba-1mp | **10.31** | 10.43 | 12.07 | 36.90 |
| rgba-2mp | **19.89** | 19.94 | 24.36 | 58.10 |
| palette-2mp | **5.59** | 5.62 | 7.69 | 11.56 |
| rgb-116mp | **999.40** | 1,040.53 | 1,282.01 | 3,040.20 |

Pillow's times are from a second run, after the fix that moved its `np.asarray` copy out of the timed region. The
run before it had that copy included.

## Output

Every decoder's output was compared with the app's fork's, sample by sample, for all seven files. The Rust harness
(`compare`) and Pillow's bench both report identical output for every file.

## Next

- Add libpng (its zlib dependency is the hard part on Windows) and libspng, for the C side of the table.
- Add Wuffs, as its PNG decoder, with the JPEG harness's setup.
- Measure allocations (a counting allocator or the heap profile the app already uses), since that's what the fork
  is for.
- Add 16-bit, interlaced and animated files, which the library sample doesn't contain.
