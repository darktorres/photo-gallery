# Thumbnails

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Thumbnails (`eval/bench_thumbs.py`, 300 random images, 2026-10-30)

Grid thumbnails for a justified layout: height = the row height in device pixels, width from the
aspect ratio, never enlarged. Quality is SSIMULACRA2 (libjxl's tool; ~90 visually lossless, 80
very high, 70 high, 50 medium). Times are one thread, files in RAM, machine in use.

**Resize** (from the original file to the exact thumbnail size; score vs a float Lanczos3 resize of the
full decode, 384 px; zone plate = worst-case aliasing test):

| Pipeline | Score mean / p10 | ms | Zone plate |
|---|---|---|---|
| Full decode + Lanczos3 | 96.6 / 95.9 | 19.0 | 94.6 |
| **DCT-scaled decode keeping ≥ 4× the target + Lanczos3** | **96.6 / 95.9** | 18.5 | 73.9 |
| DCT-scaled decode keeping ≥ 2× + Lanczos3 | 95.7 / 94.7 | 16.4 | 58.8 |
| libvips `thumbnail` (shrink-on-load + Lanczos3) | 95.5 / 94.4 | 13.2 | 58.7 |
| DCT-scaled decode keeping ≥ 1× + Lanczos3 | 87.5 / 75.5 | 7.7 | 36.7 |
| Full decode + antialiased bilinear | 86.0 / 82.3 | 13.0 | 43.9 |

- **Lanczos3 from at least 4× the target is the thumbnail path**: the full-decode quality, and on large
  originals (camera photos) still the 1/2–1/4 DCT-scaled decode. On this sample (median 800×1200)
  there is little to scale, hence the same time. libvips (= 2× margin) is softer on some images
  (p10 at 256 px: 87.7 vs 96.5).
- The embedder's input (DCT-scaled to just ≥ 320 px, "Decode benchmark") is fine for SSCD but not
  for thumbnails (p10 75). Bilinear is too soft. The ingest decodes once and makes both outputs.

**Encode** at the same quality, 384 px (mean bytes per thumbnail; × 70M):

| Format | Score 70 | Score 80 | Score 85 | Encode ms | Decode ms |
|---|---|---|---|---|---|
| JPEG (libjpeg-turbo, 4:2:0) | 13.8 KB (964 GB) | 21.9 KB (1.53 TB) | 29.6 KB (2.07 TB) | 0.9 | 0.6 |
| JPEG (jpegli, 4:2:0, sequential) | 12.2 KB (853 GB) | 19.9 KB (1.39 TB) | – | 1.4 | 0.7 |
| JPEG (jpegli defaults: 4:4:4, progressive) | 12.8 KB | 19.7 KB | 26.4 KB | 3.4 | 1.7 |
| JPEG (mozjpeg) | 12.2 KB | 19.9 KB | 28.5 KB | 12.7 | 0.8 |
| WebP (method 4) | 10.7 KB (747 GB) | 16.9 KB (1.19 TB) | 23.4 KB (1.64 TB) | 20 | 2.7 |
| JPEG XL (effort 7) | 10.8 KB | 16.4 KB | 21.7 KB | 75 | 5.0 |
| AVIF (speed 6) | 8.2 KB (571 GB) | 13.6 KB (949 GB) | 19.0 KB (1.33 TB) | 93 | 3.3 |

Other row heights at score 80: 256 px: JPEG 815 GB, WebP 611 GB, AVIF 512 GB; 512 px: JPEG 2.37 TB,
WebP 1.88 TB, AVIF 1.47 TB.

- **Disk is the binding constraint.** D: has ~690 GB free. PLAN's ~350 GB (5 KB at ~200 px) only holds
  around 256 px at score 60–70. At 384 px only AVIF at about score 70 fits.
- **AVIF is the smallest at every quality** (score 80: −38% vs JPEG, −20% vs WebP), but at speed 6 it
  encodes in ~0.1 s per 384 px thumbnail (~7 days of all 12 threads for 70M) and decodes 5× slower than
  JPEG (3 ms; a screen of 60 thumbnails is ~15 ms on 12 threads, fine).
- **AVIF encoder speed** (100 of the images, 384 px; bytes at score 70 / 80, encode ms):

  | AVIF speed | Score 70 | Score 80 | Encode ms | 70M on 12 threads |
  |---|---|---|---|---|
  | 6 (default) | ~7.6 KB | 12.9 KB | 110 | ~7.4 days |
  | **8** | 8.5 KB | 13.7 KB | 40 | ~2.7 days |
  | 9 (10 is identical) | 9.8 KB | 15.7 KB | 9 | ~0.6 days |
  | WebP method 4, for reference | 10.2 KB | 16.2 KB | 21 | ~1.4 days |

  Speed 8 keeps most of AVIF's advantage (−15% vs WebP) at 2× WebP's encode time; speed 9 is
  faster than WebP and still a little smaller.
- JPEG XL loses to AVIF on size and to WebP on speed. Progressive JPEG is ~2× slower to decode for
  0–3% fewer bytes. jpegli 4:2:0 sequential is the best JPEG (−9%, decodes like any JPEG); mozjpeg
  matches its size at 10× the encode time.

**Thumbnails as a source for later models** (SSCD fp16 on the thumbnail vs on the original, cosine):

| Height | Uncompressed thumbnail: min / p1 / p50 | As JPEG q85 (score ~82): min / p1 / p50 |
|---|---|---|
| 256 px | 0.730 / 0.872 / 0.984 | 0.661 / 0.790 / 0.931 |
| 384 px | 0.844 / 0.981 / 0.998 | 0.800 / 0.897 / 0.978 |
| 512 px | 0.912 / 0.997 / 0.999 | 0.889 / 0.948 / 0.992 |

- Compression moves the embedding more than the size does, and tall images (a 256 px-high
  infographic is ~120 px wide) lose the most. A re-analysis source would need ≥ 512 px at near-lossless
  quality, i.e. several TB. **So thumbnails are sized for display only** (PLAN §9 decision 1), and any
  model wanted later should run in the ingest pass (decision 2), which reads the originals anyway.
