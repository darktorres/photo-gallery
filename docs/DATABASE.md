# Database

Whether the catalog should move off SQLite before the 70M library, and what should hold the vectors for semantic
search later (2026-11-01). Nothing here was built or run. The facts come from each project's README, manual or
release notes, read on that date, and from Google and GitHub repository search sorted by stars (`embedded
database`, `topic:embedded-database`, `topic:database language:Rust`, `topic:sql language:Rust`,
`topic:vector-database`, `topic:approximate-nearest-neighbor-search`, `topic:full-text-search language:Rust`,
`hybrid search database`, `embedded vector search`). The 70M benchmark (§6) is what decides.

## What the catalog needs

From the code as it is (`crates/core/src/catalog.rs`, `browse.rs`, `query.rs`, `thumbs.rs`):

- **About 57 GB at 70M.** The real catalog (`D:\PhotoGallery\catalog\catalog.sqlite`) is 154 MB for 189,518 files,
  ~810 B per file. That is twice PLAN §6's 20–30 GB. The biggest item is `order_key` (134 B on average), stored in
  `files` and again in the `files_order` index: ~270 B per file, ~19 GB at 70M.
- **SQL built at run time:** `query.rs` turns the user's query into SQL (803 lines). Facet counts, folder counts,
  name order, joins between `files` and `analysis`.
- **Triggers and views:** `files_added`, `files_changed`, `analysis_*` keep `pending`, `seq` and the totals;
  `present_pdq_pairs` is a view.
- **More than one process:** the app and `gallery` (import, thumbs) open the same files at the same time.
- **Windows, from Rust, in-process.** No server for the user to run.
- **Writes in batches** (the import's transactions), point updates (ratings, rotations, border marks), reads
  while writing.

Later: semantic search (SigLIP 2 B/16 is 768-d, ~107 GB in fp16 at 70M, FINDINGS "A second model"), text search
over names, tags and maybe captions or OCR, face embeddings, places, the duplicate groups as a graph.

## Verdict

- **Keep SQLite for the catalog.** None of the alternatives handles 70M rows better at what the catalog does, and
  the ones closest to it (Turso, Stoolap) lack what the catalog uses (triggers, views, several processes) or are
  too young to trust with the only copy of the ratings. What breaks at 70M is ours to fix in any engine: the grid
  reads its whole scope into a `Vec` (`browse::list_grid`), and the facet and folder counts scan the scope on every
  query (PLAN §7 already plans the paged grid).
- **Keep the vectors out of the catalog**, as PLAN §6 already has them: the flat fp16 file is the source of truth,
  and an index beside it serves the searches. No embedded SQL engine has a vector index that holds 70M × 768-d
  on this machine except by building a separate index, which is what we'd do anyway.
- **For semantic search, measure three:** VectorChord's design (IVF + RaBitQ, ~12 GB of RAM for 100M × 768-d)
  through **LanceDB** (IVF_RQ, Rust, in-process) or **zvec** (IVF-RaBitQ / DiskANN, Windows MSVC builds), against
  **our own flat scan of 1-bit RaBitQ codes** with exact re-ranking from the fp16 file. At 70M × 768-d the 1-bit
  codes are 6.7 GB, small enough to scan whole, and a scan takes any filter (folder, date, rating) for free.
- **Revisit Turso at 1.0.** It is the only project that could replace SQLite under the same files and SQL, and it
  is moving fast (concurrent writers, FTS on tantivy). Today its manual lists no triggers, no views and no
  multi-process access.

## 1. SQL engines that could hold the catalog

| Engine | Stars | Language | Storage | Fits the catalog? |
|---|---|---|---|---|
| **SQLite** (`rusqlite`, current) | — | C | Row B-tree, WAL | Yes. Every feature we use; FTS5, R*Tree, JSON, recursive CTEs built in |
| **Turso** (ex-Limbo) | 25k | Rust | SQLite's file format, plus MVCC | Not yet: no triggers, views, multi-process (§2) |
| **libSQL** | 17k | C (SQLite fork) | SQLite + DiskANN-style vector index | Works, but Turso put its effort into the rewrite instead |
| **DuckDB** | 43k | C++ | Columnar | No: one read-write process, made for scans, not point updates (§3) |
| **Stoolap** | 1.3k | Rust | In-memory MVCC hot buffer + columnar cold volumes | Too young to trust (v0.4); its design is interesting (§4) |
| **KiteSQL** | 760 | Rust | SQL over RocksDB or LMDB | Young; SQL on a KV store, nothing we lack today |
| **GlueSQL** | 3.1k | Rust | SQL engine over pluggable stores (redb for disk) | Made for flexibility, not for 70M rows |
| **SurrealDB** (embedded) | 33k | Rust | RocksDB / SurrealKV | Its own query language (SurrealQL), BSL licence, document-graph model |
| **Embedded Postgres** + pgvector / VectorChord / ParadeDB | — | C / Rust | Postgres heap | A server process to ship and run; VectorChord and ParadeDB release no Windows builds (§5) |
| **pgrust** | 5.7k | Rust | Postgres rewritten in Rust | A server; young |
| **seekdb** (OceanBase) | 3.2k | C++ | MySQL-compatible, vector + FTS | Linux and macOS only, no Rust SDK |
| **chDB** | 2.9k | C++ | ClickHouse in-process | Analytics; Python-first |
| HeliosDB-Nano | 10 | Rust | — | Its own README says trigger bodies never run |

Also seen and set aside: CozoDB (Datalog + HNSW, no commit since 2024-12), HelixDB (graph + vector, now built on
object storage), toydb and RisingLight (educational), chai (Go).

## 2. Turso

The Rust rewrite of SQLite by the company behind libSQL, now its main project. It reads and writes SQLite files,
speaks SQLite's dialect and C API, and adds `BEGIN CONCURRENT` (MVCC, row-level conflict detection), change data
capture, encryption, exact vector search with float32, int8 and 1-bit types, and experimental FTS on tantivy.
v0.8.0 (2026-09-29) made FTS work with MVCC. The README says it "runs in production today at multiple
organizations", and is pre-1.0 with "keep independent backups".

What stops us, from its manual (`docs/manual.md`, "Limitations"):

- **No triggers and no views.** The catalog keeps `pending`, `seq` and the totals with triggers, and has a view.
- **No multi-process access** (the README lists multi-process WAL as experimental). The app and `gallery` run at
  the same time.
- **No multi-threading, no savepoints, no vacuum.** `thumbs.rs` frees pages with `incremental_vacuum`.
- **Vector search is exact only**; an index ("similar to libSQL vector search") is on the roadmap. Async I/O
  (`io_uring`) is Linux-only.

For 70M rows it would change nothing anyway: it is the same B-tree with the same page format. Its gain is
concurrent writers, which our batched import doesn't need.

## 3. DuckDB

The analytics counterpart of SQLite: columnar, vectorized, in-process. Faster than SQLite by 10–100× on
aggregates over large tables, which is what the facet counts are. But:

- One process opens a file read-write; others can only open it read-only. The app and `gallery` both write.
- Point updates and small transactions are its weak side, and the catalog has many (ratings, `pending`).
- Its vector extension (`vss`) keeps the HNSW index fully in RAM, outside DuckDB's memory limit, and builds it
  only on in-memory tables unless an experimental flag is set (duckdb.org, "Vector Similarity Search
  Extension"). Not usable at 70M.

Possible later as a read-only side engine for reports over a copy of the catalog. Not as the catalog.

## 4. Pure-Rust engines and stores

- **Stoolap** (v0.4): MVCC with a hot in-memory buffer and WAL, cold data in immutable columnar volumes (zone maps,
  bloom filters, LZ4) merged by background compaction; cost-based optimizer, parallel queries, HNSW. Built for
  row writes and analytic reads at once, which is our mix. Too young for the only copy of the ratings; worth a
  look again later.
- **Key-value stores** (redb: stable B-tree, "file format is stable"; fjall: LSM like RocksDB; heed/LMDB and
  libmdbx: memory-mapped B-trees; SurrealKV; sled: still beta). Moving to one means writing by hand what SQLite
  does now: the indexes, the query planner for `query.rs`'s queries, the triggers, the migrations. At 70M they
  have the same B-tree costs, except fjall's LSM, which writes random keys (BLAKE3 hashes, `order_key`) much
  more cheaply than a B-tree once the indexes outgrow the cache. That one cost has a cheaper fix in SQLite:
  sort each batch by key, raise the cache, or build those indexes after a bulk import.

## 5. Vector search at 70M

The size decides the design. At 70M:

| Representation | 512-d (SSCD) | 768-d (SigLIP 2 B/16) |
|---|---|---|
| fp16 (source of truth, PLAN §6) | 72 GB | 107 GB |
| int8 | 36 GB | 54 GB |
| RaBitQ 4-bit | 18 GB | 27 GB |
| RaBitQ 1-bit | 4.5 GB | 6.7 GB |

The machine has 64 GB of RAM and a 6 GB GPU, and D: is an NVMe SSD (Kingston NV2, 2 TB), so random reads of the fp16
rows for re-ranking are cheap. FINDINGS already showed USearch HNSW would need ~71 GB in int8 and 15+ hours to
build. What fits is a compressed index in RAM plus re-ranking from disk:

| Engine | Stars | In-process from Rust on Windows | Index for 70M | Filters | Notes |
|---|---|---|---|---|---|
| **LanceDB / Lance** | 12k / — | Yes (Rust crate; needs `protoc`) | IVF_PQ, IVF_RQ (RaBitQ), on disk | Prefilter with SQL (DataFusion) | Columnar files with versions; FTS (BM25); append-friendly, updates via deletion files and compaction |
| **zvec** (Alibaba) | 16k | Yes (`zvec-rust`, MSVC prebuilt or built from source) | IVF-RaBitQ, HNSW-RaBitQ, DiskANN on Windows since v0.8.0 | Scalar filters + FTS in one query | "Multiple processes can read; writes are single-process exclusive" |
| **Qdrant Edge** | 35k (Qdrant) | Yes (`qdrant-edge` crate) | HNSW with on-disk vectors, scalar / binary quantization | Payload indexes | The Qdrant engine as a library; HNSW graph cost at 70M is the open question |
| **DiskANN3** (Microsoft) | — | Library: we implement its `DataProvider` over our files | Vamana graph on disk, PQ in RAM | Hooks for predicates | Rust rewrite; real-time inserts and deletes (IP-DiskANN) |
| **hannoy** / arroy (Meilisearch) | — / 300 | Yes (on LMDB via heed) | HNSW (hannoy) or random projections (arroy) on disk | Roaring bitmaps | ~200 B of graph per vector |
| **VectorChord** (Postgres) | — | Postgres server only; no Windows release | IVF + RaBitQ: 100M × 768-d indexed in 20 min with 12 GB of RAM (its blog, 2025-12) | SQL | What Immich runs since v3 (pgvecto.rs dropped). AGPL / ELv2 |
| **sqlite-vec** | — | Yes (SQLite extension) | None: exact scan of float / int8 / bit vectors | SQL | Pre-1.0; a full int8 scan at 70M is 54 GB per query |
| DuckDB `vss`, Chroma, USearch, hnswlib | — | — | HNSW in RAM | — | Don't fit at 70M |

The **flat 1-bit scan** is the baseline to beat. RaBitQ's 1-bit codes with an error bound (VectorDB-NTU's
RaBitQ-Library) are 6.7 GB at 768-d. A scan of all of them is memory-bandwidth bound (tenths of a second on this
machine), then the best few thousand are re-ranked exactly from the fp16 file on D:. It has no recall cliff (the
HNSW and IVF results in FINDINGS "ANN" are why that matters), no index to build or maintain, and filtering is a
bitmap AND before the scan. It is also what the duplicate pass already does on the GPU, only for one query.

## 6. What to measure

On a synthetic catalog with 70M rows (~57 GB, same schema, `D:\PhotoGallery\scratch_scale\`):

- The import: batches of new files with every index in place, at 50M+ rows. Then the same with batches sorted by
  key and a larger cache.
- The folder tree's counts, the facets, `pending()`, and the grid paged by `order_key` (first page, a jump to row
  N, scrolling).
- Opening and the first frame.

Then, on 70M synthetic 768-d vectors (or the real ones once they exist): the flat 1-bit scan with exact
re-ranking, LanceDB IVF_RQ and zvec IVF-RaBitQ. For each: recall@10 against exact, query time, RAM, build time,
insert cost per 10,000 new images.
