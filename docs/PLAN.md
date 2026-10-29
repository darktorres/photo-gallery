# Photo Gallery — Plan

Status: brainstorm / pre-implementation. Written 2026-10-28 from `photo gallery.txt`, the 25 papers in
`Research/`, and lessons from `DuplicateFinder/` and `ImageRater/`.

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

## 4. Ingestion: one pass over the HDD produces everything

Reading 21.4 TB from one HDD takes **about 27 h at an average 220 MB/s, best case (est.)**. With per-file seek
overhead across 70M files, it is more like **1–3 days**. A second pass costs the same again. Two rules follow:

- **Everything we might want from the original pixels is produced in the same pass:** file hash, pixel hash,
  PDQ, embeddings, thumbnails and EXIF.
- **Read in on-disk order with a single reader thread per HDD.** Parallel random reads on one spindle thrash
  the head. M0 benchmarks three orderings on a 100k sample: path order, NTFS file-ID (MFT record) order, and
  physical order (first-extent LCN via `FSCTL_GET_RETRIEVAL_POINTERS`).

```
Everything SDK / MFT ──► work list (file id, size, mtime), sorted by on-disk location
        │
  [1 reader per HDD]  sequential reads → raw bytes ─► BLAKE3/xxh3 of file bytes   (tier 0a)
        │ bounded channel
  [CPU decode pool]   libjpeg-turbo with DCT downscale (1/2, 1/4, 1/8) where possible
        │             → EXIF orientation applied → sRGB
        ├─► hash of decoded pixels                                   (tier 0b: metadata-only differences)
        ├─► PDQ, 256 bits + 8 dihedral variants (rotations and mirrors) (tier 1)
        ├─► grid thumbnail + ThumbHash placeholder
        └─► model input (frozen, antialiased resize) ─► [GPU batcher] ─► embeddings (tier 2)
                                                     │
  [catalog writer]   one SQLite transaction per batch → crash-safe, resumable
```

- **Resumable at any point.** Each file records which stages are done, together with the version of the
  pipeline that produced them.
- **File identity is (volume serial, NTFS file ID), plus size and mtime.** Moves and renames then need no
  rehash. Later, the NTFS USN journal (or Everything) provides incremental changes.
- **Compute estimates (est.):**
  - Decoding Pinterest-size JPEGs at roughly 2–5 ms each across 12 threads is about 3k img/s, so the CPU keeps
    up.
  - SSCD-class ResNet50 at about 288 px with fp16/int8 TensorRT on the 2060 gives about 500–1000 img/s, i.e.
    20–40 h. That overlaps with the I/O, so wall-clock is roughly max(I/O, GPU) ≈ **1.5–3 days** for the first
    pass. After that, only new files are processed.

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

**Grouping:**

- Verified edges → union-find → split pass that re-checks members against a pivot (Pinterest k-cut).
- Each group gets a canonical **keeper**: the largest pixel area that contains the others, then the highest
  quality.
- "Not a duplicate" decisions are stored per pair and are never re-proposed.

---

## 6. Storage layout (D:)

| Store | Tech | Size at 70M (est.) |
|---|---|---|
| Catalog: files, hashes, ratings, tags, groups, pair decisions, pipeline versions | SQLite (WAL), `rusqlite` | 20–30 GB |
| Embeddings (source of truth) | Flat memory-mapped fp16 file, row = catalog id | ~72 GB |
| ANN index | USearch or FAISS file | 3–30 GB |
| PDQ multi-index | Rebuilt in RAM at startup from the catalog | (RAM) |
| Grid thumbnails | **Packed shard files** (not 70M loose files), shards in folder order so one screen ≈ one sequential read | 70M × ~5 KB ≈ **350 GB** at ~200 px (est.); size and format decided in M0 |
| ThumbHash placeholders | Column in the catalog | ~2 GB |

Thumbnails make browsing work even when the originals are offline (Plex-like, from the notes). They are also why
the thumbnail size is a real decision: a ≥256 px thumbnail would let *future* models (e.g., CLIP keywords) run
without another 1–3 day pass over E:, but it costs ~2–3× the disk space.

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
  that reads the packed shards. ThumbHash placeholders show instantly, and images fade in (Wallhaven-style).
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

**M0 — Evaluation and benchmarks (before any 70M work)**

- Copy a ~200k sample to D:, including known Pinterest crop/edit groups. Hand-label ~2k pairs in three
  classes: duplicate, crop/edit duplicate, not a duplicate.
- **Model bake-off** on crop recall at very low false-positive rates:
  - SSCD (`disc_mixup`) vs DINOv2 vs CLIP vs PDQ.
  - Measure flip tolerance, to decide whether to embed mirrored images too.
  - Measure crop-size limits, to decide whether reference images need *region embeddings*.
  - Python is used here only for ONNX export and the evaluation notebooks.
- **I/O benchmark** of read orderings on E: (path vs file ID vs LCN).
- **Throughput benchmarks:**
  - Decode with `turbojpeg` DCT scaling vs `zune-jpeg` vs `image`.
  - Resize with `fast_image_resize`, verified with the aliasing paper's downsampled-circle test.
  - Thumbnail encode with JPEG (turbo/jpegli) vs WebP.
  - `ort` with CUDA vs TensorRT EP, fp16 vs int8.
- **ANN bake-off** on 5M vectors: USearch HNSW int8 vs FAISS IVF-PQ, measuring recall at radius, RAM and
  insert speed.
- **Exit criteria:** chosen model, thresholds, thumbnail format and size, index type, and read order, each
  backed by a number.

**M1 — Core and ingest (on the sample)**

- Cargo workspace (`core`, `ingest`, `ui`), catalog schema, Everything-based scanner, the single-pass
  pipeline, tiers 0–1, thumbnails, resumability.

**M2 — Desktop UI**

- QML grid with lazy model and thumbnail provider, viewer and loupe, ratings in the catalog, merged-drives
  tree.

**M3 — Near-duplicates**

- Tier 2 (embeddings + ANN, query-then-insert), grouping and split pass, the review UI with feedback buttons.

**M4 — Full library run**

- Background ingestion of all 70M images with progress, pause/resume, and throttling while the UI is in use.

**M5 — Precision**

- Tier 3 verifier trained on review labels, local-feature check for heavy crops, keeper selection.

**Later (from the notes):**

- AI keywords and semantic search (CLIP/SigLIP).
- A pix-plot-style 2D map (UMAP).
- Format conversions (ImageReduce; GIF→WebP/WebM; JPEG XL/jpegli).
- gallery-dl / Grabber integration.
- Writing ratings back to XMP where the format supports it.

---

## 9. Open decisions

1. **Thumbnail size:** small (~200 px, ~350 GB) vs model-reusable (≥256 px, ~2–3× larger, makes later re-analysis possible
   without re-reading E:).
2. **What runs in the first full pass:** SSCD only, or SSCD plus a semantic model (CLIP/SigLIP) so keywords and
   semantic search never need a second pass. A semantic model would roughly double GPU time.
3. **Mirrors in tier 2:** rely on SSCD's flip tolerance, or embed the flipped image too (2× GPU time). M0
   measures this.
4. **Heavy crops:** if M0 shows SSCD misses small crops, consider region embeddings for reference images. That
   costs multiples of index size, and could be limited to cluster heads.
5. **ANN backend:** USearch (simple, about 27 GB RAM) vs FAISS (about 3 GB, training step, harder Windows
   build).

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
