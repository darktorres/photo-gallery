# Thumbnail resampling: findings and decision

Status: research and probes done (2026-11-06 to 2026-11-07). Decision in progress: adopt Magic Kernel Sharp
(MKS2021) for the display path. Not implemented; the open choices are in [Decision](#7-decision).

## 1. The question

Changing the grid's thumbnail height regenerates every thumbnail (about 190k images, 6–8 minutes). Two questions
came out of that:

1. Can a smaller size reuse an existing thumbnail instead of regenerating? (Cache and time.)
2. Which resampling filter should produce the thumbnails, and does it matter for quality?

Cache sizes on the user's library (`D:\PhotoGallery\catalog`, read-only inspection):

| Height | Entries | Size | Average per thumb |
|---|---|---|---|
| 60 | 7,439 | 13 MB | 1.8 KB |
| 220 | 189,519 | 2.0 GB | 10.5 KB |
| 800 | 189,519 | 14.7 GB | 77.7 KB |

The cache holds 16.7 GB of its 50 GB cap. The 800 px set is most of it.

## 2. Current code (verified)

- `crates/codec/src/resize.rs`: `Resizer` uses `fast_image_resize` with `Convolution(Lanczos3)` for every method.
  The module doc says: "One frozen filter for every pipeline: the resize is part of the model (PLAN §3)."
- Callers of that one resizer:
  - Analysis: PDQ input `fit_within(PDQ_SIDE)` (`crates/analysis/src/analyze.rs:109`, comment at :35 says
    Lanczos3 as `eval/pdq.py`), and the ThumbHash input (`analyze.rs:190`).
  - Thumbnails: `to_height` in `crates/render/src/maker.rs:141`.
  - On-demand thumbnail and viewer: `fit_box` in `crates/app/src/images.rs:267`; `resize_region` (the viewer's
    Lanczos region render, `docs/PLAN.md:299`).
- `docs/PLAN.md` §3 rule 8: "The resize pipeline is part of the model. Swapping resize libraries shifts features as
  much as a real change in the data. Freeze one antialiased pipeline, version it inside the index, and re-embed if
  it ever changes." §9 leaves the embedding placement open (decision 6).
- The cache keeps no record of which filter produced an entry. A filter change would mix filters in the cache unless
  it is versioned or regenerated.
- The on-demand path already serves an entry at least as tall as requested. The background pass counts an entry only
  at the exact height, which is why each size change regenerates everything.

## 3. Research

### 3.1 Theory

- **Downscaling needs an antialias (low-pass) filter first.** Detail above the output's Nyquist frequency folds back
  as false patterns. Dropping pixels is "downsampling with no antialias filter" (Brasseur, gamma article; Paint and
  some browsers do this).
- **Upscaling is interpolation.** Classical kernels (nearest, bilinear, bicubic, Lanczos, MKS) estimate values
  between existing pixels and add no detail. Learned super-resolution (ESRGAN, Real-ESRGAN, SwinIR, diffusion SR)
  generates plausible detail that was never in the source. That is enhancement, not upscaling in the classical sense.
- **Gamma.** Averaging gamma-encoded sRGB values darkens results, most visibly on high-contrast detail. ImageMagick
  and the MKS author both recommend linear-light downsampling.

### 3.2 Kernels (fast_image_resize docs, Wikipedia, libvips)

| Kernel | Character | Notes |
|---|---|---|
| Box | Soft, can show block artifacts | Area average |
| Bilinear (antialiased) | Soft | |
| Hamming | Sharper than bilinear, no ringing | Poor for upscaling |
| Mitchell–Netravali (B=C=1/3) | Balanced blur and ringing | The usual safe bicubic |
| Catmull-Rom | Sharper bicubic, mild ringing | |
| Lanczos2, Lanczos3 | Sharpest common defaults, visible ringing | Current app: Lanczos3 |
| Magic Kernel Sharp 2013 / 2021 | Kernel plus a discrete sharpen | Adopted by Facebook, Instagram, Pillow, libvips, pica, PyTorch, Rust crates |

libvips' `VipsKernel` lists NEAREST, LINEAR, CUBIC, MITCHELL, LANCZOS2, LANCZOS3, MKS2013, MKS2021.

### 3.3 Magic Kernel Sharp (Costella)

Sources: johncostella.com/magic (instructions and appendix) and the 2021 paper `johncostella.com/magic/mks.pdf`
(*Solving the mystery of Magic Kernel Sharp*, §3). The author wrote the kernel, so the claims below are his.

- Magic kernel (paper eq. 27), support 3 units (±1.5):
  - m(x) = 3/4 − x² for |x| ≤ 1/2
  - m(x) = (|x| − 3/2)² / 2 for 1/2 < |x| ≤ 3/2
  - m(x) = 0 otherwise
- **Sharp 2013** (eq. 30): taps +3/2 at n=0, −1/4 at n=±1. Applied after resampling. Kernel k2013 = m ∗ s2013
  (eq. 31). Approximately satisfies the projection identity to four bits (eq. 32).
- **Sharp 2021** (eqs. 33–35): s2021 = s2013 ∗ c2021, with c2021 = +17/18 at n=0 and +1/36 at n=±2. Taps:

  | n | 0 | ±1 | ±2 | ±3 |
  |---|---|---|---|---|
  | s2021(n) | 17/12 | −35/144 | 1/24 | −1/144 |

  The taps sum to 1 (checked with exact fractions). The paper reports the projection identity to about nine bits.
  The paper recommends 2021 when full precision matters; 2013 is faster.
- **Order of operations (appendix):**
  - Downsizing: resample with m, then sharpen in output space.
  - Upsizing: sharpen in input space first, then interpolate with m.
- **Claims:** MKS is better than the Lanczos kernels for both directions, and Lanczos produces pixelation when
  upsizing. The evidence is the author's visual tests (paper Figs. 14, 23, 30, 35) on synthetic and test images. I
  found no independent benchmark of MKS for upscaling.
- **Adoption (author's list, not verified):** Facebook (2013) and Instagram (2015) server resize; pica (default since
  Nov 2021); libvips (Nov 2024); Pillow (Mar 2025); PyTorch (Mar 2025); a Rust implementation (May 2024).

### 3.4 Gamma and linear light

- Brasseur's gamma article (ericbrasseur.org/gamma.html) shows darkening from sRGB averaging, and names software that
  is and is not gamma-correct. The examples date from 2007–2009; the software list is dated.
- ImageMagick's guidance: linear light for downsampling; sRGB for upsizing with negative-lobe filters.

### 3.5 Learned super-resolution (for completeness)

Real-ESRGAN, SwinIR (arXiv 2108.10257), Blau & Michaeli (arXiv 1711.06077: the perception–distortion trade-off, so
PSNR/SSIM are insufficient), AIM 2025 efficient perceptual SR (arXiv 2510.12765: small models can beat Real-ESRGAN under
5M params / 2000 GFLOPs), and the 2026 diffusion SR survey (Springer, *Discover Artificial Intelligence* 6:1494, read
as abstract and introduction only). None of these belongs in the thumbnail path: they generate detail, and the
thumbnail path never upscales.

### 3.6 Qt

- Qt Quick `Image.mipmap` (from the source of `qquickimage.cpp`, code.qt.io): "uses mipmap filtering when scaled or
  transformed. Mipmap filtering gives better visual quality when scaling down compared to smooth, but it may come at a
  performance cost (both when initializing the image and during rendering). By default, this property is set to false."
- `Image.sourceSize` decodes at a reduced size. Its doc text was not extracted in full.
- doc.qt.io timed out during this research. Behaviour was measured instead (below).

## 4. Probes

Scratch folder: `C:\Users\Torres\AppData\Local\Temp\claude\2c16f13ff155207f\9b44fd05-9b8e-4122-b87e-0b21678e325e\scratchpad\probe`.

Shared conventions: 220 px target; reference = today's regeneration, unencoded (draft-decoded source to ≥4× target,
then Lanczos3). Scores are SSIMULACRA2 (libjxl 0.11.1; about 90 is visually lossless, 70 high, 50 medium). The
SSIMULACRA2 ceiling for an identical filter is about 97 in our numpy-vs-PIL check, so differences under about 3
points are noise.

### 4.1 Approaches (300-file Pinterest sample from the cache, 800 px → 220 px)

Results in `probe\results.json` and `probe\ss2_results.json`. The summary of those runs:

- **Qt grid, 800 px plain:** worst; mean 72.8 across 120 cells. Mipmap lifts it (78.2 vs 72.4 on a 32-cell Windows
  run; the offscreen platform ignores mipmap).
- **Qt grid, sized decode (`sourceSize`):** mean 80.2, but the 5th percentile is 62.6.
- **400 px level scaled to 220 px:** mean 82.5, 27 KB per cell, load 0.32 s. Best quality, largest bytes.
- **B1 (derive 220 from the cached 800 px, Lanczos, encode):** mean 80.9, 10.7 KB, load 0.13 s.
- **R (regenerate from original, encoded, what the cache stores):** mean 81.3, load 0.09 s.
- **Embedded previews:** not viable (about 1 in 300 files have a usable one).

Conclusion from these runs: encoding the 220 px level costs quality. A pyramid with a 2× cover looks best on
quality, and B1 gives about the same quality as regeneration at a fraction of the bytes.

### 4.2 Filters (120 files, sRGB and linear light)

Source: `probe\filters_run.txt`. "SS2 sRGB ref" is against today's sRGB Lanczos3. "SS2 lin ref" is against linear
Lanczos3 (the linear Lanczos3 row is self-referential, so ignore its 100). dLum is the mean luminance change against
sRGB Lanczos3. Zone SSIM is against an analytic zone plate (8× box truth, in each space).

| Candidate | SS2 sRGB ref | SS2 lin ref | dLum | Zone SSIM |
|---|---|---|---|---|
| Lanczos3 sRGB (current) | 97.15 | 82.69 | 0.00 | 0.750 |
| MKS2021 sRGB (filters_mks2021.txt) | 95.71 | 81.85 | 0.00 | 0.755 |
| MKS2013+sharp sRGB | 92.59 | 83.17 | −0.01 | 0.754 |
| Catmull-Rom sRGB | 93.48 | 80.49 | 0.00 | 0.749 |
| Lanczos2 sRGB | 93.63 | 80.58 | 0.00 | 0.749 |
| Mitchell sRGB | 87.89 | 76.97 | 0.01 | 0.755 |
| Box sRGB | 87.52 | 78.10 | 0.01 | 0.571 |
| Bilinear sRGB | 86.11 | 75.60 | 0.01 | 0.733 |
| MKS2013 plain sRGB | 80.78 | 71.80 | 0.01 | 0.765 |
| MKS2021 linear (filters_mks2021.txt) | 83.90 | 94.14 | +0.60 | 0.736 |
| Lanczos3 linear | 83.40 | (self) | +0.55 | 0.732 |
| MKS2013+sharp linear | 81.69 | 90.73 | −0.06 | 0.731 |

Notes:

- The sRGB candidates are all judged against an sRGB reference, and the linear ones against a linear one. The two
  columns disagree on the ranking. Neither reference is ground truth, so the ranking depends on which reference you
  accept. The robust facts are: linear light brightens by about 0.5–1.5 levels, and it shifts SSIMULACRA2 by about
  14 points in both directions.
- The zone-plate SSIM spread is small, and the metric is crude. Read it as a sanity check, not a ranking.
- The `resize ms` column in these runs is numpy, not the app. Not comparable.

### 4.3 Encode quality (Lanczos3 sRGB, encoded vs unencoded reference, 120 files)

| Quality | Bytes | SSIMULACRA2 |
|---|---|---|
| q75 | 8,039 | 76.5 |
| q85 | 10,487 | 81.3 |
| q90 | 12,915 | 84.6 |
| q95 | 17,994 | 88.4 |

Each step up costs about 23% more bytes for about 3–4 points.

### 4.4 Stage times (serial, 30 files, PIL)

Decode (draft) 7.5 ms, resize 6.7 ms, encode q85 0.5 ms. Decode and resize cost about the same; encode is negligible.
These numbers come from a sample of ~1280 px Pinterest JPEGs. Camera originals would decode more slowly; RAW far more.

### 4.5 Ladder sizes (sample JPEG bytes, extrapolated to the 189,519 thumbs)

| Height | Mean bytes per thumb | Whole set |
|---|---|---|
| 220 | 10,488 | ~2.0 GB (matches the real cache) |
| 400 | 26,492 | ~5.0 GB |
| 800 | 77,031 | ~14.7 GB (matches the real cache) |

A 220 / 400 / 800 ladder would hold about 21.7 GB. The earlier area-based estimate for 400 px (3.7 GB) was too low,
because JPEG bytes don't scale with area.

### 4.6 Brightness on the real library (1,000 random catalog images)

Source: `probe\library_brightness.json`. Draft decode, Lanczos3 at 220 px, sRGB vs linear light.

| Measure | Value (levels) |
|---|---|
| Mean dLum (linear − sRGB) | +0.59 |
| Median | +0.46 |
| 5th / 95th percentile | +0.09 / +1.55 |
| Min / max | −0.03 / +3.54 |
| Files brighter by ≥1 level | 15.5% |
| Files darker at all | 0.3% |

By contrast (sRGB luma std): 20–40 → +0.32; 40–60 → +0.48; ≥60 → +0.70. By brightness, the effect is flat (+0.47 to
+0.63). Largest brightening: +3.54, on a high-contrast image (mean 126, std 73).

This matches the 120-file sample (+0.55). The library is almost all JPEG (187,471 of 187,807 images; mean 144 KB).

## 5. Not done

- **Upscaling with MKS.** Not tested here. The paper's upsizing claims are the author's visual tests. A test would
  compare MKS2021, Lanczos3 and bicubic at 220 → 440 px against higher-resolution originals (fidelity to real pixels,
  not detail invention).
- **Rust implementation of MKS.** Not written. `fast_image_resize` `FilterType` has no MKS. Two options:
  - A `Custom` filter with the combined kernel k(x) = Σₙ sₙ m(x+n). This is one pass, but its normalisation and
    support scaling in `fast_image_resize` are unverified.
  - Resample with m, then a 7-tap post-pass in output space (the form that was measured). Clear, but an extra pass
    and an extra rounding step.
- **Scroll and frame-rate tests.** Not measured.
- **Cold disk reads** on the real originals. Not measured.
- **Camera originals and RAW.** The sample is Pinterest JPEG only.
- **Qt `sourceSize` documentation.** Not extracted in full.

## 6. Caveats

- Samples are small: 300 files for approaches, 120 for filters and encode, 1,000 for brightness. The brightness run
  is the largest and most representative.
- Two reference choices (sRGB and linear) give opposite rankings. The brightness measurement does not depend on a
  reference, but the SSIMULACRA2 numbers do.
- Our numpy resize matches PIL to about 0.1 levels (checked). The Rust path is unverified.
- The MKS adoption list is the author's, not independently checked.
- Sharpening raises SSIMULACRA2 on some images and lowers it on others. Visual checks on the real library have not been
  done.

## 7. Decision

Options:

1. **Display path only (recommended).** Use MKS2021 for thumbnails (`to_height`, `fit_box`) and the viewer's
   `resize_region`. Keep Lanczos3 for every model input (PDQ, ThumbHash, and the SigLIP/SSCD inputs). PLAN §3 rule 8
   then holds, because the model path does not change. The cache needs a filter version so old and new thumbnails can
   be told apart, or a regeneration pass.
2. **MKS everywhere.** Also changes PDQ hashes, ThumbHash and every embedding. PLAN §3 rule 8 requires a re-embed of
   the whole library. The GPU pass is about 154 hours for 70M images, so this is expensive and is not a display choice.
3. **Keep Lanczos3.** The measured quality difference is small, and the cache-size and regeneration problems are
   independent of the filter.

Open question: which of these you want before any code changes. The brightness shift for option 1 is about +0.6
levels on average, up to +3.5 on high-contrast images. Whether that is acceptable needs a visual check on the real
library.

## 8. Sources

Read in this research:

- Wikipedia: Lanczos resampling; Bicubic interpolation; Mitchell–Netravali filters; Mipmap; Anti-aliasing filter;
  Nyquist–Shannon sampling theorem; Sinc filter; Super-resolution imaging; Bilinear interpolation; Nearest-neighbour
  interpolation
- ImageMagick: <https://usage.imagemagick.org/filter/nicolas/> (Robidoux filter page)
- Pillow concepts: <https://pillow.readthedocs.io/en/stable/handbook/concepts.html>
- libvips enum `Kernel`: <https://www.libvips.org/API/current/enum.Kernel.html> (version 8.18)
- fast_image_resize `FilterType` (docs.rs): <https://docs.rs/fast_image_resize/latest/fast_image_resize/enum.FilterType.html>
- Eric Brasseur, gamma error in picture scaling: <https://www.ericbrasseur.org/gamma.html>
- Magic kernel page: <https://johncostella.com/magic/>
- Costella 2021 paper: <https://johncostella.com/magic/mks.pdf>
- pavelp.cz resampling devlog: <https://www.pavelp.cz/posts/eng-image-resampling/>
- Qt Quick Image, mipmap and sourceSize: <https://code.qt.io/cgit/qt/qtdeclarative.git/tree/src/quick/items/qquickimage.cpp?h=dev>
- Real-ESRGAN: <https://github.com/xinntao/Real-ESRGAN>
- SwinIR: arXiv 2108.10257
- Blau & Michaeli, perception–distortion trade-off: arXiv 1711.06077
- AIM 2025 efficient perceptual SR: arXiv 2510.12765
- Image super-resolution with diffusion models, survey: <https://link.springer.com/article/10.1007/s44163-026-01674-6>

Not reachable: doc.qt.io (timed out twice); github.com/Cykooz/fast_image_resize (connection reset); cambridgeincolour.com
(Cloudflare 403); cloudinary/ssimulacra2 (not loaded).
