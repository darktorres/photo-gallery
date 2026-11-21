# JPEG encoders

Which JPEG encoder ImageReduce (and the thumbnail cache) should use: libjpeg-turbo at ImageReduce's settings
against mozjpeg, jpegli, guetzli, nvJPEG and GPUJPEG, by the bytes each needs for the same perceived quality
(`eval/jpeg-decoders/encoders.py`, 2026-10-31). Decoders are in [JPEG_DECODERS.md](JPEG_DECODERS.md); the thumbnail
encoder comparison (jpegli −9% bytes, AVIF −38%) is in THUMBNAILS.md "Thumbnails".

## Verdict

- **jpegli saves the most:** on the camera files it needs 0.69–0.89× the bytes of ImageReduce's encode for the same
  SSIMULACRA2 score, and 0.75–0.93× as baseline, which keeps the cheap decode. mozjpeg's trellis quantization comes
  next: 0.77–0.97×, or 0.78–1.02× as baseline.
- **ImageReduce doesn't use mozjpeg's gains today.** Its encode is libvips' `jpegsave` (see its switch section below),
  which writes the Annex K tables at Q75 with no trellis, so its output is libjpeg-turbo's: the `turbo` row here. The
  doc does not record whether that libvips build uses mozjpeg.
- **Re-encoding with the tables a file was made with is nearly lossless,** and every other encoder loses against
  it on those files. Some Pinterest files show this in both directions (see "Inputs"). ImageReduce's inputs are
  mostly already-compressed web images, so a switch has to be judged on them, not only on camera originals.
- **The GPU encoders don't help size:** nvJPEG (same tables) matches libjpeg-turbo to within 1.3%; GPUJPEG needs
  1.04–1.34× the bytes (fixed Huffman tables, restart markers).
- **Progressive saves at most 4.5%** (the phone photo) and costs 2.6× the decode instructions
  (JPEG_DECODERS.md "Restart markers and lossless baseline conversion"); keep baseline.

## Inputs and method

The seven decoder inputs (JPEG_DECODERS.md "Inputs"), decoded once by libjpeg-turbo to PPM. Every encoder
encodes each PPM at qualities 50–95 (guetzli: 84–95, the lowest it accepts; Pinterest-sized files only, as it
takes minutes and ~300 MB per megapixel). All at 4:4:4, as ImageReduce encodes. Every output is decoded by the same
libjpeg-turbo and scored with SSIMULACRA2 (jpegli's `ssimulacra2` tool) against the PPM it came from.

The reference is ImageReduce's setting: libjpeg-turbo baseline, Q75, 4:4:4, optimized Huffman tables. For each
other encoder, the bytes it needs to reach the reference's score are interpolated along its quality sweep (log bytes
against score), as a ratio of the reference's bytes.

**The sources are JPEGs, not pristine images.** An encoder whose quantization tables match the source's
re-quantizes almost without loss, so it scores far higher at the same nominal quality:

- **`pin-base444-3mp`** was evidently made with the Annex K tables at about Q75. libjpeg-turbo Q75 scores 94.4 on
  it, more than any other encoder reaches even at Q95.
- **`pin-base444-1mp`** matches mozjpeg's own tables: mozjpeg Q75 scores 95.2 with 6% fewer bytes, against
  libjpeg-turbo's 86.9.

The camera files (their own tables, at high quality) are the fair test, and their median is the headline figure.

| Encoder | Settings | Built |
|---|---|---|
| `turbo` (the reference) | `cjpeg -quality Q -sample 1x1 -optimize` | libjpeg-turbo 3.1.0, the vendored copy |
| `turbo-prog` | the same, `-progressive` | |
| `mozjpeg` | `cjpeg -quality Q -sample 1x1` (its defaults: trellis, its tables, progressive) | mozjpeg 5.0.0 (0826579) |
| `mozjpeg-base` | the same, `-baseline` | |
| `jpegli` | `cjpegli -q Q --chroma_subsampling=444` (progressive by default) | jpegli 031a007 |
| `jpegli-base` | the same, `-p 0` | |
| `guetzli` | `guetzli --quality Q` | guetzli 214f2bb |
| `nvjpeg` | nvImageCodec encoder (nvJPEG): quality Q, 4:4:4, optimized Huffman, baseline | CUDA 13; nvImageCodec 0.9.0.20 as measured. The pinned 0.10.0.9 gives byte-identical output on the 40 camera originals at Q75 and Q80 (80 files, 2026-11-14), so these figures stand for it too; the seven inputs above were not rerun, as their source folder isn't on this machine |
| `gpujpeg` | `gpujpegtool -e -q Q --subsampled=4:4:4 -i` (its default restart interval, 8) | GPUJPEG 37ab481 |

## Bytes for the same quality

Bytes each encoder needs to match `turbo` Q75's SSIMULACRA2 score, as a ratio of `turbo`'s bytes. Below 1 is
smaller. ">" marks files where even the encoder's highest quality scored lower than the reference (its ratio at
Q95 is shown); guetzli ran on the Pinterest files only.

| Input | `turbo` Q75 score / bpp | jpegli | jpegli-base | mozjpeg | mozjpeg-base | guetzli | turbo-prog | nvjpeg | gpujpeg |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 86.9 / 0.68 | 0.988 | 1.025 | 0.958 | 0.972 | 1.011 | 1.028 | 0.998 | 1.286 |
| pin-base444-3mp | 94.4 / 0.51 | >1.248 | >1.294 | >1.279 | >1.320 | >1.061 | 1.014 | >1.000 | >1.340 |
| pin-prog420-1mp | 84.5 / 0.68 | 0.852 | 0.889 | 0.950 | 0.985 | 0.962 | 1.001 | 1.005 | 1.281 |
| pin-base420-1mp | 84.9 / 0.82 | 0.825 | 0.869 | 0.877 | 0.902 | 0.977 | 0.999 | 1.000 | 1.219 |
| phone-base420-4mp | 77.5 / 3.10 | 0.891 | 0.925 | 0.973 | 1.022 | — | 0.956 | 0.998 | 1.041 |
| sony-base422-7mp | 77.4 / 0.62 | 0.748 | 0.793 | 0.810 | 0.827 | — | 0.998 | 0.991 | 1.218 |
| a6000-base422-24mp | 76.9 / 0.47 | **0.691** | 0.753 | 0.769 | 0.779 | — | 1.006 | 0.987 | 1.340 |
| **median, camera files** | | **0.748** | 0.793 | 0.810 | 0.827 | — | 0.998 | 0.991 | 1.218 |
| median, all | | 0.839 | 0.879 | 0.914 | 0.937 | 0.977 | 1.001 | 0.998 | 1.250 |

- **jpegli's lead grows with resolution on these three:** 0.89 on the 4 MP phone photo, 0.75 on the 7.5 MP Sony,
  0.69 on the 24 MP A6000. Three files don't show why.
- **Baseline costs jpegli and mozjpeg 1–6 points** of the saving (`-base` against the default progressive). It is
  still the right setting: the decode of every later view costs far more than the bytes.
- **guetzli doesn't beat jpegli** on the files it could run on (0.96–1.01), and takes minutes per file.
- **The same nominal quality isn't the same quality:** jpegli and mozjpeg at Q75 score 2.0–3.1 points below
  libjpeg-turbo Q75 on the camera files. Comparing at the same Q overstates their savings (jpegli Q75 is 0.58×
  the bytes on the A6000, against 0.69× at matched quality); the matched-score ratios are the ones to use.

## Encode speed

ms per megapixel at Q75, 2026-10-31, 0.5–1 core of other load. CPU encoders: the median of 3 runs of the command,
one at a time in WSL. That includes process start and reading the PPM, which inflates the 1 MP rows (and
`pin-base420-1mp` came out about 2× its neighbours in every column, so read that row as noise). nvJPEG:
in-process through nvImageCodec, host pixels in and JPEG bytes out. GPUJPEG: its own per-iteration average, first
iteration excluded, also including the copies to and from the GPU.

| Input | turbo | turbo-prog | mozjpeg | mozjpeg-base | jpegli | jpegli-base | guetzli | nvjpeg | gpujpeg |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 13.2 | 23.1 | 115.9 | 62.4 | 33.8 | 21.0 | 36,765 | 1.0 | 0.6 |
| pin-base444-3mp | 11.7 | 20.4 | 97.4 | 57.2 | 26.8 | 15.1 | 38,925 | 0.8 | 0.5 |
| pin-prog420-1mp | 13.5 | 22.4 | 101.9 | 64.2 | 37.9 | 28.2 | 43,730 | 1.1 | 0.7 |
| pin-base420-1mp | 35.8 | 57.3 | 200.6 | 115.5 | 65.9 | 35.2 | 50,747 | 0.9 | 0.6 |
| phone-base420-4mp | 21.1 | 40.8 | 224.8 | 136.9 | 43.3 | 29.3 | — | 0.9 | 0.5 |
| sony-base422-7mp | 16.2 | 25.7 | 101.9 | 62.7 | 24.4 | 11.8 | — | 0.7 | 0.5 |
| a6000-base422-24mp | 11.5 | 18.9 | 94.7 | 53.2 | 22.5 | 10.3 | — | 0.8 | 0.5 |

- **jpegli baseline costs about what libjpeg-turbo does:** 0.7–1.4× on the camera files. Its progressive default
  costs 1.5–2.2× more.
- **mozjpeg is the slow one:** 6–11× libjpeg-turbo with its progressive default, 4–6.5× as baseline, on the camera
  files.
- **guetzli takes 37–51 s per megapixel,** about 3,000× libjpeg-turbo, for no gain over jpegli.
- **The GPU encoders take 0.5–1 ms per megapixel,** 14–42× faster than libjpeg-turbo on the camera files, but
  their files are no smaller (nvJPEG) or 4–34% bigger (GPUJPEG) at the same quality.

## ImageReduce's switch on real inputs

`eval/jpeg-decoders/imagereduce_switch.py`, 2026-11-14. ImageReduce's encode is libvips 8.18.7's `jpegsave` at Q75,
4:4:4, `optimize_coding`, after a thumbnail to 3840×2160 for images over that size. Its output's quantization tables
are the standard Annex K tables at Q75, baseline, so the `JCP_FASTEST` note above holds. Each input is decoded the way
ImageReduce decodes it, both encoders get the same 8-bit sRGB pixels, and every output is decoded by libvips and
scored with SSIMULACRA2 against those pixels. Inputs are the originals ImageReduce would still receive: the library
copy's files without "mozjpeg" in the name, since ImageReduce never reprocesses its own outputs.

| Set | Files | Today's output bigger than input (kept as original) | Matched score: jpegli / vips image bytes |
|---|---:|---:|---:|
| Pinterest originals | 1,215 (1,035 JPEG, 140 PNG, 40 WebP; 53 have alpha, kept as RGB) | 981 (81%) | 0.923 |
| Camera originals | 40 (20 on the thumbnail path) | 2 (5%) | 0.833 |

The matched figure is the jpegli bytes needed for the score vips got, per file. ImageReduce can't search per file, so
the switch has to use one jpegli quality. Totals use the keep-original rule (keep the input when the output is bigger),
with the vips metadata (573 Pinterest files, 39 camera files carry ICC or EXIF) copied onto the jpegli side:

| jpegli Q | Pinterest: median Δscore / size vs vips / total bytes | Camera: median Δscore / size vs vips / total bytes |
|---|---|---|
| 65 | −5.38 / 0.785 / −18.1% | −5.57 / 0.663 / −35.2% |
| 70 | −2.70 / 0.853 / −12.1% | −3.70 / 0.716 / −29.4% |
| 75 | −0.55 / 0.893 / −7.6% | −2.00 / 0.767 / −23.3% |
| 76 | +0.09 / 0.901 / −6.8% | −1.56 / 0.775 / −22.2% |
| 77 | −0.02 / 0.913 / −5.8% | −1.00 / 0.798 / −20.3% |
| 78 | **+0.35 / 0.931 / −3.9%** | **−0.01 / 0.826 / −17.0%** |
| 80 | +0.45 / 0.950 / −2.5% | +0.28 / 0.860 / −13.8% |
| 85 | +3.08 / 1.036 / +1.9% | +3.20 / 1.031 / +2.4% |
| 90 | +4.88 / 1.140 / +8.4% | +6.07 / 1.353 / +34.7% |

- **Q78 is the setting.** It's the matched point on the camera files (median score change −0.01) and slightly better
  on the Pinterest files (+0.35), with 4–17% fewer bytes. Q80 scores better still but saves less: 2.5% and 13.8%.
- **Q77 saves a little more on the Pinterest files at the same score, but is 1 point lower on the camera files.**
  The Pinterest originals are already compressed, so most keep their originals either way.
- **Encode time** (`eval/jpeg-decoders/imagereduce_timing.py`, same files, one thread for libvips, three runs each,
  median): per megapixel, jpegli is about even with ImageReduce's encode, but a per-file cjpegli call costs more.

  | Set | vips Q75 ms/MP | cjpegli Q80 ms/MP, net of process start | cjpegli, wall time, total ratio |
  |---|---:|---:|---:|
  | Pinterest originals, 500 random (median 0.99 MP) | 13.5 | 11.2 (24.9 with startup) | 1.63× |
  | Camera originals, all 40 (median 3.8 MP) | 11.5 | 12.9 (15.9 with startup) | 1.39× |

  cjpegli starts in about 13 ms, which is most of a one-megapixel encode, so a per-file call is slower than the net
  column. The vips side encodes from memory without metadata; cjpegli runs one thread.
- **Encode time, in-process:** calling jpegli's library directly (a small C++ program, not in the repo, using the
  settings cjpegli uses) gives 12.7 ms/MP on the Pinterest sample (vips 13.5, about 6% faster) and 12.8 on the camera
  files (vips 11.5, about 11% slower). Its output is byte-identical to cjpegli's on all 40 camera files.
- **Not done:** the switch itself: a libjpegli build linked into ImageReduce, and the ICC and EXIF segments copied
  across. The score for the 20 thumbnail-path camera files is against the resized image, so it doesn't include the
  resize.
- **Checked:** alpha PNG and WebP are encoded by libvips with the alpha band dropped and RGB kept as stored, so they're
  in the tables. One PNG is over the 65,500-pixel limit: ImageReduce skips any image with a side over 65,500
  (`mainwindow.cpp` line 114) before encoding, so jpegli never sees it.

To rerun (Windows, native, not the container): set `PG_IMAGEREDUCE_VIPS_BIN` to ImageReduce's `vips-dev-8.18\bin`,
`PG_JPEGLI_BUILD` to jpegli's build folder, and `PG_DATA_ROOT` and `CAMERA_ROOT` to the library and camera folders, then

```sh
python eval/jpeg-decoders/imagereduce_switch.py run library --out WORK/library.csv
python eval/jpeg-decoders/imagereduce_switch.py run camera --n 40 --out WORK/camera.csv
python eval/jpeg-decoders/imagereduce_switch.py summarize WORK/library.csv WORK/camera.csv
python eval/jpeg-decoders/imagereduce_timing.py WORK --n 500
```

## What to do

- **ImageReduce:** switch to jpegli baseline 4:4:4 at Q78 (see above), not progressive, copying the ICC and EXIF
  segments across. Keep its "keep the original if the re-encode isn't smaller" rule, which today keeps 81% of the
  Pinterest originals.
- **Not progressive:** for any encoder.
- **The thumbnail cache:** THUMBNAILS.md "Thumbnails" already measured jpegli at −9% bytes; nothing here changes that.

## To rerun

In the container, after `just linux-eval eval/jpeg-decoders/linux_build.py` (which builds every encoder and
`ssimulacra2`) and `linux.py inputs`, from the repo. `PG_JPEG_ENC_DIR` in `.env` names the work folder on this machine;
the container mounts it, so both sides see the same files:

```sh
just linux-eval eval/jpeg-decoders/encoders.py prepare   # PPMs in PG_JPEG_ENC_DIR/src
just linux-eval eval/jpeg-decoders/encoders.py encode    # CPU encoders into PG_JPEG_ENC_DIR/out
just linux-eval eval/jpeg-decoders/encoders.py speed
just linux-eval eval/jpeg-decoders/encoders.py score     # scores (cached in PG_JPEG_ENC_DIR/scores.csv) and the tables
```

The GPU encoders run on Windows in between `encode` and `score`, writing into the same folder:
`D:\PhotoGallery\tools\jpeg-eval\venv\Scripts\python.exe eval\jpeg-decoders\gpu_encoders.py
--gpujpeg D:\PhotoGallery\tools\jpeg-eval\build\GPUJPEG\gpujpegtool.exe`. The inputs are library photos and stay
in `PG_JPEG_ENC_DIR`, never in the repo. (The figures above were measured in WSL, before the container.)
