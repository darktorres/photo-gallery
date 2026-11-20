# Plan: detect low-quality images (fake resolution, bad resampling, similar)

## Status

Not started. Planning only; nothing has been measured yet. Every number and threshold below is a hypothesis to be
checked in phase 1.

## Goal

Give each image a quality assessment, computed once on import like the other pixel facts, that says what is wrong
with it and how sure we are. The gallery can then filter, sort or badge images, for example "show me the upscaled
ones" or "hide the heavily compressed ones".

Output is a set of **measured facts with a reason**, not a single pass/fail flag. A score alone hides why an image was
flagged.

## Scope

In:

- **Fake resolution (interpolated upscales).** The image was enlarged from a smaller file, so it has no real detail
  above some frequency.
- **Bad resampling and aliasing.** Downscaled without a proper prefilter (nearest-neighbour, or bilinear with no
  antialiasing), so high-frequency junk and moiré are present.
- **Over-sharpening.** Halos and overshoot at edges, usually from an aggressive resize-then-sharpen.
- **Heavy JPEG compression.** Low quality factor, blocking, ringing.
- **Blur and clipping** (cheap extras, see below). Out-of-focus or motion-blurred images, and blown highlights or
  crushed shadows.

Out:

- **AI upscalers** (Real-ESRGAN, Topaz, waifu2x and similar). Their output looks like native detail, so they are not
  detected. Explicitly not a goal.
- Judging artistic merit or composition.

## Why this is feasible

- **Upscales** leave a sharp cutoff in the radial power spectrum at a frequency well below Nyquist. Native photos keep
  sensor noise and detail up to the limit. Upscaling also spreads edge transitions over more pixels.
- **Periodic resampling traces** (Popescu & Farid, "Exposing digital forgeries by detecting traces of re-sampling",
  2005) give a second, independent signal and can estimate the scale factor. Reliable on simple interpolation, weak
  after recompression.
- **Aliasing** shows up as spectral energy above the target Nyquist, and as moiré peaks in the spectrum.
- **Over-sharpening** shows up as overshoot on either side of step edges. Cheap to measure.
- **JPEG quality** can be read almost exactly from the quantization tables in the file header.
- **Duplicates.** The project already computes PDQ hashes with dihedral variants (`crates/analysis/src/pdq.rs`). If a
  larger copy of the same photo exists, the spectrum comparison between the two is very strong evidence of fake
  resolution. This needs no new model.

## Existing code to reuse

- `crates/analysis/src/borders.rs`: already measures local detail (gradients) and compares regions by sharpness. The
  blur measure should be shared, not duplicated.
- `crates/analysis/src/pdq.rs`, `pg_model::pdq`: near-duplicate matching for the upscale comparison.
- `crates/codec/src/decode.rs`: decoding. The quality pass must reuse the decode the import already does, not decode a
  second time (see `docs/plans/DECODE_ONCE_PLAN.md`).
- `crates/exiftool`, `crates/metadata`: `Software` tags, JPEG header access.
- `pg_model::versions`: the registry of what each analysis version computes. New facts bump it, and existing images
  are re-analyzed in place.
- `crates/codec/src/resize.rs`: the app's own resize (MKS2021, sRGB bytes). Our thumbnails are derived from the
  originals, so the quality pass measures originals only. Do not measure thumbnails.

## Measures

Each measure is computed on the decoded luma plane at full stored resolution, unless noted.

1. **Spectral cutoff.** Radial power spectrum of the luma plane (windowed, mean removed). Find the frequency `f_c`
   above which the power falls to a noise-floor fraction of its mid-band value. Store `cutoff = f_c / Nyquist`
   (1.0 means full detail up to Nyquist). Measure on flat-ish regions and on textured regions separately, since
   smooth skies have little high-frequency content anyway. Expect the upscale signal to appear as a cutoff clearly
   below 1.0 in both.
2. **Effective resolution.** `effective_long_side ≈ cutoff × long_side`, stored alongside the stored size. The most
   user-facing number: "stored 6000 px, effective ~1500 px".
3. **Periodic resampling score.** Residual from a local second-derivative filter, then search for periodic
   correlations at the lattice of candidate scale factors. Store the peak strength and the estimated factor. Only
   trust it when the peak is well above its noise floor.
4. **Edge width.** Median 10–90% transition width across strong step edges, in pixels. Upscales widen it by about the
   scale factor. Natively sharp images have narrow transitions.
5. **Overshoot (halo).** Peak overshoot on either side of strong step edges, as a fraction of the step height.
   Measures over-sharpening and some resampling artifacts.
6. **Aliasing energy.** Spectral energy above the target Nyquist of a plausible downscale, relative to the total. High
   values with moiré peaks indicate aliasing.
7. **JPEG quality factor.** Estimated from the quantization tables (standard IJG tables scaled by quality). Store the
   estimated quality, or "non-standard tables". Also store chroma subsampling. Blocking and ringing metrics are
   optional extras.
8. **Blur.** Variance of the Laplacian, computed on the luma plane, normalised by the image's mean luminance. Shared
   with `borders.rs`.
9. **Clipping.** Fraction of pixels at 0 and 255, per channel.
10. **Duplicate comparison (phase 4).** For each PDQ near-duplicate group, compare measure 1 across members. A member
    whose cutoff is no higher than a smaller member's, despite being larger, is flagged as a fake-resolution copy.

Each measure is cheap on its own. Store the raw numbers, not only the verdicts, so thresholds can change without
re-analysis.

## Design

- **One new analysis fact group**, `quality`, written by the same pixel pass that writes the other pixel facts. Stored
  per image with the analysis version. One row per image; the raw measures are columns or a small typed record
  (follow whatever `pg_model::analysis` already does for `PixelHashes` and `ImageFacts`).
- **Derived verdicts are computed from the raw measures** at read time (or in a view), with thresholds in one place.
  Changing a threshold must not need re-analysis.
- **Version bump** through the registry in `pg_model::versions`, so existing images are re-analyzed in place. Confirm
  how the registry is used before adding a new input.
- **Failure handling.** A measure that cannot be computed (image too small, unsupported format, degenerate content)
  stores "not measured", not zero. Images under `MIN_SIDE` (64 px, `pg_model::detection`) are skipped, as the border
  detector does.
- **Cost budget.** Spectrum work is an FFT per image at full resolution, which is the expensive part. Measure the
  cost in phase 1 against `docs/perf/budgets.json` before deciding on subsampling or a size cap for the spectrum.
- **Native build only**, per project rules. Run `cargo test` for the touched crates and `cargo clippy -D warnings`.

## Phases

### Phase 0: labelled set (needs the user)

Pick a sample from the library and label each image by hand. Start with about 200 images spread across:

- known-good native photos (various cameras, some night shots, some soft or foggy ones, to test false positives)
- known upscales (the user can identify these from the source or from the file's history; if not known, a duplicate
  pair helps)
- known-bad resizes (aliased, nearest-neighbour, over-sharpened)
- heavily compressed JPEGs
- blurred and clipped images

The labels are the ground truth for all thresholds. Without them, every threshold is a guess. The labelled set and
its labels go in `docs/eval/` (see `docs/eval/TEST_SET.md` for how eval sets are kept).

### Phase 1: prototype and measure (no pipeline changes)

- Write a standalone Python prototype under `tools/` that computes measures 1–9 on the labelled set. Python matches the
  precedent of the border detector, which was prototyped in Python and then ported. Run it with `-I` if it reads
  untrusted files in a folder beside the script.
- Report, per measure, how well it separates each labelled class: distributions, and a simple ROC or confusion table.
- Measure the per-image cost at full resolution.
- Decide which measures stay. Drop any that do not separate the classes, rather than keeping them for completeness.
- **Gate:** if measure 1 (spectral cutoff) and measure 2 do not separate the upscale class from native photos on the
  labelled set, stop and reassess before building anything in Rust.

### Phase 2: Rust port

- Port the kept measures into a new module in `crates/analysis` (or a submodule of `borders` for the shared blur
  measure).
- Compute them in the existing pixel pass, on the already-decoded pixels. No second decode.
- Store the raw measures in the catalog, with the version bump.
- Tests: synthetic images with known properties (a bicubic upscale of a known image, a nearest-neighbour downscale, a
  known JPEG quality, a sharpened copy) and the labelled set's agreed results.
- Compare the Rust output with the Python prototype on the labelled set. Differences must be explained, not ignored.

### Phase 3: surface it in the gallery

- Expose the derived verdicts and the effective resolution through the query layer (`crates/query`) and the grid's
  filters. Start with a single "quality" filter with reasons shown in the info panel.
- Keep the UI change separate from the analysis change, so each can be reviewed and reverted on its own.

### Phase 4: duplicate-based upscale detection

- Use the existing PDQ groups to compare measure 1 across members, as described in measure 10.
- Store the comparison result as its own fact. It is only defined for images that have a duplicate group.

## Verification

- Unit tests with synthetic images (phase 2).
- The labelled set: report precision and recall per class, not a single accuracy number, since the classes are very
  unbalanced in a real library.
- A spot check of 30 random library images that get no flag, and 30 that do, read by hand. Record the result in the
  plan.
- No regression in import time beyond the budget agreed in phase 1.

## Risks

- **Natively soft images** (fog, bokeh, night, heavy noise reduction in-camera) will look like upscales or blur.
  Mitigation: the labelled set includes them, and verdicts show their reasons so the user can judge.
- **Heavy sensor noise** can fill the high-frequency band and hide a real upscale cutoff. Mitigation: measure on
  low-noise regions too, and report the noise level.
- **Metadata is unreliable.** `Software` tags are often stripped, so they are not used as a primary signal.
- **Threshold drift** across camera types. Mitigation: thresholds are data-driven from the labelled set, and the
  labelled set spans several cameras.

## Open questions for the user

1. Can you label or point to a set of known upscaled images? Without them phase 0 has no positive class for upscales.
2. Is "heavily compressed" a flag you want, or only a reason shown alongside other flags?
3. Do you want the duplicate-based check (phase 4) at all, or only the single-image measures?
