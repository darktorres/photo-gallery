# JPEG encoders

Which JPEG encoder ImageReduce (and the thumbnail cache) should use: libjpeg-turbo at ImageReduce's settings
against mozjpeg, jpegli, guetzli, nvJPEG and GPUJPEG, by the bytes each needs for the same perceived quality
(`eval/jpeg-decoders/encoders.py`, 2026-10-31). Decoders are in [JPEG_DECODERS.md](JPEG_DECODERS.md); the thumbnail
encoder comparison (jpegli −9% bytes, AVIF −38%) is in FINDINGS "Thumbnails".

## Verdict

- **jpegli saves the most:** on the camera files it needs 0.69–0.89× the bytes of ImageReduce's encode for the same
  SSIMULACRA2 score, and 0.75–0.93× as baseline, which keeps the cheap decode. mozjpeg's trellis quantization comes
  next: 0.77–0.97×, or 0.78–1.02× as baseline.
- **ImageReduce doesn't use mozjpeg's gains today.** It runs mozjpeg in `JCP_FASTEST` mode (Annex K tables, no
  trellis), so its output is libjpeg-turbo's: the `turbo` row here.
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
| `nvjpeg` | nvImageCodec 0.9 encoder (nvJPEG): quality Q, 4:4:4, optimized Huffman, baseline | CUDA 13 |
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

## What to do

- **ImageReduce:** worth testing jpegli baseline (`-p 0`, 4:4:4) in place of mozjpeg `JCP_FASTEST`, on the files
  ImageReduce actually receives, with SSIMULACRA2 against the original. The camera-file result says 7–25% smaller
  at equal quality. The Pinterest results say a file that already matches libjpeg-turbo's Q75 tables loses quality
  on any switch, so ImageReduce should keep its "keep the original if the re-encode isn't smaller" rule.
- **Not progressive:** for any encoder.
- **The thumbnail cache:** FINDINGS "Thumbnails" already measured jpegli at −9% bytes; nothing here changes that.

## To rerun

In WSL, after `eval/jpeg-decoders/wsl-build.sh` (which builds every encoder and `ssimulacra2`), from the repo:

```
python3 eval/jpeg-decoders/encoders.py prepare   # PPMs in ~/jpeg-enc/src
python3 eval/jpeg-decoders/encoders.py encode    # CPU encoders into ~/jpeg-enc/out
python3 eval/jpeg-decoders/encoders.py speed
python3 eval/jpeg-decoders/encoders.py score     # scores (cached in ~/jpeg-enc/scores.csv) and the tables
```

The GPU encoders run on Windows in between `encode` and `score`, writing into the same folder through
`\\wsl.localhost`: `D:\PhotoGallery\tools\jpeg-eval\venv\Scripts\python.exe -I eval\jpeg-decoders\gpu_encoders.py
--gpujpeg D:\PhotoGallery\tools\jpeg-eval\build\GPUJPEG\gpujpegtool.exe`. The inputs are library photos and stay
in WSL's `~/jpeg-enc`, never in the repo.
