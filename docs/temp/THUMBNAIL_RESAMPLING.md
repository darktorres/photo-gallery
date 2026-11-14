# Thumbnail resampling: findings and decision

Status: research and probes done (2026-11-06 to 2026-11-13). Decision (2026-11-13): MKS2021 in linear light
everywhere, display and model inputs (§7). Implemented in the codec and the import version bumped to 2; the catalog is
not yet recomputed and the thumbnail cache is dropped (see [§7.2](#72-linear-light-everywhere-code-done-2026-11-13-catalog-not-yet-recomputed)).

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

### 4.7 Follow-up probes (2026-11-13)

Shared inputs: 40 files from the probe set, draft-decoded to 800 px high and Lanczos-resized to 800 (the probe's
stand-in for the cache master, not the app's exact master).

- **Rust MKS2021** (scratch crate, not in the repo; `fast_image_resize` fork `FilterType::Custom`). The kernel is
  k(x) = Σₙ sₙ·m(x−n), n = −3..3, support 4.5. Against numpy MKS2021 the mean difference is 0.111 levels per
  channel; the Lanczos3 control is 0.112. The gap comes from the fork's u8 vertical buffer, not the kernel. Serial,
  800 → 220: MKS2021 1.08 ms per image, Lanczos3 0.88 ms (1.23×). Single machine, serial build only.
- **Upscale 220 → 440** (120 files; SSIMULACRA2 against a 440 px Lanczos3 downscale of the original): Lanczos3 66.9,
  MKS2021 66.0, bicubic 65.2, bilinear 58.4. Rust MKS2021 and Lanczos3 are within 0.03 levels of each other against
  the 440 truth. Lanczos3 is ahead of MKS2021 in the Python probe.
- **Derivation from an encoded 800 px master** (120 files; every output encoded at q85 and scored against an
  unencoded Lanczos3 truth at that height):

  | Height | Lanczos3 from master | MKS2021 from master | Regenerate, encoded | Bytes (derived / regenerated) |
  |---|---|---|---|---|
  | 120 | 61.5 | 61.4 | 61.6 | 4268 / 4270 |
  | 220 | 80.9 | 80.7 | 81.3 | 10487 / 10489 |
  | 300 | 81.4 | 81.3 | 82.0 | 16855 / 16832 |
  | 400 | 80.2 | 80.0 | 82.5 | 26506 / 26347 |
  | 560 | 80.7 | 80.6 | 82.8 | 45521 / 45269 |

  Derived heights lose 0.3 to 2.5 points against regeneration, and the loss grows with height. Bytes are the same.
  Lanczos3 is 0.1 to 0.3 points ahead of MKS2021 at every height. (An earlier run scored the derived images before
  encoding and is void.)
- **Camera originals** (100 JPEGs with camera EXIF, median 3840 × 2160). Against the sRGB Lanczos3 reference: MKS2021
  sRGB 94.35 (p5 93.11), Mitchell 86.23, MKS2021 linear 79.21, Lanczos3 linear 78.86 (mean luminance +0.8). The
  reference is itself Lanczos3, so this favours Lanczos3.
- **Brightness, visual check.** Four of the 40 inputs, chosen for the largest linear-minus-sRGB luminance differences
  (+1.21 to +0.03). At 2× zoom, sRGB and linear Lanczos3 look alike; linear is slightly brighter in the mid-tones.
  MKS2021 has marginally more edge contrast. No visible defect in any of them. This is a qualitative check, not a
  ranking.
- **Cold reads.** Not measured. 300 untouched catalog JPEGs read at a median 1.1 ms on first read (p95 5.1 ms), so
  they were still in the Windows file cache. Dropping that cache needs admin rights. The cost is filter-independent
  in any case.
- **RAW.** Not measurable. The catalog holds only JPEG, PNG, WebP and video.
- **Adoption (web, §3.3).** Verified: pica's default (`mks2013`, CHANGELOG 8.0.0, 2021-11-22). Found only on the
  author's page: Facebook, Instagram. libvips lists MKS2013/MKS2021 in its enum, with an add date that does not match
  the claim; its thumbnail default is Lanczos3. Pillow: PR #8811 is open and I could not confirm it merged or became a
  default; `resize` and `thumbnail` default to BICUBIC. PyTorch: no evidence. A Rust crate, `magic-kernel`, implements
  Sharp2013 and Sharp2021. Google, Apple: no public evidence found. Adobe, Microsoft and browsers were not checked.
- **Qt scroll and frame pacing** (320 cells in 37 rows, constant 10 px per frame, 2 passes, 3 repeats; Windows
  Direct3D 11 with vsync and uncapped; offscreen). Median frame time 16.2 to 16.5 ms and p95 about 18.0 ms in every
  configuration. The Lanczos3 and MKS2021 differences are inside run-to-run noise, and mipmap on or off makes no
  difference. The ~16 ms median is the animation's 60 Hz cadence, so this test shows pacing, not the filter's cost.
  Frames over 25 ms were rare and did not point to either filter (one 881 ms stall in a Lanczos3 mipmap run, not
  investigated). Per-frame draw cost below 1 ms cannot be resolved with Qt's whole-millisecond render log.
- **Mipmap validity.** Windows reports the Direct3D 11 RHI, so mipmap is applied there. Offscreen uses the software
  renderer, so offscreen runs don't exercise it. The grid draws every thumbnail 1:1, so mipmap is never exercised by
  the grid at all; it would only matter for scaled-down images.
- **Qt `sourceSize` and `mipmap` text** (`qquickimage.cpp` in Qt 6.12.0 source). `sourceSize`: "the scaled width and
  height of the full-frame image"; "if the image's actual size is larger than the sourceSize, the image is scaled
  down"; for JPEG, "the whole image will never actually be loaded into memory." `mipmap`: "mipmap filtering gives
  better visual quality when scaling down compared to smooth, but it may come at a performance cost (both when
  initializing the image and during rendering). By default, this property is set to false."
- **Not measured:** the real gallery's justified layout, flick scrolling on real display hardware, scaled-down
  thumbnails, and cold reads (the files were cached).

### 4.8 SSCD input colour space (2026-11-13)

Question: does a linear-light resize before SSCD change retrieval of duplicates? Labels already in the repo: the
`dups2` duplicate groups (positives; pairs across groups are negatives) and the planted edits (each copy's source is
known). Model: `sscd_disc_mixup` fp32, 320 px squash, ImageNet normalisation, as `eval/models.py`. Only the colour
space differs between `srgb` (float sRGB bilinear, antialiased) and `lin` (resize in linear light, then encoded back).
`prod` is the current PIL path, shown as a control.

Sample: 300 duplicate groups (605 images); 8 planted edits × 60 sources (480 sources, 480 copies). Scripts and output
are in the session scratchpad (`colour_eval\run.py`, `results.json`).

- **Control:** the float sRGB resize and the production PIL resize give embeddings with mean cosine 0.9997. The
  resampler barely matters.
- **Embedding shift from colour space:** lin vs srgb, mean cosine 0.9975 (minimum 0.98). The embeddings move slightly.
- **Retrieval:** R@1 = 1.00 and R@10 = 1.00 for all three inputs, on groups and on planted copies. AUC 1.00. This
  test saturates, so it can't tell the inputs apart.
- **Ranking margin** (true match minus best non-match, per query): planted srgb 0.5940, lin 0.5940, difference
  0.0000 (sd 0.005). Groups: 0.7030 and 0.7030, difference 0.0000 (sd 0.004). About half the queries get a smaller
  margin under linear light and half a larger one. Per edit, the differences range from −0.0024 to +0.0009.
- **Conclusion for SSCD on these data:** linear light makes no measurable difference to duplicate retrieval. The test
  is easy, though: margins are large and the hard cases (heavy crops, similar-but-different pictures) are not well
  represented. PDQ was not run in this step.

### 4.9 Harder SSCD test and PDQ (2026-11-13)

- **Harder SSCD test:** heavy edits (crop 60% and 80%, crop with flip, letterbox, downscale, grayscale, JPEG q30,
  colour shift; 60 copies each, 480 in all) and hard negatives mined from the sample under sRGB. Still saturated: R@1
  1.00 for every input, AUC 1.00, and the copy–source margin differs by 0.0002 (lin 0.6018, sRGB 0.6020). The sample
  contains no near-miss negatives, so this test cannot separate the inputs.
- **PDQ:** stopped before it finished. It hashes the same images, comparing the production Lanczos path, a sRGB
  control and linear light (Hamming distance 31 as the match threshold). No result.
- **Size of the test:** 300 duplicate groups (605 images, from 679 groups in `dups2`), 480 planted copies with their
  480 sources per run, about 183,000 group pairs. About 1% of the library's images.

### 4.10 Linear-light MKS2021 in the Rust path, and upscaling (2026-11-13)

Rust (scratch crate, not in the repo), 40 inputs, 800 → 220 px, release, serial, Intel i7-8700, Windows 10. Three
full runs, forward and reverse order.

| Path | Whole pipeline, ms/image | Resize only, ms/image | × u8 sRGB MKS2021 (resize) |
|---|---|---|---|
| u8 sRGB Lanczos3 | — | 1.08 (0.90–1.29) | 0.83 |
| u8 sRGB MKS2021 | — | 1.30 (1.22–1.36) | 1.00 |
| f32 linear MKS2021 | 5.24 (4.90–5.80) | 4.17 | 3.2 |
| u16 linear MKS2021 | 5.52 (5.13–5.97) | 4.86 | 3.7 |

Parity with the numpy linear MKS2021 reference (mean absolute difference per channel, 0–255): f32 0.011, u16 0.028.
The u8 sRGB path against its numpy reference was 0.111 (§4.7). The colour space changes output by about 1 level on
average and brightens it by about 0.5 levels.

- **Edge pixels.** Every f32 pixel that differs by 5 or more lies within 3 px of the frame edge. The interior never
  differs by 5 or more. The edge handling of the fork and of the numpy reference differs slightly; not investigated.
- **u16.** It has 2,302 pixels off by 5 or more, most of them in the interior. The likely cause is clipping of the
  linear undershoot in the fork's u16 pass. Inferred from the source, not tested.
- **The conversions cost** 0.7 ms (u16) to 1.1 ms (f32) per image.

Upscaling, 220 → 440 (60 inputs, SSIMULACRA2): see the table below. Within each pipeline, MKS2021 and Lanczos3 are
within 0.3 points. Each pipeline scores higher against the truth in its own colour space, mostly because of brightness.

| Pipeline | vs sRGB truth | vs linear truth |
|---|---|---|
| sRGB, MKS2021 | 66.2 | 62.6 |
| sRGB, Lanczos3 | 66.5 | 62.9 |
| linear, MKS2021 | 65.1 | 66.9 |
| linear, Lanczos3 | 64.8 | 66.6 |

### 4.11 PDQ with linear light (2026-11-13)

Same images as §4.9 (300 duplicate groups, 605 images; 8 heavy edits × 60 copies with their sources). The hash is
`pdqhash.compute` on 8-bit sRGB bytes. Only the downscale to ≤ 512 px differs: `prod` is the repo's PIL Lanczos
thumbnail, `srgb` is Lanczos3 on sRGB values (control), `lin` is Lanczos3 in linear light, encoded back to sRGB. Match
threshold d ≤ 31.

| Metric | prod | srgb | lin |
|---|---|---|---|
| Copy match rate (d ≤ 31) | 0.500 | 0.500 | 0.500 |
| Copy R@1 among 480 sources | 0.573 | 0.573 | 0.563 |
| Copy d, median / p90 | 47 / 130 | 47 / 130 | 46 / 130 |
| Group-pair match rate | 0.842 | 0.842 | 0.845 |
| Cross-group false match rate | 0 | 0 | 0 |

- Linear minus sRGB: copy match rate 0.0 (95% CI 0.0 to 0.0), copy R@1 −1.0 points (CI −2.3 to +0.2), group-pair
  match rate +0.3 points. No significant difference.
- The production path (`prod`) and the sRGB control agree on every metric.
- Half the heavy copies fail the d ≤ 31 threshold under every input. That is a property of PDQ on these edits, not of
  the colour space.
- **Per-pair flips** (second run, per-pair distances saved; `results_pdq_flips.json`):
  - Group pairs: 310 positive pairs, 1 gained a match in linear light, 0 lost. 182,400 negative pairs, 0 new false
    matches.
  - Copy vs own source: 480 pairs, 0 flips in either direction.
  - Copy vs the other 479 sources: 229,920 pairs, 0 new false matches.
  - So 1 flip among 790 measured matches or near-matches. This sample is random pairs within groups and the heavy
    edits, not the near-threshold candidates the analysis pass actually compares. The flip rate for those could be
    higher. Not measured.

### 4.12 Brightness by output size (2026-11-13)

MKS2021 in linear light minus MKS2021 in sRGB, mean luminance in levels, on 200 random library originals (read-only
catalog sample). Full decode of every original for every size, so only the resize differs. A first run used a
draft decode whose scale depended on the size; it confounded the size effect, and its numbers are not used.

| Output height | Mean | Median | p5 | p95 | Max | ≥ 1 level brighter | Darker |
|---|---|---|---|---|---|---|---|
| 220 px | +0.85 | +0.65 | +0.15 | +2.12 | +6.67 | 28.5% | 0.0% |
| 440 px | +0.39 | +0.25 | +0.03 | +1.10 | +4.26 | 5.5% | 0.5% |
| 1080 px | +0.10 | +0.05 | −0.01 | +0.44 | +1.30 | 1.0% | 10.5% |

- The brightening is largest at the smallest size, where each output pixel averages the most source pixels. At
  1080 px the shift is small and changes sign on some images.
- The 220 px mean here (+0.85) is above the +0.59 from the earlier 1000-file sample (§4.6). That sample used Lanczos3
  with a draft decode, and this one uses MKS2021 with a full decode, so the figures are not directly comparable.
- The viewer's zoom sizes were not measured separately. 1080 px is the nearest proxy here.

### 4.13 Intermediate precision: f32, u16, u8 (2026-11-13)

The open decision in §7.2: what the linear-light working buffer holds. Scratch crate `scratchpad\mks\rust` and the
precision scripts in `scratchpad\precision\`. Reference: float64 MKS2021 with the fork's window rule, no clipping. 40
inputs at 800 px, downscaled to 220 px height, then encoded to 8-bit sRGB and rounded once. Timings are per image,
serial, median.

| Path | Max diff (levels) | Mean | Samples ≥ 1 | Samples ≥ 5 | ms per image |
|---|---|---|---|---|---|
| f32 linear | 1 | 0.0000 | a handful | 0 | 4.4 |
| u16 linear | 47 | 0.017 | 0.72% | 3,844 | 3.8–4.0 |
| u8 linear (no float) | 42 | 0.56 | 37.6% | 54,301 | 1.8 |

The u8 sRGB path with no conversion takes 0.9 ms per image.

- **The f32 edge difference (§4.10) is the edge rule, not f32.** The numpy reference uses the paper's two-pass form
  with edge replication. The fork uses one combined kernel with a clipped, renormalised window. Against the fork's
  rule in float64, f32 is exact. The 111 band samples that differed from numpy are all within 3 px of the border.
- **The u16 gap is clipping of the sharpen overshoot in the horizontal pass.** The taps push about 0.8% of vertical-pass
  values outside [0, 1]. f32 keeps them, and the final encode clamps once. u16 cannot hold them. Clamping the vertical
  pass alone does not reproduce the gap; clamping the horizontal intermediate to [0, 1] reproduces it exactly (max 1,
  no sample off by 5 or more). Input quantisation cannot produce errors of 47 levels.
- **Without the sharpen taps** (plain magic kernel), u16 is within 1 level everywhere, so the gap is the sharpening.
- Upscaling 220 → 440: f32 is exact against the reference. u16 reaches 72 levels off, with 16,643 samples off by 5 or
  more (the same clipping, not separately isolated). u8 linear reaches 277,101 samples off by 5 or more.

**Decision: f32** (`pg_codec::resize`). It is the only path that matches the reference, and u16 saves about 0.4 ms per
image while clipping thousands of samples. The inputs are 40 scratch images, not the library, so this is a precision
result, not a library result.

## 5. Not done

Done in §4.7: upscaling, the Rust implementation (a `Custom` filter with k(x) = Σₙ sₙ m(x−n), verified against
numpy), the camera-original comparison, and the brightness visual check.

- **Scroll and frame-rate tests.** Done in §4.7 for pacing. The filter's per-frame cost is below what the probe can resolve.
- **Cold disk reads** on the real originals. Not measurable without admin rights; the probe read cached files.
- **RAW.** None in the library.
- **Qt `sourceSize` documentation.** Done (§4.7).
- **Sharpening, visually.** Only the 4-image check in §4.7. Not a systematic review.
- **Derivation from a 400 px master or from other chains.** Not measured; only the 800 px master.
- **Cache filter version.** Design, not measurement. The cache still stores no record of the filter.

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

The options that were weighed before the decision (display only, MKS everywhere, keep Lanczos3) are superseded and
removed from this section. The brightness shift the decision accepts is about +0.85 levels on average at 220 px, and
up to +3.5 on high-contrast images (§4.12 measures it by output size). The library check is in §7.2.

Visual check, one image (2026-11-13, the user's judgement): the linear-light resizes look closer to the original, and
the sRGB resizes look darker than the original. This fits the gamma explanation in §3.4. The SSIMULACRA2 rankings in
§4.2 used an sRGB reference, so they could not show this. A linear pipeline would need a conversion before and after
the resize (not measured in the Rust path), and it changes every cached thumbnail's brightness.

**Decision (2026-11-13): linear light everywhere.** MKS2021 in linear light for the display path and for the model
inputs (PDQ, ThumbHash, and SSCD/SigLIP when built). The stored hashes change, so the `analysis` rows (230,812) and
`pdq_pairs` must be recomputed. The measurements behind the choice are in §4.8 and §4.9. They do not show a
retrieval difference between the colour spaces for SSCD, and the PDQ test was stopped before it finished, so the
decision rests on the visual check and the gamma argument (§3.4), not on a model measurement.

## 7.1 Thumbnail tiers (open, 2026-11-12)

Undecided. Keep thumbnails as they are: one entry per exact height, made from the original. Nothing in the code
depends on this section yet.

**The question.** Should the cache keep more than one height of the same image, and derive a smaller one from a
cached larger one, instead of regenerating each height from the original? Two named shapes:

- **Ladder.** A fixed set of stored heights (for example 220 / 400 / 800 px). Each height is stored, and a request
  uses the nearest stored height at least as tall.
- **Master with derived heights.** One stored master (for example 800 px). Every other height is derived from it,
  and may be cached after it is derived.

Mipmaps are a different thing. Qt's `Image.mipmap` filters at display time and stores nothing (§3.6). The tiers
question is about what the cache stores. The two can be chosen separately.

**Why it is open.**

- The 800 px set is most of the cache: 14.7 GB of 16.7 GB used, against a 50 GB cap (§1). A 220 / 400 / 800 ladder
  would hold about 21.7 GB (§4.5).
- Changing the grid height regenerates every thumbnail: about 190k images, 6–8 minutes (§1). A derived height would
  avoid that, if the derivation is good enough.
- The derivation from an encoded master has been measured once, for 800 → 220 px (§4.1, option B1): mean score 80.9 at
  10.7 KB per cell, against 81.3 for regeneration from the original at about the same load time. The 400 px level
  scaled to 220 px scored best (82.5) but took 27 KB per cell. Derivation from the 800 px master is now measured at 120 to 560 px
  (§4.7, 120 files): derived heights score 0.3 to 2.5 points below regeneration, and the gap grows with height (about
  2 points at 400 and 560). The real library is still unmeasured beyond the probe sample.
- A derived height is a second encode of the same pixels, so each step down adds encoder loss. Derived from an encoded
  master, a height may differ visibly from one made from the original. This is not measured.

**What any tiers scheme needs first.** The cache keeps no record of which filter or version made an entry (§2).
Tiers mix filters and derivations in one cache, so the cache needs a version per entry (or per height) before a
second kind of entry is written. The version is also what lets the cache tell an old entry from a new one after a
filter change (§7, option 1). This is needed whichever shape is chosen.

**Options, not yet chosen.**

1. **Keep the current cache.** One entry per exact height, regenerated on a height change. No new cost; a height
   change still takes 6–8 minutes.
2. **Ladder of fixed heights.** Store, for example, 220 / 400 / 800 px. Lowest risk for quality, because each stored
   height is made from the original. Costs about 21.7 GB at the library's current size, so the 50 GB cap leaves little
   room for other heights.
3. **Master with derived heights.** Store 800 px (or the largest used) and derive others from it, optionally caching
   the derived ones. Smallest storage, and a height change becomes a derivation rather than a full regeneration. The
   quality cost is the derivation (open above), so it needs a visual check on the real library.

**Decisions needed before any code.**

- Which heights the grid uses. The row height is adjustable at runtime (`PLAN.md` §6, §9 item 1), so this may be
  "any height from the master" rather than a fixed set.
- Whether derived heights are cached, or derived on each request.
- The visual check on the real library for the derived heights (the brightness and sharpness results in §4.6 and §6
  are the reference points).
- Which option. This section has not recommended one.

**Relation to other work.** `DECODE_ONCE_PLAN.md` treats thumbnails as display artifacts and does not depend on this
choice. It keeps two thumbnail definitions (ingest from the full decode, catch-up and on-demand from the T4 decode),
and a tiers scheme would change the second of those. Revisit it when this section is decided.

## 7.2 Linear light everywhere (code done 2026-11-13; catalog not yet recomputed)

Done: `crates/codec/src/resize.rs` (the one resizer every caller uses, f32 linear light, §4.13), `kernel.rs` (MKS2021),
`linear.rs` (the sRGB tables). `IMPORT_VERSION` 1 → 2 (`crates/analysis/src/analyze.rs`). Tests: codec 21, analysis 19,
render 10 pass; clippy is clean on those crates; `cargo check --workspace --all-targets` passes.

Done to the data: the catalog was backed up to `D:\PhotoGallery\catalog\catalog.sqlite.pre-linear.bak`, and the
thumbnail cache (`thumbs.sqlite`, 230,811 entries at 220 px) was deleted. Nothing else in the catalog has changed: its
`import_version` is still 1 and all 230,812 analyses are still version 1.

Not done (the user said not to run it): the recompute. The first open of the catalog with a version-2 build sets the
import mark to 2 and makes every analysis pending, which is when the stored PDQ, ThumbHash and pairs get rebuilt. The
comparison sheets wait on regenerated thumbnails, so they wait on the same step. The snapshot of the current hashes
and pairs is in `scratchpad\recompute\before_pdq.bin` and `before_pairs.json`, for the comparison afterwards.

The steps below are the design as it was written; where the built code differs, the code is the reference.

**Scope.** One colour space for every resize whose output is shown or stored: on-demand and background thumbnails
(`to_height`, `fit_box`), the viewer (`resize_region`), the PDQ input (`fit_within` to 512 px,
`crates/analysis/src/analyze.rs:109`), the ThumbHash input (`analyze.rs:190`), and any future embedding input. The filter
is MKS2021, as verified in §4.7 (`Custom` filter, k(x) = Σₙ sₙ·m(x−n), support 4.5).

**Per resize:**

1. Decode as today, with EXIF orientation applied.
2. Convert sRGB to linear light (the sRGB EOTF).
3. Resize with MKS2021, keeping the intermediate in floating point. Not the fork's 8-bit buffer: that buffer caused the
   0.11-level gap in §4.7, and it would also clip or round in linear light.
4. Encode back to sRGB and round to 8 bits once, for anything stored or displayed. The resize is linear for PDQ too.
   What PDQ receives is 0–255 sRGB bytes, because its spec computes luminance on those bytes
   (`hashing.pdf` pp. 10–11, Y = 0.299R + 0.587G + 0.114B).
   - Feeding linear values into PDQ's luminance step would depart from the spec and from the reference
     implementation. Not recommended, not measured.
5. ThumbHash: linear resize to 100 px, then sRGB bytes. Checked: the reference encoder (`thumbhash.js`, evanw/thumbhash)
   computes its planes from the RGBA bytes divided by 255, with no sRGB-to-linear step, and the repo's vendored copy
   reads RGB "as it is" (`vendor/README.md`, thumbhash section). The decoded placeholder is shown as sRGB, so linear
   values fed in would show as a darker placeholder than the image.

**Measurements the design still needs (in order):**

- **Float path cost.** Measured (§4.10): the f32 linear path is about 4 times the u8 sRGB MKS2021 path per image, and
  the u16 linear path is no cheaper and less accurate. Recommendation: f32.
- **Upscaling in linear light.** The viewer upscales between fit and 1:1. §4.7 tested MKS2021 upscaling in sRGB only.
- **Brightness on the library.** Linear light adds about +0.6 levels on average and up to +3.5 on high-contrast images
  (§4.6). Your visual check was one image. Gate: a sampled visual check on about 20 library images, at 220 px and at
  the viewer's sizes, before any cache is regenerated.

**Versioning.** The cache and the analysis index need a pipeline version:

- **Thumbnail cache.** Entries carry no record of their filter today (§2). Add a version per entry, or a directory per
  version. Current size is 16.7 GB of the 50 GB cap (§1). Keeping the old set until the new one is complete peaks at
  about 33 GB, which fits. The on-demand path and the background pass both regenerate by version, not only by height.
- **Analysis index.** The `analysis` table already has a `version` column (catalog schema, checked 2026-11-13). Whether it is the analyzer version needs
  checking in code. If it is, bump it so the background pass recomputes stale rows.
- **Model inputs.** No embeddings exist yet (`PLAN.md` §3 and §6), so nothing has to be re-embedded now. The version must
  be in the index so a later embedding run knows which inputs it was built from.

**Recompute cost (estimates from the numbers already in this doc):**

- **Analysis rows (230,812).** PDQ and ThumbHash change, and so does `pdq_pairs`. The import pass runs at about
  341 files per second on 12 workers (`PLAN.md`, the import measurement). At that rate a full recompute is about 11
  minutes of compute, plus decode time, which the import figure already includes. Treat this as an estimate.
- **Thumbnails (189,519 at 800 px).** A full regeneration takes 6–8 minutes (§1), the same as a height change today.
- **Re-embed.** None (see above).

**Risks.**

- **User review data.** The duplicate groups (`dups2`) and any other review decisions must survive the recompute. Any
  decision that depends on an old hash (PDQ distances, `pdq_pairs`) must be re-checked against the new hashes, not
  silently kept.
- **Brightness change.** Every cached thumbnail changes brightness. The visual check gate above is the mitigation.
- **Two colour paths during the migration.** Old and new entries coexist until the new set is complete. The viewer must
  read each entry's version, not guess.

**Decisions (2026-11-13):**

1. Intermediate precision: **f32**, measured in §4.13. u16 clips the sharpen overshoot and is not cheaper in a way that
   matters; u8 linear is far off.
2. Old cache: **dropped** (the user's choice), not kept alongside. The cache still stores no filter version, so the next
   change to the filter needs the same manual drop, or a version added then (§5).
3. ThumbHash: sRGB bytes out of the linear resize, as the reference encoder expects (step 5 above).

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
