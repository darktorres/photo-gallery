# Plan: shrink the catalog (519 MB -> about 200 MiB on this library)

## Context

The user's catalog (`D:\PhotoGallery\catalog\catalog.sqlite`, 519 MB, 715,051 files, 334 folders, 1 volume, schema v20,
`path_rules = windows`) was analysed with `dbstat` (file 495.1 MiB; 155 pages are pointer-map pages because
`auto_vacuum = incremental`; `freelist_count` 0):

| Object | MiB | Note |
|---|---|---|
| `files` table | 186.8 | 237 B/row; `order_key` is 117 B of it |
| `files_order` index | 104.1 | `order_key` again |
| `analysis` table | 56.6 | 56 B/row, 26 columns; 14 MiB unused |
| `files_folder` (folder_id, name) | 43.6 | duplicates the name |
| `UNIQUE(volume, file_ref)` | 25.2 | |
| `files_ref` | 16.2 | |
| `analysis_borders_due` | 10.6 | transient (712,576 rows due) |
| `files_changed` / `files_seq` / `files_present` | 9.3 / 9.3 / 8.4 | |
| `analysis_file_hash` + `analysis_pixel_hash` | 8.0 + 8.0 | index only NULLs today |
| `hashes_owed` | 7.1 | transient queue |
| unused space inside pages | about 75 | |

Today `order_key` = folder key (lowercase folder path, 32.6 B, repeated per file) + `0x00` + lower(name) + `0x00` +
exact name.

**Constraints from the user:** churn and compatibility are not a concern; pick the most proper design. The software will
also run on Linux and maybe macOS, so name-equivalence rules follow `PathRules` (`crates/model/src/rules.rs`), never a
hardcode. Names are canonical WTF-8 bytes (`crates/model/src/name.rs:3-4`): an unpaired Windows surrogate or a Linux
stray byte is kept as bytes, and two names are equal exactly when their bytes are.

**Coming growth, not reducible here:** the hashes pass fills `file_hash` (32 B), `pixel_hash` (16 B) and `pdq_dihedral`
(128 B) per image. Simulated: `analysis` goes 43 -> 180 MiB, plus about 45 MiB of hash indexes and 30 MiB of
`placeholders`, minus the 18 MiB of transient queues.

## Design decisions

### D1. The name key: identity, equivalence and order are three things

- **Identity** is exact bytes, always (pairing, `replaced_by`, `unchanged`, `file_by_path`'s final check).
- **Equivalence** (when two names are the same name) is the OS's, so it is `PathRules`'s: Windows = the NTFS upcase
  comparison; Posix = exact bytes (`IMG.jpg` and `img.jpg` are two files); macOS and ext4 casefold become new rules later.
- **Order** is the catalog's canonical name order: the byte order of the equivalence key, then the id. Locale-aware,
  natural-number order (ICU4X) is a presentation feature for the view layer, not part of storage and not in this plan.

**Mechanism: an expression index on a registered deterministic function.** `files.name` stays `BLOB` (exact bytes). A
function `path_name_key(name BLOB) -> BLOB`, built per `PathRules`, is registered on every catalog connection next to
`pg_query::register_functions` and `rusqlite::vtab::array::load_module` (`crates/catalog/src/core.rs:263-266`, with the
connection's rules passed in), with the deterministic flag. The index is `files_mn(mid, missing, path_name_key(name))`.
The key exists only in the index; the table keeps the name once.
- **Windows key:** each UTF-16 unit mapped through an **embedded, versioned upcase table** generated from the Unicode
  data (BMP only, like `$UpCase`; supplementary letters are left as they are), then encoded as WTF-8. The table is not the
  running OS's `RtlUpcaseUnicodeChar` (`BOUNDED_MEMORY_PLAN.md:313`, which varies by Windows build and cannot run in the
  Linux CI) and not the volume's `$UpCase` (not openable unprivileged, `BOUNDED_MEMORY_PLAN.md:1658`). The embedded table is
  deterministic across machines and testable on Linux. Its hash is the version stamp.
- **Order consequence:** the key's byte order is code-point order, which differs from NTFS's UTF-16-unit order only for
  supplementary letters against U+E000..U+FFFF. The catalog's order has to be deterministic and consistent with the
  equivalence, not equal to NTFS's. Visible difference from today: upcase order puts `[ \ ] ^ _` after the letters, lowercase
  order puts them before.
- **Posix key:** the identity function (exact bytes). The index then holds the name bytes once and the 65 MiB that a stored
  key would add on Posix rules never appears.
- **Folder keys use the same mapping.** `mfolders.key` is the folder key as `push_folder_key` builds it (`rules.rs`, drive-free,
  merges drives), with each part mapped through the rules' name key instead of lowercase. Otherwise `ς`/`σ` would be one
  name on NTFS (both upcase to `Σ`) and two folders. The keys stay stored byte keys (334 rows), so `prefix_upper_bound`
  subtree ranges work unchanged.
- **Version stamp:** `meta.name_key_version` (the table's hash, plus a key-format number). On open, when it differs from
  this build's, rewrite `mfolders.key` from `folders.path` and `REINDEX files_mn`, before any other use
  (`lifecycle::set_pairing_version` is the pattern). SQLite treats an index built under a different function or collation as
  corrupt (ICU README, section 3.3).
- **Connections without the function:** measured on SQLite 3.54.1: they read everything, also through `ATTACH` and the
  views, and a lookup that uses only the `(mid, missing)` prefix works; they cannot write the table, run
  `integrity_check` or `REINDEX` (`unknown function: path_name_key()`); `quick_check` passes. So: register it in every
  catalog connection (`Catalog` and `Reader`), in the thumbnail cache's connection (`crates/thumbcache/src/lib.rs:69,284`,
  which attaches the catalog and only reads), in the Python tools that write `files`
  (`tools/profiling/bench_data.py`, `tools/profiling/pairing/grow_catalog.py`), and add a test that opens the catalog
  without the function and checks that every statement the cache runs plans without a write to the index.
- **Equality in SQL:** the column is BINARY, so `name = ?` is exact bytes. The pairing name lookup is
  `INDEXED BY files_mn ... WHERE mid = ? AND missing = ? AND path_name_key(name) = ? AND name = ? AND id <> ?`: the index
  finds the equivalence class (case variants on Windows), the residual `name = ?` makes it exact. `file_by_path`
  (`lookup.rs:120-142`, documented case-insensitive on Windows but case-exact today) becomes a probe on the key plus the
  rules' folder check.

**Rejected alternatives (each with its reason):**
- `NOCASE`/`lower()`: ASCII only (sqlite.org/datatype3.html, section 7). ICU extension: locale order (CLDR) rather than the
  filesystem's rule, an optional build, and an OS-provided ICU that changes silently invalidates the index.
  sqlean `text_nocase`: simple folding over the BMP only, an extension DLL on every connection, no WTF-8.
- A registered collation with `name` as `TEXT`: names with unpaired surrogates or stray bytes cannot be TEXT, and the fork's
  collation callback converts through `String::from_utf8_lossy` (`rusqlite .../src/collation.rs:79-83`), which would merge
  distinct names. A mixed TEXT/BLOB column was considered and not needed.
- A stored key column (with the sparse NULL trick): with upcase keys most names differ from their key, so it costs +31.8 MiB
  for the name part (measured: 110.3 vs 78.5 MiB) and doubles on Posix. It is the fallback if the gate G1 fails.

### D2. Drop the `files_changed` index (9.3 MiB; column and triggers stay)

No production SQL reads through it: `roots.rs:283` reads `changed` through `UNIQUE(volume, file_ref)`; the one range read is
a test helper, `test_support.rs:171` (`WHERE changed > ?1 ORDER BY id`), which becomes a scan. **DECIDED by the owner
(2026-11-19): drop it now and recreate it later in the web client plan if needed.** This reverses
`docs/plans/SCAN_OPTIMIZATION_PLAN.md:1015` (D1: "keep it", decided 2026-11-15), whose reason was a planned resync reader.
That reader is concrete: `docs/temp/WEB_CLIENT_DESIGN.md:463` (the Delta request) and `:1511` (the changes cursor) read
"rows with `changed` > S via the `files_changed` index". Measured on the real catalog, that query takes 0.07-0.12 ms with the
index and about 310 ms as a full table scan, so **the web client's migration must add
`CREATE INDEX files_changed ON files(changed)`** (about 0.5 s to build on 715k rows, 8-9 MiB). Record that in the web client
plan and update both documents' wording (listed under Verification, Docs).

### D3. The hash indexes become partial, not dropped

`WHERE col IS NOT NULL`. EXPLAIN-verified on SQLite 3.54.1: `file_hash = ?` (the trigger probes in
`DECODE_ONCE_PLAN.md:726-769`) and `file_hash > ?` use the partial index with no `IS NOT NULL` term. Dropping them
outright is a measured follow-up after the hashes pass.

### D4. `analysis` keeps its own `size`/`mtime`/`rotation`/`kind`

A single `source_seq` would save 13.5 MiB (43.0 -> 29.5, measured) plus about 8 MiB once `placeholders` fills, but
staleness is deliberately a by-value comparison of the file's stat triple in every store that derives something from a
file: `analysis` (`stale!()` at `sql.rs:8`, four triggers at `schema.rs:1034-1086`, `writers.rs:332`, `lifecycle.rs:77`),
`placeholders`, `file_failures` (`thumbcache/src/reader.rs:216`), and the thumbnail cache's own file (`thumbs.sqlite`,
`thumbcache/src/schema.rs:22`). A revision counter would couple that file to the catalog's counter (`files.seq` also moves for
unrelated reasons: `lifecycle.rs:80`, `sql.rs:80-87`), so the cache could no longer survive a catalog rebuild on its own
evidence, and moving only `analysis` would leave two staleness schemes.

### D5. Release grouping and the coordinated schema change

- **Phase 1 ships alone as v21**: it touches no `files` column.
- **Phase 2 is folded into the coordinated schema change** that `docs/plans/BOUNDED_MEMORY_PLAN.md` (Phase 2: one BASELINE
  edit, then a freeze) and `docs/plans/NTFS_MFT_SCAN_PLAN.md:499-533` already schedule, because they rewrite `files` too.
  Those plans use `order_key` only as folder-key ranges (root scope and removal masks, D9 and 3.4), the unpaired indexes
  `(unpaired_anchor, order_key)` (lines 409-413) and `file_by_path` as an `order_key` probe (line 879). With `mfolders` and the
  expression index these map one to one: a range on `mfolders.key`; `(unpaired_anchor, mid, path_name_key(name))`; and
  `mid = ? AND path_name_key(name) = ?` plus the exact check. Their statements are rewritten in those terms, and one
  migration and one backup (`pre-vN.bak`, named by the version found, `crates/README.md:308-312` says `pre-v18`) cover both.
- Compaction after the migration is automatic (D6).

### D6. Compaction

The migration sets `meta.vacuum_owed`; `lifecycle::vacuum_if_owed` (`lifecycle.rs:153-181`) runs
`wal_checkpoint(TRUNCATE); VACUUM` at the next exclusive start (`VACUUM_ROOM = 1`, `MIGRATION_ROOM = 2`,
`lifecycle.rs:114,116`; `D:` has 126 GB free). After the borders and hashes passes free their queues (10.6 + 7 MiB) and
`analysis` grows (about 15 MiB unused), another compaction is wanted: add an automatic trigger when free pages pass a
threshold (no periodic trigger exists; `Catalog::shrink`, `removal.rs:264`, only runs `incremental_vacuum` after removals).

## Gates: measure these before building Phase 2

- **G1. The Rust key function's cost.** Benchmark `CREATE INDEX`, 100,000 inserts and 200,000 pinned lookups of the real
  function (embedded table, WTF-8) against the exact-bytes index on the scratch copy of the real catalog. The earlier Python
  timings are unusable (per-statement overhead). Accept B when the overhead stays near the C `NOCASE` collation's measured
  +8% / +18% / +19% (for a function call it will be higher; the bar is no more than 2x the exact-bytes index). Otherwise fall
  back to the stored-key column.
- **G2. NTFS on this machine.** In a temp directory create `σ.txt` and `ς.txt` (does the second collide?), `𐐨.txt` and `𐐀.txt`
  (Deseret, supplementary), `İ.txt` and `i̇.txt`, and compare the answers with the embedded table and with
  `RtlUpcaseUnicodeChar`. This writes a few temporary files; run it at the start of implementation.
- **G3. The full `files` side under B** on the scratch copy (the name part alone measured 78.5 MiB, identical to an
  exact-bytes index): expected about 149 MiB, against 156.7 MiB for the lowercase sparse stored key. Also EXPLAIN the grid
  (ascending and descending), the count and the pairing probe on the real data, with no `TEMP B-TREE`.
- **G4. The ordering differences list**: sort the real names by the old lowercase key and by the new key and list every
  position where they differ; they must all be of the punctuation, supplementary or `ς` kinds.

## Phase 1: migration to v21 (indexes; -32 MiB repacked, -41 MiB on today's pages)

`TO_21_...` in `crates/catalog/src/schema.rs` (`Step::Sql`, no foreign-keys-off; pattern `TO_16_NEW_FILES`):

```sql
DROP INDEX analysis_file_hash;  CREATE INDEX analysis_file_hash  ON analysis(file_hash)  WHERE file_hash IS NOT NULL;
DROP INDEX analysis_pixel_hash; CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash) WHERE pixel_hash IS NOT NULL;
DROP INDEX files_ref;
DROP INDEX files_changed;
```

- **Hash indexes** (-16 MiB now, and no index write per import row): readers `catalog/src/dups.rs:20-58` (`groups!`/`grouped!`
  filter `IS NOT NULL`) and the grid duplicates filter `crates/query/src/lib.rs:336,339`; writers are `UPDATE`s and
  `INSERT OR REPLACE` (`writers.rs:382,391,394,397`, `queue.rs:361`, `ledger.rs:544,788`). Add an EXPLAIN pin for both
  filters and the trigger probes.
- **`files_ref`** (-16.2 MiB): keep `UNIQUE(volume, file_ref)`. Pairing tier 2 (`pair.rs:157-171`) runs
  `WHERE volume IS NOT NULL AND volume <> ?4 AND file_ref = ?5 ...` (`pair.rs:164`); with no `files_ref` it would scan (311 ms for
  2,000 lookups, about 100x slower). Rewrite it to bind the unseen volumes (all volumes minus `seen`, read once with
  `seen_volumes`, `pair.rs:422`; `seen` only grows in a batch, so a stale list is a safe superset, so keep the Rust
  `!seen.contains(volume)` filter) as `volume IN rarray(?)` (`rarray` is registered at `core.rs:265` and used at
  `removal.rs:74-257`); constant SQL (`sql.rs:1-3`); skip the query when the list is empty. On Linux `volume` is a
  filesystem-UUID-derived id (`platform/src/dir.rs:482-560`). Add a plan pin and a 2+ volume pairing test (no benchmark covers
  tier 2 today; add one).
- **`files_changed`** (D2): change the "edited-index" case in `lifecycle.rs:873` to another index; fix
  `crates/catalog/examples/stmt_journal_probe.rs:54-61` (it also names dropped `last_scan`, `files_fresh`) and
  `crates/catalog/examples/pairs_load_probe.rs:51-52` (`INDEXED BY files_present`, `files_to_import`); update the
  `fixture.rs:129` comment and the `test_support.rs:171` helper. The trigger `files_changed` and `TRIGGER_TESTS`
  (`schema.rs:1141`) are unaffected.

## Phase 2: rebuild `files` and `folders`, add `mfolders` (coordinated schema change; about -70 MiB more than the earlier in-place design)

Rebuild (not `ALTER`): a composite foreign key cannot be added in place, and the retyped columns are laid out once. Pattern:
`crates/catalog/src/autoincrement.rs` (`ids_autoincrement`, `rebuild()`, foreign-keys-off via `FOREIGN_KEYS_OFF_FROM`,
`schema.rs:78`, add this version; `PRAGMA foreign_key_check` before and after; save and restore `sqlite_sequence`; keep the
`AUTOINCREMENT` text that `is_autoincrement` keys on).

```sql
CREATE TABLE mfolders(id INTEGER PRIMARY KEY, key BLOB NOT NULL UNIQUE);        -- one row per distinct merged folder key
CREATE TABLE folders(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE,
                     mid INTEGER NOT NULL REFERENCES mfolders(id), UNIQUE(id, mid));
CREATE TABLE files(id INTEGER PRIMARY KEY AUTOINCREMENT, volume INTEGER, file_ref INTEGER, created INTEGER,
  folder_id INTEGER NOT NULL, mid INTEGER NOT NULL, name BLOB NOT NULL,        -- exact bytes
  ext INTEGER NOT NULL, kind INTEGER NOT NULL CHECK (kind IN (0, 1)), size INTEGER NOT NULL, mtime INTEGER NOT NULL,
  missing INTEGER NOT NULL DEFAULT 0, rotation INTEGER NOT NULL DEFAULT 0, pending INTEGER NOT NULL DEFAULT 1,
  seq INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0, unpaired INTEGER NOT NULL DEFAULT 0,
  UNIQUE(volume, file_ref), FOREIGN KEY(folder_id, mid) REFERENCES folders(id, mid));
CREATE INDEX files_mn ON files(mid, missing, path_name_key(name));  -- grid order (both directions) and the pairing name lookup
CREATE INDEX files_fm ON files(folder_id, missing);                 -- replaces files_folder AND files_present
CREATE INDEX files_seq ON files(seq);                               -- files_to_import (partial) is kept as is
```

- **Why each piece:** `mfolders` holds each folder key once (folder rows with equal keys, `D:\x` and `E:\X`, share a `mid`, so
  files interleave by name as today: `browse/read/tests.rs:~498`, `browse/fixtures.rs:62-66`; on this catalog 334 keys, 1:1).
  The composite foreign key makes the database enforce `files.mid = folders.mid`, the denormalisation `files_mn` needs.
  `kind` 0/1 and `ext` as a small id cost 0-1 byte; `volume` is interned. `mfolders` ids are not AUTOINCREMENT (nothing
  persists a `mid`); say so in the migration's doc comment.
- **Integer codes:** `kind` through the `kind_video!()`/`kind_image!()` macros (`model/src/media.rs`), `stale!()`
  (`sql.rs:8-15`), the triggers (`analysis_changed`, `files_changed`, `analysis_removed`) and the `thumb_sources` view that
  thumbcache reads (about 33 SQL references); `ext` as an id from `exts(id, name)` (not a Rust enum order, which would persist);
  `volume` as an id of `volumes(id, serial UNIQUE)`, touching `work.rs` lanes (`22-27,59-75,171`),
  `thumbcache/src/reader.rs:118-127` and the scan. Keep `UNIQUE(volume, file_ref)` (plan pins `work.rs:703,712`).
  `analysis.kind`/`format` become integers too (measured -6.6 MiB for the table), and so do the enum text columns of tables
  that will grow: `border_sides` (`edge`, `kind`, `class`, `rejected`; 0 rows until the borders pass runs, so no size is
  measurable yet). Bounded or tiny tables (`notices`, `file_failures`, `file_writes`, `ledger_ack`) keep text. `pdq_pairs` is
  already `WITHOUT ROWID` (nothing to gain).
- **Indexes removed:** `files_order`, `files_folder`, `files_present`, `files_ref`, `files_changed`, `folders_key`.
  `files_to_import` and `analysis_failed` stay (empty, pinned).
- **Key API** (`rules.rs`, `name.rs`): replace `PathRules::order_key` (`rules.rs:257`) by `PathRules::name_key(name, out)` (the
  per-rules key above; no leading `0x00`) and the function registration; `folder_key` maps each part through it;
  `scope_keys` (`rules.rs:266-277`) returns bounds on the folder key: recursive `[key, prefix_upper_bound(key))` (empty key: no
  upper bound), non-recursive `[key, key ++ 0x00)`. `Name::name_key()` for the saved-view sort (`browse/views.rs:15-21`) and the
  client's own sorting (`client/src/tree.rs:85,106`, `model/src/tree.rs:139,284,345`) use the same key, so SQL and Rust agree
  (test: the SQL order of every real name equals the Rust sort). `folder_key()` stays the single source for SQL and the client
  tree. Replace the per-char lowercase in `Name::push_lower` (`name.rs:131`) only where the rules say; keep it for display.
- **Plan pins (the planner has no statistics):** grid/count/facet SQL (`browse/read/scope.rs:14-17`, `grid.rs:133-140,177-183`,
  `facets.rs:41,62`) read `FROM mfolders m CROSS JOIN files f ON f.mid = m.id WHERE f.missing = 0 AND m.key >= ?lo [AND m.key < ?hi]
  ORDER BY m.key {dir}, path_name_key(f.name) {dir}, f.id {dir}` (`CROSS JOIN` pins `mfolders` outer, the idiom at `pair.rs:~436`;
  EXPLAIN-verified: `SEARCH m ... (key>? AND key<?)`, `SEARCH f USING COVERING INDEX files_mn (mid=? AND missing=?)`, no sorter,
  ascending and descending); `fixtures.rs:148-155` orders the same way. `INDEXED BY files_mn` on the pairing name lookup
  (`pair.rs:238`, both `missing` states as two probes or `missing IN (0,1)` pinned): **unpinned, the planner chose `files_fm` and
  took 215 s for 100,000 lookups; pinned it takes 0.108 s.** `files_fm` for `roots.rs:183,203,311` and
  `removal.rs:74,79,210,257,334`; `INDEXED BY files_present` at `work.rs:174` becomes `files_fm`; comments naming `files_order` and
  `files_present` (`grid.rs:126,129,177`, `scope.rs:9`, `tree.rs:8`); pins at `browse/read/tests.rs:640-680,849-880`.
- **Rebuild path (`rebuild.rs:192-243`):** `copy_tables` copies shared columns by name, then runs the `Rust` steps. The retyped
  columns (`kind`, `ext`, `volume`) exist in both layouts with different types, `mid` is `NOT NULL` with no default, and
  `order_key`/`folders.key` exist only in the old one. So `files`, `folders` and `analysis` must not be copied generically when
  the old shape differs: add a "converted tables" notion to `copy_tables` (or have the `Rust` step read them from `carried.*`
  via `Source::Carried`, which exists for this), build `mfolders` first, and add `"mfolders"`, `"volumes"`, `"exts"` to
  `CARRIED` (`rebuild.rs:138-161`; `carried_is_every_table_of_the_current_schema`, `schema.rs:1118`). `table_columns`
  (`convert.rs:44`) reads `pragma_table_info`; an expression index adds no column to it.
- **Writers and copies:** `writers.rs:146,149-152,166,176,206-221,246-248` (`folder()` insert-or-get `mfolders` in the same
  transaction; positional `get_ref(8)` in `known`), `types.rs:326-378` (`FolderRows::unchanged`: compare the key and `mid`; public
  API called by `scan/src/lib.rs:~1099`; `Folder { id, key }` is pub, update `pg-scan`), `roots.rs:310-329` (`r.get(10)`),
  `pair.rs:73,443-519,670` (`r.get(10)`; `take_identity!` copies `mid`), `lookup.rs:122-137` (`fo.key = ?1` becomes a `mid`
  lookup; `file_by_path` uses the rules' key), `removal.rs:434` (delete empty `mfolders`). Every raw insert and fixture:
  `fixture.rs:98-119`, `fixture/raw.rs:70,102-104`, `fixture/state.rs:22` (`add_folder`, public: also create the `mfolders` row),
  `lifecycle.rs:1292-1301,1506-1518`, `removal.rs:682`, `ledger.rs:1056`, `roots.rs:893`, `pair.rs:669-675`,
  `schema.rs:1280-1461`, `writers.rs:913,934`; `lifecycle.rs:330-348` (`BEFORE_ORDER_KEY`) is a self-contained old fixture.
- **Pinned tests to re-pin:** `model/tests/reference_keys.tsv` (2,034 rows, no rules column: add one; Windows and Posix),
  `rules_reference.rs:55-99`, `path_laws.rs:38-92`, `name.rs` name-key tests (~348-430), `rules.rs` inline tests (439, 450-470,
  525-532), `browse/read/tests.rs:39-41,485-503,542-543,565,574,599-600,663,673`, the ignored
  `real_names_sort_and_scope_as_before`.
- **Tests that must exist:** a v20 catalog with real rows through both the migration and the rebuild gives identical results
  (the existing `every_older_version_reaches_the_current_one_on_both_paths`, `lifecycle.rs:1445-1465`, copies zero rows;
  `migration_13_is_the_same_on_both_paths`, `1480-1560`, is the model); the foreign-key and `mid` invariants hold after every
  writer; sequences restored exactly (strengthen `rehearsal.rs:~75-85` from `seq >= max`); the key function: Windows
  equivalences (`ς`/`σ`, case variants, supplementary letters), Posix exactness, WTF-8 names with unpaired surrogates, and the
  version stamp triggering a rewrite and `REINDEX`.

## Considered and not in this plan

- **`analysis` stamp** (D4) and the cost of 26 columns (each NULL is a header byte): splitting the NULL-heavy columns saves only
  about 3.6 MiB; hash columns in their own table is larger (184.6 vs 180.4 MiB). Merging `analysis` into `files` (1:1) would
  make every `seq`/`changed` stamp rewrite a wide row once hashes are stored (`TO_13`: 98.7 MB of WAL as updates against
  3.4 MB as deletes).
- **`WITHOUT ROWID` clustered by `(mid, key)`**: every secondary index and every `files(id)` reference carries the key; larger.
- **`files_seq`**: thumbnail pass `f.seq > ?` (pins at `thumbcache/src/reader.rs:197` and `thumbcache/src/lib.rs`); cannot merge into
  `changed` (`seq <= changed` does not hold). **`analysis_borders_due`** empties as the borders pass runs (`writers.rs:720`);
  **`hashes_owed`/`pairs_owed`**: a rowid table is the smallest form, and flags on the wide row are rejected by `TO_13`/`TO_15`.
  Page size 1.7% at best. `AUTOINCREMENT` costs nothing.
- **Future-growth lever needing a recall evaluation:** `pdq_dihedral` stores 4 orientations x 32 B; computing them at load time
  would cut about 91 MiB of the coming growth (see `docs/eval/VERIFIER.md`).

## Expected result (dbstat MiB on the real rows)

| Layout | MiB | vs 494.4 |
|---|---|---|
| Today | 494.4 | |
| Today, repacked (`VACUUM`: 428.3 file) | 426.6 | -13.7% |
| Earlier in-place design (Phase 1 + `mfolders`/`mid`/`name_key`), repacked, measured | 284.8 | -42% |
| Stored lowercase sparse key, new layout: files side 156.7 + analysis 43.0 + borders_due 8.1 + hashes_owed 5.5 + placeholders 1.0, measured | about 214 | -57% |
| **This plan: expression key, so no key column (about -7.5 estimated; the name part measured equal to an exact-bytes index) + `analysis` integers (-6.6)** | **about 200 (estimate; G3 measures it)** | **about -60%** |
| Stored upcase key (the fallback): +31.8 over the expression key | about 232 | -53% |

The borders (8.1) and hashes (5.5) queues are inside these totals and leave by themselves; the hashes pass then grows the file
(see Context).

## Verification

Build natively in PowerShell with `QT_DIR` on PATH (not WSL; the `pg-dev` container only for the Linux CI pass). Implement every
phase, then compile once. Never migrate the live catalog: use a copy.

1. Gates G1-G4 first (they decide the key mechanism).
2. `cargo test` for `pg-model`, `pg-catalog`, `pg-scan`, `pg-query`, `pg-library`, `pg-thumbcache`, `just lint`, `just test`,
   including the tests listed in Phase 2 and plan pins for tier 1, tier 2 (`rarray`), `grouped!`, the duplicates filter and the
   hash-trigger probes, the grid/count/facet shapes, the pinned name lookup and `files_fm` readers; Posix: `IMG.jpg` and `img.jpg`
   order by exact bytes; Windows: `file_by_path` case-insensitive.
3. The `#[ignore]` rehearsal (`crates/catalog/tests/rehearsal.rs:54-80`) on a copy of the real catalog: `integrity_check`,
   `foreign_key_check`, counts, exact sequences, the new order against the old (listing only the G4 differences), SQL order equal
   to the Rust sort for all names, `REINDEX` firing when the stored `name_key_version` differs, and a connection without the
   function able to read and refused writes.
4. Re-run `dbstat` (sqlite3 is installed via winget) on the migrated copy; expected about 200 MiB after the `VACUUM`.
5. `EXPLAIN QUERY PLAN` of every changed statement on the migrated copy; timings: full ordered read about 0.07-0.15 s,
   100,000 pinned name lookups about 0.1 s (measured on the replica), tree counts about 0.05 s.
6. Benchmarks: bump `LAYOUT = "v10-windows-rules"` (`crates/bench/benches/ir/catalog.rs:43`), delete the stale `.pre-v20.bak` in
   the bench `run/` dir, regenerate with `tools/profiling/bench_data.py`; compare `scan_walk` and the grid read with the current
   build; add a tier-2 pairing benchmark and a first-scan benchmark that exercises the name lookup (700k lookups).
7. Linux pass in the `pg-dev` container (Posix rules): reference keys, case-variant names, multi-volume pairing, the embedded
   upcase table's tests (they must not depend on the OS).
8. Docs: `crates/README.md:296-312`, `docs/plans/DATABASE.md:26-32`, `SCAN_OPTIMIZATION_PLAN.md:1015`, `REFACTOR_LEDGER.md:40`,
   `DECODE_ONCE_PLAN.md:667-669,845,852`, `docs/temp/WEB_CLIENT_DESIGN.md:463,1511` (the Delta and changes cursor now owe
   their own `CREATE INDEX files_changed` in the web client's migration; `SCAN_OPTIMIZATION_PLAN.md:1015` D1 is amended to say
   dropped on 2026-11-19), `BOUNDED_MEMORY_PLAN.md` (111, 313, 411-412, 844-848, 879, 1077; its `collation_key`
   becomes this key), `NTFS_MFT_SCAN_PLAN.md:499-533`, `UROCISSA_COMPARISON.md:83-84` (keyset paging on `order_key` becomes
   `(mid, key, id)`), `docs/reference/REFACTOR_MEASUREMENTS.md:81`, `docs/temp/INCREMENTAL_SYNC.md`, `docs/temp/WEB_CLIENT_*`,
   `docs/reference/PROFILING.md:337`, `START_PROFILE.md:56`, `NOTIFICATIONS_DESIGN.md:262`, `docs/perf/callgrind/1bb039084.md`,
   `crates/thumbcache/Cargo.toml:12` (stale `thumb_folders`), and the `schema.rs` header comments.

## Evidence

**Measured (read-only on the real catalog, via `ATTACH` into memory or a `VACUUM INTO` copy in the scratchpad; the live file was
never changed; scratch databases deleted):**
- Earlier in-place design end to end (foreign keys on, real triggers and views): `integrity_check` ok, `foreign_key_check` 0,
  sequences unchanged, 285.1 MiB after `VACUUM`; old and new order identical over all 715,051 files (0 rank mismatches, 0 name
  ties); today's key format `folder key 00 lower 00 exact` confirmed, Rust lowercase == Python per-char `lower()` for all names
  (4,558 non-ASCII); reads as fast or faster (full ordered read 0.15 s both, DESC 0.17 -> 0.11 s, scope count 0.15 -> 0.055 s,
  502,858-file scope 92 -> 54 ms). Plain `VACUUM` of today's layout: 428.3 MiB.
- New layout, lowercase sparse key, files side: `files` 84.0, `files_mn` 39.1, `files_fm` 8.1, `files_seq` 8.2, identity index 17.4 =
  156.7 MiB, against 226.9 MiB in the earlier in-place design (-70.2); composite foreign key: 0 violations. Pieces: kind/ext/volume
  as integers -17.9 (`files` 116.6 -> 104.2, identity index 17.8 -> 12.3); sparse key -22.1 (63.4% of names already lowercase);
  `files_folder` (folder_id, name) 38.6 -> `(folder_id, missing)` 8.1, which also replaces `files_present` (-30.2 net).
- Name index options on all 715,051 real names (name table plus its one index, MiB): exact bytes 78.5; expression index 78.5
  (identical); stored upcase key 110.3 (+31.8). All plan as an index SEARCH. The key was a proxy (Python `upper()` per code
  point). 0 case-variant names and 0 non-UTF-8 names in this library, so neither collisions nor the WTF-8 case were exercised.
- A connection without the registered function reads everything (also via `ATTACH`) and cannot write, `integrity_check` or
  `REINDEX` (same behaviour measured for a missing collation).
- Planner (no `sqlite_stat1` in this catalog): grid, count and pairing-probe plans on the expression index as stated in Phase 2;
  unpinned pairing lookup 215 s vs pinned 0.108 s per 100,000.
- Tier-2 pairing without `files_ref` (synthetic, 715k rows, 3 volumes): 20,000 lookups 32 ms today; 311 ms for 2,000 unrewritten;
  33 ms with `volume IN (unseen list)`. `rarray` (`rusqlite-0.41.1/src/vtab/array.rs`): cost 1, 100 rows; as an `IN` right side
  it plans like a subquery (proxy test on `json_each`).
- Pitfall: in SQL `||` on two blobs yields TEXT (never equal to a blob); build keys in Rust or `CAST(... AS BLOB)`.

**Research (primary sources):** SQLite folds only ASCII (datatype3, section 7); collations and functions in an index must be
deterministic and perfectly consistent, and a changed one corrupts the index until `REINDEX` (create_collation, ICU README,
expridx). NTFS compares through the volume's `$UpCase` (a per-UTF-16-unit uppercase table, 128 KB), and a directory can be
case-sensitive (Windows 10 build 17107+); ext4 casefold is Unicode 12.1 NFD per directory; APFS is case-insensitive by default
and case-sensitive as an option. Rust `char::to_lowercase` is the unconditional full mapping (`İ` -> `i` + U+0307) and
`char::UNICODE_VERSION` reports the toolchain's Unicode version. The bundled SQLite is 3.54.1 (the fork's `libsqlite3-sys` 0.39.1;
`REINDEX EXPRESSIONS` is in the amalgamation).

**Validated against the code:** 118 unique `file:line` citations of the earlier plan, all correct or off by a line or two
(drift fixed above: `stale!` at `sql.rs:8`, `register` at `core.rs:263`, the trigger range `schema.rs:1034-1086`, the
`rows.unchanged` call at about `scan/src/lib.rs:1099`); the `files_changed` reader list, the partial-index use by the trigger
probes, the fork's lossy collation callback, and the thumbnail cache's own connection that attaches the catalog.

**Not validated (needs the code or the gates):** the Rust key function's cost (G1), NTFS's real behaviour on this machine (G2),
the full files side under B (G3), the Phase 2 rebuild steps and migration SQL, the new tests, a real `rarray` plan pin, tier 2 on a
second real volume, insert and update cost with the composite foreign key, the Linux pass, and the line numbers cited inside the
other plans' documents.
