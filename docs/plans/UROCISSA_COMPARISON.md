# Urocissa comparison: what we can learn for a high-performance gallery

Date: 2026-11-03. Status: findings only; no code changed. Claims re-checked 2026-11-14.

Urocissa (`github.com/hsa00000/urocissa`, cloned at HEAD `102025d`) is a self-hosted gallery
built with Rust (Rocket, redb) and Vue. Its stated goal is to serve a million photos on a
4 GB server with smooth scrubbable scrolling, instant search and instant selection. Ours is
a Rust core with a Qt 6 Quick desktop UI. The goals overlap; the platforms do not.

This report compares the two on performance. Urocissa's numbers come from its own README and
docs and have not been re-measured here. Our numbers come from the measurement docs and the code.
Our paths are relative to the repo root. Urocissa paths are relative to its `gallery-backend/src/`
(Rust) or `gallery-frontend/src/` (Vue), or to its `docs/` folder. Our line references were re-checked on 2026-11-14, and the drifted ones are corrected. Urocissa's were checked
against a clone at `102025d` on the same day. Where a claim was wrong, the text says so.

## Summary

- Urocissa's most transferable idea is caching each filtered view against a version number,
  so repeat queries and facet counts are lookups rather than scans. We already have the
  version counter (`meta.seq`) but do not use it for this.
- Our facet and grid queries are the parts of our design that scale worst. `docs/plans/DATABASE.md`
  already says so. Urocissa's approach supports the fixes we have planned.
- We are ahead on import (bounded, DCT-scaled decode, PDQ near-duplicate detection), on
  layout (justified rows computed in Rust), and on measurement (deterministic instruction
  counts, dhat, SDE).
- Several of their techniques exist only because they run in a browser. We should not copy them.

## Their design, briefly

- **Data model.** Every record is held in RAM in a compact arena, addressed by generation-tagged
  `SlotRef`s (`gallery-backend/src/public/db/tree/state.rs:38-169`). Filters are bitmaps
  (`OrdinalSet`, sparse or dense) combined per expression (`state.rs:842-1171`, `:2985-3110`).
  Records are bitcode-encoded in redb for durability (`storage/v6.rs`). README:87 reports 502.6 MiB
  peak RSS for 1M metadata-only records; the other memory figures are extrapolations.
- **Browse.** A query returns a snapshot ID, a row count, and scrollbar landmarks. The snapshot
  holds the ordinal list and bitmap, and is keyed by expression, the global version counter, the locate target and the sort
  (`get_prefetch.rs:524-556`), and checked against a structural epoch when it is read (`:302-312`). Random
  sort is not cached. The client then requests fixed
  20-record rows of display sizes (`read_rows.rs`).
- **Writes.** Edits are applied in memory and flushed to redb in chunks of 8,192 records
  (`write_behind.rs:21`). The flush interval defaults to 1 s, so a crash can lose up to about a
  second of acknowledged edits.
- **Browser scrolling.** Their virtual list works around the browser's DOM height limit with a physical buffer (its README gives 33,554,400 px, which is
  Firefox's value; Chrome's is 33,554,428 px, the number their own test uses (`gallery-frontend/src/script/hook/useHandleScroll.test.ts:286`).
  Both come from a 2016 Stack Overflow test quoted in TanStack/virtual#616. The limit is per browser, so the figure is not a spec; checked) and a projection controller (`useHandleScroll.ts`,
  `docs/HYBRID_VIRTUAL_SCROLL_TESTING.md`).

## Where they are ahead (ranked by expected gain)

### 1. Cache filtered results and facets against a version

- **Ours:** `facets()` runs one live `GROUP BY` pass over the scope on every request, with no cache
  (`crates/catalog/src/browse/read/facets.rs:39`). `docs/plans/DATABASE.md:45` gives 0.6–0.9 s
  for a `GROUP BY` over 2M rows, but that is a standalone SQLite test, not `facets()`. The real
  cost is unmeasured (see Open questions).
- **Theirs:** the snapshot cache key includes the expression, `VERSION_COUNT_TIMESTAMP`, the locate target and the
  sort (`get_prefetch.rs:524-556`). The structural epoch is compared on read (`:302-312`), not hashed into the key.
- **Proposal:** cache `Facets` keyed by `(scope, query, meta.seq)`. Our `meta.seq` is bumped by
  triggers on every change (`next_seq!` in `crates/catalog/src/sql.rs:17`, used by the triggers in
  `crates/catalog/src/schema.rs`), so invalidation is
  already available. A cache hit is a map lookup; a miss is today's cost.
- **Measure:** the existing `catalog::facets.all` benchmark (93,581,775 Ir in `docs/temp/REFACTOR_MEASUREMENTS.md`, Phase 0 baseline),
  and a timed facet request on a 2M-row library.
- **Overlap:** `docs/plans/DATABASE.md:45-47` already proposes keeping counts up to date or holding facet
  columns in RAM (~0.8 GB at 70M). A cache is the smaller first step.

### 2. Compute scrollbar landmarks in the same scan as the filter

- **Ours:** the desktop scrollbar is a linear map over the whole list
  (`crates/desktop/qml/GalleryGrid.qml`, `gridBar`, lines 105–145). It shows no date markers.
- **Theirs:** year and month boundaries are computed during the filter scan and returned with
  the snapshot (`get_prefetch.rs:99-153`; checked: the boundaries are recorded during the scan's `push`).
- **Proposal:** when the list is built, record the first index of each year and month. Expose
  these as a sparse table, as `docs/plans/DATABASE.md:48` already proposes for the scrollbar.
- **Cost:** small. The data is already read; the change is UI.

### 3. Keep the filtered key list, and fetch rows only for what is visible

- **Ours:** `list_grid` reads every row in the scope into a `Vec` on each request
  (`crates/catalog/src/browse/read/grid.rs:115`). `docs/plans/DATABASE.md:48` records 2.2 s for
  189k rows. The desktop model already fetches details lazily (`crates/desktop/src/gallery.rs`).
- **Theirs:** the snapshot is ordinals plus boundaries; rows are fetched by index range, 20 per row (`read_rows.rs`; checked).
- **Proposal:** cache the filtered `order_key`/id list per query, as item 1 does for facets, and
  query the 16 columns only for the visible range. Keyset paging on `order_key` is the fallback
  (`docs/plans/DATABASE.md:48`).
- **Cost:** moderate. Do it after item 1 and measure.

### 4. Put a memory budget in the benchmarks

- **Theirs:** the storage harness fails if peak RSS exceeds 850 MiB at 1M records, and compares
  against noise floors of 10 ms (browser) and 1 ms (server) (`storage_harness.rs:32`, which fails the run above the limit;
  `performance/compare.mjs:22-27`; checked. The 1 ms floor applies to every key that is not `browser.`).
- **Ours:** `crates/bench` counts instructions with gungraun/callgrind. That is deterministic and
  good for regression. It does not measure memory at library scale.
- **Proposal:** add a synthetic 1M-row run that records peak RSS and fails above a budget. Take
  our own baseline first; do not adopt their 502.6 MiB figure. It is stated in their README (line 87), but we have not reproduced it.

### 5. In-memory bitmap filtering (largest change; last)

- **Theirs:** sparse or dense ordinal sets chosen by universe size (`state.rs:842-1171`).
- **Ours:** the desktop holds 56-byte `Entry` values (`crates/model/src/grid.rs:41`, asserted).
  Filters run in SQL. Tag, make and model filters would need in-RAM columns; `docs/plans/DATABASE.md:47`
  estimates about 11 bytes per file for facet columns.
- **Recommendation:** only if items 1 and 3 do not reach the target. It is the largest change
  and the one with the most memory risk.

## Where we are ahead

- **Import memory is budgeted.** Our pipeline caps readers at 8 and sets a byte budget of 12 MiB for each
  slot (`crates/jobs/src/pipeline.rs:43-70`). Urocissa queues one task per matched file with no cap on the
  queue (`gallery-backend/src/tasks/actor/folder_import.rs:285-330`; checked), and each task reads the whole
  file into memory before decoding (`operations/indexation/generate_dynamic_image.rs:6-10`; checked). Its
  concurrent reads are bounded by the index runtime's worker threads (`gallery-backend/src/public/constant/runtime.rs:30-37`; `execute_detached` spawns onto it,
  `mini_executor` 2.0.3 `src/lib.rs:391-392`, crate checksum matches `Cargo.lock`), so the exposure is the queue,
  not unbounded reads.
- **Decode is scaled.** JPEGs are DCT-scaled to at least 512×320 before analysis
  (`crates/analysis/src/analyze.rs:30-32`, scaling in `crates/codec/src/decode.rs`). Urocissa decodes at full
  resolution.
- **Near-duplicate detection.** We run PDQ with 16-chunk multi-index lookup at radius ≤ 31
  (`crates/dedup/src/pdq_index.rs`), with labelled recall measured in `docs/temp/VERIFIER.md`. Urocissa
  does exact hashing only. Its perceptual hash is computed but not used.
- **Layout runs in Rust.** Our justified rows are computed in `crates/client/src/layout.rs`.
  Urocissa runs Knuth–Plass in a web worker (`gallery-frontend/src/worker/toDataWorker.ts:324`) and updates
  every loaded row's offset each time a row's height arrives (`fromDataWorker.ts:96-116`; checked).
- **Durability.** Our SQLite WAL commits each change before the UI acknowledges it, so an application
  crash loses none of it. The setting is `synchronous = NORMAL` (`crates/catalog/src/schema.rs:377`): after a
  power loss or OS crash, the last commits can roll back (sqlite.org/wal.html §2.3, quoted: transactions "might rollback following a power failure or hard
  reset"; the application-crash point is on sqlite.org/pragma.html, extracted, not quoted verbatim). So the window is
  different, not absent. Urocissa acknowledges from RAM and can lose about a second of edits on any crash.
- **Measurement.** We have instruction counts, dhat heap profiles, SDE and Pin call graphs, and
  stage timings with recorded results (`docs/temp/PROFILING.md`, `docs/reference/HEAP_PROFILE.md`,
  `docs/temp/START_PROFILE.md`). Urocissa's performance gates have no committed results
  (checked: the clone tracks no performance result files).

## Trade-off, not a clear win

- **Thumbnail size.** Urocissa keeps one 1920 px JPEG per image. We keep thumbnails at the
  display height with nearest-height lookup and an LRU cap (`crates/thumbcache/src/lib.rs`).
  Theirs costs one generation up front; ours regenerates every thumbnail when the grid height changes (about 190k images, 6–8 minutes;
  `docs/temp/THUMBNAIL_RESAMPLING.md` §1), and uses
  less disk at display size. We should not change ours without measuring how often the height
  changes in practice.
- **Write path.** Their write-behind buffer trades durability for throughput. We do not need it
  unless a bulk rating path is added and measured as slow. None exists yet (see Open questions).

## Not transferable

- **Hybrid virtual scroll.** It exists to work around the browser's DOM height limit. A Qt
  `ListView` has no such limit.
- **Image worker pool, Cache Storage, and Cache-Control headers.** Web-only.
- **Versioned image keys.** We already have them: `image://thumb/<key>/<height>` URLs, where the key is `<file id>@<mtime>.<rotation>.<turns>`, include
  mtime and rotation (`crates/desktop/src/providers.rs`).

## Their weaknesses to avoid

- The README's "under a second" claim for a million records has no committed evidence (checked: the clone
  tracks no performance result files).
- The image worker pool is sized `<=` rather than `<`, creating one worker more than intended
  (`gallery-frontend/src/store/workerStore.ts:42`; checked: the loop runs `concurrencyNumber + 1` times).
- Row offsets are updated in O(loaded rows) on each arrival (`fromDataWorker.ts:96-116`).

## Open questions

Both still need a measurement or an instrumented count that this repo does not contain.

- **How often does the grid height change in real use?** Not answerable from the repo: nothing records it.
  Each change regenerates the cache at the new height (about 190k thumbnails, 6–8 minutes;
  `docs/temp/THUMBNAIL_RESAMPLING.md` §1), and it enters through `set_thumb_height`
  (`crates/app/src/library.rs:252`). A counter there, over real sessions, would answer it. Until then the
  thumbnail trade-off stays open.
- **How slow is facet computation at 2M rows?** Not measured. The 0.6–0.9 s in `docs/plans/DATABASE.md:45`
  and `:192` comes from a standalone SQLite 3.49 test on a generic table with random keys. It does not run
  `facets()` (`crates/catalog/src/browse/read/facets.rs:39`), which has the file joins, one `CASE` per facet
  and the scope filter; the app ships SQLite 3.54. A real figure needs `facets()` on a catalog of at least
  2M files. The largest layout in the repo is 200,000 files (`tools/profiling/bench_data.py -large`), so a
  larger catalog has to be generated first.

## Settled on 2026-11-14

- **Is bulk rating slow at 100k items?** The premise is wrong: there is no bulk rating path. A rating is one
  file: `crates/desktop/src/shell.rs:252` → `GalleryModel::set_rating` (`crates/desktop/src/gallery/edits.rs:75`)
  → `Catalog::set_rating` (`crates/catalog/src/queue.rs:37`), one `IMMEDIATE` transaction per call, on the GUI
  thread. What remains is a narrower point: how long each rating blocks the GUI, and how long it waits behind the
  background writer (`busy_timeout` 5 s, `crates/catalog/src/schema.rs:377`). A Python proxy (WAL,
  `synchronous = NORMAL`, one update trigger and one queue insert, 100k serial single-row transactions; not the
  app's schema) took 552 µs per rating, 55 s in all. It is indicative only: the Rust path and the app's real
  triggers are not measured. Nothing here justifies write-behind.
