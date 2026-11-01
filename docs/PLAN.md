# Photo Gallery — Plan

Written 2026-10-28 from `photo gallery.txt`, the 25 papers in `Research/`, and lessons from `DuplicateFinder/`
and `ImageRater/`. Status (2026-10-31): M0's benchmarks are done, its labels and thresholds are not; M1 and M2 are
built (§8); M3–M5 are not started.

Numbers marked **(est.)** are back-of-envelope estimates to be replaced by benchmarks in M0. Numbers with a
paper name are from that paper.

---

## 1. Goals and constraints

- A desktop photo gallery for a library of **~70M images**, with near-duplicate detection aimed at **Pinterest
  saves: many crops and edits of the same picture**, plus the same pin saved at different sizes.
- **Rust** for everything that runs at scale. **Windows only** for now.
- A **high-performance desktop UI** first. Phone/web access is not a goal for now.
- **Non-destructive:** nothing is ever deleted automatically. Removals go to a quarantine folder or the
  Recycle Bin after review.

### Hardware budget (this machine)

| Resource | Spec | Implication |
|---|---|---|
| CPU | i7-8700, 6C/12T, AVX2 (no AVX-512) | Decode, hashing and ANN search compete for 6 cores |
| RAM | 64 GB | 70M float vectors do **not** fit (72 GB at 512-d fp16), so we need compressed or quantized indexes |
| GPU | RTX 2060, **6 GB**, Turing sm_75, fp16/int8 tensor cores | Fine for inference with batched fp16/int8 TensorRT; too small to hold the vector index |
| E: | 22 TB Exos HDD, 21.4 TB used (assumed to be the library) | **The full read is the bottleneck**: about 1–3 days, and only sequential, ordered reads are fast |
| D: | 2 TB NVMe, 802 GB free | Holds the catalog, vectors, ANN index and thumbnails |
| C: | 22 GB free | Put **nothing** of ours here; no caches in `%LOCALAPPDATA%` |

---

## 2. Lessons from previous attempts

- **DuplicateFinder** used ResNet152 *class logits* as the descriptor. Those encode "what object is this", not
  "is this the same image". It also squashed images with `Resize((224,224))` and searched with `k=2`, so it
  could find pairs but never groups.
- **`DuplicateFinder/docs/*.md`** contain unmeasured figures ("SSCD 100 ms per pair", "+10–15% accuracy") and
  frame detection as running a model on every pair. At this scale every image gets **one embedding, computed
  once**, and comparing two embeddings is a dot product.
- **ImageRater** proved out a Rust/Axum backend, Everything SDK v3 FFI, stay-open ExifTool and a variant cache.
  It also measured that an XMP rating only round-trips through Everything for **JPEG**. Therefore the **catalog
  database is the source of truth** for ratings and tags, and writing XMP back to files is optional.

---

## 3. What the research says, applied to 70M images on one machine

1. **Use a tiered pipeline.** Cheap tiers resolve the easy cases; expensive tiers only see the uncertain ones
   (Pinterest; High-Confidence NDD; Benchmarking NDD).
2. **Use the big companies' pipeline shape, not their distributed infrastructure.** Pinterest needed LSH on
   Spark for billions of images. At 70M, a compressed ANN index on one machine is enough. LSH on dense
   embeddings is a poor fit: about 10¹² candidate pairs at 10M (MMDS ch. 3, worked by the research reader).
3. **False positives grow with N².** Even a false-positive rate of 10⁻⁹ gives about 500 false pairs at 1M
   images, and about 2.5M at 70M (Benchmarking NDD §1). Thresholds must be calibrated on *this* library at
   very high precision, and every action stays review-first.
4. **Use a radius query, not top-k** (Pinterest §7). Most images have no duplicate, while a popular pin may have
   50.
5. **Near-duplicate is not transitive.** Chains, hub images and collages bridge unrelated groups. Connected
   components need a **split pass** (Pinterest greedy k-cut §4.3; Dong et al. graph cut).
6. **Generate candidates only against cluster heads, then augment** with close members of each cluster. This
   added +13% recall at Pinterest (§4.1.3), and it shrinks the index when the library has many copies.
7. **Global descriptors fail on heavy crops.** That is exactly the Pinterest case, so crop robustness is the
   main thing M0 has to measure (see §8).
8. **The resize pipeline is part of the model.** Swapping resize libraries shifts features as much as a real
   change in the data (Aliased Resizing, Table 1). Freeze one antialiased pipeline, version it inside the
   index, and re-embed if it ever changes.
9. **Decode and I/O are the bottleneck, not the model** (DALI article; and see the I/O estimate in §4).

---

## 4. Ingestion: one pass over the HDD produces every analysis

Reading 21.4 TB from one HDD takes **about 27 h at an average 220 MB/s, best case (est.)**. With per-file seek
overhead across 70M files, it is more like **1–3 days**. A second pass costs the same again. Two rules follow:

- **Every analysis of the original pixels is produced in the same pass:** file hash, pixel hash, PDQ,
  embeddings, dimensions, ThumbHash placeholder and EXIF.
- **Thumbnails are not part of the import** (user decision, 2026-10-30): the import stays fast, and a
  background task makes them afterwards (§6).
- **Read in on-disk order with a single reader thread per HDD.** Parallel random reads on one spindle thrash
  the head. M0 compared path order, NTFS file-ID (MFT record) order and physical order (first-extent LCN via
  `FSCTL_GET_RETRIEVAL_POINTERS`): MFT order, or 8 reads in flight, reaches the disk's sequential speed, ~3× path
  order (FINDINGS "I/O"). The import reads in MFT order with 8 reads in flight.

```text
Everything SDK / MFT ──► work list (file id, size, mtime), sorted by on-disk location
        │
  [1 reader per HDD]  sequential reads → raw bytes ─► BLAKE3/xxh3 of file bytes   (tier 0a)
        │ bounded channel
  [CPU decode pool]   libjpeg-turbo with DCT downscale (1/2, 1/4, 1/8) where possible
        │             → EXIF orientation applied → sRGB
        ├─► hash of decoded pixels                                   (tier 0b: metadata-only differences)
        ├─► PDQ, 256 bits + 8 dihedral variants (rotations and mirrors) (tier 1)
        ├─► dimensions + ThumbHash placeholder (the grid lays out before any thumbnail exists)
        └─► model input (frozen, antialiased resize) ─► [GPU batcher] ─► embeddings (tier 2)
                                                     │
  [catalog writer]   one SQLite transaction per batch → crash-safe, resumable
```

- **Resumable at any point.** Each file records which stages are done, together with the version of the
  pipeline that produced them.
- **File identity is (volume serial, NTFS file ID), plus size and mtime.** Moves and renames then need no
  rehash. Later, the NTFS USN journal (or Everything) provides incremental changes.
- **Compute estimates:**
  - Decoding with DCT scaling measured ~550 img/s on 12 threads (M0, FINDINGS "Design conclusions so far"), not
    the ~3k img/s first estimated, so the CPU, not the disk (~1,600 files/s), sets the pace.
  - SSCD at 320 px, fp16 on TensorRT (via `ort`'s TensorRT provider), measured at ~1,390 img/s on the 2060
    (M0), i.e. ~14 h. The pass is then CPU-bound on decoding, roughly **1.5–2 days** for the first pass. After
    that, only new files are processed.

---

## 5. Duplicate detection tiers

| Tier | Signal | Catches | Index | Size at 70M (est.) |
|---|---|---|---|---|
| 0a | File-byte hash | Identical files | SQLite index | ~1 GB |
| 0b | Decoded-pixel hash | Same pixels, different metadata or container | SQLite index | ~1 GB |
| 1 | PDQ 256-bit (+ dihedral variants at query time) | Re-sizes (Pinterest 236x/474x/736x/originals), recompressions, mirrors and 90° rotations, light crops | Multi-index hashing: pigeonhole split → exact search within radius r | ~2.2 GB RAM |
| 2 | Copy-detection embedding (SSCD baseline) | **Crops, edits, overlays, filters** | ANN radius search vs cluster heads + augmentation | see below |
| 3 | Verifier on uncertain pairs | Rejects false positives; confirms heavy crops | — | — |

**Tier 2 storage, 70M vectors:**

- **Source of truth:** 512-d fp16 on D:, memory-mapped, about **72 GB**. Used for exact re-ranking.
- **In-RAM search index:** two candidates, chosen by benchmark in M0 on a 5M sample:
  - **USearch HNSW, PCA-256, int8:** about 18 GB of vectors plus about 9 GB of graph, roughly **27 GB of RAM**.
    No training step, native incremental inserts, and Windows and Rust support.
  - **FAISS IVF65536 + OPQ/PQ32:** about **2.8 GB**. Needs k-means training on a random sample, and the C++
    build on Windows is CPU-only (FAISS GPU is effectively Linux-only).
- **Incremental from day one** (Pinterest new-vs-old pattern):
  1. Ingest a *random* 2M sample spread across the library first.
  2. Fit the PCA and the index (and IVF, if FAISS) on that sample.
  3. From then on, each new image **queries, then inserts**, so the "self-join" happens during ingestion and
     there is never a separate 70M × 70M batch job.

**Tier 3, the verifier:**

- Starts as a small classifier (logistic regression, then an MLP) over: embedding cosine, PDQ distance, the
  ratio of the two images' sizes, aspect ratios, and local-feature inlier count.
- **Training labels come from the review UI** ("duplicate" / "not a duplicate" buttons). This is Pinterest's
  approach scaled down: they trained on more than 1B pairs; we bootstrap from about a few thousand.
- **Local-feature verification**, only for pairs in the ambiguous band:
  - Options are OpenCV ORB/SIFT/AKAZE through the `opencv` crate, or a learned matcher (ALIKED/DISK +
    LightGlue exported to ONNX, run through `ort`).
  - Geometric check: Hough pre-filter, then clique-filtered RANSAC. That paper reports about 12 ms per pair
    (RANSAC 2D Cliques).
- **Vision-language model as a last-resort judge** (option, later): only for pairs the verifier can't settle
  geometrically, e.g. a liquify/slimming edit vs another frame, or a hair recolor (FINDINGS: the gray zone).
  Ask a VLM (e.g. Claude vision) "same picture, possibly edited, or a different picture?", cache the answer per
  pair, and treat it like a review label. Expected to be a small fraction of pairs; cost and latency per pair
  decide whether it's worth it over hand review.

**Grouping:**

- Verified edges, signed (match / aligned-but-different, weighted by the verifier's margin) → correlation
  clustering by greedy additive edge contraction (multicut). Measured in M0 to beat union-find plus a split
  pass (FINDINGS: "Shortcut and grouping"). Union-find stays useful for sharding: clusters never cross components.
- Each group gets a canonical **keeper**: the largest pixel area that contains the others, then the highest
  quality. **Folder preferences** override this: an ordered list of patterns (e.g. prefer `originals\` over
  `backup\` or `pinterest\`); newest/oldest as further tie-breakers.
- "Not a duplicate" decisions are stored per pair and are never re-proposed.

**Removal (quarantine and restore):**

- Removed files are *moved* into a quarantine folder on their own drive, so the move is a rename, not a copy.
  The folder structure is kept: `E:\pinterest\a\b.jpg` → `E:\_quarantine\<date>\pinterest\a\b.jpg`.
- Every move is recorded (catalog table and a `manifest.json` per batch): original path, quarantine path, group,
  keeper, reason, time.
- **Restore** puts files back from the record and refuses to overwrite a file that has reappeared at the
  original path. Emptying the quarantine is a separate, explicit step.

---

## 6. Storage layout (D:)

| Store | Tech | Size at 70M (est.) |
|---|---|---|
| Catalog: files, hashes, ratings, tags, groups, pair decisions, pipeline versions | SQLite (WAL), `rusqlite` | 20–30 GB |
| Embeddings (source of truth) | Flat memory-mapped fp16 file, row = catalog id | ~72 GB |
| ANN index | USearch or FAISS file | 3–30 GB |
| PDQ multi-index | Rebuilt in RAM at startup from the catalog | (RAM) |
| Thumbnail cache | **Packed shard files** (not loose files), keyed by (file, size) | Size-capped, least recently used evicted |
| ThumbHash placeholders | Column in the catalog | ~2 GB |

**Thumbnails (user decision, 2026-10-30):** made by a **background task after the import**, never during it,
at the size the grid currently uses. The size is **adjustable at runtime**: changing it queues new
thumbnails, and visible items are made first, on demand. Placeholders (ThumbHash) and the dimensions come from
the import, so the grid lays out instantly while thumbnails fill in. Thumbnails are display-only (FINDINGS
"Thumbnails": compression moves embeddings too much to re-analyse them), so the cache can be evicted and rebuilt
at any time. It costs a second read of the originals, in the background and in on-disk order.

**Cache layout now (2026-10-31):** one SQLite file, `thumbs.sqlite`, with the entries (file, height, source,
size, last use) apart from the JPEGs and the totals kept by triggers, so nothing that runs often reads the JPEGs
(FINDINGS "Startup"). The JPEGs are keyed by entry id, so they can move to packed shard files later without
touching the entries. Format and cap are still open (§9.1).

---

## 7. Desktop UI: Qt 6 Quick (QML) + CXX-Qt

Why:

- It's the stack you already gravitate to (Gyroflow, the FluentUI/QML bookmarks, ImageReduce in Qt/C++).
- GPU scene graph, real touch support, and fast image rendering.
- **CXX-Qt** (KDAB) is the actively maintained Rust↔Qt bridge.
- Gyroflow (cloned at `Documents/GitHub/gyroflow`) uses the older `qmetaobject-rs`, but it's a useful reference
  for Windows build and deployment of a Rust + QML application.

Shape:

- **Virtualized grid:** a GridView/TableView (or custom justified layout) backed by a Rust `QAbstractListModel`
  whose rows are fetched **lazily in pages** from SQLite. The UI never holds 70M items.
- **Justified layout computed in Rust.** Immich's `justified-layout` is Rust source, so we can use it natively
  instead of through WASM. QML only positions the delegates.
- **Thumbnail provider:** a `QQuickAsyncImageProvider` (a thin C++ class if CXX-Qt can't express it directly)
  that reads the cache at the current grid size, or asks the thumbnail task for it first. ThumbHash
  placeholders show instantly, and images fade in (Wallhaven-style).
- **Viewer:**
  - Original via a fast decode path.
  - An antialiased downscale for fit-to-screen.
  - A **1000% loupe** window to check sharpness.
  - An optional bright border or marching ants to reveal black bars.
- **Duplicate review:**
  - Group view: keeper first, then each candidate showing *why* it matched (PDQ distance, cosine, inliers).
  - Side-by-side compare with a slider and a crop-alignment overlay.
  - Keyboard-first: keep / quarantine / "not a duplicate".
- **Merged-drives view:** a virtual tree where folders with the same name across drives appear merged.

The `core` crate has no UI dependency, so a web or phone UI can be added later against the same core.

---

## 8. Milestones

### M0 — Evaluation and benchmarks (before any 70M work)

- Copy a ~200k sample to D:, including known Pinterest crop/edit groups. Hand-label ~2k pairs in three
  classes: duplicate, crop/edit duplicate, not a duplicate.
- **Model bake-off** on crop recall at very low false-positive rates:
  - SSCD (`disc_mixup`) vs DINOv2 vs CLIP vs PDQ (done: SSCD fp16; PDQ as a cheap tier for near-exact copies;
    FINDINGS "Design conclusions so far").
  - Measure flip tolerance, to decide whether to embed mirrored images too (done on the subset: SSCD tolerates
    flips, so no mirrored embedding; to confirm on the full set).
  - Measure crop-size limits, to decide whether reference images need *region embeddings* (partly: a global
    threshold fails on heavy crops but rank holds, so k-NN rank plus a verifier; region embeddings still open,
    §9 decision 4).
  - Python is used here only for ONNX export and the evaluation notebooks.
- **I/O benchmark** of read orderings on E: (path vs file ID vs LCN) (done: MFT order, or 8 reads in flight, ~3×
  path order; FINDINGS "I/O").
- **Throughput benchmarks:**
  - Decode with `turbojpeg` DCT scaling vs `zune-jpeg` vs `image` (done: DCT-scaled decode, FINDINGS "Decode
    benchmark"; the import uses libjpeg-turbo).
  - Resize with `fast_image_resize`, verified with the aliasing paper's downsampled-circle test (not done).
  - Thumbnail encode with JPEG (turbo/jpegli) vs WebP (done: AVIF/WebP/JPEG XL too, FINDINGS "Thumbnails").
  - `ort` with CUDA vs TensorRT EP, fp16 vs int8 (done: TensorRT fp16, FINDINGS "TensorRT and ONNX Runtime").
- **ANN bake-off** on 5M vectors: USearch HNSW int8 vs FAISS IVF-PQ, measuring recall at radius, RAM and
  insert speed (done on synthetic vectors: exact GPU search is fast enough for 70M all-pairs, ~5.5 days, and
  the approximate indexes lose partners; §9 decision 5, FINDINGS "Nearest-neighbour search").
- **Exit criteria:** chosen model, thresholds, thumbnail format and size, index type, and read order, each
  backed by a number. Met: the model and the read order; the thumbnail size is no longer fixed (§6). Open: the
  thresholds (they wait on the hand marks, FINDINGS "Pending"), the thumbnail cache's format and cap, and the
  index type (§9 decision 5).

### M1 — Core and ingest (on the sample)

- Cargo workspace (`core`, `ingest`, `ui`), catalog schema, scanner, the single-pass import pipeline,
  tiers 0–1, resumability.
- Background thumbnail task (after import, size chosen at runtime) and its cache.
- **Status (2026-10-30):** built and verified on `dups2` (FINDINGS "M1 import"; usage in `crates/README.md`).
  The scanner lists directories by handle (NTFS file IDs, no admin) instead of Everything: the installed
  Everything is 1.4, whose SDK has no file IDs. Not yet run on the full 190k sample.

### M2 — Desktop UI

- QML grid with lazy model and thumbnail provider, viewer and loupe, ratings in the catalog, merged-drives
  tree.
- **Status (2026-10-30):** first version built (`gallery.exe`, `crates/ui`). Justified rows from Rust in a
  virtualized ListView. The row height is adjustable at runtime, with thumbnails made on demand at that size
  (newest request first) and ThumbHash placeholders. Viewer with Lanczos fit, 1:1, 10× loupe and a frame to
  reveal black bars. Ratings 0–5 in the catalog (schema v2). The tree merges folders by their path below the
  drive.
- **One program (user decision, 2026-10-30):** no separate importer. `gallery.exe` holds the library (folders
  added in the app; schema v3) and runs scan → import → thumbnails as a background job, with progress and
  pause. A new folder, a rescan or a new grid size interrupts the pass in progress, which resumes from the
  catalog. The same passes are commands of the same exe (`gallery import`, …) for scripts and profiling.
  Part of M4 is therefore done; still open there are throttling while the UI is in use, and the 70M scale.
- **ImageRater's features (user request, 2026-10-30):** ported as features, not code, and without Everything.
  Rating bar with ImageRater's symbols, rate-and-advance with a popup, ↑/↓ shortcuts, filters (unrated, rated,
  1–5) and sorts (largest, newest) in SQL, saved views, the last place restored at start, continuous zoom
  (Lanczos region renders between fit and 1:1, nearest neighbour above), Real Size, zen mode, swipes, ±4
  preloading. Ratings are written back into the files (XMP via ExifTool, queued and in place) for every format
  that holds XMP, and read from them at import with a three-way sync. Turns are lossless and in the pixels where
  the format allows. Usage in `crates/README.md`. Reverse image search (Yandex) came after, inside the app (below).
- **Filters and videos (user request, 2026-10-30):** one query model (`query.rs`, JSON in views and the
  session) replaces the rating filter. Its facets are type (image, animated, video), extension, rating, shape,
  size, megapixels, duration, date taken (read at import), date modified, duplicates, import status and name.
  Facets combine with AND, values within one with OR, and the type and extension facets show counts. Sorts are
  a key and a direction. To make "video" a type, videos are now scanned, imported (streamed hash, probe, poster
  frame), thumbnailed and played (Qt Multimedia). Animated GIF/APNG/WebP play too. Decoding uses our own
  FFmpeg 9 (BtbN `lgpl-shared`, which has dav1d), shared with Qt's player in place of Qt's copy. On a GPU
  without AV1 decoding the player decodes on the CPU (`crates/ui/src/gpu_device.rs`). Videos get no
  near-duplicate detection, and their ratings and turns stay in the catalog (see Later).
- **Yandex reverse image search (user request, 2026-10-30):** the viewer's ≈ button uploads the photo and shows two
  groups in a panel: the photo's other sizes (to find a better copy) and similar images (to find more), all of
  them, not just the first page (`pg_core::yandex`, ported from the crawlers repo's `search.py`; why the browser
  sees more is in `docs/FINDINGS.md`). A click picks a result; **Save N images…** downloads the picks into a
  folder chosen in a dialog, as `size WxH.ext` and `similar N.ext`, with "(2)", "(3)"… added on a clash. Open, for
  picking and saving:
  - **Select all** for each group, and **Shift-click** to pick a range.
  - **Double-click** a result to see it at full size before saving.
  - The folder dialog opens at the photo's folder, or the last folder saved to.
  - Saved files are named after the photo (`IMG_1234 (yandex 1920x1080).jpg`), so they sort next to it.
- **Schema (user decision, 2026-10-30):** one baseline, at version 1, until the first catalog worth keeping (M4
  at the latest). Pre-release changes edit it in place, with no new version; catalogs made before are deleted
  and re-imported. The migration loop exists, tested with a synthetic step, with no migrations yet; that first
  kept catalog starts real ones.
- **Open: the video player.** Qt Multimedia plays videos, with hardware decoding off on a GPU without AV1
  decoding (a stopgap). The choice between our own engine on our FFmpeg 9 and libmpv is in `docs/VIDEO_PLAYER.md`.
- **Open for M4:** the grid loads the whole result list (filtered and sorted in SQL) into memory. That is fine
  per folder, but "All folders" at 70M needs the paged model of §7.

### M3 — Near-duplicates

- Tier 2 (embeddings + ANN, query-then-insert), grouping (multicut), the review UI with feedback buttons.

### M4 — Full library run

- Background ingestion of all 70M images with progress, pause/resume, and throttling while the UI is in use.

### M5 — Precision

- Tier 3 verifier trained on review labels, local-feature check for heavy crops, keeper selection.

**Later (from the notes):**

- AI keywords and semantic search (CLIP/SigLIP).
- A pix-plot-style 2D map (UMAP).
- Format conversions (ImageReduce; GIF→WebP/WebM; JPEG XL/jpegli).
- gallery-dl / Grabber integration.
- Rating from a phone (ImageRater was a web app on the LAN): a small triage page served by `gallery.exe`.
- More formats: HEIC/AVIF through libheif (dav1d, libde265), RAW through LibRaw (embedded previews, ratings in
  `.xmp` sidecars), and JPEG XL through libjxl, all built through a pinned vcpkg manifest on D:.
- Video near-duplicates (frame sampling, re-encodes, trims), and ratings written into video files without
  rewriting gigabytes.
- HDR video posters: tone mapping (PQ/HLG) instead of a plain conversion to RGB.
- Browsing any folder, not only the catalog's (user idea, 2026-11-01): open an arbitrary path in the grid and
  viewer, listed from the disk as it is. Such a folder has no catalog rows, so no thumbnail cache, ThumbHash
  placeholders or catalog facets; that is expected.

---

## 9. Open decisions

1. ~~**Thumbnail size**~~ **Decided 2026-10-30:** no fixed size. Thumbnails are a background task after the
   import, at a size adjustable at runtime, in a size-capped cache (§6). Open: the cache format and its cap.
2. ~~**What runs in the first full pass**~~ **Decided 2026-10-30:** SSCD plus a semantic model, **SigLIP 2
   So400m/16 NaFlex** (native aspect ratio, 256 patches; TensorRT fp16), so keywords and semantic search need no
   second decode of the originals. Cost: ~154 GPU hours for 70M (~6.4 days), so the GPU, not decoding (~1.5 days),
   becomes the limit of that pass. NaFlex needs only ~208 × 304 px at 256 patches, so it can also run as a
   background GPU task on the import's downscaled working image instead of holding up the import (FINDINGS: "A
   second model" and "NaFlex").
3. **Mirrors in tier 2:** rely on SSCD's flip tolerance, or embed the flipped image too (2× GPU time). Measured
   on the subset: SSCD tolerates flips, so no flipped embedding, pending confirmation on the full set.
4. **Heavy crops:** if M0 shows SSCD misses small crops, consider region embeddings for reference images. That
   costs multiples of index size, and could be limited to cluster heads.
5. **ANN backend:** USearch (simple, about 27 GB RAM) vs FAISS (about 3 GB, training step, harder Windows
   build). Measured on 5M synthetic vectors (2026-10-31): neither, for duplicate candidates. Exact fp16 search on
   the 2060 runs at 10.4 G pairs/s, ~5.5 days for 70M all-pairs and ~70 s per 10,000 new images, at the recall
   ceiling; the best approximate index (FAISS IVF, nprobe 256) found 77% of planted partners at cosine ≥ 0.4
   and HNSW int8 26%, on data that is a worst case for them. Recommended: exact search, which replaces Tier 2's
   in-RAM index and the query-then-insert scheme above with one all-pairs batch job plus brute force for new
   images. Semantic search (one text query against 70M) still needs an index. Not decided yet.

## 10. Key references (in `Research/`)

- Evolution of a Web-Scale Near Duplicate Image Detection System (Pinterest): tiers, verifier, grouping,
  incremental updates.
- Benchmarking unsupervised near-duplicate image detection: low false-positive-rate regime, hard-negative
  calibration.
- Squint Hard Enough: PDQ behaviour and robustness.
- High-Confidence Near-Duplicate Image Detection: hub and bridge false positives, 4-block Hamming index.
- Billion-scale similarity search with GPUs (FAISS): IVF-PQ, k-NN graph construction.
- On Aliased Resizing…: the resize filter as part of the model.
- On the De-duplication of LAION-2B: a fixed threshold on quantized distances fails; use relative thresholds.
- RANSAC with 2D Geometric Cliques: fast geometric verification.
