# Urocissa comparison: what we can learn for a high-performance gallery

Date: 2026-11-03. Status: findings only; no code changed.

Urocissa (`github.com/hsa00000/urocissa`, cloned at HEAD `102025d`) is a self-hosted gallery
built with Rust (Rocket, redb) and Vue. Its stated goal is to serve a million photos on a
4 GB server with smooth scrubbable scrolling, instant search and instant selection. Ours is
a Rust core with a Qt 6 Quick desktop UI. The goals overlap; the platforms do not.

This report compares the two on performance. Urocissa's numbers come from its own README and
docs and have not been re-measured here. Our numbers come from the measurement docs (index: `docs/FINDINGS.md`) and the code.
Our paths are relative to the repo root. Urocissa paths are relative to its `gallery-backend/src/`
(Rust) or `gallery-frontend/src/` (Vue), or to its `docs/` folder. Line references were checked
against the working tree on 2026-11-03 unless marked "not re-checked"; a "not re-checked"
reference was read from Urocissa's source but not verified against a checkout.

## Summary

- Urocissa's most transferable idea is caching each filtered view against a version number,
  so repeat queries and facet counts are lookups rather than scans. We already have the
  version counter (`meta.seq`) but do not use it for this.
- Our facet and grid queries are the parts of our design that scale worst. `docs/DATABASE.md`
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
  holds the ordinal list and bitmap, and is cached by expression, a global version counter, and
  a structural epoch (`get_prefetch.rs:524-556`, `:302-312`). The client then requests fixed
  20-record rows of display sizes (`read_rows.rs`).
- **Writes.** Edits are applied in memory and flushed to redb in chunks of 8,192 records
  (`write_behind.rs:21`). The flush interval defaults to 1 s, so a crash can lose up to about a
  second of acknowledged edits.
- **Browser scrolling.** Their virtual list works around the browser's 33,554,400 px DOM height
  limit with a physical buffer and a projection controller (`useHandleScroll.ts`,
  `docs/HYBRID_VIRTUAL_SCROLL_TESTING.md`).

## Where they are ahead (ranked by expected gain)

### 1. Cache filtered results and facets against a version

- **Ours:** `facets()` runs one live `GROUP BY` pass over the scope on every request, with no cache
  (`crates/catalog/src/browse/read/facets.rs:29-64`). `docs/DATABASE.md:42` records 0.6–0.9 s
  for a `GROUP BY` over 2M rows.
- **Theirs:** the snapshot cache key includes the expression, `VERSION_COUNT_TIMESTAMP`, and the
  structural epoch (`get_prefetch.rs:524-556`).
- **Proposal:** cache `Facets` keyed by `(scope, query, meta.seq)`. Our `meta.seq` is bumped by
  triggers on every change (`next_seq!` in `crates/catalog/src/lib.rs:59-63`, used by the triggers in
  `crates/catalog/src/schema.rs`), so invalidation is
  already available. A cache hit is a map lookup; a miss is today's cost.
- **Measure:** the existing `catalog::facets.all` benchmark (93,581,775 Ir in `docs/REFACTOR_MEASUREMENTS.md`, Phase 0 baseline),
  and a timed facet request on a 2M-row library.
- **Overlap:** `docs/DATABASE.md:43-44` already proposes keeping counts up to date or holding facet
  columns in RAM (~0.8 GB at 70M). A cache is the smaller first step.

### 2. Compute scrollbar landmarks in the same scan as the filter

- **Ours:** the desktop scrollbar is a linear map over the whole list
  (`crates/desktop/qml/GalleryGrid.qml`, `gridBar`, lines 105–145). It shows no date markers.
- **Theirs:** year and month boundaries are computed during the filter scan and returned with
  the snapshot (`get_prefetch.rs:99-153`; not re-checked).
- **Proposal:** when the list is built, record the first index of each year and month. Expose
  these as a sparse table, as `docs/DATABASE.md:46` already proposes for the scrollbar.
- **Cost:** small. The data is already read; the change is UI.

### 3. Keep the filtered key list, and fetch rows only for what is visible

- **Ours:** `list_grid` reads every row in the scope into a `Vec` on each request
  (`crates/catalog/src/browse/read/grid.rs:106-148`). `docs/DATABASE.md:45` records 2.2 s for
  189k rows. The desktop model already fetches details lazily (`crates/desktop/src/gallery.rs`).
- **Theirs:** the snapshot is ordinals plus boundaries; rows are fetched by range (not re-checked).
- **Proposal:** cache the filtered `order_key`/id list per query, as item 1 does for facets, and
  query the 16 columns only for the visible range. Keyset paging on `order_key` is the fallback
  (`docs/DATABASE.md:46`).
- **Cost:** moderate. Do it after item 1 and measure.

### 4. Put a memory budget in the benchmarks

- **Theirs:** the storage harness fails if peak RSS exceeds 850 MiB at 1M records, and compares
  against noise floors of 10 ms (browser) and 1 ms (server) (`storage_harness.rs:29-32`;
  `performance/compare.mjs:22-27`; not re-checked).
- **Ours:** `crates/bench` counts instructions with gungraun/callgrind. That is deterministic and
  good for regression. It does not measure memory at library scale.
- **Proposal:** add a synthetic 1M-row run that records peak RSS and fails above a budget. Take
  our own baseline first; do not adopt their 502.6 MiB figure, which is unverified.

### 5. In-memory bitmap filtering (largest change; last)

- **Theirs:** sparse or dense ordinal sets chosen by universe size (`state.rs:842-1171`).
- **Ours:** the desktop holds 56-byte `Entry` values (`crates/model/src/grid.rs:41`, asserted).
  Filters run in SQL. Tag, make and model filters would need in-RAM columns; `docs/DATABASE.md:44`
  estimates about 11 bytes per file for facet columns.
- **Recommendation:** only if items 1 and 3 do not reach the target. It is the largest change
  and the one with the most memory risk.

## Where we are ahead

- **Import is bounded.** Our pipeline caps readers at 8 and sets a byte budget of 12 MiB for each
  slot (`crates/jobs/src/pipeline.rs:39-66`). Urocissa spawns one task per matched file with no
  cap (`tasks/actor/folder_import.rs:285-330`; not re-checked), and reads whole files into memory before
  decoding (`operations/indexation/generate_dynamic_image.rs`; not re-checked).
- **Decode is scaled.** JPEGs are DCT-scaled to at least 512×320 before analysis
  (`crates/analysis/src/analyze.rs:32-34`, scaling in `crates/codec/src/decode.rs`). Urocissa decodes at full
  resolution.
- **Near-duplicate detection.** We run PDQ with 16-chunk multi-index lookup at radius ≤ 31
  (`crates/dedup/src/pdq_index.rs`), with labelled recall measured in `docs/QUALITY.md`. Urocissa
  does exact hashing only. Its perceptual hash is computed but not used.
- **Layout runs in Rust.** Our justified rows are computed in `crates/client/src/layout.rs`.
  Urocissa does Knuth–Plass wrapping in the client and updates every loaded row's offset each
  time a row's height arrives (`fromDataWorker.ts:96-116`; not re-checked).
- **Durability.** Our SQLite WAL commits each change before the UI acknowledges it. Urocissa
  acknowledges from RAM and can lose about a second of edits on a crash.
- **Measurement.** We have instruction counts, dhat heap profiles, SDE and Pin call graphs, and
  stage timings with recorded results (`docs/PROFILING.md`, `docs/HEAP_PROFILE.md`,
  `docs/START_PROFILE.md`). Urocissa's performance gates have no committed results
  (not re-checked).

## Trade-off, not a clear win

- **Thumbnail size.** Urocissa keeps one 1920 px JPEG per image. We keep thumbnails at the
  display height with nearest-height lookup and an LRU cap (`crates/thumbcache/src/lib.rs`).
  Theirs costs one generation up front; ours regenerates when the grid height changes, and uses
  less disk at display size. We should not change ours without measuring how often the height
  changes in practice.
- **Write path.** Their write-behind buffer trades durability for throughput. We do not need it
  unless bulk rating is measured as slow. A 100k-item rating test would settle it.

## Not transferable

- **Hybrid virtual scroll.** It exists to work around the browser's DOM height limit. A Qt
  `ListView` has no such limit.
- **Image worker pool, Cache Storage, and Cache-Control headers.** Web-only.
- **Versioned image keys.** We already have them: `image://thumb/<file>/<height>` keys include
  mtime and rotation (`crates/desktop/src/providers.rs`).

## Their weaknesses to avoid

- The README's "under a second" claim for a million records has no committed evidence.
- The image worker pool is sized `<=` rather than `<`, creating one worker more than intended
  (`src/store/workerStore.ts:42`; not re-checked).
- Row offsets are updated in O(loaded rows) on each arrival (`fromDataWorker.ts:96-116`).

## Open questions

- How often does the grid height change in real use? This decides the thumbnail trade-off.
- How slow is facet computation at 2M rows on current hardware? `docs/DATABASE.md` was measured
  earlier; a fresh measurement is needed before we commit to item 1.
- Is bulk rating slow at 100k items? This decides whether any write-behind is needed.
