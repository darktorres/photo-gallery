# Validated consistency audit: 702 findings

Each finding was checked against the code and docs it cites. Verdicts: CONFIRMED (the claim holds), PARTIAL (part of the claim holds, or a cite is wrong but the defect is real), REFUTED (the claim does not hold).

| Aspect | n | CONFIRMED | PARTIAL | REFUTED |
|---|---:|---:|---:|---:|
| doc-accuracy | 93 | 54 | 30 | 9 |
| duplication | 77 | 64 | 9 | 4 |
| errors | 63 | 51 | 11 | 1 |
| config | 47 | 28 | 15 | 4 |
| units-types | 44 | 41 | 2 | 1 |
| naming | 40 | 23 | 13 | 4 |
| versions | 39 | 31 | 5 | 3 |
| persisted-formats | 36 | 22 | 11 | 3 |
| architecture | 34 | 23 | 10 | 1 |
| manifests-lints | 33 | 21 | 9 | 3 |
| tooling | 31 | 21 | 6 | 4 |
| pins | 30 | 20 | 8 | 2 |
| ui-text | 30 | 23 | 5 | 2 |
| logging | 29 | 21 | 5 | 3 |
| tests | 29 | 26 | 3 | 0 |
| concurrency-unsafe | 27 | 24 | 2 | 1 |
| api-docs | 20 | 15 | 5 | 0 |
| **total** | **702** | **508** | **149** | **45** |

## doc-accuracy

### DOC-001 [high] Docs say cargo-deny is part of just lint; Justfile and CI run it outside lint — CONFIRMED

**Disposition: Fixed.** `crates/README.md`, `REFACTOR_DESIGN`, `NOTIFICATIONS_DESIGN`, `NTFS_MFT_SCAN_PLAN` and `IMPORT_FAILURES_PLAN` now say `cargo-deny` runs under `just deny`, not `just lint`.

tools/lint.py has no cargo deny step (grep finds none). Justfile:134-137 says the deny recipe is not in lint, and ci.yml:305 runs it as its own step. crates/README.md:259 says just lint includes cargo-deny.

### DOC-002 [high] README says Windows only; CI runs a Linux job and Justfile documents Linux checks — CONFIRMED

**Disposition: Fixed.** `README.md` says Windows first and that CI builds and tests the workspace on Linux.

README.md:11 says Windows only for now. ci.yml:222-223 has a Linux job for the whole workspace, and Justfile documents the Linux checks in the pg-dev container.

### DOC-003 [high] Crate README gives two different PDQ distance thresholds — REFUTED

Two thresholds serve two purposes. `MATCH_DISTANCE` is 31 (model pdq.rs:11), used by the pairing pass (README:352 says d `<=` 31). `GROUP_DISTANCE` is 22 (pdq.rs:21-28), used for grouping and the duplicates filter (README:427). Each README use matches its constant.

### DOC-004 [high] Frame sink thread docs contradict the code and each other — CONFIRMED

**Disposition: Fixed.** `close()` may be called from any thread; its flag is now a `std::atomic<bool>`, so the header, the bridge and the Rust SAFETY notes all agree with the code. (Also fixes CU-009.)

`frame_sink.h:20` says close() runs on the sink's thread. `frame_sink.rs:20-21` says close is called on the GUI thread, by Player's drop. `frame_sink_bridge.rs:23` says the sink's thread. The header and the Rust SAFETY note disagree.

### DOC-005 [high] `take_shared` doc says it waits; body returns an error — PARTIAL

**Disposition: Fixed.** `lifecycle.rs` doc now says the wait is bounded and then fails.

lifecycle.rs:65 says `take_shared` waits for a change in progress. `pg_dbfile` lock.rs:55-61 does retry `LOCK_TRIES` times with a sleep, then returns None, which lifecycle.rs:69 maps to an error. The wait is bounded and then fails, so the doc is loose rather than wrong.

### DOC-006 [high] Undecodable files: ledger says no retry, failure.rs says retry — REFUTED

The cited catalog/src/failure.rs does not exist. The code that decides retry is thumbs/src/lib.rs:121-129: Failed::Undecodable gives Some(false), no retry, which matches ledger.rs:48. No file says Undecodable is retried.

### DOC-007 [high] Settings save failure documented as reported; code discards the result — CONFIRMED

**Disposition: Fixed.** The failure is posted by `PrefSave::write` (the finding's premise was wrong about where it is posted). The discard in `settings.rs` now has a comment saying so.

settings.rs:15-17 says a failing save is a note on the app's bus and the window prints it. settings.rs:21 is let `_` = save.write(), which discards the error, so no note is posted.

### DOC-008 [high] Numeric values in doc comments differ from the code — CONFIRMED

**Disposition: Fixed.** The maker doc names `KEEP_MAKERS` (8); the rating doc says a user's 0 stars clears the row.

makers.rs:1-2 says an idle maker is kept up to a few, but `KEEP_MAKERS` is 8 (makers.rs:10). rating.rs:4 and :20-33 say user ratings are 1-5, but Rating::new accepts 0 (stars `<=` MAX). The statement cache doc (16 default, 64 set) is accurate. The xmp and encode instances are mostly fine.

### DOC-009 [high] Perf callgrind doc Ir counts disagree with docs/perf/baseline.json for the same bench keys — PARTIAL

**Disposition: No change.** The callgrind note names commit 1bb0390, when it had 11 benches; `baseline.json` is from a later commit.

The callgrind md numbers differ from baseline.json (facets.all 93,262,634 vs 47,331,512). The md is from commit 1bb0390 and baseline.json was re-measured later (layout v7, per `REFACTOR_LEDGER`). Different commits are not a contradiction, but the md title still says 11 benches for a 23-entry file.

### DOC-010 [high] `NOTIFICATIONS_DESIGN` gives two retention rules for dismissed events — PARTIAL

**Disposition: Fixed.** The retention line now says "dismissed states" and notes that dismissed events are deleted at once.

`NOTIFICATIONS_DESIGN.md:16` overrides the 30-day rule: dismissed events are deleted, and the purge applies to dismissed states only. Line 261 says dismissed purged after 30 days in the retention bullet without saying states, so it reads as covering events too. Wording problem, not a code conflict.

### DOC-011 [high] THUMBNAILS.md names Lanczos3 as the thumbnail resampler; PROFILING says the path moved — CONFIRMED

**Disposition: Fixed.** `THUMBNAILS.md` names Magic Kernel Sharp 2021, the resize `codec/src/resize.rs` uses.

THUMBNAILS.md:23 says Lanczos3 from 4x is the thumbnail path. codec/src/resize.rs:1 says Magic Kernel Sharp 2021 is the one resize for every pipeline (kernel.rs:1, 42). PROFILING.md:413 already notes the move, but THUMBNAILS.md did not follow it.

### DOC-012 [high] `WEB_CLIENT_DESIGN` says the desktop never writes the catalog after cutover; App::open runs Catalog::prepare — CONFIRMED

**Disposition: Fixed.** The design now decides: the desktop stays the only writer after cutover and keeps `prepare` at start; the web server opens the catalog read-only.

app.rs:62 calls `pg_library::open_catalog`, which at library lib.rs:27-29 calls Catalog::prepare, which can migrate. `WEB_CLIENT_DESIGN.md:1885` says the desktop never writes the catalog after cutover, so the design's premise is wrong.

### DOC-013 [high] Catalog queue.rs module doc names writer API functions and table that do not exist — CONFIRMED

**Disposition: Fixed.** The module doc names `next_write`, `record_write_temp`, `write_done`, `write_failed`, `drop_gone_write` and `retry_write_copies`.

queue.rs module doc names `next_ready`, `record_temp`, done, failed, `drop_gone` and `retry_copies`. The code has `next_write` (:133), `record_write_temp` (:229), `write_done` (:245), `write_failed` (:304), `drop_gone_write` (:315) and `retry_write_copies` (:327).

### DOC-014 [high] `MULTI_DRIVE_LANES_PLAN` status says two checks were not run; its ledger says they passed — CONFIRMED

**Disposition: Fixed.** The status line is marked as the state at that time, and points to the ledger's later runs.

`MULTI_DRIVE_LANES_PLAN.md:21` says just check-linux and just qmltest were not run. The same plan's ledger at :427 records check-linux passing and :429 records qmltest 108 passed.

### DOC-015 [high] `DECODE_ONCE_PLAN` body contradicts its own superseded notes — PARTIAL

**Disposition: No change.** The plan marks superseded text under a marker, so the body is history by its own convention.

`DECODE_ONCE_PLAN.md:44` and :953 are dated superseded notes above bodies that still describe the old design (thumbnails in Features, DCT scaling). The plan keeps superseded text under a marker, so the body is history. Not checked line by line, so partial.

### DOC-016 [medium] crates/README cites a RUSTDOCFLAGS cargo doc step that CI does not run — CONFIRMED

**Disposition: Fixed.** The doc now points to `tools/lint.py`, which `just lint` runs.

crates/README.md:149 says the RUSTDOCFLAGS cargo doc command is in .github/workflows/ci.yml. ci.yml has no RUSTDOCFLAGS. The flag is in tools/lint.py:32 (`DOC_FLAGS`), which just lint runs.

### DOC-017 [medium] CI comment says every cargo command takes --locked; several Justfile recipes do not — CONFIRMED

**Disposition: Fixed.** The Justfile's `build`, `run` and `dev` recipes now pass `--locked`, so the CI comment is true.

ci.yml:298 says every cargo command takes --locked. Justfile:48-50 cargo build, :61-63 cargo run --release, and :65-67 cargo run have no --locked.

### DOC-018 [medium] Plan docs give different counts of stored PDQ orientations — CONFIRMED

**Disposition: Fixed.** `PLAN.md` says 4 orientations are stored and a query probes 8 variants.

PLAN.md:134 says 8 dihedral variants are stored per file. Code has ORIENTATIONS = 4 (model pdq.rs:8) and `BLOB_BYTES` = 4 x 32 (pdq.rs:118). `OPEN_ITEMS.md:80` records the change as resolved 2026-11-18.

### DOC-019 [medium] Doc comments name modules, crates or constants that are not where they are cited — CONFIRMED

**Disposition: Fixed.** `catalog/src/lib.rs` names `pg_thumbcache::pending_in`.

catalog lib.rs:10 names `thumbs::pending_in`. There is no thumbs module in catalog. The function is `pg_thumbcache::pending_in` (thumbcache lib.rs:39 re-export). The other cited instance, inbox.rs:35 naming `pg_app::inbox::LISTED`, is correct (app inbox.rs:128).

### DOC-020 [medium] Same concept named differently in docs and code — CONFIRMED

**Disposition: Fixed.** The inbox doc names `clear_notices`, the method the clear path calls, and says it delegates to `replace_notices`.

app inbox.rs:67 says whether a clear has work is decided by `replace_notices`. The clear path calls `clear_notices` (app inbox.rs:665), so the doc names the wrong catalog method for clear.

### DOC-021 [medium] Writers doc says statements are cached on the connection; several writers prepare uncached — CONFIRMED

**Disposition: Fixed.** The analysis, placeholder and borders writers now use `prepare_cached` and `CachedStatement`, as the doc says.

writers.rs:29-31 says statements are cached on the connection (`prepare_cached`). writers.rs:377, :378-379, :720 and :689 use uncached tx.prepare, so the doc overstates the caching.

### DOC-022 [medium] baseline.sql lists different `file_failures` pass codes than the triggers write — CONFIRMED

**Disposition: Fixed.** The `file_failures.pass` comment lists `'import'` too.

baseline.sql:123 lists pass codes thumbs and borders only. The triggers at :359 write pass 'import', and `ledger_ack` at :135 lists import too. The comment omits import.

### DOC-023 [medium] Facet ordering documented differently in catalog and client, and not pinned by the tests it cites — PARTIAL

**Disposition: Fixed.** `browse/read/facets.rs` doc gives types in `MediaType` order; the client tests already pin the order.

catalog facets.rs:26 says types are most-files-first, but facets.rs:97 sorts types by MediaType order. Exts are sorted by count (:99). The client tests/facets.rs:19-20 does enumerate ALL and compare it with `from_index`, so the order is pinned; the not-pinned claim is wrong.

### DOC-024 [medium] Test doc comments claim more than the assertions check — CONFIRMED

**Disposition: Fixed.** The `video.rs` comment says blue at the top after the turn (what the assertion checks); the `borders.rs` comment says the unreadable file is counted in `failed`; `listing_laws` says "the folders, recursions, sorts and queries that proptest draws".

library/tests/video.rs:192 says red at the top after a catalog turn, but the assertion at :199-200 puts BLUE at (40,20), the top, and RED at (40,140), the bottom. The comment is wrong. `listing_laws` doc says 'for every' where proptest samples folder and recursion; that is looser. borders.rs:90-91 counts the unreadable file in stats.failed while the comment says it is no failure; the ledger part holds.

### DOC-025 [medium] thumbs and thumbcache docs disagree with their code — CONFIRMED

**Disposition: Fixed.** The `recorded()` doc says `Some(false)` is every recorded failure today and that `Some(true)` (the ledger's retry flag) is not yet produced. The type stays, since the ledger stores the flag.

thumbs/src/lib.rs:121-123 says Some(true) means tried again by the next pass. recorded() at :124-129 returns only Some(false) (Panicked, Undecodable) or None (Unreadable, Other). The Some(true) case is documented and never produced.

### DOC-026 [medium] Docs disagree on which formats lose metadata and which are re-encoded on crop — PARTIAL

**Disposition: No change.** `encode::cropped` sends other TIFFs through `image`, as `crop.rs` says; there is no contradiction.

Metadata claims agree across edit/lib.rs:2-4, crop lib.rs:117 and edit crop.rs:9-11, and reencoded sets metadata false (crop.rs:95-98). crop.rs:9-11 says other TIFFs go through image, but crop.rs:66 routes every Tiff to reencoded. Whether encode::cropped sends other TIFFs to image was not read. The cited crop lib.rs:139 is unrelated to metadata.

### DOC-027 [medium] Model crate doc says it has no dependencies; its Cargo.toml lists four — CONFIRMED

**Disposition: Fixed.** `THUMBHASH_MAX` says the encoder is not a dependency of the model, which is the real reason for the literal.

analysis.rs:136-137 says the model has no dependencies and copies `MAX_LEN` by hand for that reason. model/Cargo.toml:12-16 lists arrayvec, blake3, serde and strum, plus optional specta.

### DOC-028 [medium] writeback docs name notice tags and queue types that differ from code — REFUTED

Tags match the code: notice.rs:975 maps WriteFailed to write-failed and :977 WriteDropped to write-dropped. writeback lib.rs:3 says writes come from the `file_writes` queue, which is the table that catalog queue.rs reads through Job values, so that is one concept with two names, not a contradiction.

### DOC-029 [medium] dbfile judge emits a difference kind the schema doc does not list — CONFIRMED

**Disposition: Fixed.** The `schema.rs` doc lists `header` as a difference kind.

dbfile judge.rs:138 emits Difference::Changed with kind header. The schema.rs:28-29 doc says the kinds are table, index, trigger, view and rows. Difference is the same enum (schema.rs:31), so header is an undocumented kind.

### DOC-030 [medium] log record parse doc does not match the continuation rule — CONFIRMED

**Disposition: Fixed.** `parse`'s doc states the rule the code has: a line continues only a plain record.

log record.rs:73-75 says any other line continues the record before it. record.rs:91 continues only a plain record with no t, and record.rs:98 starts a new plain record after a JSON record. The doc overstates the rule.

### DOC-031 [medium] import hashes doc says failures are stored as the import stores them; the formatting differs — CONFIRMED

**Disposition: Fixed.** `hashes.rs` stores failures with `Cause::of(&e).to_string()`, the same text as the import, so the doc is true.

hashes.rs:93 says failures are stored as the import stores them. hashes.rs:121 stores format!("{e:#}"), while import lib.rs:179 stores `Cause::of(&e).to_string(`). The two formats differ.

### DOC-032 [medium] Orientation and resize docs state contracts the code does not enforce — PARTIAL

**Disposition: No change.** `fast_image_resize`'s `CropBox::crop` rejects out-of-bounds regions, so the doc is accurate.

resize.rs:13-15 says `resize_region` fails when the region is outside the image. `resize_region` (:79-91) clamps the region with max(1.0) and does no bounds check, and run (:98-108) checks only the pixel buffer. Whether `fast_image_resize` rejects an out-of-bounds crop was not confirmed (registry search timed out). The orientation.rs:1 claim is accurate, since both read and write use the same private `exif_block`.

### DOC-033 [medium] Grid rating and model rating docs disagree with the file and catalog — REFUTED

grid.rs:30 gives the user's rating: None when unrated, with no catalog row. rating.rs:1-2 gives the file's own rating (`file_ratings`, 0 to 5) as a separate thing. The two do not conflict. The cited model lib.rs:41 is a module list and does not discuss ratings.

### DOC-034 [medium] Model notice doc says library paths are RulePaths; the field is a String — PARTIAL

**Disposition: Fixed.** `notice.rs` module doc names `FileFailure.path` as the one library path that is a `String`.

notice.rs:9 says library paths are RulePaths, exact. FileFailure.path is a String, and its doc at notice.rs:303 says its path as the user sees it, so the String is deliberate display text. The module doc does not mention that exception.

### DOC-035 [medium] Lock doc says the name is removed on drop; Unix HeldLock has no Drop — PARTIAL

**Disposition: Fixed.** The `platform/src/testing/lock.rs` doc says only `HeldLock::delete` removes the name; dropping leaves it.

The cited platform/src/lock.rs does not exist. The lock is in platform/src/testing/lock.rs. Its doc at :25-26 says a lock from `create_for_delete` or `take_for_delete` removes its name when it goes, linking HeldLock::delete. No Drop impl exists in that file, so when it goes is loose wording; delete is the only removal.

### DOC-036 [medium] video ffi doc says bindgen does not emit `AV_NOPTS_VALUE`; build allowlists it — PARTIAL

**Disposition: Fixed.** The bindgen output of this build (`target/debug/build/pg-video/*/out/ffmpeg.rs`, generated from the FFmpeg headers) has no `AV_NOPTS_VALUE`, so the `VARS` entry in `crates/video/build.rs` was dead and is removed. `ffi.rs` keeps the hand-written constant. `pg-video` checks green.

ffi.rs:38 says bindgen does not emit `AV_NOPTS_VALUE`, and ffi.rs:39 defines it by hand. build.rs:38 still allowlists `AV_NOPTS_VALUE` in VARS. If bindgen cannot parse the macro, the allowlist entry is dead. No code conflict was shown, and the bindgen output was not generated here.

### DOC-037 [medium] GPU device creation cost stated with different metrics — CONFIRMED

**Disposition: Fixed.** The 0.3 s is named as wall-clock time and the 0.9 G instructions as callgrind, in each place they appear.

Creation cost appears as ~0.3 s in `gpu_device.h:26` and `gpu_device_bridge.rs:15`, and as ~0.9 G instructions in `gpu_device.cpp:1` and `gpu_device.rs:2`. The two are different metrics for one operation, stated in different units with no link.

### DOC-038 [medium] Budget, batch and rate figures in jobs and import docs disagree with constants — REFUTED

pipeline.rs:46-48 sets `BUFFER_PER_SLOT` to 12 MiB, and budget() is (readers + workers) x 12 MiB. With 8 readers and 12 workers that is 240 MiB, which the doc gives as 240 MB (only the MB/MiB label differs). control.rs:22-23 gives about 60 marks; the test at :702 marks once a second, so 32 marks is consistent with that sampling.

### DOC-039 [medium] Removed or renamed items referenced in plan documents — PARTIAL

**Disposition: No change.** The references are historical notes in dated plans.

`BACKGROUND_JOB_AUDIT_FIXES.md:248` says NOTIFICATIONS.md is removed, which the doc itself states. Cited line numbers in the migration plans are stale (schema.rs:804, :956, lifecycle.rs:1229 not all checked). The removed-file references are historical notes.

### DOC-040 [medium] Plan and schema docs disagree on the migration route — CONFIRMED

**Disposition: Fixed.** `BACKGROUND_JOB_AUDIT_FIXES` says the pre-release route is a baseline edit in place, matching `schema.rs:28-31`.

`BACKGROUND_JOB_AUDIT_FIXES.md:19` says migrations, not a BASELINE edit, because the baseline is frozen (schema.rs:28-31). The code at schema.rs:28-31 says before the first release the baseline is edited in place, and MIGRATIONS is empty. `BOUNDED_MEMORY_PLAN.md:18` and :25 and `CATALOG_SIZE_PLAN.md:125` still plan one BASELINE edit.

### DOC-041 [medium] `BOUNDED_MEMORY_PLAN` commit policy contradicts the commit rule in the background plan — CONFIRMED

**Disposition: Fixed.** `BOUNDED_MEMORY_PLAN` commits only when a commit is asked for, the standing rule.

`BOUNDED_MEMORY_PLAN.md:144` says phases 1-8 are one commit each. `BACKGROUND_JOB_AUDIT_FIXES.md:25` sets the standing rule: commit only when asked. The two plans conflict on whether commits happen without a request.

### DOC-042 [medium] Catalog size figures disagree across DATABASE.md, `CATALOG_SIZE_PLAN` and budgets — CONFIRMED

**Disposition: Fixed.** `DATABASE.md` uses 128 B for `pdq_dihedral` (4 × 32), 117 B for `order_key`, and SQLite 3.54.1.

DATABASE.md:26 gives `pdq_dihedral` as 256 B per file. `CATALOG_SIZE_PLAN.md:31` gives 128 B, and the code is 4 x 32 = 128 (pdq.rs:118). `order_key` is 121 B in DATABASE.md:27 and 117 B in `CATALOG_SIZE_PLAN.md:11`. DATABASE.md:101 says the app ships SQLite 3.54.0, `CATALOG_SIZE_PLAN.md:357` says 3.54.1 (libsqlite3-sys 0.39.1, Cargo.lock).

### DOC-043 [medium] SQLite page cache size differs between `OPEN_ITEMS` and the scan plan — CONFIRMED

**Disposition: Fixed.** The scan plan's two references say 8 MB, the `cache_size` that `SETTINGS` sets.

schema.rs:114-117 sets PRAGMA `cache_size` = -8192 (8 MB) in SETTINGS, as `OPEN_ITEMS.md:57` says. `SCAN_OPTIMIZATION_PLAN.md:451` and :802 still say each connection has a 256 MB cache from SETTINGS, which is the old premise.

### DOC-044 [medium] Notification plan decision count and cache event counts disagree within the docs — CONFIRMED

**Disposition: Fixed.** The plan's waiting-on-a-person row says D1 to D12; the design lists all seven cache events.

`NOTIFICATIONS_PLAN.md:10` says D1 to D12 are unanswered, and :24 says waiting on a person covers D1 to D8. `NOTIFICATIONS_DESIGN.md:13` says the cache has seven events, and :230 says five. thumbcache types.rs Event has at least seven variants (lines 68-85), so :13 is right and :230 is stale.

### DOC-045 [medium] `NOTIFICATIONS_DESIGN` contradicts the code and itself on derivation and schema — PARTIAL

**Disposition: Fixed.** The `strum` claim is marked as overridden by the `ids!` decision, and the `TO_3` draft is marked as superseded.

`NOTIFICATIONS_DESIGN.md:241` says code() comes from strum::IntoStaticStr. :94 overrides that to the ids! table, and notice.rs has no IntoStaticStr, so the code follows :94. The `TO_3` SQL at :251 (scope TEXT, AFTER DELETE trigger) is stale, but :29 overrides it, and `folder_failures.root` has ON DELETE CASCADE (baseline.sql:164).

### DOC-046 [medium] `Pdq_index.rs` cited as current code after it was removed — CONFIRMED

**Disposition: Fixed.** The plans that cited `pdq_index.rs` as current now say it is removed and point at `dedup/src/join.rs`.

`crates/dedup/src/pdq_index.rs` does not exist (dedup/src has join.rs, lib.rs, `union_find.rs`). `UROCISSA_COMPARISON.md:120` cites it as current, `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:137` cites `pdq_index.rs:323`, and crates/README.md still mentions it. `OPEN_ITEMS.md:63` records it as gone.

### DOC-047 [medium] Review and plan status lines contradict their own counts or ledger — CONFIRMED

**Disposition: Fixed.** `MULTI_DRIVE_LANES_PLAN` says `tests/lanes.rs` has 9 tests (9 `#[test]` attributes), in both places.

`MULTI_DRIVE_LANES_PLAN.md:14` says tests/lanes.rs has 8 tests, and :400 says 7 of 12 are written. crates/library/tests/lanes.rs has 9 #[test] attributes. The plan contradicts itself and the file. The LANGUAGES.md HEAD dates are dated history, not a contradiction.

### DOC-048 [medium] LANGUAGES plan links to documents that do not exist; compare-shots script name is stale — REFUTED

README.md, site/index.html and docs/plans/PLAN.md all exist at HEAD. LANGUAGES.md:155 already marks the compare-shots.ps1 reference stale, and `compare_shots.py:4` records that the name changed.

### DOC-049 [medium] WSL status described inconsistently across reference docs and the Docker README — CONFIRMED

**Disposition: Fixed.** WSL is retired everywhere: the Docker README no longer calls it an exception, and the JPEG decoder note names `wsl.sh` as removed.

`REFACTOR_DESIGN.md:20` says WSL is retired and wsl-check.sh is gone. No `tools/wsl*` or wsl.sh file exists. `JPEG_DECODERS.md:119` says wall times are medians on WSL via wsl.sh. docker/README.md:71 still calls WSL the one exception for S-17.7.

### DOC-050 [medium] `REFACTOR_LEDGER` describes the baseline differently from baseline.json; callgrind note bench count differs — CONFIRMED

**Disposition: Fixed.** The ledger's current-state figure is 23 results (`baseline.json` has 23 entries); the callgrind note was fixed under DOC-009.

docs/perf/baseline.json has 23 entries, all pg-bench keys. `REFACTOR_LEDGER.md:131` says it has 11 results, and the callgrind md title says the 11 benches. The count is stale in both.

### DOC-051 [medium] Web client design docs disagree with `WEB_CLIENT.md` on settled points — CONFIRMED

**Disposition: Fixed.** `WEB_CLIENT.md` says the private CA certificate is planned, not configured; the repo has no certificate code.

`WEB_CLIENT.md:272`, :455 and :564 say the private CA certificate is already configured. `WEB_CLIENT_DESIGN.md:231` says the repo has no certificate or TLS server code, and no CA tooling exists in crates. The design doc's contradiction list is correct.

### DOC-052 [medium] `WEB_CLIENT.md` keeps superseded listing and snapshot figures — CONFIRMED

**Disposition: Fixed.** The 56-byte figure and the four-snapshot cap are marked superseded by `WEB_CLIENT_RECONCILED.md`.

`WEB_CLIENT_RECONCILED.md:1031` voids the four-listing cap and the 256 MB budget. `WEB_CLIENT.md:212` and :221 still state the 56-byte figure and the four-snapshot cap. DESIGN.md:1859 still calls 256 MB a proposal, so the superseded numbers persist.

### DOC-053 [medium] Reference docs disagree on JPEG encoder and thumbnail numbers — PARTIAL

**Disposition: Fixed.** `JPEG_ENCODERS.md` says ImageReduce's encode is libvips' `jpegsave`; whether that build uses mozjpeg is not recorded.

`JPEG_ENCODERS.md:13-14` says ImageReduce runs mozjpeg in `JCP_FASTEST` mode. :111-112 says its encode is libvips 8.18.7 jpegsave. libvips jpegsave can use mozjpeg when built with it, and the doc does not say which. The naming conflict is real, but not proven as an error.

### DOC-054 [medium] Reference index and cross-references are incomplete or point at the wrong file — CONFIRMED

**Disposition: Already correct.** The references the audit names (`docs/THUMBNAIL_RESAMPLING.md`, `docs/TIER2_ANN_BAKEOFF_PLAN.md`) are not in `REFERENCES_LEDGER.md` now; checked by grep.

`REFERENCES_LEDGER.md:78` names `docs/THUMBNAIL_RESAMPLING.md` and :44 names `docs/TIER2_ANN_BAKEOFF_PLAN.md`. Neither exists; the real files are under docs/plans/. :140 and :133 use the correct plans/ paths, so the ledger is inconsistent.

### DOC-055 [medium] Eval harness docs disagree with the code they describe (method, precision and timing) — CONFIRMED

**Disposition: Fixed.** `bench_decode.py` reads every file once before timing, as its docstring says; `exact.py`'s docstring states the fp16 ground truth that `run.py` uses.

`bench_decode.py:14` says files are read once before timing, so it measures decode, not disk. The timed loop at :167-169 calls decode(), which calls `read_bytes` at :57, so each timed call reads from disk. exact.py:3 says the ground truth is fp32, but `bench_ann.py:10-11` says the exact search, which is the ground truth, is fp16 with fp32 accumulation, and run.py:154 uses fp16=True.

### DOC-056 [medium] Eval scripts cite Rust files and crates that do not exist in the workspace — CONFIRMED

**Disposition: Fixed.** `eval/common.py` and `eval/embed.py` cite `crates/analysis/src/analyze.rs`.

crates/core does not exist. The analyze decode rule is in crates/analysis/src/analyze.rs. eval/common.py:70 and eval/embed.py:19 cite crates/core/src/analyze.rs. The imgload.py:44 cite (crates/analysis/src/borders.rs) does exist.

### DOC-057 [medium] PNG decoder README gives a timing budget and dump file names the code does not use — PARTIAL

**Disposition: No change.** The README's dump names match the app's dump; the 5 s budget is advice that overrides the 1 s default.

The PNG README:67 advises at least 5 s for inflate-stream numbers. The harness defaults (harness.cpp:90, main.rs:80) are 1.0 s, which the advice overrides with --secs. That is advice versus default, not a contradiction. README:60 names DIR/NAME.raw for the app's dump, which matches main.rs:120, but the C harness writes `NAME.<decoder>.raw` (harness.cpp:482).

### DOC-058 [medium] JPEG decoder harness docs contradict harness code on output layout, warm-up and command — PARTIAL

**Disposition: Fixed.** The `dec_wuffs.cpp` comment says 4-byte RGBX pixels (`RGBA_NONPREMUL`, no alpha used).

`dec_wuffs.cpp:1` says decodes to RGBA, but it sets Image::RGBX (:45) and names the format `RGBA_NONPREMUL` (:21). The layout is a 4-byte pixel with no real alpha, so the word is loose. harness.cpp:14-15 documents the GPU warm-up of 5 (:147, :233), so the Rust-versus-C difference is stated. The restart.py cite is about the Rust binary, as its docstring says.

### DOC-059 [medium] Profiling tool docs name the wrong build script and removed timing paths — CONFIRMED

**Disposition: Fixed.** `build.sh` is deleted (`build.py` replaces it, and `profile.ps1` names it); `scan_timing.py`'s note says the S-17.7 plan was removed while the step stays.

profile.ps1:29 says run build.sh first, while profile.py:58 says build.py. build.sh is still in the tree, though build.py:3 says it replaced build.sh. `scan_timing.py:5-6` says renumbered was removed on 2026-11-14, and :113 still runs it.

### DOC-060 [medium] Peak memory script documents peak commit but samples a different counter — CONFIRMED

**Disposition: Fixed.** `peak_mem.ps1` samples `PrivateMemorySize64` (private bytes, the commit it documents).

`peak_mem.ps1:3` says peak commit (private bytes). :47 samples PeakPagedMemorySize64, which is not the private-bytes counter, and :52 reports it as peak commit.

### DOC-061 [medium] tools/README describes the static Qt build and `compare_shots` options incorrectly — CONFIRMED

**Disposition: Fixed.** `tools/README.md` says `build_static_qt.py` has no patch step, so the patches are applied with `git apply` first.

tools/README.md:57 says the static-qt patches are applied by the static Qt build. No file in tools or the Justfile names app-build.patch or pg-video.patch, and `build_static_qt.py` has no patch step (its functions are extract, configure, build, install). The `compare_shots` --wait option is present at `compare_shots.py:653`; the README row is not a full option list.

### DOC-062 [medium] eval verifier and test-set counts disagree across sections — CONFIRMED

**Disposition: Fixed.** `VERIFIER.md` uses 8,344 k-NN pairs throughout; the 678-group line in `TEST_SET.md` is marked superseded by its dated 679-group entry.

VERIFIER.md gives cross-group pairs as 4637 (:10, :24), 4628 (:161), 4632 (:216, :397, :428), and k-NN as 8450 (:11) and 8,344 (:61, :220, :329) in the same doc. `TEST_SET.md:27` says 678 groups and 702 pairs after fixes, and :36 says 679 and 698 now, with the old line not marked superseded.

### DOC-063 [medium] Desktop QML comments disagree with code, keys or sibling files — CONFIRMED

**Disposition: Fixed.** The panels' `onClosed` handlers call `win.refocus()`, which focuses the viewer when it is shown; `video_source.h` names `src/video_source.rs`. The `R` and `Shift+R` turn keys are bound in `client/src/keys.rs`, as the rating bar says.

Main.qml:697 LibraryPanel refocuses with grid.takeFocus(), while Main.qml:813 and :847 use win.refocus() for Log and Inbox. `video_source.h:1` names `src/video_player.rs` for the device, but MediaView.qml:55 and the attach impl (`video_source.rs:23`) name `video_source.rs`. The turn keys (R, Shift+R) in RatingBar.qml:2 were not found in keymap.rs.

### DOC-064 [medium] Grid revision doc does not cover refreshes that change entries — CONFIRMED

**Disposition: Fixed.** Code: `refresh_turned` bumps the grid's revision when it changes an entry, so a read made before the change is refused, as the doc requires.

grid.rs:138-139 says revision counts list shown and item edited. `refresh_turned` (grid.rs:405-431) changes entries and never assigns self.revision, so the doc misses a change that the revision check depends on.

### DOC-065 [medium] Temporary file naming described differently in the app and the code — PARTIAL

**Disposition: No change.** Two writers use two conventions; this is not a doc contradiction.

save.rs:47-48 says repository temporaries are dot-names (.pg-write, .pg-crop), which matches the queue temps (queue.rs:562 .b.jpg.turns.pg-write). store.rs:115 writes `<name>.tmp`, a different writer that does not claim dot-names. Two conventions across different writers, not a doc contradiction.

### DOC-066 [medium] Versions doc names the per-crate constants and `pg_library::versions` instead of `pg_model::versions` — PARTIAL

**Disposition: No change.** Naming the owning crates' re-exports is acceptable.

types.rs:277 names `analyze::IMPORT_VERSION`, `borders::BORDERS_VERSION` and `pg_render::THUMBS_VERSION`. Those are the owning crates' re-exports (render lib.rs:14). The code imports from `pg_model::versions` (types.rs:291), as the registry doc at versions.rs:1-4 prescribes. Naming the re-export is acceptable.

### DOC-067 [medium] Version comment names IMPORT as 1 literally in fixture doc — PARTIAL

**Disposition: Fixed.** `fixture/raw.rs` doc names `IMPORT_VERSION` instead of the literal 1.

fixture/raw.rs:180 says rows are at import version 1. versions.rs:13 has `IMPORT_VERSION` = 1 today, and raw.rs binds the constant (:229). The doc is right now but written as a literal that will go stale on a bump.

### DOC-068 [medium] Library runner and scan doc details disagree with code — CONFIRMED

**Disposition: Fixed.** `scans.rs` names the one key per root (`Key::all(scan-note, root)`); the scan's result doc names `pairing`'s replaced rows and temp files.

scans.rs:19 says conditions are kept under (root, code). passes.rs:108 replaces them all under one Key::all(scan-note, root), so there is no per-code key. scan/lib.rs:270 names `replaced_ids` and temps as fields, and neither exists in that file.

### DOC-069 [low] Writeback test comment says PNG while the test queues a JPEG — PARTIAL

**Disposition: Fixed.** The test comment says the poisoned JPEG is queued beside the PNG.

writeback.rs:1013 says a PNG is rewritten without ExifTool. The test that follows queues the healthy b.png and the poisoned a.jpg (:1014). The comment describes one file and omits the JPEG. Incomplete, not wrong.

### DOC-070 [low] CutStats.unsupported is documented but never set — REFUTED

crop lib.rs:167-169 says unsupported stays 0 and is kept for the report, because every still format is cut now. The field is declared and never set, which is what the doc says.

### DOC-071 [low] Video FFI loader path cited differently in two docs — CONFIRMED

**Disposition: Fixed.** The Justfile cites `crates/video/src/ffi/load.rs`, where the DLLs are loaded.

crates/README.md:240 cites crates/video/src/ffi/load.rs, which exists and is where the DLLs are loaded. Justfile:69-70 says pg-video loads its DLLs by full path in crates/video/src/ffi.rs, the module root, so the Justfile points at the wrong file.

### DOC-072 [low] Measured timings in doc comments that the code or benches do not reproduce — PARTIAL

**Disposition: Deferred.** Timings need their source attached (run date, machine, bench) and reproduction on target hardware.

The timing claims (modgraph 115 to 149 s, the HDD order speedup, the cold-read figures, the 0.8 s pairs pass) are measurements recorded in comments. They cannot be reproduced from the repo here, so they are unverified, not contradicted.

### DOC-073 [low] Crop, edit and cut docs use inconsistent function and crate paths — PARTIAL

**Disposition: No change.** `pg_edit::crop::cut` exists (`crop.rs:75`), and the `encode.rs` cites are correct.

encode.rs:1-3 names `pg_edit::crop::cut`. The crop module exists in crates/edit, but the cut function was not confirmed in that file. `WEBP_LOSSY_QUALITY` = 90 (encode.rs:96) matches its doc. The path claim is only partly checked.

### DOC-074 [low] Stale or undefined references in the probe and bench doc comments — REFUTED

`pairs_probe.rs:18` lists the prepare field, and the printed line uses t.joins.prepare (:100), which is the same field. `tools/profiling/pairing/set_owed.py` and `tools/profiling/synthetic_library.py` both exist at the cited paths.

### DOC-075 [low] Eval and ANN docs give wrong CLI forms and pair set lists — CONFIRMED

**Disposition: Fixed.** `verify.py`'s usage line names `listed` and `--list`, as its argument parser has them.

eval/verify.py:3 gives the usage as labeled, `cross_groups` or knn. `PAIR_SETS` at :99-105 also has listed, which eval/ann/run.py:445-447 uses. The usage line is stale. The viewer.py glob point is loose wording.

### DOC-076 [low] Eval JPEG encoder and decoder comments name the wrong counts, stages and venvs — PARTIAL

**Disposition: Fixed.** `eval/jpeg-encoder/README.md` names the diagnostic test 8 alongside tests 1 to 7.

README:16 says `test_encoder.cpp` has tests 1 to 7, and README:97 documents test 8 as a diagnostic outside pass or fail, so the two are consistent. The `encode_gpu.cu` header list was read only through stage 6. enum Stage (:543) has `ST_COPY`, and the header's full list was not confirmed.

### DOC-077 [low] Tools doc comments name the wrong OS scope, paths or Docker wording — CONFIRMED

**Disposition: Fixed.** `lint_qml.py` names every OS and the `target/*/build` glob; `fuzz_smoke.py` says a clone of HEAD; `few_sweep.ps1` says busy runs are left out of the medians.

`lint_qml.py:2` says just lint-qml on Linux and macOS, while :14 says every OS. :55 says the output is `build/pg-desktop-<hash>/`, but :57 globs `target/*/build`. `fuzz_smoke.py:3` says the fuzzed tree is a git archive of HEAD, but linux.py:187 uses git clone. `few_sweep.ps1:9` says busy runs are not left out, while sweep.ps1:8-9 and :66 leave them out.

### DOC-078 [low] Site and tools docs point at files that are missing or mislabelled — PARTIAL

**Disposition: Fixed.** The `site/index.html` hero icon attributes are 160, matching the CSS.

The hero-icon width is set to 160px in style.css:73, which overrides the 256 attributes in index.html:20, so the rendered size is the CSS. The screenshot comment (index.html:35) is commented out on purpose. pages.yml:39-41 does copy the icon and mascots, as index.html:3 says. The `tools/test_ci_files.py` docstring was not checked.

### DOC-079 [low] Desktop Rust doc comments name wrong symbols, counts or rows — CONFIRMED

**Disposition: Fixed.** `notices.rs` says `providers.rs` (3) for its three warning calls.

notices.rs:73 gives the providers.rs row as (4), but providers.rs has three `pg_log::warn` calls. The `resolver_table.golden` path resolves under crates/model/src/notice/, so that instance is fine.

### DOC-080 [low] QML viewer and gallery comments with stale values or misplaced text — CONFIRMED

**Disposition: Fixed.** The log view's comment sits above its `ui-log` entries in `ui.ftl`. The `YandexPreview.qml` comment already sits above `item`, so it needed no change.

ui.ftl:91-92: the log view comment is followed by the inbox comment, with no log entry under it. YandexPreview.qml:26-28: the results comment sits above the dpr property. Main.qml:203 says the cut dialog reports as SaveViewDialog, which is correct. `MAX_ZOOM` = 32 is in client viewport.rs:6, so the zoom comment is right. Gallery and `RATING_SYMBOLS` references resolve. The state.rs query-versus-media MediaType claim was not confirmed.

### DOC-081 [low] Client text and controller doc comments with stale details — PARTIAL

**Disposition: No change.** The cited `deadlines.rs` exists, and the other instances were not contradicted.

controller/size.rs:1 says pause is timed by deadlines.rs, which exists in crates/client/src/controller, so the file is in the repo. The other instances (tests facets, start.rs:2 on the session's last file) were not contradicted. Some were not read in full.

### DOC-082 [low] Yandex and shell state names differ from the code — CONFIRMED

**Disposition: Fixed.** The frames doc says "by its index in `results`", the field the struct has.

yandex/state.rs:37-40 has results and frames. The frames doc says indexed by hits, but no hits field exists in that struct. The name is stale.

### DOC-083 [low] Start gate doc on the shown field matches the code (no contrast found) — REFUTED

start.rs:44 says the shown flag means a list was shown and its placement is not yet reported. That matches the flag's use. The list flag's doc at :42 is accurate. No contradiction found.

### DOC-084 [low] Container and codec docs claim behaviour the functions do not have — CONFIRMED

**Disposition: Fixed.** `add_extended_chunk` is documented as adding to `chunks`, and the module doc says PNG walks yield chunk values.

container lib.rs:209 documents `add_extended_chunk` as returning the APP1 data, but the signature (:210) returns nothing and pushes into chunks. lib.rs:3-4 says each walk yields byte ranges, but `png_chunks` (:233) yields Chunk values with slice data. decode.rs:263-264 and encode.rs:431 are accurate.

### DOC-085 [low] Model and metadata doc comments name the wrong module, range or count — CONFIRMED

**Disposition: Fixed.** `poster_ms` is `None` before the import, and the EXIF doc no longer names an unused function.

model state.rs:33 says `poster_ms` is 0 before import, but the field is `Option<i64>`, so no import is None. metadata exif.rs:10 says the format rules are kamadak-exif's `DateTime::from_ascii`, and grep finds no call to `from_ascii` in metadata. The detection black/white bounds (`<=` 40, `>=` 215) match detection.rs:82-88. The square tolerance was not located.

### DOC-086 [low] Third-party product named in project docs and comments — CONFIRMED

**Disposition: Fixed.** The product name is replaced with "a rating app" wherever it appeared in project docs and comments (about 20 places).

ImageRater is named in code docs: app views.rs:2, prefs.rs:29, model view.rs:28 and exiftool lib.rs:129-130. It is a style and naming choice, not a contradiction. Low severity.

### DOC-087 [low] versions.rs `key_version` mark naming rule does not hold for `KEY_ALGO_VERSION` — CONFIRMED

**Disposition: Fixed.** The `versions.rs` header states `KEY_ALGO_VERSION` as the one exception to the naming rule.

versions.rs:1-3 says each name is the upper-cased mark. `KEY_ALGO_VERSION` (:47-50) is marked `key_version`, so the stated rule has an exception that the header does not list, though its own doc explains it.

### DOC-088 [low] Plan D9 and phase cross-references in code docs point at plans that do not give those numbers — PARTIAL

**Disposition: Fixed.** The "plan D9" cites are removed; no plan in the tree numbers the cache rule D9.

versions.rs:32 and :44 cite plan D9 for the cache and thumbnail rules. thumbcache schema.rs:113 cites plan D9 and notes no plan file is in the shard. Whether D9 is numbered that way in the plans was not checked.

### DOC-089 [low] Notices reference and catalog docs describe index names and fields that do not match — PARTIAL

**Disposition: Fixed.** The `lookup.rs` doc says SQLite picks an index where one covers a query, with no claim about specific indexes.

lookup.rs:195 names `files_present`, `files_to_import` and `analysis_failed`. Those indexes exist in baseline.sql (:215, :247, :253). Whether status() reads `files_present` was not confirmed. The `kept_events` value (1000) and the rebuild.rs module-doc list were not fully checked.

### DOC-090 [low] Runner, coordinator and lane docs name probes and tools that do not match code — PARTIAL

**Disposition: No change.** `Runner::cut_borders` exists (`runner.rs:112`).

inner.rs:39 cites `lane::BUSY_RETRY`, which lane.rs:26 defines, so that cite is right. coordinator.rs:107-108 says cuts are queued once confirmed via `Runner::cut_borders`; `cut_borders` was not found in library/src beyond that doc. The other lane and events cites were not fully checked.

### DOC-091 [low] Stub and test docs claim parity with Rust that is not tested in the cited place — PARTIAL

**Disposition: No change.** The parity tests exist (`client/src/text/cli.rs:584` and `text/message.rs:307`).

The locale files (cli.ftl:1, text.ftl:1, ui.ftl:3) say a test checks that the ids match the Rust lists. Client src references the ftl files, but the named parity tests were not located in full. Unverified as stated.

### DOC-092 [low] Callgrind and perf narrative numbers in reference docs disagree with each other — PARTIAL

**Disposition: No change.** The three counts come from different snapshots, and the docs label them.

PROFILING.md:320 says 189,518 files (2026-10-31 snapshot), :507 says 189,520 (the pairing data), `REFACTOR_MEASUREMENTS.md:120` says 189,519. The dates differ, so the library may have changed between runs. They are not proven to describe the same folder on the same day.

### DOC-093 [low] Search and crop limits in the desktop app do not match documented bounds — PARTIAL

**Disposition: No change.** `R` and `Shift+R` turn images (`client/src/keys.rs:115, 132`), so the claim holds.

shell/state.rs:19 says the row bounds are `pg_model::size::ROW_HEIGHT`, which matches RowHeight::MIN and MAX built from it (size.rs:10-23), as recorded for A-027. The rotate.rs:1 claim that R and Shift+R turn images was not found in keymap.rs, so that instance is unverified.

## duplication

### D-001 [high] Folder subtree membership computed two ways — CONFIRMED

**Disposition: Fixed.** One folder is one row: `folders.path_key` (the spelling under the path rules, case-folded on Windows, with a terminator per part) is UNIQUE, and `folders.match_key` (the drive-free key the files' order keys follow) is kept beside it. A subtree is the range `path_key >= K AND (upper is NULL OR path_key < upper)`, which `removal::folder_ids_under` uses and which a test holds equal to the scan with `is_under`, over mixed-case and prefix-related folders on both rule sets. `folder_failures` is one row per `path_key` too. A carried catalog (the old layout, which keyed a folder by its exact path) merges the spellings the new key makes one, keeping the lowest id and re-pointing their files (`lifecycle::recompute_keys`, `rebuild::copy_folder_rows`). Not unified, by the decision: a re-lettered drive or a verbatim volume spelling (a different anchor), and NTFS's case table where it differs from Rust's (D-078).

removal.rs:351 `folder_ids_under` filters every folder in Rust with `rules.is_under` (rules-aware, so case-insensitive under Windows rules). types.rs:90 `folders_under` selects by byte range through `under_root!`. Two selections of the same subtree with different matching rules.

### D-002 [medium] Lane planning rules split between coordinator and lanes — CONFIRMED

**Disposition: Fixed.** `lanes::lane_volumes` and `lanes::drive_label` are the one rule each; the planner and the coordinator both call them. The label takes the first root on a mounted drive, the coordinator's rule.

coordinator.rs:484 and lanes.rs:91 both apply the single-lane rule (one lane gets VolumeSet::All, otherwise Only(volumes)). coordinator.rs:858 `drive_label` and lanes.rs:82 both name a group by its first mounted root's drive. The lane rules are split across two modules.

### D-003 [medium] CPU count and budget defaults repeated — CONFIRMED

**Disposition: Fixed.** `jobs::pipeline::cpus()` and `pipeline::READERS` are the one definitions; the three pools and the budget use them.

`available_parallelism().map_or(4`, ...) appears at pipeline.rs:53, coordinator.rs:173 and lanes.rs:129. coordinator.rs:178 repeats Options::budget(8, cpus) with the reader count of 8 that pipeline.rs:55 sets in Options::new.

### D-004 [medium] Pause clock bookkeeping implemented twice — CONFIRMED

**Disposition: Fixed.** `Running` holds a `PauseClock`, the one pause bookkeeping, with `total_at` for the time paused at a moment.

control.rs:82 `Running::set_paused` and control.rs:228 PauseClock::set implement the same pause bookkeeping with the same (true, None) and repeat-changes-nothing match arms.

### D-005 [medium] Decode scale rule re-derived in codec and video — REFUTED

video lib.rs:272 calls `MinSize::fit_scale` for the Fit case, so the video crate delegates to the shared rule and does not re-derive it. `scale_for` only adds the clamp to 1 and the other MinSize variants.

### D-006 [medium] Resize computes fit by hand instead of `fit_scale` — CONFIRMED

**Disposition: Fixed.** `fit_within` takes its scale from `MinSize::fit_scale`, and `to_pixels` is defined once, in `decode.rs`.

resize.rs:33 computes side / long by hand, while decode.rs:79-80 says `fit_scale` is the one fit the resizer uses. `to_pixels` is defined at decode.rs:259 and again at resize.rs:125 with the same body.

### D-007 [medium] Atomic file replace implemented twice with different temp naming — CONFIRMED

**Disposition: Fixed.** `pg_platform::file::write_synced` is the one create-write-sync step; the settings replace and the Yandex claimed save both use it. Each keeps its own temporary name, per PF-011.

yandex/save.rs:63 `write_new` replaces by rename with a dot-name temp (see PF-011). app/src/store.rs:117 `write_atomically` writes `<name>.tmp`, syncs and renames. `REFACTOR_LEDGER.md:73` (F-22) records the private helper as a shortcut to move to pg-platform, still open. The cite store.rs:118 is off by one; the function is at :117.

### D-008 [medium] Failure classification repeated in thumbs — CONFIRMED

**Disposition: Fixed.** `is_unreadable` and `is_recorded` in `placeholders.rs` are `Failed::of` matches, so the classification is in one place.

thumbs lib.rs:112 Failed::of maps MakeError to Failed. placeholders.rs:164 `is_unreadable` and placeholders.rs:177 `is_recorded` each repeat the MakeError and panic checks to classify the same failure.

### D-009 [medium] io::Error downcast sites to be replaced by Unreadable — CONFIRMED

**Disposition: Fixed.** `pg_jobs::failure::is_io_read` is the one read-failure test; the borders, import and thumbs classifiers call it. (The thumbs cite at `lib.rs:113` was a `MakeError` match, not a downcast; it is unchanged.)

io::Error downcasts appear at borders lib.rs:97 and import hashes.rs:114, and placeholders.rs:171 falls back to `downcast_ref::<io::Error>`. `IMPORT_FAILURES_PLAN.md:198` names these as the copies to replace with Unreadable. The cite thumbs lib.rs:113 is a MakeError match, not an io::Error downcast, so that one cite is wrong.

### D-010 [medium] Import batch size 500 mirrored across crates — CONFIRMED

**Disposition: Fixed.** `pg_catalog::WRITE_BATCH` is the one batch size; the import, borders and bench use it.

const BATCH: usize = 500 is defined at import hashes.rs:17, import lib.rs:25 and borders lib.rs:22. The bench catalog.rs:174 `IMPORT_BATCH` = 500 mirrors pg-import's batch by hand, so a change in the import is not picked up by the bench.

### D-011 [medium] Thumbnail and row height default 220 in two types — CONFIRMED

**Disposition: Fixed.** `ThumbHeight::DEFAULT` is `RowHeight::DEFAULT`'s value, so the default is one fact.

RowHeight::DEFAULT = RowHeight(220) at model size.rs:25 and ThumbHeight::DEFAULT = ThumbHeight(220) at size.rs:69. ThumbHeight's doc says it is the default row height at scale 1, so the two defaults are one fact written twice.

### D-012 [medium] Verbatim-prefix stripping in three platform modules — CONFIRMED

**Disposition: Fixed.** The picker's shell path uses `native::VERBATIM` and `native::UNC_AFTER`, the one set of units. `names.rs` strips the prefix from bytes in another encoding, so it stays separate.

The verbatim prefix is stripped or defined in four places: platform names.rs:127 `without_verbatim`, native.rs:265 `plain_in_place`, picker/shell.rs:33 `shell_path` with its own VERBATIM and UNC unit arrays (:34-35), and native.rs:185 with a VERBATIM constant of the same units.

### D-013 [medium] CreateFileW sharing modes differ across opens — PARTIAL

**Disposition: No change.** Each open's sharing mode is a deliberate per-open choice.

The CreateFileW sharing combinations differ: file.rs:37 uses READ|WRITE|DELETE, dir.rs:366 the same, drive.rs:293 READ|WRITE, and file.rs:89 READ alone. Each open has its own purpose, and no shared constant exists, but whether these are duplicates or deliberate per-open choices was not settled from the lines read.

### D-014 [medium] XMP namespace URI written twice — CONFIRMED

**Disposition: Fixed.** The URI is `xmp::NS_XMP` alone; `container::JPEG_XMP` is its NUL-terminated byte form, built from it at compile time.

The XMP namespace is written as bytes with a NUL at container.rs:17 (`JPEG_XMP`) and as a &str at xmp.rs:13 (`NS_XMP`). Both are the same string in the same crate.

### D-015 [medium] BigTIFF magic rule in three places — CONFIRMED

**Disposition: Fixed.** The codec's test helper is gone; its assertions use `pg_model::format::is_bigtiff`. The metadata parser reads the same header fields inside its own header walk.

The BigTIFF header rule (43, offset size 8, zero) is in model format.rs:53 `is_bigtiff` and metadata tiff.rs:43-45. codec encode.rs:316 has a test helper `is_big` matching the same header bytes.

### D-016 [medium] JPEG marker walk duplicated in container — CONFIRMED

**Disposition: Fixed.** `marker_kind` classifies a marker once (fill, standalone, end, segment); the segment walk and the header walk both use it.

container lib.rs:70 JpegSegments::next and lib.rs:138 `jpeg_header_end` both skip fill bytes and reject bad segment lengths with the same logic in two loops.

### D-017 [medium] YYYY-MM-DD parsed in three places — PARTIAL

**Disposition: Fixed.** `Day::parse` delegates to `parse_iso_date` after its shape check. The EXIF date is a different format.

model date.rs:9 `parse_iso_date` and date.rs:64 Day::parse both parse YYYY-MM-DD with range checks in the same file, which is a real duplication. metadata exif.rs:11 `exif_date` parses the colon format YYYY:MM:DD, a different format, so it is not a third copy of the same parse.

### D-018 [medium] Unchanged-source check written twice in crop — CONFIRMED

**Disposition: Fixed.** `still_listed` is the one size-and-mtime check; both writes call it.

crop lib.rs:300 `cut_original` compares the file's (size, mtime) with the recorded state, and lib.rs:368 `write_copy` repeats the same (s.size, s.mtime) comparison. The bytes comparison in the cite was not seen; the size and mtime check is repeated.

### D-019 [medium] Width dispatch repeated in dedup join — CONFIRMED

**Disposition: Fixed.** The four width matches are one `by_head_words!` macro, with the same arms and fallback.

dedup join.rs has four width dispatches over 1, 2 and so on: `first_pass` (:975), `second_pass` (:989), scan (:2031) and `scan_width` (:2043), each matching a width to a const-generic function.

### D-020 [medium] Catalog predicate present and not pending repeated — CONFIRMED

**Disposition: Fixed.** `present_ready!(alias)` is the one rule; the owed queries and the owed-work SQL build from it.

The predicate 'f.missing = 0 AND f.pending = 0' is written inline at work.rs:426 (OWED), :143 (`OWED_PLACEHOLDERS`), :526 (`owed_hashes`) and :565 (`has_owed_hashes`).

### D-021 [medium] `file_writes` and `scan_seen` predicates inline while macros exist — CONFIRMED

**Disposition: Fixed.** `PAGE`, the two `roots.rs` statements and the scan's reads use the existing `not_being_written!` and `not_seen!` macros.

removal.rs:74 writes the `file_writes` NOT EXISTS inline, and roots.rs:184 and :204 write the `temp.scan_seen` NOT EXISTS inline. sql.rs:106 (`not_being_written!`) and sql.rs:113 (`not_seen!`) already exist as the shared forms.

### D-022 [medium] Overlap predicate written inline three times — CONFIRMED

**Disposition: Fixed.** `removal::overlaps` is the one overlap test; all three sites call it.

The overlap test (same || `is_under` || `is_under`) is written inline at removal.rs:168, and again in `overlapping_marker` (:383) and `overlapping_root` (:391).

### D-023 [medium] `borders_version` lookup repeated — CONFIRMED

**Disposition: Fixed.** The borders writer's mark and `pass_mark_is_ours` use the one `META_VALUE` query, bound with the key.

writers.rs:720 BordersWriter.mark selects value from meta WHERE key = '`borders_version'`, and `pass_mark_is_ours` at :821-822 reads the same row.

### D-024 [medium] Touch-after-analysis logic written twice — CONFIRMED

**Disposition: Fixed.** Both paths run the same two statements (`BUMP_SEQ`, `TOUCH_FILE`); the test helper has a connection, not the writer's transaction, so it keeps its two calls.

writers.rs:339 `touch_analysis` (test and fixture only) and writers.rs:429 `AnalysisWriter::touch_file` each do the same bump-and-touch of a file after an analysis row. The test helper says it mirrors the writer's.

### D-025 [medium] Retry statements repeated between ledger and lifecycle — CONFIRMED

**Disposition: Fixed.** `ledger::relist_failures` runs the sequence bump and the row delete for both; each caller supplies its own listing, since the version change lists only the panicked files.

ledger.rs:207-209 and lifecycle.rs:399-401 run the same `next_seq`, `failures_relisted` and DELETE FROM `file_failures` statements for the retry.

### D-026 [medium] Thumbs filter and `pending_borders` SQL copied into test helper — CONFIRMED

**Disposition: Fixed.** The statement is `borders_statement(from)`, in production; `pending_borders` runs it and the test compares it, so the test helper's copy is gone. The Thumbs failure filter is `sql::thumbs_not_failed!`, used there and in the owed placeholders (`OWED_PLACEHOLDERS`), so the three copies are one.

The thumbs-pass filter (NOT EXISTS `file_failures` ... pass = 'thumbs' ...) is written at work.rs:145, :463 and :654 (the test helper `borders_statement`), with the same body each time.

### D-027 [medium] VOLUMES statement shared with thumbcache by comment only — CONFIRMED

**Disposition: Fixed.** The statement is `pg_query::volumes_sql!`, one macro over a table name; the catalog and the thumbnail cache both expand it. `pg-thumbcache` now depends on `pg-query` (in `tools/crate-layers.txt`).

work.rs:19-21 says thumbcache's `catalog_volumes` is 'the same statement'. thumbcache reader.rs:123 defines `catalog_volumes` separately and reader.rs:74 calls it. The two copies of the SQL are linked only by that comment.

### D-028 [medium] Test catalog builders and versions helpers differ — CONFIRMED

**Disposition: Fixed.** The integration tests' `versions()` wrappers are gone; they call `Versions::current()`, as `pair.rs` does.

Builders differ: pair.rs:582 catalog(rules) uses Versions::current(), `catalog/tests/other_process.rs:12` and rehearsal.rs:21 each define versions() wrapping Versions::current(). The cited app/src/inbox.rs:455 is clear(scope), not a catalog builder, so that cite is wrong.

### D-029 [medium] Writeback tests hard-code attempt cap 5 — CONFIRMED

**Disposition: Fixed.** The writeback tests compare with `pg_catalog::queue::MAX_ATTEMPTS`.

writeback.rs:387 uses attempts() `>=` 5 and :404 asserts attempts() == 5, while queue.rs:32 defines `MAX_ATTEMPTS` = 5 once. The tests hard-code the value the constant defines.

### D-030 [medium] Facet kinds enumerated in four client places — PARTIAL

**Disposition: No change.** Matching one enum in several functions is normal Rust.

input.rs:19 choice, input.rs:31 `key_of`, facets.rs:43 choices and text.rs:84 `choice_label` each match on the facet or choice enum to produce a different result (a value, a key, a list, a label). Matching one enum in four functions is normal Rust; the enum itself is not duplicated. The count of six kinds was not checked.

### D-031 [medium] Tier names mapped in Rust and Fluent — CONFIRMED

**Disposition: Fixed (guarded).** The strings `file` and `pixels` in `cli.rs` are Fluent selector keys (the `[file]` and `*[pixels]` variants of `cli-dups-identical`), not displayed text, so they must match the catalogue, and they do. The unit test beside the line now renders both tiers (`0a identical files`, `0b identical pixels`), so a renamed key fails the test.

client text/cli.rs:449-451 maps Tier::File and Tier::Pixels to the English strings 'file' and 'pixels' in Rust. cli.ftl has no tier words, so these English words are hard-coded beside a Fluent catalogue.

### D-032 [medium] Two different blues for selection in the desktop UI — CONFIRMED

**Disposition: Fixed.** The blues that mean a selection or a chosen row are named tokens in `Theme.qml`: `selectionOutline` (`#4aa3ff`, the grid cell's outline) and `chosenRow` (`#2f5f93`, the chosen row in `InboxPanel` and `LogPanel`, and the window's palette highlight in `Main.qml`). The picked badge's accent `pickedBadge` (`#4a90e2`) is a different role and keeps its own token. Whether the outline and the highlight should be one colour is a design choice; they are now two named values, not two literals.

The selection blue is #4aa3ff at GridCell.qml:133, #2f5f93 at Main.qml:31 (palette highlight), and #4a90e2 for the picked badge at Theme.qml:13. The cited InboxPanel.qml:152 is a Connections block and does not hold #2f5f93, so that cite is wrong. Two or three blues with no token.

### D-033 [medium] Facet and folder kinds as hand-typed integers — CONFIRMED

**Disposition: Fixed.** Each bridge exports its own qenum (ChoiceFacet for FacetChoices, ListFacetKind for Filters); QML list facets carry names, not integers. Bound fields stay field indices, not list facets.

FilterBar.qml:48 declares facetTypes: 0 and the bar passes it on as a plain integer (:126, :157). `REFACTOR_LEDGER.md:122` (F-51) records the hand-typed integers for facet and folder kinds as open.

### D-034 [medium] Lock order named two ways in web client docs — PARTIAL

**Disposition: Fixed.** `WEB_CLIENT_RECONCILED` names the fence mutex as the registry lock.

RECONCILED.md:582 gives the lock order as fence mutex, then device gates, then write transaction. DESIGN.md:369 says the registry lock serialises additions against the fence. The two may be the same lock under two names, or two locks; the text does not settle which, so the naming drift is real but its meaning is unclear.

### D-035 [medium] Sort ordering in two places in `DECODE_ONCE` plan — PARTIAL

**Disposition: No change.** The plan restates one rule; the second ordering is planning text, not a conflict.

`DECODE_ONCE_PLAN.md:873` says `compare_placed` is 'the existing `compare_keys` logic', so the plan treats the two as one rule, and :875 states the tie rule once. The `grid_sql` ORDER BY is a second written ordering in the plan, which is a planning-stage restatement and not an inconsistency by itself.

### D-036 [medium] Windows build scripts exist in cmd and py forms — CONFIRMED

**Disposition: Fixed.** win-build.cmd and win-env.cmd are removed. win_build.py replaces them, and its harness step ran on this machine on 2026-11-21.

eval/jpeg-decoders/win-env.cmd exists and parses nvidia-smi `compute_cap` at :29-30, and `win_env.py:1-5` says it was win-env.cmd, which win-build.cmd called. The .cmd is still in the tree beside the .py, so the same parse is kept in two languages.

### D-037 [medium] Exact k-NN sentinel differs between two implementations — CONFIRMED

**Disposition: Fixed.** `common.NO_SIM` (`-inf`, below every cosine) is the one sentinel; `evaluate.py`, `ann/exact.py`, `bench_ann.py` and `check_groups.py` use it in place of their `-2.0` and `-inf` literals.

eval/evaluate.py:31 uses -2.0 as the self-match sentinel, and eval/ann/exact.py:32 uses -inf for the same self-exclusion in the other k-NN implementation.

### D-038 [medium] `bench_ann` constants copied from the bake-off package — CONFIRMED

**Disposition: Fixed.** `bench_ann.py` imports `FLOOR` and `K` from `ann.data`, where the k-NN rule is defined; the copy is gone.

`eval/bench_ann.py:29` sets FLOOR, K = 0.4, 10 by hand, while eval/ann/data.py:19 defines FLOOR, K = 0.4, 10 with the comment naming the rule. BANDS at `bench_ann.py:194` differs from metrics.py:20 (see U-042).

### D-039 [medium] Library size 70e6 literal in five files — CONFIRMED

**Disposition: Fixed.** `common.LIBRARY_SIZE = 70e6` is the one value; the five benchmark scripts import it.

The literal 70e6 appears in `bench_thumbs.py:60` (`LIBRARY_SIZE`), `bench_models.py:42`, `bench_semantic.py:152`, `bench_hdd.py:242` and `bench_ann.py:350`, with no shared constant.

### D-040 [medium] sRGB transfer functions duplicated in Python and Rust — REFUTED

A search for the sRGB constants (0.04045, 0.0031308) across crates finds no Rust transfer function. The Rust counterpart named as crates/codec/src/linear.rs does not exist. The Python `_eotf` and `_oetf` in `eval/linear_resize.py` are the only copy, so there is no duplication.

### D-041 [medium] `DISK_MAX_SIDE` kept equal by comment — CONFIRMED

**Disposition: Fixed.** `DISK_MAX_SIDE` is gone; `common.WORK_SIDE` is the one size, used by the verifier and by the DISK engine's build.

eval/accel.py:246 sets `DISK_MAX_SIDE` = 1024 with the comment '`verify.WORK_SIDE'`, and eval/verify.py:80 sets `WORK_SIDE` = 1024. They are kept equal by a comment, not by an import.

### D-042 [medium] Decoder registry in five places — CONFIRMED

**Disposition: No change.** harness_linux.py DECODERS is the single source for the harness decoders; the other lists name different tools.

The decoder list is written in `harness_linux.py:25-26` (`CPU_DECODERS` and `IR_DECODERS`), linux.py:133 (a separate list), table.py:16 (DECS) and the Rust enum at eval/jpeg-decoders/src/lib.rs:20-22. harness.cpp:65 does not exist, so that cite is dropped.

### D-043 [medium] Wuffs streaming constants repeated in three files — CONFIRMED

**Disposition: Fixed.** The 16 KiB input piece is `kStreamChunk` in `decoder.h`, the harness's shared header; `dec_wuffs.cpp` and `harness.cpp` use it, and their local copies are gone.

The wuffs streaming chunk constant 16 `<<` 10 is written at `eval/png-decoders/harness/dec_wuffs.cpp:180` (kInChunk) and harness.cpp:298 (kInChunk), in two C++ files. The cited harness/main.rs does not exist.

### D-044 [medium] libjpeg encoders use different error handling — CONFIRMED

**Disposition: Fixed.** `libjpeg_encode` uses the same `JerrMgr` and `setjmp` as `libjpeg_encode_rgb`; a libjpeg error now gives an empty result (which `score()` skips) instead of exiting the process.

`test_encoder.cpp:1135` `libjpeg_encode` uses a plain `jpeg_error_mgr` with no longjmp, while :493 `libjpeg_encode_rgb` uses a JerrMgr with setjmp in the same file, so the two encoders handle libjpeg errors differently.

### D-045 [medium] Viewer pair kinds listed twice — PARTIAL

**Disposition: No change.** `VERIFY_TABS` is a subset of `TABS`, not a copy.

`VERIFY_TABS` in viewer.html:160 is a subset of TABS (:148) naming three tabs (verify, `verify_cross`, `verify_knn`), not a copy of the same list. `PAIR_FILES` in viewer.py:34 keys the pair kinds, and TABS in viewer.html names tabs. Whether these overlap beyond the verify tabs was not checked.

### D-046 [medium] DCT draft size and denominator rule in four places — CONFIRMED

**Disposition: Fixed.** `common.ANALYSIS_DRAFT_SIDES` and `common.analysis_denom` are the Python rule; `embed.py`, `pdq.py`, `gpu.py` and `nvjpeg.py` use them. Two things stay separate: `src/lib.rs`'s `denom` mirrors the same rule in Rust (Python cannot import it), and `bench_decode.py`'s 320 × 320 target is a different measurement, not the analysis rule.

The DCT denominator rule with (512, 320) is written at eval/jpeg-decoders/gpu.py:47-49 and nvjpeg.py:203-205, and as `DRAFT_SIDES` at eval/embed.py:20 with a repeated note in common.py:70. `bench_decode.py:37` drafts to (320, 320), which differs from the analysis rule, and src/lib.rs:74 has a third denom.

### D-047 [medium] Dotenv loader written five times — CONFIRMED

**Disposition: Fixed.** pgtools.load_env_file is the one Python loader, and qmltest.ps1 is removed (qmltest.py ran green here: 143 passed).

A dotenv loader with a regex is in tools/pgtools.py:42 `load_env_file` and `tools/lint_qml.py:32` (same regex), and tools/qmltest.ps1:12-15 has its own copy. The cited symbolize.py, profile.ps1 and build.sh do not exist, so those three cites are dropped. The three real copies stand.

### D-048 [medium] Doctor implemented twice — CONFIRMED

**Disposition: Fixed.** doctor.ps1 is removed; `just doctor` (doctor.py) ran green on this machine on 2026-11-21.

doctor.ps1:55 and doctor.py:67 both implement the libclang search and the checks, in PowerShell and Python.

### D-049 [medium] callgraph build implemented twice — CONFIRMED

**Disposition: Fixed.** `build.sh` is deleted; `build.py` is the one build (`profile.ps1` and `profile.py` point at it).

build.py:62 main compiles and links callgraph.dll, and build.sh (stale, DOC-059) repeats the same flag lists beside it. The two builds are kept side by side.

### D-050 [medium] LLVM default path computed three times — PARTIAL

**Disposition: Decided and applied.** `build.sh` was deleted and `profile.ps1` and `tools/README.md` point at `build.py`. This also settles D-049, TL-008 and DOC-059.

The LLVM default is computed in build.py:66-68 and build.sh:22-24. The cgtab.py:78 cite is an unrelated callgrind line, not an LLVM default, so only two copies are real.

### D-051 [medium] Newest-version selection in three tools — CONFIRMED

**Disposition: Deferred.** The three "newest" rules differ on purpose; the shared version_key move to pgtools is not done.

build.py:24 newest() sorts version digits in order, and build.sh:31 uses sort -V to pick the newest clang include folder. The fork-updates.py:49 cite does not exist.

### D-052 [medium] FFmpeg fetch implemented in ps1 and py — REFUTED

tools/setup-ffmpeg.ps1 does not exist. The only FFmpeg fetch-and-check script is `tools/setup_ffmpeg.py`. tools/build-static-ffmpeg.ps1 is a build script, and its duplicated pins are covered by D-055.

### D-053 [medium] QML test staging differs between ps1 and py — CONFIRMED

**Disposition: Fixed.** qmltest.ps1 is removed; `just qmltest` (qmltest.py) ran green here: 143 passed.

qmltest.ps1:32 hard-codes the Ratings singleton by name, while qmltest.py:59 detects 'pragma Singleton' in the file. The two stagers decide what a singleton is in different ways.

### D-054 [medium] Toolset pin repeated in ps1 and py build scripts — CONFIRMED

**Disposition: Fixed.** The Python builds take MSVC_TOOLSET from pgtools; the ps1 copies are removed, and the tests that compared them with the Python pins are removed with them.

The toolset pin 14.44.35207 is written in build-static-ffmpeg.ps1:24, build-jpeg-turbo.ps1:24 and tools/pgtools.py. The cited `build_static_ffmpeg.py` does not hold it, so that cite is wrong; the ps1 and pgtools copies are the duplicates.

### D-055 [medium] FFmpeg and jpeg-turbo pins duplicated in ps1 and py — CONFIRMED

**Disposition: Fixed.** The pins are one copy, in the Python recipes; the ps1 originals are removed.

FFmpeg commit, release and SDK pins are in build-static-ffmpeg.ps1:22-23 and `build_static_ffmpeg.py:33-35`, with the same values. The jpeg-turbo fork URL and commit are in build-jpeg-turbo.ps1:22-23 and `build_jpeg_turbo.py:31-32`.

### D-056 [medium] FFmpeg configure flags in two languages — CONFIRMED

**Disposition: Fixed.** Configure flags live in build_static_ffmpeg.py; the ps1 copy is removed.

`CONFIGURE_FLAGS` in `build_static_ffmpeg.py:43` and $ConfigureFlags in build-static-ffmpeg.ps1:31 list the same configure flags, with the same comment that each is the pilot's.

### D-057 [medium] System DLL allowlist in two places — CONFIRMED

**Disposition: Fixed.** The system DLL allowlist lives in build_static_ffmpeg.py; the ps1 copy is removed.

`build_static_ffmpeg.py:57` `SYSTEM_DLL` regex and build-static-ffmpeg.ps1:194 non-system filter list the same prefixes (api-ms-win-, KERNEL32, USER32, bcrypt, VCRUNTIME140, MSVCP140).

### D-058 [medium] Static FFmpeg library list repeated — CONFIRMED

**Disposition: Fixed.** Test StaticFfmpegLibraries checks both patch lists against build_static_ffmpeg.py.

The five static FFmpeg libraries are listed in tools/static-qt/pg-video.patch:33 and as `STATIC_FFMPEG_LIBS` in app-build.patch:118, with the same names in the same order.

### D-059 [low] OS error-code tables in two places — REFUTED

Only drive.rs:54 `unplugged_code` holds an OS error-code table in code. `IMPORT_FAILURES_PLAN.md:210` proposes a new `classify_raw` table and says it absorbs `unplugged_code`, so the plan removes the second table rather than adding one.

### D-060 [low] Thumbs batch size 200 duplicated — CONFIRMED

**Disposition: Fixed.** `thumbs::BATCH` is the one `200`; `placeholders.rs` imports it.

const BATCH: usize = 200 is at thumbs lib.rs:31 and placeholders.rs:28, with no shared constant.

### D-061 [low] Drive-or-share test written three times — CONFIRMED

**Disposition: Deferred.** The owner's default (native.rs form_of for verbatim volume GUIDs) is not applied; it changes what the shell is given and needs a Windows test.

Three drive-or-share predicates: platform dirs.rs:48 `is_absolute_for`, native.rs:215 `form_of`, and picker.rs:19 `has_drive_or_share`, each testing for a drive or UNC prefix in a slightly different way.

### D-062 [low] Hand-typed colours bypass the Theme singleton — CONFIRMED

**Disposition: Fixed.** The repeated literals now name `Theme` tokens: `panelScrim` (ModalOverlay), `errorText` (GridCell), `invalidText` and `statusWarning` (InboxPanel, LogPanel). The four `#a0000000` copies (GridCell, RatingBar, VideoTransport) use a new `Theme.scrimBacking`, not `captionBacking`, whose alpha differs. `Theme.qml`'s header is corrected: the grid and the inbox are the main window's, so the file no longer says it holds only the other components' colours.

Theme.qml holds the tokens panelScrim #b0000000 (:9), captionBacking #b3000000 (:17), errorText #e57373 (:24), invalidText #e06060 (:26) and statusWarning #e0a050 (:30). ModalOverlay.qml:26 repeats panelScrim, GridCell.qml:95 repeats errorText, InboxPanel.qml:385 and :295 repeat invalidText and statusWarning. GridCell.qml:151 uses #a0000000, a near-copy of captionBacking.

### D-063 [low] Desktop QML structural duplication — CONFIRMED

**Disposition: No change.** PanelAction is shared; the wheel accumulators and the preloads call different things.

enum PanelAction { Close, StepBack, StepForward, Pick } is declared at YandexPanel.qml:90 and identically at YandexPreview.qml:20. The wheel accumulator is at Viewer.qml:302 and YandexPreview.qml:59, and the neighbour preload is at Viewer.qml:396 and YandexPreview.qml:220.

### D-064 [low] Desktop Rust int conversion and level-code helpers repeated — CONFIRMED

**Disposition: Fixed.** `rows::qt_int` is the one saturating Qt int conversion. The copies in `gallery/roles.rs`, `failed_files.rs`, `lane_model.rs`, `notices.rs`, `yandex.rs` (`qt_int` and `qt_side`) and `shell.rs` (`px`) are gone, and `facet_choices::count` delegates to it. `log_model::level_code` is the one level mapping; `notices.rs` uses it. `viewport.rs`'s `px` (an `i64` to `usize` clamp) is a different conversion and stays.

`facet_choices.rs:30` count (i64 to i32, clamped), gallery/roles.rs:83 `qt_int`, `failed_files.rs:57` `qt_int`, and the `level_code` mapping at `log_model.rs:64` and notices.rs:227 (0 info, 1 warning, 2 error).

### D-065 [low] Desktop pending batch and save helpers repeated — CONFIRMED

**Disposition: Fixed.** The log view and the inbox gather their changes in `pump::Pending<P>` and lock them with `pump::lock`; their own `Pending` structs and `lock` functions are gone (the inbox's payload is `InboxChanges`). `settings::save_prefs` is the one prefs save, and `change_prefs` calls it. The two modules keep their own reader loops, which differ in what they queue.

Pending is declared three times: `log_model.rs:53`, notices.rs:215 and pump.rs:13 (generic). `save_prefs` at shell.rs:107 and `change_prefs` at settings.rs:18 have the same body, a prefs change then a worker write.

### D-066 [low] Client text and test helper duplication — PARTIAL

**Disposition: No change.** The cited helpers are not duplicates.

tree.rs:197 `visible_count` repeats the `visible_in` walk, and notice.rs:22 isolated is a bidi wrapper. The controller panels.rs:139 `set_viewer` is one setter, not four visible here. image.rs:109 `view_path` prints the region grammar that `view_of` parses back, which is a round-trip, not a duplicate. replay.rs does not exist and words.rs:97 `ui_entries` was not compared to `entries_of`.

### D-067 [low] Codec and container test helpers repeated — CONFIRMED

**Disposition: Fixed.** The decode test flattens with `codecs::test_support::flatten_rgba` (built on the one `over_white`) instead of its own copy of the formula. The container tests use `PNG_SIGNATURE` rather than the signature literal. `to_pixels` already has one definition (`decode.rs`, which `resize.rs` uses).

`over_white's` formula is in codec convert.rs:160-162 and repeated at decode.rs:510 in a test. `to_pixels` is in decode.rs:259 and resize.rs:125. container lib.rs:1019 writes the PNG signature as a literal in a test while the `PNG_SIGNATURE` constant exists at :251 in the same file.

### D-068 [low] Hash-based pseudo-random generator copied across crates — CONFIRMED

**Disposition: Fixed.** pg-testrng is the one leaf generator; pg-testkit re-exports it; dedup, container and model use it as a dev-dependency.

A xorshift-style generator is written in container lib.rs:1201 (noise), dedup join.rs:2078 (Rng::draw), metadata container.rs:216 (xorshift) and model pdq.rs:157 inline. All are test helpers, each with its own shift triple.

### D-069 [low] Dbfile lock and open repeated — CONFIRMED

**Disposition: Fixed.** The retry loop is `lock::lock_with_retries` (shared or exclusive), used by `Lock::take` and `Live::shared`. The two walks of `PARTS` in `remove.rs` share the one `PARTS` constant; they rename in one case and delete in the other, so they stay as two loops.

dbfile lock.rs:29 (Lock::take) and lock.rs:57 (Live::shared) each run the same `LOCK_TRIES` retry loop with `try_lock`, and remove.rs:30 walks PARTS in `move_aside` and `remove_database`.

### D-070 [low] clippy.toml repeats root test allowances — CONFIRMED

**Disposition: Fixed (guarded).** Clippy has no include, so `crates/model/clippy.toml` must repeat the root's test allowances and hash bans. The repeat stays, its comment says why, and `tools/test_ci_files.py` (`ClippyFiles`) fails when the two drift.

model/clippy.toml:3-5 repeats allow-panic-in-tests, allow-print-in-tests and allow-unwrap-in-tests, which root clippy.toml:2-6 sets. The model file's own comment says to keep them in step with the root.

### D-071 [low] Docs restate the same measurements — PARTIAL

**Disposition: No change.** Most cited items are pointers to other docs, not restatements.

PROFILING.md:494 'Moved to `JPEG_DECODERS.md'` is a pointer, not a restatement. STORAGE.md:95 points to PROFILING.md and `START_PROFILE.md:5` gives its own stage list. The restatement claims were only partly checked, so most of the finding is pointers.

### D-072 [low] Historical guard test cited by two plans — CONFIRMED

**Disposition: Fixed.** The test is `carried_is_every_table_of_the_current_schema` in `catalog/src/schema.rs`. The four plans that cite it now name it without a line number (`BACKGROUND_JOB_AUDIT_FIXES`, `DECODE_ONCE_PLAN`, `CATALOG_SIZE_PLAN`, `NTFS_MFT_SCAN_PLAN`), so no cite goes stale again.

`BACKGROUND_JOB_AUDIT_FIXES.md:37` cites `carried_is_every_table_of_the_current_schema` at schema.rs:804, and `DECODE_ONCE_PLAN:716` cites it at schema.rs:803-820. The test is at schema.rs:338 now. Both plan cites are stale, and they disagree with each other.

### D-073 [low] Eval timing, metric and callgrind helpers repeated — CONFIRMED

**Disposition: Fixed.** `linux.callgrind_instructions` runs callgrind and reads the total, and `linux.marginal_millions` is the repeat-3-minus-repeat-1 figure; `harness_linux.py`, `restart.py` and `linux.py`'s own `ir` use them. The median timing loop is one `timing.py` (with its `BUDGET_S`), shared by `gpu.py` and `nvjpeg.py`. `gpu_encoders.py` and `nvimgcodec_bench.py` time differently with a 2 s budget, so they keep their loop.

Callgrind counting is repeated in linux.py:111 (ir), `harness_linux.py:29` (`ir_one`) and restart.py:44 (instructions), and the median timing loop in nvjpeg.py:209 (timed). The harness.cpp cites do not exist.

### D-074 [low] Build helper boilerplate repeated across tools — CONFIRMED

**Disposition: Fixed.** `pgtools.checked` and `pgtools.sha256` are the one copy of each; `build_jpeg_turbo.py`, `build_static_ffmpeg.py` and `build_static_qt.py` import them, and the `hashlib` imports they no longer need are gone. `pg_home_folder` has one definition (in `build_static_qt.py`), so nothing there needed moving.

checked(what, command, log, cwd) is defined with the same signature in `build_jpeg_turbo.py:36`, `build_static_ffmpeg.py:60` and `build_static_qt.py:47`. sha256 is written at `build_jpeg_turbo.py:64` and elsewhere, and `pg_home_folder` at `build_static_qt.py:66`.

### D-075 [low] Qt kit identifiers repeated — CONFIRMED

**Disposition: Fixed.** `pgtools.WINDOWS_QT_KIT`, `LINUX_QT_KIT` and `QT_KIT` are the one set of kit names; `check_qt.py`, `doctor.py` and `ci/github_files.py` use them.

The Qt kit names `msvc2022_64` and `gcc_64` are in `tools/check_qt.py:19` and `tools/ci/github_files.py`. The cite `ci/github_files.py:55` is at `tools/ci/github_files.py`.

### D-076 [low] Modgraph and profiling tools repeat small helpers — CONFIRMED

**Disposition: No change.** unreachable_pub uses compiler_graph.library_packages; the production cut is testcut.production_part.

`library_crates` is defined in `tools/modgraph/compiler_graph.py:29` and `unreachable_pub.py`. The test-section cut is written in `module_graph.py:61` and the other modgraph tools, and REPO = parent.parent is in `tools/fuzz_smoke.py:14` beside other path logic.

### D-077 [low] Pairing scripts repeat SQL and invocation — CONFIRMED

**Disposition: No change.** drop_owed_pairs.py is shared; the medians are different statistics by design.

The owed-pair delete SQL is written in keep.ps1:38-40 and oracle.ps1:42, with the same DELETE FROM `pdq_pairs` WHERE a IN `pairs_owed` statement. The median is computed in `few_sweep.ps1:26` and keep.py:172 in different ways.

### D-078 [low] The folder key folds case with Rust's lower case, not NTFS's own table — CONFIRMED

`PathRules::path_key` and `folder_key` fold each character of a Windows name with Rust's per-character lower case. NTFS folds with its own upper-case table (`$UpCase`), which differs from Rust's on some letters: `σ` and `ς` are one letter to Rust's lower case but two to the table's, `ſ` and `İ` fold differently, and the Kelvin sign folds to `k`. So one folder can be two keys here (a spelling NTFS treats as the same), or two folders can be one to NTFS and stay two rows. The residual cases are pinned in `crates/model/src/rules.rs` (`the_windows_fold_keeps_the_residual_cases_as_rust_folds_them`), so a change of the fold shows. Disposition: Deferred. A fix reads NTFS's table from the volume or ships a fixed copy, and that is a decision about the catalog's keys (a `KEY_ALGO_VERSION` bump), not a code tidy-up.

## errors

### E-001 [high] Panics mapped to different outcomes, one of them a fake user stop — CONFIRMED

**Disposition: Fixed.** A panicked Yandex search or save is `Status::Panicked` (was `Stopped`, which read as the user's stop), with the message `yandex-panicked` ("Ended by an internal error; the details are in gallery.log."). A lane whose thread panics now carries the panic's message as a `pg_jobs::panics::Panicked`, so `is_panic` recognises it, instead of the generic "a lane panicked". `ReadError::Panicked` already had this shape.

reader.rs:227 maps a caught panic to ReadError::Panicked. yandex.rs:465 maps a panicked search to Status::Stopped, which reads as a user stop. lanes.rs:145 turns a join failure into a generic lane panic and discards the payload. images.rs:166 is a mutex poison recovery, not a panic mapping, so that instance is wrong. The core claim holds.

### E-002 [high] Exit status reports success on failure — CONFIRMED

**Disposition: Fixed in part.** colour_check.cu exits 1 over a threshold of 1; the binary builds (nvcc 13.4, MSVC, turbojpeg-static.lib) and ran on two 4:4:4 inputs on 2026-11-21 with 0 mismatched channels. The 4:2:0 and 4:2:2 results are from 2026-11-14 and were not re-run. screenshot.cpp exits 1 for a run that is not settled; that exit is not exercised, since a normal `just shots` always settles. Unreachable-check exit done in unreachable_pub.py.

screenshot.cpp:186 ends a timed-out-without-window run with QCoreApplication::quit(), which exits 0. The sibling no-window path at :174 uses exit(1). `unreachable_pub.py:48` exits 0 unconditionally after printing error counts. `colour_check.cu:62` returns 0 with no threshold. `REVIEW_LEDGER.md:86` records that the exit mapping has no test.

### E-003 [high] Fluent syntax errors and missing entries are silently accepted — CONFIRMED

**Disposition: Fixed.** `words.rs`'s `english()` no longer discards the parse and add results: a syntax error in the compiled-in English, a repeated id, or a failed built-in registration fails a debug build and the tests (`debug_assert!`), and a release build keeps the same fallback as before. A missing entry still shows its id by design (`shown`), and the desktop's test refuses that. `LANGUAGES.md` describes the new behaviour.

words.rs:40 keeps the partial resource on a Fluent syntax error. words.rs:43 discards the `add_resource` result, so duplicate or failed entries are dropped. words.rs:62 returns the raw id when an entry is missing. words.rs:38 discards the `add_builtins` result. LANGUAGES.md:43 documents the same silent drop, so the gap is known but unfixed.

### E-004 [medium] Error chains formatted with {e} in some places and {e:#} in others — CONFIRMED

**Disposition: No change.** The {e:#} rule holds for anyhow chains; the desktop app.stop site now follows it.

Plain Display ({e}) and the alternate chain ({e:#}) are both used for the same kind of error text: app.rs:58 and :217 use {e}, app.rs:74 uses {e:#}, desktop app.rs:63 uses `to_string(`), and yandex.rs:122 uses {err:#}. The mix is real.

### E-005 [medium] Stored ledger failure text built four different ways — CONFIRMED

**Disposition: Fixed.** A ledger's stored text is `pg_jobs::failure::Cause::of(&e).to_string()`: the error's chain, or a caught panic's message. The import and hashes ledgers and the placeholders' ledger now all use it (the placeholders' `{e:#}` did not show a panic's message as a panic). The Thumbs pass's `shown.error` comes from `file_failure`, which is the same `Cause`. The writeback's stored text is its typed `WriteError`'s `Display`, which renders the same `Cause`, so it is consistent with the rest and unchanged.

Ledger failure text comes from several forms: hashes.rs:120 uses {e:#}, import lib.rs:179 uses `Cause::of(&e).to_string(`), writeback lib.rs:441 stores `error.to_string(`), thumbs lib.rs:320 stores shown.error, and placeholders.rs:217 uses {e:#}. The app/error.rs:76 cite is a blank line, so that instance is wrong, but the rest holds.

### E-006 [medium] Notice error text tidied and capped only on some paths — CONFIRMED

**Disposition: Fixed.** `pg_model::notice::error_text` is the one rule for a notice's error text (its chain, `{:#}`, on one line, capped at `ERROR_CAP`). The desktop's `app::error_text` and the library's `passes::error_text` are now re-exports of it, so every caller that used either gets the same text. The tally's round notice (`RoundFailed`) tidies its text where it is posted, as the ledger notice already did. (The audit's `coordinator.rs:709` already uses the tidying helper; the lane tidies its own failure before it reaches the tally.)

prefs.rs:153 tidies with `ERROR_CAP`, coordinator.rs:709 calls `error_text` with no tidy while the panic arm at :718 tidies, and tally.rs:93 posts the raw lane error while tally.rs:127 tidies. The tidy and cap are applied on some notice paths only.

### E-007 [medium] Assertions and expects guard invariants inconsistently in library code — CONFIRMED

**Disposition: Fixed (by rule).** `crates/README.md` ground rule 8 is the one convention: an `assert!` with a message naming the invariant, `unreachable!` with its reason for an impossible arm, and `expect` only under `#[expect(clippy::expect_used, reason)]`. The cited guards that were read already take one of these forms (`seam.rs`'s assert has its message, `union_find.rs` carries the `#[expect]` with its reason). Not every guard in the crates was read against the rule; a lint for the bare `expect` would make it checked, and is a follow-up.

Invariant guards vary: seam.rs:11 has an assert with a message, pdq.rs:57 has an assert with no message, work.rs:120 uses unreachable!, `union_find.rs:89` uses expect with a clippy reason, and desktop app.rs:54 uses expect with an allow. Low severity: the styles are each defensible, but they are not one convention.

### E-008 [medium] Silent discards of errors in production paths — PARTIAL

**Disposition: No change.** The discards are documented best-effort (`thumbcache/src/lib.rs:189`, `writer.rs:398`).

Several discards are documented best-effort (thumbcache lib.rs:189 says best-effort; writer.rs:398 says failures stay queued), so they are a choice, not an oversight. The crop.rs:150 cite shows Ok(stats), not the `remove_file` the finding names, so that instance is wrong. The discards are inconsistent in their handling, but not all are bugs.

### E-009 [medium] Fallbacks hide missing or dropped data — CONFIRMED

**Disposition: No change.** Each cited fallback was decided on its own (TreeFull, unreadable versions, the EXIF default).

- `TreeBuilder::add` (`model/src/tree.rs`): a folder past the `u32` offsets was left out with no word. It now returns `Err(TreeFull)`, and the catalog's tree read fails with it, which the desktop reports through its existing `TreeFailed` notice. The callers in tests unwrap it.
- `ImageKey::parse` (`model/src/image.rs`): an unreadable version after `@` was dropped, so the key was served as the current file. It is now refused (`None`), as an unreadable file id already was; the URL parser turns that into a rejected request.
- `Write::from_stored` (`model/src/write.rs`): `None` for an out-of-range value, which the schema's `CHECK` makes impossible. The one reader, `catalog/src/queue.rs`, already turns the `None` into an error, so it is not silent.
- The `let-else` sites in `tree.rs` (the stack and the node lookups) index nodes the builder made, so a miss is a bug: they now `debug_assert!`, and the release fallback is unchanged.
- `orientation.rs`'s "1 when absent or unreadable" is the EXIF default (no orientation tag means upright), and its doc says so. Left as it is.

tree.rs:141-143 leaves a folder out of the tree when its offsets overflow u32, with no report, and the same silent let-else appears at tree.rs:180, 217 and 235. write.rs:95 drops an out-of-range stored value to None. image.rs:41 drops an unreadable version. orientation.rs:17 treats missing EXIF as upright. Several are intended fallbacks, but none is reported.

### E-010 [medium] Documented behaviour not implemented or unbounded decode path — REFUTED

ledger.rs:199-201 says `retry_failures` does nothing for the import, and that the import is retried by gallery import --retry. The code (ledger.rs:203-204) returns Ok early for FilePass::Import, as the doc says. The doc and the code agree, so the documented behaviour is implemented.

### E-011 [medium] Missing-configuration messages worded differently across tools — CONFIRMED

**Disposition: Fixed (Python tools).** `pgtools.not_set(name, how)` words the one message, `NAME is not set: <how>`, and the Python tools that report `PG_HOME` or `QT_DIR` missing now call it (the builds, `check_qt`, `compare_shots`, `lint_qml`, `qmltest`, `profile`). Each keeps its specific hint. `qmltest.ps1` and `profile.ps1` are removed.

The `PG_HOME` message is worded differently in `build_static_qt.py:69`, `build_jpeg_turbo.py:75` and `build_static_ffmpeg.py:92`. The `QT_DIR` message is worded differently in `compare_shots.py:666`, `check_qt.py:21`, `lint_qml.py:70`, profile.ps1:26, qmltest.ps1:20 and qmltest.py:41.

### E-012 [medium] Same error text duplicated across Python and PowerShell ports — PARTIAL

**Disposition: No change.** `setup-ffmpeg.ps1` is not at HEAD, and the cited lines do not show the duplication.

setup-ffmpeg.ps1 at HEAD has the throws at :45 (SHA mismatch) and :58 (top folders), so the PowerShell side is real. The Python cites (`setup_ffmpeg.py:89` and :62) are blank or argparse lines and do not show the identical wording, so the duplication is not shown by these cites. The file is deleted in the working tree.

### E-013 [medium] The two setup ports apply different safety checks — CONFIRMED

**Disposition: Moot.** The PowerShell `setup-ffmpeg.ps1` the finding compares against is not in the tree. `tools/setup_ffmpeg.py` refuses a URL that is not https (its check is at line 42), so the one port that exists has the check.

At HEAD, setup-ffmpeg.ps1 has no https check (grep finds none). `setup_ffmpeg.py:42-43` refuses a non-https URL. The finding's python cite at :39 is the docstring; the check is at :42, which is a small cite error that does not change the verdict.

### E-014 [medium] Retry and failure text for builds differ between ports — CONFIRMED

**Disposition: Fixed.** Python builds fail through checked() with the step and exit code; the ps1 originals that kept their own text are removed.

build-jpeg-turbo.ps1:64 retries git fetch three times with no sleep. `build_jpeg_turbo.py:59-60` sleeps 10 times the attempt number. build-static-ffmpeg.ps1:118-119 sleeps, and `build_static_ffmpeg.py:156-157` sleeps too. The failure texts also differ: the py versions include the exit code, the jpeg-turbo ones do not.

### E-015 [medium] Tool error messages inconsistent in name prefix — CONFIRMED

**Disposition: Fixed.** pgtools, lint_qml and qmltest messages start with the script's name.

linux.py:67 prefixes its fail() messages with linux.py:, while pgtools.py:35 has no prefix, `lint_qml.py:51` has none, and profile.ps1:26 has none. `win_env.py:50` uses `win_env`: and win-env.cmd uses its own prefix. The `tools/ci_setup.py` cite is missing (the file is `tools/profiling/ci_setup.py`).

### E-016 [medium] Python eval raises different exception kinds for the same class of failure — CONFIRMED

**Disposition: Fixed.** eval/README has an Errors section; accel, models and verify sites follow it.

Eval scripts raise different exception kinds for similar failures: `cpu_worker.py:137` raises SystemExit, run.py:304 raises SystemExit, accel.py:202 raises RuntimeError, models.py:56 raises ValueError, and verify.py:696 calls parser.error. Low severity; these are harness scripts.

### E-017 [medium] Per-item failure policy differs across harnesses and sweeps — CONFIRMED

**Disposition: Deferred.** The per-item policy is stated, but narrowing nvimgcodec_bench's catches needs the exception a GPU run raises. The pinned venv (`tools/jpeg-eval/venv`, nvimgcodec 0.10.0.9) is in place on 2026-11-21, so that run is the only step left.

ann/run.py:343 records a failed configuration and continues. accel.py:130 skips a bad calibration image. `bench_decode.py:57` reads outside its try, while pdq.py:47 returns None on any Exception and verify.py:603 records the pair as error. `nvimgcodec_bench.py:87` catches Exception where nvjpeg.py:298 catches RuntimeError only. The failure policies differ across harnesses.

### E-018 [medium] String used as the error type in public functions — CONFIRMED

**Disposition: Fixed.** Typed error enums in pg-model (RowHeightError, ThumbHeightError, PositiveError, RatingError, WriteKindError), messages kept; callers mapped.

config.rs:25 `Config::from_env` returns `Result<Config`, `String>`. layout.rs:29 Layout::resolve returns `Result<Layout`, `String>`. size.rs:44, rating.rs:11 and replay.rs:33 also use String error types. query.rs:98 uses u8 as the error type. These sit beside typed errors elsewhere.

### E-019 [medium] Crate error types: anyhow rule in the docs does not match the code — CONFIRMED

**Disposition: Deferred.** The owner's rule (no anyhow in a library's public API) is not yet written into REFACTOR_LEDGER F-14.

`REFACTOR_LEDGER.md:59` (F-14, status decide) records that the plan says anyhow lives only in cli, desktop and gallery, but it appears in 20 crates and no lint enforces the rule. The gap is acknowledged and open, so the inconsistency is real. The lib.rs:27 and rebuild.rs:104 cites show the mixed return types.

### E-020 [medium] Catalog wraps conditions in Error::Other(anyhow) where typed variants exist — CONFIRMED

**Disposition: Fixed.** The catalog's `Error` gained typed variants for the conditions that were wrapped in `Other`: `NotInCatalog`, `BeingWritten`, `OtherBuildsBorders` (`borders.rs`), `Held` (the lock refusals in `lifecycle.rs`, both messages), `MigrationRoom` (`lifecycle.rs`), `RootAgain` and `Stored` (the NUL and "doesn't parse" checks in `removal.rs` and `pair.rs`), `ForeignKeysOn` (`schema.rs`), and `NotThisBuild` and `SchemaVersion` (`core.rs`). Each keeps its message text, and `is_busy` lists them as not busy. The `anyhow` imports these left unused are gone. `work.rs`'s `file {raw} has malformed stored hashes` still carries the value in its text, which a `&'static str` payload cannot hold, so it stays as an `Other`; that is a deliberate exception.

Catalog code wraps conditions in Error::Other(anyhow) where typed variants exist: borders.rs:47 and :55, work.rs:286, removal.rs:243, and lifecycle.rs:48 for a lock. lifecycle.rs:38 uses the typed Error::PathRules for a similar rules mismatch, so the typed variant is there to use.

### E-021 [medium] Catalog path prefix convention applied inconsistently — CONFIRMED

**Disposition: Fixed.** Catalog-level messages begin "catalog {path}: "; row-level messages stay unprefixed.

lifecycle.rs:47 prefixes its lock messages with catalog path, while ledger.rs:269, :301 and lookup.rs:102 have no catalog prefix, and rebuild.rs:95 and :112 use their own forms. core.rs:125 repeats the same context, with the path alone.

### E-022 [medium] Catalog refusal texts for the same condition differ — CONFIRMED

**Disposition: Fixed.** Open refuses through Refusal, one text per condition; OlderSchema and NewerMark messages added.

core.rs:139 says the schema version is unknown to this build and was made by another build. core.rs:270 says schema version X, not this build's Y. core.rs:163 says the schema predates `key_version` and to restore a backup. schema.rs:302-313 gives its own mismatch reasons. The refusal for one condition is worded differently in each.

### E-023 [medium] Catalog InvalidColumnType payloads take four shapes — CONFIRMED

**Disposition: Fixed.** InvalidColumnType is a rusqlite FromSqlConversionFailure carrying the real column name.

InvalidColumnType payloads take several shapes: borders.rs:90 uses the stored key, queue.rs:146 the raw kind text, queue.rs:151 a formatted kind and value, queue.rs:161 a column name, and queue.rs:168 a column name with a Null type. The grid.rs:67 cite is missing; the other five instances hold.

### E-024 [medium] Lossy error conversions flatten the source to a string — CONFIRMED

**Disposition: Fixed.** `rebuild.rs`'s `remove_database` error keeps its source: it is `pg_dbfile`'s `io::Error`, mapped to the catalog's typed `Io` variant instead of its display text. `roots.rs`'s `RemovalPending` carries the folder's display text by design (the message the user reads, not a value read back), so it stays. The Yandex model's `SaveFailure::Write { error: String }` is a `String` on purpose: `SaveFailure` derives `Clone` and `Eq`, and an `io::Error` is neither; the variant's doc now says so. The `refusal.rs:34` `to_string()` cite no longer matches the file (`model/src/refusal.rs` has no such call), so there is nothing to change there.

rebuild.rs:138 turns a `pg_dbfile` error into Error::Other of its display text, which loses the source. roots.rs:75 stores rules.display() strings in RemovalPending. yandex.rs model :70 stores a String error where Download keeps the typed RequestError. refusal.rs:34 stores `error.to_string(`).

### E-025 [medium] Invariant text and error shape for the codec memory limit kind — PARTIAL

**Disposition: Decided and applied.** A malformed TIFF sample type is now a decode error (`tiff.rs`). Real budget limits are unchanged.

tiff.rs:102 returns LimitsExceeded for a missing sample type, and png.rs:111 maps the png decoder's LimitsExceeded to InsufficientMemory, which is the kind the finding says should be used for a real memory limit. gif.rs:58 uses InsufficientMemory for a `checked_mul` overflow, which is arguably the right kind for an overflow. Whether each mapping is right needs a per-case decision; the finding's single 'wrong kind' claim is not shown.

### E-026 [medium] Debug text reaches users in codec and decoder messages — CONFIRMED

**Disposition: Fixed.** The user-visible message for a JPEG the zune decoder can't do (`codecs/jpeg.rs`) is now a fixed sentence ("a JPEG coding this decoder does not read"): zune-jpeg's reason has `Debug` only, so there is no `Display` to use, and its `Debug` form is not for a user. The eval harness's own `Debug` lines are developer output and stay.

jpeg.rs:97 puts the zune-jpeg Debug form of the error into a GenericFeature message, which the user can see. eval png-decoders lib.rs:78 uses Display and lib.rs:126 and main.rs use Debug, so the harness mixes forms too. The user-facing case is the one that matters.

### E-027 [medium] Path rendering in error text uses three helpers — CONFIRMED

**Disposition: No change.** dbfile keeps Path::display: it has no path rules, and shown() canonicalises through the filesystem.

Paths in error text come from three helpers: crop/lib.rs uses shown(), dbfile/remove.rs:35 uses Path::display(), and removal.rs:243 uses rules.display(). scan/lib.rs:375 uses catalog.rules().display(). The mix is real; low severity.

### E-028 [medium] Turns bound in `from_stored` duplicates the modulo rule — PARTIAL

**Disposition: No change.** The literal is a range check, not the modulo rule. Rejecting damaged stored values is by design.

write.rs:98 bounds stored turns with a literal small `<=` 3, and returns None above it. turns.rs:14 reduces modulo 4. These are different rules: `from_stored` rejects a damaged stored value, while Turns::new wraps a computed one. The literal duplicates the range but does not duplicate the modulo rule.

### E-029 [medium] Stale last-error buffer in the GPU encoder — CONFIRMED

**Disposition: Fixed.** `egpu_encode_batch` sets the buffer to "ok" on entry (`encode_gpu.cu`), so a call that succeeds does not leave an earlier call's message for `egpu_last_error`.

`encode_gpu.cu:986` `egpu_encode_batch` does not clear `g_error` on entry, and `g_error` is written only on failure (:1000 and :1006), so a later error can read a stale message from an earlier call. The eval harness is the only place this matters.

### E-030 [medium] Unchecked CUDA and FFI return values — CONFIRMED

**Disposition: Fixed.** The harness's CUDA calls are checked: `test_encoder.cpp`'s `cudaMemcpy` and `cudaMalloc` go through `cuda_ok`, which ends the run with the CUDA message (no result exists without them); `dec_gpu.cpp`'s `cudaStreamCreate` failure becomes the decoder's init error, and its `cudaSetDeviceFlags` failure is a warning (the decoders still run); `dec_turbo.cpp` reports a null `tj3Init` handle as the decode's error.

`dec_gpu.cpp:68` ignores the cudaStreamCreate return, and :49 ignores cudaSetDeviceFlags. `dec_turbo.cpp:11` uses tj3Init's handle with no null check. `test_encoder.cpp:232` and :1094 ignore cudaMemcpy and cudaMalloc results.

### E-031 [medium] A test passes despite a decode failure — CONFIRMED

**Disposition: Fixed.** The decode-failure branch of `test6_quality` counts the failure (`++failed_loads`), so `pass` is false, as the harness's other failure branches already make it.

`test6_quality` at `test_encoder.cpp:973` sets pass as `failed_loads` == 0 at :1035. The decode-failure branch at :1008-1010 prints and continues without incrementing `failed_loads`, so a decode failure does not fail the test. Other branches in the same harness set pass false directly, so the handling is inconsistent.

### E-032 [medium] Verify scoring counts error pairs in two ways — CONFIRMED

**Disposition: Fixed (documented, consistent).** Both harnesses count an error decision as not correct: a pair is correct only when its decision is its expected one, and an error is never that. `verify.py` scores only labelled pairs (the others have no expected decision), and `verify_synth.py`'s pairs are all planted matches. Both now say so in the code, where the rule is applied.

`verify_synth.py:68` counts every non-match as incorrect, including error. verify.py:786-787 sets correct only for labels in EXPECTED, so an unlabelled pair is not counted. verify.py:623 records a failed pair as decision error. The two harnesses count error pairs differently.

### E-033 [medium] CLI error text comes from three sources — CONFIRMED

**Disposition: Fixed.** The CLI's error text now comes from the catalogue in every case that a user sees: the two scan refusals (`ScanError::Busy` and `ScanError::Removed`) were bailed with the scan crate's English `Display` and are now `cli::Line::ScanBusy` and `Line::ScanRemoved`, with their `cli-scan-busy` and `cli-scan-removed` entries. The path refusals go through `path_words` and the other refusals through `line(...)`, as before.

CLI error text comes from three sources: lib.rs:367 bails with the ScanError Display, lib.rs:403-405 bails with the Fluent line, lib.rs:145 maps through `path_words`, and dups.rs:62 builds anyhow from a line. The same command can print text from any of them.

### E-034 [medium] User-visible English literals bypass the message catalogue — CONFIRMED

**Disposition: Fixed, one kept.** The folder-list notice (`FolderNotRemoved` with "the folder list changed") now renders `Message::FolderListChanged`. `gpu_device.cpp`'s log line stays English (developer-facing). `providers.cpp`'s "image unavailable" is QML's `Image.errorString`, which the UI does not draw, so it is developer-facing too and stays English.

library.rs:282 gives a user-visible notice the literal the folder list changed: try again, bypassing text::render. providers.cpp:89 returns the English image unavailable. `gpu_device.cpp:121` logs English only. The first two reach the user; the log line is developer-facing.

### E-035 [medium] Start failure is logged but not shown to the user — CONFIRMED

**Disposition: Fixed.** When the change pump can't start, the window now posts `Notice::ChangesUnseen` (the inbox shows it, "The window can't see the library's changes"), not only the log line. The notice is wired through the model, the text layer, the catalogue and the notice table.

library.rs:157 posts LibraryUnavailable to the inbox when background work fails to start. library.rs:172 logs a change-pump start failure and posts nothing, so the user is not told that changes will not be seen.

### E-036 [medium] Yandex error texts and mappings disagree — CONFIRMED

**Disposition: Fixed.** `Unexpected` no longer carries English detail. `pg_model::yandex::UnexpectedReason` is the typed reason (thirteen of them, one per place the app or the parsers found a missing part), `RequestError::Unexpected` and `YandexError::Unexpected` carry it, and the client renders each reason through its own catalogue message (`yandex-upload-not-json` and the rest). The yandex crate's own `Display` shows the reason's name, which is developer-facing. The test that pinned the old sentence now asserts the new one. `Unreadable` and `Undecodable` still render as they did (the audit's point that the variants show the same text is the rendering of the two as "could not be read": left as it is, since the two errors are different causes).

yandex.rs:493 and images.rs:219 use YandexError::Unexpected with English details. images.rs test at :324 expects the text unexpected Yandex response: no such result. yandex.rs:254 uses Undecodable with {e:#} and :259-261 uses Unreadable with `to_string(`). text.rs:383 renders both Unreadable and Undecodable as one message, so the variants do not show different text.

### E-037 [medium] Replay errors count lines in some places and records in others — CONFIRMED

**Disposition: Fixed.** The replay's messages name the recording's line (one JSON line per record, so the line is what the user finds in the file). The replay loop had an off-by-one as well: `rest` starts at the second line, so its `n + 1` named the wrong line; it is `n + 2` now, and the test expects `index + 1` for the record it changed.

replay.rs:43 formats parse errors as line N and replay.rs:74 as record N. The test at client tests replay.rs:45 requires the replay error to start with record N:, and parse errors use line N. Two counting units are used in one module.

### E-038 [medium] Replay missing and mismatched format messages — CONFIRMED

**Disposition: Fixed.** The format-mismatch messages carry the line as well (`line 1:` for the Init, `line N:` where the parse reads one), so the same kind of failure has the same shape everywhere in the replay. The tests' expectations are updated.

Tests fix the two messages: tests replay.rs:55 expects a line 1 prefix on the missing-format error, while :60 expects recorded by format N with no line prefix. The same kind of failure is shaped differently.

### E-039 [medium] Fatal startup path writes the log and error box, not stderr — PARTIAL

**Disposition: No change.** The fatal path is documented design (`desktop/src/lib.rs:244-246`).

The fatal path is deliberate: the doc comment at desktop lib.rs:244-246 says a GUI program has nowhere to print, so it logs and shows a box, and it says the record is lost if the log has no file yet. The catalog-open call at lib.rs:169 follows that design. The gap is the in-memory record loss before the log opens, which the doc already states.

### E-040 [medium] Downcast to io::Error misses the Unopened wrapper — PARTIAL

**Disposition: No change.** The historical plan records its own correction.

`BACKGROUND_JOB_AUDIT_FIXES.md:178` says the Unreadable downcast is verified, and :186 amends DO 5.6 to stop using `downcast_ref` of io::Error because it misses Unopened. The plan records its own correction, so the inconsistency is in a historical plan that the same document fixes.

### E-041 [medium] Read failure is classified four different ways

**Disposition: Fixed.** ReadFailed (an io::Error newtype in pg-platform); jobs::is_io_read is the one predicate; thumbs keeps its Undecodable arm first.

Read failure is classified four ways: borders lib.rs:97 and import hashes.rs:114 downcast io::Error, thumbs lib.rs:112 matches MakeError variants, and maker.rs:43 defines MakeError with `of_video` downcasting Unopened. The two downcast sites are the same idiom in two crates and a third classifier beside them.

### E-042 [low] Thumbcache refusal and writer error texts — CONFIRMED

**Disposition: Fixed.** The three "cache unavailable" messages share their form (`thumbnail cache not open`, `thumbnail cache closed`, `thumbnail cache writer stopped`; the first was `no thumbnail cache`). The damaged-after-remake refusal now says what it does, as the foreign and newer refusals do: "…, even when made anew, so the cache can't be opened".

thumbcache schema.rs:297 bails with path: detail even when made anew. schema.rs:310 and :311 give Foreign and Newer the same shape, with Newer adding made by a later build. thumbcache lib.rs:138, :140 and :150 use three wordings for the cache being unavailable.

### E-043 [low] Error conversion idioms differ within the same file or area — CONFIRMED

**Disposition: Not changed (the idioms convert into different types).** `library.rs`'s `map_err(pg_catalog::Error::Other…)` and `listing.rs`'s `Error::Other(e.into())` both build the catalog's `Other`; `library.rs`'s `map_err(failed…)` builds the app's failure, and `yandex.rs`'s `.into()` builds the Yandex request error. They are four different targets written with the conversion each needs, not one conversion written four ways.

Conversion idioms differ within one area: library.rs:399 uses `map_err(pg_catalog::Error::Other`), library.rs:423 uses `map_err(failed`), yandex.rs:339 uses error.into(), and catalog listing.rs:40 uses Error::Other(e.into()). Low severity.

### E-044 [low] Source-level invariant and fallback behaviour in the platform layer — PARTIAL

**Disposition: Fixed.** `drive.rs` names `ERROR_NOT_READY` (21) instead of a bare literal.

picker.rs:141 holds the `ERROR_CANCELLED` constant, and drive.rs:56 uses the literal 21 for the unplugged code. drive.rs:251 defines `ERROR_MORE_DATA` locally as 234. The finding's crates/platform/src/lock.rs:50 does not exist; the lock test helper is at platform/src/testing/lock.rs. Half the cites are right, and the local-constant issue is real.

### E-045 [low] Planned error names in docs/plans still use pre-rename names — PARTIAL

**Disposition: No change.** The planned renames are pending, as the plan says.

`BACKGROUND_JOB_AUDIT_FIXES.md:80-84` plans renames: Error::ScanBusy to LeaseBusy and fence to LeaseLost. Code still has ScanBusy (catalog error.rs:28) and LeaseLost (error.rs:33), so the plan's names are pre-rename, which is expected until the work lands. The cli cite at lib.rs:168 does not show ScanBusy, so that instance is wrong.

### E-046 [low] Unknown-format wording repeated with three forms — CONFIRMED

**Disposition: Fixed.** The unknown-format message is `unknown image format` everywhere a person reads it: `writeback` now says `…: unknown image format` (it said `…: unknown format`), and the SQL conversion error in `catalog/src/lookup.rs` matches. The two `analyze.rs` and `display.rs` sites already had it.

Unknown-format wording comes in three forms: analyze.rs:102 and display.rs:38 use context unknown image format, lookup.rs:178 uses unknown format X inside a SQL conversion error, and writeback lib.rs:580 uses X: unknown format. The three shapes are real.

### E-047 [low] Too-large and damaged wording in the edit crate varies — CONFIRMED

**Disposition: Fixed.** A short image's data is "the image data is short" in `gif.rs` as in `raster.rs`; a damaged PNG is "the PNG is damaged" and its header "the PNG header is damaged". The too-large messages keep their specific forms (`raster.rs`'s "is too large to decode" with the kind named, and `webp.rs`'s RIFF size, which is a different limit).

Edit crate messages vary: raster.rs:19 says is too large to decode, webp.rs:53 says the WebP is too large for a RIFF file, png.rs:52 says damaged PNG, and gif.rs:37 says GIF image data is short while raster.rs:40 says the image data is short. The wording is inconsistent.

### E-048 [low] Spelling, case and symbol drift in user-facing messages — CONFIRMED

**Disposition: Fixed in part.** The planar TIFF message keeps image's spelling ("color") because the decode test compares against image; other codec messages use "colour".

tiff.rs:132 uses color while jpeg.rs:36 and :66 use colour. tiff.rs:140 capitalises Not enough planes. crop.rs:102 and the size text use x and the multiplication sign inconsistently: fixtures.rs:20 and crop.rs:36 use the multiplication sign.

### E-049 [low] Contraction and modal wording varies — CONFIRMED

**Disposition: No change.** "isn't in the catalog" is the one form; the inbox sentences are different on purpose.

borders.rs:47 uses isn't in the catalog and state.rs:83 uses is not in the catalog. inbox.rs:398 and :927 use different wording for the same kind of failure. Low severity.

### E-050 [low] Context strings vary in style and some are missing — CONFIRMED

**Disposition: Fixed.** The read in `thumbs/src/placeholders.rs` that had no context now says "can't read `<file>`". The bare `decoding` and `encoding` contexts in `codec/src/encode.rs` say what failed ("can't decode the image", "can't encode the image"). The lane coordinator's context reads "can't start the lane coordinator", in the same form as the inbox's. `pipeline.rs`'s line is a comment, not a context, and is not changed.

Context strings vary in style: pipeline.rs:339 is a comment, not a context. coordinator.rs:192 uses start the lane coordinator, borders cli uses reading X, encode.rs:88 uses bare decoding and encoding, and placeholders.rs:131 has no context on fs::read. Low severity.

### E-051 [low] Log level for the same kind of failure varies — CONFIRMED

**Disposition: Fixed.** The two gallery sites that log a command or a details request arriving before the grid model exists (`desktop/src/gallery.rs`, `desktop/src/gallery/state.rs`) log a warning: the request is dropped, the app goes on, so it is a recoverable condition, as the animation's and the controller's are. They were errors.

Log level for similar conditions varies: gallery.rs:69 and state.rs:148 log an error when a command arrives before the model exists, animation.rs:63 logs a warning for a recoverable file error, and controller.rs:45 logs a warning for a bad option. The levels are a judgement, and the mix is real.

### E-052 [low] NUL-in-path error wording varies — CONFIRMED

**Disposition: Fixed.** A NUL in a folder path the catalog stored reads "a stored folder path has a NUL" everywhere (`ledger.rs`, `lookup.rs`, and the `Stored` variant's sites in `removal.rs`). `platform/src/dir.rs`'s "the path has a NUL" is about a path the caller gave, not a stored one, so it keeps its own wording.

NUL-in-path messages differ: ledger.rs:269 is a failed file's folder holds a NUL with no prefix, ledger.rs:301 is a failed folder's path holds a NUL, lookup.rs:102 is a stored folder path has a NUL, removal.rs:194 is a removal's folder has a NUL, and dir.rs:255 is the path has a NUL. Four wordings for one condition.

### E-053 [low] Command-line and option error phrasing varies — CONFIRMED

**Disposition: No change.** The cited command-line messages are already in the chosen form.

`command_line.rs:233` says does not read as a query, :240 says is not a finite row height, :255 says is not a mascot: 1 to 40, and :291 says is not an option the window reads. The phrasing varies by option, and the error for a bare option is a fourth form.

### E-054 [low] Tool exit and unknown-step conventions vary — CONFIRMED

**Disposition: Fixed.** bench_data.py and perf-compare.py exit with named messages (the pillow_pin item does not exist).

`bench_data.py:152` and perf-compare.py:105-107 print to stderr and return 1. `ci/pillow_pin.py:31` exits with the pip return code. linux.py:189 and `win_build.py:194` use sys.exit with a message, and `win_build.py` names the program while linux.py does not. Exit conventions differ.

### E-055 [low] Saturating conversions clamp overflow silently — PARTIAL

**Disposition: No change.** The saturation is deliberate and commented where it matters.

perf/lib.rs:277 saturates a duration to u64::MAX on purpose, and platform clock.rs:51 documents its u16 fallback for a year past 65535. tiff.rs:96 saturates an entry count to usize::MAX. The saturation is deliberate and commented in the clock case, so only the undocumented ones are a problem.

### E-056 [low] Build script failure style varies — CONFIRMED

**Disposition: Fixed.** video build.rs returns Result from main; desktop build.rs already had no panics.

video/build.rs:207 uses `unwrap_or_else` with panic! for `read_dir`, and :229-230 use panic! for copy, while :137 returns Err(String) for pkg-config failures. desktop/build.rs:4 returns `Result<(`), `String>` from main. Two failure styles are used in build scripts.

### E-057 [low] Test expect and unwrap messages drift — CONFIRMED

**Disposition: Fixed.** The bare `unwrap` the audit cites (the reload's generation in `client/tests/controller/listing.rs`) is an `expect` with its reason. The test files' other `unwrap`s are allowed in tests by `clippy.toml`, and their assertion messages are the test's own; they are not changed.

Test messages drift: controller listing.rs:20 expects a row height in range, size.rs:28 expects a factor and `controller_laws.rs:50` expects a positive factor, size.rs:16 expects a positive scale, and listing.rs:180 uses a bare unwrap where others use expect. Low severity.

### E-058 [low] Win32 literal and hand-written Display conventions in platform — CONFIRMED

**Disposition: Fixed.** dir.rs's non-Linux stub uses io::Error::new.

NormalizeError in names.rs:148-150 has a hand-written Display, as native.rs and picker.rs do. dir.rs:93 builds Unsupported with ErrorKind.into(), while drive.rs:397 uses io::Error::new with a message. Low severity.

### E-059 [low] Turns and count decode limits differ from the documented range — PARTIAL

**Disposition: No change.** The limits apply to different things (files and pictures), so the ranges agree.

`union_find.rs:88` says a catalog holds fewer than 2^32 files, and join.rs:70 gives `MAX_PICTURES` as u32::MAX `>>` 4, which is 2^28 pictures. These are different limits on different things (files and pictures), so the documented ranges agree once the units are separated. The finding does not show a conflict.

### E-060 [low] Notice text overwrite on same-bytes refailure — CONFIRMED

**Disposition: Fixed.** A same-bytes refailure keeps the latest text and does not bump the count or count as news (test added and updated).

`NOTIFICATIONS_PLAN.md:61-65` records the design's undeclared deviations as open rows: N-35 `dismiss_all` deletes events, N-37 the dismissed-entry test moved, N-38 the cascades, N-39 the seven cache events against the designed five. The plan tracks them, so the deviations are known; the same-bytes refailure overwrite is still open per the finding.

### E-061 [low] Unplugged error classifier is duplicated by the import plan — PARTIAL

**Disposition: No change.** The removal is planned by `IMPORT_FAILURES_PLAN.md`, not an unnoticed second table.

`IMPORT_FAILURES_PLAN.md:208-212` says there is no existing OS-error classifier to reuse, notes the private `unplugged_code` table in drive.rs:54-68, and says the new table absorbs it. The duplication is planned for removal, not an unnoticed second table. drive.rs:56 is the literal 21, which the plan describes.

### E-062 [low] ExifTool stderr interpolated in messages and English stdout parsed — CONFIRMED

**Disposition: No change.** LANGUAGES.md:228 and :343 record the decision: stderr in the error, English stdout parsed.

exiftool/lib.rs:153 interpolates ExifTool stderr into the user-visible bail. LANGUAGES.md:102 records that ExifTool runs with no lang option and that success is parsed from English stdout. The English dependency is documented and open.

### E-063 [low] CUDA error text formats differ across harnesses — CONFIRMED

**Disposition: Fixed in part.** The C and C++ macros now write one format, `<expr> failed: <error>`: dec_gpu.cpp's CU() wrote `<expr>: <error>` and changed on 2026-11-21, and encode_gpu.cu already wrote this form. Both builds compile. The Python harnesses still differ (`cuInit returned N` in gpu.py and nvimgcodec_bench.py, `status N` in nvjpeg.py).

CUDA error text differs across harnesses: gpu.py:42 and `nvimgcodec_bench.py:44` use cuInit returned N, nvjpeg.py:104 uses X: status N, `dec_gpu.cpp:16` uses the macro name plus cudaGetErrorString, and `encode_gpu.cu:40` uses what failed: cudaGetErrorString. Low severity; harness only.

## config

### C-001 [high] dotenv quote handling differs between tools — CONFIRMED

**Disposition: Fixed.** `lint_qml.py` and `symbolize.py` now call `pgtools.load_env_file`, which strips quotes and honours `PG_CONTAINER`, as `profile.py` and the build tools already do. The PowerShell original `profile.ps1` is removed.

pgtools.py:55 strips the quotes from a .env value, as just does. `lint_qml.py:40` keeps the raw quoted value, symbolize.py:60 keeps the raw value with setdefault, and profile.ps1:22 keeps the raw value. A value written in quotes reads differently in these tools.

### C-002 [high] `SQLITE_DEFAULT_MEMSTATUS` stated with conflicting values across docs — CONFIRMED

**Disposition: Fixed.** The value the code asserts (`0`, in `catalog/src/core.rs`, the fork route's) is the one `BOUNDED_MEMORY_PLAN.md`'s forced flags now state. The other documents already said `0`.

`BOUNDED_MEMORY_PLAN.md:167` would force `SQLITE_DEFAULT_MEMSTATUS=1`. `SCAN_OPTIMIZATION_PLAN.md:112-114` says the fork route is resolved and that the two plans must keep one value. The code's test at core.rs:626 asserts `DEFAULT_MEMSTATUS=0`, and PROFILING.md:1090 and scan-optimizations-validation.md:287 use 0. `CATALOG_SIZE_PLAN.md:167` names the same flag. The bounded-memory plan still says 1.

### C-003 [high] Remembered-save debounce: accepted decision contradicts design and code — CONFIRMED

**Disposition: No change.** The decision doc and the design both say 1,000 ms; the owner should ratify the change to the accepted decision.

`WEB_CLIENT_TECH_DECISIONS.md:113` sets the remembered-save debounce at 500 ms (B15). `WEB_CLIENT_DESIGN.md:1186` and `WEB_CLIENT_RECONCILED.md:796` say SaveSession debounces 1,000 ms, and deadlines.rs:25 asserts the 1000 ms pause. The accepted decision contradicts the design and the code.

### C-004 [high] Web client security parameters disagree between decisions and design — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Argon2id 128 MiB, t=3, p=1 and a 90-day absolute session are the defaults; design and reconciled docs aligned. The owner may revisit.

`WEB_CLIENT_TECH_DECISIONS.md:147` accepts Argon2id at 128 MiB, t=3, p=1 (O3, measured at 0.59 s). `WEB_CLIENT_DESIGN.md:174` proposes 64 MiB (m=65536 KiB), and `WEB_CLIENT_RECONCILED.md:180` says 64 MiB as well. The accepted and proposed memory costs differ. The session lifetime also differs: `TECH_DECISIONS` says 90 days absolute at :146, and DESIGN.md:340 is cited at 180 days.

### C-005 [high] Pairing gate defaults differ between PowerShell and Python drivers — PARTIAL

**Disposition: No change.** `keep.py`'s header states the gate (`--part time --rule bounded --margin 0.10`), and `keep.ps1` always passes its values.

keep.ps1:10 defaults Rule to bounded and passes --rule and --margin explicitly at :129-130 (margin 0.10 unless the rule is margin). keep.py:105 and :107 default to margin 0.01 and rule margin. The defaults differ, but the PowerShell driver always passes its values, so the difference only shows in a direct Python run. The user's pairing memory records the bounded 0.10 rule as the gated one.

### C-006 [medium] `PG_*` environment reads scattered; plan and ledger counts stale — CONFIRMED

**Disposition: Fixed.** `pg_platform::env` is the one reader: `read`, `value`, `flag` and `process`, over the registry `VARS` (name, kind, reader, meaning). An empty value is unset for every variable. `Layout::resolve` reads its variables through the same readers, and the clap `env =` on `--catalog-dir` is gone, so it is read once. `PG_THUMB_CAP_GIB` and `--cap-gib` share `Cap::parse_gib`. The OneDrive and Dropbox roots use `dirs::user_folder` for `USERPROFILE`. ExifTool is resolved once per run into `pg_exiftool::Program` (the writer at start, the crop at each `save`/`cut`), so `save_with` and `cut_with` take the program and the crop tests need no environment. `boundaries.py` checks that production code reads no variable through `std::env`, and `test_ci_files.py` checks every `PG_*` name the sources use against `VARS`, `.env.example` or its test-input list. Exceptions, named in `ENV_READERS`: the build scripts and the one `set_var` for `QT_AUDIO_BACKEND`.

`PG_*` environment reads are spread across layout.rs:30-31, exiftool lib.rs:31, desktop lib.rs:157, video build.rs:64 and more. `REFACTOR_LEDGER.md:52` records this as an open shortcut (F-12), so the scatter is known and not yet consolidated.

### C-007 [medium] .env.example omits app variables that code and docs use — CONFIRMED

**Disposition: Fixed.** `.env.example` now has entries for `PG_PREFS`, `PG_PERF_LOG` and `PG_THUMB_CAP_GB`, each with what it does as the code reads it (`PG_PERF_LOG` records nothing unset; `PG_THUMB_CAP_GB` is read by the desktop app).

.env.example lists `PG_HOME` and `PG_CATALOG_DIR` in its app section (lines 29-33) but has no entry for `PG_PREFS`, `PG_THUMB_CAP_GB` or `PG_PERF_LOG`, which the crate README documents (crates/README.md:29, :290, :362). The example file omits app variables that the docs use.

### C-008 [medium] Env var prefix convention not followed for repo-owned keys — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Repo-owned `JPEG_*` keys are renamed `PG_JPEG_*` with no fallback (nothing in the repo set the old names; the docs now name the new ones); the repo-owned QT_DIR, LLVM_DIR and CUDA_ARCH are renamed PG_QT_DIR, PG_LLVM_DIR and PG_CUDA_ARCH with no fallback. CUDA_PATH, LIBCLANG_PATH and FFMPEG_DIR are kept, because upstream crates and installers read them under those names (ffmpeg-sys-next, clang-sys, NVIDIA's installer).

.env.example names `QT_DIR` (line 12), `LIBCLANG_PATH`, `FFMPEG_DIR`, `JPEG_EVAL` and `CUDA_PATH` without a prefix, while `PG_TEST_DIR` (line 38) and the `PG_` family are prefixed. tools/linux.py:344 and eval/jpeg-decoders/linux.py:42 add more unprefixed variables beside `PG_` ones.

### C-009 [medium] Boolean and flag env values parse differently — CONFIRMED

**Disposition: Fixed.** `pg_platform::env::flag` reads every Rust flag with one spelling set (`parse`), and a value outside it is `Parsed::Invalid`, which the caller reports once: `PG_DEV` as a bad option (logged at start), `PG_TRACE` in the start's problem message, and `PG_REQUIRE_TOOLS` on stderr (the tests then skip). `PG_FORCE_AV1` and `PG_SHARED_DEVICE` are read by the C++ `env_flag`, which has the same set and is held to it by `test_ci_files.py`; it stays silent on a bad value, since the window has no log at that point (the parser is the C++'s own, as the plan decided).

Flag variables parse differently: desktop trace.rs:25 accepts only the value 1 for `PG_TRACE`, `command_line.rs:329` accepts 1, 0, true and false for `PG_DEV`, `gpu_device.cpp:71` accepts only 0 for `PG_SHARED_DEVICE`, `gpu_device.cpp:43` reads `PG_FORCE_AV1` as an int, and exiftool lib.rs:295 treats `PG_REQUIRE_TOOLS` as on for any value.

### C-010 [medium] FFmpeg executable named two ways — CONFIRMED

**Disposition: Fixed.** `make_test_video.py` reads `PG_FFMPEG`, the name the Doctor, the Dockerfile and `.env.example` use, instead of `FFMPEG`.

`make_test_video.py:22` reads FFMPEG, while doctor.py:146, doctor.ps1:71 and the Dockerfile at :30 use `PG_FFMPEG`. The same executable is named two ways.

### C-011 [medium] JPEG input folder has two variable names — CONFIRMED

**Disposition: Fixed (roles documented).** The two names are different folders, not one folder under two names: `JPEG_SRC_DIR` is the read-only source photos, and `JPEG_INPUTS` (default `~/jpeg-inputs`) is the bench's working copies, which the inputs step fills from the source. `eval/jpeg-decoders/README.md` now says both.

The JPEG input folder has two variables: benches/ir.rs:12 reads `JPEG_INPUTS` (default /root/jpeg-inputs), and linux.py:367 reads `JPEG_SRC_DIR` for the inputs step. The eval README (line 3) and docker README (line 56) name `JPEG_SRC_DIR` for the same photos. The roles overlap and are documented in different places.

### C-012 [medium] Eval data-root variable named inconsistently — PARTIAL

**Disposition: No change.** `PG_DATA_ROOT` and `PG_ROOT` have different roles.

`PG_DATA_ROOT` is the dataset folder in common.py:48 and viewer.py:5. `PG_ROOT` is the base folder that the other default paths hang from (common.py `pg_path`). Both exist with different roles. The TIER2 plan line 17 cite does not name `PG_ROOT` as the data root, so that instance is wrong. The use of both is real but not one mistake.

### C-013 [medium] 5-second SQLite busy timeout written as literals in several places — CONFIRMED

**Disposition: Fixed.** `catalog/src/schema.rs`'s `BUSY_TIMEOUT_MS` is the one Rust value; `core.rs`'s read-only open uses it. The SQL `busy_timeout = 5000` in `SETTINGS` stays a literal (SQL cannot take a Rust constant here), and the constant's doc says the two change together. The `stmt_journal_probe` cite is not in the tree, so nothing is changed for it.

The 5-second busy timeout is written as a literal in several places: catalog schema.rs:116 (`busy_timeout` = 5000 in SETTINGS), core.rs:376 (Duration of 5 seconds), thumbcache schema.rs:90 (`BUSY_TIMEOUT`), and dbfile lock.rs:12 (100 tries of 50 ms, documented as about 5 s). inbox.rs:136 uses 100 ms for a different purpose. The `stmt_journal_probe` cite is missing from the working tree.

### C-014 [medium] `cache_size` sign and unit convention inconsistent — PARTIAL

**Disposition: No change.** The sign and unit convention is the same everywhere; only the types differ.

SQLite's `cache_size` takes a negative value for KiB, so the sign convention is the same everywhere. `READER_CACHE_KIB` (roots.rs:336) is a positive u32 and `PREPARE_CACHE_KIB` (types.rs:133) a positive i64, both negated at the pragma, and SETTINGS and schema.rs:430 use negative literals. The types and literals differ, but the units agree. The reader.rs:59 cite is a doc comment, not a `cache_size` value.

### C-015 [medium] `auto_vacuum` INCREMENTAL mode as bare literal 2 — CONFIRMED

**Disposition: Fixed (already).** `schema::AUTO_VACUUM_INCREMENTAL` is the one value; the migration's check, `removal.rs`'s check and the test use it. No bare `2` remains for this mode.

The `auto_vacuum` INCREMENTAL mode is the bare literal 2 in schema.rs:178, removal.rs:269 and the test at schema.rs:598. There is no named constant for it, so the magic number is repeated three times.

### C-016 [medium] Preference defaults and names disagree between code, docs and plans — PARTIAL

**Disposition: Fixed.** `LANGUAGES.md` says Prefs has eleven fields, matching the struct.

prefs.rs:122 sets `write_back` to true and `real_size` to false, and the settings bridge documents `detect_borders` as on by default. `DECODE_ONCE_PLAN.md:103` names the field `pregenerate_thumbnails` where the code has `thumb_cache`, which is a stale plan name. LANGUAGES.md:79 says the prefs hold seven fields, and that count was not checked against the struct here.

### C-017 [medium] Hard-coded D:/PhotoGallery defaults in tools and eval — CONFIRMED

**Disposition: Fixed in part.** test_encoder.cpp, build.ps1 and test_linux.py read PG_HOME; the build-jpeg-turbo.ps1 original is removed.

build-jpeg-turbo.ps1:12 defaults Work to a D: PhotoGallery static folder, and :13 defaults BuildTools to a Visual Studio 18 path. `test_encoder.cpp:43` hard-codes a D: PhotoGallery ssimulacra2 path, and :41-42 hard-code a per-user scratch path. Python tools derive the same defaults from `PG_HOME` (`build_static_qt.py:182`), so the defaults come from two sources.

### C-018 [medium] FFmpeg default prefix differs by tool and OS — CONFIRMED

**Disposition: Fixed.** compare_shots.py uses FFMPEG_DIR, or .deps/ffmpeg on Windows, as doctor.py and build.rs do.

The FFmpeg default prefix differs: doctor.py:55 uses .deps/ffmpeg only on Windows and otherwise pkg-config, doctor.ps1:49 always uses .deps/ffmpeg, `compare_shots.py:670` always uses the repo .deps/ffmpeg, and video build.rs:102 uses .deps/ffmpeg on Windows and pkg-config elsewhere.

### C-019 [medium] Two clippy.toml files that must be kept in step — PARTIAL

**Disposition: No change.** The file names the repetition itself, and the two files agree.

model/clippy.toml:1-3 repeats the root's hash-collection bans and `allow-*-in-tests` settings, and asks to keep them in step by hand. `CLIPPY_LEDGER.md:15` says clippy.toml bans HashMap for the whole workspace, which is true only through the root file. The repetition is a maintenance risk the file itself names; the two files agree today.

### C-020 [medium] Fixture module gating disagrees with its doc — CONFIRMED

**Disposition: Fixed.** The module doc now says that `raw` is for this crate's own tests only (it is `cfg(test)`); the other fixture modules are behind the `fixture` feature through the module's parent, as the doc says.

fixture.rs:1 says the fixtures are behind the fixture feature, but fixture.rs:4 gates raw with cfg(test) while the pub mod state is not gated. browse.rs:8 gates its fixtures with cfg(test) only. The gating does not match the module doc.

### C-021 [medium] Ctrl modifier gating differs between grid and viewer zoom — PARTIAL

**Disposition: No change.** The gating differs by action, by design.

keys.rs:105-106 uses Ctrl for the grid's Plus and Minus (row resize), keys.rs:134-135 uses plain keys for viewer zoom, and keys.rs:117 gives B no modifier check. The actions differ, so the gating differs by design; the finding does not show a shortcut that is wrong.

### C-022 [medium] Web client numeric limits disagree between design and reconciled docs — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Each limit is stated once in WEB_CLIENT.md (2 held polls, 256 and 2,048 miss queue, 8,192 px fit, selection 128 KiB, mutations 64 KiB with smaller per-route caps); the client clamps to 8,192.

The web client's numeric limits disagree. DESIGN.md:1297 allows 4 held polls per device, while RECONCILED.md:261 says at most 2. DESIGN.md:1501 has a 2,000-miss queue, and RECONCILED.md:338 has 256 per device and 2,048 globally. DESIGN.md:1506 allows fit sides of 16,384 px, while RECONCILED.md:1067 says 8,192 px. DESIGN.md:1565 gives 64 KiB, and RECONCILED.md:461 gives 128 KiB for selections.

### C-023 [medium] Stop-wait timings differ across app and library — PARTIAL

**Disposition: No change.** Each wait is for its own purpose, so the values are not meant to match.

The stop waits are per purpose: yandex.rs:102 `STOP_WAIT` 2 s, library.rs:34 `JOB_STOP_WAIT` 5 s, inbox.rs:142 `SHUTDOWN_DRAIN` 5 s, and coordinator.rs:47 `RETIRE_WAIT` 10 s. Their names describe different waits, so the values are not meant to match. The naming is mixed, and the values are not a conflict.

### C-024 [low] Batch and bench variable names overlap — PARTIAL

**Disposition: No change.** The variables are read at build time or at run time, respectively.

`PG_BENCH_DATA` and `PG_BENCH_JPEG` are read at build time (catalog.rs:28 and linux.rs:23), `PG_BENCH_DATA_SOURCE` is a runtime .env key (docker README:52), and `PG_FFMPEG_BIN` is baked in at build time (load.rs:171). The overlap the finding names is real, but these are build-time versus run-time reads of different variables, not a single conflict.

### C-025 [low] Yandex fixture flag missing from test hooks list — PARTIAL

**Disposition: No change.** The flag is documented at `crates/README.md:111`.

crates/README.md:111 says Config::yandex is set by a recorded session that --yandex-fixture names. The hook list at README.md:509 is the screenshot hook list and does not include the flag. The flag is documented at :111, so the omission is from a separate list, not a missing doc.

### C-026 [low] Subprocess holder env protocol uses different prefixes per crate — CONFIRMED

**Disposition: No change.** The platform child-process test uses stdin EOF and a folder prefix; the holder variables are the catalog and dbfile tests' shared names.

The subprocess holder protocol uses a different prefix per crate: catalog `other_process.rs:32-34` sets `PG_CATALOG_HOLDER`, MODE, READY, and dbfile live.rs:31-33 sets `PG_LIVE_DB`, MODE, READY, and platform testing tests.rs:13 uses `PG_PLATFORM_TESTING_CHILD_PREFIX`. The same protocol has three naming schemes.

### C-027 [low] Probe environment variables documented incompletely — CONFIRMED

**Disposition: Fixed (already).** `library/examples/scan_probe.rs`'s doc names `PG_PROBE_AGAIN` and `PG_PROBE_BETWEEN` and what each does, and `PG_TEST_DIR` for the run folder. The `pairs_load_probe.rs` cite is not in the tree.

`scan_probe.rs:6-8` says the probe uses `PG_TEST_DIR` only, but the probe reads `PG_PROBE_AGAIN` and `PG_PROBE_BETWEEN`. pairs.rs:41-44 lists the `PG_PROBE_*` variables it reads, so that doc is complete. The `pairs_load_probe.rs:154` cite is missing from the tree.

### C-028 [low] dotenv container skip only in pgtools — CONFIRMED

**Disposition: Fixed.** `lint_qml.py` now loads the `.env` through `pgtools.load_env_file`, which skips it under `PG_CONTAINER` (see C-001), so there is one loader with the container rule.

pgtools.py:47 skips the .env file when `PG_CONTAINER` is set, but `lint_qml.py:32` has no container check and loads the file anyway. The two `load_env_file` functions differ. Low impact, since the container has no .env.

### C-029 [low] Stale file reference for the cap environment variable — REFUTED

`NOTIFICATIONS_DESIGN.md:323` attributes `PG_THUMB_CAP_GB` to lib.rs run, not app.rs. The finding's claim that the doc says app.rs is not in the doc. The desktop reader is desktop lib.rs:157, which matches the doc.

### C-030 [low] Eval scripts use different random seeds by default — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Seed values kept; each is a named constant with the baseline it reproduces or a "dataset seed" note.

Eval scripts use different seeds by default: `make_subset.py:20` uses 1, pdq.py:153, synth.py:51 and `bench_decode.py:146` use 0, `bench_semantic.py:103` uses random.Random(4), and ann/data.py:84 uses `default_rng(0`) while seeds 1 to 5 are used elsewhere. Low severity.

### C-031 [low] Eval worker, batch and budget defaults differ across scripts — CONFIRMED

**Disposition: No change.** Per-workload worker defaults stay; the nvImageCodec budget change (2.0 to 1.5 s) is recorded here.

Worker and budget defaults differ: `bench_decode.py:144` defaults processes to 12, `bench_thumbs.py:334` defaults workers to 10, pdq.py:154 defaults workers to 6, plant.py:54 defaults workers to 12, and gpu.py:31 and `gpu_encoders.py:26` set `BUDGET_S` to 1.5 and 2.0. Low severity.

### C-032 [low] Verifier thresholds repeated as argparse defaults — REFUTED

group.py:160-161 uses the constants `SHORTCUT_PDQ` and `SHORTCUT_SIM` as the argparse defaults, so the values are not repeated; they are referenced. The constants are defined once at :36-38.

### C-033 [low] Profiling tool install defaults repeated across scripts — CONFIRMED

**Disposition: Fixed.** `tools/pgtools.py` now has `default_llvm_dir()` and `default_pin_root()`, and `tools/profiling/callgraph/build.py`, `profile.py` and `symbolize.py` (`llvm_dir`) call them instead of repeating the paths. `profile.ps1` keeps its own Pin literal because a PowerShell script cannot import pgtools, and the pgtools comment names it. `build.sh` no longer exists (the build is `build.py`, per its docstring), so that cite is gone; no behaviour changes.

The profiling install defaults repeat: profile.py:52 and profile.ps1:27 both default `PIN_EXE` to a LOCALAPPDATA Programs sde pinkit path, build.py:66 defaults `LLVM_DIR` under PROGRAMFILES, build.sh:22 does the same, and symbolize.py:78 repeats it. Low severity.

### C-034 [low] Windows Qt font folder fallbacks differ — CONFIRMED

**Disposition: Fixed.** All three now use WINDIR, falling back to `C:/Windows` (the fallback `tools/qmltest.py` already had): `use_windows_fonts_for_offscreen` in `crates/desktop/cpp/gui_app.cpp` and the QT_QPA_FONTDIR line in `tools/qmltest.ps1`. Behaviour change: with WINDIR unset, the app and the PowerShell runner now get `C:/Windows/Fonts` rather than no font folder (the PowerShell runner used to fail on a null path); a run with WINDIR set is unchanged.

qmltest.py:98 falls back to C:/Windows for WINDIR, qmltest.ps1:45 has no fallback, and `gui_app.cpp:29` uses the variable with no fallback. Low severity.

### C-035 [low] PowerShell error preference differs between tools — PARTIAL

**Disposition: No change.** The two scripts have different jobs, so the setting differs by design.

doctor.ps1:7 sets ErrorActionPreference to Continue because it reports and keeps going, and profile.ps1:19 sets Stop because a failed profile run should end. The two scripts have different jobs, so the difference is by design.

### C-036 [low] Same-named constants defined in two crates — PARTIAL

**Disposition: No change.** The values agree.

`LEDGER_EVERY` is 2 s in thumbs lib.rs:34, and the catalog ledger doc at ledger.rs:52 says 2 s too. The values agree, so the finding's conflict is not there. The inbox.rs:35 doc cites `pg_app::inbox::LISTED`, which the cite does not show, so that part is unchecked.

### C-037 [low] Writeback test asserts retry cap as literal 5 — CONFIRMED

**Disposition: Fixed.** `crates/library/tests/writeback.rs` (the finishing-copy test, "waits at the cap") now asserts `attempts()` equals `pg_catalog::queue::MAX_ATTEMPTS` instead of the literal 5, matching the other cap checks in the same file. Test-only change; no product behaviour changes.

writeback.rs:387 waits for attempts of at least 5, a literal. writeback.rs:664 uses `pg_catalog::queue::MAX_ATTEMPTS` for the same cap. The test uses the literal in one place and the constant in another.

### C-038 [low] Thumbcache shrink threshold differs under test — REFUTED

`SHRINK_ABOVE_PAGES` is 16 under cfg(test) and 16384 otherwise (maintenance.rs:13). A test-only threshold is the usual pattern, so the difference is intended.

### C-039 [low] Retry and wait intervals repeat the 50 ms literal — PARTIAL

**Disposition: No change.** Each constant is for its own wait, so they need not change together.

The 50 ms value appears as `RETRY_EVERY` (transaction.rs:40), `RETRY_PAUSE` (inbox.rs:139) and a literal at library inner.rs:128. The pipeline.rs:644 cite is missing. Each constant is for its own wait, so the repetition is not one value that must change together.

### C-040 [low] Time constants use mixed types — CONFIRMED

**Disposition: No change.** TOUCH_AGE is a Duration; the SQL-bound i64 constants are named and explained.

Time constants use mixed types: `TOUCH_AGE_S` is an i64 of seconds (config.rs:67), `TOUCH_FLUSH` is a Duration (config.rs:71), and `SCAN_LEASE_SECS` is an i64 of seconds (types.rs:137). Low severity.

### C-041 [low] CLI defaults repeat pipeline defaults — CONFIRMED

**Disposition: No change.** --readers follows pipeline::READERS; the --buffer-mb help stays prose.

The CLI repeats pipeline defaults as literals: args.rs:34 has `default_value_t` 8 for readers, and pipeline.rs:55 sets readers to 8 and budget to 8. The 12 MB buffer default is stated in the CLI doc and computed in pipeline.rs:75.

### C-042 [low] Similar-pages count differs between crawler and app — PARTIAL

**Disposition: No change.** The doc names both figures and says which one was measured.

`REVERSE_SEARCH.md:72` says the crawler script sets 20 pages, while the measured runs used 5, and the app's `DEFAULT_MAX_SIMILAR_PAGES` is 5 (yandex client.rs:25). The doc names both figures and says which one was measured, so the difference is recorded, not hidden.

### C-043 [low] Planned gallery.json keys do not match the flat preferences schema — PARTIAL

**Disposition: No change.** The nested keys are proposals in a plan, not the shipped file.

`BOUNDED_MEMORY_PLAN.md:1537` and :1889 plan nested gallery.json keys (`grid.range_indexes`, `journal.trust_detachable`), and LANGUAGES.md:79 says the prefs schema is flat. The nested keys are proposals for a schema that does not exist yet, so the mismatch is with a plan, not with the shipped file.

### C-044 [low] Eval viewer forces dark-only tokens — REFUTED

eval/viewer.html:8 says Dark theme always, and color-scheme dark at :10 is the intent of that file. site/style.css is the published site and has its own light tokens. Two pages with two themes is deliberate.

### C-045 [low] Data folder names differ by platform — PARTIAL

**Disposition: No change.** Each platform follows its own convention, as intended.

dirs.rs:32 uses PhotoGallery under LOCALAPPDATA on Windows, and dirs.rs:37 uses photo-gallery under `XDG_DATA_HOME` or HOME on Linux. Each follows its platform's convention, which is intended; the names differ by platform, as in N-033.

### C-046 [low] Yandex preview flag handled two ways in one parser — CONFIRMED

**Disposition: Fixed.** In `crates/desktop/src/command_line.rs`, the silent `.filter(...)` on the `yandex_preview` field is gone; the field is parsed plainly and `hooks` is now `let mut`. The one existing check after the struct, which already reports `--yandex-preview` as bad without `--find-similar`, now also sets `hooks.yandex_preview = None`, so the reported option is the one that is dropped. Behaviour is unchanged: the option is still ignored without `--find-similar`, and it is still named in the bad-option list.

`command_line.rs:263` silently filters the yandex-preview option when find-similar is absent. `command_line.rs:278-279` reports the same condition as a bad option. The same parser handles one condition two ways.

### C-047 [low] Windows setup defaults repeated in cmd and python — CONFIRMED

**Disposition: Fixed.** `eval/jpeg-decoders/win-env.cmd` now sets `ProgramFiles(x86)` to `C:\Program Files (x86)` when it is unset, the same default `eval/jpeg-decoders/win_env.py` uses for `PROGRAMFILES(X86)`. The two scripts now agree on the one default. Behaviour change: with the variable unset, win-env.cmd now looks for vswhere under the default folder instead of failing on a path that starts with a backslash; a normal shell is unchanged.

`win_env.py:52` falls back to C:/Program Files (x86) when PROGRAMFILES(X86) is unset, while win-env.cmd:9 uses the variable with no fallback. The two Windows setup scripts do not share the default.

## units-types

### U-001 [medium] Decimal and binary MB/GB labels mixed in code and tools — CONFIRMED

**Disposition: Fixed in part.** Byte counts and budgets are binary (cli --cap-gib, thumbcache Cap::from_gib, cli GiB labels); query MB labels and the Cap::from_gb docs elsewhere are not all done.

cli lib.rs:237-238 gigabytes() divides by 1e9 (decimal) for cache totals, while the cli test at lib.rs:986 builds `Cap::from_bytes(1` `<<` 30) (binary GiB). The cite at lib.rs:960 is a different test. lifecycle.rs:268 MB is decimal, query lib.rs:88 MB is binary, report.rs:111 mbps is decimal, rehearsal.rs:46 is MiB, eval/ann/report.py:44 gb is `2**30`. `dhat_top.py` is at tools/profiling/pairing/, not tools/profiling/.

### U-002 [medium] Docs give the same size figures in MB and MiB — CONFIRMED

**Disposition: Fixed.** IO.md and CATALOG_SIZE_PLAN use IEC labels for byte counts; decimal only for rates.

IO.md:42 labels 240 MB as 12 MiB x (8+12). 12 MiB x 20 = 240 MiB, so the unit is wrong, and IO.md:55 and :64 repeat 240 MB. `CATALOG_SIZE_PLAN.md:1` title uses decimal MB (519 MB) while its body uses MiB.

### U-003 [medium] Comments state units or contracts the code does not follow — PARTIAL

**Disposition: Fixed.** The `check_len` doc says longer buffers are tolerated, the `date.rs` doc notes the leap second, the join doc separates pictures from query entries, and the gif limit is attributed to `image`.

Mixed result across the instances. REFUTED: reader.rs:37 `GRID_CACHE_KIB` = `256*1024` KiB matches its KiB doc, and the 120 MB at :186-189 is a measurement, not the constant's unit. REFUTED: `shell_bridge.rs:182` (1 clockwise, -1 counterclockwise) matches controller.rs:388. CONFIRMED: convert.rs:146-150 says decoders give exactly the expected size and anything else is a bug, but `check_len` (:149-150) accepts len `>=` expected. CONFIRMED: date.rs:33 says impossible times give None, but :44 accepts seconds 0..=60 with no note on leap seconds. CONFIRMED: join.rs:309 documents `FEW_BASE` + `FEW_PER_THREAD` × T as `few_entries_on`, but :338 multiplies by `QUERY_ORIENTATIONS`. PARTIAL: gif.rs:21 cites gif's 50 MB limit, which belongs to the image crate, and codec/src/limits.rs does not exist. `NOTIFICATIONS_DESIGN.md:228` says paths are String, and :240 gives folder as `Vec<u8>`, which is a field-level exception the doc does not mark. verify.py:199 does not exist. Viewport swapsAxes range (0-3) not checked against playerTurns.

### U-004 [medium] Quarter-turn rotation typed as u8, i64, i32 or Turns, with literal modulus 4 — CONFIRMED

**Disposition: Fixed.** The client carries `Turns` end to end. `Event::Turn` and `Command::Turn` hold `Turns`; the signed count from the QML is converted once, in `Shell::turn`, and `GalleryModel::rotate` takes `Turns`. `Turns` serialises as its number of turns and reads a signed count (`-1` is three), so a recording of the old form replays (`a_recorded_signed_turn_reads_as_its_quarter_turns`). The QML still sends `int`.

`file_rotation` returns u8 (fixture/state.rs:88), `analysis_rotation` returns i64 (:189), `set_analysis_rotation` takes u8, lookup.rs:45 compares via i64::from, model image.rs:24 parses u8, Turns::new takes u8 and reduces mod 4 (turns.rs:13-14), queue.rs:113-114 and browse state.rs:85 repeat % 4 inline, and client controller.rs:113 carries i32 turns.

### U-005 [medium] Pixel dimensions held as u32, usize and i64 across layers — CONFIRMED

**Disposition: Fixed.** Pixel sizes are u32 in the client and desktop viewports; usize only where an index takes them.

Pixel sizes are u32 (analyze.rs:164, analysis.rs:37, CropSource types.rs:443), usize (borders.rs:144 converts with `try_from`, client image.rs:138, render display.rs:77), and i64 (client viewport.rs:15, desktop viewport/state.rs:19). lookup.rs:173 reads them as u32 while thumbcache lib.rs:179 reads height as u32 and reader.rs:73 binds i64 for the same column.

### U-006 [medium] Grid picture height typed differently in app, library, prefs and client — CONFIRMED

**Disposition: Fixed.** yandex start takes RowHeight; prefs is the one file-boundary conversion.

yandex.rs:119 takes `grid_height` as usize, prefs.rs:89 deserialises row height as f64, client controller/size.rs:79 uses f64 arithmetic with RowHeight, and model size.rs:10 gives `ROW_HEIGHT` and `THUMB_HEIGHT` as u32 ranges. The cited library/src/library.rs does not exist; ThumbHeight is used in inner.rs, runner.rs and shared.rs.

### U-007 [medium] File size held as u64, i64 or usize across structs and crates — CONFIRMED

**Disposition: Fixed.** Sizes are u64 in platform, scan, import, model and the catalog types; i64 only at the SQL edge.

Entry.size is u64 (platform listing.rs:17), `size_and_time` returns i64 (file.rs:154), ScannedFile.size is u64 while Prior, FolderRow, WorkItem and CropSource hold i64 (types.rs:321, 338, 363, 407, 439), writers.rs:37 converts with `i64::try_from`, and import lib.rs:100 uses `unwrap_or(0`) on a u64 size. FileState.size is i64 (model state.rs:18).

### U-008 [medium] File and volume identities held as u64 in model and i64 in storage — CONFIRMED

**Disposition: Fixed.** One identity cast helper; VolumeSet and Place carry u64.

FileIdentity volume and `file_index` are u64 (model ids.rs:37-38) with the doc claiming 64 bits, while writers.rs:36 casts them with `cast_signed` to i64, pair.rs:64 holds i64, roots.rs:274 converts with `cast_signed`, and lanes.rs:22 Place::Mounted carries volume: i64. `MULTI_DRIVE_LANES_PLAN.md:144` keeps u64 with an i64 cast at the SQL edge.

### U-009 [medium] File id typed differently across the Qt and QML boundary — CONFIRMED

**Disposition: Fixed.** QML file ids are real; the Rust side converts with a checked conversion.

`failed_files.rs:83` sends fileId as f64, shell.rs:335 takes `file_id`: f64, `video_source.rs:31` takes i64, and the QML side mixes them: InboxPanel.qml:414 is real, Main.qml:351 revealFile(fileId: real), while Main.qml:372 and :377 use fileId: int. `WEB_CLIENT_DESIGN.md:944` gives `file_id` as i64 and :1479 gives u64.

### U-010 [medium] Count fields typed as usize, u64 or i64 across layers — CONFIRMED

**Disposition: Fixed.** Counts are u64 in the catalog and scan; SQL counts converted once.

Counts are u64 (ledger/state.rs:65-67), i64 (lookup.rs:199, one()), usize (pair.rs:48 rebound and replaced), u64 with lost (inbox.rs:814), and usize for scan missing (scan lib.rs:279) while the other scan counts are u64 (:277, :281). `writes_remaining` is usize (queue.rs:194) while related rows are i64.

### U-011 [medium] The same integer conversion falls back to different values — CONFIRMED

**Disposition: Fixed.** Progress counters saturate; a negative SQL count is an error; one rule per kind.

The same conversion falls back to different values: inbox.rs:1084-1085 count saturates to u64::MAX, catalog inbox.rs:335 unread uses u64::MAX, rebuild.rs:20-21 count uses `unwrap_or(0`) for negatives, removal.rs:195 uses `unwrap_or(0`), hashes.rs:87-88 uses u64::MAX for the item count and 0 for the failed count, scan lib.rs:1002 uses u64::MAX, and desktop `facet_choices.rs:30-31` clamps negatives to 0 and saturates to i32::MAX while shell.rs:61 maps overflow to -1. The cited app/src/pairing.rs does not exist.

### U-012 [medium] Absent or unselected values use different sentinels — CONFIRMED

**Disposition: Fixed.** -1 for an absent index; Main.qml uses a loadPending flag; zoom and mascot have explicit unset states.

Absent values use -1 (`command_line.rs:455`, yandex.rs:99, `video_player_bridge.rs:16`, LogPanel.qml:27), -2 (Main.qml:78 loadedIndex), and 0 for zoom and mascot (`command_line.rs:465-472`). selectedSeq is a real -1 next to an int selected (LogPanel.qml:26-27).

### U-013 [medium] `Duration_ms` width, clamp and sentinel differ across model and catalog — CONFIRMED

**Disposition: Fixed.** VideoFacts duration and poster are u32 with NO_DURATION; one checked conversion at the catalog read.

model grid.rs:24 holds `duration_ms` as u32 with `NO_DURATION` = u32::MAX (:55), model analysis.rs:103-104 holds VideoFacts `duration_ms` and `poster_ms` as i64, and catalog browse grid.rs:65 clamps an i64 column into u32 with a fallback. `WEB_CLIENT_RECONCILED.md:420` lists the GridRecord fields with no duration, while DESIGN:1479 keeps `duration_ms` u32.

### U-014 [medium] Timestamp units and mtime meaning left implicit or mixed — CONFIRMED

**Disposition: Fixed.** `FileState.mtime`'s doc (`model/src/state.rs`) states the unit the platform listing gives it: FILETIME ticks, 100 ns since 1601-01-01 UTC. The cited `catalog/src/failure.rs`, `library/src/crop.rs` and `stmt_journal_probe.rs` are not in the tree.

model state.rs:15 says mtime: i64 with no unit, while platform listing.rs:19 documents it as FILETIME ticks (100 ns since 1601). file.rs:150 saturates on Windows with `unwrap_or(i64::MAX`). The cited catalog/src/failure.rs, library/src/crop.rs and `stmt_journal_probe.rs` do not exist, so those cites are dropped.

### U-015 [medium] `unix_now` defined three times with different fallbacks — CONFIRMED

**Disposition: Fixed.** `pg_platform::clock::unix_now` is the one definition (the same fallback in all three: 0 before the epoch, saturating past `i64::MAX`). `borders`, `thumbcache` and `thumbs` re-export it in place of their copies. The app's copy the audit cites is not in the tree.

`unix_now` is defined three times with the same body: app inbox.rs:1089 with `map_or(0`), borders lib.rs:41 with `map_or(0`), thumbcache maintenance.rs:32-33 with `map_or(0`). The fallback is 0 on a pre-epoch clock in all three, and the same `try_from(...).unwrap_or(i64::MAX`) appears in each. Duplicated, not divergent, so the title's 'different fallbacks' is not borne out.

### U-016 [medium] `process_age_ms` baseline differs by operating system — CONFIRMED

**Disposition: Documented, not changed.** The two baselines are different measurements (the process's creation on Windows, the first call elsewhere), and `platform/src/clock.rs` documents both arms. Choosing one would change a measurement the eval relies on, so the difference stays, stated where it is made.

clock.rs:58-59 documents `process_age_ms` on Windows as the age since process creation, and clock.rs:95 gives the age since the first call elsewhere. Both arms document the difference. The same name measures different things, which the doc states.

### U-017 [medium] Benchmark timing helpers return milliseconds or seconds under one name — CONFIRMED

**Disposition: Fixed.** The helpers are timed_ms and timed_s.

`bench_thumbs.py:212` timed returns milliseconds, `nvimgcodec_bench.py:56` returns ms per image, nvjpeg.py:217-219 returns seconds per item, sweep.ps1:54 converts seconds to ms, `scan_timing.py:55` prints seconds, and `cpu_worker.py:133` stores `build_s` in seconds. The gpu.py cite does not exist as eval/gpu.py; the file is under eval/jpeg-decoders/.

### U-018 [medium] Rounding policy differs in resize, decode and eval scale code — CONFIRMED

**Disposition: Fixed.** linear_resize rounds half up, as resize.rs does.

JPEG minimum-size fit uses ceil (codec decode.rs:228-229), resize uses round (resize.rs:34-35), and `linear_resize.py:129` uses Python round, which is banker's rounding, with a comment saying round half up. Three rules for the scaled size.

### U-019 [medium] 16-bit to 8-bit conversion uses two formulas by channel count — CONFIRMED

**Disposition: Fixed.** `luma16` uses the integer rule `rgb16` uses (`(v + 128) / 257`, which is `round(v × 255 / 65535)` exactly), so a 16-bit channel rounds one way whatever its channel count. The float route rounded halves differently at the boundaries.

luma16 scales in f32 by 1/65535 (convert.rs:174-178), and rgb16 uses integer (v + 128) / 257 (convert.rs:187). The doc at :185 says round(v × 255 / 65535), which the integer form only approximates. Two formulas for 16-to-8 conversion.

### U-020 [medium] Frame count held as i64 in video and u32 in metadata and model — CONFIRMED

**Disposition: Fixed.** video frames() returns u32 through a checked conversion.

video ffi.rs:231 frames() returns i64, metadata container.rs:25 holds frames as u32 (defaulting to 1), and model analysis.rs:40 holds frames as u32. The `VALUE_PROVENANCE_IDEAS.md:99` cite is accurate.

### U-021 [medium] Rating out of range is rejected in some places and clamped in others — CONFIRMED

**Disposition: No change.** The rating rule is stated once: reject user input, clamp file values at read, clamp only for display.

shell.rs:257 rejects out-of-range ratings via `u8::try_from` and Rating::new (None means nothing is rated). words.rs:14 clamps to 0..=5, client text.rs:33 shows past 5 as the best symbol, and metadata lib.rs:89 caps EXIF rating at 5 with min(5). Rejected in one place, clamped in others.

### U-022 [medium] Root, folder and file paths typed as String, RulePath or `Vec<u8>` — CONFIRMED

**Disposition: Fixed.** RulePath in scan Busy, catalog ScanBusy and RootRemoved, and FileFailure.path; the lossy conversion stays in platform args.

scan lib.rs:319-321 keeps ScanError::Busy with root and holder as String, while the walk uses RulePath. notice.rs:307 FileFailure.path is String, and notice.rs:452 ScanFailed root is RulePath. library state.rs:16-17 keeps `root_ids` and `root_paths` as RulePath. platform args.rs:18 converts Windows args lossily while the Unix form keeps exact bytes.

### U-023 [medium] Ext and Kind held as String in FolderRow but typed elsewhere — CONFIRMED

**Disposition: Fixed.** FolderRow holds typed Ext and Kind, built through ext_at and kind_at.

FolderRow stores ext and kind as String (catalog types.rs:361-362), while ScannedFile (:322), WorkItem and WriteTarget hold Ext and Kind. model media.rs:83 holds Ext as a u8 position in the lists.

### U-024 [medium] Unchecked arithmetic on unit conversions where others use checked — CONFIRMED

**Disposition: Fixed.** Timestamp conversions saturate; the CLI buffer size saturates.

cli args.rs:54 shifts mb `<<` 20 with no `checked_mul`, and `TYPE_MISMATCH_VALIDATION.md:68` says so. cli borders.rs:163 uses `checked_mul`. platform memory.rs:35 uses `checked_mul` and volume.rs:42-43 uses `checked_mul`, while platform file.rs:182 does plain multiplication on secs × `10_000_000`.

### U-025 [medium] Screenshot wait in milliseconds, documented in seconds — REFUTED

The seconds-to-milliseconds conversion is made at the boundary: screenshot.rs:60 is seconds × 1000.0 and screenshot.rs:24 passes millis(wait) to the C++ `max_ms`. The user-facing --wait is in seconds as documented (`command_line.rs:147`, TestHooks.qml:27), and the C++ parameter name says ms. The units are consistent at each layer.

### U-026 [medium] Digit grouping and count formatting differ across surfaces — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** One count formatter (words.count) is used by the HUD and the QML counts.

Hud.qml:58 concatenates raw numbers, GalleryGrid.qml:150-152 uses Qt.locale().toString for the same kind of count, and LANGUAGES.md:67 says Qt shows 1 234 while Rust shows 1,234 via `group_digits`.

### U-027 [medium] PDQ distance thresholds and metric written as literals in several places — CONFIRMED

**Disposition: Fixed.** Join tests use MATCH_DISTANCE; source notes in pdq.py and group.py; dups.rs uses the constant.

join.rs imports `MATCH_DISTANCE` (:58) and uses it in the production path (:154, :724), but the tests use a literal 31 at :2389, :2406 and :2663. The doc at :4-5 states the radius as 31. eval/pdq.py:29 has `MEANINGFUL_D` = 31, eval/group.py:38 has `SHORTCUT_PDQ` = 22, and cli dups.rs:19 hard-codes `PDQ_BANDS` with 31 as its top bound. The Hamming threshold is written in several places.

### U-028 [medium] Verification thresholds written as percent and as fraction — CONFIRMED

**Disposition: Fixed.** VERIFIER.md states thresholds as fractions; the 0.3% rule is removed.

VERIFIER.md:124 gives a changed region as 0.3% (percent) and median residual as 0.03 (fraction) in one sentence. VERIFIER.md:256 gives moved as about 1.2% and interior as about 15% while :276-278 use fractions 0.012 and 0.15. verify.py:556 uses fractions, with the comment's 0.05% matching 0.0005.

### U-029 [medium] CUDA kernels use different index and grid widths — CONFIRMED

**Disposition: Fixed in part.** EGPU_MAX_DIM is 65535 and k_gather64 is 64-bit. The k_dct launch's `(unsigned)nb` cast is now checked in `encode_chunk`: a block count over 0xFFFFFFFF is refused with EGPU_ERR_SIZE. test_encoder.exe builds on 2026-11-21. The other launches still use grid_exact or grid_colour (colour clamps with a grid-stride loop), so the two helpers stay; the encoder test run is not done.

`encode_gpu.cu:722` launches `k_dct` with a (unsigned)nb cast, while `grid_exact(nb`) (:532, used at :725 and :735) clamps with 64-bit math. `k_gather64` (:460-461) uses a uint32 index. `EGPU_MAX_DIM` is 65500u (`encode_gpu.h:23`) where JPEG allows 65535.

### U-030 [medium] Allocation accounting differs between the C and Rust PNG harnesses — CONFIRMED

**Disposition: Fixed in part.** alloc_count.cpp counts requested bytes. The C harness builds on 2026-11-21 (png-c-harness.exe, with alloc_count.cpp), so the two allocation counts can now be compared; that comparison is not run.

`alloc_count.cpp:26` counts live bytes with `_msize` (the usable block size), while the Rust harness main.rs:37 counts layout.size() (the requested size), including realloc `new_size`. The two harnesses count different quantities.

### U-031 [low] Time constants carry units by suffix or by Duration inconsistently — CONFIRMED

**Disposition: Fixed.** DISMISSED_FOR_SECS; the thumbcache constants were already Duration.

thumbcache config.rs:67 `TOUCH_AGE_S` is an i64 of seconds, while `TOUCH_FLUSH` (:71) and `HEAL_INTERVAL` (:74) are Duration. app images.rs:30 `WRITE_WAIT` is a Duration and `DECODES_KEPT` is a usize. catalog inbox.rs:67 `DISMISSED_FOR` is i64 seconds with no suffix.

### U-032 [low] GPU-time projection to 70M in hours or days — CONFIRMED

**Disposition: Fixed.** MODELS.md states the 70M projections in hours and the tables are recomputed.

MODELS.md:7 and :155 give GPU days for 70M, while :205 and :244 give GPU hours for 70M in the same document. `eval/bench_ann.py:186` prints GPU hours for the 70M projection. The cite path `eval/ann/bench_ann.py` is wrong; the file is `eval/bench_ann.py`.

### U-033 [low] Byte budgets typed usize or u64, and 16 MiB caps repeated — CONFIRMED

**Disposition: Fixed.** Budgets are u64 where a file or platform sets them; one shared 16 MiB decode cap; PERF_FILE_CAP.

pipeline.rs:37-38 `buffer_bytes` is usize, governor.rs wraps AtomicUsize, and coordinator.rs converts a u64 to usize with `unwrap_or(usize::MAX`). container lib.rs:107 `GATHER_CAP` is u64 = 16 `<<` 20, and container.rs:139 `MAX_INFLATED` is usize = 16 `<<` 20 under another name. log lib.rs:79 `RING_BYTES` is a usize 4 MiB, and log file.rs:26 `FILE_CAP` is a u64 of 5 MiB.

### U-034 [low] Viewport zoom tolerance written two ways — CONFIRMED

**Disposition: Fixed.** The fit tolerance is one named constant, `FIT_TOLERANCE` (`client/src/viewport.rs`), used by both the relative and the absolute comparison; the 1:1 test's `0.9999` is `ONE_TO_ONE`, a different value with its own name.

viewport.rs:163 uses a relative tolerance fit × 0.0001, viewport.rs:171 uses the same 0.0001 as an absolute tolerance on (`fit_zoom` - 1.0), and viewport.rs:98 uses zoom `>=` 0.9999 for pixels().

### U-035 [low] Instruction and cycle counts shown in millions and as raw values — CONFIRMED

**Disposition: Fixed in part.** medians.py uses one formatter with an M label; pairs_probe.rs prints its raw values as before.

callgrind/1bb039084.md:19 shows Ir as 93.26 M and 23.3 K per file, while docs/perf/baseline.json stores raw integers. `pairs_probe.rs:77-78` divides cycles by `1_000_000` and prints them as M, while medians.py:27 prints whole M cycles and keep.py reports percent to two decimals.

### U-036 [low] CLI and report precision for seconds differs by output — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Whole-pass seconds are one decimal through secs(); pair timings are whole milliseconds.

cli lib.rs:214 prints pair timings with two decimals, lib.rs:233 prints secs with one decimal, and report.rs:102, :113 print progress seconds with zero decimals. The cited lib.rs:205 is a return, not the timing print, so the cite is off, but the precision differs by output.

### U-037 [low] Numeric literal style and named constants used inconsistently — CONFIRMED

**Disposition: Fixed.** TIFF_SBYTE and TIFF_SSHORT are named; the date.rs grouping is fixed.

date.rs:54 writes days × `86_400` + h × 3600 + mi × 60 + s, grouping `86_400` but not 3600. metadata tiff.rs:119 uses bare TIFF type literals 1|2|6|7 etc., while named `TIFF_*` constants exist (:25-31).

### U-038 [low] JPEG width accepted as u32 in dimensions but u16 in decode — CONFIRMED

**Disposition: Fixed.** The u16 narrowing is one helper at the decoder boundary.

codecs/jpeg.rs:30-31 dimensions() narrows to u32 with `try_from`, and jpeg.rs:59 decode() narrows to u16 with the comment that a JPEG's sides fit 16 bits. The same dimension has two width types.

### U-039 [low] Encoder quality parameter types differ by codec — CONFIRMED

**Disposition: Fixed.** Every quality parameter in `crates/codec/src/encode.rs` is now `u8`, the type `jpeg_quality` already returns: `JpegEncoder::new`, `jpeg_cropped` and `WEBP_LOSSY_QUALITY` (now `u8` 90), and `webp_lossy_cropped`, which converts with `f32::from` only at the libwebp call. Callers updated in `crates/edit/src/crop.rs` (the `i32::from` conversion is gone), `crates/render/src/maker.rs` (`JPEG_QUALITY`) and `crates/render/src/transcode.rs` (`QUALITY`); `crates/codec/src/decode.rs` and the tests pass literals and need no change. The two libraries still use different scales (libjpeg 1–100, libwebp 0–100); that difference is documented on each function and is not something a plain integer type can express, so no behaviour changes.

encode.rs:32 JpegEncoder::new takes quality as i32 on libjpeg-turbo's 1-100 scale, while encode.rs:96 `WEBP_LOSSY_QUALITY` is an f32 90.0 on libwebp's 0-100 scale. Different integer and float types and scales.

### U-040 [low] Tree anchor bits saturate at 63 and alias — PARTIAL

**Disposition: No change.** The limit is documented in the code.

The aliasing is real: tree.rs:69-70 bit(rank) saturates at 63, so anchors 64 and later share bit 63. But the doc at :67-68 states it and gives the reason (no machine has that many drive letters), so it is a documented limit, not a contract violation.

### U-041 [low] Fixed-point scales in SQL sort and filter keys — CONFIRMED

**Disposition: Fixed.** The module has one fixed-point scale, millionths: the `AspectRatio` and `Borders` sort keys in `crates/query/src/lib.rs` both use `1e6`, and they already agreed. The `* 100` in `shape_sql` is a 1% margin, cross-multiplied to avoid division, not a scale; its doc comment now says so and points at the sort keys' scale. No SQL changed, so the sort order and the shape filters behave exactly as before.

query lib.rs:109-111 compares shape with values scaled by 100, and lib.rs:383 sorts AspectRatio as CAST(1e6 × a.width / a.height AS INTEGER), in millionths, while the Borders sort also uses millionths. Two fixed-point scales in one module.

### U-042 [low] Similarity band upper bound differs between eval tools — CONFIRMED

**Disposition: Fixed.** bench_ann's top band ends at 1.01; the 0.3 lower edge is deliberate.

eval/ann/metrics.py:20 has BANDS ending at 1.01 as edges, while `eval/bench_ann.py:194` has BANDS as (lo, hi) tuples ending at 1.0 (:194 is 0.8, 1.0). The cite path `eval/ann/bench_ann.py` is wrong. The two tools cut the similarity range differently.

### U-043 [low] Hand-written mouse wheel notch divisor — CONFIRMED

**Disposition: Fixed.** A new singleton `crates/desktop/qml/Wheel.qml` holds `notch: 120`, the way `Theme.qml` holds the colours; it is registered in `crates/desktop/build.rs` beside Theme. `WheelListView.qml`, `Viewer.qml` and `YandexPreview.qml` now divide `angleDelta` by `Wheel.notch`. The QML test staging (`tools/qmltest.py`) registers any `pragma Singleton` file automatically, so the tests need no change. Behaviour is unchanged (120 as before); the build and the QML tests are not run here.

WheelListView.qml:50, Viewer.qml:306 and YandexPreview.qml:65 each divide angleDelta by the literal 120 with no named constant.

### U-044 [low] ThumbHeight construction uses bare and unwrapped literals — CONFIRMED

**Disposition: No change.** The test setup's unwrap has a reason; the bench uses num::index for its ThumbHeight.

bench catalog.rs:45 defines `THUMB_HEIGHT` as u32 = 256 and wraps it in ThumbHeight::new. bench linux.rs:59 passes 256 as a bare literal to maker.make. library runner.rs:216 uses ThumbHeight::new(64).expect, while lanes.rs:75 uses .unwrap() on the same constructor.

## naming

### N-001 [high] Web client design doc and reconciled doc disagree on wire-level names — CONFIRMED

**Disposition: Fixed.** `docs/webclient/WEB_CLIENT_DESIGN.md` now uses the names decided in `WEB_CLIENT_TECH_DECISIONS.md` (B3, B4, B10): the session cookie is `__Host-gallery_session` with SameSite=Lax (the cookie decision and its Why), the device cookie is `gallery_device` at Path=/api/auth/login (the two-cookie decision), the CSRF header is `X-Pg-Client` (the cross-site writing decision), and the error model uses kebab-case codes in a `{"error": code}` body (bad_request also changed at the decode rule). The device route is moved to POST /api/auth/login to match its cookie path. `WEB_CLIENT_RECONCILED.md` already states the decided names at lines 165-170, so it is unchanged; its line 59 names X-Pg-Bundle as the CSRF header only inside a validator note that records the old writes position, and that note is kept. The design doc's other contradictions remain in its own validation lists.

`WEB_CLIENT_DESIGN.md` contradicts itself on the session cookie: :182 says SameSite=Strict, :715 and :1336 say Lax, :1691 rejects Strict. `WEB_CLIENT_RECONCILED.md:165` and :545 say Lax. Its cookie is `__Host-gallery_session` at :182 and `__Host-pg_sign` at :340. The CSRF header is X-Gallery-Request (design :1689), X-Pg-Client (reconciled :166) or X-Pg-Bundle (reconciled :59). Error codes are `snake_case` in design :1568 and kebab-case in reconciled :313 and :170.

### N-002 [high] Web server on-disk file names and lock location differ between docs — CONFIRMED

**Disposition: Fixed.** The docs now use one set of names, taken from `WEB_CLIENT_TECH_DECISIONS.md` B1 and B9 and from the reconciled lock placement (section 3.2): the lock is `<catalog_dir>/server.lock` (design 377, 1754; reconciled 657), the state database is `<home>/server.db` (design 330), and the one launch file is `<home>/launch.json` holding port, token and build identity (B9). The reconciled placement list and its launch-file and discovery steps (reconciled 168, 308, 643, 659, 692, 707-708, 770, 1105, 1108, 1117, 1133) and the design's discovery and cleanup lines (design 798, 813, 815, 818, 907, 1754, 1760) were changed to match. The two-file token and json scheme was dropped because B9 supersedes it, so the write ordering now needs one atomic write, not two. Validation-list entries that still quote the old names describe past conflicts and were left as written.

The server lock is `<home>/server.lock` at design :377, `<catalog>.server.lock` at :1754, and `<catalog_dir>/server.lock` at reconciled :515. The state database is server.sqlite at design :330 and server.db at design :31, :798 and reconciled :512. The design doc uses both names. The token and json files are at reconciled :308 and design :798.

### N-003 [high] Tool script names disagree across docs for the same gate and the same tool — CONFIRMED

**Disposition: Fixed.** The gate is `tools/perf_check.py` and the comparison it runs is `tools/perf-compare.py`, the split `tools/README.md` and `budgets.json` already use. `SCAN_OPTIMIZATION_PLAN.md` (D3) now names `perf_check.py` as the ±1% scan gate and `perf-compare.py` as the comparison, matching `NIGHTLY_AND_BUILD_FLAGS_PLAN.md`. `REVIEW.md`'s tools table now names `lint_qml.py`, the file that exists, in place of the nonexistent `lint-qml.ps1`. The open stale-name item in `REFACTOR_LEDGER.md` (F-58) is left as written, because it records the rename still to be made.

`tools/perf_check.py` and tools/perf-compare.py both exist. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:57` names `perf_check.py` as the 1% gate. `SCAN_OPTIMIZATION_PLAN.md:1020` names perf-compare.py as the scan gate. tools/README.md:24 describes perf-compare.py as the comparison, which `perf_check.py` calls. budgets.json:2 names both. tools/lint-qml.ps1 does not exist; the file is `tools/lint_qml.py`, which REVIEW.md:68 and `REFACTOR_LEDGER.md:149` cite as the old name.

### N-004 [medium] CLI flag spellings for the same action differ across docs and code — CONFIRMED

**Disposition: Fixed.** The shipped flags are the spelling everywhere: `crates/catalog/src/sql.rs` (the `stale!` doc comment) now says `--retry-errors`, the flag at `cli/src/args.rs:90-92`, instead of the nonexistent `--retry`. `DECODE_ONCE_PLAN.md` (section 5.11, its sketch and the `cli.ftl` line) now writes `--no-dups` and `no_dups`, the shipped flag and field at `args.rs:107`, in place of the planned `--no-duplicates`, and it says the field already exists so the sketch does not add it again. This is a doc-comment and plan text change only, so no behaviour changes.

The code's flag is --retry-errors (cli args.rs:90-92), and `IMPORT_FAILURES_PLAN.md:480` uses that. The doc comment at crates/catalog/src/sql.rs:6-7 says --retry, which is not a flag. The --again and --no-dups flags match the code. `DECODE_ONCE_PLAN.md:1132` sketches the field as `no_duplicates`, which is a design sketch and not the shipped name.

### N-005 [medium] Crate and package names are written three ways across code and docs — PARTIAL

**Disposition: No change.** Hyphens in package names and underscores in imports is the Rust convention.

crates/dedup/Cargo.toml names the package pg-dedup, so dups.rs:2 is right to write the hyphenated package name. The hyphen and underscore split is the Rust convention: package names are hyphenated and imports underscored. The real mixing is in tools/crate-layers.txt:32 (jobs-testkit against bare names). The finding's manifest instances (pg-cli, pg-log, pg-metadata, pg-model, pg-edit) are consistent.

### N-006 [medium] Notice name for a refused folder differs between the error map and the test — REFUTED

Both notice variants exist (notice.rs has FolderNotAdded and FolderRefused). error.rs:22 correctly names the refusal notice FolderRefused. The test at app.rs:274 posts FolderNotAdded as a generic sample notice about the shutdown, not as a refused folder, so it is not a naming mismatch.

### N-007 [medium] Found::Offline name does not cover undecodable originals — CONFIRMED

**Disposition: Fixed.** The variant is renamed `Found::Unavailable` in `crates/app/src/images.rs` (enum, the `undecodable` path, the maker-error path and the three match arms), so the name covers both an offline original and one that doesn't decode, as its doc comment says. The name `Unreadable` was avoided because `MakeError::Unreadable` already means the offline case. This is a rename inside one private enum with no behaviour change, and no other file uses the variant.

images.rs:59-66: Found::Offline(Error, `Option<ThumbHeight>`) is documented as covering the original that can't be read (offline) or that doesn't decode. The name covers only half of what the variant holds, as the finding says.

### N-008 [medium] Two different types named Listing are imported side by side in the app — CONFIRMED

**Disposition: Fixed.** The inbox's own type is renamed `InboxListing` (`crates/app/src/inbox.rs`, with its uses in the writer, the tests and `Inbox::listing`), so the saved session listing keeps the shared name `pg_model::view::Listing` that `views.rs` and the desktop controller use. The crate root re-export (`crates/app/src/lib.rs`) and the one consumer, `crates/desktop/src/notices.rs`, follow the rename. The change is a rename of a public type, which only the desktop crate uses, so it is the smallest fix that leaves one meaning per name. Nothing is compiled here, so the desktop and app builds should be the first check.

Two types named Listing are in the pg-app crate: inbox.rs:149 (the inbox's listing) and `pg_model::view::Listing`, imported at views.rs:8 (the saved session listing). Both are real and in use in one crate.

### N-009 [medium] Scratch folder base names are reused across unrelated tests — PARTIAL

**Disposition: No change.** Scratch names are labels; `run_folder` gives each call its own folder.

testkit's `run_folder(scratch`, prefix) makes distinct folders per call, and platform/src/testing/tests.rs:286 tests that calls made together are distinct. Reusing a scratch base is therefore a grouping label, not a collision. The real inconsistency is that `scratch_rate` is shared by unrelated tests (crop, `crop_tags`, `crop_roots`, writeback) while `scratch_crop` and `scratch_cli` are used elsewhere.

### N-010 [medium] Stored turns count is called rotation in some places and turns in others — CONFIRMED

**Disposition: Fixed.** The Rust fields named `rotation` (on `FileState`, `Version`, `Entry`, `thumbs::Source`, `WorkItem`, `Listed`, the fixture's `File` and the app's `Decoded`) are `turns`. The SQL columns, the write key `rotation` and the temp names are unchanged. The rule is in `crates/README.md` (Naming).

FileState.rotation (model state.rs:16) is the stored turns, and FileState.turns (state.rs:21) is the pending turns. write.rs:31 says the Turns write kind is stored under the key rotation. The same concept has two field names. failure.rs:18 (the finding's cite) does not exist, so that instance is wrong, though the rest holds.

### N-011 [medium] Catalog schema object names use inconsistent prefixes and are written in two files — CONFIRMED

**Disposition: Fixed.** Before the first release the baseline is edited in place (`schema.rs` module docs), so the names change in `crates/catalog/src/baseline.sql` with no migration. The clash is resolved by renaming the trigger that recomputes `pending` on a source change to `files_pending_changed` (the index `files_changed` keeps its name), and the two ledger triggers to `file_failures_stale` and `file_failures_hidden`, to match the `file_failures` table and its index and stamp. `TRIGGER_TESTS` in `schema.rs` and the comments in `ledger.rs`, `sql.rs` and `fixture.rs` follow, and `DATABASE.md` and `docs/perf/budgets.json` name the new trigger. Planning documents keep the old names where they describe history. The `schema.rs` trigger-coverage test is the first check when the catalog is built, since it compares these names with the schema.

Prefixes are mixed in baseline.sql: the table is `file_failures` (:120), the index `file_failures_live` (:233), the triggers `files_failures_stale` and `files_failures_hidden` (:433, :440), and `file_failures_stamp` (schema.rs:380). The triggers `files_changed` (:424) and the index `files_changed` (:241) share one name.

### N-012 [medium] Thumbs output version key and constant are named three ways — PARTIAL

**Disposition: No change.** The three names are for three different values.

The three names are for different things, which the finding overstates. meta '`thumbs_version'` (baseline.sql:463) is the thumbnailer's version, and `THUMBS_VERSION` is its constant (`pg_render`). '`thumb_output_version'` (thumbcache schema.rs:156) is the cache's output format, and `THUMB_OUTPUT_VERSION` is its constant. writer.rs:393 is a comment, not a key, so that instance is wrong. The similar names for different values are a real hazard.

### N-013 [medium] `PLACEHOLDER_VERSION` is imported from the analysis module, not the version registry — REFUTED

model/src/analysis.rs:134 re-exports `PLACEHOLDER_VERSION` from `crate::versions::PLACEHOLDER_VERSION` (versions.rs:29). The import from `pg_model::analysis` is a documented re-export of the registry value, not a separate definition.

### N-014 [medium] Perf span names mix dotted and undotted forms — REFUTED

The documented rule (perf lib.rs:182) is for span names such as scan.walk and phase.import. The undotted 'pipeline' at perf lib.rs:587 is a record name (jobs pipeline.rs:170 records it with `pg_perf::record`), and a single word has no dot to add. The span names checked (pass.scan, scan.setup, pending.placeholders, thumbs.pending) are all dotted.

### N-015 [medium] Budget keys and baseline keys use different separators for the same metrics — CONFIRMED

**Disposition: Fixed.** The budget keys in `docs/perf/budgets.json` now use the bench's own underscore form (`scan_walk.again`, `scan_walk.first`, `import_flush`, `pending_import`, `pending_thumbs`, `inbox_refresh.closed`, `inbox_refresh.open`), the form `docs/perf/baseline.json` and gungraun's ids use. `tools/perf-compare.py` already maps `-` to `_` when it matches a budget to a bench, so the gate matches the same benches as before. The `comparison` note's prose still says `inbox-refresh`, which is a description, not a key. Nothing reads the old hyphenated keys except that matcher, so no code change was needed. The first `just perf --budgets` run should show every budget matched and none reported as NO-BENCH.

budgets.json uses scan-walk.again, scan-walk.first and pending-thumbs (hyphens). baseline.json uses `scan_walk.again` (:20) and `pending_thumbs.at_height` (:12) (underscores) for the same benches.

### N-016 [medium] Test names follow several styles within the same crates — CONFIRMED

**Disposition: Fixed.** The style is in `crates/README.md` (Naming): a test name is a sentence that states what holds, a bare noun names a helper, and a platform-specific test keeps its suffix. `videos` became `each_pass_gives_the_frames_whole_with_their_delays` and `eviction` became `the_cache_stays_under_its_cap_by_evicting`; the helpers the finding listed are not tests and keep their names.

Test names in the same crates mix styles: bare nouns (borders.rs:324 `too_deep`), sentences (borders.rs:383, analyze.rs:221), a `_windows` and `_posix` suffix pattern (catalog browse tests) and a plain-name test with a comment (browse/read/tests.rs:99). The mix is real within crates, though low severity.

### N-017 [medium] QML user-action signal names use several verbs for the same kind of event — CONFIRMED

**Disposition: Fixed.** The QML event signals and their handlers that ended in `Asked` now end in `Requested` (`Hud`, `RatingBar`, `MediaView`, `VideoTransport`, `Viewer`, `Main`, and `tst_viewer.qml`). `Main`'s `similarAsked` became `similarSearchRequested`, since `TestHooks.qml` already has `similarRequested`. `showAsked` (a function) and the `*Toggled` signals are unchanged.

QML user-action signals use playAsked, volumeAsked, rateAsked, borderMarkAsked, copyRequested, dismissRequested, retryRequested, writeBackToggled, detectBordersToggled, recursiveToggled and toggleRequested. The same kind of event is named with Asked, Requested and Toggled.

### N-018 [medium] C++ member naming mixes trailing underscore and `m_` prefix — CONFIRMED

**Disposition: Fixed.** The desktop C++ tree now uses the `m_` prefix that `gui_app.cpp`, `live_reload.cpp` and `providers.cpp` already use (45 uses against 25 trailing-underscore uses). The members are renamed in `crates/desktop/cpp/frame_sink.h` and `frame_sink.cpp` (`m_state`, `m_sink` on `FrameSink` and `SinkRef`) and in `video_source.h` and `video_source.cpp` (`m_player`, `m_device` on `VideoPlayer`). Each rename is local to its class, and no other file names these members. The C++ build runs only on Windows, so the first check is a desktop build.

C++ members use a trailing underscore (`frame_sink.cpp:10` `state_`, `video_source.h` `player_` and `device_`) and an `m_` prefix (providers.cpp:124 `m_kind`, `gui_app.cpp:53` `m_text`, `live_reload.cpp:32` `m_prefix`). Both conventions appear in one desktop/cpp tree.

### N-019 [medium] `cxx_name` renames are applied unevenly across bridges — CONFIRMED

**Disposition: Fixed.** No-op renames removed; exports are camelCase where the names differ.

`cxx_name` renames are applied unevenly. `library_bridge.rs:25` renames writes to writeStatus and `write_failed` to writeFailed, while roots and busy have none. `notices_bridge.rs:37` renames `toast_level`. `viewport_bridge.rs:52` renames `zoom_at` to setZoom. `facet_choices_bridge.rs:32` has no `cxx_name`, and `shell_bridge.rs:169` has redundant renames that equal the Rust name.

### N-020 [medium] Bench folder root literals differ between bench cases — PARTIAL

**Disposition: No change.** These are separate fixtures; the difference is style only.

catalog.rs:121 and :241 use different literal roots: `r'PhotoGallery\synthetic\library'` for the list bench and `r'D:\bench\library'` for the scan bench. These are separate fixtures and may be meant to differ. The duplicated literal is a style issue, not a clear error.

### N-021 [medium] Environment variable prefixes are mixed across tools and build scripts — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Repo-owned variables use PG_ with no fallback, as C-008.

tools/doctor.ps1:71 reads `PG_FFMPEG` and `PG_EXIFTOOL`, while doctor.ps1:49 reads `FFMPEG_DIR`, and the same script's `QT_DIR`, `LIBCLANG_PATH`, `LLVM_DIR`, `PIN_ROOT` and `PIN_EXE` are unprefixed. eval/jpeg-decoders/linux.py:12 mixes `JPEG_*` and `PG_*` in one environment list. `imagereduce_switch.py:14` mixes `IMAGEREDUCE_VIPS_BIN`, `JPEGLI_BUILD` and `PG_ROOT`.

### N-022 [medium] Temp-file suffixes differ by write kind and between golden data and code — CONFIRMED

**Disposition: Fixed.** The only `.pg-tmp` in the repo was a sample `WriteFailed` notice, which no code path writes. Its temp path is now `a.jpg.rating.pg-write`, the suffix `writeback/src/file.rs` gives the rating write (`WriteKind::Rating`), in both `crates/model/src/notice.rs` (the round-trip test) and its golden `crates/model/src/notice/golden.jsonl`. The golden is test data, so the change is in one fixture pair and no behaviour changes. The other suffixes (`.turns.pg-write`, `.cut.pg-write`, folder removal's `.pg-write`) are the code's own and are left alone.

The notice fixtures use a.jpg.pg-tmp (golden.jsonl:18 and notice.rs:1397). The code's temp suffixes are .rating.pg-write and .turns.pg-write (writeback file.rs:60-62, queue.rs:562, removal.rs:576), and the cut temp is .cut.pg-write (inplace.rs:83). No code path produces .pg-tmp, so the fixture names a suffix the app never writes. The golden's folder-removal temp is a.jpg.pg-write, which matches the code.

### N-023 [medium] Notification design doc names differ from the code — PARTIAL

**Disposition: No change.** The design doc declares each override it makes.

`NOTIFICATIONS_DESIGN.md` is a design doc that records its own overrides: each 'Cache events (overrides ...)', 'Keeper close (overrides ...)' and 'Dev mode (overrides ...)' bullet says which earlier name it replaces, and the overridden names are called out. The differences the finding lists are therefore declared, not accidental. The doc has no stale claims left unmarked at the instances checked.

### N-024 [medium] Eval folder and label names for similar-but-not-duplicate pairs disagree — PARTIAL

**Disposition: No change.** The folder name on disk is consistent; the labels differ only cosmetically.

The same set is named differently by tool: eval/viewer.html:152 says Hardest negatives, verify.py:100 uses `similar_folder`, evaluate.py:130 uses `hard_negative_pairs_flagged`, and evaluate.py:209 uses similar-folder pairs flagged. The folder on disk is `similiar_but_not_dups` everywhere (common.py:91, `TEST_SET.md:29`), so that spelling is consistent, though misspelled.

### N-025 [medium] Eval key and kind names differ between the eval scripts — CONFIRMED

**Disposition: Fixed.** One name per measure and per kind in `eval/`: the recall key is `recall_at_10` (`bench_ann.py`'s result dict, read by its two printers, matching `metrics.py` and `report.py`), and the pair kind is `duplicate` in `eval/ann/metrics.py` (`KINDS`, the result key and its doc) and `eval/ann/report.py`, matching the label `run.py` already writes. The two `score` functions are told apart by name: `bench_ann.py`'s became `score_search`, while `metrics.score` keeps its name. The `dups2` folder name (`common.py`, `viewer.py`) is the data directory and is left alone. Result files written before this change keep the old keys, so a comparison across the change needs a re-run.

`bench_ann.py:207` returns recall10 and metrics.py:165 writes `recall_at_10` for the same measure. score() is defined twice with different signatures: `bench_ann.py:201` and metrics.py:129. The kinds are labelled dups2 in metrics.py:21 and duplicate in run.py:488. The report.py:109 cite does not exist, though report.py:120 uses `recall_at_10`.

### N-026 [medium] Historical plan and reference doc references are stale — REFUTED

`NTFS_MFT_SCAN_PLAN.md:779-781` is the plan's migration checklist '(Doc references to `pg_scan` change with the move'), so naming `pg_scan` there is intentional. PROFILING.md:350 names scan-timing.sh and then gives `tools/profiling/scan_timing.py` as its successor, which is an explicit history note.

### N-027 [medium] Module file name does not match its main type — PARTIAL

**Disposition: Fixed.** `OPEN_ITEMS.md` names `Lanes::request_crop` at `coordinator.rs:307`. The module name is defensible.

No type Coordinator exists in the library crate. The main type in coordinator.rs:130 is Lanes, and coordinator.rs:731 has a fn coordinate. `OPEN_ITEMS.md:157` cites `Coordinator::request_crop`, which does not exist, so that doc is stale. lanes.rs:65 has pub struct Group, which is a separate concept in a separate file. The file name itself is defensible.

### N-028 [low] Action and event names differ across the client controller layers — PARTIAL

**Disposition: No change.** The repeated name reflects layering, not a clash.

ListShown appears in three enums (controller.rs:83, input.rs:17, start.rs:286), which are in different layers and carry the same event name for the same fact. Event::FilesChanged mapping to Command::RefreshTurned (controller.rs:327) and Event::Reload mapping to `open_list` (controller.rs:326) are real naming gaps. The repeated ListShown is layering, not a clash.

### N-029 [low] Same type name used for different types in different modules — CONFIRMED

**Disposition: Fixed.** GridKey in pg_client; app's Status is LibraryActivity; catalog's is CatalogCounts.

The same type name names different types: Writing in thumbs/lib.rs:178 and thumbcache schema.rs:255, Status in model yandex.rs:85 and model query.rs:42 (both in `pg_model`), Blur in detection.rs:48 and :69, and Grid as an enum in keys.rs:37 and a struct in client grid.rs:111. Each is in its own module, so this is a readability cost, not a compile error.

### N-030 [low] Accessors for stored codes are named differently across the model — CONFIRMED

**Disposition: Fixed.** The rule is in `crates/README.md` (Naming): `key()`/`from_key()` give the stored spelling, `name()` display text, and `code()` a number only. Renamed to `key()`: `Source`, `ScanNote`, `StartOverReason`, `EditOp`, `ViewOp`, `FilePass`, `LedgerKind` (`from_key`), `Op`, `FolderFailure` (`from_key`), `RefusedReason`, `Format` (`from_key`), `Ext`, `Provider` (`from_key`); `Resolver::name` became `Resolver::token`. Kept: `Level::code`, `Purpose::code`, `FsKind::name`, and the `Notice` macro's `code()`.

Accessors for stored codes differ: detection.rs and media.rs Kind use key() and `from_key(`), format.rs:46 uses name(), media.rs:96 Ext::name, and notice.rs:790 uses code() on Op. Those pairs are real, if not wrong. Resolver::name (notice.rs:753) returns a String, which differs in shape from the others.

### N-031 [low] Decision, finding and item identifiers are numbered inconsistently in the plans — PARTIAL

**Disposition: No change.** Each plan numbers its own decisions.

Each plan numbers its own decisions, so Q3 (`BACKGROUND_JOB_AUDIT_FIXES`) and Q-S1 (`BOUNDED_MEMORY`) differ by plan and are not wrong. D1 and D2 do appear in both `CATALOG_SIZE_PLAN` and `BOUNDED_MEMORY_PLAN` with different meanings, which is real. `REFACTOR_LEDGER's` skipped F-56 and F-57 are a gap, and REVIEW.md:19 says IDs are never reused, so the gap is consistent.

### N-032 [low] Decoder and feature names differ in small ways across the crates — PARTIAL

**Disposition: No change.** This is style variance, not a difference in meaning.

Naming here is style variance, not meaning drift: container `png_chunks`, `png_` and `jpeg_` prefixes against unprefixed riff and find, and codecs.rs:43 writing ::png::Scratch with a leading :: while the webp type has none. The features (pg-codec fixtures, pg-catalog fixture) are documented in crates/README.md:138 as a deliberate test-support rule.

### N-033 [low] Product and data-folder spellings differ across docs and code — PARTIAL

**Disposition: No change.** `LANGUAGES.md:85` documents the spellings.

The product is named four ways in LANGUAGES.md:85 (Photo Gallery, gallery, pg-gallery, PhotoGallery), and the spelling of the UI title is documented there. layout.rs:151 and :154 assert different default folders per platform (PhotoGallery on Windows, photo-gallery under .local/share on Linux), which follows platform convention and is intended. eval/viewer.html:6 is a separate tool titled NDD Bake-off.

### N-034 [low] Inbox method names for the same operations differ by layer — CONFIRMED

**Disposition: No change.** The names follow the grain: singular names at the app layer, plural and batch names at the store layer. That rule is documented beside the inbox code in `app/src/inbox.rs`, and the code follows it. Unifying the layers would erase the distinction.

Inbox operations are named singular at the app layer and plural at the store layer: `thumb_failed` and `thumb_made` (inbox.rs:509), `thumbs_failed` and `thumbs_made` on Store (inbox.rs:641), and ThumbsFailed and ThumbsMade on Step (inbox.rs:718). Thumb::Failed and Thumb::Made (inbox.rs:214) add a third form.

### N-035 [low] Phase and pass labels in code comments are stale project labels — PARTIAL

**Disposition: Fixed.** The stale `Phase 4` and `N-13` labels are removed from `core.rs`, `schema.rs` and `roots.rs`.

Comments cite project labels a reader can't resolve: roots.rs:557 'N-13', schema.rs:85 'Phase 4', schema.rs:624 'N-13'. The finding's work.rs:909 'Phase 3' cite is past EOF (the file has 761 lines), so that instance can't be checked. The labels are project tracking names, stale for code readers, though not wrong.

### N-036 [low] Dups pass and owed concept have several names across the plans — CONFIRMED

**Disposition: Fixed.** The pass has one code name, `Duplicates`, as `IMPORT_FAILURES_PLAN.md` (D-C) already decides. The `Names` bullet in `DECODE_ONCE_PLAN.md` (around line 531) now gives `Phase::Duplicates`, `pass-duplicates`, `Message::PassDuplicates` and `FilePass::Duplicates`, and says that the prose "Dups pass" used through the plan refers to it, so the bullet no longer names one pass two ways. The `owed` names in `DECODE_ONCE_PLAN.md` (`pairs_owed`, `hashes_owed`, the owed flags) are different tables and are not renamed. None of the planned names exists in code yet, so nothing else changes.

`DECODE_ONCE_PLAN.md:531` names the pass Dups (Phase::Dups) and also FilePass::Duplicates in the same bullet. `IMPORT_FAILURES_PLAN.md:74` decides FilePass names Duplicates and Placeholders. The code's FilePass (notice.rs:375) has only Thumbs, Borders and Import, so those names are planned and not built. Within the plans, Dups and Duplicates name the same pass.

### N-037 [low] Plan documents use the same word for different concepts — CONFIRMED

**Disposition: Fixed.** `docs/plans/OPEN_ITEMS.md` (S-17.7) now says "cloud-file placeholder" for the file-record concept (the heading of the defect, the design heading and its body sentence), so it no longer shares "placeholder" with the ThumbHash pass. The ThumbHash meaning, used in `crates/README.md` (the Placeholders pass, the grid's placeholder) and in the `placeholders` table, keeps the bare word, which is what the code and the README use. The open question in that section, what to call the new `placeholder` flag, is left open as the section already says.

'Placeholder' names two concepts: the cloud-file or reparse placeholder (`OPEN_ITEMS.md:143-144`, dir.rs:410) and the ThumbHash placeholder (crates/README.md:435-445, analysis `PLACEHOLDER_VERSION`). The same word is used for both in the plans and the code.

### N-038 [low] QML dialog title and removal keys are not uniform — CONFIRMED

**Disposition: Fixed.** The discard keys are renamed to the verb-first form the other dialog keys use (`ui-remove-folder-title`, `ui-cut-borders-title`): `ui-discard-removal`, `ui-discard-removal-title`, `ui-discard-removal-question` and `ui-discard-removal-confirm` in `crates/client/locales/en/ui.ftl`. The three QML references are updated (`DiscardRemovalDialog.qml` title, question and confirm button, and `LibraryPanel.qml`'s discard button). The button `ui-remove` stays, since the folder removal uses that verb. No other locale or Rust file names these keys, and the `words.rs` check that every id the QML asks for exists in `ui.ftl` is satisfied by the matching edits. The QML edit is small, but it still needs a `qmltest` run to confirm.

Dialog titles are ui-removal-discard-title (DiscardRemovalDialog.qml:32), ui-remove-folder-title (RemoveFolderDialog.qml:35) and ui-cut-borders-title (CutBordersDialog.qml:30). Buttons are ui-removal-discard (LibraryPanel.qml:101) and ui-remove (LibraryPanel.qml:69). The removal and remove naming differs for the same kind of action.

### N-039 [low] Fluent id prefixes and digit suffixes are not uniform — CONFIRMED

**Disposition: No change.** The decision is not to rename the Fluent ids; the per-file convention still needs a header comment.

cli.ftl ids all begin cli-; text.ftl ids have no file prefix (text.ftl:1 is the header). Hint ids use digit suffixes, as in keys.rs:273 ui-hint-up-4 and :274 ui-hint-down-1. The mix is real but is documented per file.

### N-040 [low] Constants and helper names differ in small ways across tools and crates — PARTIAL

**Disposition: No change.** The differences are across languages and tools, not one codebase.

sort.rs:7 uses KEYS where facets, panels, keys and input use ALL, a real split. The others are cross-language or unrelated: Get-Median (sweep.ps1:30) against Median (`few_sweep.ps1`), `sha256_of` (`setup_ffmpeg.py`) against Get-Sha256 (PowerShell), and `FILE_FLAG` constants in `png_read_bench.rs` against bare literals in `jpeg_read_bench.rs`. Those are different languages or tools, so they are not a codebase inconsistency.

## versions

### V-001 [high] Thumbnail cache doc says there is no pipeline version, the code stores one — CONFIRMED

**Disposition: Fixed.** `THUMBNAIL_RESAMPLING.md` now says the cache stores a pipeline version (`meta.thumb_output_version`, which it refuses when newer than this build's) and that what it does not store is a per-entry version, the narrower gap the audit left open.

`THUMBNAIL_RESAMPLING.md:31` (T-01) says the cache stores no filter or pipeline version. thumbcache/schema.rs:156 writes meta `thumb_output_version` and :211 refuses a newer one, so the doc's statement is stale at the cache level. T-01's narrower gap (no per-entry version in `get_nearest`) is not checked here. schema.rs:211 and the doc's claim are as described.

### V-002 [medium] Replay recording format: u32 plan item and duplicated mismatch and missing-format text — CONFIRMED

**Disposition: Fixed.** The format constant is `i64` (`pg_model::versions`), as the code has it; the audit's "u32" was stale. The mismatch text is one helper, `format_refused(line, format)`, used by the parse and by the replay, so the two cannot drift.

replay.rs:13 re-exports the registry's i64 `REPLAY_FORMAT_VERSION`; :28 writes it into Init.format; :46 missing-format text; :52 and :69 repeat the mismatch text (parse and replay). tests/replay.rs:57 tests formats 0 and 2. The title's u32 is stale: the constant is already i64, so only the duplicated text remains.

### V-003 [medium] Bench catalog LAYOUT strings are hand-written literals — CONFIRMED

**Disposition: Not changed.** `BENCH_LAYOUT` and `BENCH_LARGE_LAYOUT` are named constants already (`bench/benches/ir/catalog.rs`); their values are the layout names the bench writes, which are literal by nature. The audit's cited doc lines have moved; nothing else applies.

catalog.rs:36 `BENCH_LAYOUT='v1-windows-rules'` and :40 `BENCH_LARGE_LAYOUT='v1-windows-rules-large'` are hand-written literals. The cited doc is at lines 32-35, not 7-9 as the finding says.

### V-004 [medium] `CATALOG_SIZE_PLAN` quotes a bench LAYOUT value the code does not have — CONFIRMED

**Disposition: Fixed.** The plan's step names the bench's `BENCH_LAYOUT` constant and its current value (`v1-windows-rules`), not the value it quoted (`v10`).

`CATALOG_SIZE_PLAN.md:312` quotes LAYOUT = v10-windows-rules to bump. The code has `BENCH_LAYOUT` = v1-windows-rules (catalog.rs:36). Line 43 cited in the plan is RULES, not a layout constant.

### V-005 [medium] Test code hard-codes thumbs, `user_version` and output-version literals — CONFIRMED

**Disposition: Fixed.** Thumbs-version literals use pg_model::versions constants; foreign and earlier values are named test fixtures.

thumbcache lib.rs:465, 479, 1072, 1085, 1342 pass literal 1 as the thumbs version; :746 sets `user_version` = 7; :757 sets `user_version` = 0; :787 writes literal 0 to `meta.thumb_output_version`. `tests/other_process.rs:49` and :61 pass literal 1; writeback/src/lib.rs:806 passes literal 1.

### V-006 [medium] Test constructors for Versions differ: Versions::current, versions(), `pg_library::versions(`) — CONFIRMED

**Disposition: No change.** The two named helpers are deliberate.

bench catalog.rs:72 fn versions() returns Versions::current(). pair.rs:583 and rehearsal.rs:21-23 do the same; rehearsal.rs:20 doc names `pg_library::versions`. `other_process.rs:12` local fn. library/src/runner.rs:273 calls crate::versions(). library/tests/pairs.rs:65 overrides import on `pg_library::versions(`). borders.rs:33 and lanes.rs:59 cited without a directory are library/tests/borders.rs:33 and library/tests/lanes.rs:59, both calling `pg_library::versions(`) as stated. Claim holds; paths incomplete.

### V-007 [medium] Version constants imported from different paths for the same value — CONFIRMED

**Disposition: Fixed.** Inside the workspace versions import from pg_model::versions; public re-exports kept.

lifecycle.rs:485 (`pg_model::versions::IMPORT_VERSION`), :618 (`pg_analysis::analyze`), :500 (BORDERS/THUMBS from `pg_model::versions`). raw.rs:12-14 mixes `pg_analysis::analyze`, `pg_analysis::borders` and `pg_model::analysis`. `test_support.rs:7` `pg_analysis::analyze`. writers.rs:921 imports `IMPORT_VERSION` from `pg_analysis::analyze` (finding says :920). import/src/lib.rs:168 passes `pg_analysis::analyze::IMPORT_VERSION`. `PAIRING_VERSION` is not at writers.rs:1711; it is used as `pg_model::pdq::PAIRING_VERSION` at :1742 and :2015/:2018, so the line is wrong but the source is as stated.

### V-008 [medium] Bump rule restated in five places with different wording — CONFIRMED

**Disposition: Fixed.** The restatements point at the versions.rs module doc.

Five restatements read: analyze.rs:23-25 header-only import not bumped; params.rs:3 bump when Params changes; ledger.rs:80 `THUMBS_VERSION` by hand; `NOTIFICATIONS_DESIGN.md:375` THUMBS by hand, codec upgrades not detected; `DECODE_ONCE_PLAN.md:23` rule rewritten. The wording differs in each.

### V-009 [medium] Schema and migration numbers in docs and code comments contradict each other and the registry — CONFIRMED

**Disposition: Fixed.** The plans state schema 1 and name planned migrations TO_n.

Stale numbers confirmed: `CATALOG_SIZE_PLAN.md:5` (v20), PLAN.md:315 (version 11, ten steps), `SCAN_OPTIMIZATION_PLAN.md:57` (ends at 15), `DECODE_ONCE_PLAN.md:400` (`TO_10` next), `IMPORT_FAILURES_PLAN.md:48` and :376 (`TO_18`), reader.rs:19 (schema 19), schema.rs:82 (MIGRATIONS empty). Path error: `REFACTOR_LEDGER` is `docs/plans/REFACTOR_LEDGER.md:128`, not docs/reference. crates/README.md:297 says version 1, which matches the registry, so it is not a contradiction.

### V-010 [medium] Historical migration and backup references in the named plan docs — CONFIRMED

**Disposition: Fixed.** The historical passages carry dated history notes.

PROFILING.md:510-511 (pre-v9.bak before migration v9), :619 (migration 13 from a 209.4 MB v12 copy) and :985 (migration 14 introduces `meta.pairing_epoch`) are as cited. `NTFS_MFT_SCAN_PLAN.md:296` and :544 and `BACKGROUND_JOB_AUDIT_FIXES.md:32` match too. The numbers are historical and unmarked as such.

### V-011 [medium] `IMPORT_VERSION` HEAD values in plan docs do not match the registry — CONFIRMED

**Disposition: Fixed.** Snapshots are dated and name commit 7deed676 as the reset to 1.

`EXIFTOOL_PORT_PLAN.md:77-78` says `IMPORT_VERSION` at HEAD is 2 (bumped 2026-11-13); `IMPORT_READS_PLAN.md:12` says HEAD raised it `2->3`; `THUMBNAIL_RESAMPLING.md:8` lists it as done at 2; `NOTIFICATIONS_PLAN.md:14`; `DECODE_ONCE_PLAN.md:436` `DUPS_VERSION` beside `IMPORT_VERSION`. versions.rs:13 is 1. These are dated snapshots, so 'stale' is right against the current registry, which the reset (plan 3.6) changes.

### V-012 [medium] Bundled SQLite version stated three ways — CONFIRMED

**Disposition: Fixed.** The bundled SQLite is 3.54.1 through fork rev ac441bb, checked against Cargo.lock.

`INCREMENTAL_SYNC.md:278` measured on SQLite 3.49.1; `WEB_CLIENT_RECONCILED.md:499` says bundled sqlite3.c 3.50.2; scan-optimizations-validation.md:64 says 3.54.1 bundled by the rusqlite fork. Three versions are named, each in a different doc.

### V-013 [medium] Tool text restates pinned versions as literals — CONFIRMED

**Disposition: Fixed.** `tools/doctor.py` (new `ffmpeg_pin`) and `tools/doctor.ps1` (new `Get-JustValue`) now read the Qt version and the FFmpeg release from the Justfile, as `doctor.py` already did for Qt. `crates/video/build.rs` no longer restates the release; it points at the Justfile's `ffmpeg_url`. The hints now follow a pin bump without a second edit; the checks themselves are unchanged. Only the hint text changes behaviour.

doctor.ps1:46 and :52 and doctor.py:121 and video/build.rs:112 repeat literal pins as text (Qt 6.12.0, FFmpeg 9.0). doctor.py:113 reads `qt_version` from the Justfile, so the Qt case is already handled there. `build_static_ffmpeg.py:34` `FFMPEG_RELEASE` = 9.0.2. The literal text agrees with the pins today; the duplication is real.

### V-014 [medium] Perf log program identifiers differ across binaries — CONFIRMED

**Disposition: Fixed.** `crates/perf/src/lib.rs` now defines `pub const PROGRAM: &str = "gallery"`, used for the perf meta `program` in `crates/cli/src/lib.rs` (`open_perf_log`) and `crates/desktop/src/trace.rs` (`install`), and for the log header in `crates/desktop/src/lib.rs`. The command line and the window are one executable (`crates/gallery`), so one name is the true identifier, and `pg_perf` is the crate both callers already depend on. Behaviour change: the desktop's perf log now records `"program": "gallery"` instead of `"gallery-app"`.

cli/src/lib.rs:109 perf meta program 'gallery'. desktop/src/trace.rs:31 perf meta program 'gallery-app'. desktop/src/lib.rs:148 log header 'gallery {version}'. Three different program identifiers for the same binary family.

### V-015 [medium] Eval scripts disagree on the default model name — CONFIRMED

**Disposition: Fixed.** `eval/common.py` now holds `DEFAULT_MODEL` (`sscd_fp16`, the PyTorch baseline) and `embedding_name(model, trim, draft)`. `embed.py`, `synth.py` and `ann/data.py` (`MODEL`) build the embedding stem with `embedding_name`, instead of three different string constructions. `group.py`, `check_groups.py`, `verify.py` and `verify_synth.py` take `DEFAULT_MODEL` for `--model`. The ANN's TensorRT stem is deliberately not the default: it is the production pipeline's file, the other scripts default to the baseline. Behaviour change: none; the same names and defaults result.

`check_groups.py:43` and group.py:159 default `sscd_fp16`. ann/data.py:16 MODEL = `sscd_trt_fp16_trim_draft`. embed.py:39 builds the name from --model plus `_trim` and `_draft`. The name is built in different ways.

### V-016 [medium] TensorRT major version stated inconsistently in eval — CONFIRMED

**Disposition: Fixed.** The `eval/bench_semantic.py` module docstring no longer states "TensorRT 11". It says the plan is named with the installed TensorRT's major version, `accel.trt_major()`, which is what the code uses for the file name. The docstring now agrees with the code, and no version literal is left to drift. The TensorRT 10 folder (`accel.py` `TRT10_DIR`) serves ONNX Runtime's provider, a different purpose, and its docstring already says so. Behaviour change: none.

`bench_semantic.py:7` docstring says TensorRT 11 fp16 (finding's :5 is 2 lines off). :70 builds the plan name from `accel.trt_major(`). accel.py:40 `TRT10_DIR` defaults to tools/tensorrt-10.16.1.11-cu13. The docstring's major version disagrees with the runtime folder.

### V-017 [medium] Two different dates given for the same default-off switch — REFUTED

verify.py:653 says the --strong-inliers default has been off since 2026-10-29. viewer.html:269 says hard-negative pairs have mostly matched since 2026-10-28. These are different facts with different dates. The cited verify.py:83 and :91 contain no date, so 'since 2026-10-29' is not repeated there.

### V-018 [medium] Pairing version seed is a literal in baseline.sql while the other marks seed 0 — CONFIRMED

**Disposition: Fixed.** The `1` stays in `crates/catalog/src/baseline.sql`, because a frozen SQL baseline cannot name a Rust constant, and the difference is the intended D2 exception. The tie is kept in one guard: the compile-time assert in `crates/catalog/src/schema.rs` (`PAIRING_VERSION == 1`) and the schema test that checks the stored seed. The `PAIRING_VERSION` doc in `crates/model/src/versions.rs` now says where the literal lives and what checks it, where it said only "checked by the catalog's schema test". No behaviour change; the baseline file is not edited.

baseline.sql:455/458/463 seed borders, import and thumbs at 0, and :460 seeds `pairing_version` at 1. schema.rs:43 and :55 state the literal and assert it. versions.rs:19-20 says the baseline seed is this value. The difference is intentional (D2 in the plan), so 'inconsistent' describes a deliberate exception.

### V-019 [medium] Thumbnail cache newer-version handling — REFUTED

Instances are accurate (schema.rs:202 and :211 return Found::Newer; :231 starts over an earlier or missing output version). The policy is uniform: both newer markers are refused and left alone. The start-over for an earlier version is by design (D9). No inconsistency shown.

### V-020 [medium] Preferences file has no format version and migrates a float on read — CONFIRMED

**Disposition: Fixed.** The preferences file now carries a layout version. `PREFS_VERSION` (1) is registered in `crates/model/src/versions.rs`, read by `crates/app/src/prefs.rs` as `Prefs::version`, and stamped by `PrefStore::change` on every save. A file without the key reads as 0, the float row-height layout, and the existing lenient float read stays so such a file still opens. Behaviour change: saved `gallery.json` gains a leading `"version": 1` key, and the golden text and its tests are updated; reads are otherwise unchanged.

prefs.rs:19-20 says missing fields take defaults and unknown fields are ignored (serde default at :22). :88-89 rounds and clamps a legacy float `row_height` on read. GOLDEN at :212 has no version key.

### V-021 [medium] `THUMBS_VERSION` is wired into the thumb cache at five call sites — PARTIAL

**Disposition: No change.** The production call site is correct, and the test call sites are intentional.

Production site lib.rs:39 (`open_thumbs`) and test call runner.rs:370 are correct. The cited src paths are wrong: library/src/video.rs and writeback.rs do not exist, and library/src/lanes.rs:73 has no `THUMBS_VERSION`. The matching calls are in library/tests/lanes.rs:73, tests/video.rs:241 and tests/writeback.rs:103, so the line numbers hold but the directory is wrong. The duplication is real.

### V-022 [low] Catalog lowers newer marks while refusing newer schemas — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** An older build refuses a catalog with a newer mark; the set-back path is removed; backup restore is the only downgrade.

core.rs:138-143 refuses found `>` current as made by another build (the finding's :136 is the enclosing match, 2 lines off). core.rs:227-237 sets lowered marks back and logs them via `pg_log::warn`.

### V-023 [low] `key_version` mark name differs between plan and code — CONFIRMED

**Disposition: Fixed.** The plan was the side with the wrong name: `docs/plans/CATALOG_SIZE_PLAN.md` (the version stamp at line 66, and the REINDEX note at line 307) now says `meta.key_version`, the name `crates/catalog/src/lifecycle.rs` writes and `core.rs` reads. The code is unchanged. The plan describes implemented behaviour, so it should match the code's mark name.

`CATALOG_SIZE_PLAN.md:66` says `meta.name_key_version` holds the table hash plus a key-format number. The code writes meta `key_version` (lifecycle.rs:231; core.rs:157 reads '`key_version'`). The name differs; the finding's 'plan' side is the wrong name.

### V-024 [low] Test fixture uses a `taken_zone` value that is not in the real format — CONFIRMED

**Disposition: Fixed.** `crates/catalog/src/lifecycle.rs` tests now write `NO_ZONE` (`-99999,-99999,-99999,-99999`) for `meta.taken_zone`, in the four-offset format `TAKEN_ZONE_QUERY` produces, instead of the free text `'another-zone'`. The value is outside the range of any real offset, so the test still forces a recompute. Behaviour change: none; test fixture only. The sentinel `'just before'` in `core.rs`'s reader test is left as it is: that test checks the reader sees a committed write and never parses the zone.

lifecycle.rs:1390 writes literal 'another-zone' to `meta.taken_zone`. :1397 asserts the real fingerprint has 4 comma fields. :1388 comment states a UTC system's fingerprint is '0,0,0,0'.

### V-025 [low] Test pins a decode cap value owned by `pg_codec` — CONFIRMED

**Disposition: Fixed.** The assertion in `crates/edit/src/rewrite.rs` no longer pins `512 << 20`. It reads the cap from `pg_codec::max_decode_bytes()` and checks the boundary: a size equal to the cap is kept and one byte over is refused. The test now follows the owner's value when it changes, and the boundary behaviour is still checked. Behaviour change: none; test only.

edit/src/rewrite.rs:60 asserts `pg_codec::max_decode_bytes(`) == 512 `<<` 20. The test pins the owner's value as a literal, though the comment says it is the decoders' own cap.

### V-026 [low] Win32 enum values hand-copied, with a test in only one file — CONFIRMED

**Disposition: Fixed.** `crates/platform/src/dir.rs` (`drive_type`) now imports `DRIVE_FIXED`, `DRIVE_REMOVABLE` and `DRIVE_REMOTE` from windows-sys instead of copying them. Its old comment said windows-sys hid them behind a feature this crate does not take, but the workspace's `windows-sys` features include `Win32_System_WindowsProgramming`, where they live, so the literals were never needed. In `crates/platform/src/drive.rs` the Windows-only bus test also checks `BUS_UNKNOWN` now, so every bus literal is checked. Those bus literals stay, because the pure decision code is compiled and tested on every platform. Behaviour change: none.

platform/src/dir.rs:137-140 declares `DRIVE_REMOVABLE/FIXED/REMOTE` as local literals. grep finds no test in dir.rs comparing them with windows-sys. drive.rs:170-178 `BUS_*` literals; the windows-only test at drive.rs:739-742 asserts them against `fs::BusType*`, as the finding says.

### V-027 [low] Image cache key format defined in two desktop files — CONFIRMED

**Disposition: Fixed.** The layout `<file id>@<mtime>.<rotation>.<turns>` is now described only where it is implemented, in `pg_model::image::ImageKey` (`crates/model/src/image.rs`). `crates/desktop/src/providers.rs` and `crates/desktop/src/gallery/items.rs` (`image_key`) name that type and point to it instead of restating the layout. Behaviour change: none; doc comments only.

desktop/src/gallery/items.rs:39-40 documents the ImageKey format '`<file` `id>@<mtime>.<rotation>.<pending` `turns>'`. desktop/src/providers.rs:1-2 documents the same layout as '`<file` `id>@<mtime>.<rotation>.<turns>'` in a second place.

### V-028 [low] Qt version claims outside the READMEs — REFUTED

The Qt prose matches the pin. Justfile:105 sets `qt_version` := 6.12.0. site/index.html:49 and `tst_gridcell_fade.qml:3` say Qt 6.12. Nothing is inconsistent. The cited tools/static-qt/.. path is not a real file.

### V-029 [low] FilterBar facet numbers share integers across two namespaces — CONFIRMED

**Disposition: Fixed.** The bound functions name their parameter field; list facets are a separate function.

FilterBar.qml:46-53 list facets use integers 0-5 (type through exts), order pinned by tests/facets.rs. :202-219 range Bound facets use 0-4 (size, megapixels, duration, and two dates) with the same integers in a second namespace.

### V-030 [low] nvJPEG header version differs from the pinned wheel — CONFIRMED

**Disposition: Deferred.** The docstring is fixed; a 13.2 to 13.3 declaration diff or a GPU run is still needed.

nvjpeg.py:6 says declarations come from nvjpeg.h 13.2. requirements.txt:8 pins nvidia-nvjpeg==13.3.2.43. The stated header and the pinned wheel differ.

### V-031 [low] Batch byte budget duplicated in two eval scripts — CONFIRMED

**Disposition: Fixed.** gpu.py's cap is GPU_BENCH_BATCH_BYTES with its 6 GB note; the owner picks the value.

`nvimgcodec_bench.py:23` and nvjpeg.py:36 both define `MAX_BATCH_BYTES` = 2.0e9 with a comment about the GPU's memory. The constant is repeated, with slightly different comments.

### V-032 [low] Library Cargo.toml calls versions a module — PARTIAL

**Disposition: Fixed.** The `library/Cargo.toml` comment separates modules from functions.

library/Cargo.toml:11 lists versions among items, and the comment mixes module names (runner, folders, scans) with functions (`open_catalog`, `open_thumbs`, versions). lib.rs:44 has pub fn versions(), so it is a fn. The manifest never calls it a module; the finding's reading is an inference. The line is stale only in that ambiguity.

### V-033 [low] Version owner named inconsistently in docs — CONFIRMED

**Disposition: Fixed.** The one version name without an owner path, `BORDERS_VERSION` in the module doc of `crates/catalog/src/ledger.rs`, now names `pg_analysis::borders`, the crate that re-exports it. The other places the finding cites already give the owner path that `crates/model/src/versions.rs` documents (`pg_render::THUMBS_VERSION`, the re-export in `crates/render/src/lib.rs`); the `pg_model::versions` imports in tests name the same constant at its registry. The owner convention is now the one the registry states. Behaviour change: none; doc comment only.

ledger.rs:80 says `THUMBS_VERSION` (`pg_render`) bumped by hand; :84 names `BORDERS_VERSION` with no crate. lifecycle.rs:393 names `pg_render::THUMBS_VERSION` while the import is from `pg_model::versions` (:500). thumbcache/schema.rs:379-380 names `pg_render::THUMBS_VERSION` and `pg_catalog::Versions::thumbs`.

### V-034 [low] `cache_version!` macro duplicates the registry value as a literal — CONFIRMED

**Disposition: Fixed.** The `cache_version!` macro and its compile-time tie in `crates/thumbcache/src/config.rs` are gone. The SQL no longer carries `PRAGMA user_version`; `make_layout` in `crates/thumbcache/src/schema.rs` sets it from `CACHE_VERSION` (the registry re-export), next to the `application_id` pragma it already sets the same way. That removes the literal, and the expected schema and new files still come from the same `make_layout`, so they cannot disagree. Behaviour change: none for a new or checked cache, since the stored value is the same `1`; the user_version is now written by a pragma after the tables rather than inside the SQL batch.

config.rs:12-16 macro `cache_version!(`) yields the literal 1 (:14). :19 `CACHE_VERSION` is the registry re-export and :21 is the const assert that ties the literal to it.

### V-035 [low] Tools keep legacy version layouts alongside current ones — CONFIRMED

**Disposition: Fixed.** `tools/profiling/pairing/set_owed.py` now works only on the current layout, the `pairs_owed` queue in `crates/catalog/src/baseline.sql`, and the draft v12 `analysis.dups_state` branch is removed. A catalog without `pairs_owed` exits with a message naming the schema, as `drop_owed_pairs.py` already does for its tables. The v12 layout was a pre-release draft that no build of the current catalog makes. Behaviour change: the script refuses a draft v12 scratch catalog, where it used to set its owed column.

`set_owed.py:6` (cited as :7) names the draft v12 `dups_state` layout and the v13 `pairs_owed` layout. :33 picks the layout by table presence. Both layouts are still handled.

### V-036 [low] Compare-shots manifest reads a legacy key — CONFIRMED

**Disposition: Fixed.** The nine committed references under `tools/shots/reference/` that used the single `fixture` key were rewritten to the `fixtures` form that `fixture_hashes` writes, with the same default-template hash under `default`. `manifest_fixtures` in `tools/compare_shots.py` now reads only `fixtures`, and its docstring and the module docstring say so. Behaviour change: a manifest still carrying the old `fixture` key is now a `KeyError` rather than being read; none remain in the repo.

`compare_shots.py:275-277` reads the legacy fixture key or the new fixtures dict. :453 writes fixtures. The legacy read is kept alongside the new write.

### V-037 [low] ExifTool version given as an example and a probe version in code — PARTIAL

**Disposition: No change.** There is no conflicting pin, and the probe note is dated.

`install_exiftool.py:8` uses --version 13.59, which matches the Justfile pin (`exiftool_version` 13.59.0, `exiftool_source_version` 13.59). The 13.50 probe at exiftool/src/lib.rs:187 is a dated test note, not a conflicting pin. So the 'inconsistent' example does not hold; the probe note is only a historical reference.

### V-038 [low] Gate baseline commit named two ways — PARTIAL

**Disposition: No change.** Both references are accurate for their dates.

PROFILING.md:401-402 says the baseline was last re-measured at e7576f0. `REFACTOR_MEASUREMENTS.md:82` says HEAD's counts are baseline.json at a22fbad, in a dated 2026-11-04 section. Both are accurate for their dates, so they are not contradictory. The doc does not say which is current.

### V-039 [low] Argon2 implementation named differently between design and measurement — PARTIAL

**Disposition: Fixed.** `WEB_CLIENT_DESIGN.md` points to the argon2-cffi measurement in `WEB_CLIENT_TECH_DECISIONS.md`.

`WEB_CLIENT_DESIGN.md:174` proposes the argon2 Rust crate 0.5.3 with m=65536, t=3. `TECH_DECISIONS.md:16` states the measurement used argon2-cffi 25.1.0 (Python). The library difference is stated in `TECH_DECISIONS`, so 'undocumented' overstates it; only the design has no pointer to the measurement's library.

## persisted-formats

### PF-001 [high] PDQ dihedral blob size documented as 256 bytes but stored as 128 — CONFIRMED

**Disposition: Fixed.** The `pdq_dihedral` comment in `crates/catalog/src/baseline.sql` now says 4 orientations, 128 bytes per row, matching `pdq.rs` `ORIENTATIONS` and `BLOB_BYTES`. The memory note in `docs/plans/DECODE_ONCE_PLAN.md` (section 3.6) now gives 4 × 32 bytes and about 128 MB at a million files. The code is the source of truth here, so the docs were corrected rather than the blob widened; no schema or stored-data change.

baseline.sql:30 says 8 orientations of 32 bytes (256). pdq.rs:8 ORIENTATIONS = 4 and pdq.rs:118 `BLOB_BYTES` = ORIENTATIONS × `HASH_BYTES`, `HASH_BYTES` = 32 (pdq.rs:4), so the stored blob is 128 bytes. The `DECODE_ONCE_PLAN:526` figure of 256 MB also assumes 8. The `grow_catalog.py:78` cite is at `tools/profiling/pairing/grow_catalog.py`.

### PF-002 [high] ExFat refusal spelled 'ex-fat' in serde and 'exfat' in code() — CONFIRMED

**Disposition: Fixed.** `RefusedReason::code()` in `crates/model/src/refusal.rs` now returns `ex-fat`, the same kebab-case form serde persists and every other variant's code uses. The Fluent selector in `crates/client/locales/en/text.ftl` (`refused-reason`) changed from `[exfat]` to `[ex-fat]` so the words still select. Behaviour change: the code string in logs (`Display`) and the Fluent key are now `ex-fat`; the persisted notice tag is unchanged, so no stored data is affected.

refusal.rs:8 serde uses `rename_all` kebab-case, so ExFat serialises as ex-fat, while refusal.rs:79 code() returns exfat. Both are strings for the same variant; the serde one is the persisted tag.

### PF-003 [high] Unrated stored two ways and ratings CHECK forbids the file-side form — PARTIAL

**Disposition: No change.** `writers.rs:541` documents that a user's 0 stars clears the row. `file_ratings` allows 0 by design.

Unrated is stored two ways: the ratings table (baseline.sql:81) has CHECK BETWEEN 1 AND 5, so the user side deletes the row for 0 stars (writers.rs:545-546), while `file_ratings` (baseline.sql:85-88) allows 0..5 and takes Some(value) unchanged. The title's claim that the ratings CHECK forbids the file-side form is muddled: the CHECK on ratings applies to the user side. The writers.rs:380 and :383 cites are the pair-delete and analysis insert, not ratings writes.

### PF-004 [high] Web client design docs disagree on thumbnail caching and key contents — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** O11 option (a): private, max-age=604800, immutable; the owner may revisit.

`WEB_CLIENT_DESIGN.md:1322` says the thumbnail URL includes turns and the response is public, immutable. :1495 says no turns component and private. :687 says private. `WEB_CLIENT.md:317` says public and keyed without turns. The design doc contradicts itself and the design doc contradicts `WEB_CLIENT.md`.

### PF-005 [high] Session token digest algorithm disagrees within `WEB_CLIENT_DESIGN` — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** The device key is a secret; the server stores its BLAKE3 digest; the design and reconciled docs agree.

`WEB_CLIENT_DESIGN.md:178` and :340 store the BLAKE3 digest of the token. :1694 (labelled Proposal) says the server stores its SHA-256. The same document gives two digests for the same stored token.

### PF-006 [high] QML Settings keys in camelCase vs gallery.json `snake_case` keys — CONFIRMED

**Disposition: No change.** The `Settings { id: appSettings }` in `crates/desktop/qml/Main.qml:39` is not Qt's `QtCore` settings store. `Main.qml` imports only `QtQuick`, `QtQuick.Controls.Fusion`, `QtQuick.Window` and `pg.gallery`, and `pg.gallery` exports the Rust `Settings` QObject (`crates/desktop/src/settings_bridge.rs`, `#[qml_element]`). Its camelCase properties (`writeBack`, `detectBorders`, and so on) are cxx-qt renames of the snake_case `Prefs` fields, read and written through `app::get().prefs()` into `gallery.json`. So there is one store, and the only key mismatch is the Qt property name, which is by design. `tests/qml/tst_viewer.qml` uses the same type.

Main.qml:39 still declares Settings { id: appSettings }, and Main.qml:690-691 feed writeBack and detectBorders from it. Rust reads the same preferences from prefs().get() (settings.rs:26-27), whose on-disk keys are `snake_case` (prefs.rs:212 GOLDEN). `REFACTOR_DESIGN.md:34` says gallery.json is the only prefs file. The QML Settings store is a live second store that contradicts that, not just a key-case difference.

### PF-007 [medium] `file_failures` pass set documented as thumbs and borders only — CONFIRMED

**Disposition: Fixed.** The `file_failures` schema in `docs/reference/NOTIFICATIONS_DESIGN.md` now lists `'thumbs' | 'borders' | 'import'` for `pass`, matching `FilePass` in `crates/model/src/notice.rs` and the `baseline.sql` comments. The import rows are noted as trigger-kept, as the design's import ledger section says. The cited `schema.rs:114` is a `cache_size` comment, so the design doc was the only stale text. Documentation only.

`NOTIFICATIONS_DESIGN.md:268-270` documents pass as thumbs | borders, but FilePass (model notice.rs:375-382) has Thumbs, Borders and Import. The cited schema.rs:114 comment is about `cache_size` and does not list pass values, so that cite is wrong; the real stale doc is `NOTIFICATIONS_DESIGN`.

### PF-008 [medium] Ledger pass and kind codes spelled several ways across catalog, bridge and inbox — CONFIRMED

**Disposition: No change.** The numeric pass index is the UI contract, mapped in one place (ledgers.rs).

Ledger pass is numeric in the bridge (`failed_files_bridge.rs:60`: 0 thumbs, 1 borders, 2 import, 3 scans), text in notices.rs:535 (thumbs, borders, import, scan-folders), and pass.code() or kind.code() bound into SQL (ledger.rs:423-424), and the inbox uses the text code. Three encodings for the same ledger identity.

### PF-009 [medium] Notice level code mapping duplicated in three places — CONFIRMED

**Disposition: Fixed.** The mapping now lives once, on the model's `Level` in `crates/model/src/notice.rs`, as `Level::code()` (0 info, 1 warning, 2 error) and `Level::from_code()` (any other code reads as error, as the catalog did). The copies in `crates/catalog/src/inbox.rs` (`level_code`, `level_of`), `crates/desktop/src/log_model.rs` (`level_code`) and `crates/desktop/src/notices.rs` are removed, and their call sites use the method. Behaviour is unchanged: the catalog's i64 column now reads and binds as i32, which holds these values, and the QML role and toast values are the same integers. Note for compile: `log_model.rs` lost its top-level `Level` import, which moved into its test module.

Level mapping Info=0, Warning=1, Error=2 is written three times: inbox.rs:424 `level_code`, `log_model.rs:64` `level_code` and notices.rs:227 `level_code`. The inbox version is i64, the desktop ones i32. baseline.sql:109 states the same mapping as the schema comment.

### PF-010 [medium] Hard-coded SQL literals where shared constants or macros exist — PARTIAL

**Disposition: Fixed.** `AUTO_VACUUM_INCREMENTAL` is a named constant in catalog and thumbcache. The `0xFFFFFFFFFFFF` mask literal is unchanged; it is documented as deliberate.

The mask 0xFFFFFFFFFFFF is repeated as a literal in sql.rs:91 and at least seven places in work.rs (57, 64, 465, 483, 505, 656, 665), with no named constant, so the repetition is real. The premise that a shared constant exists is wrong for this literal. `live_failures!` (sql.rs:130) is already a macro and its literal is documented as deliberate, because the partial index needs the literal condition. The thumbcache `auto_vacuum` literal 2 (schema.rs:229) has no named constant, a real case.

### PF-011 [medium] Temp-file suffix convention diverges — CONFIRMED

**Disposition: No change.** The dot-name rule holds for temps beside photos; app data files use `<file>.tmp`.

Temp names differ: writeback file.rs:46 uses a leading dot and .pg-write; yandex save.rs:50-52 uses `.<name>.pg-save`; crop lib.rs:402 uses .pg-crop; app store.rs:118 uses `<name>.tmp`, not dot-prefixed; eval plant.py:38 uses .tmp. The model notice.rs:1397 test fixture uses .pg-tmp, which matches no real writer.

### PF-012 [medium] Version meta key names differ for the same thumbnail version — PARTIAL

**Disposition: No change.** The keys are in different databases and name different constants.

`thumb_output_version` is written to thumbcache meta (thumbcache schema.rs:156), and `thumbs_version` is the catalog meta mark (core.rs:225, fixture/raw.rs:270). They are different databases and different constants (`THUMB_OUTPUT_VERSION` and `THUMBS_VERSION`). The writer.rs:393 cite is a comment about eviction and does not name a key. The cited key names are not the same value under two names.

### PF-013 [medium] Missing value encoded as -1 in some places and null or NULL in others — PARTIAL

**Disposition: No change.** The persisted sentinels are documented in `prefs`; the others are in-memory UI values.

Persisted -1 is the prefs `last_file` sentinel (prefs.rs:104, 'a negative number reads as none') and the eval ids .npy (`cpu_worker.py:54`). Most other -1 cites (LaneRow.qml:17, VideoLog.qml:9) are in-memory UI sentinels that are never written. SQL uses NULL for unknown created (baseline.sql:208). The persisted mix is real but narrower than the finding says.

### PF-014 [medium] Image URL grammar differs between the Rust parser and QML — REFUTED

Viewport.qml:32 does not exist. A search for a fit/ image URL form in the QML and in client image.rs found nothing. The shown image URLs are `image://full/<key>/full`, `image://full/`... region and `image://thumb/<key>/<height>` (items.rs:72-81, client image.rs:159), and they parse back to the same request. No second grammar is in use.

### PF-015 [medium] Web client thumbnail URL shape differs between two docs — CONFIRMED

**Disposition: Fixed.** The thumbnail URL has the settled shape in both docs.

`WEB_CLIENT_RECONCILED.md:320` has `/api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@`... ; `WEB_CLIENT_DESIGN.md:691` has `/api/thumbs/{file_id}@{mtime}.{rotation}.{thumbs_version}/{height}` with no `catalog_id` and a different version position. Two documents specify the same URL differently.

### PF-016 [medium] Web client device cookie name and path differ — CONFIRMED

**Disposition: No change.** The finding's cited text is stale: `WEB_CLIENT_DESIGN.md` no longer names `pg_device` or Path=/api/login. Its current cookie decision (about line 340) is `gallery_device`, Path=/api/auth/login, SameSite=Lax, which is what `WEB_CLIENT_RECONCILED.md` section 2.4 (line 543) states. `pg_device` now appears only in the reconciliation log's list of dropped names (`WEB_CLIENT_RECONCILED.md:1286`), so the two decision docs agree.

`WEB_CLIENT_DESIGN.md:340` names the device cookie `pg_device` with Path=/api/login and SameSite=Strict. `WEB_CLIENT_RECONCILED.md:543` names `gallery_device` with Path=/api/auth/login and SameSite=Lax. The name, path and SameSite all differ between the two docs.

### PF-017 [medium] Quarantine location and migration number disagree in web client docs — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** Quarantine is dropped per TECH_DECISIONS:200; the design and reconciled documents are updated.

`WEB_CLIENT_DESIGN.md:941` keeps a per-volume `_quarantine` folder and Recycle Bin is rejected; `WEB_CLIENT_TECH_DECISIONS.md:200` says quarantine is dropped and deletes go to the Recycle Bin. The migration number also differs: DESIGN:952 says migration 10 adds quarantine tables, RECONCILED.md:440 says migration 11.

### PF-018 [medium] README data folder list omits thumbs.sqlite — PARTIAL

**Disposition: Fixed.** `crates/README.md` names `thumbs.sqlite`.

crates/README.md:288-290 lists the catalog folder as holding catalog.sqlite and the thumbnail cache, without the file name. The same README names thumbs.sqlite at line 71 and at 360. layout.rs:75 puts thumbs.sqlite in `catalog_dir`. The data-folder list omits the file name, but does name the cache, so this is a wording gap, not a missing file.

### PF-019 [medium] `io_attempts` columns and triggers differ between two plan docs — CONFIRMED

**Disposition: Fixed.** io_attempts columns and trigger match between the two plans.

`DECODE_ONCE_PLAN.md:409` defines `io_attempts(file_id`, pass, attempts) with no `next_try_at`. `IMPORT_FAILURES_PLAN.md:385` defines `io_attempts` with `next_try_at` DEFAULT 0 and an `io_attempts_due` index. The two plans give different column sets for the same table name.

### PF-020 [medium] `work_leases` and `root_removals` columns differ across `TO_10` and the full design — CONFIRMED

**Disposition: Fixed.** BOUNDED_MEMORY_PLAN says which root_removals columns come when.

`BACKGROUND_JOB_AUDIT_FIXES.md:49` gives `work_leases` six columns. `BOUNDED_MEMORY_PLAN.md:462` gives twelve columns. `root_removals` also differs: `BACKGROUND_JOB_AUDIT_FIXES.md:52` (id, path, hidden, report, finished, `cursor_path`, `cursor_id`) against `BOUNDED_MEMORY_PLAN.md:434` (which adds `key_lo` and `key_hi`).

### PF-021 [medium] AUTOINCREMENT declared inconsistently across plans and catalog — REFUTED

The docs agree with the built catalog. baseline.sql:204 and :208 declare roots, folders and files with AUTOINCREMENT, and scans (baseline.sql:7-8) has no AUTOINCREMENT, which matches `BACKGROUND_JOB_AUDIT_FIXES.md:140`. DATABASE.md:33 lists files, folders and roots and does not mention scans, which is accurate. The mfolders table exists only in `CATALOG_SIZE_PLAN` as a planned table.

### PF-022 [medium] Moved-aside catalog and backup file names — REFUTED

The names are one scheme across code and docs. rebuild.rs:26 uses .building, rebuild.rs:31 uses .pre-v{version}.bak, and rebuild.rs:44 uses .partial beside the backup. The moved-aside names (.replaced, .bad, numbered .1) come from one function, dbfile remove.rs:14. inbox.rs:44 and rehearsal.rs:79 describe the same names. The cited lifecycle.rs:752 is a test that uses the same scheme.

### PF-023 [medium] Analysis row version uses `IMPORT_VERSION` — PARTIAL

**Disposition: No change.** Using `IMPORT_VERSION` for the analysis is by design (`versions.rs:10`).

The analysis version is `IMPORT_VERSION` by design: versions.rs:10 says the analysis pipeline's version is `pg_analysis::analyze::IMPORT_VERSION`, so an analysis made at another version is redone. The value is documented. Naming it `IMPORT_VERSION` is confusing, but the code and the doc agree, so this is a naming issue, not a stored-value defect.

### PF-024 [medium] Manifest key case, path style and encoding differ across build tools — CONFIRMED

**Disposition: Deferred.** The manifest keys and paths are aligned; the recipes have not been run to diff the two manifest outputs (RUN).

`build_static_ffmpeg.py:253` writes CRT entries with keys Name, MSVCRT, LIBCMT, while :294 writes libraries with name, bytes, sha256. `build_jpeg_turbo.py:143` stores an absolute library path, and `build_static_qt.py:226` stores a path relative to the prefix. Key case and path style differ across the three manifests.

### PF-025 [medium] Fixture hash input separators differ inside one tool — CONFIRMED

**Disposition: Fixed.** `broken` in `fixture_hashes()` (`tools/compare_shots.py`) now builds its lines as `os.sep` + relative path, the same form `fixture_hash()` uses for the default template, instead of `as_posix()` with no leading separator. One separator scheme now runs through the tool; the default's leading separator stays, because the PowerShell version depends on it. Behaviour change: the broken template's recorded hash changes, so references recorded under `tools/shots/reference/` will report the broken-template scenarios as changed fixtures until they are re-recorded. That re-record needs the Windows shot run, which this pass cannot do.

`compare_shots.py:258` prefixes each name with os.sep, and :267 uses `as_posix` for broken files. The docstring at :256 explains the leading separator for compatibility with the PowerShell version, so that part is intentional. The two functions still use different separators inside one tool.

### PF-026 [low] Rotation write named Turns, rotation and turns — CONFIRMED

**Disposition: Fixed.** `WriteKind::Turns` and `Write::Turns` are renamed to `Rotation`, and the `write_turns!` macro to `write_rotation!`. The stored key and the temp name stay `rotation` (the variant and the key now agree in name), and `the_rotation_kind_is_still_stored_as_rotation` pins the key.

The Rust kind is WriteKind::Turns (writeback.rs:470, :836, :875), but the queue key is the string 'rotation' (writeback.rs:928 and queue.rs:70 via `write_rotation!`). queue.rs:562 uses the temp name .b.jpg.turns.pg-write with 'rotation' as the kind.

### PF-027 [low] Span name differs for the pairing pass — CONFIRMED

**Disposition: No change.** The pairing pass follows the same two-span pattern as every other pass, so it is not an outlier. `crates/jobs/src/control.rs` (`span_name`, about line 235) opens a phase span per `Phase` (`phase.pairing`, like `phase.import` and `phase.borders`), and each pass function opens its own pass span (`pass.pairing` in `crates/import/src/pairs.rs:136`, like `pass.import` in `crates/import/src/lib.rs`, `pass.borders` and `pass.thumbs`). The phase span wraps the pass span, and the perf report reads both. The cited `import/src/control.rs` does not exist; the file is `jobs/src/control.rs`.

pairs.rs:136 opens the span pass.pairing, while control.rs:248 maps Phase::Pairing to phase.pairing. The cited import/src/control.rs:248 is wrong; the real file is jobs/src/control.rs. The span and phase names for the same work differ.

### PF-028 [low] Tombstone retention and listing key proposals conflict with decided docs — PARTIAL

**Disposition: No change.** Both documents treat retention as undecided.

`INCREMENTAL_SYNC.md:60` proposes replacing 90-day retention and marks it Recommended, not accepted. `TECH_DECISIONS.md:207` keeps 90 days and marks it as a proposal. Both documents treat retention as undecided, so they do not contradict each other on the decision. The conflict with the bounded-memory plan's Phase 2 freeze is noted in `INCREMENTAL_SYNC.md:58` and is real, but it is one open item, not a split decision.

### PF-029 [low] Historical thumbs-v3 cache name in profiling notes — PARTIAL

**Disposition: No change.** The note is dated history, not a live name.

`START_PROFILE.md:219` is a historical note that the first run of a new build made the empty thumbs-v3.sqlite. That is a dated record, not a live name. The PROFILING.md:170 cite does not exist: grep for thumbs-v3 in docs finds it only in this audit. PROFILING.md:171 names thumbs.sqlite, which is the current name.

### PF-030 [low] Catalog `application_id` checked only when `user_version` is positive — PARTIAL

**Disposition: Decided and applied.** The catalog checks `application_id` once a file has objects or a version, at any version. A new file has neither. Regression test: `the_application_id_is_checked_at_version_zero_too`.

schema.rs:310 checks `application_id` only when `user_version` is above 0. judge.rs:131 treats id 0 as not foreign and any other differing id as foreign. The two rules differ for id 0 and for version 0. Neither doc states which is intended, so this is a real divergence whose intent is unrecorded.

### PF-031 [low] JSON indentation varies across persisted tool outputs — CONFIRMED

**Disposition: Fixed.** Every JSON file the eval and tools scripts write, and the console print of the same objects, now uses `indent=2`: the former `indent=1` and no-indent writers in `eval/ann/run.py`, `eval/ann/report.py` (its embedded JSON too), `eval/ann/cpu_worker.py`, `eval/ann/methods.py`, `eval/bench_ann.py`, `eval/bench_semantic.py`, `eval/bench_thumbs.py`, `eval/group.py`, `eval/viewer.py` (marks, `ensure_ascii` kept), `eval/plant.py`, `tools/modgraph/api_surface.py`, `tools/modgraph/module_graph.py` and `tools/linux.py`. Two writers stay compact on purpose: the JSON Lines file in `eval/ann/run.py` (one record per line) and the HTTP reply in `eval/viewer.py`. Behaviour change: the file bytes differ in whitespace only, and every reader parses JSON, so none breaks.

eval/ann/run.py:497 writes `verify_meta.json` with no indent, :300 uses indent=1, report.py:178 uses indent=1, `bench_decode.py:207` and verify.py:901 use indent=2, and tools/perf-compare.py:102 uses indent=2 with `sort_keys`. Five indent choices across eval and tools.

### PF-032 [low] CSV schemas differ for the same kind of output — CONFIRMED

**Disposition: No change.** The files that are the same kind already share a schema: `misses_*`, `hard_negatives_*` and `review_*` in `eval/evaluate.py` all write `similarity, path_a, path_b`. `unpaired_pairs_*` adds a `mutual` column that carries a real fact (whether the two are mutual nearest neighbours), so it is a different kind of list. `pairs_knn.csv` (`eval/ann/run.py`) is an id-based input for `verify.py`, not a list of paths for a person, so its schema follows its consumer. The timing CSV (`eval/jpeg-decoders/imagereduce_timing.py`, fixed `FIELDS`) and the switch CSV (`imagereduce_switch.py`, keys derived from the rows) are two scripts whose readers are separate, so neither is made to match the other. Changing any of these would alter a file a reader depends on for no gain in consistency.

`pairs_knn.csv` has header a,b,label with integer ids (run.py:500). `misses_*.csv` has `similarity,path_a,path_b` with file paths (evaluate.py:156). `unpaired_pairs` has a fourth column layout (evaluate.py:177). `imagereduce_timing.py:27` has a fixed FIELDS list that `imagereduce_switch.py` does not match. `bench_ann.py:190` writes exact.npz keys ids, sims, rate while the other npz keys differ.

### PF-033 [low] pairKey implemented twice in the eval viewer — CONFIRMED

**Disposition: Fixed.** `eval/viewer.html` now defines one sort-and-join, `keyOf(a, b)`, and `pairKey(r)` is `keyOf(r.path_a, r.path_b)`. The second copy that was declared beside `setStyle` is removed, so the existing `keyOf` call sites (the group and mark code) use the single definition. No behaviour change: both copies produced the same string.

viewer.html:165 defines pairKey as sort-and-join with |, and viewer.html:607 defines keyOf as the same expression. The viewer's own comment at :164 says it is the same as viewer.py's `pair_key`, so there are two copies in one file plus the Python one.

### PF-034 [low] Analysis row column lists disagree with the plan — PARTIAL

**Disposition: No change.** The planned upsert is not built yet.

writers.rs:382-383 INSERT OR REPLACE names 18 columns, and no `dups_state` appears in the catalog source. `DECODE_ONCE_PLAN.md:475` describes a 29-column upsert with `dups_state`, which is a planned change. The plan is not built, so the drift is expected unless the plan says it is done. The 27-column `analysis_rewritten` trigger (plan :716) is also a planned statement that was not checked in code.

### PF-035 [low] Research manifest has double .html extensions — CONFIRMED

**Disposition: Fixed.** The five captured pages `08`, `36`, `44`, `45` and `48` in `docs/research/` are renamed on disk to a single `.html` (a plain rename, not staged), and their five rows in `docs/research/MANIFEST.tsv` now give the same names. The cause is that the saver appended `.html` to URLs that already end in `.html`. The content hashes in the manifest are of the bytes, so they are unchanged. Nothing else in the repo names these files.

docs/research/MANIFEST.tsv has five rows ending .html.html (grep count 5), matching the rows 08, 36, 44, 45 and 48.

### PF-036 [low] Inbox key separator differs between the catalog and QML fixture — PARTIAL

**Disposition: No change.** The catalog key and the desktop key are different identities.

The catalog's `key_text` (inbox.rs:394-396, used at :366) joins with U+001F. The desktop InboxEntryId key is row:{id}:{uid} and ledger:{code} (notices.rs:156-163), a different identity used for selection. The QML fixture (`tst_inbox_panel.qml:109`, row:2:2) follows the desktop key, not the catalog one. The cited fixture therefore agrees with the desktop side. Whether the catalog key and the desktop key should match was not settled, so the finding names the wrong pair.

## architecture

### A-001 [high] thumbcache reads catalog tables directly despite views-only claim — CONFIRMED

**Disposition: Fixed.** Catalog views for the high-water mark, file ids, volumes and failures; thumbcache reads views only.

thumbcache reader.rs reads the catalog's tables directly through ATTACH: `cat.sqlite_sequence` at :33 and cat.files in `catalog_volumes` at :121-123. The crate's own comment (Cargo.toml:11-13) says it reads through the views alone. The pg-catalog entry at Cargo.toml:24 is the dev dependency for the tests, which the comment also says.

### A-002 [high] `REFACTOR_DESIGN` states a wrong desktop dependency set and a non-existent query path — PARTIAL

**Disposition: Fixed.** `REFACTOR_DESIGN` says the desktop has eight pg crates at runtime (it includes `pg-perf`).

`REFACTOR_DESIGN.md:11` says the desktop depends on seven pg crates at runtime. desktop Cargo.toml:20-22 lists eight, including pg-perf, so the count is wrong. `REFACTOR_DESIGN.md:46` says `pg_catalog::query` does not exist and that `from_json` and `to_json` are in pg-query. pg-query has both (lib.rs:319 and :325), so that line is correct, and the finding misreads it.

### A-003 [medium] NoticeSink described three ways and held as trait object and concrete type — CONFIRMED

**Disposition: Fixed.** `docs/reference/NOTIFICATIONS_DESIGN.md` now describes `NoticeSink` as the `pg-jobs` trait, which is what the code has, in place of the `Arc<dyn Fn>` sentence. The concrete holding is gone: `LibraryService` (`crates/app/src/library.rs`) and `PrefStore` / `PrefSave` (`crates/app/src/prefs.rs`) now hold `Arc<dyn NoticeSink>`, since they only post and resolve, and the `Inbox` import is removed from both. `App::open` in `crates/app/src/app.rs` passes `Arc<Inbox>` unchanged, which coerces at the call. Behaviour is unchanged.

`NOTIFICATIONS_DESIGN.md:309` describes NoticeSink as Arc of a Fn in pg-library. :139 says it is a trait, and :69 says it moved to pg-jobs. The code has a trait in jobs `notice_sink.rs:1`, library holds Arc of dyn NoticeSink (library.rs:55), and LibraryService and PrefStore hold a concrete Arc of Inbox (library.rs:273, prefs.rs:142). Three descriptions and two holding styles.

### A-004 [medium] Model holds display text against its own rule — CONFIRMED

**Disposition: Fixed.** `FolderTree::anchors` in `crates/model/src/tree.rs` now returns the anchor names as an iterator of `&Name`, in text order, and no longer builds a string. The space-joined `D: E:` text moved to `pg_client::text::anchors` (`crates/client/src/text.rs`), which the desktop's `ROLE_DRIVES` in `crates/desktop/src/folders.rs` now calls. The model test, the catalog's `flat` helper and the `multi_anchor` check in `crates/catalog/src/browse/read/tests.rs` use the names directly. This is the placement REFACTOR_LEDGER F-37 asks for; the ledger row itself is not edited here.

model tree.rs:311-313 returns FolderTree::anchors as a String of display text, D: E:. `REFACTOR_LEDGER.md:98` (F-37) records it as a shortcut: the plan puts display text in pg-client. notice.rs:1-4 says nothing in the model is English, so the rule is stated and this method breaks it.

### A-005 [medium] Picker purpose is a number in one place and an enum in another — CONFIRMED

**Disposition: Fixed.** QML now names the purpose: the new `crates/desktop/qml/PickerPurpose.qml` (an enum `Purpose` with `AddFolder`, `SaveCrops`, `YandexSave`, registered in `crates/desktop/build.rs`, the same pattern as `PanelKind.qml`) replaces the bare `pick(0)`, `pick(1)` and `pick(2)` calls in `Main.qml` and `YandexPanel.qml`. The Rust `Purpose` enum in `crates/desktop/src/picker/state.rs` stays the source, and its doc now says the codes are the positions in `PickerPurpose`; the bridge docs in `picker_bridge.rs` and `gallery_bridge.rs` name the variants. The codes themselves are unchanged, so no behaviour changes. A new test in `crates/desktop/src/picker/state.rs` (`picker_purpose_lists_the_variants_in_code_order`) fails if the order in `PickerPurpose` and the codes come apart.

The picker's purpose is a Rust enum (edits.rs:166, Purpose::SaveCrops), a number in the bridge doc (`gallery_bridge.rs:163` says purpose 1), and a bare number from QML (YandexPanel.qml:168 calls savePicker.pick(2)). The same value is named three ways.

### A-006 [medium] Desktop imports the same app items from different paths — CONFIRMED

**Disposition: Fixed.** On the current tree no edit is needed: every desktop import of a crate item goes through `pg_app::reexport` (`InboxEntry`, `Facets`, `LaneProgress`, `Begun`, `Change`, and the rest), and every import of an item `pg_app` defines itself (`LaneLines`, `Failed`, `FailedItems`, `InboxEvent`, `InboxListing`, `Prefs`) goes through the `pg_app` root. The lines the finding cites have moved; `crates/desktop/src/notices.rs:98-99` now shows the two paths split by that rule, not a conflict. Checked with a grep of `pg_app::` across `crates/desktop/src`.

Desktop imports app items from two paths: notices.rs:93 and `failed_files/state.rs:4` and filters.rs:16 use `pg_app::reexport`, while notices.rs:94, `lane_model.rs:23` and `failed_files.rs:17` import the same kind of item from `pg_app` directly. The reexport module is the documented boundary, so the direct imports are inconsistent with it.

### A-007 [medium] Re-export allowlist differs between design doc and boundary tool — CONFIRMED

**Disposition: Fixed.** The tool's allowance for `crates/testkit/src/lib.rs` is kept, since that crate exists to re-export `pg-platform`'s test folders under the names the tests use, and the design doc's ground rule 2 in `docs/reference/NOTIFICATIONS_DESIGN.md` now lists both boundary files and names `REEXPORT_BOUNDARIES` as the check. The doc was the side that was out of date. No code or tool change; the check's behaviour is unchanged.

`NOTIFICATIONS_DESIGN.md:197` says a production pub use naming another crate is allowed only in app/src/reexport.rs. boundaries.py `REEXPORT_BOUNDARIES` (lines 33-35) also allows crates/testkit/src/lib.rs. The design doc and the check disagree on the allowlist.

### A-008 [medium] Module docs name crates outside their dependency set — PARTIAL

**Disposition: No change.** The references are prose, not imports.

client image.rs:51, render transcode.rs:1-2, thumbs placeholders.rs:7 and log file.rs:9 name crates outside their dependency set. These are prose references, not imports, so no build depends on them. They only become broken links if written as intra-doc links, and the cites show plain prose. The finding is about wording, not dependency.

### A-009 [medium] Test fixtures come from four different places — CONFIRMED

**Disposition: Fixed.** VIDEO_SAMPLE and the shared image builders are in pg-testkit; library drops its direct image dev-dependency.

Test fixtures come from four places: pg-testkit (edit Cargo.toml:10-11 says so), `pg_codec::fixtures` (edit crop.rs:156 and rotate.rs:89), and a file from another crate, `include_bytes` of ../../video/tests/sample.mp4 (library tests writeback.rs:291). `REFACTOR_LEDGER.md:63` (F-18) says the image dev-dependency should move to pg-testkit.

### A-010 [medium] Platform path rules reached directly instead of injected or wrapped — PARTIAL

**Disposition: No change.** The desktop uses native rules by design (`catalogs.rs:17-21`).

Direct uses of `PATH_RULES` are real: schema.rs:269 and :291 pass it into `bring_to`, which takes rules as a parameter (:146-148), and desktop folders.rs:49, `failed_files.rs:76` and gallery model.rs:99 use it directly. The desktop uses the native rules by design, as catalogs.rs:17-21 documents, so only the places that skip the wrapper are open.

### A-011 [medium] Catalog module docs omit or contradict their imports — CONFIRMED

**Disposition: Fixed.** The module docs of `crates/catalog/src/lifecycle.rs` now name every module it uses (`progress`, `transaction`, `ledger` as well as `types` and `error`), and that of `crates/catalog/src/writers.rs` names the lease, `current` and `lifecycle` it imports. Each list was built from the file's non-test `crate::` uses, so the docs match the imports; the code is unchanged.

lifecycle.rs:2-3 says the file uses types and error and that its SQL macros come from sql. Line 14 imports crate::progress::FreeSpace, which the doc does not name. writers.rs:5 does not list the lease and lifecycle imports at lines 136-137. The module docs are out of step with their imports.

### A-012 [medium] Browse read side reaches into borders and writers despite leaf claim — CONFIRMED

**Disposition: Fixed.** The one reach was `browse/read/details.rs` calling `crate::borders::border_sides_on`. That read of the `border_sides` table now lives in `browse/read/details.rs` as `border_sides_on`, re-exported from `browse/read.rs`, and `Catalog::border_sides` in `crates/catalog/src/borders.rs` calls it, so the read side no longer depends on borders. The read side has no reach into writers (the finding's second claim does not hold; grep finds none). The `scope.rs` leaf doc is accurate for `scope.rs` itself and is unchanged. Behaviour is unchanged.

browse/read/details.rs:46 calls `crate::borders::border_sides_on`, so the read side reaches the borders module. borders.rs:12-14 imports core, current, error and the writers. scope.rs:2 says the read-side scope is a leaf that uses nothing of its siblings, which the details call contradicts.

### A-013 [medium] Library lane and inner module docs contradict their imports and calls — CONFIRMED

**Disposition: Fixed.** The module doc of `crates/library/src/lanes.rs` no longer says the whole module is pure. It now says the grouping (`plan`, `assign`, `place_on_volume`) is pure and tested without a disk, and names the two probing paths, `probe_place` (which calls `pg_scan::root_volume` and a `Drives`) and `groups` (which uses `SystemDrives`). Only the doc changed; the `pg-scan` dependency is the plan's to fold, not this doc's to settle.

lanes.rs:5-6 says the module is pure and tested without a disk. groups at :74-76 calls SystemDrives, and `probe_place` at :177 calls `pg_scan::root_volume`. The module is not pure, and it depends on pg-scan, which the plans say is being folded into pg-library.

### A-014 [medium] Platform rule enforced through different mechanisms per crate — PARTIAL

**Disposition: No change.** The real gap is an unadopted process ban, listed in `CLIPPY_LEDGER.md`.

The platform rule uses different mechanisms by design: boundaries.py checks the platform words with an empty `PLATFORM_GLUE` (boundaries.py:40), and clippy `disallowed_types` is the mechanism for process spawning. `CLIPPY_LEDGER.md:299` lists the process ban as warn with adoption still to do. The mechanisms are not one rule in one place, but each rule has a check; the gap is the unadopted ban, not a conflict.

### A-015 [medium] Dependency and library-crate counting differs between module-graph tools — CONFIRMED

**Disposition: Fixed.** normal_pg_deps in compiler_graph.py is used by edges.py and api_surface.py.

The module-graph tools count library crates and dependencies in different ways: `compiler_graph.py:38` uses any lib in kind, `unreachable_pub.py:27` uses kind equal to ['lib'], `api_surface.py:28` reads normal dependencies only, `make_html.py:35` reads all packages, and edges.py:43 reads normal dependencies only. The same question gets different answers across the tools.

### A-016 [medium] Qt objects invoked by string name across the C++ and Rust boundary — CONFIRMED

**Disposition: No change.** The string boundary is intended and a test guards each name.

Qt objects are reached by string name across the boundary: screenshot.cpp reads the QML property shellCore (Main.qml:118) and invokes pending by name, and deadline.cpp:28 calls invokeMethod with the name timeout. A rename in QML or Rust breaks these silently at run time.

### A-017 [medium] Shortcut policy location and QML controller logic conflict — CONFIRMED

**Disposition: Deferred.** The 117 controller functions in QML need Rust facades, each checked by a Qt run.

GlobalShortcuts.qml:4 and Main.qml say what a shortcut does is the controller's, and the controller decides. `REFACTOR_LEDGER.md:116` (F-45) records 106 functions of controller logic still in QML, with Main 25, Viewer 17, InboxPanel 15 and FilterBar 11. The stated policy and the code disagree, and the ledger records it as open.

### A-018 [medium] Version registry location differs from the planned location — PARTIAL

**Disposition: No change.** The registry location matches the code; the planned location was not verified.

`tools/test_versions.py:14` scans crates/model/src/versions.rs as the registry, and versions.rs:1-3 is the registry (commit 991f5119 made the hand-written versions its). The test and the file agree. The finding cites a planned location that the instances do not show, so the mismatch is with the plan, which is not verified here.

### A-019 [medium] Catalog-free command path and catalog folder creation in the CLI — PARTIAL

**Disposition: No change.** The split is intended and documented in the function's doc comment.

cli lib.rs:130-139 runs borders of piped pixels or of outside files without a catalog and creates the catalog folder for every other command. The split is intended and documented in the function's own doc comment, so the two paths differ on purpose.

### A-020 [medium] Python tooling uses different import and run conventions — CONFIRMED

**Disposition: Fixed.** eval/ann modules insert eval on sys.path once; compile-checked (an import smoke test needs torch).

Python tooling uses different import conventions: flat imports (eval/accel.py:33, ann/data.py:13), package imports (ann/exact.py:12), sys.path.insert (`github_files.py:18` and `test_linux.py:11`), a local run function (linux.py:85), and an import from pgtools (`win_build.py:26`) that `linux_build.py` does not use.

### A-021 [low] Historical layering claims in plan docs without status markers — PARTIAL

**Disposition: No change.** The plan states the change as a plan item, which serves as its status.

`BACKGROUND_JOB_AUDIT_FIXES.md:176` says the analysis to platform layer is to be added to crate-layers.txt, and crate-layers.txt:31 still lacks platform for analysis, so the plan is not executed. The finding wants status markers on the plan's layering claims. The plan does state its change as a plan item, which is the status marker the finding asks for.

### A-022 [low] Plan layer maps predate the current crate-layer file — PARTIAL

**Disposition: Fixed.** The stale `DECODE_ONCE_PLAN` layer map is marked stale; `tools/crate-layers.txt` is the current map.

`DECODE_ONCE_PLAN.md:1176` gives an import map of analysis catalog dedup jobs model platform render thumbcache trace. crate-layers.txt:40 gives analysis catalog container dedup jobs model perf platform, so the plan's map is stale. `GUI_THREAD_PLAN.md:111` (catalog may depend on dbfile, log, model, platform, query) matches crate-layers.txt:25, and `MULTI_DRIVE_LANES_PLAN.md:53` (jobs not on catalog or scan) matches crate-layers.txt:28. Only one of the four plan maps is stale.

### A-023 [low] Eval scripts import private helpers and tool internals from other scripts — CONFIRMED

**Disposition: Fixed in part.** verify_types.py holds the shared types; Features and PairRunner remain in verify.py because they need torch and kornia.

`bench_thumbs.py:51` imports the private `_vips` from `bench_decode`, and `verify_synth.py:23` imports ROTATIONS, Checkpoint and PairRunner from verify. Scripts reach into another script's internals.

### A-024 [low] App accessor list omits thumbs() — CONFIRMED

**Disposition: Fixed.** The accessor list in the module doc of `crates/app/src/app.rs` now names `App::thumbs` between `edit` and `library`, matching the accessors `App` has. Doc-only change.

app.rs:6-8 lists the accessors inbox, catalogs, prefs, images, reading, views, edit, library and yandex. The thumbs accessor at :121-123 is missing from the list, though it exists.

### A-025 [low] Import and accessor path style varies within crates — CONFIRMED

**Disposition: No change.** The in-file inconsistencies are fixed; the rest is the usual Rust form.

Import and call style varies: notices.rs:95 imports text and calls render bare, desktop lib.rs:169 calls the full path, providers.rs uses app::get() at :45 and crate::app::get() at :99 in the same file, and library passes.rs:9 imports some crates and calls others by full path.

### A-026 [low] Desktop window modules read app state directly, bypassing wrappers — CONFIRMED

**Disposition: Fixed.** The reader's session was reached as `app::get().reading().session()` in eight window modules. `crates/desktop/src/app.rs` now has `pub(crate) fn session()` next to `get()`, and those eight call sites (`failed_files.rs`, `filters.rs`, `folders.rs`, `gallery/edits.rs`, `gallery/model.rs`, `gallery/state.rs`, `notices.rs`, `shell.rs`) call it. The `library::running()` wrapper that `lane_model.rs` uses stays as it is; the other `app::get().library()` calls in `library.rs` are the service's own methods, not the started job, so they are not bypassing it. Behaviour is unchanged.

`failed_files.rs:144-145` calls the notices helper and reaches app::get().reading().session() directly. library.rs:31 wraps app::get().library().running() in running(), and `lane_model.rs:33` uses that wrapper. The window modules use the wrapper in some places and the app directly in others.

### A-027 [low] Row height constants named by a doc that the code does not use — REFUTED

The doc at shell/state.rs:19 names `pg_model::size::ROW_HEIGHT`. size.rs:10 defines `ROW_HEIGHT` as the range 60 to 800, and RowHeight::MIN and MAX (size.rs:21 and :23) are built from its ends. The code uses the bounds that come from the named constant, so the doc is accurate.

### A-028 [low] Leaf-below-core rationale repeated and reverse edges exist — CONFIRMED

**Disposition: Deferred.** Whether core reaches roots needs a cargo-modules graph; the remaining leaf headers need pointers.

The same leaf-below-core rationale is repeated across the catalog's module headers (dups/identity.rs:1, inbox/id.rs:1, and in failure.rs, ledger state.rs, listing.rs, coverage.rs, lease.rs). roots.rs:44 imports ledger::failure and removal, which shows reverse edges from the module that the core reaches.

### A-029 [low] Catalog comments tie to other crates' internals and phase numbers — PARTIAL

**Disposition: Fixed.** The stale "Phase 4" label in `core.rs` is removed. The other two cited probe files do not exist.

The two probe cites (`stmt_journal_probe.rs:9` and `pairs_write_probe.rs:11`) do not exist at those paths, so those instances cannot be checked. core.rs:109 says Test-only until the crash tests (Phase 4), a stale phase label that a reader cannot resolve. The finding has one real instance and two unchecked ones.

### A-030 [low] Pairing tooling couples to the v13 queue table — CONFIRMED

**Disposition: Fixed.** The delete is in `tools/profiling/pairing/drop_owed_pairs.py`, which keep.ps1 runs before each workload; keep.ps1 itself has no SQL. The script now checks that the `pdq_pairs` and `pairs_owed` tables exist, as `set_owed.py` already does for its two layouts, and exits with a message naming the missing tables instead of a bare "no such table" after a rename. The table names are still the catalog's, so the check is the coupling's guard, not its removal; no change to what is deleted.

keep.ps1:38-40 deletes from `pdq_pairs` where the rows are in `pairs_owed`. Both tables exist in the schema and the probes (`crates/catalog/examples/pairs_write_probe.rs`). The profiling script depends on the table names and the pairing state of schema v13, so a schema rename breaks it without a check.

### A-031 [low] Static Qt wiring lives in patch files that edit crate sources — CONFIRMED

**Disposition: Deferred.** Moving the static Qt wiring into the sources needs a Windows static Qt and FFmpeg build first.

The static Qt build is wired through patch files that edit crate sources: tools/static-qt/app-build.patch:19-21 adds QuickControls2, QuickLayouts and Svg to desktop build.rs under `PG_QT_STATIC`, and pg-video.patch:63-65 adds a cfg-gated use of `pg_platform`. The crate sources differ from what the patches assume if they change.

### A-032 [low] Writeback holds a leaked static thumbnail cache — CONFIRMED

**Disposition: Fixed.** The thumbnail cache is an `Arc<ThumbCache>`, shared by the writeback, the library, the app, the CLI and their tests; nothing is leaked. The read connections that were kept in each reading thread's thread-local are a per-cache pool (`thumbcache::reader::ReaderPool`), so a connection outlives its thread and closes with the cache. `ThumbCache::close` finishes the reads in progress, stops the writer, then closes the idle connections, and the app's shutdown calls it first. The CLI's `write` command closes its cache before it joins its event printer (`write_pending`), with a test that runs it under a timeout.

writeback lib.rs:165-167 holds thumbs as a leaked static reference, and the test at :812 creates it with Box::leak. The production code and the test use the same pattern, so the leak is in the design and in the tests.

### A-033 [low] Desktop and app module boundary details — PARTIAL

**Disposition: No change.** The finding does not show that any boundary is wrong.

keymap.rs:68 imports a cxx-generated MediaKind from `gallery_bridge` in a non-bridge module, and `yandex_bridge.rs:1-2` says the allow is on this module and lib.rs. The shell state.rs:9 carries its own expect. These are small module boundary differences, and the finding does not show that any is wrong.

### A-034 [low] CLI manifest dependency order — CONFIRMED

**Disposition: Fixed.** The sixteen `pg-*` entries under `[dependencies]` in `crates/cli/Cargo.toml` are in alphabetical order now, with the same paths and no other change. Cargo does not depend on the order, so the build is unaffected.

cli Cargo.toml:11-13 lists pg-library before pg-borders and pg-dedup, so the sixteen pg dependencies are not in alphabetical order. Low severity.

## manifests-lints

### ML-001 [high] Client clippy.toml drops the `temp_dir` ban that root and model carry — PARTIAL

**Disposition: Fixed.** The client and model `clippy.toml` now ban `std::env::temp_dir`, as the root does. Neither crate calls it. The model file did not carry the ban before, so the finding's premise was partly wrong.

The root clippy.toml:25 bans `std::env::temp_dir`, and crates/client/clippy.toml has no `temp_dir` entry, so the client's gap is real. crates/model/clippy.toml does not ban `temp_dir` either (it bans `read_to_string`, write and `read_dir` at :20-23), so the finding's claim that model carries it is false. The cite clippy.toml:21 is a comment line; the ban is at :25.

### ML-002 [high] Three clippy.toml files repeat root settings by hand — CONFIRMED

**Disposition: No change.** Clippy has no include, so the repeats are required; a test guards them.

The root clippy.toml:2-4 sets the `allow-*-in-tests` keys, and crates/model/clippy.toml:3-5 and crates/client/clippy.toml:3-5 repeat them by hand. The model file's own comment at :1-2 says to keep them in step with the root file, so the duplication is acknowledged and manual.

### ML-003 [high] Workspace lint comment says warn while clippy lints are deny — PARTIAL

**Disposition: Fixed.** The `Cargo.toml` comment scopes "warn" to the rust lints; the clippy group is deny where it says so.

The comment at Cargo.toml:60 says the lints are kept at warn so work in progress builds, and it sits above the [workspace.lints.rust] table, which is the rust lint group. The clippy group at :71-98 is deny, under its own comment at :79-80. The comment is accurate for the table it sits on and does not cover the clippy group, so the conflict is in reading, not in the lint levels.

### ML-004 [medium] `CLIPPY_LEDGER` level column disagrees with its decision column — REFUTED

`CLIPPY_LEDGER.md:230` and :298 use the Level column for the current level (warn) and the Work column for the target (adopt at deny). Current and target are different columns by design, so the rows do not contradict each other.

### ML-005 [medium] `CLIPPY_LEDGER` warn-first rollout conflicts with the -D warnings gate — CONFIRMED

**Disposition: Fixed.** `docs/plans/CLIPPY_LEDGER.md` now says the 171-name row and its paragraph add each name at `deny` in one change, with the reason that `just lint` runs `-D warnings`. The lints have zero hits, so deny passes the gate directly and no warn stage is needed. The change is to the plan text only; `Cargo.toml` is not edited, so the lint levels are unchanged.

`CLIPPY_LEDGER.md:16` says just lint runs clippy with -D warnings, so a warn-level lint fails the gate. Line 77 says the 171 names are to be added at warn with no code change, and their status is not started. Adding them at warn as the ledger says would fail the gate that the same document describes.

### ML-006 [medium] eval/png-decoders standalone workspace has no dependabot entry or lockfile check — CONFIRMED

**Disposition: Fixed.** `.github/dependabot.yml` gains a weekly cargo entry for `/eval/png-decoders` with the same minor-and-patch group as the other standalone workspaces, and its header comment now lists it. `tools/lint.py` STEPS gains a `cargo metadata --locked` check for `eval/png-decoders/Cargo.toml`, so a stale `Cargo.lock` fails `just lint`. Behaviour change: `just lint` now also resolves that workspace's lockfile, which needs its lock to be current.

.github/dependabot.yml has entries for / (grouped), /eval/jpeg-decoders, /fuzz, /tools/shots and /tools/lint, and none for /eval/png-decoders, which is a standalone workspace. lint.py:23-24 runs cargo metadata --locked on fuzz and eval/jpeg-decoders only. The .gitignore:5-6 note says the PNG workspace has its own Cargo.lock, which nothing checks for staleness.

### ML-007 [medium] Crate-root print-lint attribute present in some crates and absent in others — CONFIRMED

**Disposition: Fixed.** `crates/dbfile/src/lib.rs`, `crates/crop/src/lib.rs` and `crates/thumbs/src/lib.rs` now carry the same `#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]` line and comment as the other library roots, so every library crate has it. I chose adding over removing: removing would change which test targets the lint covers, while adding changes nothing, because the workspace already denies both lints. The per-crate lines stay redundant with `[workspace.lints.clippy]`, which is a known cost of the uniform form.

The print-lint crate attribute is present in catalog lib.rs:12, client lib.rs:3, codec lib.rs:7, container lib.rs:9, dedup lib.rs:6, metadata lib.rs:12 and perf lib.rs:17, and absent from dbfile lib.rs, crop lib.rs and thumbs lib.rs. The workspace deny at Cargo.toml:81-82 and the allow-print-in-tests key already cover it, so the per-crate attribute is partly redundant and its presence is uneven.

### ML-008 [medium] No rust-version declared while newer syntax and APIs are used — CONFIRMED

**Disposition: Fixed.** rust-version 1.89 in the docs and manifests.

No Cargo.toml sets rust-version (verified at P-007 and here). `LINT_TOOLS.md:31` says the MSRV is not set anywhere. The code uses let-chains (dups.rs:85, desktop app.rs:60, scan lib.rs:562) and `is_none_or` (browse tests.rs:579), which need a recent toolchain, so the minimum is not declared.

### ML-009 [medium] Feature names mix singular and plural test-support forms — CONFIRMED

**Disposition: Fixed.** The codec's test-support feature is renamed from `fixtures` to `fixture` in `crates/codec/Cargo.toml`, with its `cfg` in `crates/codec/src/lib.rs` and the dev-dependency lines in `crates/edit/Cargo.toml` and `crates/library/Cargo.toml`. The module `pg_codec::fixtures` keeps its name, so no code path changes. The feature name now matches pg-catalog's `fixture`; the docs that named the old feature (`crates/README.md`, `docs/plans/LINT_TOOLS.md`, `docs/plans/IMPORT_READS_PLAN.md`) follow. Anyone who enables `pg-codec/fixtures` by hand breaks, and only the three dev-dependency lines did.

The test-support feature is fixture in catalog Cargo.toml:14 and cli/library dev-dependencies, and fixtures in codec Cargo.toml:29, with library also enabling codec fixtures at Cargo.toml:39. The singular and plural forms are both used for the same kind of thing.

### ML-010 [medium] allow(...) used where the convention is expect with reason — CONFIRMED

**Disposition: Fixed.** The two `Kernel` variants `Popcnt` and `Avx2` in `crates/dedup/src/join.rs` now use `cfg_attr(not(target_arch = "x86_64"), expect(dead_code, reason = "..."))` in place of `allow(dead_code)`. The `cfg` guard is unchanged, and the expectation holds: on non-x86_64 the variants are never constructed (`best_kernel` and the test `kernels` build them only under `cfg(target_arch = "x86_64")`), so the lint fires and is met. Behaviour is unchanged; the `allow_attributes` warn no longer fires on these lines.

dedup join.rs:288 and :290 use `cfg_attr` with `allow(dead_code`), and the workspace sets `allow_attributes` at warn (Cargo.toml:77). The convention in this repo is expect with a reason, and these two sites have neither.

### ML-011 [medium] QML files missing pragma ComponentBehavior: Bound — CONFIRMED

**Disposition: Fixed.** `pragma ComponentBehavior: Bound` is added to `crates/desktop/qml/WheelListView.qml`, `PanelKind.qml`, `Ratings.qml` and `Theme.qml` (after their existing pragmas), and to `PickerPurpose.qml`, which the grep showed was also missing it. None of the five has a delegate, Repeater or Component of its own, so the pragma changes no binding; it only brings them into line with the other component files. The QML lint (`just lint-qml`) has not been run on this change.

WheelListView.qml:1 and PanelKind.qml:1 have no pragma, Ratings.qml:2 and Theme.qml:4 carry only pragma Singleton, and Viewer.qml:18 has pragma ComponentBehavior: Bound. The pragma is present in some component files and absent in others, which is the inconsistency the finding names.

### ML-012 [medium] `unreachable_pub` expect applied to every bridge module except two — CONFIRMED

**Disposition: Fixed.** `crates/desktop/src/lib.rs` now puts the same ``#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` ...")]`` on `mod settings_bridge` and `mod trace_bridge`, the two bridges that lacked it. Both files have the same `#[cxx_qt::bridge] pub mod qobject` shape as the bridges that already carry the expect, so the expectation is met by the same items. The other bridge declarations were already consistent; the finding's list of exceptions is complete.

desktop lib.rs:76 puts the `unreachable_pub` expect on the screenshot bridge, and :81 and :86 on other bridges. `settings_bridge` (lib.rs:79) and `trace_bridge` (lib.rs:84) are bridge modules with no such expect. The two listed bridges are not the only exceptions, so the bridge list is incomplete in the same way.

### ML-013 [medium] Python noqa suppressions mix bare codes and parenthesised reasons — CONFIRMED

**Disposition: Fixed.** Every `# noqa` that had no reason now has the repo's `# noqa: CODE (reason)` form, with the reason written at each site. Those were the E731 lambdas in `eval/accel.py`, `eval/ann/report.py`, `eval/jpeg-decoders/nvimgcodec_bench.py` (two) and `eval/verify.py`; the E402 imports in `eval/jpeg-decoders/win_build.py` (four), `eval/jpeg-decoders/win_env.py`, `eval/png-decoders/win_build.py` and `eval/test_benches.py` (two); and the S310 calls in `tools/fork-updates.py` and `tools/setup_ffmpeg.py`. The finding's example sites (`accel.py:298`, `report.py:59`) had moved and were among these. No suppression code or behaviour changed; the reasons are comments the linter ignores.

accel.py:131 and run.py:343 use a parenthesised reason after noqa, `nvimgcodec_bench.py:87` uses one too, and accel.py:298 and report.py:59 use noqa E731 with no reason. The reason convention is applied unevenly.

### ML-014 [medium] `REFACTOR_LEDGER` says no wasm32 check exists, but lint.py runs one — CONFIRMED

**Disposition: Fixed.** The F-17 row in `docs/plans/REFACTOR_LEDGER.md` is now marked `done`, with the `tools/lint.py` wasm32 step and the two CI jobs that run `just lint` as its evidence. The row's problem text had been correct when written, so it now says so rather than being deleted. No code changed; the ledger is the stale record, and the check it describes is already in the gate.

`REFACTOR_LEDGER.md:62` (F-17) says there is no wasm32 check in just lint or CI. tools/lint.py:21 runs cargo check for pg-model and pg-client on wasm32-unknown-unknown, and `NOTIFICATIONS_DESIGN.md:367` describes that check. The ledger is stale.

### ML-015 [medium] Boundary bans described as enforced by deny.toml are not all present — PARTIAL

**Disposition: Fixed.** The codec `Cargo.toml` comment now says `deny.toml` names the crates allowed to depend on the image decoders.

`REFACTOR_LEDGER.md:61` (F-16) records the planned windows-sys and cxx bans as missing, and that is an open item, not a claim that deny enforces them. codec Cargo.toml:10 says image decoders are linked here and nowhere else (deny.toml), but deny.toml:56 lets image be wrapped by pg-library, pg-testkit and pdqhash as well as pg-codec. The codec comment overstates the ban.

### ML-016 [medium] Ruff repo-wide errors outside the lint scope — REFUTED

The `TIER2_ANN_BAKEOFF_PLAN.md:117` note recorded 9 ruff errors and one format diff on 2026-11-14. Run today, ruff check . reports All checks passed and ruff format --check . reports 182 files already formatted, with ruff 0.17.3. The repo-wide check is clean now, so the plan's note is stale and nothing is outside the lint scope.

### ML-017 [low] Plan docs cite stale Cargo.toml and deny.toml line numbers — CONFIRMED

**Disposition: Fixed in part.** The live cites are replaced; six remain in dated review text.

Cargo.toml:69 is `undocumented_unsafe_blocks`; `GUI_THREAD_PLAN.md:1719` cites line 65, which is a different lint. `VALUE_PROVENANCE_IDEAS.md:33` cites the cast lints at 86-94, and they are at 88-98. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:192` and :143 cite lines that moved. The stale line citations are real.

### ML-018 [low] WTFPL allowance in deny.toml matches no crate — PARTIAL

**Disposition: Fixed in part.** WTFPL is removed from deny.toml; cargo deny check licenses still needs to run (network).

deny.toml:34 allows WTFPL. Whether any crate uses it needs cargo-deny to run, and I did not run it, so the claim that the entry matches no crate is unverified. LANGUAGES.md:426 is a phase table row, not a cite of this entry, so it does not show the entry is unused.

### ML-019 [low] quick-xml entries left in manifests after the planned removal — PARTIAL

**Disposition: No change.** The removal is planned and has no date, so the entries are expected for now.

`EXIFTOOL_PORT_PLAN.md:378-380` plans the removal of legacy and quick-xml, including fuzz/Cargo.toml:82, deny.toml:53 and :84, and the lock lines. The entries are still present, which is expected while the removal is a plan, not done. The finding treats the planned removal as overdue, but the plan does not set a date.

### ML-020 [low] Windows feature additions planned but not in the workspace list — PARTIAL

**Disposition: No change.** The additions are planned, so the gap is between a plan and the manifest.

LANGUAGES.md:51, `EXIFTOOL_PORT_DESIGN.md:471` and `MULTI_DRIVE_LANES_PLAN.md:135` name Win32 features (Globalization, `System_Time`, `System_Ioctl`) that are not in Cargo.toml:49-54. These are planned additions, so the gap is between a plan and the manifest, not a bug.

### ML-021 [low] Hash-container guidance in plan docs contradicts the clippy ban — CONFIRMED

**Disposition: Fixed.** Item 13 of `docs/plans/SCAN_OPTIMIZATION_PLAN.md` now names the walk's seen set as a `BTreeSet`, with the clippy ban as the reason, matching the validation note at the plan's later section. This is a plan-text fix only; the code already uses the ordered set, so nothing in `crates/` changes.

`SCAN_OPTIMIZATION_PLAN.md:967` says item 13 makes the walk's seen set a HashSet, which the clippy ban forbids. The validation note at :1051 says the hash types were replaced with BTreeSet and BTreeMap. The plan contradicts itself, and the NIGHTLY appendix states the ban correctly.

### ML-022 [low] Cargo build-script directive written with one colon and with two — CONFIRMED

**Disposition: Fixed.** The `cargo::` directives in `tools/static-qt/pg-video.patch` and `tools/static-qt/app-build.patch` are now `cargo:`, the form every repo `build.rs` (`crates/video`, `crates/desktop`, `crates/gallery`, `eval/jpeg-decoders`) uses, and the one mention in `docs/reference/STATIC_BUILD.md` follows. The two forms are equivalent to cargo, so no build changes; the patch hunk headers count lines, which are unchanged. The patches are applied at build time, so a static-build run is the check that they still apply.

tools/static-qt/pg-video.patch:11 uses cargo:rerun-if-env-changed with one colon, and :12 uses cargo::rustc-check-cfg with two. app-build.patch:11 uses cargo:: with two colons. Cargo accepts both, but the patches mix the two forms.

### ML-023 [low] Windows macro gating differs between C++ files — CONFIRMED

**Disposition: Fixed.** The three `#if defined(_WIN32)` tests in `crates/desktop/cpp/video_source.cpp` are now `#ifdef Q_OS_WIN`, the test `gpu_device.cpp`, `gui_app.cpp` and the headers use. The file already includes `<QtCore/QFile>` above the first test, so `Q_OS_WIN` is defined there; it is set wherever `_WIN32` is. No behaviour change, and the `#ifndef NOMINMAX` guards inside the block are untouched.

`video_source.cpp:7` and its other branches use the compiler macro `_WIN32`, while `gpu_device.cpp:22` and its other branches use Qt's `Q_OS_WIN`. The same platform test is written two ways across the desktop C++ files.

### ML-024 [low] Shebang lines present on some Python tools and absent on others — PARTIAL

**Disposition: No change.** Cosmetic; left as is.

The shebang mix is real: tools/profiling/bench.py has a shebang line, `bench_data.py` has none, and cgtab.py and `ci_setup.py` have one. Two of the cited files do not exist at the cited paths (`tools/profiling/grow_catalog.py`, keep.py and medians.py are missing; keep.py is at tools/profiling/pairing/keep.py).

### ML-025 [low] Manifest dependency spelling and ordering vary — CONFIRMED

**Disposition: Fixed.** `crates/desktop/Cargo.toml` writes `cxx-qt-lib` as `cxx-qt-lib.workspace = true` and `cxx-qt-lib.features = [...]`, the dotted form its neighbours use, in place of the inline table; the features are additive with the workspace entry, so the resolved set is the same. `crates/catalog/Cargo.toml` lists its `pg-*` path dependencies in alphabetical order, which moves `pg-platform` after `pg-perf` and `pg-dbfile` to the top of the group. Cargo.lock is not affected, since no version or feature set changes. The dotted feature form is the one place a build could object, so a `cargo metadata --locked` run is the check.

desktop Cargo.toml:31 writes cxx-qt-lib = { workspace = true, features = [...] } inline while the lines above use name.workspace = true. catalog Cargo.toml:21 lists pg-platform before pg-dbfile, out of order with the rest of the list. Low severity.

### ML-026 [low] Edition written literally in two standalone manifests — REFUTED

fuzz and eval/png-decoders are standalone workspaces with their own lockfiles, so they cannot inherit edition from the root with edition.workspace. The literal edition is the only option for those manifests, so the difference is required by their layout.

### ML-027 [low] Expect reason text repeated across state files with escaped apostrophe — CONFIRMED

**Disposition: No change.** The escape is gone; each expect needs its own reason literal.

The same reason, with an escaped apostrophe, is repeated in the desktop state files: `lane_model/state.rs:41`, yandex/state.rs:11, animation/state.rs:11, and the finding counts 11 files in all. The repetition is real. The escape is valid Rust but unusual, and the text should be shared or shortened.

### ML-028 [low] Platform-gated tests named differently in catalog and client — CONFIRMED

**Disposition: No change.** The OS-gated test names are distinct.

lifecycle.rs:812 and :838 use plain cfg(unix) and cfg(windows) tests with descriptive names, while the browse tests use `under_both_rules!` with `_windows` and `_posix` suffixes. The two catalog test styles for the same platform split are real.

### ML-029 [low] Test-file lint allowance style differs between test files — CONFIRMED

**Disposition: Fixed.** `crates/catalog/tests/rehearsal.rs` had a file-wide `#![expect(clippy::unwrap_used)]`; it is now item-level `#[expect(clippy::unwrap_used, reason = ...)]` on its three non-test helpers (`counts`, `check`, `kept_copies`), the form `crates/library/tests/*.rs` uses. The `#[test]` function there relies on `allow-unwrap-in-tests`, as the library tests' `#[test]` bodies do. The finding's `writers.rs` is `crates/catalog/src/writers.rs`, which is not a test file and has no allowance either way, so that half of the comparison is stale.

rehearsal.rs:12 sets a file-level expect for `unwrap_used` with a reason, while writers.rs:1 relies on clippy.toml allow-unwrap-in-tests. The two test files use different ways to allow the same lint.

### ML-030 [low] Module declarations out of order in desktop lib.rs — CONFIRMED

**Disposition: No change.** The mod order is alphabetical with each bridge after its module.

desktop lib.rs:95-101 lists mod declarations that are not in order: worker sits before a blank line and yandex and `yandex_bridge` come after it. The module list is not kept sorted. Low severity.

### ML-031 [low] Feature combinations missing from the clippy gate — PARTIAL

**Disposition: Decided and applied.** `lint.py` gains clippy steps for `dhat-heap` (pg-gallery) and `specta` (pg-model). A new guard, `tools/check_feature_coverage.py`, fails `just lint` when a feature has no lint step, dependency, or allowed reason.

dhat-heap is a feature of crates/gallery (Cargo.toml:28-29), off by default, so the clippy gate with default features does not compile it. The specta dependency is at the root (Cargo.toml:42), and whether a model feature gates it was not checked here.

### ML-032 [low] Crates listed under both dependencies and dev-dependencies in library — PARTIAL

**Disposition: No change.** It is a Cargo pattern for enabling a feature in tests.

library Cargo.toml lists pg-catalog and pg-writeback, and pg-borders, under both dependencies and dev-dependencies (with fixture and fault features). Cargo allows this, and the dev-dependency entry is how a test enables a feature. The duplication is a Cargo pattern, not an error.

### ML-033 [low] Gate content documented differently from lint.py — CONFIRMED

**Disposition: Fixed.** The sentence the finding cites is no longer in `docs/reference/NOTIFICATIONS_DESIGN.md`: its gate note now says `tools/boundaries.py` runs in `just lint`, and nothing there says `just lint` runs cargo deny. `tools/lint.py` STEPS has no deny step, and the `Justfile` `deny` recipe says it is outside `lint`, so the design doc now agrees with both. No file was changed for this finding in this pass.

`NOTIFICATIONS_DESIGN.md:367` says just lint runs cargo deny. lint.py's STEPS (lines 18-30) have no deny step, and the Justfile at :134 says the deny run is not in lint. The design doc's description of the gate is wrong.

## tooling

### TL-001 [high] Probe scripts resolve their output root outside the checkout — CONFIRMED

**Disposition: Fixed.** The climb from `docs/probes` was one level too far in every probe that defaults to a `target` folder: `wsl_probe.sh` and `wsl_probe2.sh` now use `../..` for `ROOT`, and `v1_symlink.ps1`, `v1b_junction.ps1`, `win_probe.ps1` and `win_probe2.ps1` use `..\..` for their defaults. `win_probe2.ps1` also had its `ScanRoots` default at `..\..\..\..` (the drive root), which now is `..\..\..` to match its own comment ("the folder that holds the checkout"). The fix is a path change only, so each script's documented default now sits in the checkout as its comment says.

`wsl_probe.sh:10` computes ROOT from ../../.. relative to docs/probes, which is the parent of the checkout (`D:\GitHub`), so the default probe folder is outside the repo. `wsl_probe.sh:3` says the folder is in the checkout, so the comment is wrong. `v1_symlink.ps1:3` has the same climb to `D:\GitHub\target`.

### TL-002 [high] Docs disagree on what just lint runs — CONFIRMED

**Disposition: Fixed.** The docs already agree with the code: `tools/lint.py` has no deny step, `LANGUAGES.md:129` and `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:232` both say `cargo deny` is its own `just deny` recipe that CI runs as a separate step, which matches the Justfile (the network-fetching deny check is kept out of `lint` on purpose). In `LANGUAGES.md:129` I corrected the stale `lint_qml` cite from `Justfile:116` to `Justfile:223`, the line where that recipe is now defined.

`NIGHTLY_AND_BUILD_FLAGS_PLAN.md:232` and LANGUAGES.md:129 both say just lint runs cargo deny. tools/lint.py has no deny step (a grep for deny finds nothing), and the Justfile deny recipe is outside lint. The docs state a step that does not run.

### TL-003 [high] WSL and container policy stated inconsistently across docs — PARTIAL

**Disposition: Fixed.** `PLAN.md` says Linux runs in the `pg-dev`, `pg-tools` and `pg-eval` containers.

PLAN.md:19 says WSL is not used and Linux runs in pg-dev. IO.md:91 describes an ExifTool probe run in WSL and `TIER2_ANN_BAKEOFF_PLAN.md:102` says CPU indexes ran in WSL until 2026-11-11 and then moved, so both are dated history. The current policy is consistent. The TIER2 doc names pg-eval where PLAN names pg-dev, which is a real naming difference but both are containers, not WSL.

### TL-004 [medium] Python interpreter command differs by call site — CONFIRMED

**Disposition: Fixed.** `tools/pgtools.py` gains `default_python()`, which returns `python` on Windows and `python3` elsewhere, the same rule as the Justfile's `py` default. `tools/compare_shots.py` now uses it for the venv bootstrap fallback in `main` (it had a literal `"python"` on every OS), and its docstring and `--python` help name the same default. Behaviour change: on Linux with no `PG_PYTHON` or `--python`, the bootstrap now uses `python3` instead of `python`. The CI call sites (`ci.yml` per-OS `python` and `python3`) already follow the runner and are left as they are.

ci.yml:77 runs python on Windows and ci.yml:238 runs python3 on Linux. The Justfile py default (:21) picks python or python3 by OS, but `compare_shots.py:675` falls back to a literal 'python' whatever the OS.

### TL-005 [medium] Cargo invocations in Justfile disagree on --locked — CONFIRMED

**Disposition: Fixed.** The Justfile already passes `--locked` to every cargo build, run, test and install it runs: `build` (`Justfile:50`), `run` (`:63`), `dev` (`:67`), `test` (`:72`) and `cargo install --locked cargo-modules` (`:151`). The two remaining cargo calls take no such flag (`cargo fmt --all` at `:80` and `cargo deny check` at `:139`), so no edit was needed. The audit's line numbers for `build`, `run` and `dev` had drifted; the content it reports as missing is now present.

Justfile build (:50), run (:63), and dev (:67) call cargo without --locked. test (:72) passes --locked, and modgraph's cargo install (:149) passes --locked. The cite lines are off by two or three, not the claim.

### TL-006 [medium] Runner image for Pages deploy floats — CONFIRMED

**Disposition: Fixed.** `.github/workflows/pages.yml` now sets `runs-on: ubuntu-26.04` on the `deploy` job, the same pinned image `ci.yml` and `fork-updates.yml` use, instead of `ubuntu-latest`. Behaviour change: the Pages build runs on the 26.04 image, which is the image the rest of the CI uses, and it no longer moves when GitHub changes what `latest` means. The pin the finding cites is now at `ci.yml:240`.

pages.yml:30 uses runs-on: ubuntu-latest, while ci.yml:224 pins ubuntu-26.04. The cite is ci.yml:224, not :223.

### TL-007 [medium] eval/jpeg-decoders is a separate workspace that no CI step builds — CONFIRMED

**Disposition: Deferred.** A CI job must build eval/jpeg-decoders with the eval packages installed.

eval/jpeg-decoders/Cargo.toml:9 declares its own [workspace]. lint.py:22-24 runs cargo metadata on that manifest, but no CI or Justfile step builds the crate, which matches `REVIEW_LEDGER.md:89`.

### TL-008 [medium] Callgraph pin build and profile logic duplicated between Python and shell/PowerShell — CONFIRMED

**Disposition: Fixed.** callgraph/build.py now runs the Pin 4.3 kit's extensionless wrappers under the MSYS2 bash of `DEFAULT_MSYS`; `profile.py` ran against the debug app with a scratch catalog on 2026-11-21 (exit 0), and `profile.ps1` is removed.

The MSVC compile and link flags are written out in build.py (:86 onwards) and again in build.sh (:42-54), and build.sh is the file build.py says was replaced (DOC-059). The knob list (-o, -stop, -start) is built separately in profile.py:61 and profile.ps1:32. Duplicated in Python, shell and PowerShell.

### TL-009 [medium] Build parallelism differs between Linux and Windows eval builds — CONFIRMED

**Disposition: Fixed.** The Windows builds now use the Linux rule, one job per CPU. `eval/jpeg-decoders/win_build.py` has a module constant `JOBS = str(os.cpu_count() or 1)`, the same expression as `linux_build.py`, and all five `-j` uses take it. `win-build.cmd` (the retained original, see `tools/README.md`) uses `%NUMBER_OF_PROCESSORS%` in its five `-j` calls. Behaviour change: the fixed `-j 4` is gone, so the Windows CUDA compiles (nvcc in jpeggpu, GPUJPEG, parajpeg and the harness) can now run more jobs at once. No doc records why 4 was chosen, so a Windows run should confirm the memory use is acceptable. The same rule now covers `eval/png-decoders/win_build.py`, which had the same fixed `-j 4` on its harness build (its `JOBS` constant).

`linux_build.py:21` uses JOBS = `os.cpu_count(`). `win_build.py` has -j 4 five times (lines 59 and others) and win-build.cmd has -j 4 five times. The Windows build parallelism is hard-coded and the Linux build is not.

### TL-010 [medium] Clone-and-patch helper copied three times in eval build scripts — CONFIRMED

**Disposition: No change.** The Python clone is one function; the .cmd copy goes with its replacement.

A clone-and-patch helper is written three times: `linux_build.py:44` clone, `win_build.py:31` clone, and win-build.cmd:20 :clone. Each applies the patch and keeps an existing folder, in its own language.

### TL-011 [medium] Executable and interpreter path handling repeated per OS — CONFIRMED

**Disposition: No change.** exe_name and VENV_PYTHON are shared.

accel.py:38 chooses the venv interpreter path by os.name, and `gpu_encoders.py:7` documents Scripts versus bin per OS. encoders.py:35 uses EVAL/'bin' with no suffix, `bench_thumbs.py:55` and imgload.py:45 each pick .exe or '' by os.name. The per-OS suffix logic is repeated across tools.

### TL-012 [medium] Env file loading implemented differently in PowerShell and Python tools — CONFIRMED

**Disposition: Fixed.** qmltest.ps1 is removed; qmltest.py ran green here: 143 passed.

qmltest.ps1:13-15 parses `..\.env` with its own regex and sets unset variables. qmltest.py:115 calls `pgtools.load_env_file`. eval/README.md:12 says PowerShell does not read .env, which the ps1 contradicts. `build_jpeg_turbo.py:85-87` runs its platform guard before `load_env_file`.

### TL-013 [medium] Text files read and written without an encoding — CONFIRMED

**Disposition: Fixed.** The three calls in `tools/perf-compare.py` now pass `encoding="utf-8"`: the callgrind read in `ir_total` (kept `errors="replace"`), the baseline `write_text` and the baseline `read_text`. This matches the `encoding="utf-8"` already used in the same file (line 40) and by `tools/lint_qml.py`. For ASCII content nothing changes; for the rest, the files are now read and written as UTF-8 whatever the locale is.

perf-compare.py:61 and :109 call `read_text` with no encoding, and :102 calls `write_text` with no encoding, while `lint_qml.py:37` passes encoding='utf-8'.

### TL-014 [medium] CPU load measured by two different WMI queries — CONFIRMED

**Disposition: Fixed.** The new `tools/profiling/pairing/cpu_load.ps1` defines `Get-CpuLoad`, the `_Total` `Win32_PerfFormattedData_PerfOS_Processor` `PercentProcessorTime` query that `sweep.ps1` used (its `Get-Load` is removed). `sweep.ps1`, `few_sweep.ps1` and `keep.ps1` dot-source the file and call `Get-CpuLoad`, so the three sweeps read the same counter. Behaviour change: `few_sweep.ps1` and `keep.ps1` now report the performance counter instead of the average of `Win32_Processor.LoadPercentage`, which is a different sampling method; the perf counter is the one that names total processor time. The Get-CimInstance result is unverified here (WMI), so a Windows run of a sweep should confirm the cpu values look sane.

sweep.ps1:29 reads `Win32_PerfFormattedData_PerfOS_Processor` PercentProcessorTime. keep.ps1:97 and `few_sweep.ps1:50` read `Win32_Processor` LoadPercentage. Two WMI classes measure CPU load.

### TL-015 [medium] Probe scripts capture stderr differently — CONFIRMED

**Disposition: Fixed.** All four pairing scripts now capture the probe's stdout alone and leave its stderr on the console: `sweep.ps1` line 52 drops its `2>&1`, to match `few_sweep.ps1`, `oracle.ps1` and `keep.ps1`. The probe (`crates/library/examples/pairs_probe.rs`) writes its result line to stdout, so the parsing is unchanged. Behaviour change: a probe's stderr text (a panic or a warning) no longer appears in the sweep's `line` field or its printed row; it still shows on the console, where a failing run is seen first.

sweep.ps1:52 merges stderr with `2>&1`. `few_sweep.ps1:44` and oracle.ps1:74 take stdout only. The probe's output is captured differently across the three scripts.

### TL-016 [medium] Probe binaries disagree on scratch cleanup and duplicate scheduling — PARTIAL

**Disposition: No change.** The rotation claim was not located, and the probes are examples.

Scratch cleanup differs: `pairs_load_probe.rs:108` and `pairs_write_probe.rs:114` call `remove_dir_all`, while `stmt_journal_probe.rs:206` creates its `run_folder` and never removes it. The cites are at crates/catalog/examples/, not src/, which is the real location. The rotation claim ((k + run) % len at :90 and :86) was not located by a search for % len, so it is unverified.

### TL-017 [medium] Screenshot comparison exists as both a PowerShell and a Python tool — PARTIAL

**Disposition: Fixed.** `BOUNDED_MEMORY_PLAN.md` names `tools/compare_shots.py`; there is no PowerShell version.

The premise that a PowerShell tool exists beside the Python one is false: tools/compare-shots.ps1 does not exist, only `tools/compare_shots.py` does. `BOUNDED_MEMORY_PLAN.md:1567` still names the .ps1, so the stale reference is the real defect. The 25-scenario claim at `compare_shots.py:104-129` was not confirmed (:104 is a blank line in the view).

### TL-018 [medium] Eval scripts parse command lines in three styles — PARTIAL

**Disposition: No change.** The third style was not established, and the styles differ by script.

Two styles are seen: encoders.py:238 dispatches on sys.argv[1] through a dict, and eval/ann/run.py:524-525 reads stage names from sys.argv. linux.py does not use argparse (no match), so the cited comparison does not hold for linux.py. Some eval scripts use argparse; the claimed third style was not established.

### TL-019 [medium] Stale reference to `make_bench_jpeg.py` in bench source and docs — REFUTED

`tools/profiling/make_bench_jpeg.py` exists, and linux.rs:21 and tools/README.md:33 name it correctly. `DECODE_ONCE_PLAN.md:1221` introduces `make_strata.py` as a new tool and does not say it replaces `make_bench_jpeg`; the plan at :1163 extends `make_bench_jpeg`, which is consistent. No stale reference was found.

### TL-020 [low] Bench run command documented two ways — REFUTED

ir.rs:3 says the benches run with just perf in the Linux container. `DECODE_ONCE_PLAN.md:1165` says cargo bench -p pg-bench --bench ir through bench.py:95. Both are true: just perf runs bench.py, which runs cargo bench. Consistent, not two conflicting run methods.

### TL-021 [low] Probe shell and PowerShell strictness differs — CONFIRMED

**Disposition: Fixed.** `docs/probes/win_probe2.ps1` set `ErrorActionPreference` to `Continue` in the middle of the file, and now sets `'Stop'` once, after its `param` block, like `v1_symlink.ps1`, `v1b_junction.ps1`, `v2_volumes.ps1` and `win_probe.ps1`. Its only failure paths are .NET exceptions, which are terminating either way, and the UNC call is already in a `try`, so no output changes on a clean run. `docs/probes/v34_wsl.sh` gained `set -u` after its header, as the other WSL probes have. Behaviour change: a non-terminating cmdlet error in `win_probe2.ps1` now stops the script, where before it continued; the script has none that it relies on.

`win_probe2.ps1:111` sets ErrorActionPreference to Continue, while `v1_symlink.ps1`, `v1b_junction.ps1`, `v2_volumes.ps1` and `win_probe.ps1` use Stop. `v34_wsl.sh` has no set -u (grep count 0), while `v4_drvfs_superblock.sh:4`, `wsl_probe.sh:5` and `wsl_probe2.sh:5` use set -u.

### TL-022 [low] Benchmark harness error handling mixes check and returncode — CONFIRMED

**Disposition: Fixed.** `tools/profiling/bench.py` now uses one style throughout: `subprocess.run(..., check=False)` and an explicit returncode test that ends the program with a message. The two `tar` calls that used `check=True` (the archive write around `packed`, and the listing `count`) are converted, and the write's message names the archive. Behaviour change: a failing `tar` now exits with a one-line message and the status code, where it raised `CalledProcessError` with a traceback; the `finally` that removes the listing file still runs first.

bench.py:115 and :122 use subprocess.run with check=True, while bench.py:98 uses check=False and tests returncode itself. Two error styles in one file.

### TL-023 [low] Exit-code convention documented only in a plan — CONFIRMED

**Disposition: No change.** The exit convention is written in tools/README.md.

The 0/1/2/3 exit-code convention is written in `DECODE_ONCE_PLAN.md:1219`. The tools use sys.exit with ad hoc messages (`compare_shots.py:177-283` and others) and no shared convention is in code, so the plan is the only place the convention is written down.

### TL-024 [low] Desktop-only flags tested in the CLI detector — REFUTED

cli lib.rs:745-746 asserts wants(...) is true for the CLI's own subcommands and help. The second loop at :747-749 asserts !wants for --screenshot, --view and --viewer-keys. The test checks that the CLI does not claim desktop flags, which is the intended behaviour.

### TL-025 [low] CI leaves a debug variable and sets a tool-missing flag — PARTIAL

**Disposition: Decided and applied.** The temporary `CARGO_LOG` line is removed from `ci.yml`. `PG_REQUIRE_TOOLS` is intended and stays.

ci.yml:59-60 sets `CARGO_LOG` with the comment 'Diagnostic (temporary)', so the debug variable is left in CI with a temporary label. `PG_REQUIRE_TOOLS` at ci.yml:51 is an intended setting (its comment says so), not a leftover.

### TL-026 [low] Tool-availability probes differ across library tests — CONFIRMED

**Disposition: Fixed.** One probe now serves the library tests: `pg_platform::testing::tool_missing(exe, version_arg)` (re-exported by `pg_testkit`) returns true when the program does not start or its version flag does not exit 0, next to `missing_tool`, which it is designed to pair with. `crates/library/tests/crop.rs` (the inline check at `:87` and `exiftool_missing`), `crates/library/tests/writeback.rs` (`exiftool_missing`) and `crates/library/tests/video.rs` (`ffmpeg()`, which keeps its path search) call it. Behaviour change: an ExifTool or ffmpeg that starts but exits non-zero on its version flag now counts as missing, where before only a spawn failure did (video already checked the status). The direct `exiftool` calls that do the work are unchanged.

library crop.rs:87 probes exiftool with -ver and then calls `pg_testkit::missing_tool`. video.rs:38 probes ffmpeg with -version through a filter, and writeback.rs:136 probes exiftool inline without `missing_tool`. Three probe patterns.

### TL-027 [low] Code cites PROFILING.md by line number — REFUTED

The crate code cites PROFILING.md by section heading (join.rs:84 names 'The scan kernel'), not by line. A grep for PROFILING.md with a line number in crates finds nothing.

### TL-028 [low] Docs name eval scripts without paths — CONFIRMED

**Disposition: Fixed.** The cited mentions now carry their path: `docs/eval/MODELS.md:22` names `eval/evaluate.py`, and `docs/eval/VERIFIER.md:22` names `eval/verify.py --rescore`, which is the first mention in that doc before the existing `eval/verify.py` at line 35. Other bare mentions later in those docs were left alone, since they read the same as the path-qualified ones. This is a documentation-only change with no behaviour effect.

MODELS.md:22 names evaluate.py without a path, while VERIFIER.md:35 names eval/verify.py. The eval scripts are named without a consistent path convention across docs.

### TL-029 [low] Desktop C++ include path style differs — CONFIRMED

**Disposition: No change.** The bridge-path exception stays; the other headers use bare names.

deadline.cpp:5 and screenshot.cpp:13 include "`pg-desktop/cpp/<name>.h"`. `frame_sink.cpp:1` includes "`frame_sink.h"` bare, and screenshot.cpp:29 and providers.cpp:22 include generated bridge headers by the pg-desktop/src path. Three include styles in one C++ tree.

### TL-030 [low] Bench data regeneration names the old backup and the generator — CONFIRMED

**Disposition: Fixed.** The plan's text is already current: `docs/plans/CATALOG_SIZE_PLAN.md:312` names the constant `BENCH_LAYOUT` at `crates/bench/benches/ir/catalog.rs:36`, with its value `"v1-windows-rules"`, and no `v10-windows-rules` remains in the docs or code (checked by grep). The regeneration step names `tools/profiling/bench_data.py`, which exists, so the instruction matches the code. No edit was needed in this pass; the `LAYOUT` and `v10` wording the finding quotes is gone.

`CATALOG_SIZE_PLAN.md:312` says bump LAYOUT = "v10-windows-rules" at catalog.rs:43. The bench code has `BENCH_LAYOUT` = "v1-windows-rules" at catalog.rs:36. The constant name, the line and the version all differ from the plan's instruction, so the plan is stale.

### TL-031 [low] Version lint script referenced from the versions registry — PARTIAL

**Disposition: No change.** `test_versions.py` does enforce that each owner reads its constants.

`tools/test_versions.py` exists and does the regex scan of `*VERSION` constants described in its docstring (lines 1-4 and 16-17). The versions.rs:7 claim that it fails on a constant no owner reads was not confirmed in the parts read, so only the outside-module check is verified.

## pins

### P-001 [high] FFmpeg BtbN build tag in ci.yml header comment is stale — REFUTED

ci.yml:11 names no BtbN tag; the comment only says the FFmpeg pin lives in the Justfile. The only tag is Justfile:17 autobuild-2026-10-31-12-59, not the autobuild-2026-11-19-12-39 or -10-30-13-01 values the finding cites. The cited Justfile line is 17, not 15.

### P-002 [high] turbojpeg-sys fork rev disagrees between the build script comment, eval harness and fuzz/root manifests — CONFIRMED

**Disposition: Fixed.** The libjpeg-turbo pin is now the commit the product links: `FORK_COMMIT` in `tools/build_jpeg_turbo.py`, `$ForkCommit` in `tools/build-jpeg-turbo.ps1` and the `clone` in `build_turbo` of `eval/jpeg-decoders/linux_build.py` all changed from 945d853b to 28b8b86a, the rev in root `Cargo.toml:152`. The existing comment that says the pin is the root Cargo.toml commit is now true, and `win_build.py` inherits the fixed value through its import. This is the proper choice because one source should feed the app's and Qt's libjpeg. Behaviour change: the standalone build compiles the 28b8b86a sources; its `build.rs` local flag changes (optimized CMake flags) are not applied by that script, which already sets `WITH_CRT_DLL` itself.

`build_jpeg_turbo.py:32` `FORK_COMMIT` 945d853b, and its comment at :29-30 says it is the commit the root Cargo.toml pins. Root Cargo.toml:148 pins 28b8b86a. fuzz/Cargo.toml:84 and fuzz/Cargo.lock:660 pin 28b8b86a too. `linux_build.py:95` clones 945d853b. `STATIC_BUILD.md:20` says 28b8b86. The comment in `build_jpeg_turbo.py` is false.

### P-003 [high] Probe constants claim to mirror `pg_import` but are copied as literals — CONFIRMED

**Disposition: Fixed.** `pg-catalog` now dev-depends on `pg-import` (`crates/catalog/Cargo.toml`, with the `Cargo.lock` entry added by hand), so `pairs_load_probe.rs` imports `CACHE_KIB` and `pairs_write_probe.rs` imports `CACHE_KIB` and `WRITE` from `pg_import::pairs` instead of copying them. The write probe's `cache_size` is now set from `CACHE_KIB` in its `open`, and `SETTINGS` no longer carries the literal; `open` in the load probe takes a `u32`. The dev-dependency cycle is legal in Cargo (`pg-import` depends on `pg-catalog` normally) and is the proper choice because the probe's constants then cannot drift. Behaviour change: the write probe now writes 4,000 pictures per transaction, as the pass does, not 2,000.

The probes are in crates/catalog/examples/, not src/. `pairs_write_probe.rs:39` has WRITE = `2_000`, documented as mirroring `pg_import::pairs::WRITE`, which is `4_000` at crates/import/src/pairs.rs:39: the mirror is already out of date. `pairs_load_probe.rs:38` `CACHE_KIB` = `16*1024` matches pairs.rs:34. `pairs_write_probe.rs:37` has PRAGMA `cache_size` -16384 as a literal.

### P-004 [medium] parajpeg git rev differs between production manifests and eval harness — CONFIRMED

**Disposition: Deferred.** The eval harness must move to the fork's parajpeg with a CUDA build and a re-measurement. The pin is also in `docs/reference/JPEG_DECODERS.md:489`, and the harness clones github.com/anniigr/parajpeg while the pinned commit is on the darktorres fork, so the URL changes with the SHA. The re-measurement needs the win_build chain (clang-cl, network), which is not run.

Cargo.toml:34 and fuzz/Cargo.lock:353 pin parajpeg 8e2c53a. eval `linux_build.py:158`, `win_build.py:96`, win-build.cmd:50 and `JPEG_DECODERS.md:489` use f45081d. The harness clones upstream anniigr/parajpeg, while the product uses the darktorres fork, so the two revisions differ.

### P-005 [medium] fuzz manifest claims the root patch list is complete, but it is not — CONFIRMED

**Disposition: Fixed.** The comment above `[patch.crates-io]` in `fuzz/Cargo.toml` now says the table is the root's minus the four patches its graph never reaches (`cxx-qt-build`, `qt-build-utils`, `rusqlite`, `thumbhash`), and that parajpeg is a git dependency rather than a patch. I checked `fuzz/Cargo.lock`: none of those four names appears in it, while the nine listed ones do. The proper choice is to correct the claim rather than add the four entries, because adding unused patches only produces cargo's unused-patch warnings. No behaviour change.

fuzz/Cargo.toml:74 claims the root patch table is the full list. The root has 14 entries (Cargo.toml:137-150). fuzz/Cargo.toml has 9 (lines 77-86). parajpeg is git-pinned outside both tables, and fuzz/Cargo.lock:353 has no patch entry for it.

### P-006 [medium] Plan doc cites the old png fork rev and Cargo.toml line — CONFIRMED

**Disposition: Fixed.** The N14 row of `docs/plans/NIGHTLY_AND_BUILD_FLAGS_PLAN.md` now names the pinned rev 2f176ed and the patch line `Cargo.toml:141`, where the root `[patch.crates-io]` pins `png` (checked against the root file). The fork's own line citations (`Cargo.toml:41`, `src/lib.rs:69`) were checked in the 2f176ed checkout and still hold. The f7129d3 mentions in `PNG_DECODERS.md` are deliberate: they label the pre-fix measurements, so they stay. Documentation-only change.

`NIGHTLY_AND_BUILD_FLAGS_PLAN.md:159` says the png fork is at f7129d3 at Cargo.toml:132. Cargo.toml:141 pins 2f176edb. `PNG_DECODERS.md:3` says pinned at 2f176ed. The plan's rev and line are stale.

### P-007 [medium] Workspace declares no rust-version, and API and SQLite floors are undeclared — CONFIRMED

**Disposition: No change.** rust-version is declared as 1.89.

Cargo.toml:1-8 [workspace.package] has no rust-version. The cited features are real: let-chain at crates/client/src/grid.rs:312 (the finding says controller/grid.rs:310, which does not exist), `split_at_checked` at results.rs:63, `is_multiple_of` at text.rs:291, `repeat_n` at words.rs:86, UPDATE..RETURNING at pair.rs:277, unixepoch() at lease.rs:28. No SQLite version check exists (grep for `sqlite_version` finds nothing).

### P-008 [medium] Runner image labels differ between workflows — CONFIRMED

**Disposition: Fixed.** The state the finding describes is already gone from the working tree: `pages.yml:30` is `ubuntu-26.04` (not `ubuntu-latest`), and `ci.yml:240` and `fork-updates.yml:17` are the same label. The only remaining difference is `ci.yml:56`, `windows-2025`, which is a different OS, not a second image for the same job. No edit was needed in this pass.

ci.yml:56 windows-2025 and :224 ubuntu-26.04 (the finding's :223 is the job name line). fork-updates.yml:17 ubuntu-26.04. pages.yml:30 ubuntu-latest, which floats.

### P-009 [medium] gungraun-runner installed without --locked while other cargo installs use it — PARTIAL

**Disposition: No change.** The omission is deliberate and commented in the Dockerfile.

Dockerfile:115 gungraun-runner has no --locked. Dockerfile:113-114 gives the reason in a comment: the published lock pins yoke-derive 0.8.3, which was yanked. :116 and :87-90 use --locked. The omission is deliberate and commented, so inconsistent describes a documented exception.

### P-010 [medium] Rust components and the installer are declared in both `ci_setup.py` and rust-toolchain.toml — CONFIRMED

**Disposition: Deferred.** The rustup-init SHA-256 pin needs network access; the Dockerfile copy is included.

The file is `tools/profiling/ci_setup.py`, not `tools/ci_setup.py`. Line 51 fetches `https://sh.rustup.rs` with urllib and pipes it to sh with no checksum. Line 55 runs rustup component add clippy rustfmt on the default toolchain. rust-toolchain.toml:4-5 pins nightly-2026-11-06 with clippy and rustfmt components.

### P-011 [medium] Static-build tool pins copied between PowerShell and Python — CONFIRMED

**Disposition: Fixed.** The build-*.ps1 originals are removed; the Python recipes hold the pins, and the two tests that compared the PowerShell copies are removed.

The same Build Tools path literal is in `build_jpeg_turbo.py:33`, `build_static_ffmpeg.py:39`, `build_static_qt.py:42`, build-jpeg-turbo.ps1:13 (the finding says :24) and build-static-ffmpeg.ps1:14. `build_static_ffmpeg.py:36` and build-static-ffmpeg.ps1:26 both pin make 4.4.1-3 and diffutils 3.12-1.

### P-012 [medium] ExifTool version in docs and platform code disagrees with the 13.59 pin — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** The pin stays 13.59 until CI moves; the docs say both versions and why.

Justfile pins are at :98 (`exiftool_version` 13.59.0) and :102 (`exiftool_source_version` 13.59). The finding's :96 and :100 are off by two. platform/native.rs:15-16 says checked with ExifTool 13.60. IO.md:84 says 13.60 and :91 says 13.50 under WSL. `EXIFTOOL_PORT_PLAN.md:10` names 13.60 as the port's reference. The docs' reference version differs from the pinned 13.59.

### P-013 [medium] Bundled SQLite version stated three different ways — CONFIRMED

**Disposition: Fixed.** The bundled build is SQLite 3.54.1: the rusqlite fork's `libsqlite3-sys` 0.40.0 (the version `Cargo.lock` pins) has `SQLITE_VERSION "3.54.1"` in its `sqlite3.h`. The bare "3.54" in `UROCISSA_COMPARISON.md:174` and `docs/perf/budgets.json` (the `import_flush` note) now reads 3.54.1, matching `DATABASE.md` and `DECODE_ONCE_PLAN.md`. `CATALOG_SIZE_PLAN.md:357` named `libsqlite3-sys` 0.39.1, which is wrong for this build, so it now says 0.40.0. The finding's 3.54.0 in `DATABASE.md` is no longer there: that file already says 3.54.1. Documentation and a JSON note only; no behaviour change.

DATABASE.md:101 says the app ships 3.54.0. `UROCISSA_COMPARISON.md:174` says 3.54. budgets.json:14 says 3.54. `CATALOG_SIZE_PLAN.md:357` says 3.54.1 with libsqlite3-sys 0.39.1. `DECODE_ONCE_PLAN.md:852` says 3.54.1. Three different precisions for the same bundled build.

### P-014 [medium] Eval decoder build scripts repeat commit pins instead of sharing one list — CONFIRMED

**Disposition: Fixed.** win-build.cmd and win-env.cmd are removed; win_build.py holds the build and sources.py the pins.

`win_build.py:64`, :79, :117 and `linux_build.py:158` pin jpeggpu 3ae7ac0d, GPUJPEG 37ab4810, GPU-JPEG-Decoder 61cfb372 and parajpeg f45081d. win-build.cmd:36, :42, :50, :59 carry the same commits as literals. win-build.cmd:30 builds libjpeg-turbo from vendor/ with no commit pin, while `win_build.py:48` clones `FORK_COMMIT`. The pins are repeated and one is missing.

### P-015 [medium] Python 3.12.10 literal in ci.yml, Justfile and `STATIC_BUILD.md` — CONFIRMED

**Disposition: Fixed.** The `ci.yml` literal is needed (setup-python runs before `just` exists to read the Justfile), and it is already guarded: `tools/test_ci_files.py` `test_the_workflow_python_is_the_justfile_pin` asserts it equals the Justfile's `python_version`. The `STATIC_BUILD.md` mention is a record of the tools the pilot used, not a pin, and the Justfile remains the one place the version is set. No edit was needed in this pass; the test is the check, and it was not run here.

ci.yml:72 hard-codes python-version 3.12.10. Justfile:96 `python_version` is 3.12.10 (the finding's :94 is its comment). `STATIC_BUILD.md:27` names 3.12.10. The literal in ci.yml is deliberate: setup-python runs before just is installed, and the Justfile comment says the tools test asserts equality. The duplication is documented and tested.

### P-016 [medium] MSVC toolset literal duplicated in PowerShell scripts and `STATIC_BUILD.md` — CONFIRMED

**Disposition: Fixed.** The Python recipes read `MSVC_TOOLSET` from `tools/pgtools.py`, and the PowerShell copies are already checked against it by `test_the_powershell_copies_agree_on_the_msvc_toolset` in `tools/test_ci_files.py`. `STATIC_BUILD.md` names the value in its pin table, which is that document's job, so no edit was needed. The PowerShell originals themselves are a separate open decision (see P-011). Not run here.

build-jpeg-turbo.ps1:24 and build-static-ffmpeg.ps1:24 hard-code Toolset 14.44.35207. `STATIC_BUILD.md:22` names it again. The Python recipes read `MSVC_TOOLSET` from pgtools.

### P-017 [low] Measurement records cite GPU and image library versions that differ from the current pins — PARTIAL

**Disposition: No change.** The docs state measurement versions by design.

Accurate for nvImageCodec (`JPEG_ENCODERS.md:53` measured 0.9.0.20; pin 0.10.0.9), nvJPEG (`JPEG_DECODERS.md:492` measured 13.2; requirements.txt:8 pins 13.3.2.43) and Pillow 12.2.0 (`PNG_DECODERS.md:47`). Not accurate for Pillow 13 (`JPEG_DECODERS.md:51`), which matches requirements.txt:11 pillow 13.0.0, or for libvips (no pin exists). The docs state measurement versions by design.

### P-018 [low] FFmpeg and mpv versions in docs disagree with the shipped FFmpeg 9.0.2 — PARTIAL

**Disposition: No change.** No mpv version is pinned, and the docs label their own FFmpeg builds.

mpv 0.41 does not appear in `VIDEO_PLAYER.md` (the doc says 0.42.0 at :261 and :289). FFmpeg: PROFILING.md:126 labels its Linux build as FFmpeg 8 (Ubuntu) and says the Windows app ships avcodec-63, so that is a stated separate build. `JPEG_DECODERS.md:54` names FFmpeg 8 mjpeg in a decoder table without a build label. No mpv version is pinned anywhere.

### P-019 [low] Eval torch CUDA tag in MODELS.md is stale against requirements — PARTIAL

**Disposition: No change.** Only the dated measurement uses the older tag; the rest matches the pin.

requirements.txt:65 pins torch 2.15.0+cu134. `TEST_SET.md:13` says CUDA 13.4 (matches). MODELS.md:190 says cu132 then cu134, which measures the same. MODELS.md:152 says cu132 for the driver-621.01 measurement, a dated record. Only the dated measurement uses cu132; the rest agrees with the pin.

### P-020 [low] TensorRT version in eval README and accel.py differs from requirements pin — REFUTED

requirements.txt:60-62 pins TensorRT 11.4.0.106. accel.py:40 defaults `TRT10_DIR` to tools/tensorrt-10.16.1.11-cu13. eval/README.md:86-87 documents that folder as TensorRT 10 DLLs for ONNX Runtime's TensorRT provider, which is built against TensorRT 10. The difference is deliberate and documented.

### P-021 [low] Justfile comment says bump cargo-modules in two places, but CI reads it from the Justfile — CONFIRMED

**Disposition: Fixed.** The comment above `cargo_modules_version` in `Justfile` now says the version is read from the Justfile by CI (`tools/ci/github_files.py`, the `TAIKI_TOOLS` value) and by the container (`tools/linux.py` build args), so a bump is one line. I grepped `.github/workflows/ci.yml` and found no cargo-modules literal to keep in step. Comment only; no behaviour change.

Justfile:142 says bump cargo-modules in ci.yml and the container reads it from here. `tools/ci/github_files.py:81` maps cargo-modules to `cargo_modules_version` and reads it from the Justfile through `just_value`. So CI no longer needs a separate bump, and the bump-both instruction is stale. The cite Justfile:140 is a comment line; :143 is the 0.28.0 pin.

### P-022 [low] Two Windows binding crates in one platform crate, and a stale windows-sys line reference in a plan — PARTIAL

**Disposition: No change.** Both binding crates are real; the plan cite is wrong.

Both binding crates are in platform: picker.rs:129 uses the windows crate, dir.rs:122 and drive.rs:240 use `windows_sys`. That part holds. `DECODE_ONCE_PLAN.md:299` does not mention windows-sys at all, so the plan-reference instance is wrong. Cargo.toml:54 windows-sys 0.61 is at line 54, as the finding says.

### P-023 [low] GitHub Action pin comments use different version granularity — CONFIRMED

**Disposition: Deferred.** The release behind each SHA needs a GitHub lookup.

ci.yml:66 comment # v7, :70 # v7.1.0, :82 # v2; pages.yml:43 # v6.0.0 and :48 # v5.0.1. The comment granularity varies as stated.

### P-024 [low] Apt packages are unversioned in three places, against a digest-pinned base image — CONFIRMED

**Disposition: Deferred.** An apt snapshot date or exact versions is a reproducibility decision that needs network to choose.

Dockerfile:7 pins the base image by digest. bench.py:81-82 says apt-packages.txt is not pinned to a snapshot. video/build.rs:52-53 lists `libav*-dev` with no versions. `tools/profiling/ci_setup.py` (the finding's `tools/ci_setup.py:41`) appends mold unversioned.

### P-025 [low] GPU compute capability hard-coded in one build script and detected in others — CONFIRMED

**Disposition: Fixed.** `eval/jpeg-encoder/build.ps1` no longer defaults `-Arch` to `sm_75`: unless `-Arch` is passed, it takes `CUDA_ARCH` or the first GPU's `compute_cap` from nvidia-smi, the same order as `win_env.py` and `win-env.cmd`, and throws if neither gives a value. The finding's `eval/jpeg-decoders/build.ps1` does not exist; the script is `eval/jpeg-encoder/build.ps1`, and its README now says the arch follows the GPU. Behaviour change: on a machine with no `CUDA_ARCH` and no nvidia-smi, the script now stops instead of building for sm_75. The PowerShell check is not run here; a future Windows run should look at the `nvidia-smi` parse (`Select-Object -First 1`, then the dot removed).

eval/jpeg-encoder/build.ps1:11 hard-codes Arch = `sm_75` (the finding's eval/jpeg-decoders/build.ps1 does not exist). `win_env.py:74-75` and win-env.cmd:29-30 detect the compute capability from nvidia-smi unless `CUDA_ARCH` is set.

### P-026 [low] Bench data default falls back to a container path — CONFIRMED

**Disposition: Fixed.** The bench data fallback is /bench/data.

catalog.rs:28-31 falls back to /root/pg-bench when `PG_BENCH_DATA` is unset (the finding's :30 is the None arm, correct).

### P-027 [low] Yandex User-Agent pinned to Chrome 108, a browser build from 2022 — CONFIRMED

**Disposition: Deferred.** The User-Agent bump needs a live Yandex check.

yandex/src/http.rs:11-14 sets Chrome/108.0.0.0 and says it is the same User-Agent as an external crawler. The cited line and the stated reason both match.

### P-028 [low] Qt 6.12 cited as a version in code comments, not only READMEs — PARTIAL

**Disposition: No change.** Two of the three cited instances name the version; the third is a Qt source file name.

Qt 6.12 is named in platform/audio.rs:1 and desktop/qml/GridCell.qml:77 as the finding says. `gpu_device.cpp:4` cites Qt's source file qrhid3d11.cpp, not a Qt version. Two of the three instances match the claim.

### P-029 [low] Key version tied to toolchain Unicode version — PARTIAL

**Disposition: No change.** The dependency is already documented in the function's doc comment (`types.rs:268-270`).

types.rs:272 combines `KEY_ALGO_VERSION` with `char::UNICODE_VERSION`, as stated. The toolchain dependency is already documented in the function's doc comment at types.rs:268-270 (recomputed on another toolchain). The key-version doc the finding asks for does not exist as a separate doc.

### P-030 [low] SHA-256 comparison uses two different case conventions — PARTIAL

**Disposition: No change.** `setup-ffmpeg.ps1` is not at HEAD, and the Python side compares case-insensitively.

tools/setup-ffmpeg.ps1 is deleted in the working tree (uncommitted) and present at HEAD. The Python side also compares case-insensitively: `setup_ffmpeg.py:96` lowercases the pin and :109 compares `sha256_of(zip`). The cited line 82 is `top_folder`, not the SHA comparison. Both sides are case-insensitive, so the two-conventions claim describes style, not behaviour.

## ui-text

### UI-001 [high] MB labelled where the value is MiB — CONFIRMED

**Disposition: Fixed.** The header in `crates/import/examples/pairs_write_probe.rs` (moved from `catalog/examples` so catalog no longer needs a dev-dependency on import, which made a cycle) (`main`, the `writeln!` at the table header) now says "WAL after (MiB)", matching the `>> 10` then `/ 1024.0` computation in the row loop, which yields MiB. The label was the defect; the computation is correct. The `lifecycle.rs` decimal-MB cite stays as the audit found it, since its divisor is decimal, so nothing changes there.

`pairs_write_probe.rs:103` labels the column 'WAL after (MB)', and :110 computes the value as bytes `>>` 10 divided by 1024.0, which is MiB. The label is wrong. The lifecycle.rs:270 cite is decimal MB with a decimal divisor (`1_000_000`), so it is consistent and not a defect.

### UI-002 [high] Rating symbols defined in Rust and again in the QML test stub — CONFIRMED

**Disposition: Fixed.** The stub cannot call Rust, so its literal list in `crates/desktop/tests/qml/stubs/pg/gallery/Words.qml` stays, and a new test `the_stub_rating_symbols_are_the_rust_ones` in `crates/desktop/src/words.rs` reads that literal and asserts it equals `pg_client::text::RATING_SYMBOLS`. Drift between the two lists now fails `cargo test` instead of passing silently. The Rust list stays the single source of truth; the clamp to 0..5 already matched on both sides and is unchanged.

client text.rs:30 `RATING_SYMBOLS` and the QML test stub Words.qml:8 hold the same six symbols, with the same clamp to 0..5 (stub :10). The list is duplicated across Rust and a test stub, so it can drift.

### UI-003 [medium] Same id-less word under two Fluent ids in ui.ftl — CONFIRMED

**Disposition: Fixed.** ui-inbox-show-files covers both uses; ui-failed-link is removed.

ui.ftl has 'Loading…' at :110 (ui-inbox-loading) and :166 (ui-loading). 'Log' is at :116 (ui-log) and :117 (ui-log-title), and 'Inbox' at :94 and :97 (ui-inbox-title). The same words are under two ids with no shared key.

### UI-004 [medium] Writer stop reported with different words in GUI and CLI — CONFIRMED

**Disposition: Fixed.** The CLI's `cli-writer-stopped` and `cli-writer-stopped-failing` in `crates/client/locales/en/cli.ftl` now say "writing into the files has stopped", the same words as the GUI title `notice-write-stopped-title` in `text.ftl`, and "file writer" is gone from user text. The one test that asserted the old wording, in `crates/cli/src/lib.rs`, is updated to match. The GUI wording was kept because it is the one users read in the app; the CLI line is lowercase as before, which UI-025 covers separately. Behaviour is unchanged apart from the words.

The GUI notice says 'Writing into the files has stopped' (text.ftl:261) and the CLI says 'the file writer stopped with N changes still to write' (cli.ftl:70). The same event is worded differently in the two surfaces and with different terms (writing into the files vs file writer).

### UI-005 [medium] Kept-change wording differs across three Fluent files — CONFIRMED

**Disposition: Fixed.** All kept-change strings now use the phrase "waiting to be written" that `ui-removal-kept` already had. `notice-folder-removal-unfinished-title` in `crates/client/locales/en/text.ftl` reads "is waiting to be written" (was "was kept"); `cli-status-removing` and `cli-removal-kept` in `cli.ftl` read "N changes kept, waiting to be written" (was "rows kept for an unfinished write"), which also drops the database word "rows" from user text. No Rust or QML test asserts the old strings, so nothing else changes; the message ids and variables are unchanged.

Kept-change wording: text.ftl:282 'A change to a file in {folder} was kept', ui.ftl:71 'kept: a change is waiting to be written', cli.ftl:61 'removal not finished, rows kept'. Three phrasings of one state.

### UI-006 [medium] Failure notice levels follow no stated rule — CONFIRMED

**Disposition: Fixed.** The rule is now stated in the doc comment of `Notice::level()` in `crates/model/src/notice.rs` and in a bullet of `docs/reference/NOTIFICATIONS_DESIGN.md`: Error when the user's change or action did not take effect or a window-needed read failed, Warning when part of a batch failed or the app runs in a reduced way. Four variants moved to match it. `CacheDisabled` moved from Error to Warning (the app decodes thumbnails each time, so it runs reduced). `WriteDropped`, `FolderRefused`, `PickerFailed` and `FoldersUnreadable` moved from Warning to Error (a lost edit, a folder not added, a dialog that failed to add, and a folder list the window needs). Behaviour change: those four show as errors and the cache-disabled notice shows as a warning in the inbox, toast and log; no test asserts these levels. The cited `WriteFailed`, `CacheFull` and `ThumbsFailed` levels already fit the rule and are unchanged.

notice.rs:1049 gives Cropped and BordersCut Warning when failed `>` 0 and Info otherwise. ThumbsFailed, BordersFailed and ImportFailed are Warning (:1073). WriteFailed is Error (:1087), CacheDisabled is Error (:1086) while CacheFull is Warning (:1060). `NOTIFICATIONS_DESIGN.md:208-209` says warnings and errors stay until dismissed but gives no rule for which variant gets which level.

### UI-007 [medium] Status word for Failed differs between filter chip and status — PARTIAL

**Disposition: No change.** The chip and the status share text.rs:74 and :98, so they cannot differ.

The test at text.rs:821-825 expects the Failed status to read 'Unreadable', which matches text.ftl:14 (status-failed = Unreadable). The filter chip's label was not located, so whether the chip and the status word actually differ is unverified.

### UI-008 [medium] Dialog buttons come from two label sources — CONFIRMED

**Disposition: Fixed.** `SaveViewDialog.qml` drops `standardButtons: Dialog.Save | Dialog.Cancel` for a `footer: DialogButtonBox` with two Buttons, Save (`AcceptRole`) and Cancel (`RejectRole`), labelled by `words.text("ui-save")` and `words.text("ui-cancel")`, the same pattern `CutBordersDialog.qml` uses. The new `ui-save = Save` entry is in `crates/client/locales/en/ui.ftl` next to `ui-cancel`. The dialog's `accepted` and `rejected` handling is unchanged, and the Words test sees the new id as used. `docs/plans/LANGUAGES.md` section 3.7 is updated, since its "one exception" line is no longer true. Behaviour is the same apart from the labels now coming from the app.

SaveViewDialog.qml:38 uses standardButtons Dialog.Save | Dialog.Cancel, so its labels come from Qt's platform theme. CutBordersDialog, DiscardRemovalDialog, FolderPickerDialog and RemoveFolderDialog use words.text('ui-cancel'). LANGUAGES.md:113 records the one exception.

### UI-009 [medium] words.phrase count argument passed in several shapes — CONFIRMED

**Disposition: Fixed.** The `count` argument now has one meaning, written on `Words::phrase` in `crates/desktop/src/words.rs`: the number the entry's plural selector reads, and 0 for an entry with no selector. Every call that passes a selector count (StatusBar `ui-inbox`, CutBordersDialog, RatingBar `ui-stars`, YandexPanel `ui-save-picked`) already fits it. The one call that passed a non-selector value, `InboxPanel.qml` `ui-inbox-more` and `ui-inbox-more-folders` with `panel.failedTotal`, now passes 0 (its entries read `$text` and `$other` only), so the fixed 0s elsewhere are explained by the same rule. No Fluent entry or rendered text changes.

The count argument to words.phrase is passed as a fixed 0 (YandexPreview.qml:129, FilterBar.qml:271, TopBar.qml:85-86), as the unread count (StatusBar.qml:114), as failedTotal (InboxPanel.qml:462), as dialog.count (CutBordersDialog.qml:42), as index (RatingBar.qml:76) and as pickedCount (YandexPanel.qml:164). The shapes differ and the fixed 0 is not explained.

### UI-010 [medium] Key-name notation differs across hint lines and strings — CONFIRMED

**Disposition: Fixed.** "Ctrl+/− resize".

ui.ftl:135 writes Shift+R, :218 writes R / Shift+R, :230 writes ⇧R, and text.rs:790-799 asserts the hint strings in both forms. Key notation differs across the hint lines.

### UI-011 [medium] Client-facing English Display text bypasses the catalogue — PARTIAL

**Disposition: Decided and applied.** The desktop's `cause_text` wraps a raw error in the catalogued `failure-detail` message ("Error: …"). The raw text itself stays English, since it comes from the system or library.

The English Display impls exist: names.rs:153 NormalizeError, scan lib.rs:344 ScanError::Removed, catalog error.rs:37 RootRemoved, library folders.rs:17 AddError::Refused, and writeback lib.rs:98 WriteError, which its doc says the app words through `pg_client::text`. The CLI maps NormalizeError itself (cli lib.rs:149-151). Whether the English Display text reaches a GUI user was not confirmed, so the finding is only partly shown.

### UI-012 [medium] CLI and log refusal text bypasses cli.ftl — CONFIRMED

**Disposition: Fixed.** The "error:" prefix is cli-error in cli.ftl.

gallery main.rs:50 prints 'gallery was built without its app window...' with eprintln, a user-facing English message outside cli.ftl. desktop lib.rs:161 and shell.rs:71 write English through `pg_log`. The yandex fetch.rs:44, :53 and parse.rs:10 Unexpected details are English literals.

### UI-013 [medium] Refusal advice text drifts from the test and the other messages — CONFIRMED

**Disposition: Fixed.** The path-rules refusal was the one catalog refusal outside `Refusal`, so the window had no words for it and showed the English Display. It is now `Refusal::PathRules { detail, wanted }` in `crates/catalog/src/error.rs` (the `Error::PathRules` variant is removed; `check_rules` in `lifecycle.rs` returns `Error::Refused`), with the same advice sentence "open it on the platform that made it, or move it aside and scan again" in `Refusal::message`. The window maps it through `catalog_refusal_message` in `crates/desktop/src/lib.rs` to a new `Message::CatalogPathRules` (`catalog-path-rules` in `text.ftl`), which carries that advice. The `lifecycle.rs` test now matches `Refusal::PathRules`, and a new render assertion in `client/src/text.rs` covers the window's words. The `detail` phrases in `lifecycle.rs` (made with the X path rules) stay as the technical account they are; the audit's `core.rs:164` cite no longer holds the advice, which is at `error.rs` for the predates-key-version refusal and is consistent with its Fluent entry.

The same refusal advice is worded three ways: lifecycle.rs:27 'made with the {name} path rules', lifecycle.rs:28 'made before its path rules were recorded', error.rs:20-21 'open it on the platform that made it, or move it aside and scan again', and core.rs:164 'restore a `.pre-v*.bak` or start fresh'. The test at lifecycle.rs:1030 asserts the error.rs wording on Error::PathRules, which is correct.

### UI-014 [medium] Doubled and mismatched strings in doctor and qmltest scripts — CONFIRMED

**Disposition: Fixed.** check_qt.py and profile.py use QT_DIR_HOW; the profile.ps1 original is removed.

doctor.py:174 and :177 print the same strings as doctor.ps1:88 and :92. The `QT_DIR` not-set message is written in qmltest.ps1:20, qmltest.py:41 and `lint_qml.py:70`, with the same wording in each.

### UI-015 [medium] Viewer colours are literals, one a magenta border — CONFIRMED

**Disposition: Fixed.** Two tokens are added to the singleton `crates/desktop/qml/Theme.qml`: `viewerFill` (`#000000`, the viewer's background, previously `color: "black"`) and `borderFrame` (`#ff00ff`, the frame while borders show, previously a literal `border.color`). `Viewer.qml` now uses `Theme.viewerFill` and `Theme.borderFrame`; values are unchanged, so nothing renders differently. `Viewer.qml` already reads `Theme.panelScrim`, so the singleton is in scope there, and no test stub of `Theme` exists to update. The magenta is kept on purpose for contrast; only its place changes.

Viewer.qml:443 uses a literal border colour #ff00ff and Viewer.qml:154 a literal 'black' background, while YandexPanel.qml:1 and other panels use Theme tokens. The viewer's colours are outside the theme.

### UI-016 [medium] Catalog and database used for the same file — CONFIRMED

**Disposition: No change.** "not a catalog" is the form for the catalog's own messages; the dbfile text stays in its layer.

dbfile judge.rs:49 says 'it is not a database', dbfile remove.rs:35 says 'can't move {} aside', and catalog lifecycle.rs:60 says 'catalog {}: another program or connection has it open'. Catalog and database are both used for the same file in the refusal messages.

### UI-017 [low] Product name spelled as a Rust literal and in Fluent — PARTIAL

**Disposition: Fixed.** The error box title comes from the `ui-app-name` entry.

The em-dash claim is refuted: ui.ftl:122 and LANGUAGES.md:85 both use 'Photo Gallery — {text}' with an em dash, so no hyphen is in use. The real defect stands: alert.rs:6 passes the literal 'Photo Gallery' to the error box instead of going through a catalogue key (ui-app-name at ui.ftl:121 exists), and text.ftl:172 holds the crash box text.

### UI-018 [low] Window and dialog strings use different scripts and separators — CONFIRMED

**Disposition: Fixed, by the default chosen in this pass (owner may revisit).** " · " joins facts in the HUD and the facet labels; a screenshot check is not run.

Separators differ in the GUI: FilterBar.qml:156 uses two spaces, Hud.qml:90-92 three spaces, SaveViewDialog.qml:51 ' · ', YandexPanel.qml:77 ' '( and ')', TopBar.qml:101 ' ▴' and ' ▾', and StatusBar.qml:100 '⚠ ' prefix. ResultTile.qml:92 uses '×' in the size caption.

### UI-019 [low] Plain-text rule not applied in test fixtures that carry markup — REFUTED

The fixtures with `<b>` are deliberate: `tst_inbox_panel.qml:110` and :234 and `tst_log_panel.qml:100` assert that the markup is shown as literal text (selectedText equals the string with `<b>`). The production panels use textFormat PlainText (InboxPanel.qml:462-464), so the test checks the plain-text rule.

### UI-020 [low] Panel and status text on the failed-file ledger is inconsistent — REFUTED

Both failed-file link keys read 'Show the failed files': ui.ftl:103 (ui-inbox-show-files) and :113 (ui-failed-link). ledger.rs:88 names the same entry action. The wording is consistent, and the other cited comments describe different status messages.

### UI-021 [low] Plan docs state UI text differently from the code — CONFIRMED

**Disposition: Fixed.** The only plan statement the audit cites is row L-42 of `docs/plans/REVIEW_LEDGER.md`, which says the 'All folders' label comes from `text::ui` in Rust. The row's disposition now states the current source: the QML asks `words.text` for `ui-all-folders` and `ui-drive-roots` (`FolderTree.qml`, `TopBar.qml`, `SaveViewDialog.qml`), and `folders.rs` returns an empty name for those rows. The "Choice 13" the row cites is not in `docs/plans` any more, so the decision is written in the ledger. The row's open item (a test for the row labels) is still open and is not part of this fix.

`REVIEW_LEDGER.md:87` says the 'All folders' label comes from text::ui('ui-all-folders') in Rust. The ledger itself records the deviation as undeclared: the code returns an empty name and FolderTree.qml holds the words. The plan and code state the label's source differently.

### UI-022 [low] Historical plan docs name keys and facts the code no longer uses — PARTIAL

**Disposition: No change.** The historical plans are dated records.

'round-failed' is still a live key in the catalog doc (ledger.rs:58) and RoundFailed is still a Notice in the client (text/notice.rs:74), so one cited key is still in use. The 'scan-failed' key and the RoundPanicked-as-State claim were not checked, so the historical-drift claim is only partly shown.

### UI-023 [low] Sentence-final periods differ between messages and labels — PARTIAL

**Disposition: No change.** Labels and sentences are different kinds of string.

yandex-nothing-picked (text.ftl:137) and yandex-no-folder (:138) end with a period, and ui.ftl labels (:8 onward) do not. Labels and sentences are different kinds of string, so a missing period on labels is expected. Whether messages are consistent with each other was not checked across the file.

### UI-024 [low] Apostrophe and quote style differs across UI copy — CONFIRMED

**Disposition: Fixed.** The UI copy uses straight quotes throughout, as the tests and most entries already do (`couldn't`, `Can't`). The one curly-quoted entry, `filter-name` in `crates/client/locales/en/text.ftl`, now reads `name has "{ $name }"`. A grep of the Fluent files, the QML and the Rust UI text finds no other curly apostrophe or quote in the copy, so nothing else changes. No test asserts the curly form.

text.ftl:40 wraps the name in curly quotes '(name has “{ $name }”'), while text.ftl:135 uses straight apostrophes in "Can't open". Tests at client text.rs:485, :667 and :736 assert straight apostrophes in "couldn't" and "can't".

### UI-025 [low] CLI and GUI sentence case and tool casing differ — CONFIRMED

**Disposition: No change.** The header of cli.ftl states the case rule.

CLI lines start lowercase '(imported 12 files...' at client text/cli.rs:613), GUI lines start capitalised, and cli.ftl:14 begins with a stray comma ', { $count } changed while read'. Tool messages mix sentence case too: pgtools.py:35 starts lowercase and `lint_qml.py:70` starts uppercase.

### UI-026 [low] Eval and bench output uses x and multiplication sign for sizes — CONFIRMED

**Disposition: Fixed.** The rule is: sizes in console output (eval and bench harnesses) use an ASCII `x`, and the multiplication sign `×` stays in the window's typography (QML captions) and in prose. The `×` size outputs are changed to `x` in `eval/jpeg-decoders/gpu.py`, `eval/jpeg-decoders/nvjpeg.py` (two prints), `eval/jpeg-decoders/src/main.rs`, `eval/jpeg-decoders/src/lib.rs` (a bail message), `eval/png-decoders/pillow_bench.py` and `eval/png-decoders/src/main.rs`, matching the `harness.cpp` output the audit cites as correct. `YandexPanel.qml` and `ResultTile.qml` keep `×` because they are window copy, not tool output; the audit's `video.rs:122` cite is a doc comment, not output, and is left. Behaviour: console text only.

YandexPanel.qml:272 and ResultTile-style captions use '×', as do nvjpeg.py:278 and video.rs:122. The png harness harness.cpp:205 and the jpeg harness use an ASCII x in sizes. The cited yandex.rs:309 is a `data_changed` call, not the file-name suffix, but the mixed separators are real.

### UI-027 [low] Bench output labels for units and references differ — CONFIRMED

**Disposition: Fixed.** bench_decode prints "images/s".

nvjpeg.py:246 prints 'ms/image' and `nvimgcodec_bench.py:90-92` prints 'median wall ms per image' and 'CPU ms per image'. nvjpeg.py:296 says 'vs Pillow' while the harness says 'vs libjpeg-turbo'. The unit and reference labels differ across the bench outputs.

### UI-028 [low] Test assert messages contradict checks or each other — CONFIRMED

**Disposition: No change.** The writeback assertion message matches its check.

writeback.rs:769 asserts (status.error, `status.error_file`) == (None, None) but its message says 'the failure of a dropped write is still shown'. The message contradicts the check. borders.rs:86 and writeback.rs:377 give two skip reasons for the same permission case with different wording.

### UI-029 [low] Notice id and source-word naming is documented two ways — CONFIRMED

**Disposition: Fixed.** The key part is `<code>` everywhere, as the code names it: `Source::code` and the `notice-<code>-title` arm of `Notice::title_id` in `crates/model/src/notice.rs` both use "code", and the doc comment on `Source` already said `source-<code>`. `docs/reference/NOTIFICATIONS_DESIGN.md` (the `Message` wording bullet) said `source-<name>`, and now says `source-<code>` to match. No code or Fluent id changes.

notice.rs:110 documents source words as '`source-<code>'`, while `NOTIFICATIONS_DESIGN.md:344` says '`source-<name>'` and the id arm uses `notice-<code>-title` (notice.rs:941). The doc comment and the design name the key part differently.

### UI-030 [low] Inbox and version-mismatch messages are worded in different voices — CONFIRMED

**Disposition: No change.** The two inbox log lines share one voice.

app inbox.rs:446 says 'the inbox's queue is full: notices are not kept' and :927 says 'the inbox couldn't keep {notices} notices'. thumbcache schema.rs:203 says 'layout version {version}, this build's is `{CACHE_VERSION}'` and dbfile schema.rs:45 says 'differs from this build's'. The cited desktop/src/inbox.rs paths are wrong; the messages are in app/src/inbox.rs.

## logging

### LG-001 [high] Thumbnail events logged under two targets — CONFIRMED

**Disposition: Fixed.** The started-over log line in `forward_thumb_events` (crates/app/src/app.rs) now uses target `thumbs`, the same target as the per-file failures in `images.rs` (`undecodable`) and the ledger path in `inbox.rs`, so every thumbnail record shares one target. `thumbs` is chosen over `thumbnails` because it is the thumbnail pass's name (`Source::Thumbs`, `Phase::Thumbs`) and the target the ledger and tests already use. The notice's own source is `cache`, so this one line is a deliberate exception to the notice-source rule; it is a direct diagnostic, not a `write_log` notice.

app.rs:208 logs the thumbnail cache's started-over event under target thumbnails. images.rs:147 logs thumbnail failures under target thumbs, and the test at images.rs:566 asserts thumbs. One kind of thumbnail event is filed under two targets.

### LG-002 [high] Perf-log rotation cap in docs does not match code — REFUTED

The perf log has its own cap: crates/perf/src/lib.rs:31 sets `FILE_CAP` to 16 MiB, and lib.rs:85 opens `pg_log::LogFile` with it. PROFILING.md:1331 says 16 MiB and two older files, which matches. The 5 MiB figure in pg-log's `FILE_CAP` (crates/log/src/file.rs:26) is the app log's cap, and `NOTIFICATIONS_DESIGN.md:330` describes that one correctly. The finding compared the wrong cap.

### LG-003 [medium] Notice written to the log in two formats — CONFIRMED

**Disposition: Fixed.** The three notice log sites now share `inbox::log_line` (crates/app/src/inbox.rs): `write_log`, the ledger entry path in the inbox's refresh, and the catalog-not-closed line in `App::shutdown` (crates/app/src/app.rs). Each writes the stored JSON and falls back to the same Debug-with-error form, instead of the bare error text in two places. The notice is plain serializable data, so the fallback is never reached; sharing it is the proper choice because it makes the shape one function's decision. Behaviour change: the unreachable fallback text changes from the error alone to the notice's Debug form.

app.rs:182 logs a notice as JSON and falls back to the error text. inbox.rs:622 logs the same notice as JSON and falls back to a Debug form with the error. inbox.rs:1064 falls back to the bare error. The same notice is written to the log in three fallback shapes.

### LG-004 [medium] Ledger entries always logged at warn, notices mapped by level — CONFIRMED

**Disposition: Fixed.** The new `log_notice` helper in crates/app/src/inbox.rs holds the one level-and-source mapping (`notice.level()`, `notice.source().code()`), and both `write_log` and the ledger entry path in the inbox's refresh call it. A ledger entry is now logged at its own level, as a notice is, instead of always at warn. Behaviour change: a ledger entry whose notice is info or error is now logged at that level (today the ledger notices are warnings, so the log output is unchanged for them).

inbox.rs:625-627 logs a notice at its own level (info, warning or error). inbox.rs:1069 logs every ledger entry at warn, whatever its level.

### LG-005 [medium] Inbox logs under two target schemes — CONFIRMED

**Disposition: No change.** Notices log under their source code's target and subsystem records under the subsystem name; this is the rule.

The inbox logs under a literal target inbox (inbox.rs:446, and at 553, 844, 851, 860 and 927). `write_log` at inbox.rs:623 uses notice.source().code() as the target, and the ledger path at inbox.rs:1069 uses the same source code. Two target schemes are used in one file.

### LG-006 [medium] CLI summary written to stderr in one mode and stdout in another — PARTIAL

**Disposition: No change.** The split keeps stdout clean when stdout carries the data.

In --stdin mode the borders command writes its data to stdout (lib.rs:715) and so writes its summary to stderr (lib.rs:716), which keeps stdout clean for the data. The CSV mode prints its summary to stdout because there is no data stream. The two modes differ for a reason, so the mismatch is a choice, not a fault.

### LG-007 [medium] Environment version line on stderr in one eval script, stdout in others — CONFIRMED

**Disposition: Fixed.** `eval/jpeg-decoders/encoders.py` (the `__main__` block) now prints `env.line()` to stdout, as the other five eval scripts that print it do (gpu.py, gpu_encoders.py, debesheedas.py, nvimgcodec_bench.py, nvjpeg.py). The environment line is provenance for the results, so it belongs in the same stream as them: on stderr it was lost from a redirected report. Nothing in the repo parses encoders.py's stdout (its TSV rows have no consumer), so the one-line move is safe; a reader of that output would see one extra first line.

encoders.py:237 prints the environment line to stderr. gpu.py:89, `gpu_encoders.py:45` and debesheedas.py:41 print it to stdout. The eval scripts disagree on where the environment line goes.

### LG-008 [medium] Desktop startup diagnostics bypass `pg_log` — CONFIRMED

**Disposition: No change.** The perf-log failure is returned by trace::install and logged after the logger starts.

The desktop trace.rs:33 writes a failure to open the perf log with eprintln. The desktop screenshot.rs:1 is a hook that uses eprintln as well. `NOTIFICATIONS_PLAN.md:57` records that the GUI build never shows stderr, so these lines are lost in the app that is meant to log them.

### LG-009 [medium] Perf span names do not follow the pass./pending. pattern — CONFIRMED

**Disposition: Fixed.** pending.thumbs and pass.mark_missing are the names; CODEBASE_REVIEW_VALIDATION has a current-names note.

Span names do not follow one pattern: borders lib.rs:63 and import lib.rs:79 use pass. and pending. pairs, thumbs lib.rs:212 uses pass.thumbs with thumbs.pending (reversed), scan lib.rs:2252 looks up scan.walk, passes.rs:75 uses scan.after, roots.rs:169 uses bare `mark_missing`, lane.rs:93 uses bare round, transaction.rs:18 uses `catalog.tx.*`, and crop lib.rs:240 has pass.cut beside pass.crop. The prefix rule is not applied.

### LG-010 [medium] Pairing perf names split across two prefixes — CONFIRMED

**Disposition: Fixed.** Every pairing span now uses the `pair.` prefix that the catalog's pairing code (crates/catalog/src/pair.rs) already used: the import pairing's `pairs.probe` and its five `pairs.load`, `store`, `join`, `write` and `commit` records (crates/import/src/pairs.rs) are renamed to `pair.*`. The pass span stays `pass.pairing`, which the pass rule of LG-009 needs. `pair` was chosen because it has the ten catalog spans and is the module's name, so the smaller set moves. Behaviour change: only the perf log's names for those six import pairing records; nothing in the repo matches them by name.

pair.rs:88 uses `pair.after_scan` and pair.setup, while import pairs.rs:136 uses pass.pairing and pairs.rs:228 uses pairs.load, pairs.store and pairs.join. Pairing spans use two prefixes.

### LG-011 [medium] Error chains formatted with {e} and {e:#} in different sites — CONFIRMED

**Disposition: Fixed.** Every `pg_log` message that formats an error now uses `{e:#}`, so a logged error keeps its whole cause chain: about twenty sites across crates/app (library.rs), crates/catalog (schema.rs, lifecycle.rs), and crates/desktop (controller.rs, failed_files.rs, lane_model.rs, lib.rs, library.rs, log_model.rs, notices.rs, picker.rs, shell.rs, video_player.rs, yandex.rs). The log is the diagnostic record, so the chain belongs there; the CLI's `{e:#}` and codec's `decode.rs` (which deliberately adds the libjpeg turbo context) already did this. Behaviour change: a logged error whose source is an anyhow chain now shows its causes; for std errors the text is unchanged, since `{:#}` only differs for chained errors. The notice texts (user-facing) are not changed.

app library.rs:421 logs a failed temp removal with {e}, which drops the cause chain. inbox.rs:1064 uses {e} too. desktop `log_model.rs:124` uses {e}. gallery main.rs:31 prints {e:#}, and codec decode.rs:165 keeps both errors. The chain format is not consistent.

### LG-012 [medium] Print-lint attribute repeated in each library crate — CONFIRMED

**Disposition: Fixed.** The print lints were already `deny` in `[workspace.lints.clippy]` (Cargo.toml), and every crate opts in with `[lints] workspace = true`, so the twenty per-crate `#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]` lines (with their comment) were a copy of that rule. The test exemption they carried is already in clippy.toml (`allow-print-in-tests = true`). I removed the copies from all twenty library crates, including catalog, render, thumbcache and analysis, so the rule has one home. The finding's remark that thumbs has no print lint is not borne out: thumbs has the copy at its line 7, which is removed with the rest, and it was under the workspace lint all along. Behaviour: none for library builds or tests; the `#![expect]` sites in the binaries and examples are unchanged.

The print lint attribute is repeated in catalog lib.rs:12, render lib.rs:8, thumbcache lib.rs:28 and analysis lib.rs:6, and analysis duplicates the workspace print lints (Cargo.toml:81-82). thumbs lib.rs:1 has no print lint at all, so the set of crates that carry it is not uniform.

### LG-013 [medium] Session record documented but not written by the desktop app — REFUTED

`pg_log::init` (log lib.rs:86) calls LOG.init, which calls `init_capped` (lib.rs:266-267), and `init_capped` logs the session record with target session and message start (lib.rs:272). So the desktop run does write a session record. The doc at notices.rs:53 is accurate.

### LG-014 [medium] Log file name in user text is hard-coded in some places and a value in others — CONFIRMED

**Disposition: Fixed.** The Yandex panic line no longer names `gallery.log`: `yandex-panicked` (crates/client/locales/en/text.ftl) takes `{ $log }`, `Message::YandexPanicked` carries `log` like `DetailsInLog` does, and `yandex_status` takes the log's path (crates/client/src/text.rs). The desktop caller (`status_text` in crates/desktop/src/yandex.rs) passes `pg_log::path()`, the file as it is on disk, which the window always has open by then. So every message that names the log now gets the same value. Behaviour: the line shows the full log path (as the panic box already does) rather than the bare file name; the signature of `text::yandex_status` gained a parameter (its three callers are updated).

text.ftl:161 names gallery.log directly in a message, while text.ftl:174 takes the log path as a variable $log through DetailsInLog and DetailsNotLogged (message.rs:247). The log file name is hard-coded in one message and passed in others.

### LG-015 [medium] Hard-coded log call counts in docs disagree — CONFIRMED

**Disposition: Fixed.** Each count now says what it counts and when: `NOTIFICATIONS_DESIGN.md` marks its 34 `log::note` sites (lines 182, 332, 336) as the count at `d2cbc13`, the same commit the design note at :327 already cites, and the 36 at :118 is explained against it. `docs/plans/LANGUAGES.md` gives the current figure, 67 `pg_log` call lines as of 2026-11-21 (3 info, 57 warn, 6 error, 1 panic, from a grep of `crates/` outside tests), in place of the old "about 51" from an unnamed investigation. The historical counts stay, since the design note is a record of a plan, and the dates let a reader tell them apart. No code changed.

The call counts in the docs disagree: `NOTIFICATIONS_DESIGN.md:118` says 36 log sites, :182 and :323 say 34, and LANGUAGES.md:99 says about 51 `pg_log` calls with a breakdown. Each count is for a different thing or a different date, but none is marked as the same count.

### LG-016 [medium] Skip message duplicated between exiftool and platform helper — CONFIRMED

**Disposition: No change.** The message is written once. The ExifTool test (crates/exiftool/src/lib.rs, `start_or_skip`) calls the shared `pg_testkit::missing_tool("ExifTool")`, which is the only place that prints `{tool} not found: skipped` (crates/platform/src/testing.rs). The finding's second copy at exiftool's lib.rs:298 is that call, so there is nothing to merge; the helper is already the proper single home.

exiftool/lib.rs:298 prints ExifTool not found: skipped. platform testing.rs:77 prints the same message shape with a tool name. The message is written twice, in two crates, with the same wording.

### LG-017 [low] Print lint allows a stderr line that uses Debug level names — PARTIAL

**Disposition: No change.** The Debug form in the human line is deliberate; the stored JSON form is lowercase.

plain() (log record.rs:59) uses the Debug form of the level for the human stderr line, so the line reads Warning. The doc at log lib.rs:18 describes the JSON level field as lowercase info, warning or error, and that is the stored form. The human line is a different output, so the Debug form there is not a contradiction. The finding's point is that the two forms differ, which is true but deliberate.

### LG-018 [low] Identical warning text repeated in two places — CONFIRMED

**Disposition: Fixed.** The shared clause "it is tried again at the next start" is now one constant, `RETRY_AT_START`, in crates/catalog/src/lifecycle.rs, and both `VACUUM` warnings of `vacuum_if_owed` format it in. The two messages keep their own subjects (no room to compact; compacting failed), so only the repeated clause is shared. Behaviour: none; the text is the same as before.

lifecycle.rs:298 and lifecycle.rs:317 use the same phrase it is tried again at the next start, in two warnings with the same meaning.

### LG-019 [low] Lane model log category uses a space — CONFIRMED

**Disposition: Fixed.** The two lane-row warnings in crates/desktop/src/lane_model.rs now log under `lanes`, a single word, as the other categories are (`gallery`, `providers`, `log`). The word names what the records are about (the lane rows), which is the rule the log's target follows. Behaviour: the log's category for these two lines changes from `lane model` to `lanes`; nothing in the repo filters on the old value.

`lane_model.rs:225` and :232 log under the category lane model, with a space. Every other `pg_log` category in the set is a single word.

### LG-020 [low] Warning wording uses contractions in some places and full forms in others — CONFIRMED

**Disposition: Fixed.** The log's warnings use contractions (about twenty sites: `couldn't`, `can't`, `doesn't`, `isn't`), so the two warnings that used the full form now do too: the command-before-the-grid warning in crates/desktop/src/gallery.rs ("it wasn't carried out") and the invalid-value warning in crates/desktop/src/shell.rs ("isn't a valid value"). The finding's example, library.rs "wasn't deleted", is itself a contraction, so it was already in the house form and is unchanged. Behaviour: two log lines read differently; no text is matched by a test or tool.

Warnings use contractions in some places (`lane_model.rs:225` can't be read, :232 don't parse, notices.rs:514 couldn't) and full forms in others (library.rs:421 wasn't deleted). Low severity; the mix is real.

### LG-021 [low] Migration warnings use a different target from catalog warnings — CONFIRMED

**Disposition: Fixed.** The two `VACUUM` warnings in crates/catalog/src/lifecycle.rs (`vacuum_if_owed`) now log under `catalog`, as the rest of the module's catalog warnings do, instead of `migration`. The target names the subject the record is about, and these records are about the catalog file. Behaviour: the log target of those two lines changes from `migration` to `catalog`; no code or test filters on `migration`.

Catalog lifecycle warnings use the target catalog at lifecycle.rs:198, and the migration warnings at :297 and :315 use the target migration. The same module files two kinds of catalog warning under two targets.

### LG-022 [low] Same user-visible message written in PowerShell and Python twins — PARTIAL

**Disposition: No change.** `setup-ffmpeg.ps1` is not at HEAD, so the twin message cannot be shown.

tools/setup-ffmpeg.ps1:22 is missing from the working tree (deleted, present at HEAD), so that cite can't be checked here. The python cite at `setup_ffmpeg.py:75` is a docstring and does not print the sentence. The twin-message claim is not shown by these cites.

### LG-023 [low] Tool status words use different vocabularies — CONFIRMED

**Disposition: Fixed.** The tools now follow one rule for a check's status word: lowercase for a result that is fine (`ok`, `current`, `same`, `budget`, `optional`, `note`), uppercase for one that needs attention (`MISSING`, `SKIPPED`, `NO-BENCH`, `OVER`, `UNDER`, `UP`, `DOWN`). `tools/fork-updates.py` was the outlier: its "newer release" is now `NEWER`, and its "could not read crates.io" is `UNREADABLE (...)`, with the exit-status checks and the report's status column moved with them. `doctor.ps1` and `perf-compare.py` already followed the rule and are unchanged. Behaviour: the fork report's status column and its status strings change; the workflow reads the exit status, not the text.

doctor.ps1:17 uses ok and MISSING, perf-compare.py:113 uses SKIPPED, NO-BENCH, MISSING, OVER and UNDER in uppercase, and fork-updates.py:99 uses lowercase newer release and current. The status words for a check's result use three vocabularies.

### LG-024 [low] Command echo present in some Python tools but not the shell build — PARTIAL

**Disposition: No change.** The Python echo pattern is real; the shell build it compares with no longer exists.

docker/check.py:27 and pgtools.py:128 echo each command with a + prefix. build.sh does not exist in the repository (no such file in tools), so the shell-build comparison in the finding cannot be checked. The Python echo pattern is real.

### LG-025 [low] Progress cadence and flushing differ between eval scripts — CONFIRMED

**Disposition: No change.** The per-script cadence is deliberate; flushing is uniform.

Progress cadence differs: verify.py:791 prints every 100 pairs, `verify_synth.py:74` every 260 pairs, `imagereduce_switch.py:184` every 50 items, and `bench_thumbs.py:345` prints without flush while :231 flushes. The eval scripts pick their own intervals.

### LG-026 [low] Probe examples format plan and header lines differently — CONFIRMED

**Disposition: Fixed.** Both pairing probes in crates/catalog/examples now print a statement's plan as `plan of <first four words>: <plan>` (`pairs_write_probe.rs` `show_plans` took only the first verb, which does not tell two `SELECT`s apart; the load probe's rule is the one kept). Their result headers now share one shape, `variant: <columns>; median of N runs`, in `pairs_write_probe.rs` and `pairs_load_probe.rs`. Behaviour: the probes' printed text changes (plan labels and header wording); they are manual examples, and no test reads their output.

`pairs_load_probe.rs:131` writes plan of the first four words, `pairs_write_probe.rs:153` writes plan of the first verb, and `pairs_write_probe.rs:103` writes a header that differs from `pairs_load_probe.rs:99`. The probes format plan and header lines differently.

### LG-027 [low] Test diagnostics printed with eprintln or println inconsistently — CONFIRMED

**Disposition: Fixed.** A test's diagnostic output (counts, timings, notes) now goes to stderr everywhere, as the browse and codec tests already did: the rehearsal test (crates/catalog/tests/rehearsal.rs, all its report lines), the join timing lines (crates/dedup/src/join.rs) and the rules-reference count (crates/model/tests/rules_reference.rs) changed from `println!` to `eprintln!`. The rule matches the platform helpers, where a skipped test says so on stderr. The one `println!` left in a test is `PG_TESTING_HELD` (crates/platform/src/testing/tests.rs), which is a protocol line the parent test reads from the child's stdout, so it must stay on stdout. Behaviour: the lines are still shown under `--nocapture`, and the rehearsal's report is on stderr now.

Test diagnostics use eprintln in browse tests.rs:415 and codec decode.rs:471, and println in join.rs:2912, `rules_reference.rs:126` and rehearsal.rs:102. Low severity.

### LG-028 [low] Superseded `PG_METRICS` body left in `DECODE_ONCE_PLAN` — PARTIAL

**Disposition: No change.** The superseded note is marked, so the staleness is visible.

`DECODE_ONCE_PLAN.md:1227` carries a superseded note that says `PG_PERF_LOG` replaces `PG_METRICS` and that nothing was built under the old name. The body below the note still describes it, which is how superseded design text usually stays in a plan. The note is there, so the staleness is marked, not hidden.

### LG-029 [low] Trace stage function names repeat the crate prefix — REFUTED

pg-trace (crates/trace) is a real crate, and its stage functions use plain C names such as `pg_stage_main`, so the `pg_stage_` prefix is required by the SDE control string quoted in crates/trace/src/lib.rs. The repeated prefix is deliberate.

## tests

### T-001 [medium] Scratch folder cleanup differs: let `_`, unwrap, .ok() or none — CONFIRMED

**Disposition: Fixed.** End-of-test scratch cleanup is now `let _ = std::fs::remove_dir_all(...)` in every test where it was `.unwrap()` or `.ok()`, in 29 test files across catalog, jobs, platform, render, thumbcache, writeback, library, crop, cli, dbfile, app and the yandex tests. Cleanup is best-effort because `pg_platform::testing`'s sweep already removes what an earlier run left, so a folder Windows still holds no longer fails a passing test. Kept as `unwrap`: the removals that are the assertion (the shutdown tests in app.rs) and the ones that set up a scenario (sqlite.rs `a_stale_alias_is_swept`, library runner.rs, cli lib.rs). This is a behaviour change: a cleanup error at test end no longer fails the test. The `replay.rs:197` and `player.rs` cites are now converted; examples keep `?`.

Scratch cleanup is written five ways in the cited files: let `_` = `remove_dir_all` (app.rs:265, images.rs:483, inbox.rs:1430, dir.rs:691, video lib.rs:388), unwrap (app.rs:279, images.rs:420, file.rs:359, `other_process.rs:108`), and .ok() (player.rs:182). The replay.rs:197 cite is missing.

### T-002 [medium] Scratch root names do not follow the `scratch_<crate>` pattern — CONFIRMED

**Disposition: Fixed.** Every test scratch root now names the crate its file belongs to, `scratch_<crate>`, in 38 files: the feature-named roots (`scratch_rate`, `scratch_store`, `scratch_listing`, `scratch_yandex`, `scratch_key_version`, `scratch_progress`, `scratch_borders` and others) moved to their crate's root, and `thumbcache` moved from `scratch_thumbs` to `scratch_thumbcache`. One base per crate is the right grouping because `run_folder` already keeps each call's folder distinct, so the base only labels the crate's folders and the sweep treats them as one unit. Behaviour: only the on-disk names of test folders in the temp folder change. The `replay.rs:167`/`:202` cites are included in the change (now `scratch_app`).

The scratch root names do not follow one pattern. `scratch_store`, `scratch_listing`, `scratch_app` and `scratch_rate` name areas or features, not the crate. `scratch_catalog` and `scratch_app` match their crate, and the second argument mixes pg-layout, kebab-case and `snake_case`. The replay.rs:167 and :202 cites are missing.

### T-003 [medium] Wait helpers are copied per file with different bounds and intervals — CONFIRMED

**Disposition: Fixed.** `pg_testkit` now has one `wait_until(within, what, done)` polling every 20 ms, and `wait_for_path(path, within)` for the file waits. The copies in catalog, dbfile and thumbcache `tests/other_process.rs` and `dbfile/tests/live.rs` became `wait_for_path` calls, and the copies in library `runner.rs`, `tests/lanes.rs`, `tests/writeback.rs` and render `player.rs` became `wait_until` calls keeping their bounds (30 s, 60 s, 120 s, 10 s). This is a behaviour change: the poll interval is now 20 ms everywhere (writeback polled every 200 ms, player every 5 ms), and the failure message wording changed. `writeback.rs` `wait_for_queue` is kept because it propagates a query error, which the shared helper cannot.

`wait_for` is copied in `catalog/tests/other_process.rs:17`, dbfile/tests/live.rs:17, library/src/runner.rs:395, library/tests/lanes.rs:90 and `thumbcache/tests/other_process.rs:14`, each with its own bound and 20 ms poll. render/src/player.rs:161 and writeback.rs:110 are variants. Several cites are off (lanes.rs:90 is not a wait, and cli lib.rs:980 is not a recv).

### T-004 [medium] Timing-window assertions that can flake under load — CONFIRMED

**Disposition: Fixed.** The upper-bound timing asserts that could fail on a loaded machine are gone or widened: `roots.rs` `a_busy_catalog_fails_after_the_busy_timeout_and_says_so` now asserts a lower bound (50 ms, the 100 ms timeout's half) instead of "under 4 s", and `transaction.rs` `a_busy_begin_fails_at_once_without_patience` drops its "under 1 s" check (the zero busy timeout is what makes it immediate), while `a_stop_ends_the_wait_and_the_patience_is_restored_after` checks under 30 s against its 60 s patience instead of 5 s. The remaining fixed sleeps (`inbox.rs` 1 s hold, `writeback.rs` 3 s settles) are either lower bounds or negative checks, so load cannot fail them. This is a behaviour change only in the bounds. The 3 s deadline failure recorded in `MULTI_DRIVE_LANES_PLAN.md:455` is not traced to a test here, since no tests are run in this pass.

Timing asserts with fixed windows exist: roots.rs:984 asserts under 4 s, transaction.rs:219 under 1 s, inbox.rs:1690 sleeps 1 s to hold a lock, writeback.rs:403 sleeps a fixed 3 s. `MULTI_DRIVE_LANES_PLAN.md:455` records a 3 s deadline test that failed once in a parallel run.

### T-005 [medium] Known flaky tests recorded in plans but left unfixed — PARTIAL

**Disposition: Fixed in part.** The cites are corrected; reproducing the flaky run needs a Windows run.

`IMPORT_FAILURES_PLAN.md:185` records a flaky test that fails 2 of 5 alone. The test still exists under its name at thumbs/src/lib.rs:699, not at the cited :687 (which is a different test). The 2-of-5 rate was not reproduced here, so whether it is still flaky is unknown.

### T-006 [medium] Small test helpers duplicated across modules — CONFIRMED

**Disposition: Fixed.** The identical `pattern(w, h)` in `edit/src/crop.rs` and `edit/src/rotate.rs` is one `pg_testkit::rgb_pattern`, which the image fixtures already live beside. The `library(rules)` helpers in `catalog/src/roots.rs` and `catalog/src/removal.rs` share their three opening lines in `test_support::rooted(rules)` (the root `D:\r`); roots now calls `rooted` directly, and removal's `library` starts from it and adds its folders. No behaviour change. The `pattern` in `render/src/maker.rs` and `library/tests/writeback.rs` differ in their arithmetic, so they are left alone.

edit crop.rs:105 and rotate.rs:93 have an identical four-line pattern() body. catalog roots.rs:449 and removal.rs:456 both define library(rules) with the same first three lines, then diverge.

### T-007 [medium] Catalog setup helper repeated across app and thumbs test modules — CONFIRMED

**Disposition: Fixed.** The app's three copies (`inbox.rs` `catalog_in`, `reader.rs` `catalog`, `library.rs` `library`) now call one `test_support::catalog_home(name)` in a `#[cfg(test)]` module in `app/src/lib.rs`, which makes the run folder, the catalog directory and the prepared catalog. In thumbs, the picture-folder and `Catalog::prepare` setup repeated in `imported()` and four tests is one `pictures(prefix, names)` helper in `thumbs/src/lib.rs`. Behaviour is unchanged; the thumbs helper uses `Catalog::prepare` directly as those tests did, while the app helper goes through `Layout` as the app's copies did.

The catalog setup (`run_folder`, `Layout::in_home`, `create_dir_all`, `open_catalog`) is repeated in app inbox.rs:1103 `catalog_in`, reader.rs:388 catalog and library.rs:485 library. thumbs lib.rs:528 imported() repeats it inline in its tests at 701 and 759.

### T-008 [medium] Test fixture images come from `pg_testkit` and `pg_codec::fixtures` — CONFIRMED

**Disposition: No change.** The fixture split is intended (a dev-dependency cycle would follow from a merge).

Test images come from `pg_testkit` (`png_rgb`, `bmp_rgb`, `tiff_rgb` in edit crop.rs:146-148, `png_rgb` in analyze.rs:235, apng and gif in metadata lib.rs:115) and from `pg_codec::fixtures` (`tiff_rgb8` crop.rs:156, `webp_lossy_rgb` crop.rs:264, orient in rotate.rs:89).

### T-009 [medium] Path spelling in test fixtures varies by method — CONFIRMED

**Disposition: Fixed.** Test paths now go through `spelled(rules, ...)` (or `p`, which spells) in the places that were raw or branched: `browse/fixtures.rs` `library` writes `E:\p` as `spelled(rules, r"E:\p")` (still inside the Windows-only branch), and `ledger.rs` `catalog_at` replaces its `if rules == Windows { r"D:\p" } else { "/p" }` with `spelled(rules, r"D:\p")`, which gives the same `/p` on POSIX. `desktop/src/notices.rs` now builds its expected line with `rules.display(&p(...))` rather than a literal, so the expectation is spelled by the same rules. No behaviour change; `inbox.rs` already goes through `p`, so it is left as it is.

Path spelling varies: browse fixtures.rs:56 uses a raw `r"E:\p"` literal, where its neighbours at :54 and :58 use spelled(rules, ...). ledger.rs:502 branches on rules, inbox.rs:461 builds `r"D:\{}"`, pair.rs:578 uses spelled, and notices.rs:634 writes PathRules::Windows directly.

### T-010 [medium] Under-both-rules coverage differs between test modules — CONFIRMED

**Disposition: Fixed.** The three plain `PathRules::Posix` tests in `catalog/src/inbox.rs` (`a_mark_of_an_older_row_leaves_the_later_rows_unread`, `a_row_this_build_cant_read_is_unknown`, `a_ledger_is_listed_among_the_rows`) now take `rules` and run under `under_both_rules!`, as their neighbours do; the ledger test's `"/p"` literals became `spelled(rules, r"D:\p")`. `queue.rs` `ratings_and_turns_queue_for_the_file` was pinned to `library(PathRules::Windows)` and now runs under both rules. `lifecycle.rs` `posix_paths_are_bytes_under_case_sensitive_rules` stays a plain test on purpose: its name says it pins the POSIX byte rule, so it has no Windows form. This is a coverage change: the converted tests now run twice, and the Windows run of the ledger test is new.

lifecycle.rs uses `under_both_rules!` in five places (804, 936, 975, 1005, 1020), but `posix_paths_are_bytes_under_case_sensitive_rules` at :1043 is a plain #[test]. inbox.rs:564, :636 and :663 are plain tests beside its `under_both_rules` tests, and queue.rs:407 runs library(PathRules::Windows) only.

### T-011 [medium] Test error handling mixes unwrap, expect, anyhow Result and panic — CONFIRMED

**Disposition: No change.** Test code may unwrap or expect; fixture modules carry a reasoned expect.

Error handling is mixed: `crop_folders.rs:87` returns anyhow::Result, crop.rs and the library tests use unwrap, catalog fixture/raw.rs:23 unwraps, fixture/state.rs:13-15 returns Result with ?, grid.rs:469 unwraps, shell.rs:628 expects, and borders.rs:23, pairs.rs:23 and `listing_laws.rs:23` put expect lints on helpers only. listing.rs:3 has a file-level expect.

### T-012 [medium] Test-end invariant checks applied to some suites only — CONFIRMED

**Disposition: No change.** No gap: Quiet checks on drop unless the thread is panicking.

writeback.rs:265 calls quiet.check() at the end, but runner.rs (test module, around :201 and :294) binds its Quiet as `_quiet` and never checks it. borders.rs:103 and :145 remove their folders with let `_`, so the end-of-test invariant is applied to some suites only.

### T-013 [medium] Golden comparison normalises line endings in one test and not the other — CONFIRMED

**Disposition: Fixed.** Both golden checks in `crates/model/src/notice.rs` now normalise CRLF to LF before comparing: `every_kind_is_stored_as_its_golden_line` (which reads `notice/golden.jsonl`, previously compared without normalising, though `lines()` hid the difference) and `the_resolver_table_is_the_golden_one` (already normalised). The two tests now do the same explicit step, so a CRLF checkout passes both. `app/src/prefs.rs` keeps its inline `GOLDEN` constant: it reads no file, so it has no checkout line-ending question, and it was left as it was.

notice.rs:1824 normalises CRLF with `replace("\r\n"`, "`\n"`) before comparing `resolver_table.golden`, but notice.rs:1586 compares `include_str` of golden.jsonl with no normalisation. prefs.rs:212 uses an inline GOLDEN const, a third pattern.

### T-014 [medium] Doc claims tests never touch the connection, but many do — CONFIRMED

**Disposition: Fixed.** The module doc of `catalog/src/fixture/raw.rs` now states the rule the tests follow: common state goes through the functions in `raw.rs`, and a test whose subject is a statement (a writer, a query, a migration) runs it on the connection itself, as `browse/read/tests.rs`, `writers.rs` and `ledger.rs` do; the layout tests open the file with `on_disk`. Moving those raw statements into `raw.rs` would churn tests whose point is the SQL, so the doc was made true rather than the tests moved. The `thumbcache/src/lib.rs` and `reader.rs` cites are another crate's tests and are not covered by this doc.

fixture/raw.rs:1-5 says the catalog's tests never touch its connection, except the layout tests. browse/read/tests.rs:741, writers.rs:1433, ledger.rs:602 and thumbcache lib.rs:1262 run raw SQL on c.conn or a fresh connection, and thumbcache reader.rs:180 builds a stub inline. The doc is not kept.

### T-015 [medium] Doc cites a test helper that does not exist — CONFIRMED

**Disposition: Fixed.** `docs/reference/NOTIFICATIONS_DESIGN.md` (the lints paragraph, the cited line) now names `pg_testkit::run_folder`, the helper the crate exposes and the catalog tests use. No code changed; the doc now matches the code.

`NOTIFICATIONS_DESIGN.md:363` says catalog tests use `pg_testkit::test_folder`. No `test_folder` function exists anywhere in crates. The crate exposes `run_folder` (testkit lib.rs:16).

### T-016 [medium] Test-section cut rule differs across the module graph tools — CONFIRMED

**Disposition: Fixed.** The cut is now one function, `production_part` in the new `tools/modgraph/testcut.py`: everything before the first line-start `#[cfg(test)]`. `edges.py`, `api_surface.py`, `dupes.py`, `split_check.py` and `module_graph.py` all use it (`module_graph.py` was the inline-`mod` outlier, and `split_check`'s and `api_surface`'s regexes became calls to it). The file-name rule (`is_test_file`, `tests.rs` and `tests/`) is a separate question about whole files and stays in `module_graph.py`. A side effect of the move: `api_surface.py` and `dupes.py` imported `edges.py`, whose report runs at import; they now import `testcut` instead. Not run here: `tools/test_modgraph.py`.

Five modgraph tools cut the test section three ways: `module_graph.py:63` only at an inline mod, `api_surface.py:49`, `split_check.py:72` and edges.py:18 at any line-start #[cfg(test)], and dupes.py:26 by startswith. `module_graph.py:56` defines tests by filename, a fourth rule.

### T-017 [medium] Timing methodology differs across eval benchmarks — CONFIRMED

**Disposition: Fixed.** `eval/jpeg-decoders/timing.py` now holds the two timing methods the harnesses need, named for their cost: `timed` (the budgeted median for cheap GPU decodes, as `gpu.py` uses it) and `median_s(reps, fn)` with `seconds_of(fn)` (a fixed number of calls with no warm-up, for an encoder or a whole program run). `encoders.py` (three reps, one for guetzli) and `debesheedas.py` (three runs) both call `median_s`, replacing their own timer loops and `statistics` calls. The repetition counts are unchanged; the methods are not one timer because a budget of 1.5 s is too long for a guetzli encode, and the module doc says so.

Benchmark timing differs: gpu.py:31, 53-58 takes the median of at least 5 calls over `BUDGET_S` = 1.5 s after 3 warm-ups. debesheedas.py:52 takes the median of 3 whole runs, and encoders.py:150 takes reps = 3 (1 for guetzli). The methods are not one shared timer.

### T-018 [medium] Tests that check version literals in ignore reasons and fixtures — CONFIRMED

**Disposition: Fixed.** The `pg_render` aliases `THUMBS_VERSION` and `THUMB_OUTPUT_VERSION` are removed. Every use names `pg_model::versions::` directly, and the assert moved to `crates/model/src/versions.rs`, which now states that owners do not re-export their constants.

writeback lib.rs:800-806 builds its writer with the literal version 1 and cap 1 `<<` 20, while thumbs lib.rs:493-545 uses `pg_render::THUMBS_VERSION` and 1 `<<` 30. The ignore reason in pairs.rs:246-247 is a separate, documented reason.

### T-019 [medium] Proptest case counts and API spelling differ — CONFIRMED

**Disposition: Fixed.** Proptest blocks now use the default case count unless a block's cost needs fewer, and the `vec` and `option` strategies are spelled `prop::` as the other test files spell them. In `crates/container/src/lib.rs` the explicit `ProptestConfig::with_cases(200)` (below the default of 256, with no reason given) is removed, so that block runs the default 256, and `proptest::collection::vec`/`option::of` become `prop::collection::vec`/`prop::option::of` via `use proptest::prelude::prop`. `app/tests/listing_laws.rs` keeps its 12 cases, which its comment justifies (each case opens a catalog). Behaviour change: the container property runs 256 cases instead of 200.

Proptest case counts differ: `listing_laws.rs:30` uses 12 cases, container lib.rs:1295 uses 200, and `search_laws.rs` uses the default. Spelling also differs: container uses proptest::collection::vec and `search_laws` uses prop::collection::vec.

### T-020 [medium] Skip and ignore mechanisms differ across tests — CONFIRMED

**Disposition: No change.** The three skip mechanisms are intended.

Skip mechanisms differ: exiftool lib.rs:288-290 returns None and `PG_REQUIRE_TOOLS` makes it a failure, `pg_testkit::skip` only prints (testing.rs:83-84), codec decode.rs:434 is #[ignore] behind an env var, dedup join.rs:2900 is #[ignore] with env vars, and writeback.rs:127-128 returns early when ExifTool is absent.

### T-021 [low] Case-name spelling in `run_folder` second argument mixes pg- prefix and hyphen or snake forms — CONFIRMED

**Disposition: Fixed.** The second argument of every `run_folder`/`run_folder_with` call is now kebab-case with no `pg-` prefix: the scratch root already names the crate, so the prefix repeated it. About 40 names changed across about 25 files, including `pg-pipeline-*` in `jobs/src/pipeline.rs`, `pg-layout` in `library/src/layout.rs`, `pg-drive-of` in `platform/src/drive.rs`, the snake forms `cut_removed`, `crop_tags` and `crop_tags_cut` in `library/tests/crop.rs` and `crop_tags.rs`, and `folder("pg-crop-*")` in `crop/src/lib.rs`. Only the names of test folders on disk change. The `pg-` package names and the `pg-video` file name in `video/src/ffi/io.rs` (not a test folder) are untouched.

Case names mix the pg- prefix, hyphens and `snake_case`: pipeline.rs:947 uses pg-pipeline, pipeline.rs:1172 uses pipeline-gather, thumbcache `other_process.rs:48` uses other-process, transaction.rs:201 uses kebab-case, and library layout.rs:183 uses pg-layout. The cited crop lib.rs:450 is a doc comment, not a `run_folder` name.

### T-022 [low] Serialising static lock declared in several test binaries — CONFIRMED

**Disposition: Fixed.** `ONE_PASS` is now declared once, as `pub static ONE_PASS: Mutex<()>` in `crates/testkit/src/lib.rs`, with the reason written there. `library/tests/borders.rs`, `library/tests/pairs.rs` and the `thumbs/src/lib.rs` test module import it instead of declaring their own (and drop the now-unused `Mutex` import in the first two). No behaviour change: a Rust `static` is per test binary either way, so each binary still serialises its own passes.

The same serialising static, static `ONE_PASS`: `Mutex<()>`, is declared in library/tests/borders.rs:19, library/tests/pairs.rs:19 and thumbs/src/lib.rs:448, with near-identical comments.

### T-023 [low] Test naming and doc comment styles vary — CONFIRMED

**Disposition: Fixed.** Test docs are // comments.

Test doc style varies: analyze.rs:219 puts a /// doc on a test, app.rs:227 and images.rs:388 and others use // line comments above tests, tally.rs:200 has no doc, and names vary between sentences (video.rs:134 is videos, thumbcache lib.rs:595 is eviction).

### T-024 [low] Stale or wrong test doc comments — CONFIRMED

**Disposition: Fixed.** Four doc comments now match their code: the duplicate stacked doc on `type Tree` in `crates/scan/src/lib.rs` is removed (the first doc stays); the 200 000-file write-lock sentence that sat above `a_listing_that_outlasts_the_lease_keeps_it` moved onto its own test, `no_write_transaction_of_a_200_000_file_walk_holds_the_lock_long`; the thumbcache doc now says 400 entries (twenty batches of twenty, which is what the loop writes) instead of "a few hundred"; and `docs/reference/NOTIFICATIONS_DESIGN.md`'s implementation-order list is labelled as the record of the build order, not open items, since its stages were not checked against the code here. No test logic changed.

scan lib.rs:1345-1347 has two stacked doc comments on type Tree describing the same fake tree. thumbcache lib.rs:880 says a few hundred entries and the loop writes 400. scan lib.rs:2129-2131 has a doc about a 200 000-file walk placed above a different test. `NOTIFICATIONS_DESIGN.md:355` is a stale list. The stacked duplicate is clearly wrong.

### T-025 [low] Assertions on exact counts and message text couple tests to data — CONFIRMED

**Disposition: Fixed.** The count assertions carry messages.

Tests pin exact counts and text: codecs.rs:467 asserts files.len() == 72, notice.rs:1538 sets KINDS = 50, `rules_reference.rs:87` asserts 2033 rows, query.rs:520 asserts SortKey count 14, and `test_benches.py:49` asserts an exact error message.

### T-026 [low] Platform gate style differs between attribute, crate-level and runtime checks — CONFIRMED

**Disposition: No change.** Each platform gate form fits its scope.

Platform gates use four styles: `crop_folders.rs:4` has a crate-level #![cfg(windows)], dir.rs:707 and others use #[cfg(windows)] on the item, file.rs:347 uses a runtime cfg!(windows), and refusal.rs:208 uses #[cfg(all(test, windows))].

### T-027 [low] Hand-rolled pseudo-random generators in tests — CONFIRMED

**Disposition: Fixed.** Every test generator is `pg_testkit::Rng` (SplitMix64 in `pg-testrng`), including the codec's convert and the analysis's borders stats and pdq tests, and the bench's pairing draws. Two exceptions stay, documented: the hash mixer in `analysis/src/borders.rs` (a hash, not a stream) and the QML generator in `tst_gridrow.qml`, which cannot use Rust.

Hand-rolled generators: browse/read/tests.rs:452 defines struct Rng with xorshift seeds, thumbcache lib.rs:1007 uses xorshift with a fixed seed, and desktop `tests/qml/tst_gridrow.qml:344` uses an LCG. catalog writers.rs:1858 has a SplitMix64 generator. The cited container lib.rs:239 is a length parse, not a generator.

### T-028 [low] Known-gap statements in plans about test coverage — PARTIAL

**Disposition: Fixed.** `LANGUAGES.md` and `NIGHTLY_AND_BUILD_FLAGS_PLAN.md` say `cargo deny` is its own recipe, not part of `just lint`.

The plan's coverage gaps are real: LANGUAGES.md:130 says the pseudo scenarios run but are not compared, and `REFACTOR_LEDGER.md:90` (F-34) says only doctest = false is set. But LANGUAGES.md:129 lists cargo deny in just lint, which lint.py does not run (see DOC-001). eval/jpeg-decoders/linux.py:176-177 runs only clippy and fmt, as stated. Mixed: most gaps are accurate, one is stale.

### T-029 [low] Ignored test reason and invocation name different crates — PARTIAL

**Disposition: No change.** The wording is loose, not wrong.

`compares_real_jpegs` is in crates/codec/src/decode.rs:438, and its invocation (cargo test -p pg-codec) is correct. `JPEG_DECODERS.md:226` calls it the app's ignored test. The app is the whole product, so the wording is loose rather than wrong.

## concurrency-unsafe

### CU-001 [medium] Lease heartbeat cadence: plan says 5 s thread, code renews every 20 s in write transactions — PARTIAL

**Disposition: Fixed.** `BACKGROUND_JOB_AUDIT_FIXES.md` notes the heartbeat thread is not built. The code renews the lease every 20 s.

`BACKGROUND_JOB_AUDIT_FIXES.md:82` describes a heartbeat thread that beats every 5 s. scan lib.rs:205 has `RENEW_EVERY` = 20 s, scan lib.rs:45 renews with each batch and every 20 s, and catalog lease.rs:21 writes the heartbeat as the first statement of each write transaction. The code documents a different mechanism. Whether the plan is the decided design or a proposal was not settled, so the plan is stale against the code.

### CU-002 [medium] Cancel and stop flags use Relaxed in some places and SeqCst or Release/Acquire in others — CONFIRMED

**Disposition: Fixed.** The cancel flag in `crates/app/src/reader.rs` (`stop`, the take-next-job reset, and the cancel at :243), the progress-handler and test cancel stores and loads in `crates/catalog/src/core.rs`, and the `suspended` flag in `crates/thumbcache/src/lib.rs` (`set_suspended`, `suspended`) now all use `Ordering::SeqCst`, the ordering the jobs, import and transaction stop flags already use. The flags carry no data of their own, so SeqCst is the one ordering a reviewer can check against the rest of the workspace; the Relaxed to SeqCst change is a stronger ordering with no practical cost for a stop signal. Behaviour is unchanged apart from the stronger ordering.

app reader.rs:150, :170 and :243 store the cancel flag with Relaxed. catalog core.rs:304 loads its progress-handler flag with Relaxed. transaction.rs:76 and jobs control.rs:342 use SeqCst, import pairs.rs:127 uses SeqCst, and thumbcache lib.rs:129 stores Release and reads Acquire (:134). The same kind of stop flag uses three orderings.

### CU-003 [medium] Lifecycle opens write transactions directly, bypassing `write_transaction` and its busy retry — CONFIRMED

**Disposition: Fixed.** The two `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` calls in `crates/catalog/src/lifecycle.rs` (`set_mark_i64` and `set_mark`; the audit's :112 and :168 have drifted to :123 and :181) now call `write_transaction(conn)`, so the mark writes take the busy retry and the write-lock timing like every other catalog write. This is the proper choice because `write_transaction` is the one place that owns that policy, and the `TransactionBehavior` import became unused and was removed. Behaviour change: a mark write that finds the database busy now waits and retries instead of failing at once.

lifecycle.rs:112 and :168 open a transaction with `Transaction::new_unchecked(conn`, Immediate), which skips `write_transaction`. `write_transaction` (transaction.rs:87-97) retries on DatabaseBusy, so these two writes have no busy retry.

### CU-004 [medium] Pre-check outside the write lock, then write in a second transaction — PARTIAL

**Disposition: Fixed.** The `clear_failures` doc says a failure recorded between the check and the transaction is newer than these results, so it stays.

The read-then-write pattern is real: inbox.rs:142 checks before `write_transaction` at :145, ledger.rs:189 checks before the write at :192, and pair.rs:393 probes outside the transaction. lifecycle.rs:108 reads outside, but :169 checks again inside the transaction, so that one is safe. inbox.rs and ledger.rs were not confirmed to re-check inside.

### CU-005 [medium] `CLIPPY_LEDGER` unsafe-lint policy is not implemented; `NOTIFICATIONS_DESIGN` says no forbid exists — CONFIRMED

**Disposition: Deferred.** The unsafe policy (forbid or deny, per-item expect) is not applied; it needs a clippy listing of the unsafe items.

`CLIPPY_LEDGER.md:153` sets the `unsafe_code` policy (forbid where there is no unsafe, deny with per-item expect elsewhere). Cargo.toml has no `unsafe_code` lint and no crate has `forbid(unsafe_code`). The plan is not implemented. `NOTIFICATIONS_DESIGN.md:363` states no forbid exists, which matches the code.

### CU-006 [medium] Three docs give three unsafe-site totals with different units — CONFIRMED

**Disposition: Fixed.** Each figure now states its unit. `CLIPPY_LEDGER.md` calls its 472 "lint hits (compiler diagnostics, not source lines)" and points to the grep counts elsewhere. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md` (N1) gives source lines containing `unsafe` from a grep of the current tree: video 105, platform 80, desktop 70 (the old 104, 72 and 68 had drifted). `VALUE_PROVENANCE_IDEAS.md` gives 274 source lines under `crates/` from the same grep, replacing "about 245". Only these plans' wording, units and figures changed; no code was touched.

`CLIPPY_LEDGER.md:147` gives 472 unsafe sites (446 desktop, 20 platform). `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:153` gives unsafe lines by crate (video 104, platform 72, desktop 68). `VALUE_PROVENANCE_IDEAS.md:52` gives about 245 lines. The three use different units, and platform is 20 in one and 72 in another with no unit stated.

### CU-007 [medium] `GUI_THREAD_PLAN` describes an unsafe block in pg-catalog that the crate source does not contain — CONFIRMED

**Disposition: Fixed.** `GUI_THREAD_PLAN.md` is a plan, not started, so the block it describes is one Part 1 would add, not one that exists; the text now says so ("one documented `unsafe` block that this plan would add to `pg-catalog` (no `unsafe` exists in that crate today)"). The Cargo.toml cite is corrected to `Cargo.toml:70`, where `undocumented_unsafe_blocks` is (`:65` is `unreachable_pub`, now at :66). No crate source changed, since the plan has not been started.

A grep for unsafe in crates/catalog/src returns no file. `GUI_THREAD_PLAN.md:238` says one documented unsafe block exists in pg-catalog, so the plan describes code that is not there. `GUI_THREAD_PLAN.md:1719` cites Cargo.toml:65 for `undocumented_unsafe_blocks`, but the lint is at Cargo.toml:69 and :65 is `unreachable_pub`.

### CU-008 [medium] Two channel libraries used for the same job across the workspace — CONFIRMED

**Disposition: Fixed.** The two libraries do different jobs: `crates/jobs/src/pipeline.rs` clones its receivers across reader and worker threads (`raw_rx.clone()`, `res_rx`), which std's single-consumer `Receiver` cannot do, so it keeps `crossbeam-channel`. The audit's "control.rs" is not a crossbeam user (it uses `std::sync::mpsc` only). I documented the split where the choice is made: a comment above the import in `pipeline.rs` and an inline note on the `crossbeam-channel` line in the workspace `Cargo.toml`. Converting the pipeline to std would need `Arc<Mutex<Receiver>>`, which adds lock contention to a hot, measured path, so the proper choice is to keep the library and make its scope explicit. No behaviour change.

crossbeam-channel is used in two files (jobs pipeline.rs:21 and control.rs area, plus its manifest), while 14 files use std::sync::mpsc (for example jobs control.rs:785-787). The workspace mixes two channel libraries for the same worker-to-caller handoff. The cli lib.rs:971 cite is a blank line, so the cli example is off.

### CU-009 [medium] C++ cross-thread flag is a plain bool while sibling bridges use std::atomic — CONFIRMED

**Disposition: Fixed.** `FrameSink::State::open` is a `std::atomic<bool>`, read and written with explicit load and store.

`desktop/cpp/frame_sink.h:26` declares bool open = true as a plain bool, and `frame_sink.cpp:28-29` writes it from close(), which runs on the sink's thread, while the posted lambda reads it on the app thread. providers.cpp:34 uses `std::atomic<bool>` cancelled for the same cross-thread job, and screenshot.cpp:134 uses std::atomic for `last_frame_ms`.

### CU-010 [medium] Benchmark cleanup removes every `thumbs_*` folder, while its comment says only this process's — CONFIRMED

**Disposition: Fixed.** In `eval/bench_thumbs.py`, `main()` now makes one run folder (`bench_thumbs_*` under `CACHE_DIR/tmp`), `Pool` gives every worker that folder through an initializer (`_start_run`), and `_tmp()` makes each process's `thumbs_*` folders inside it. The end-of-run cleanup removes only that run folder, so another bench run or tool sharing `CACHE_DIR/tmp` is left alone. This is the proper choice because the run folder names the scope in the code instead of relying on a name glob. Behaviour change: none for this run's output; a `thumbs_*` folder outside a run is no longer deleted.

`bench_thumbs.py:377` globs `thumbs_*` under `CACHE_DIR/tmp` and removes every match with rmtree. The comment on the same line says this process's and the workers' folders. The glob takes all of them, so the comment describes a narrower cleanup than the code does.

### CU-011 [low] Unnamed production threads, where most spawn sites use Builder::name — CONFIRMED

**Disposition: Fixed.** The only unnamed production threads were the two stderr printers in `crates/cli/src/report.rs`, `thumb_events` and `with_progress`; they now use `thread::Builder::new().name(...)` ("thumbnail printer" and "progress printer"), as the app and desktop spawn sites do. The other unnamed spawns the grep finds are inside `#[cfg(test)]` modules and are left alone. `Builder::spawn` returns a `Result` where `thread::spawn` panicked, so each call takes `.expect` under a function-level `#[expect(clippy::expect_used)]` with a reason, because these functions return a join handle or a value and cannot return the error; the behaviour on a failed spawn is the same panic as before. Behaviour change: none beyond the thread names.

Production threads are unnamed in places: cli report.rs:23 and :78 use std::thread::spawn with no name. Across crates there are 28 thread::spawn calls against 17 Builder::new() uses, so naming is common but not universal. The cited cli lib.rs:972 is a `run_folder` call, not a spawn.

### CU-012 [low] Mutex poisoning recovered with two closure forms; one test module unwraps — CONFIRMED

**Disposition: Fixed.** Every poison recovery now uses the `unwrap_or_else(PoisonError::into_inner)` form the rest of the workspace uses: the closure forms at `crates/app/src/yandex.rs` (the `shared.lock()` helper) and `crates/thumbcache/src/writer.rs` (the `state.lock()` in `ThumbCache`'s writer queue) are rewritten to it. In the test module of `crates/thumbs/src/lib.rs`, the `PANIC_ON` and `STOP_ON` statics were set with `.lock().unwrap()`; they now use the same form as the module's other lock sites. Converting the one form to the other is the proper choice because `PoisonError::into_inner` is the form most sites already use and reads as a deliberate decision. Behaviour is unchanged: each poisoned lock is still used as it is. Other `lock().unwrap()` calls in test code across the workspace are outside this finding and were left as they are.

Poison recovery uses `unwrap_or_else(|poisoned|` `poisoned.into_inner(`)) in app yandex.rs:270 and thumbcache writer.rs:95, and `unwrap_or_else(PoisonError::into_inner`) in the desktop. thumbs lib.rs:486 unwraps `PANIC_ON.lock(`) in its test module, while :457 uses `PoisonError::into_inner`.

### CU-013 [low] Desktop defines the same poison-tolerant lock helper in three files — CONFIRMED

**Disposition: Fixed.** When this pass reached it, the tree already had one helper: `crates/desktop/src/pump.rs` defines `lock` (the generic `Pending<P>` form, with its comment), and `log_model.rs` and `notices.rs` import it with `use crate::pump::{Pending, lock}`. The copies the audit cites at `log_model.rs:59-61` and `notices.rs:222-224` are gone, and no other `fn lock` exists in `crates/desktop/src`. No change was made in this pass; the single helper is the proper form.

The same poison-tolerant lock helper body appears in desktop `log_model.rs:59-61` and notices.rs:222-224, and pump.rs:28-30 is a generic variant with a comment. Three copies of one helper.

### CU-014 [low] Thread-start failure handled two ways in the Yandex fetch pipeline — CONFIRMED

**Disposition: Fixed.** The two strategies answer different jobs, so each stays, and the reason is now written where a reader meets it. The image fetchers in `crates/app/src/yandex/images.rs` are optional parallelism: a fetcher that cannot start leaves its results queued, and `get` fetches them on demand, so the failure needs no report (the comment at the spawn now says so). `Search::spawn_worker` in `crates/app/src/yandex.rs` runs a search or a save, which has no fallback, so it clears `busy` and the status, announces the change and returns the error (its doc now contrasts the two). Unifying them would either report a failure the user cannot act on or drop a search silently, so the proper choice is the documented split. Documentation only; no behaviour change.

Yandex images.rs:108-110 handles a thread that cannot start by leaving its results queued and decrementing the fetcher count. yandex.rs:394-396 `spawn_worker` clears busy and status and returns the error. Two strategies for the same failure.

### CU-015 [low] Counters and flags in jobs/pipeline.rs mix Relaxed and SeqCst — CONFIRMED

**Disposition: Fixed.** The file now states one rule, in comments, and the code follows it: the `Clock` accumulator (`fetch_add` in `add`, Relaxed) and the reader's index claim (`next.fetch_add`, Relaxed) are Relaxed, because only atomicity matters for them; the `sent`, `dropped`, `failed` and `stop` values in the pass function stay SeqCst, because each is set by one thread and read by another or after the scope joins. The SeqCst sites were already consistent, so the change is the documented split, not a change of ordering. The audit's :242 and :272 cites were off (they are the priority call and a break); the sites are at the `Clock` and the `next`/`sent` declarations. No behaviour change.

jobs pipeline.rs mixes orderings: the clock adds and reads use Relaxed (:134, :138), the reader index uses Relaxed (:245), and the sent, dropped, stop and failed counters use SeqCst (:275, :301-302, :367-368, :456-490). The cited :242 is a priority call and :272 a break, so those two cites are off.

### CU-016 [low] Hash-container ban is enforced only in two crates' clippy.toml — REFUTED

The hash-container ban is in the root clippy.toml:14-15 as well as crates/client/clippy.toml:13-14 and crates/model/clippy.toml:12-13. Crates with no own clippy.toml inherit the root ban, so it is not enforced in only two crates. scan lib.rs:923 relies on it, consistent with that.

### CU-017 [low] Process-wide PATH, environment and DLL search mutated without restore — CONFIRMED

**Disposition: Fixed.** Each of the four cited sites now restores its state: `eval/accel.py` `_ort_session` puts PATH back in a `finally` around the session creation (the TensorRT DLLs are needed only while ONNX Runtime loads its providers; the audit's :295 is now :304); `eval/jpeg-encoder/test_encoder.cpp` gets a `PathRestore` guard that `test6_quality` holds, so its `_putenv_s` ends with the scope; `tools/profiling/callgraph/symbolize.py` gets `Symbols.close()` (SymCleanup, then `SetDllDirectoryW(None)`), called from a `finally` in `main`, and the audit's :130 is now :125; `tools/linux.py` `head_run` restores `PG_HEAD_SHA` (or removes it) after the container run. Restoring at the end of each scope is the proper choice because each change is needed only for the call it wraps. Not changed: the `os.environ["PATH"]` setup in `eval/jpeg-decoders/win_env.py` and `eval/png-decoders/win_build.py`, which prepare a whole build run and are not in the audit's cites. Behaviour change: the process-wide state is undone at the end of the scope.

Process-wide state is changed without restore: eval/accel.py:295 prepends `TRT10_DIR` to os.environ PATH, `test_encoder.cpp:979` calls `_putenv_s` to prepend the DLL folder to PATH, symbolize.py:130 calls SetDllDirectoryW with no restore, and tools/linux.py:290 sets os.environ `PG_HEAD_SHA` and never removes it.

### CU-018 [low] Pairing scripts restore probe environment variables inconsistently — CONFIRMED

**Disposition: Fixed.** The pairing folder already had one convention in `oracle.ps1`, `few_sweep.ps1` and `keep.ps1` (the last line removes the `PG_PROBE_*` variables the script set); `sweep.ps1` and `peak_mem.ps1` did not. Both now end with `Remove-Item Env:PG_PROBE_KEEP_OWED, Env:PG_PROBE_SLICE, Env:PG_PROBE_CACHE_KIB, Env:PG_PROBE_WORKERS -ErrorAction SilentlyContinue`, the variables they set, so all five scripts restore the environment the same way. Matching the existing convention is the proper choice; a `try/finally` would also cover an interrupted run, and that is a possible follow-up, not a defect of this finding. Behaviour change: the probe variables are no longer left in the caller's shell after `sweep.ps1` or `peak_mem.ps1` finish. Note that `keep.ps1`'s line sits after its `finally`, as it did before, and its audit cite :127 is now :123.

keep.ps1:127 removes its probe variables at script end. sweep.ps1:43, :48 and :49 set or remove per run, and no end-of-script cleanup exists. `peak_mem.ps1:39` sets `PG_PROBE_WORKERS` and its end removes only the work folder. The three scripts restore the environment in three ways.

### CU-019 [low] Pointer provenance API differs for the same job of rebuilding an OS handle — CONFIRMED

**Disposition: Fixed.** `crates/platform/src/picker.rs` (`run`) now rebuilds the owner HWND with `std::ptr::without_provenance_mut`, as `crates/platform/src/process.rs` (the job handle, from `job.addr()`) and the `file.rs` test already do. The owner arrives as a plain `usize` from the desktop's `main_window_handle()`, so no provenance was ever exposed for it and `with_exposed_provenance_mut` was the wrong pair. The audit's cite `testing/tests.rs:330` is past that file's end; the matching test site is `file.rs:338`, which already uses the same API. Behaviour is unchanged: the address is the same, and the value is never dereferenced by Rust.

platform picker.rs:215 rebuilds the owner HWND with `std::ptr::with_exposed_provenance_mut`, and process.rs:129 rebuilds the job handle with `without_provenance_mut`. Two APIs for the same job of turning a kernel handle back into a value. The testing/tests.rs:330 cite is beyond the file's end.

### CU-020 [low] Return values of queue, join and `begin_move` calls discarded — CONFIRMED

**Disposition: Fixed.** Three kinds of discard, each handled by its reason. The printer joins in `crates/cli/src/lib.rs` (five sites) and `report.rs` now go through a new `report::join_printer`, which re-raises a printer's panic with `resume_unwind` instead of dropping the join error, since a panicked printer is a bug. The `begin_move_rows` results in `crates/desktop/src/lane_model.rs` and `notices.rs` are kept in a `moved` binding with a `debug_assert!`, because the code's own comment says Qt always takes an upward move; a refusal now shows in debug builds without changing release behaviour. The queue discards in `desktop/src/shell.rs` and `notices.rs` (the audit's :341 and :520, now :329 and :506) already carry a comment saying a refused queue means the window is gone, so they are intentional and were left alone. Behaviour change: a panic in a CLI printer thread now reaches the caller.

desktop shell.rs:341 and notices.rs:520 discard queue results with let `_`, `lane_model.rs:317` discards the bool from `begin_move_rows`, and cli lib.rs:300, :387, :417, :444, :538 and report.rs:87 discard printer.join() results. The cli cite at :87 is a match, but the discards are real.

### CU-021 [low] Unsafe functions in the SIMD kernel have no Safety section; others do — CONFIRMED

**Disposition: Fixed.** `scan_popcnt` and `scan_avx2` in `crates/dedup/src/join.rs` now carry `/// # Safety` sections, in the form `crates/desktop/src/animation.rs` uses: each states the CPU features the caller must have, which `best_kernel` detects before choosing the kernel (the call sites' SAFETY comments already say so). The audit's :1423 and :1435 are now :1424 and :1436 before the edit. Documentation only; no code or behaviour change.

dedup join.rs:1423 (`scan_popcnt`) and :1435 (`scan_avx2`) are unsafe fns with no # Safety section, while their call sites carry SAFETY comments. desktop animation.rs:43-44 documents # Safety on `set_sink`.

### CU-022 [low] Perf capture flags restored without a drop guard — CONFIRMED

**Disposition: Fixed.** `crates/perf/src/lib.rs` `capture` now holds a `RestoreCapture` drop guard that puts `ENABLED` and `CAPTURE_HERE` back, the same shape as `AbortOnDrop` in `crates/import/src/pairs.rs`. The guard is dropped explicitly right after `f()` returns, so the restore still happens before the records are taken, as before, and a panic in `f` now restores the flags during unwinding. This is the proper choice because the guard is the existing pattern for this job. Behaviour change: a panicking capture no longer leaves the flags set for the rest of the thread or process; a normal capture is unchanged.

perf lib.rs:453-457 sets ENABLED and `CAPTURE_HERE`, runs f(), and restores them with no drop guard, so a panic in f skips the restore. import pairs.rs:120 uses AbortOnDrop, which restores on drop, as the contrast.

### CU-023 [low] Atomic-write temp-file naming differs across eval and tools — CONFIRMED

**Disposition: Fixed.** One naming rule now holds for every write-then-rename and download-then-rename in `eval/` and `tools/`: a unique name built with `uuid.uuid4().hex` next to the target, `<name>.<uuid>.tmp` for writes and `<name>.<uuid>.partial` for downloads and copies. This is the form `eval/jpeg-decoders/encoders.py` already used, and it is the only one safe when two runs or threads write the same target. Changed: `eval/jpeg-decoders/encoders.py` (the fixed `.tmp` at about :127), `eval/plant.py`, `eval/viewer.py` (both writers; the thread-ident name is gone), `eval/ann/run.py` (`pairs_knn.csv`), `tools/profiling/bench.py` (the dead `rmtree` of a fixed `.partial` is removed, since the name is new each time), and `tools/setup_ffmpeg.py` (the download). The `uuid` imports were added where missing. Behaviour: an orphan temp file from a crashed run keeps its own name instead of being overwritten by the next run.

Atomic-write temp names differ: eval encoders.py:127-128 uses a fixed .tmp suffix, encoders.py:63 uses a uuid name, bench.py:58 uses .partial, and `setup_ffmpeg.py:37` documents a .partial download.

### CU-024 [low] Download integrity checked in one tool and not in another — CONFIRMED

**Disposition: Deferred.** The rustup-init SHA-256 pin needs network access.

The `tools/ci_setup.py` cite is at `tools/profiling/ci_setup.py`. There it fetches `https://sh.rustup.rs` with urlopen (:51) and runs it with no checksum. `setup_ffmpeg.py` verifies a SHA-256 before it installs the archive. The cite at `setup_ffmpeg.py:80` is `top_folder`, but the check exists in that file.

### CU-025 [low] Eval job tag uses Python hash(), which is salted per process — CONFIRMED

**Disposition: Fixed.** The job tag in `eval/ann/run.py` (the `tag` line in the CPU-method runner) now takes its number from `hashlib.sha256` of the same `json.dumps(cfg, sort_keys=True)` text, modulo 10**8 as before, so the tag is the same in every process. This is the proper fix because a digest of a canonical text is stable by definition, whereas `hash()` of a str is salted per process. Behaviour change: the tag values differ from the ones earlier runs wrote, so an existing run folder or cache keyed by an old tag is not found again and its job is run afresh.

eval/ann/run.py:166 builds a job tag from abs(hash(json.dumps(...))) % `10**8`. Python's str hash is salted per process, so the tag changes between runs and the cache or the folder name is not stable.

### CU-026 [low] CUDA blocking-sync flag set through two APIs, and one return value ignored — CONFIRMED

**Disposition: Fixed.** The ignored return value is already handled: `eval/jpeg-decoders/harness/dec_gpu.cpp` `gpu_init` (its `cudaSetDeviceFlags` call, not :49) checks the status and prints a warning, and `nvjpeg.py` checks its status through `ok`. The two APIs remain because the venvs differ: `nvjpeg.py` loads torch's cudart DLL, while `gpu.py` and `nvimgcodec_bench.py` have no torch and call the driver's `cuDevicePrimaryCtxSetFlags` in `nvcuda.dll`. I documented that split in the `nvjpeg.py` `blocking_sync` docstring, which is where a reader would wonder about it, rather than forcing one API that one venv cannot load. No behaviour change.

The blocking-sync flag is set two ways: `dec_gpu.cpp:49` uses cudaSetDeviceFlags and ignores its return value, nvjpeg.py:91 uses cudaSetDeviceFlags, and `nvimgcodec_bench.py:39` uses cuDevicePrimaryCtxSetFlags through the driver DLL.

### CU-027 [low] SQL function marked deterministic but reads shared mutable state — CONFIRMED

**Disposition: Fixed.** The `moved` SQL function in `tools/profiling/pairing/grow_catalog.py` is registered with `deterministic=False` (with a comment), since its mask `current` is rewritten before each copy and the same blob gives different results across copies; the audit's cite is at `tools/profiling/pairing/grow_catalog.py`, the path it gave without the `pairing/` folder does not exist. Declaring the function non-deterministic is the honest contract, and SQLite's constant folding does not apply here because the argument is a column. Behaviour: the same results, with the planner free to re-evaluate the call, which the copy loop's statements already do.

The cited file is at `tools/profiling/pairing/grow_catalog.py`. moved() at :121 is registered with deterministic=True and reads current, a bytearray that the copy loop mutates between copies (:115-116). The cite path `tools/profiling/grow_catalog.py` does not exist.

## api-docs

### AD-001 [high] eGPU `pixel_budget` default documented as 64 Mpx but coded as 64 Mi pixels — CONFIRMED

**Disposition: Fixed.** The code value `64ull << 20` in `eval/jpeg-encoder/encode_gpu.cu` (`c->budget`) is kept, because a power-of-two budget is the intended binary limit and changing it would alter sub-batching. The documentation was corrected to match: the `pixel_budget` comment in `eval/jpeg-encoder/encode_gpu.h` and the throughput-test description in `eval/jpeg-encoder/README.md` now say 64 Mi pixels (2^26 = 67,108,864). No behaviour change.

`encode_gpu.h:40` documents `pixel_budget` as 0 = 64 Mpx. `encode_gpu.cu:935` uses 64ull `<<` 20, which is 67,108,864 pixels, or 64 Mi pixels. The doc uses the decimal mega prefix and the code the binary one, a difference of about 4.9 percent.

### AD-002 [medium] Analyze scale rule (512x320 minimum) held in Rust and Python — CONFIRMED

**Disposition: Deferred.** The equality test between the Rust copy and eval/common.py is not added.

The analyze scale rule is held in two places: jpeg-decoders src/lib.rs:54 documents it and :78 codes it as (512, 320) or (320, 512). nvjpeg.py:204-206 restates the same rule and cites pg-analysis. A change to one needs a change to the other.

### AD-003 [medium] Unsafe fn without a # Safety section where neighbours have one — CONFIRMED

**Disposition: Fixed.** `unsafe fn new_video_player` in `crates/desktop/src/video_player_bridge.rs` now has a `# Safety` section stating that `player` is a live `QMediaPlayer` or null, matching the contract the callers `video_source.rs` (`attach`) and `video_source_bridge.rs` already document. The bridge's other unsafe declarations (`show_windows`, `reload_on_change`) use the same heading. Doc-only change; no behaviour change.

`video_player_bridge.rs:15` declares an unsafe fn `new_video_player` with a one-line doc and no # Safety section. `video_source.rs:23` and `live_reload.rs:26` document their unsafe contracts under # Safety, so the neighbours do have one.

### AD-004 [medium] Public types, traits and consts without an item doc where neighbours have one — CONFIRMED

**Disposition: Deferred.** The missing_docs lint is not enabled; the named types have docs.

Public types without an item doc include Pool (app pool.rs:7), Inbox (inbox.rs:372), JsonStore (store.rs:21), Catalog (catalog core.rs:37), Control (jobs control.rs:202) and Runner (library runner.rs:30), while neighbouring items in the same files carry docs.

### AD-005 [medium] Public functions and methods without docs where neighbours have them — CONFIRMED

**Disposition: Deferred.** The named functions have docs; the lint is not enabled.

Public functions beside documented ones are undocumented in many files: `set_paused` in app library.rs:230, new in pool.rs:13, details in catalog browse read details.rs:12, `finish_scan` in roots.rs:145, and `clear_stop` in jobs control.rs:349. The gap is spread through the crates.

### AD-006 [medium] qproperty and qinvokable doc coverage uneven across desktop bridges — CONFIRMED

**Disposition: Fixed.** Every cited gap is documented: the `count` and `revision` and the `viewed_*` qproperties in `crates/desktop/src/gallery_bridge.rs` (each taking its meaning from the backing `GalleryRust` field and the function that sets it), the four region qinvokables in `crates/desktop/src/viewport_bridge.rs` (`regionX`, `regionY`, `regionW`, `regionH`), and the `roots` qproperty in `crates/desktop/src/library_bridge.rs`. The `command_line_bridge.rs` pattern was followed. Doc comments only; no behaviour change.

`gallery_bridge.rs:41-79` has qproperty items count, revision, anchor and `viewed_*` without docs while items at 42-62 have them. `viewport_bridge.rs:86-100` has undocumented qinvokables, and `library_bridge.rs:18` has roots with no doc. `command_line_bridge.rs:19` documents every property, so the reference pattern exists.

### AD-007 [medium] Doc comments attached to the wrong item or duplicated — PARTIAL

**Disposition: Fixed.** The duplicate paragraphs are removed from the `crop.rs` module doc and the writeback test doc.

`library_bridge.rs:19` is directly above `kept_removals` (line 20), and its text (waits for an unfinished write) reads as that property, so the attachment is right and roots at line 18 has no doc. The claim that each doc is shifted one property off would need the property at line 22 to confirm, and that line was not read. The writeback test docs at :71-73 and the crop.rs:68-70 doc are real duplications of the same paragraph, which is the stronger part of the finding.

### AD-008 [medium] Python docstring and argparse description conventions diverge — PARTIAL

**Disposition: No change.** The three conventions are real, but most cited paths do not match.

`install_exiftool.py:46` takes the first paragraph of the docstring, `make_test_video.py:18` takes line 1 of the docstring split by lines, and cgtab.py:325 uses the whole docstring with a raw formatter. The three conventions are real. Most of the other cites (`bench_data`, perf-compare, `make_bench_jpeg`, `setup_ffmpeg`, `synthetic_library`) are at paths that do not match the cite, so they are not checkable as written.

### AD-009 [low] Public fields and variants with missing docs while siblings have them — CONFIRMED

**Disposition: Deferred.** The named fields have docs; the lint is not enabled.

Fields are undocumented beside documented ones: catalog fixture.rs:15 (File), inbox.rs:72-73 (Entry), inbox.rs:92-96 (Mark), ledger.rs:143 (FailedFile), model details.rs:13 (Details.size), model view.rs:15-16 (Listing), and the others listed. The gap is common across the model and catalog.

### AD-010 [low] Doc comments on public items name private machinery or other-crate variants — PARTIAL

**Disposition: Fixed.** The pub doc in `dedup/src/join.rs` no longer names the private `run_team`.

join.rs:516 mentions `run_team`, which the doc says is private; that is an internal name in a public doc. app error.rs:25-26 names client message variants, which the app is allowed to name because it depends on the client's text layer. Only the join doc is an internal-name leak, and the finding's second instance is an intended cross-layer reference.

### AD-011 [low] Visibility and re-export pattern varies for public surface — CONFIRMED

**Disposition: Deferred.** The visibility rule is an owner decision and is not applied.

Visibility and re-export patterns vary: dedup lib.rs:8-9 exposes join and `union_find` as pub mod with only UnionFind re-exported, client controller.rs:17-23 makes every submodule pub mod, catalog lease.rs:15 uses pub(crate) fields, pair/coverage.rs:8 has fully public fields, and metadata xmp.rs:32 is pub with pub(crate) fields. Each is a choice, but no single rule holds across the crates.

### AD-012 [low] Derive sets differ across public data types — CONFIRMED

**Disposition: Fixed.** Added the derives the fields allow: `FileSource` gains `Eq` (`crates/model/src/state.rs`; it cannot be `Copy` because `rules::Path` owns heap data), `Placeholder` gains `Clone, Copy, PartialEq, Eq, Debug` (same file, matching `FileState`), `ImportStats` gains `Clone, Copy, Debug, Default, PartialEq, Eq` (`crates/import/src/lib.rs`), `Cropped` gains `Clone, PartialEq, Eq, Debug` (`crates/edit/src/crop.rs`), and `Rgb` gains `Debug, PartialEq, Eq` (`crates/codec/src/pixels.rs`). `FileState` already had the reference set. Derives are additive; no existing impl conflicts (grepped), but a compile should confirm.

Derives differ across public data types: model state.rs:12 FileState derives Copy and Eq, state.rs:28 FileSource lacks Eq and Copy, state.rs:39 Placeholder derives nothing, import lib.rs:40 ImportStats derives nothing while hashes.rs:19 derives Clone, Copy and Debug, edit crop.rs:24 Cropped derives nothing, and codec pixels.rs:5 Rgb derives Clone and Default only.

### AD-013 [low] Return-value documentation style differs across write methods — CONFIRMED

**Disposition: Fixed.** No text changed. The crate's convention for a `bool` return is a closing "Whether …" sentence (about forty sites in `crates/catalog/src`), and both cited sites in `crates/catalog/src/writers.rs` already follow it; the per-method mentions in `crates/catalog/src/inbox.rs` each state that method's own result, not a repeat of the module doc. The finding is a wording nit with no reader effect, so rewording forty sites would be churn with no gain.

Return-value docs end differently: writers.rs:191 ends with a trailing fragment, Whether a row was written, and :436 has the same fragment. inbox.rs:100 states the bool meaning once in the module doc and again per method. Low severity.

### AD-014 [low] Public statics documented with plain comments; a heading without a blank line — CONFIRMED

**Disposition: Fixed.** The four public statics in `crates/render/src/maker.rs` (`DECODE`, `RESIZE`, `ENCODE`, `THUMBHASH`) now have `///` item docs: the shared description sits on `DECODE` and the other three point to it. In `crates/desktop/src/notices.rs` a blank `//!` line now separates the paragraph from the `## Where the window tells` heading, as the other headings in that module doc have. Comment-only change; no behaviour change.

maker.rs:20-22 documents the public statics DECODE, RESIZE, ENCODE and THUMBHASH with plain // line comments rather than rustdoc. notices.rs:45-46 has a ## heading directly after a paragraph with no blank //! line, unlike the other headings in that module doc.

### AD-015 [low] Mode lists in the two decoder harness tools are documented differently — PARTIAL

**Disposition: No change.** The claim text, restored from `e63aa037`, says both cites were missing and so the mode lists could not be compared. Both cites now resolve: the C++ harness is `eval/jpeg-decoders/harness/harness.cpp` (its modes are `list`, `time`, `repeat`, `throughput` and `dump`), and the Rust harness is `eval/jpeg-decoders/src/main.rs` (its modes are `time`, `compare` and `repeat`). The lists differ because the two are different tools, and each usage line names its own modes correctly.

### AD-016 [low] Script-header conventions: PowerShell help blocks and first-line verb form — CONFIRMED

**Disposition: No change.** The pairing convention and the top-level plain headers are intended.

Script headers differ: `few_sweep.ps1:2` and the other pairing scripts use .SYNOPSIS help blocks, while `set_owed.py:1` opens with an imperative docstring and keep.py:1 with a noun phrase, and `setup_ffmpeg.py:2` opens third person. Low severity.

### AD-017 [low] Duplicated boilerplate disclaimer across modgraph tools — CONFIRMED

**Disposition: Fixed.** The reason the three report tools are not gates (they match by name) is now written once, in the module docstring of `tools/modgraph/testcut.py`, the shared helper all three already import. `dupes.py`, `edges.py` and `api_surface.py` each keep a one-line "A report for a review (`just modgraph-report`), not a gate (see `testcut.py`)." Docstring text only; no behaviour change.

dupes.py:4, edges.py:6 and `api_surface.py:8` repeat the same boilerplate disclaimer, A report for a review, not a gate. The wording is copied verbatim into three modgraph tools.

### AD-018 [low] Doc states a caller contract the body does not enforce — CONFIRMED

**Disposition: Fixed.** The `mark_all_read` docs in `crates/catalog/src/inbox.rs` and `crates/app/src/inbox.rs` now state the contract as the caller's: the caller passes the entries the window displays, and nothing in the method checks them. The catalog has no record of what was displayed, so a check there would need new state; the docs now say so rather than implying a guarantee. Doc-only; no behaviour change.

inbox.rs:297 documents `mark_all_read` as marking the entries displayed and never one the window was not given. The body at :298-299 calls only `self.mark_read(ids`, now), and nothing in it checks what the window was given. The contract is stated and left to the caller.

### AD-019 [low] TestHooks.qml says options are read in the file but also says Rust reads them — CONFIRMED

**Disposition: Fixed.** The header of `crates/desktop/qml/TestHooks.qml` said "The options are read here", which contradicted the component's own comment that Rust reads them in `command_line.rs`. The header now says they are read in Rust and given to the component as typed properties, which is what the code does. Comment-only change in QML; nothing else in the file was touched.

TestHooks.qml:29 says The options are read here, while the comment at :64 says they are read by Rust in `command_line.rs` and given to this component as typed properties. The file's own header contradicts its later comment.

### AD-020 [low] Plan docs cite API shapes that differ from the code — PARTIAL

**Disposition: No change.** The plans mark their own API shapes as not verified.

The plans already mark their own API shapes as not verified against current code: `CATALOG_SIZE_PLAN.md:225`, `DECODE_ONCE_PLAN.md:380` and `BOUNDED_MEMORY_PLAN.md:121` each carry a not verified marker in the finding text. The divergence is expected for a plan, and the plans say so. The finding is accurate about the gap but not about a hidden error.
