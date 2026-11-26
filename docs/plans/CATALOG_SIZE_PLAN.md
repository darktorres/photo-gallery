# Plan: shrink the catalog (495 MiB -> about 200 MiB on this library)

## Status (2026-11-26)

**Phases 1 and 2 are built** (uncommitted when this was written), and the gates ran on a scratch copy of the owner's
catalog (715,152 files; a read-only `VACUUM INTO` copy, carried by this build's rebuild):

- **G1 passes.** `crates/catalog/examples/name_index_probe.rs`, release build, each side's better of two alternated runs
  (warm cache): the expression index against `(mid, missing, name)` takes 0.99x the time for 100,000 inserts and 1.23x
  for 200,000 pinned lookups (1.48 s against 1.21 s); its build was not slower. The bar was 2x.
- **G2 changed the table** (Gates, G2's result): NTFS's upcase is Unicode 5.1's, round-trip mappings only.
- **G3:** the carried file is 192.7 MiB after its owed `VACUUM` (D6), from 426.3 MiB: `files` 72.7, `files_mn` 39.3,
  `analysis` 36.4, the identity index 12.3, `files_fm` 10.0, `files_seq` 8.2 (the files side 142.5, against about 149
  expected), `analysis_borders_due` 8.1, `hashes_owed` 5.5. The grid's, the count's and the pairing probe's plans are
  the pinned ones (the plan tests in `browse/read/tests.rs` and `pair.rs`); no sorter.
- **G4 passes.** `real_names_sort_and_scope_as_the_reference` on the carried copy: 68 adjacent pairs of the 715,152
  names changed order, every one a `_` now after the letters; the grid's SQL order equals the Rust sort; every folder
  in every scope agrees with the reference match.
- **The rehearsal** (`crates/catalog/tests/rehearsal.rs`, strengthened: `integrity_check` with the functions, exact
  sequences) passes on the copy: every row carried, the foreign keys whole, the upgrade 32 s in a debug build.

Still open: the Linux pass (Verification, step 7), the benchmarks (step 6: the bench data is regenerated under
`v1-windows-rules`), and the docs of Verification step 8 other than this one. What was built differs from the text
below in these points, each decided while building:

- **No version is bumped** (the rule, which covers every versioned thing before the first release, is in
  `crates/model/src/versions.rs`). `KEY_ALGO_VERSION` is 1 (it was 2 before this plan) and the bench layouts stay `v1`.
  A catalog of the earlier keys still recomputes them: its `key_version` mark carried the toolchain's Unicode version,
  this build's carries the upcase table's hash. The bench data is made again under the same name
  (`bench_data.py --replace`), and `bench.py` replaces the container's copy when its stamp differs.
- **No `exts` table.** An extension's code is its position in `pg_model::media::EXTENSIONS`, a table that is only ever
  appended to (`Ext::code`, pinned by a test), so a later build's code past this build's table is an extension it
  doesn't hold (`lifecycle::mark_unheld_missing`). A table of names in the catalog would cost a join on every grid read
  for nothing a fixed list doesn't give; the SQL that shows a name uses the registered `pg_ext(code)`.
- **`analysis.codec`.** A video's codec was stored in `analysis.format`, which `format_at` then misread as an unknown image
  format (an existing bug). `format` is now an image's `Format::code` (NULL for a video), and `codec` a video's text.
- **`files_fm` is `(folder_id, mid, missing)`**, not `(folder_id, missing)`: its leading pair is the composite foreign
  key's child index, which SQLite needs to check a folder's update or delete without a scan.
- **The composite foreign key is `ON UPDATE CASCADE`**: a folder moved to another merged folder (`recompute_keys`)
  carries its files' `mid` in the same statement.
- **`path_name_key` is registered by `bring_to` too**, so every connection that makes the schema or migrates it has it;
  `core::register` takes the rules, and `core::read_only` registers it on the readers. The Python tools register it
  through `tools/pgtools.py` (`register_catalog_functions`), `bench_data.py` before its `VACUUM INTO` (which remakes the
  indexes) as well as `grow_catalog.py`.
- **`thumb_sources` joins `volumes`** (its `volume` is the serial), so the thumbnail pass's volume filter seeks the serial
  and then the identity index; `+volume` keeps the since-checkpoint read on `files_seq`.
- **The carry's conversions are expressions** (`rebuild::converted`), one per retyped or new column, each reading both
  the old shape and this one, so one copy path serves the earlier layout and a drift of this one. `volumes` is made from
  the old serials when the old file has no such table; `mfolders` is one placeholder row the copied folders hold until
  `recompute_keys` moves each to its key's row. An old extension name this build doesn't hold is stored as -1 and its
  file is missing.
- **`crates/model/tests/reference_keys.tsv` is kept as the record of the old keys**, not re-pinned: `rules_reference.rs`
  checks that the fold changed exactly for the characters the upcase table explains, and counts the order changes.
- **`stmt_journal_probe` is brought to this layout** (the writer's statements, the walk's seen set in place of
  `last_scan`), as Phase 1 asked.

The plan was written on 2026-11-19, against a catalog of schema v20 with a migration chain. The version of 2026-11-25
re-based it on the code of that day:

- **No migrations.** The chain was baked into one baseline (`crates/catalog/src/baseline.sql`, `BASELINE_SCHEMA = 1`,
  `MIGRATIONS` empty), and the baseline is edited in place until the first release (`RELEASED` empty,
  `crates/catalog/src/classify.rs`). There is no back-compat (`docs/plans/HEADLESS_SESSION_PLAN.md`, decisions). So
  "v21", "v22", `TO_21_...`, the `autoincrement.rs` rebuild pattern (the file is gone), `FOREIGN_KEYS_OFF_FROM` and the
  `pre-vN.bak` backup are all replaced by baseline edits. A catalog of the earlier layout takes `classify`'s
  `RebuildCarry`, whose carry converts its rows (D5).
- **The folder keys moved since.** `folders` is now `(path, path_key UNIQUE, match_key)` (`KEY_ALGO_VERSION = 2`,
  `crates/model/src/versions.rs`): `path_key` is the folder's identity, one row per spelling in any case under the
  Windows rules, and `match_key` is the drive-free key its files' `order_key` follows (`folders_match_key` indexes it).
  The `key_version` mark and its recompute exist (`lifecycle::set_key_version`, `recompute_keys`); the mark is
  `KEY_ALGO_VERSION` plus the toolchain's Unicode version. This plan's `mfolders.key` is today's `match_key`, and its
  name key replaces the toolchain's case folding in all three keys (D1).
- **`files.rotation` is `files.turns`** now; the analysis keeps `rotation` (the turns it was decoded with).
- **The thumbnail cache no longer attaches the catalog** on its production path (`crates/thumbs/src/pending.rs` merges two
  streams). Only the legacy `pending_for` (`crates/thumbcache/src/reader.rs`) still does, and
  `docs/plans/BOUNDED_MEMORY_PLAN.md` step 4.6 deletes it.
- **Later decisions that change the target** (2026-11-25, `docs/plans/NTFS_MFT_SCAN_PLAN.md` Catalog, adopted by the
  user): **a row per link** (a `links` table takes each file's folder, name, ext and order key) and **volume-relative
  paths** (a `volumes` table; drive letters are never stored). The order is fixed: this plan's Phase 2 lands first, then
  the MFT plan's step 1, then the bounded-memory plan's Phase 2 (its Q-N3). So Phase 2 below targets today's `files`, and
  the MFT plan's step 1 then moves `mid`, `name`, `ext` and `files_mn` to `links` (D7).
- **One `volumes` table for both plans** (D8): the interning of Phase 2, which the MFT plan's step 1 extends.
- **Two other plans touch this one's decisions:** the bounded-memory plan's ChangeFeed (step 7.1) reads `files.changed`
  in order, which D2's dropped index served; and the MFT plan's `content_gen`, which D4's staleness rule must absorb.
  Both are reconciled below.

The measurements below (Context, Expected result, Evidence) were taken on 2026-11-19 at schema v20. The layout they
measured is the same in every part this plan changes (the `files` columns and indexes, `analysis`), except the folder
keys, so they still stand; G3 re-measures the result.

## Context

The user's catalog (`D:\PhotoGallery\catalog\catalog.sqlite`, 495 MiB (519 MB), 715,051 files, 334 folders, 1 volume, schema v20
when measured, `path_rules = windows`) was analysed with `dbstat` (file 495.1 MiB; 155 pages are pointer-map pages because
`auto_vacuum = incremental`; `freelist_count` 0):

| Object                                          | MiB             | Note                                  |
|-------------------------------------------------|-----------------|---------------------------------------|
| `files` table                                   | 186.8           | 237 B/row; `order_key` is 117 B of it |
| `files_order` index                             | 104.1           | `order_key` again                     |
| `analysis` table                                | 56.6            | 56 B/row, 26 columns; 14 MiB unused   |
| `files_folder` (folder_id, name)                | 43.6            | duplicates the name                   |
| `UNIQUE(volume, file_ref)`                      | 25.2            |                                       |
| `files_ref`                                     | 16.2            |                                       |
| `analysis_borders_due`                          | 10.6            | transient (712,576 rows due)          |
| `files_changed` / `files_seq` / `files_present` | 9.3 / 9.3 / 8.4 |                                       |
| `analysis_file_hash` + `analysis_pixel_hash`    | 8.0 + 8.0       | index only NULLs today                |
| `hashes_owed`                                   | 7.1             | transient queue                       |
| unused space inside pages                       | about 75        |                                       |

When measured, `order_key` = folder key (lowercase folder path, 32.6 B, repeated per file) + `0x00` + lower(name) +
`0x00` + exact name. Today it is the folder's `match_key` plus the name in the same form.

**Constraints from the user:** churn and compatibility are not a concern; pick the most proper design. The software will
also run on Linux and maybe macOS, so name-equivalence rules follow `PathRules` (`crates/model/src/rules.rs`), never a
hardcode. Names are canonical WTF-8 bytes (`crates/model/src/name.rs`): an unpaired Windows surrogate or a Linux stray
byte is kept as bytes, and two names are equal exactly when their bytes are.

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
function `path_name_key(name BLOB) -> BLOB`, built per `PathRules`, is registered on every catalog connection beside
`pg_query::register_functions` and the `rarray` module (`register`, `crates/catalog/src/core.rs`, with the connection's
rules passed in), with the deterministic flag. The index is `files_mn(mid, missing, path_name_key(name))`. The key exists
only in the index; the table keeps the name once.

- **Windows key:** each UTF-16 unit mapped through an **embedded, versioned upcase table** generated from the Unicode
  data (BMP only, like `$UpCase`; supplementary letters are left as they are), then encoded as WTF-8. The table is not the
  running OS's `RtlUpcaseUnicodeChar` (`BOUNDED_MEMORY_PLAN.md` step 1.6, collation; it varies by Windows build and
  cannot run in the Linux CI), not the volume's `$UpCase` (not openable unprivileged, `BOUNDED_MEMORY_PLAN.md` §5), and
  not the toolchain's case folding that today's keys use (it moves with the Rust toolchain's Unicode version). The
  embedded table is deterministic across machines and testable on Linux. Its hash is the version stamp. The
  bounded-memory plan's journal collation (`collation_key`) uses the same table.
- **Order consequence:** the key's byte order is code-point order, which differs from NTFS's UTF-16-unit order only for
  supplementary letters against U+E000..U+FFFF. The catalog's order has to be deterministic and consistent with the
  equivalence, not equal to NTFS's. Visible difference from the lowercase key: upcase order puts `[ \ ] ^ _` after the
  letters, lowercase order puts them before.
- **Posix key:** the identity function (exact bytes). The index then holds the name bytes once, and the 65 MiB that a
  stored key would add on Posix rules never appears.
- **Folder keys use the same mapping.** `folders.path_key` and `folders.match_key` (and so `mfolders.key`, which is the
  `match_key`) map each part through the rules' name key instead of today's lowercase, so a folder and a file are one
  name under the same rule NTFS applies. (This text first said `ς`/`σ` are one name on NTFS; gate G2 showed they are
  not: NTFS keeps only uppercase mappings that round-trip, and `Σ` lowercases to `σ`.) The keys stay stored byte keys
  (334 rows), so `prefix_upper_bound` subtree ranges work unchanged.
- **Version stamp:** the `key_version` mark exists (`KEY_ALGO_VERSION`, with `set_key_version` and `recompute_keys` in
  `crates/catalog/src/lifecycle.rs`). Its value becomes the embedded table's hash plus a key-format number, in place of
  the toolchain's Unicode version. When it differs on open, `recompute_keys` rewrites the folder keys, then
  `REINDEX files_mn` runs, before any other use. SQLite treats an index built under a different function or collation as
  corrupt (ICU README, section 3.3). `recompute_keys` today holds every folder in RAM and runs in one transaction; the
  bounded-memory plan's step 2.5 pages it.
- **Connections without the function:** measured on SQLite 3.54.1: they read everything, also through `ATTACH` and the
  views, and a lookup that uses only the `(mid, missing)` prefix works; they cannot write the table, run
  `integrity_check` or `REINDEX` (`unknown function: path_name_key()`); `quick_check` passes. So: register it on every
  catalog connection (`Catalog` and `Reader`), on any thumbnail-cache connection that still attaches the catalog (only
  the legacy `pending_for`, until the bounded-memory plan deletes it), and in the Python tools that write `files`
  (`tools/profiling/bench_data.py`, `tools/profiling/pairing/grow_catalog.py`). Add a test that opens the catalog without
  the function and checks that every statement a reader without it runs plans without a write to the index.
- **Equality in SQL:** the column is BINARY, so `name = ?` is exact bytes. The pairing name lookup (today
  `folder_id = ? AND name = ? AND id <> ?`, `crates/catalog/src/pair.rs`) becomes
  `INDEXED BY files_mn ... WHERE mid = ? AND missing = ? AND path_name_key(name) = ? AND name = ? AND id <> ?`: the index
  finds the equivalence class (case variants on Windows), the residual `name = ?` makes it exact. `file_by_path`
  (`crates/catalog/src/lookup.rs`, documented case-insensitive on Windows, case-exact for the name today) becomes a probe
  on the key plus the rules' folder check.

**Rejected alternatives (each with its reason):**

- `NOCASE`/`lower()`: ASCII only (sqlite.org/datatype3.html, section 7). ICU extension: locale order (CLDR) rather than the
  filesystem's rule, an optional build, and an OS-provided ICU that changes silently invalidates the index.
  sqlean `text_nocase`: simple folding over the BMP only, an extension DLL on every connection, no WTF-8.
- A registered collation with `name` as `TEXT`: names with unpaired surrogates or stray bytes cannot be TEXT, and the fork's
  collation callback converts through `String::from_utf8_lossy` (`rusqlite .../src/collation.rs:79-83`), which would merge
  distinct names. A mixed TEXT/BLOB column was considered and not needed.
- A stored key column (with the sparse NULL trick): with upcase keys most names differ from their key, so it costs +31.8 MiB
  for the name part (measured: 110.3 vs 78.5 MiB) and doubles on Posix. It is the fallback if the gate G1 fails.
- Keeping the toolchain's case folding: its result moves with the Rust toolchain's Unicode version, so every toolchain
  update that changes Unicode recomputes every key (today's `key_version` rule), and it is a lowercase, not NTFS's upcase.

### D2. Drop the `files_changed` index (9.3 MiB; column and triggers stay)

No production SQL reads through it today: the one range read is a test helper (`test_support.rs`,
`WHERE changed > ?1 ORDER BY id`), which becomes a scan. **DECIDED by the owner (2026-11-19): drop it now and recreate it
later in the plan that needs it.** This reverses `docs/plans/SCAN_OPTIMIZATION_PLAN.md` D1 ("keep it", decided
2026-11-15), whose reason was a planned resync reader.

Two plans now owe it back, whichever lands first:

- the web client: `docs/webclient/WEB_CLIENT_DESIGN.md` (the Delta request and the changes cursor) reads "rows with
  `changed` > S via the `files_changed` index". Measured on the real catalog: 0.07-0.12 ms with the index, about 310 ms as
  a full table scan;
- `docs/plans/BOUNDED_MEMORY_PLAN.md` step 7.1, the paged ChangeFeed over `(changed, id)`.

Each adds `CREATE INDEX files_changed ON files(changed)` (about 0.5 s to build on 715k rows, 8-9 MiB) to the baseline
with its change, and says so. Record that in both and update the wording (Verification, Docs).

### D3. The hash indexes become partial, not dropped

`WHERE col IS NOT NULL`. EXPLAIN-verified on SQLite 3.54.1: `file_hash = ?` (the trigger probes of
`docs/plans/DECODE_ONCE_PLAN.md` Phase 4) and `file_hash > ?` use the partial index with no `IS NOT NULL` term. The
bounded-memory plan's `hash_runs` triggers (its D27) probe the same way. Dropping them outright is a measured follow-up
after the hashes pass.

### D4. `analysis` keeps its own `size`/`mtime`/`rotation`/`kind`

A single `source_seq` would save 13.5 MiB (43.0 -> 29.5, measured) plus about 8 MiB once `placeholders` fills, but
staleness is deliberately a by-value comparison of the file's stat triple in every store that derives something from a
file: `analysis` (`stale!()`, `crates/catalog/src/sql.rs`, and the analysis triggers in `baseline.sql`), `placeholders`,
`file_failures`, and the thumbnail cache's own file (`thumbs.sqlite`, its `Source`). A revision counter would couple that
file to the catalog's counter (`files.seq` also moves for unrelated reasons), so the cache could no longer survive a
catalog rebuild on its own evidence, and moving only `analysis` would leave two staleness schemes.

**`content_gen` (2026-11-25).** The MFT plan adds `files.content_gen`, bumped by an external data-change reason from the
journal, so a rewrite that restores size and mtime is caught. It is added to the triple, not put in its place: the
stores compare (size, mtime, rotation or turns, kind, content_gen). A catalog carried by a rebuild keeps `content_gen`
verbatim, so the cache still survives the rebuild on its evidence. A catalog made again starts every file at 1: an old
cache entry then matches only when size, mtime and the generation all agree, which is the by-value rule's own case.
`docs/plans/NTFS_MFT_SCAN_PLAN.md` (content generation) says the same.

### D5. Landing: baseline edits, carried by the rebuild

- **Both phases are edits to `crates/catalog/src/baseline.sql`.** Phase 1 changes indexes only and can land on its own,
  at any time. Phase 2 lands before the MFT plan's step 1 and the bounded-memory plan's Phase 2 (their order, Status),
  so they write their statements against its layout.
- **A catalog of the earlier layout** takes `classify`'s `RebuildCarry`: a fresh catalog in the new layout, and the old
  rows carried into it. For Phase 1 the plain carry is enough (the tables keep their columns). For Phase 2 the carry must
  convert (Phase 2, Rebuild path): the same "converted tables" notion the MFT plan's step 1 needs for its split (its
  Catalog, item 8). It is built once, here.
- **What the other plans keep using:** the bounded-memory plan ranges over the folder key in steps 3.6, 4.1 and 7.4, which
  become `mfolders.key` ranges and `(mid, path_name_key(name))` probes. Its 2026-11-07 anchors, `unpaired_anchor` and
  removal masks are gone (its D9 and D34), so nothing else maps.

### D6. Compaction

A rebuild writes a fresh file, but not a compact one: the carry fills the indexes row by row (in id order, not key
order), and the key recompute moves every file's index entries. Measured on the owner's catalog (2026-11-26): 213.9 MiB
carried, 22.7 MiB of it unused inside pages; a `VACUUM` (3.6 s; it copies each index in key order) makes it 192.7 MiB
with 1.6 MiB unused. So a rebuild that carried rows sets `meta.vacuum_owed`, and the start's `lifecycle::vacuum_if_owed`
runs it (under the rebuild's exclusive hold, with `VACUUM_ROOM`; a failure keeps the mark for the next start, where a
failure inside the rebuild would carry nothing). `MIGRATION_ROOM` stays for what else sets it. After
the borders and hashes passes free their queues (10.6 + 7 MiB) and `analysis` grows (about 15 MiB unused), another
compaction is wanted: add an automatic trigger when free pages pass a threshold (no periodic trigger exists;
`Catalog::shrink` in `crates/catalog/src/removal.rs` only runs `incremental_vacuum` after removals).

### D7. With links, the name index moves (the MFT plan's step 1)

After this plan, the MFT plan's step 1 splits `files` into `files` (identity and per-file data) and `links` (folder, name,
ext, order). Under that split:

- `mid`, `name` and `ext` move to `links`, with the composite foreign key `(folder_id, mid) → folders(id, mid)`;
- `files_mn` becomes `links_mn(mid, path_name_key(name))` on `links`. A link row exists only while it is listed (an
  unseen link is deleted, and a file with no link is missing), so the index needs no `missing` term;
- the pairing name lookup and `file_by_path` probe `links_mn`;
- `files_fm` (`folder_id, missing`) becomes the links' folder index;
- `files_identity`, `files_seq` and the partial work-list indexes stay on `files`.

Gate G3's figures then split between the two tables; the sum is about the same, since each name is still stored once.

### D8. One `volumes` table

Phase 2 interns the volume: `volumes(id INTEGER PRIMARY KEY, serial INTEGER NOT NULL UNIQUE)`, and `files.volume`
becomes its id. The MFT plan's step 1 extends this same table (`guid_path`, `label`, for the volume-relative paths and
offline volumes) instead of making its own keyed by serial. The 64-bit serial stays the volume's identity
(`FileIdentity::volume: u64`, `crates/model/src/ids.rs`), and the catalog maps serial ↔ id at its edge: the work lists'
`VolumeSet` and the lanes (`crates/library/src/lanes.rs`) keep speaking serials. Clones still copy the serial, so the
duplicate-serial refusal stays.

## Gates: measure these before building Phase 2

- **G1. The Rust key function's cost.** Benchmark `CREATE INDEX`, 100,000 inserts and 200,000 pinned lookups of the real
  function (embedded table, WTF-8) against the exact-bytes index on the scratch copy of the real catalog. The earlier Python
  timings are unusable (per-statement overhead). Accept B when the overhead stays near the C `NOCASE` collation's measured
  +8% / +18% / +19% (for a function call it will be higher; the bar is no more than 2x the exact-bytes index). Otherwise fall
  back to the stored-key column.
- **G2. NTFS on this machine.** In a temp directory create `σ.txt` and `ς.txt` (does the second collide?), `𐐨.txt` and `𐐀.txt`
  (Deseret, supplementary), `İ.txt` and `i̇.txt`, and compare the answers with the embedded table and with
  `RtlUpcaseUnicodeChar`. This writes a few temporary files; run it at the start of implementation.

  **Result (2026-11-26, C:, Windows 10 19045, NTFS):** run over the whole BMP, not only these pairs. A file was made
  for every BMP character in one folder, in code-point order, and a refused create named the character it folds onto:
  973 characters fold. `σ`/`ς`, `𐐨`/`𐐀`, `İ`/`i̇`, `ß`/`ẞ`, `ı`/`I`, `ſ`/`S`, `µ`/`Μ` and `ǅ`/`Ǆ` are all two names.
  The table first generated (Unicode 15.1.0's simple uppercase, 1,190 pairs) folded 217 characters NTFS keeps apart.
  The rule that reproduces all 973 folds, and the character each folds onto, exactly is **Unicode 5.1.0's simple
  uppercase, kept only where the uppercase's simple lowercase is the character back** (973 pairs): a one-way mapping
  (`ς`, `ı`, `ſ`, `µ`, the titlecase digraphs, the Greek symbol forms) and every case pair added from 5.2 on (Latin
  Extended-C's later letters, Georgian Mtavruli, Cherokee small letters, Latin Extended-D) are their own uppercase. The
  table was regenerated so (`tools/gen_upcase.py`, with the rule in its doc). `RtlUpcaseUnicodeChar` was not compared:
  the volume's own answers are what names collide by. Another volume, formatted by another Windows, may carry another
  `$UpCase`; the library's volume (D:) was not probed, since the probe writes files to it.
- **G3. The full `files` side under B** on the scratch copy (the name part alone measured 78.5 MiB, identical to an
  exact-bytes index): expected about 149 MiB, against 156.7 MiB for the lowercase sparse stored key. Also EXPLAIN the grid
  (ascending and descending), the count and the pairing probe on the real data, with no `TEMP B-TREE`.
- **G4. The ordering differences list**: sort the real names by today's key and by the new key and list every position
  where they differ; they must all be of the punctuation, supplementary or `ς` kinds. (The test that lists them,
  `real_names_sort_and_scope_as_the_reference`, accepts a pair whose first differing characters are not both ASCII
  letters or digits, or one name in two cases.)

## Phase 1: indexes (-32 MiB repacked, -41 MiB on today's pages)

An edit to `crates/catalog/src/baseline.sql`; the indexes become:

```sql
CREATE INDEX analysis_file_hash  ON analysis(file_hash)  WHERE file_hash IS NOT NULL;
CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash) WHERE pixel_hash IS NOT NULL;
-- files_ref and files_changed are removed.
```

- **Hash indexes** (-16 MiB now, and no index write per import row): readers are the duplicate counts in
  `crates/catalog/src/dups.rs` (which filter `IS NOT NULL`) and the grid's Duplicates filter (`crates/query/src/lib.rs`,
  the EXISTS forms); writers are the analysis store and the ledger's writes. Add an EXPLAIN pin for both filters and the
  trigger probes.
- **`files_ref`** (-16.2 MiB): keep `UNIQUE(volume, file_ref)`. Pairing tier 2 (`crates/catalog/src/pair.rs`, the two
  statements with `volume IS NOT NULL AND volume <> ? AND file_ref = ?`) would scan without it (311 ms for 2,000
  lookups, about 100x slower). Rewrite it to bind the unseen volumes (all volumes minus `seen`, read once with
  `seen_volumes`; `seen` only grows in a batch, so a stale list is a safe superset, and the Rust `!seen.contains(volume)`
  filter stays) as `volume IN rarray(?)`, as constant SQL, skipping the query when the list is empty. On Linux `volume`
  is a filesystem-UUID-derived id (`crates/platform/src/dir.rs`). Add a plan pin and a 2+ volume pairing test, and a
  tier-2 benchmark (none covers it today).
- **`files_changed`** (D2): change the "edited-index" case of the schema-difference test (`crates/catalog/src/lifecycle.rs`,
  `("edited-index", ...)`) to another index; fix `crates/catalog/examples/stmt_journal_probe.rs` and
  `crates/catalog/examples/pairs_load_probe.rs` where they name dropped indexes (and the long-gone `last_scan` and
  `files_fresh`); update the fixture comment and the `test_support.rs` helper. The trigger `files_changed` and the trigger
  tests are unaffected.
- **Carry:** none needed; the tables keep their columns, so a catalog of the earlier layout is carried plainly.

## Phase 2: rebuild `files` and `folders`, add `mfolders` and `volumes` (about -70 MiB more than an in-place design)

An edit to `baseline.sql`. The layout (today's columns, retyped and re-indexed):

```sql
CREATE TABLE volumes(id INTEGER PRIMARY KEY, serial INTEGER NOT NULL UNIQUE);   -- D8
CREATE TABLE exts(id INTEGER PRIMARY KEY, name BLOB NOT NULL UNIQUE);
CREATE TABLE mfolders(id INTEGER PRIMARY KEY, key BLOB NOT NULL UNIQUE);        -- one row per distinct match key
CREATE TABLE folders(id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL, path_key BLOB NOT NULL UNIQUE,
                     mid INTEGER NOT NULL REFERENCES mfolders(id), UNIQUE(id, mid));
CREATE TABLE files(id INTEGER PRIMARY KEY AUTOINCREMENT, volume INTEGER REFERENCES volumes(id), file_ref INTEGER,
  created INTEGER, folder_id INTEGER NOT NULL, mid INTEGER NOT NULL, name BLOB NOT NULL,        -- exact bytes
  ext INTEGER NOT NULL REFERENCES exts(id), kind INTEGER NOT NULL CHECK (kind IN (0, 1)), size INTEGER NOT NULL,
  mtime INTEGER NOT NULL, missing INTEGER NOT NULL DEFAULT 0, turns INTEGER NOT NULL DEFAULT 0,
  pending INTEGER NOT NULL DEFAULT 1, seq INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0,
  unpaired INTEGER NOT NULL DEFAULT 0,
  UNIQUE(volume, file_ref), FOREIGN KEY(folder_id, mid) REFERENCES folders(id, mid));
CREATE INDEX files_mn ON files(mid, missing, path_name_key(name));  -- grid order (both directions) and the pairing name lookup
CREATE INDEX files_fm ON files(folder_id, missing);                 -- replaces files_folder AND files_present
CREATE INDEX files_seq ON files(seq);                               -- files_to_import (partial) is kept as is
```

- **Why each piece:** `mfolders` holds each match key once (folder rows with equal keys, `D:\x` and `E:\X`, share a
  `mid`, so files interleave by name across drives as today; on this catalog 334 keys, 1:1). It replaces
  `folders.match_key` and its index. The composite foreign key makes the database enforce `files.mid = folders.mid`, the
  denormalisation `files_mn` needs. `kind` 0/1 and `ext` as a small id cost 0-1 byte; `volume` is interned (D8).
  `mfolders` ids are not AUTOINCREMENT (nothing persists a `mid`); say so in the baseline's comment. `order_key` goes:
  the grid orders by `(mfolders.key, path_name_key(name), id)`.
- **Integer codes:** `kind` through the `kind_video!()`/`kind_image!()` macros (`crates/model/src/media.rs`), `stale!()`,
  the triggers that read it and the `thumb_sources` view (about 33 SQL references at the 2026-11-19 count); `ext` as an id
  from `exts` (not a Rust enum order, which would persist); `volume` through `volumes`, touching the work lists'
  `VolumeSet` statements (`crates/catalog/src/work.rs`), the thumbnail candidates, the lanes' volume reads and the scan
  writer. `analysis.kind`/`format` become integers too (measured -6.6 MiB for the table), and so do the enum text columns
  of tables that will grow: `border_sides` (`edge`, `kind`, `class`, `rejected`; 0 rows until the borders pass runs, so
  no size is measurable yet). Bounded or tiny tables (`notices`, `file_failures`, `file_writes`, `ledger_ack`) keep text.
  `pdq_pairs` is already `WITHOUT ROWID`.
- **Indexes removed:** `files_order`, `files_folder`, `files_present`, `folders_match_key` (and Phase 1's `files_ref`,
  `files_changed`). `files_to_import` and `analysis_failed` stay (pinned).
- **Key API** (`crates/model/src/rules.rs`, `name.rs`): replace `PathRules::order_key` by `PathRules::name_key(name, out)`
  (the per-rules key of D1; no leading `0x00`) and the function registration; the folder keys (`path_key`, `match_key`)
  map each part through it; `scope_keys` returns bounds on the folder key: recursive `[key, prefix_upper_bound(key))`
  (empty key: no upper bound), non-recursive `[key, key ++ 0x00)`. `Name::name_key()` for the saved-view sort
  (`crates/catalog/src/browse/views.rs`) and the client's own sorting (`crates/client/src/tree.rs`,
  `crates/model/src/tree.rs`) use the same key, so SQL and Rust agree (test: the SQL order of every real name equals the
  Rust sort). The folder-key builder stays the single source for SQL and the client tree. Replace the per-char lowercase
  in `Name` only where the rules say; keep it for display. `KEY_ALGO_VERSION`'s doc states the new rule; the version is
  not bumped before the first release (`crates/model/src/versions.rs`), and the mark's table hash recomputes the keys.
- **Plan pins (the planner has no statistics):** the grid, count and facet SQL (`crates/catalog/src/browse/read/scope.rs`,
  `grid.rs`, `facets.rs`) read `FROM mfolders m CROSS JOIN files f ON f.mid = m.id WHERE f.missing = 0 AND m.key >= ?lo
  [AND m.key < ?hi] ORDER BY m.key {dir}, path_name_key(f.name) {dir}, f.id {dir}` (`CROSS JOIN` pins `mfolders`
  outer, the idiom the pairing already uses; EXPLAIN-verified: `SEARCH m ... (key>? AND key<?)`,
  `SEARCH f USING COVERING INDEX files_mn (mid=? AND missing=?)`, no sorter, ascending and descending); the browse
  fixtures order the same way. `INDEXED BY files_mn` on the pairing name lookup (both `missing` states as two probes, or
  `missing IN (0,1)` pinned): **unpinned, the planner chose `files_fm` and took 215 s for 100,000 lookups; pinned it takes
  0.108 s.** `files_fm` for the readers by folder in `roots.rs` (`mark_missing`, the folder rows) and `removal.rs` (the
  hide and delete pages); `INDEXED BY files_present` in `work.rs` becomes `files_fm`; the comments naming
  `files_order` and `files_present`; the plan pins in `browse/read/tests.rs`.
- **Rebuild path (the carry, D5):** `copy_tables` (`crates/catalog/src/rebuild.rs`) copies shared columns by name, then
  runs the conversions. The retyped columns (`kind`, `ext`, `volume`) exist in both layouts with different types, `mid` is
  `NOT NULL` with no default, and `order_key` and `folders.match_key` exist only in the old one. So `files`, `folders` and
  `analysis` must not be copied generically when the old shape differs: add a "converted tables" notion to `copy_tables`
  (a conversion reads its table from `carried` itself, through `Source::Carried`, which exists for this), build
  `volumes`, `exts` and `mfolders` first, and add them to `CARRIED` (`carried_is_every_table_of_the_current_schema`
  enforces it). `table_columns` (`crates/catalog/src/convert.rs`) reads `pragma_table_info`; an expression index adds no
  column to it. The MFT plan's step 1 adds its own conversions (one link per file) to the same notion.
- **Writers and copies** (find by symbol; the 2026-11-19 line list is stale): `ScanWriter` (`folder()` inserts or gets the
  `mfolders` row in the same transaction; the known-row reads by position), `FolderRows::unchanged` (compare the key and
  `mid`; public, used by the walk in `pg-scan`), the folder-row reads in `roots.rs`, pairing's row reads and
  `take_identity` (copies `mid`), `file_by_path` (a `mid` lookup and the rules' key), the removal's folder cleanup (deletes
  empty `mfolders`), and every raw insert and fixture (`fixture.rs`, `fixture/raw.rs`, `fixture/state.rs`'s public
  `add_folder`, which also creates the `mfolders` row, and the tests' raw inserts in `lifecycle.rs`, `removal.rs`,
  `ledger.rs`, `roots.rs`, `pair.rs`, `schema.rs`, `writers.rs`). `recompute_keys` rewrites `mfolders.key` and the folder
  keys, and no longer re-keys `files` (no `order_key`).
- **Pinned tests to re-pin:** `crates/model/tests/reference_keys.tsv` (2,034 rows, no rules column: add one; Windows and
  Posix), `rules_reference.rs`, `path_laws.rs`, the name-key tests in `name.rs`, the `rules.rs` inline tests, the browse
  read tests, the ignored `real_names_sort_and_scope_as_before`.
- **Tests that must exist:** a catalog of the earlier layout, with real rows, carried by the rebuild gives identical
  results (the carry test of D5, with at least one row in every table); the foreign-key and `mid` invariants hold after
  every writer; sequences restored exactly (strengthen `crates/catalog/tests/rehearsal.rs` from `seq >= max`); the key
  function: Windows equivalences (`ς`/`σ`, case variants, supplementary letters), Posix exactness, WTF-8 names with
  unpaired surrogates, and the version stamp triggering `recompute_keys` and `REINDEX`.

## Considered and not in this plan

- **`analysis` stamp** (D4) and the cost of 26 columns (each NULL is a header byte): splitting the NULL-heavy columns saves only
  about 3.6 MiB; hash columns in their own table is larger (184.6 vs 180.4 MiB). Merging `analysis` into `files` (1:1) would
  make every `seq`/`changed` stamp rewrite a wide row once hashes are stored (measured on the 2026-11 pairs-queue migration:
  94.1 MiB (98.7 MB) of WAL as updates against 3.4 MB as deletes).
- **`WITHOUT ROWID` clustered by `(mid, key)`**: every secondary index and every `files(id)` reference carries the key; larger.
- **`files_seq`**: the thumbnail pass reads `f.seq > ?` (`Catalog::thumb_candidates`); it cannot merge into `changed`
  (`seq <= changed` does not hold). **`analysis_borders_due`** empties as the borders pass runs (the bounded-memory plan's
  step 2.1 moves the flag to `files`). **`hashes_owed`/`pairs_owed`**: a rowid table is the smallest form, and flags on the
  wide row were rejected when the queues were made. Page size 1.7% at best. `AUTOINCREMENT` costs nothing.
- **Future-growth lever needing a recall evaluation:** `pdq_dihedral` stores 4 orientations x 32 B; computing them at load time
  would cut about 91 MiB of the coming growth (see `docs/eval/VERIFIER.md`).

## Expected result (dbstat MiB on the real rows)

| Layout                                                                                                                                                    | MiB                                      | vs 494.4       |
|-----------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------|----------------|
| Today                                                                                                                                                     | 494.4                                    |                |
| Today, repacked (`VACUUM`: 428.3 file)                                                                                                                    | 426.6                                    | -13.7%         |
| Earlier in-place design (Phase 1 + `mfolders`/`mid`/`name_key`), repacked, measured                                                                       | 284.8                                    | -42%           |
| Stored lowercase sparse key, new layout: files side 156.7 + analysis 43.0 + borders_due 8.1 + hashes_owed 5.5 + placeholders 1.0, measured                | about 214                                | -57%           |
| **This plan: expression key, so no key column (about -7.5 estimated; the name part measured equal to an exact-bytes index) + `analysis` integers (-6.6)** | **about 200 (estimate; G3 measures it)** | **about -60%** |
| Stored upcase key (the fallback): +31.8 over the expression key                                                                                           | about 232                                | -53%           |

The borders (8.1) and hashes (5.5) queues are inside these totals and leave by themselves; the hashes pass then grows the file
(see Context). Measured on 2026-11-19, at schema v20.

## Verification

Build natively in PowerShell with `PG_QT_DIR` on PATH (not WSL; the `pg-dev` container only for the Linux CI pass). Implement every
phase, then compile once. Never open the live catalog with a development build: carry a copy.

1. Gates G1-G4 first (they decide the key mechanism).
2. `cargo test` for `pg-model`, `pg-catalog`, `pg-scan`, `pg-query`, `pg-library`, `pg-thumbcache`, `just lint`, `just test`,
   including the tests listed in Phase 2 and plan pins for tier 1, tier 2 (`rarray`), the duplicate counts, the Duplicates
   filter and the hash-trigger probes, the grid/count/facet shapes, the pinned name lookup and `files_fm` readers; Posix:
   `IMG.jpg` and `img.jpg` order by exact bytes; Windows: `file_by_path` case-insensitive.
3. The `#[ignore]` rehearsal (`crates/catalog/tests/rehearsal.rs`) on a copy of the real catalog, carried by the rebuild:
   `integrity_check`, `foreign_key_check`, counts, exact sequences, the new order against the old (listing only the G4
   differences), SQL order equal to the Rust sort for all names, `REINDEX` firing when the stored `key_version` differs,
   and a connection without the function able to read and refused writes.
4. Re-run `dbstat` (sqlite3 is installed via winget) on the carried copy; expected about 200 MiB.
5. `EXPLAIN QUERY PLAN` of every changed statement on the carried copy; timings: full ordered read about 0.07-0.15 s,
   100,000 pinned name lookups about 0.1 s (measured on the replica), tree counts about 0.05 s.
6. Benchmarks: regenerate the bench data under the same `BENCH_LAYOUT` (`crates/bench/benches/ir/catalog.rs`,
   `"v1-windows-rules"`; no bump before the first release) with `tools/profiling/bench_data.py --replace`; compare
   `scan_walk` and the grid read with the current build; add a tier-2 pairing benchmark and a first-scan benchmark that
   exercises the name lookup (700k lookups).
7. Linux pass in the `pg-dev` container (Posix rules): reference keys, case-variant names, multi-volume pairing, the embedded
   upcase table's tests (they must not depend on the OS).
8. Docs: `crates/README.md` (the catalog paragraphs), `docs/plans/DATABASE.md`, `SCAN_OPTIMIZATION_PLAN.md` D1 (amended to
   say dropped on 2026-11-19), `REFACTOR_LEDGER.md`, `DECODE_ONCE_PLAN.md` (its `files_changed` and `order_key` mentions),
   `docs/webclient/WEB_CLIENT_DESIGN.md` (the Delta and the changes cursor owe their own `files_changed`, D2),
   `BOUNDED_MEMORY_PLAN.md` (step 7.1 owes it too; steps 3.6, 4.1 and 7.4 in this plan's terms; its `collation_key` is
   this key), `NTFS_MFT_SCAN_PLAN.md` (Catalog: D7 and D8 here), `UROCISSA_COMPARISON.md` (keyset paging on `order_key`
   becomes `(mid, key, id)`), `docs/reference/REFACTOR_MEASUREMENTS.md`, `docs/webclient/INCREMENTAL_SYNC.md`,
   `docs/webclient/WEB_CLIENT_*`, `docs/reference/PROFILING.md`, `START_PROFILE.md`, `NOTIFICATIONS_DESIGN.md`,
   `docs/perf/callgrind/1bb039084.md`, the header comment of `crates/thumbcache/Cargo.toml` (stale `thumb_folders`), and
   the `schema.rs` and `baseline.sql` header comments. Recount each cite at the time.

## Evidence

**Measured (read-only on the real catalog, via `ATTACH` into memory or a `VACUUM INTO` copy in the scratchpad; the live file was
never changed; scratch databases deleted; 2026-11-19, schema v20):**

- Earlier in-place design end to end (foreign keys on, real triggers and views): `integrity_check` ok, `foreign_key_check` 0,
  sequences unchanged, 285.1 MiB file size after `VACUUM` (the table's dbstat figure for the same layout is 284.8 MiB);
  old and new order identical over all 715,051 files (0 rank mismatches, 0 name
  ties); the key format of the time `folder key 00 lower 00 exact` confirmed, Rust lowercase == Python per-char `lower()` for
  all names (4,558 non-ASCII); reads as fast or faster (full ordered read 0.15 s both, DESC 0.17 -> 0.11 s, scope count
  0.15 -> 0.055 s, 502,858-file scope 92 -> 54 ms). Plain `VACUUM` of that layout: 428.3 MiB.
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
`char::UNICODE_VERSION` reports the toolchain's Unicode version. The bundled SQLite is 3.54.1 (`REINDEX EXPRESSIONS` is in the
amalgamation).

**Validated against the code (2026-11-19):** 118 unique `file:line` citations of the earlier plan, the `files_changed` reader
list, the partial-index use by the trigger probes, the fork's lossy collation callback, and the thumbnail cache's connection
that attached the catalog. **Re-checked 2026-11-25:** the migration chain is gone (baseline only); `folders` has
`path_key` and `match_key` with the `key_version` recompute; `files.turns` replaced `rotation`; `files_changed` still has
no production reader; tier 2's two `volume <> ? AND file_ref = ?` statements and `seen_volumes` are as described; the
thumbnail cache's production path no longer attaches the catalog. Line cites were replaced by symbols.

**Not validated (needs the code or the gates):** the Rust key function's cost (G1), NTFS's real behaviour on this machine (G2),
the full files side under B (G3), the carry conversions and the new tests, a real `rarray` plan pin, tier 2 on a second real
volume, insert and update cost with the composite foreign key, the Linux pass, and the cites inside the other plans' documents.
