# Photo Gallery — Plan

Written 2026-10-28 from `photo gallery.txt`, the 25 papers in `Research/`, and lessons from `DuplicateFinder/`
and `ImageRater/`. Status (checked against the code 2026-11-12): M0's benchmarks are done, its labels and thresholds
are not; M1 and M2 are built (§8), with video, borders and reverse image search added since 2026-10-31; M3–M5 are not
started in Rust. The tier-2 prototypes are Python scripts in `eval/`.

Numbers marked **(est.)** are back-of-envelope estimates to be replaced by benchmarks in M0. Numbers with a
paper name are from that paper. Measured figures are as recorded when measured; they were not re-run for this check.

---

## 1. Goals and constraints

- A desktop photo gallery for a library of **~70M images**, with near-duplicate detection aimed at **Pinterest
  saves: many crops and edits of the same picture**, plus the same pin saved at different sizes.
- **Rust** for everything that runs at scale. **Windows only** for now (decided 2026-10-28). CUDA, nvJPEG, PyTorch,
  TensorRT and ONNX Runtime all work there. Linux runs only in the `pg-dev` container (`tools/linux.py`), for the CI
  checks, the callgrind benches, the fuzzer and the eval's Linux parts. WSL is not used.
- **Quality over speed** (user decision, 2026-10-28): days or weeks of processing are acceptable. Heavier models
  (SSCD-large, fp32), exact rather than approximate k-NN (TIER2_ANN_BAKEOFF_PLAN.md: ~5.5 days of GEMM for 70M), multiple
  embeddings per image and a learned geometric verifier are all in scope. Resumability and the user's review time
  (precision) are the real constraints.
- A **high-performance desktop UI** first. A phone/browser client over the LAN is in scope after it; see `docs/temp/WEB_CLIENT.md`.
- **Non-destructive:** nothing is ever deleted automatically. Removals go to a quarantine folder or the
  Recycle Bin after review.

### Hardware budget (this machine)

| Resource | Spec | Implication |
|---|---|---|
| CPU | i7-8700, 6C/12T, AVX2 (no AVX-512) | Decode, hashing and ANN search compete for 6 cores |
| RAM | 64 GB | 70M float vectors do **not** fit (72 GB at 512-d fp16), so we need compressed or quantized indexes |
| GPU | RTX 2060, **6 GB**, Turing sm_75, fp16/int8 tensor cores | Fine for inference with batched fp16/int8 TensorRT; too small to hold the vector index |
| E: | 22 TB Exos HDD, 21.4 TB used (assumed to be the library) | **The full read is the bottleneck**: about 1–3 days, and only sequential, ordered reads are fast |
| D: | 2 TB NVMe, **116 GB free** (2026-11-12; 802 GB on 2026-10-28) | Holds the catalog, vectors, ANN index and thumbnails. At 116 GB free, the 72 GB embedding store of §6 would leave little room |
| C: | **65 GB free** (2026-11-12; 22 GB on 2026-10-28) | Put **nothing** of ours here; no caches in `%LOCALAPPDATA%` |

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

- **Every analysis of the original pixels is produced in the same pass:** file hash, pixel hash, PDQ, dimensions,
  ThumbHash placeholder and EXIF. Embeddings join this pass when M3 is built; the import does not compute them yet.
- **Thumbnails are not part of the import** (user decision, 2026-10-30): the import stays fast, and a
  background task makes them afterwards (§6).
- **Neither are borders** (user decision, 2026-11-01): the detector needs full-size pixels and the import's decode
  is DCT-scaled. A background pass after the import decodes each still image at full size, which reads it a
  second time (BORDERS.md "Border detector in the gallery"). This bears on where embeddings run (§9, decision 6).
- **Read in on-disk order.** Parallel random reads on one spindle thrash the head. M0 compared path order, NTFS
  file-ID (MFT record) order and physical order (first-extent LCN via `FSCTL_GET_RETRIEVAL_POINTERS`): MFT order, or
  8 reads in flight, reaches the disk's sequential speed, ~3× path order (../reference/IO.md "I/O"). The import sorts by MFT
  record and keeps 8 reads in flight (`pg-jobs`, `Options::readers`, default 8).

```text
Everything SDK / MFT ──► work list (file id, size, mtime), sorted by on-disk location
        │
  [1 reader per HDD]  sequential reads → raw bytes ─► BLAKE3 of file bytes                (tier 0a)
        │ bounded channel
  [CPU decode pool]   libjpeg-turbo with DCT downscale (1/2, 1/4, 1/8) where possible
        │             → EXIF orientation applied → sRGB
        ├─► XXH3 of decoded pixels                                   (tier 0b: metadata-only differences)
        ├─► PDQ, 256 bits + 8 dihedral variants (rotations and mirrors) (tier 1)
        ├─► dimensions + ThumbHash placeholder (the grid lays out before any thumbnail exists)
        └─► model input (frozen, antialiased resize) ─► [GPU batcher] ─► embeddings (tier 2, not built)
                                                     │
  [catalog writer]   one SQLite transaction per batch → crash-safe, resumable
```

- **Resumable at any point.** Each file records which stages are done, together with the version of the
  pipeline that produced them.
- **File identity is (volume serial, NTFS file ID), plus size and mtime.** Moves and renames then need no
  rehash. Later, the NTFS USN journal (or Everything) provides incremental changes.
- **Compute estimates:**
  - Decoding with DCT scaling measured ~550 img/s on 12 threads (M0, "Design conclusions so far" below), not
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
| 1 | PDQ 256-bit (the 8 dihedral variants are stored per file, `analysis.pdq_dihedral`, and each is queried) | Re-sizes (Pinterest 236x/474x/736x/originals), recompressions, mirrors and 90° rotations, light crops | Multi-index hashing: pigeonhole split → exact search within radius r | ~2.2 GB RAM |
| 2 | Copy-detection embedding (SSCD baseline) | **Crops, edits, overlays, filters** | ANN radius search vs cluster heads + augmentation | see below |
| 3 | Verifier on uncertain pairs | Rejects false positives; confirms heavy crops | — | — |

**Status (2026-11-12):** tiers 0a, 0b and 1 are built (`pg-dedup`, `pg-catalog`). The `dups` command groups them by
plain connected components (`pg-dedup`'s `UnionFind`). Tiers 2 and 3 are not in Rust; their prototypes are
`eval/embed.py`, `eval/ann/`, `eval/verify.py` and `eval/group.py`.

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
  geometrically, e.g. a liquify/slimming edit vs another frame, or a hair recolor (../eval/VERIFIER.md, "Gray zone").
  Ask a VLM (e.g. Claude vision) "same picture, possibly edited, or a different picture?", cache the answer per
  pair, and treat it like a review label. Expected to be a small fraction of pairs; cost and latency per pair
  decide whether it's worth it over hand review.

**Grouping** (the Rust grouping is still plain union-find over the tier 0–1 pairs; multicut is not built):

- Verified edges, signed (match / aligned-but-different, weighted by the verifier's margin) → correlation
  clustering by greedy additive edge contraction (multicut). Measured in M0 to beat union-find plus a split
  pass (../eval/VERIFIER.md: "Shortcut and grouping"). Union-find stays useful for sharding: clusters never cross components.
- Each group gets a canonical **keeper**: the largest pixel area that contains the others, then the highest
  quality. **Folder preferences** override this: an ordered list of patterns (e.g. prefer `originals\` over
  `backup\` or `pinterest\`); newest/oldest as further tie-breakers.
- "Not a duplicate" decisions are stored per pair and are never re-proposed.

**Removal (quarantine and restore; not built yet, nothing moves duplicates into quarantine):**

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
| Thumbnail cache | **One SQLite file**, `thumbs.sqlite` (WAL): entries and JPEGs in separate tables, keyed by (file, height) | Byte cap, default 50 GB; least recently used evicted |
| ThumbHash placeholders | Column in the catalog | ~2 GB |

**Thumbnails (user decision, 2026-10-30):** made by a **background task after the import**, never during it,
at the size the grid currently uses. The size is **adjustable at runtime**: changing it queues new
thumbnails, and visible items are made first, on demand. Placeholders (ThumbHash) and the dimensions come from
the import, so the grid lays out instantly while thumbnails fill in. Thumbnails are display-only (THUMBNAILS.md
"Thumbnails": compression moves embeddings too much to re-analyse them), so the cache can be evicted and rebuilt
at any time. It costs a second read of the originals, in the background and in on-disk order.

**Cache layout now (2026-10-31):** one SQLite file, `thumbs.sqlite`, with the entries (file, height, source,
size, last use) apart from the JPEGs and the totals kept by triggers, so nothing that runs often reads the JPEGs
(../reference/START_PROFILE.md "Startup"). The JPEGs are keyed by entry id, so they can move to packed shard files later without
touching the entries. Format: JPEG (libjpeg-turbo q85, 4:2:0). Cap: 50 GB by default, in bytes (§9.1).

---

## 7. Desktop UI: Qt 6 Quick (QML) + CXX-Qt

Why:

- It's the stack you already gravitate to (Gyroflow, the FluentUI/QML bookmarks, ImageReduce in Qt/C++).
- GPU scene graph, real touch support, and fast image rendering.
- **CXX-Qt** (KDAB) is the actively maintained Rust↔Qt bridge.
- Gyroflow (a local clone of Gyroflow) uses the older `qmetaobject-rs`, but it's a useful reference
  for Windows build and deployment of a Rust + QML application.

Shape:

- **Virtualized grid:** a GridView/TableView (or custom justified layout) backed by a Rust `QAbstractListModel`
  whose rows are fetched **lazily in pages** from SQLite. The UI never holds 70M items.
- **Justified layout computed in Rust.** Immich's `justified-layout` is Rust source, so we can use it natively
  instead of through WASM. QML only positions the delegates. Built in `crates/client/src/layout.rs`.
- **Thumbnail provider:** a `QQuickAsyncImageProvider`, built as a thin C++ class (`crates/desktop/cpp/providers.cpp`)
  in front of the Rust loaders. It reads the cache at the current grid size, or asks the thumbnail task for it
  first. ThumbHash placeholders show instantly, and images fade in (Wallhaven-style).
- **Viewer:**
  - Original via a fast decode path.
  - An antialiased downscale for fit-to-screen.
  - A **1000% loupe** window to check sharpness.
  - An optional bright border or marching ants to reveal black bars.
- **Duplicate review** (not built: the desktop has a Duplicates facet, but no group, compare or keep view yet; M3):
  - Group view: keeper first, then each candidate showing *why* it matched (PDQ distance, cosine, inliers).
  - Side-by-side compare with a slider and a crop-alignment overlay.
  - Keyboard-first: keep / quarantine / "not a duplicate".
- **Merged-drives view:** a virtual tree where folders with the same name across drives appear merged.

`pg-app` (the application layer) and `pg-client` (the front end's logic) have no UI dependency, so a web or phone UI
can be added later against the same crates.

---

## 8. Milestones

### M0 — Evaluation and benchmarks (before any 70M work)

- Copy a ~200k sample to D:, including known Pinterest crop/edit groups. Hand-label ~2k pairs in three
  classes: duplicate, crop/edit duplicate, not a duplicate.
- **Model bake-off** on crop recall at very low false-positive rates:
  - SSCD (`disc_mixup`) vs DINOv2 vs CLIP vs PDQ (done: SSCD fp16; PDQ as a cheap tier for near-exact copies;
    "Design conclusions so far" below).
  - Measure flip tolerance, to decide whether to embed mirrored images too (done on the subset: SSCD tolerates
    flips, so no mirrored embedding; to confirm on the full set).
  - Measure crop-size limits, to decide whether reference images need *region embeddings* (partly: a global
    threshold fails on heavy crops but rank holds, so k-NN rank plus a verifier; region embeddings still open,
    §9 decision 4).
  - Python is used here only for ONNX export and the evaluation scripts in `eval/`.
- **I/O benchmark** of read orderings on E: (path vs file ID vs LCN) (done: MFT order, or 8 reads in flight, ~3×
  path order; ../reference/IO.md "I/O").
- **Throughput benchmarks:**
  - Decode with `turbojpeg` DCT scaling vs `zune-jpeg` vs `image` (done: DCT-scaled decode, JPEG_DECODERS.md "Decode benchmark"; the import uses libjpeg-turbo).
  - Resize with `fast_image_resize`, verified with the aliasing paper's downsampled-circle test (not done).
  - Thumbnail encode with JPEG (turbo/jpegli) vs WebP (done: AVIF/WebP/JPEG XL too, THUMBNAILS.md "Thumbnails").
  - `ort` with CUDA vs TensorRT EP, fp16 vs int8 (done: TensorRT fp16, ../eval/MODELS.md "TensorRT and ONNX Runtime").
- **ANN bake-off** on 5M vectors: USearch HNSW int8 vs FAISS IVF-PQ, measuring recall at radius, RAM and
  insert speed. A first run on synthetic vectors was invalid (unclustered data, untuned indexes; TIER2_ANN_BAKEOFF_PLAN.md
  "Nearest-neighbour search, first attempt"); it is being redone on the test set's real embeddings plus planted
  edits, with tuned indexes (§9 decision 5).
- **Exit criteria:** chosen model, thresholds, thumbnail format and size, index type, and read order, each
  backed by a number. Met: the model and the read order; the thumbnail size is no longer fixed (§6). Open: the
  thresholds (they wait on the hand marks, the first item in `docs/temp/OPEN_ITEMS.md`), and the index type
  (§9 decision 5). The thumbnail cache's format and cap are set in code (§6).

### M1 — Core and ingest (on the sample)

- Cargo workspace, catalog schema, scanner, the single-pass import pipeline, tiers 0–1, resumability. (Since split
  into the `crates/*` packages in `crates/README.md`: `pg-catalog`, `pg-analysis`, `pg-jobs`, `pg-library`,
  `pg-cli`, `pg-desktop` and others; `core`, `ingest` and `ui` no longer exist, see `docs/temp/REFACTOR_PLAN.md`.)
- Background thumbnail task (after import, size chosen at runtime) and its cache.
- **Status (2026-10-30):** built and verified on `dups2` (5,937 files; usage in `crates/README.md`). Measured with
  GPU benchmarks running alongside, so the speeds are lower bounds:
  - **Import:** 341 files/s on 12 workers (14 ms per image on one thread: decode 8.7, PDQ 2.3, Lanczos to 512 px
    1.7, rest under 0.2). CPU-bound, as M0 predicted.
  - **PDQ:** the Rust port is bit-exact with the `pdqhash` crate. Against M0's hashes, 74% are identical and 98%
    within 4 bits (max 14, from the DCT-scaled decode). Labeled-pair recall at d ≤ 0 / 8 / 16 / 22 / 31 is
    **30.8 / 66.0 / 75.4 / 80.4 / 83.2%** (M0: 32.6 / 65.7 / 74.8 / 80.1 / 82.8%). No cross-group pair at d ≤ 31.
  - **Tiers 0a/0b:** 0 byte-identical groups and 234 pixel-identical groups (468 files), from the `.mozjpeg.jpg`
    re-encodes (../eval/TEST_SET.md).
  - **Resumable:** an import killed after 1,500 files and rerun ends with the same 2,784 pairs as an uninterrupted
    run. Renamed and moved files keep their analysis; a deleted file is marked missing; a touched file is
    re-analyzed alone.
  - **Thumbnail task:** its I/O and CPU priority are in THUMBNAILS.md, "Background task priority".
  - The scanner lists directories by handle (NTFS file IDs, no admin) instead of Everything: the installed
    Everything is 1.4, whose SDK has no file IDs. Not yet run on the full 190k sample.

### M2 — Desktop UI

- QML grid with lazy model and thumbnail provider, viewer and loupe, ratings in the catalog, merged-drives
  tree.
- **Status (2026-10-30):** first version built (`gallery.exe`, from `crates/gallery`; the app is `crates/desktop`).
  Justified rows from Rust in a
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
- **Filters and videos (user request, 2026-10-30):** one query model (`crates/model/src/query.rs`, with its SQL in `crates/query`; JSON in views and the
  session) replaces the rating filter. Its facets are type (image, animated, video), extension, rating, shape,
  size, megapixels, duration, date taken (read at import), date modified, duplicates, import status and name.
  Facets combine with AND, values within one with OR, and the type and extension facets show counts. Sorts are
  a key and a direction. To make "video" a type, videos are now scanned, imported (streamed hash, probe, poster
  frame), thumbnailed and played (Qt Multimedia). Animated GIF/APNG/WebP play too. Decoding uses our own
  FFmpeg 9 (BtbN's GPL shared build, `gpl-shared` since 2026-11-02, `lgpl-shared` before; both have dav1d), shared with Qt's player in
  place of Qt's copy. On a GPU
  without AV1 decoding the player decodes on the CPU (`crates/desktop/src/gpu_device.rs`). Videos get no
  near-duplicate detection, and their ratings and turns stay in the catalog (see Later).
- **Yandex reverse image search (user request, 2026-10-30):** the viewer's ≈ button uploads the photo and shows two
  groups in a panel: the photo's other sizes (to find a better copy) and similar images (to find more), all of
  them, not just the first page (`pg_yandex`, ported from the crawlers repo's `search.py`; why the browser
  sees more is in `docs/temp/REVERSE_SEARCH.md`). Picking and saving (done 2026-11-01):
  - A click picks a result, **Shift-click** a range (to the anchor's state), and each group has **Select all**.
  - **Double-click** shows a result at full size, fitted or 1:1 (←/→ step, Space picks, Escape goes back).
  - **Save N images…** opens its folder dialog at the folder saved into last (kept in `gallery.json`), or else
    the photo's folder.
  - Saved files are named after the photo (`IMG_1234 (yandex 1920x1080).jpg`, `IMG_1234 (yandex similar 3).jpg`),
    so they sort next to it; "(2)", "(3)"… are added on a clash.
- **Borders (user request, 2026-11-01):** the border detector and its review moved from the eval tools into
  the gallery (the detector is `pg_analysis::borders`, a port of `tools/borders.py`, which it matches on all 187,806 Pinterest images; the pass that finds and stores them is `pg-borders`;
  the script is gone, and the eval's `--trim` uses the gallery's detector).
  - **Detection:** a background pass after the import, on full-size decodes (§4). Results are in the catalog
    (cut, every side found with why, "not a border" marks), with its own detector version, so a change looks
    again without a new import.
  - **Review:** a Borders facet (none, black, white, coloured, blurred bars) with counts, a review facet, a
    border-depth sort, and a ▭ badge. **B** shows the cut on tiles and in the viewer (rejected candidates as
    yellow lines, peek under the cut on hover), and **N** marks a file as not a border.
  - **Save cropped copies…** writes the listed files cropped into a chosen folder, never over the originals:
    JPEG losslessly, other formats exactly with their metadata.
- **Schema (user decision, 2026-10-30):** one frozen baseline at version 1, then migrations. Every start compares a
  catalog's schema with the one this build creates (`pg-dbfile` compares each object's definition, the seed rows and
  the application id; the schema lives in `crates/catalog/src/schema.rs`). A catalog that differs is rebuilt: a new
  one in this build's layout is made, the old rows are copied into it with their ids, and the old file is kept as
  `catalog.sqlite.replaced` (then `.replaced.1`, …). A copy that can't take the rows leaves the new catalog empty,
  with the reason noted. The thumbnail cache starts over the same way. Nothing is deleted.
  **Status (2026-11-12):** the catalog is at version 9. `MIGRATIONS` holds eight steps after the baseline (versions
  2 to 9; version 2 is the C-8 change-stamp triggers, `TO_2_CHANGE_STAMPS`), and the migration loop runs them. The
  plan's "a difference is a refusal, not a rebuild" is not in the code: a schema that differs at its version is still
  rebuilt, as above.
- **Taken time frame (decided 2026-11-05, by the code's design, recorded here; not a user decision):** the date
  filters and the date sort compare `analysis.taken` as wall-clock time. A camera's clock is kept as it is. A
  video's UTC creation time is converted to this computer's zone at import and kept as the instant in
  `analysis.taken_utc`; when the system's zone changes (`meta.taken_zone` holds the zone's offsets on four fixed
  dates), `taken` is computed again from the instants. The instant is kept because the wall clock alone loses it in
  a repeated hour. Modified stays a UTC FILETIME; a local day's bounds are the instants at which the local clock
  first reads that midnight (`local_start_sql`: the earliest instant in a repeated hour, the transition instant
  when midnight is skipped), so the day starts where the local clock does.
- **SQL audit fixes (2026-11-05, by the code's design; not a user decision):** the schema check is described above
  (`crates/catalog/src/schema.rs`); a catalog that differs is moved aside as `catalog.sqlite.replaced`. The filter bar's facets are one grouped scan of the folder
  (`browse/read/facets.rs`; the per-facet statements remain as the test's reference). The writer's next job comes
  off the index `file_writes_next` on `((temp IS NULL), queued)`. Duplicates: a file without a pixel hash is in no
  group, since `GROUP BY` had put every NULL in one; PDQ band counts take one pass over `present_pdq_pairs`. The
  pairing's tier-2 volume set is built once per scan. Reads of a file's details and of the changes run in one
  snapshot; the writes that span rows are one transaction; every connection sets `busy_timeout`. The thumbnail
  cache checks its totals at most once a day (`HEAL_INTERVAL`, at open). Deferred: removing the `files_added`
  trigger (P5), which changes `seq`'s granularity and needs the thumbnail pass's tests first; and the duplicates'
  EXISTS against IN (P9), whose two audits disagree, so it waits for a measurement at 100k in Rust. The bench's
  LAYOUT is not regenerated, so the bench copy lacks `file_writes_next`; it changes the bench's speed, not results.
  The schema check (2026-11-05) finds a stored catalog that differs by its definitions, not by its names: the bench's
  stored data (`v2-windows-rules`) predates the current schema, so `prepare` moves it aside and the bench measured an
  empty catalog. The earlier remark that it only "lacks an index" was wrong. LAYOUT is now `v7-windows-rules` (and
  `v7-windows-rules-large`), and the bench asserts that its catalog was not rebuilt (`catalog.rebuilt().is_none()`).
  The bench data is made by `tools/profiling/bench_data.py` (it needs the synthetic library). It is not in the repo,
  so this check cannot tell whether it has been regenerated for v7.
- **Open: the video player.** Qt Multimedia plays videos, with hardware decoding off on a GPU without AV1
  decoding (a stopgap). The choice between our own engine on our FFmpeg 9 and libmpv is in `docs/VIDEO_PLAYER.md`.
- **Open for M4:** the grid loads the whole result list (filtered and sorted in SQL) into memory. That is fine
  per folder, but "All folders" at 70M needs the paged model of §7.

### M3 — Near-duplicates

- Tier 2 (embeddings + ANN, query-then-insert), grouping (multicut), the review UI with feedback buttons.
- **Status (2026-11-12):** not started in Rust. The Python prototypes in `eval/` (`embed.py`, `ann/`, `verify.py`,
  `verify_synth.py`, `group.py`) are the evidence behind §5 and §9. The review UI (group view, feedback buttons) is not built.

### M4 — Full library run

- Background ingestion of all 70M images with progress, pause/resume, and throttling while the UI is in use.
- **Status:** not started at 70M. The background passes (scan, import, thumbnails, borders) already run in the app (M2).

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
   import, at a size adjustable at runtime, in a size-capped cache (§6). Set in code: JPEG q85 with a 50 GB default
   cap. `pg-render`'s maker still lists the format as open (its comment), so this is worth a decision.
2. ~~**What runs in the first full pass**~~ **Decided 2026-10-30:** SSCD plus a semantic model, **SigLIP 2
   So400m/16 NaFlex** (native aspect ratio, 256 patches; TensorRT fp16), so keywords and semantic search need no
   second decode of the originals. Cost: ~154 GPU hours for 70M (~6.4 days), so the GPU, not decoding (~1.5 days),
   becomes the limit of that pass. NaFlex needs only ~208 × 304 px at 256 patches, so it can also run as a
   background GPU task on the import's downscaled working image instead of holding up the import (../eval/MODELS.md: "A second model" and "NaFlex").
3. **Mirrors in tier 2:** rely on SSCD's flip tolerance, or embed the flipped image too (2× GPU time). Measured
   on the subset: SSCD tolerates flips, so no flipped embedding, pending confirmation on the full set.
4. **Heavy crops:** if M0 shows SSCD misses small crops, consider region embeddings for reference images. That
   costs multiples of index size, and could be limited to cluster heads.
5. **ANN backend:** USearch (simple, about 27 GB RAM) vs FAISS (about 3 GB, training step, harder Windows
   build), or exact search on the GPU (10.4 G pairs/s on the 2060). Open: the comparison is being redone on the
   test set's real SSCD embeddings plus planted hard edits, with every index tuned to its best and the harness
   validated first; the first run, on unclustered synthetic vectors, was invalid (TIER2_ANN_BAKEOFF_PLAN.md "Nearest-neighbour search, first attempt").

6. **Embeddings and trimming:** design conclusion 3 trims borders before embedding, but the borders now come from
   a pass after the import (§4), while §4 puts embeddings in the import. Either embeddings move after the borders
   pass (a background GPU task, as decision 2 already allows for SigLIP 2), or the embedding pass runs the
   detector itself on a full-size decode. Not decided yet. Embeddings are not in the Rust import yet, so nothing is
   forced until M3; the eval's `embed.py --trim` runs the gallery's detector.

## Design conclusions so far

1. **Mirrors:** SSCD is flip-tolerant, so there is no need to store a flipped embedding
   (resolves PLAN §9 decision 3, pending confirmation on the full set).
2. **Heavy crops:** a single global threshold fails, but rank still works. Use k-NN rank plus a
   verifier for the ~0.4–0.88 band (Pinterest-style), not a lower global threshold.
3. **Borders:** trim borders before embedding, including blurred padding bars (a common
   Pinterest/TikTok edit seen in `dups2/075`). Measured 2026-10-30: fixes letterboxed copies, neutral elsewhere
   ("Trimming before embedding").
4. **Rotation:** SSCD still ranks copies at any rotation in the top 10 (R@10 1.00), but far below
   threshold, so rank + verifier covers it: the verifier retries a pair that doesn't match with the other
   image rotated in 45° (or 90°) steps, with no new false matches ("Verifier: TensorRT, rotations, SIFT,
   resolution"). Gate the retries to keep their cost down. Apply EXIF orientation everywhere; PDQ's dihedral
   query covers the exact-copy tier.
5. **Throughput:** read in MFT order with a deep queue (~1,600 files/s), decode with DCT scaling
   (~550 img/s on 12 threads), embed with SSCD fp16 on TensorRT (1,390 img/s; 410 in torch). The CPU is
   now the limit: decoding ~1.5 days for 70M, plus thumbnail encoding. SSCD fp16 is the model; 288px is
   the same quality (R@10, cross-group 0) if speed is ever needed. SSCD-large, DINOv2 and CLIP are
   dropped as candidate models. int8 is not used (see "TensorRT and ONNX Runtime").
6. **PDQ** is an optional cheap tier for near-exact copies (~80% of labeled pairs at d ≤ 22 with
   0 false positives), never a replacement for SSCD.
7. **Thumbnails:** Lanczos3 from a DCT-scaled decode kept ≥ 4× the target; display-only (not a
   re-analysis source). **Decision 2026-10-30:** not made during the import; a background task makes
   them afterwards at a size adjustable at runtime, into a size-capped cache (PLAN §6). The size/format
   numbers in "Thumbnails" now size that cache rather than a fixed 70M store.

## 10. Key references (excerpts in `docs/research/papers/`, with findings in `docs/research/REFERENCES_LEDGER.md`; the claims cited here and in §3 were checked against the papers on 2026-11-13)

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
