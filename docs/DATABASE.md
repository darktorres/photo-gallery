# Database

Whether the catalog should move off SQLite before the 70M library, and what should hold text search, vectors and
the duplicate graph (2026-11-01).

How it was found:

- **Candidates:** GitHub repository search sorted by stars (`topic:database language:Rust`, `topic:embedded-database`,
  `topic:sql language:Rust`, `topic:vector-database`, `topic:approximate-nearest-neighbor-search`, `topic:ann`,
  `topic:hnsw`, `topic:diskann`, `topic:rabitq`, `topic:olap`, `topic:key-value-store language:Rust`,
  `topic:graph-database language:Rust`, `topic:search-engine language:Rust`, `topic:sqlite-extension`,
  `topic:photo-management language:Rust`), Google, and Reddit (r/rust, r/sqlite, r/vectordatabase, r/selfhosted,
  r/DataHoarder, r/digiKam, r/immich, r/dataengineering, r/LocalLLaMA; threads read in full).
- **Evidence:** each candidate's README, manual, compatibility list, changelog and issues, and the code where those
  disagreed.
- **Hands-on:** small tests on this machine (i7-8700, Windows 10) with prebuilt wheels and binaries, no builds: 1–2M rows
  for the catalog engines, 1M random 768-d vectors for the vector engines. Nothing ran at 70M; §8 is that benchmark.

## What the catalog needs

From the code (`crates/core/src/catalog.rs`, `browse.rs`, `query.rs`, `thumbs.rs`):

- **About 57 GB at 70M.** The real catalog (`D:\PhotoGallery\catalog\catalog.sqlite`) is 154 MB for 189,518 files,
  ~810 B per file, twice PLAN §6's 20–30 GB. The biggest item is `order_key` (134 B on average), stored in `files` and
  again in `files_order`: ~270 B per file, ~19 GB at 70M.
- **SQL built at run time** (`query.rs`, 803 lines): facet counts, folder counts, name order, joins of `files` and
  `analysis`, keyset paging on a BLOB key.
- **Row triggers and a view:** `files_added`, `files_changed`, `analysis_*` keep `pending`, `seq` and the totals;
  `present_pdq_pairs` is a view. Partial indexes. `auto_vacuum = INCREMENTAL` in the thumbnail cache.
- **Two processes write the same file** at once: the app and `gallery` (import, thumbs).
- **Windows, from Rust, in-process.** Batched writes plus point updates (ratings, rotations), reads while writing.

Later: semantic search (SigLIP 2 B/16, 768-d, ~107 GB in fp16 at 70M, FINDINGS "A second model"), text search over
names, folders, tags and maybe captions or OCR, the duplicate groups.

## Verdict

- **The catalog stays in SQLite.** Of more than 20 engines and stores checked, only embedded Postgres meets every requirement, and it means
  shipping and supervising a server process for no gain at this scale. Turso is the one to revisit: it has caught up on
  triggers and views, but two writing processes are still "not production ready" and it was 2× slower than SQLite here.
  SQLite's limits (281 TB, 2⁶⁴ rows) are far away; what breaks at 70M is ours, in any engine (§3).
- **Facet and folder counts must stop scanning.** SQLite took 0.6–0.9 s for a GROUP BY over 2M rows here, so ~20–30 s
  over 70M. Keep the counts up to date in the transaction (the thumbnail cache already does this with triggers), or
  hold the facet columns in RAM (~11 B per file, ~0.8 GB), or let DuckDB read the catalog for them (§3).
- **The grid must page.** `browse::list_grid` reads the whole scope into a `Vec`: 2.2 s for 189k rows, minutes and
  tens of GB at 70M. Keyset paging on `order_key`, plus a sparse in-RAM table of every ~1000th key for the scrollbar
  (PLAN §7).
- **Text search: FTS5 with the trigram tokenizer**, contentless, in its own attached file (§5).
- **Vectors stay beside the catalog**, as PLAN §6 has them. Benchmark **Lance** (IVF_RQ/IVF_PQ), **zvec** (IVF-RaBitQ,
  DiskANN) and **Qdrant Edge** against a **flat 2-bit scan with exact re-ranking** from the fp16 file (§6). My first
  draft said a flat 1-bit scan would take tenths of a second at 70M; measured scans say about 1–2 s, so it is a
  baseline, not the answer.
- **Duplicate groups need no graph engine:** a group table, and recursive CTEs where a walk is needed (§7).

## 1. Catalog engines

Checked against the catalog's needs, with each project's own documents and a hands-on test where a wheel or binary
exists. "Two writers" means two processes writing one database.

| Engine | Larger than RAM | Two writers | Row triggers, views | Partial indexes | Windows + Rust | Deciding fact |
|---|---|---|---|---|---|---|
| **SQLite** (current) | Yes | Yes (one at a time, WAL) | Yes | Yes | Yes (`rusqlite`) | Meets everything; one writer at a time |
| **Turso** 0.8.3 | Yes | Experimental flag, "not production ready" | Yes (COMPAT.md; the manual is stale) | Used only on exact match | Yes | 2× slower than SQLite here; no `incremental_vacuum` |
| **Embedded Postgres** 18.6 | Yes | Yes (server) | Yes | Yes | Yes (`postgresql_embedded`, MSVC binaries) | A server process to ship; triggers made inserts 7× slower |
| **DuckDB** 2.0 | Yes | No (one read-write process) | Statement-level only; row-level "not yet supported" | No | Yes (`duckdb-rs`) | GROUP BY 150× faster than SQLite; point updates 50–70× slower |
| **Stoolap** 0.4.2 | Cold data on disk, hot rows in RAM | No | No triggers | No | Yes | Indexes cover hot rows only: key lookup 14 ms, ordered page 592 ms at 2M |
| **seekdb** 1.4.0 | Yes | Yes, through a shared server | Yes | No | Windows wheel; Rust crate | "Embedded" is a 127 MB server process; Windows startup bug #1544 |
| **SurrealDB** 3.3 | Yes (LSM) | No (lock file since SurrealKV 1.0) | `DEFINE EVENT` in the transaction | No | Yes | GROUP BY over 1M rows took 19.8 s; no SQL joins |
| **CozoDB** 0.7.6 | Yes | Only on its SQLite backend | Triggers, no views | No | Yes | Unmaintained since 2024; no storage stability before 1.0 |
| **KiteSQL** 0.6 | Yes (RocksDB/LMDB) | No | No triggers | No | Rust | No BLOB type; TPC-C at one warehouse only |
| **GlueSQL** 0.20 | Yes (redb) | No | No | No | Rust | No secondary indexes on redb; scans collect into memory |
| **chDB** | — | — | — | — | No Windows build | ClickHouse doesn't compile on Windows |
| **pgrust** 0.4-beta | Yes | Server | Yes | Yes | No Windows plans | Beta, "a lot of bugs" (README) |
| **HelixDB** 3.5 | Yes | No ("detected newer DB client") | — | — | Wheel | One writer shuts the other out |
| **Grafeo** 0.6 | Main graph store in RAM | No | — | — | Wheel | Graph database; LpgStore can't go to disk |
| MatrixOne, HeliosDB-Nano | | | | | | A Kubernetes server; a README saying trigger bodies never run |

Key-value stores (no SQL: the query layer, indexes, triggers and migrations would be ours to write):

| Store | Two processes | Notes |
|---|---|---|
| **redb** 4.4 | Reads yes (3.0); writes "experimental" (4.3), `MultiWriter` unreleased (4.5) | B-tree, "the file format is stable" |
| **fjall** 3.1 | "may **not** be loaded in parallel from separate processes" | LSM; 3.0 "comfortably beats sled and redb in most workloads" (its author) |
| **heed / LMDB, libmdbx** | Yes (readers + one writer) | One report of LMDB degrading above tens of millions of keys with frequent writes (r/rust) |
| canopydb, sled | — | "Do not trust it with production data"; sled is a stalled rewrite |

Published key-value numbers (canopydb's BENCHMARKS.md, older versions, 5M keys larger than a 4 GB memory limit, Linux):
fjall did 7.37M writes and 3.95M reads in 3 minutes, sled 5.40M and 0.67M. Random inserts with fsync per commit:
canopy 186k, redb 170k, fjall 133k, heed 75k in a minute; write amplification fjall ~10×, redb ~48×. The LSM wins on
random-key inserts, which is the one place the catalog could gain (BLAKE3 hashes, `order_key`).

### Hands-on, 2M rows (random 32-byte keys, unique index, 100k-row transactions, default settings)

| Engine | Insert | Size | Lookup by key | GROUP BY | Page of 200 by key | Point UPDATE |
|---|---|---|---|---|---|---|
| SQLite 3.49 | 48–55k rows/s | 276 MiB | 11 µs | 610–860 ms | 0.15 ms | 76–101 µs |
| DuckDB 2.0 | 74k rows/s | 391 MiB | 1.3 ms | 4 ms | 16 ms | 5.4 ms |
| Stoolap 0.4.2 | 48k rows/s | 178 MiB | 14 ms | 42 ms | 592 ms | 32 µs |
| Turso 0.8.3 (CLI, same SQL as SQLite) | 14k vs SQLite's 29k rows/s | | 20k lookups: 473 vs 216 ms | 242 vs 189 ms | | |

And 1M rows with an index on `(dir, key)`, GROUP BY over 5,000 dirs: SurrealDB 2.0 89 s insert, 19.8 s GROUP BY;
Cozo/RocksDB 24 s, 1.8 s; seekdb 25 s, 0.05 s; Grafeo 36 s, 0.30 s; Postgres 16 s, 0.14 s (110 s with a row trigger).

### SQLite itself

- Limits: "about 281 terabytes", 2⁶⁴ rows (`limits.html`); client/server is suggested near "the terabyte range"
  (`whentouse.html`).
- `BEGIN CONCURRENT` is on a branch, not in a release (active, 2026-10-27); commits still serialize. HC-tree is a
  prototype: "only works on POSIX, not win32", one process, "no support for recovering from a power failure".
- One writer at a time is the real constraint. Turso measured SQLite's p99.9 at 1.2 s with 32 writing connections
  (writers sleep on the lock) against 2.4 ms for its MVCC; with one connection SQLite is faster (Turso, P99CONF,
  2026-10). Our writers are two, batched; the importer's transactions should stay short so the app's writes (ratings)
  don't wait behind them.
- Random keys hurt once an index outgrows the cache: one report has 1M-row batches slowing from 0.7 s to 12.6 s at
  100M rows with random keys (setup unknown). That is our BLAKE3 and `order_key` indexes. Fixes: sort each batch by
  key, a larger cache plus `mmap_size`, or build those indexes after a bulk import.

### Turso

The Rust rewrite of SQLite, its company's main project (libSQL is maintained, but "new features are being developed in
Turso"; libSQL's DiskANN index hasn't changed since 2024).

- **Caught up:** COMPAT.md marks `CREATE TRIGGER`, `CREATE VIEW`, `SAVEPOINT`, `ATTACH` as supported. Tested: AFTER
  INSERT/UPDATE OF … WHEN/DELETE triggers kept a counter right; views and savepoints work. The manual's
  "Limitations" (no triggers, no views) is out of date.
- **Not yet:** a second process gets "File is locked by another process" unless both pass
  `--experimental-multiprocess-wal --vfs experimental_win_iocp`; then both wrote and saw each other's rows, but the
  manual says "not production ready so do not use it for critical data", and 0.6–0.8's changelog is full of Windows
  WAL fixes. `ATTACH` needs an experimental flag. `incremental_vacuum` is unsupported. A partial index is used only
  when the query repeats its condition exactly (`WHERE rating = 5` scanned; SQLite used the index). Vector indexing is
  "roadmap"; FTS (tantivy) experimental. "We don't support mixed SQLite and Turso in multi-process scenarios."
- **Quality:** deterministic simulation, Antithesis, differential fuzzing, formal methods (README; the CEO's AMA,
  r/IAmA 2026-06). Users call it reliable outside production; no 1.0 date.
- **Would replace SQLite when:** two-process WAL leaves experimental on Windows and passes our own two-writer crash test;
  `incremental_vacuum` lands; the planner uses implied partial indexes; it matches SQLite at 70M with our settings.
  The reason to move would be `BEGIN CONCURRENT`, which our batched importer doesn't need.

### DuckDB

The columnar engine, and the one that answers facet queries 150× faster. Not the catalog: one read-write process
(others read-only, or DuckDB as a server), row-level triggers unsupported, no partial indexes, its indexes "do not
affect … join, aggregation, and sorting", UPDATE on an indexed column is DELETE + INSERT. Its `vss` HNSW index lives
fully in RAM outside the memory limit. It can **attach a SQLite file and read it while SQLite writes** (WAL); that is
the side path for counts in §3.

### Embedded Postgres

The only alternative that meets every catalog need: theseus-rs ships Windows MSVC binaries, `postgresql_embedded`
bundles them, and triggers, partial indexes, keyset paging and two writers all worked. pgvector builds on Windows
(`nmake /F Makefile.win`); VectorChord builds with MSVC per its maintainer but ships no Windows package. The cost: a
server process to start, find and stop from both the app and `gallery`, a port of `query.rs`'s SQL, and slower writes
(PL/pgSQL row triggers took the 1M insert from 16 s to 110 s). Immich runs Postgres + VectorChord, but as a server app.
Worth it only if the catalog needed many concurrent writers or server-side vectors; it doesn't.

### SurrealDB

Embedded in Rust, multi-model, but: one process per database (SurrealKV 1.0 added a lock file; the older Python wheel
had none, and a second process silently wrote to the same log), no SQL joins, GROUP BY over 1M rows 19.8 s here, HNSW
graph in RAM, DiskANN with unquantised vectors. In August 2025 it shipped with fsync off by default
(`SURREAL_SYNC_DATA`), which its founder confirmed (r/rust, 678 points); 3.x now syncs every commit by default. Users
report replacing embedded SurrealDB with SQLite ("up to 20 times faster in bulk inserts", much less RAM) and
undocumented breaking changes from 2.x to 3.x. Its benchmark page beats SQLite with 128 × 48 concurrent connections,
which measures SQLite's lock, not its engine. Licence BSL 1.1 (desktop use allowed), Apache-2.0 from 2030.

## 2. What other photo managers use

| App | Catalog | Vectors | Text | Scale |
|---|---|---|---|---|
| Lightroom Classic | SQLite (`.lrcat`) | — | — | Catalogs "into millions of photos" (Lightroom Queen) |
| digiKam | 4 SQLite files (core, thumbs, similarity, recognition) or MariaDB | in SQLite | — | A developer: SQLite with WAL on NVMe now does well, the docs' MariaDB advice needs correcting; users run 1–1.5M images on SQLite |
| Spacedrive | SQLite (SeaORM/sqlx), splitting into one database per source | LanceDB (stub) | Dropped FTS5 for names: an in-RAM name registry is faster for prefixes | "microseconds at million-file scale" from an in-memory arena |
| Urocissa | redb, memory-first | — | — | "one million photos on a 4 GB RAM server" |
| Ente (mobile) | SQLite | usearch HNSW in Rust | — | — |
| Immich | Postgres | VectorChord (moved from pgvecto.rs: index build 28 min → 29 s, RAM 1090 → 645 MiB on 166k faces) | pg_trgm | Users run 250k+ |
| PhotoPrism | MariaDB; SQLite "not a good choice" (lock errors while indexing) | — | — | — |
| lap | SQLite | BLOBs, scanned | `LIKE` | 100k+; "not validated" at 3M |

The pattern: an embedded store for the catalog, hot metadata such as names in RAM, vectors in a separate index,
trigram search for substrings, and caches in files of their own.

## 3. What breaks at 70M, whatever the engine

- **The grid** (`browse::list_grid`) materializes the whole scope. Keyset paging on `order_key` and a sparse key table.
- **Counts.** Folder tree and facets run `count(*) … GROUP BY` over the scope: 0.6–0.9 s at 2M here, ~20–30 s at 70M.
  The tracewayapp benchmark (2026-07) found SQLite dashboards stop loading within 5 s at 100k–1M rows and DuckDB 100×
  later. Three ways out, in order of preference:
  1. Counters kept in the transaction by triggers, per folder and per facet value (as the thumbnail cache's totals).
     Exact for the unfiltered views; filtered views still need a scan.
  2. The facet columns (folder, type, extension, rating, date, size class) in RAM, ~11 B per file, ~0.8 GB, scanned
     by all cores: tens of milliseconds for any filter. Rebuilt from the catalog at startup, like the PDQ index.
  3. DuckDB attached to the catalog read-only, for ad-hoc analytics.
- **Random-key index inserts** (above).
- **The import's batches against the app's writes:** short transactions, `BEGIN IMMEDIATE`, a busy handler.

## 4. Size of `order_key`

~19 GB of the 57 GB. Storing it only in the index (a covering index, or `WITHOUT ROWID` keyed by it) or shortening it
(folder id + name key instead of the full folder key per file) would save most of that. To be measured in §8.

## 5. Text search

- **SQLite FTS5** with the trigram tokenizer: substring search, also serves `LIKE`/`GLOB`; matches need 3+ characters.
  `contentless_delete=1` (3.43+) keeps no copy of the text and still deletes; `detail=none` shrinks the index (an
  example in the docs: 743 MiB full, 134 MiB none, word tokenizer). In its own attached file so it can be rebuilt.
  Index folder paths once per folder, not per file. `ORDER BY rank` is slow at millions of rows (r/sqlite); names
  don't need ranking.
- **tantivy** (16k stars, MIT, Windows): for ranked captions or OCR later. Open Windows bug: a single IndexWriter's
  commit fails with "PermissionDenied (OS error 5)" from its merge thread (#2847).
- **SeekStorm** (Apache-2.0, 2024): no character-level substring search, internal document ids only, self-published
  benchmarks. No.

No published trigram index size for file names exists; measure on the real names.

## 6. Vector search at 70M

Sizes at 70M:

| Representation | 512-d (SSCD) | 768-d (SigLIP 2 B/16) |
|---|---|---|
| fp16 (source of truth, PLAN §6) | 72 GB | 107 GB |
| int8 | 36 GB | 54 GB |
| 4-bit (RaBitQ, TurboQuant) | 18 GB | 27 GB |
| 2-bit | 9 GB | 13 GB |
| 1-bit | 4.5 GB | 6.7 GB |

The machine has 64 GB of RAM, a 6 GB RTX 2060, and D: is an NVMe SSD (Kingston NV2), so exact re-ranking from the fp16
file is cheap. FINDINGS "ANN": USearch HNSW would need ~71 GB and 15+ hours; HNSW and IVF lost many duplicate partners
on random data.

### Hands-on, 1M random normalized 768-d vectors

Random data is the worst case for ANN recall, IVF most of all; the recall shows the quantizer, not SigLIP. Latency is
one query at a time, with a VM running in the background.

| Engine / index | Build | Build peak RAM | Index | Recall@10, p50 |
|---|---|---|---|---|
| turbovec 4-bit, flat scan | 25 s | 3.9 GB | 371 MB (26 GB at 70M) | 0.79, 25 ms |
| turbovec 2-bit, flat scan | 10 s | 3.5 GB | 187 MB (13 GB at 70M) | 0.40, 12 ms |
| turbovec + exact re-rank from the float file | | | | 4-bit, 100 candidates: 1.00, 69 ms; 2-bit, 300: 0.98, 48 ms |
| zvec IVF-RaBitQ 4-bit, 1024 lists | 432 s | 14.1 GB | 405 MB + its own fp32 copy | all lists + refiner: 0.997, 98 ms; 10% filter: 1.0, 87 ms |
| Lance IVF_RQ 4-bit, 1024 partitions | 113 s | 4.4 GB | 391 MB + the data column | all + re-rank: 0.99, 52 ms; 10% filter, 64 partitions: 0.145 |
| zvec DiskANN, degree 64 | stopped after 54 CPU-minutes | 9.4 GB | | |

At scale, from others:

- **TurboVec, 101M × 768-d** (MS MARCO, 32 vCPUs, r/vectordatabase 2026-06): 4-bit 37.4 GB, recall@10 0.914, p50
  **5.67 s**; 2-bit 18.9 GB, 0.608, 3.19 s. 54 ms at 1M, 562 ms at 10M: a flat scan grows linearly.
- **VectorChord**: 100M × 768-d indexed in 18 minutes at recall 0.949, using 12 GB on a 128 GB machine with clustering
  reduced to 100 dimensions; served on 32 GB at 16 QPS, recall 0.95, single-threaded (their blog).
- **PipeANN**: 80M × 768-d in 10 GB at recall 0.99 (README; Linux only, io_uring).
- **AlayaLite LASER**: 100M in 22.5 GiB resident (AGPL-3.0, static build).
- **SatoriDB**: 1B vectors, 95% recall, larger than RAM (Linux only, io_uring); its design (HNSW over cluster centres
  in RAM, ~2,000-vector buckets on disk that split, after SPFresh) is the one to copy if we build our own.

### Engines

| Engine | In-process Rust on Windows | Index for 70M | Inserts, deletes | Filters | Notes |
|---|---|---|---|---|---|
| **Lance / LanceDB** 14 / 0.41 | Yes (crates; Windows CI) | IVF_RQ (1-bit: d/8 + 20 B, ~8 GB), IVF_PQ (~7 GB), on disk, 6 GiB LRU cache | New rows scanned exactly until `optimize_indices()`, no retraining; deletion bitmaps, compaction | Prefilter with scalar indexes; recall drops with selective filters in IVF | Vectors must also live in a Lance column; MVCC multi-process |
| **zvec** 0.8.0 | Yes (`zvec-rust`, FFI, MSVC prebuilt) | IVF-RaBitQ, HNSW-RaBitQ, DiskANN (PQ in RAM, 1 B per chunk) | Insert, upsert, delete, WAL | SQL-like | Its own fp32 copy; heavy builds; docs say Linux-only where the release notes and our test say Windows |
| **Qdrant Edge** 0.9 (beta) | Yes (crate, wheel) | HNSW with binary/scalar/PQ/TurboQuant quantization, memory tiers | Brute force until `optimize()` | Payload indexes, all field types | Not tested; HNSW build at 70M is the open cost |
| **turbovec** 1.1.2 | Yes (native crate) | Flat 2/4-bit TurboQuant | Add without training, µs removes | Id allowlist or bitmask | Whole index in RAM, cost linear in N |
| **DiskANN3** 0.61 (Microsoft) | Yes (crates; Windows CI, IOCP) | Vamana on disk, PQ in RAM | Disk provider is build-and-search only; updates in the memory providers | Label filters | Implement `DataProvider` over our files; needs a cargo build |
| hannoy / arroy | LMDB; arroy has Windows CI | HNSW / random projections | Yes | Bitmaps | Full vectors; no 70M figures |
| RaBitQ-Library | C++/Python, Windows wheels, no Rust | IVF-RaBitQ, HNSW-RaBitQ, SymphonyQG | IVF add/remove | | Its authors' comparison: RaBitQ "consistently achieves higher recall" than TurboQuant |
| sqlite-vector, sqlite-vec, vectorlite | SQLite extensions | Flat scans; vectorlite HNSW in RAM | | SQL | Too slow or too big at 70M |
| Not usable here | | | | | VectorChord (Postgres only, no Windows package, AGPL/ELv2), PipeANN, vsag, SatoriDB (Linux only), AlayaLite (AGPL, static), DuckDB `vss`/USearch/hnswlib/Chroma (HNSW in RAM), rust-diskann (~215 GB file) |

Order to benchmark on the real embeddings once they exist: the flat 2-bit scan with re-ranking (13 GB, ~1 s, recall
~0.98, filters free, nothing to build), Lance IVF_RQ (lean build, prefilter), zvec IVF-RaBitQ, then Qdrant Edge and
DiskANN3 if the first two miss. Per engine: recall@10 against exact, latency, RAM, build time, cost per 10,000 new
images, and recall under a folder filter.

## 7. The duplicate graph

A table of group ids (and the pair decisions already in `pdq_pairs`) in the catalog covers the groups; recursive CTEs
cover walks. graphqlite (SQLite extension, Cypher, connected components) could compute groups beside the catalog if
needed. Graph databases (Grafeo, HelixDB, TriviumDB) keep their main store in RAM or allow one writer.

## 8. What to measure

A synthetic catalog with 70M rows, the same schema (~57 GB, `D:\PhotoGallery\scratch_scale\`), with our real PRAGMAs:

- Import batches with every index in place at 50M+ rows; then sorted batches, a larger cache, and indexes built after.
- `order_key` stored once or shortened (§4): size and grid speed.
- Folder tree and facets: today's GROUP BY, trigger-kept counters, and the in-RAM facet columns.
- The grid paged by `order_key`: first page, jump to row N, scrolling. `pending()`. Opening and the first frame.
- The app writing ratings while `gallery` imports.

## Not verified

Turso's two-process mode under load, its crash recovery and speed at 70M; GlueSQL and KiteSQL hands-on; redb's
benchmark dataset size; seekdb's IVF_PQ on disk; Qdrant Edge, DiskANN3 and AlayaLite hands-on; multi-process access
for turbovec and Qdrant Edge; Lance on Windows rename semantics; Lightroom's own limits (Adobe's pages refused the
fetch); index sizes of trigram FTS5 on file names.
