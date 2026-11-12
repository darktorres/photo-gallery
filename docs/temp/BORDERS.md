# Borders

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Border cleaning (Python prototype, 2026-10-28)

The Python prototype, since ported to the gallery and removed (in git history): "Border detector in the gallery"
below. Finds solid (incl. JPEG-noisy colored) and blurred-padding borders and crops them; dry run by
default, writes cleaned copies only with `--apply`, never touches originals. Default output is a
lossless `jpegtran` crop (left/top snapped to the 8/16 px JPEG block).

Sample of 1,500 random test-set images (79 img/s, 4 workers):

- **35 with borders (2.3%)**, all correct on visual review: black letterbox bars, thin
  (5–20 px) dark frames around AI art, white side padding, a purple frame.
- **Rejected look-alikes:** plain studio walls, skies, and white backgrounds. Rules that made the
  difference:
  - **Real padding is digitally flat:** median deviation 0–3 and line color drift ≤ 5; walls
    have 5–7 and 9–17.
  - **Blurred bars have a straight seam (≥ 0.7) and come in equal pairs:** a one-sided "blurred
    bar" is always window light, curtains or a horizon.
- **Detection only on stored pixels**, so EXIF orientation stays valid after cropping.
- **For the NDD pipeline:** the same detector should run before embedding (design conclusion 3).

### Trimming before embedding (`embed.py --trim`, `synth.py --trim`, subset, 2026-10-30)

The detector runs on each decoded image and the crop is embedded (`imgload.TrimmedSource`). It trims 223 of the
subset's 9,306 images (2.4%), and it also runs on the synthetic edited copies.

| SSCD fp16 | Untrimmed | Trimmed |
|---|---|---|
| Letterbox 10%: R@1 / cosine p50 | 0.807 / 0.740 | **0.930 / 0.998** |
| Other 17 synthetic edits: R@1 | – | same within ±1 image (R@10 1.000 everywhere) |
| Labeled pairs: R@1 / R@10 | 0.964 / 1.000 | 0.962 / 1.000 |
| Flagged pairs at 80 / 90 / 95 / 98% recall | 2,086 / 2,485 / 2,643 / 2,835 | 2,085 / 2,465 / 2,627 / 2,839 |
| Cross-group pairs flagged at 98% recall | 4 / 915,283 | 4 |

- **A letterboxed copy becomes the same picture to SSCD** (cosine 0.74 → 0.998), and nothing else moves: the labeled
  set barely has borders, so it's neutral there. Trimming stays in the design (design conclusion 3); cost is the
  detector's CPU time (~80 img/s per thread on full-size images, less on the DCT-scaled decode).
- Measured with the Python prototype. `--trim` now runs the gallery's detector (`gallery borders --stdin`) on the
  same loaded images, and gives the same box on all 900 checked (below), so the table stands.

## Border detector in the gallery (Rust, `crates/analysis/src/borders.rs`, 2026-11-01)

A port of the Python prototype, in integer arithmetic where the prototype used float32. It runs in a pass of its own
after the import, on the stored pixels decoded at full size, and its results are in the catalog (usage in
`crates/README.md`).

- **Parity with the prototype on all 187,806 Pinterest images:** the same **4,539 borders with the same cut on
  every file** (31,077 vs 31,084 files with rejected candidates only).
  - All remaining differences are the prototype's float32 rounding at exact ties of the seam's detail test: 7
    statuses and about 1,600 printed straightness values (±0.01).
  - Rerunning the differing files in Python with exact arithmetic makes every one of them match the port.
- **Not on the import's decode.** The import decodes JPEGs DCT-scaled (≥ 512 × 320), and the detector is
  calibrated on full-size pixels (3 px minimum, 5–20 px frames, per-row seam tests), so it can't run there. A gate
  on the scaled decode was tried: decode at full size only when a side's first line is solid, or two opposite
  sides look blurred.
  - It lost no border, but sent **24%** of all files to a second full decode (44% before requiring the blurred
    pair). Only 3,648 of those 45,349 had a border.
  - Downscaling averages away JPEG noise, so plain backdrops, skies and white product shots look solid at
    reduced scale.
  - So detection became a background pass after the import, on full-size decodes (decision): exact on
    every file, and the import stays as fast as before. The cost is a second read of each still image.
- **Speed:** on `dups2`, the pass took 16.4 s for 5,934 files (362 files/s, 12 workers), the import 20.9 s.
- **Eval:** `imgload.trim_borders` pipes each loaded image to `gallery borders --stdin`. On 900 images (300 with
  borders, 300 with rejected candidates, 300 random) it gives the prototype's box on all 900 (307 trimmed).
