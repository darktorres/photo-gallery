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

tools/lint.py has no cargo deny step (grep finds none). Justfile:134-137 says the deny recipe is not in lint, and ci.yml:305 runs it as its own step. crates/README.md:259 says just lint includes cargo-deny.

### DOC-002 [high] README says Windows only; CI runs a Linux job and Justfile documents Linux checks — CONFIRMED

README.md:11 says Windows only for now. ci.yml:222-223 has a Linux job for the whole workspace, and Justfile documents the Linux checks in the pg-dev container.

### DOC-003 [high] Crate README gives two different PDQ distance thresholds — REFUTED

Two thresholds serve two purposes. `MATCH_DISTANCE` is 31 (model pdq.rs:11), used by the pairing pass (README:352 says d `<=` 31). `GROUP_DISTANCE` is 22 (pdq.rs:21-28), used for grouping and the duplicates filter (README:427). Each README use matches its constant.

### DOC-004 [high] Frame sink thread docs contradict the code and each other — CONFIRMED

`frame_sink.h:20` says close() runs on the sink's thread. `frame_sink.rs:20-21` says close is called on the GUI thread, by Player's drop. `frame_sink_bridge.rs:23` says the sink's thread. The header and the Rust SAFETY note disagree.

### DOC-005 [high] `take_shared` doc says it waits; body returns an error — PARTIAL

lifecycle.rs:65 says `take_shared` waits for a change in progress. `pg_dbfile` lock.rs:55-61 does retry `LOCK_TRIES` times with a sleep, then returns None, which lifecycle.rs:69 maps to an error. The wait is bounded and then fails, so the doc is loose rather than wrong.

### DOC-006 [high] Undecodable files: ledger says no retry, failure.rs says retry — REFUTED

The cited catalog/src/failure.rs does not exist. The code that decides retry is thumbs/src/lib.rs:121-129: Failed::Undecodable gives Some(false), no retry, which matches ledger.rs:48. No file says Undecodable is retried.

### DOC-007 [high] Settings save failure documented as reported; code discards the result — CONFIRMED

settings.rs:15-17 says a failing save is a note on the app's bus and the window prints it. settings.rs:21 is let `_` = save.write(), which discards the error, so no note is posted.

### DOC-008 [high] Numeric values in doc comments differ from the code — CONFIRMED

makers.rs:1-2 says an idle maker is kept up to a few, but `KEEP_MAKERS` is 8 (makers.rs:10). rating.rs:4 and :20-33 say user ratings are 1-5, but Rating::new accepts 0 (stars `<=` MAX). The statement cache doc (16 default, 64 set) is accurate. The xmp and encode instances are mostly fine.

### DOC-009 [high] Perf callgrind doc Ir counts disagree with docs/perf/baseline.json for the same bench keys — PARTIAL

The callgrind md numbers differ from baseline.json (facets.all 93,262,634 vs 47,331,512). The md is from commit 1bb0390 and baseline.json was re-measured later (layout v7, per `REFACTOR_LEDGER`). Different commits are not a contradiction, but the md title still says 11 benches for a 23-entry file.

### DOC-010 [high] `NOTIFICATIONS_DESIGN` gives two retention rules for dismissed events — PARTIAL

`NOTIFICATIONS_DESIGN.md:16` overrides the 30-day rule: dismissed events are deleted, and the purge applies to dismissed states only. Line 261 says dismissed purged after 30 days in the retention bullet without saying states, so it reads as covering events too. Wording problem, not a code conflict.

### DOC-011 [high] THUMBNAILS.md names Lanczos3 as the thumbnail resampler; PROFILING says the path moved — CONFIRMED

THUMBNAILS.md:23 says Lanczos3 from 4x is the thumbnail path. codec/src/resize.rs:1 says Magic Kernel Sharp 2021 is the one resize for every pipeline (kernel.rs:1, 42). PROFILING.md:413 already notes the move, but THUMBNAILS.md did not follow it.

### DOC-012 [high] `WEB_CLIENT_DESIGN` says the desktop never writes the catalog after cutover; App::open runs Catalog::prepare — CONFIRMED

app.rs:62 calls `pg_library::open_catalog`, which at library lib.rs:27-29 calls Catalog::prepare, which can migrate. `WEB_CLIENT_DESIGN.md:1885` says the desktop never writes the catalog after cutover, so the design's premise is wrong.

### DOC-013 [high] Catalog queue.rs module doc names writer API functions and table that do not exist — CONFIRMED

queue.rs module doc names `next_ready`, `record_temp`, done, failed, `drop_gone` and `retry_copies`. The code has `next_write` (:133), `record_write_temp` (:229), `write_done` (:245), `write_failed` (:304), `drop_gone_write` (:315) and `retry_write_copies` (:327).

### DOC-014 [high] `MULTI_DRIVE_LANES_PLAN` status says two checks were not run; its ledger says they passed — CONFIRMED

`MULTI_DRIVE_LANES_PLAN.md:21` says just check-linux and just qmltest were not run. The same plan's ledger at :427 records check-linux passing and :429 records qmltest 108 passed.

### DOC-015 [high] `DECODE_ONCE_PLAN` body contradicts its own superseded notes — PARTIAL

`DECODE_ONCE_PLAN.md:44` and :953 are dated superseded notes above bodies that still describe the old design (thumbnails in Features, DCT scaling). The plan keeps superseded text under a marker, so the body is history. Not checked line by line, so partial.

### DOC-016 [medium] crates/README cites a RUSTDOCFLAGS cargo doc step that CI does not run — CONFIRMED

crates/README.md:149 says the RUSTDOCFLAGS cargo doc command is in .github/workflows/ci.yml. ci.yml has no RUSTDOCFLAGS. The flag is in tools/lint.py:32 (`DOC_FLAGS`), which just lint runs.

### DOC-017 [medium] CI comment says every cargo command takes --locked; several Justfile recipes do not — CONFIRMED

ci.yml:298 says every cargo command takes --locked. Justfile:48-50 cargo build, :61-63 cargo run --release, and :65-67 cargo run have no --locked.

### DOC-018 [medium] Plan docs give different counts of stored PDQ orientations — CONFIRMED

PLAN.md:134 says 8 dihedral variants are stored per file. Code has ORIENTATIONS = 4 (model pdq.rs:8) and `BLOB_BYTES` = 4 x 32 (pdq.rs:118). `OPEN_ITEMS.md:80` records the change as resolved 2026-11-18.

### DOC-019 [medium] Doc comments name modules, crates or constants that are not where they are cited — CONFIRMED

catalog lib.rs:10 names `thumbs::pending_in`. There is no thumbs module in catalog. The function is `pg_thumbcache::pending_in` (thumbcache lib.rs:39 re-export). The other cited instance, inbox.rs:35 naming `pg_app::inbox::LISTED`, is correct (app inbox.rs:128).

### DOC-020 [medium] Same concept named differently in docs and code — CONFIRMED

app inbox.rs:67 says whether a clear has work is decided by `replace_notices`. The clear path calls `clear_notices` (app inbox.rs:665), so the doc names the wrong catalog method for clear.

### DOC-021 [medium] Writers doc says statements are cached on the connection; several writers prepare uncached — CONFIRMED

writers.rs:29-31 says statements are cached on the connection (`prepare_cached`). writers.rs:377, :378-379, :720 and :689 use uncached tx.prepare, so the doc overstates the caching.

### DOC-022 [medium] baseline.sql lists different `file_failures` pass codes than the triggers write — CONFIRMED

baseline.sql:123 lists pass codes thumbs and borders only. The triggers at :359 write pass 'import', and `ledger_ack` at :135 lists import too. The comment omits import.

### DOC-023 [medium] Facet ordering documented differently in catalog and client, and not pinned by the tests it cites — PARTIAL

catalog facets.rs:26 says types are most-files-first, but facets.rs:97 sorts types by MediaType order. Exts are sorted by count (:99). The client tests/facets.rs:19-20 does enumerate ALL and compare it with `from_index`, so the order is pinned; the not-pinned claim is wrong.

### DOC-024 [medium] Test doc comments claim more than the assertions check — CONFIRMED

library/tests/video.rs:192 says red at the top after a catalog turn, but the assertion at :199-200 puts BLUE at (40,20), the top, and RED at (40,140), the bottom. The comment is wrong. `listing_laws` doc says 'for every' where proptest samples folder and recursion; that is looser. borders.rs:90-91 counts the unreadable file in stats.failed while the comment says it is no failure; the ledger part holds.

### DOC-025 [medium] thumbs and thumbcache docs disagree with their code — CONFIRMED

thumbs/src/lib.rs:121-123 says Some(true) means tried again by the next pass. recorded() at :124-129 returns only Some(false) (Panicked, Undecodable) or None (Unreadable, Other). The Some(true) case is documented and never produced.

### DOC-026 [medium] Docs disagree on which formats lose metadata and which are re-encoded on crop — PARTIAL

Metadata claims agree across edit/lib.rs:2-4, crop lib.rs:117 and edit crop.rs:9-11, and reencoded sets metadata false (crop.rs:95-98). crop.rs:9-11 says other TIFFs go through image, but crop.rs:66 routes every Tiff to reencoded. Whether encode::cropped sends other TIFFs to image was not read. The cited crop lib.rs:139 is unrelated to metadata.

### DOC-027 [medium] Model crate doc says it has no dependencies; its Cargo.toml lists four — CONFIRMED

analysis.rs:136-137 says the model has no dependencies and copies `MAX_LEN` by hand for that reason. model/Cargo.toml:12-16 lists arrayvec, blake3, serde and strum, plus optional specta.

### DOC-028 [medium] writeback docs name notice tags and queue types that differ from code — REFUTED

Tags match the code: notice.rs:975 maps WriteFailed to write-failed and :977 WriteDropped to write-dropped. writeback lib.rs:3 says writes come from the `file_writes` queue, which is the table that catalog queue.rs reads through Job values, so that is one concept with two names, not a contradiction.

### DOC-029 [medium] dbfile judge emits a difference kind the schema doc does not list — CONFIRMED

dbfile judge.rs:138 emits Difference::Changed with kind header. The schema.rs:28-29 doc says the kinds are table, index, trigger, view and rows. Difference is the same enum (schema.rs:31), so header is an undocumented kind.

### DOC-030 [medium] log record parse doc does not match the continuation rule — CONFIRMED

log record.rs:73-75 says any other line continues the record before it. record.rs:91 continues only a plain record with no t, and record.rs:98 starts a new plain record after a JSON record. The doc overstates the rule.

### DOC-031 [medium] import hashes doc says failures are stored as the import stores them; the formatting differs — CONFIRMED

hashes.rs:93 says failures are stored as the import stores them. hashes.rs:121 stores format!("{e:#}"), while import lib.rs:179 stores `Cause::of(&e).to_string(`). The two formats differ.

### DOC-032 [medium] Orientation and resize docs state contracts the code does not enforce — PARTIAL

resize.rs:13-15 says `resize_region` fails when the region is outside the image. `resize_region` (:79-91) clamps the region with max(1.0) and does no bounds check, and run (:98-108) checks only the pixel buffer. Whether `fast_image_resize` rejects an out-of-bounds crop was not confirmed (registry search timed out). The orientation.rs:1 claim is accurate, since both read and write use the same private `exif_block`.

### DOC-033 [medium] Grid rating and model rating docs disagree with the file and catalog — REFUTED

grid.rs:30 gives the user's rating: None when unrated, with no catalog row. rating.rs:1-2 gives the file's own rating (`file_ratings`, 0 to 5) as a separate thing. The two do not conflict. The cited model lib.rs:41 is a module list and does not discuss ratings.

### DOC-034 [medium] Model notice doc says library paths are RulePaths; the field is a String — PARTIAL

notice.rs:9 says library paths are RulePaths, exact. FileFailure.path is a String, and its doc at notice.rs:303 says its path as the user sees it, so the String is deliberate display text. The module doc does not mention that exception.

### DOC-035 [medium] Lock doc says the name is removed on drop; Unix HeldLock has no Drop — PARTIAL

The cited platform/src/lock.rs does not exist. The lock is in platform/src/testing/lock.rs. Its doc at :25-26 says a lock from `create_for_delete` or `take_for_delete` removes its name when it goes, linking HeldLock::delete. No Drop impl exists in that file, so when it goes is loose wording; delete is the only removal.

### DOC-036 [medium] video ffi doc says bindgen does not emit `AV_NOPTS_VALUE`; build allowlists it — PARTIAL

ffi.rs:38 says bindgen does not emit `AV_NOPTS_VALUE`, and ffi.rs:39 defines it by hand. build.rs:38 still allowlists `AV_NOPTS_VALUE` in VARS. If bindgen cannot parse the macro, the allowlist entry is dead. No code conflict was shown, and the bindgen output was not generated here.

### DOC-037 [medium] GPU device creation cost stated with different metrics — CONFIRMED

Creation cost appears as ~0.3 s in `gpu_device.h:26` and `gpu_device_bridge.rs:15`, and as ~0.9 G instructions in `gpu_device.cpp:1` and `gpu_device.rs:2`. The two are different metrics for one operation, stated in different units with no link.

### DOC-038 [medium] Budget, batch and rate figures in jobs and import docs disagree with constants — REFUTED

pipeline.rs:46-48 sets `BUFFER_PER_SLOT` to 12 MiB, and budget() is (readers + workers) x 12 MiB. With 8 readers and 12 workers that is 240 MiB, which the doc gives as 240 MB (only the MB/MiB label differs). control.rs:22-23 gives about 60 marks; the test at :702 marks once a second, so 32 marks is consistent with that sampling.

### DOC-039 [medium] Removed or renamed items referenced in plan documents — PARTIAL

`BACKGROUND_JOB_AUDIT_FIXES.md:248` says NOTIFICATIONS.md is removed, which the doc itself states. Cited line numbers in the migration plans are stale (schema.rs:804, :956, lifecycle.rs:1229 not all checked). The removed-file references are historical notes.

### DOC-040 [medium] Plan and schema docs disagree on the migration route — CONFIRMED

`BACKGROUND_JOB_AUDIT_FIXES.md:19` says migrations, not a BASELINE edit, because the baseline is frozen (schema.rs:28-31). The code at schema.rs:28-31 says before the first release the baseline is edited in place, and MIGRATIONS is empty. `BOUNDED_MEMORY_PLAN.md:18` and :25 and `CATALOG_SIZE_PLAN.md:125` still plan one BASELINE edit.

### DOC-041 [medium] `BOUNDED_MEMORY_PLAN` commit policy contradicts the commit rule in the background plan — CONFIRMED

`BOUNDED_MEMORY_PLAN.md:144` says phases 1-8 are one commit each. `BACKGROUND_JOB_AUDIT_FIXES.md:25` sets the standing rule: commit only when asked. The two plans conflict on whether commits happen without a request.

### DOC-042 [medium] Catalog size figures disagree across DATABASE.md, `CATALOG_SIZE_PLAN` and budgets — CONFIRMED

DATABASE.md:26 gives `pdq_dihedral` as 256 B per file. `CATALOG_SIZE_PLAN.md:31` gives 128 B, and the code is 4 x 32 = 128 (pdq.rs:118). `order_key` is 121 B in DATABASE.md:27 and 117 B in `CATALOG_SIZE_PLAN.md:11`. DATABASE.md:101 says the app ships SQLite 3.54.0, `CATALOG_SIZE_PLAN.md:357` says 3.54.1 (libsqlite3-sys 0.39.1, Cargo.lock).

### DOC-043 [medium] SQLite page cache size differs between `OPEN_ITEMS` and the scan plan — CONFIRMED

schema.rs:114-117 sets PRAGMA `cache_size` = -8192 (8 MB) in SETTINGS, as `OPEN_ITEMS.md:57` says. `SCAN_OPTIMIZATION_PLAN.md:451` and :802 still say each connection has a 256 MB cache from SETTINGS, which is the old premise.

### DOC-044 [medium] Notification plan decision count and cache event counts disagree within the docs — CONFIRMED

`NOTIFICATIONS_PLAN.md:10` says D1 to D12 are unanswered, and :24 says waiting on a person covers D1 to D8. `NOTIFICATIONS_DESIGN.md:13` says the cache has seven events, and :230 says five. thumbcache types.rs Event has at least seven variants (lines 68-85), so :13 is right and :230 is stale.

### DOC-045 [medium] `NOTIFICATIONS_DESIGN` contradicts the code and itself on derivation and schema — PARTIAL

`NOTIFICATIONS_DESIGN.md:241` says code() comes from strum::IntoStaticStr. :94 overrides that to the ids! table, and notice.rs has no IntoStaticStr, so the code follows :94. The `TO_3` SQL at :251 (scope TEXT, AFTER DELETE trigger) is stale, but :29 overrides it, and `folder_failures.root` has ON DELETE CASCADE (baseline.sql:164).

### DOC-046 [medium] `Pdq_index.rs` cited as current code after it was removed — CONFIRMED

`crates/dedup/src/pdq_index.rs` does not exist (dedup/src has join.rs, lib.rs, `union_find.rs`). `UROCISSA_COMPARISON.md:120` cites it as current, `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:137` cites `pdq_index.rs:323`, and crates/README.md still mentions it. `OPEN_ITEMS.md:63` records it as gone.

### DOC-047 [medium] Review and plan status lines contradict their own counts or ledger — CONFIRMED

`MULTI_DRIVE_LANES_PLAN.md:14` says tests/lanes.rs has 8 tests, and :400 says 7 of 12 are written. crates/library/tests/lanes.rs has 9 #[test] attributes. The plan contradicts itself and the file. The LANGUAGES.md HEAD dates are dated history, not a contradiction.

### DOC-048 [medium] LANGUAGES plan links to documents that do not exist; compare-shots script name is stale — REFUTED

README.md, site/index.html and docs/plans/PLAN.md all exist at HEAD. LANGUAGES.md:155 already marks the compare-shots.ps1 reference stale, and `compare_shots.py:4` records that the name changed.

### DOC-049 [medium] WSL status described inconsistently across reference docs and the Docker README — CONFIRMED

`REFACTOR_DESIGN.md:20` says WSL is retired and wsl-check.sh is gone. No `tools/wsl*` or wsl.sh file exists. `JPEG_DECODERS.md:119` says wall times are medians on WSL via wsl.sh. docker/README.md:71 still calls WSL the one exception for S-17.7.

### DOC-050 [medium] `REFACTOR_LEDGER` describes the baseline differently from baseline.json; callgrind note bench count differs — CONFIRMED

docs/perf/baseline.json has 23 entries, all pg-bench keys. `REFACTOR_LEDGER.md:131` says it has 11 results, and the callgrind md title says the 11 benches. The count is stale in both.

### DOC-051 [medium] Web client design docs disagree with `WEB_CLIENT.md` on settled points — CONFIRMED

`WEB_CLIENT.md:272`, :455 and :564 say the private CA certificate is already configured. `WEB_CLIENT_DESIGN.md:231` says the repo has no certificate or TLS server code, and no CA tooling exists in crates. The design doc's contradiction list is correct.

### DOC-052 [medium] `WEB_CLIENT.md` keeps superseded listing and snapshot figures — CONFIRMED

`WEB_CLIENT_RECONCILED.md:1031` voids the four-listing cap and the 256 MB budget. `WEB_CLIENT.md:212` and :221 still state the 56-byte figure and the four-snapshot cap. DESIGN.md:1859 still calls 256 MB a proposal, so the superseded numbers persist.

### DOC-053 [medium] Reference docs disagree on JPEG encoder and thumbnail numbers — PARTIAL

`JPEG_ENCODERS.md:13-14` says ImageReduce runs mozjpeg in `JCP_FASTEST` mode. :111-112 says its encode is libvips 8.18.7 jpegsave. libvips jpegsave can use mozjpeg when built with it, and the doc does not say which. The naming conflict is real, but not proven as an error.

### DOC-054 [medium] Reference index and cross-references are incomplete or point at the wrong file — CONFIRMED

`REFERENCES_LEDGER.md:78` names `docs/THUMBNAIL_RESAMPLING.md` and :44 names `docs/TIER2_ANN_BAKEOFF_PLAN.md`. Neither exists; the real files are under docs/plans/. :140 and :133 use the correct plans/ paths, so the ledger is inconsistent.

### DOC-055 [medium] Eval harness docs disagree with the code they describe (method, precision and timing) — CONFIRMED

`bench_decode.py:14` says files are read once before timing, so it measures decode, not disk. The timed loop at :167-169 calls decode(), which calls `read_bytes` at :57, so each timed call reads from disk. exact.py:3 says the ground truth is fp32, but `bench_ann.py:10-11` says the exact search, which is the ground truth, is fp16 with fp32 accumulation, and run.py:154 uses fp16=True.

### DOC-056 [medium] Eval scripts cite Rust files and crates that do not exist in the workspace — CONFIRMED

crates/core does not exist. The analyze decode rule is in crates/analysis/src/analyze.rs. eval/common.py:70 and eval/embed.py:19 cite crates/core/src/analyze.rs. The imgload.py:44 cite (crates/analysis/src/borders.rs) does exist.

### DOC-057 [medium] PNG decoder README gives a timing budget and dump file names the code does not use — PARTIAL

The PNG README:67 advises at least 5 s for inflate-stream numbers. The harness defaults (harness.cpp:90, main.rs:80) are 1.0 s, which the advice overrides with --secs. That is advice versus default, not a contradiction. README:60 names DIR/NAME.raw for the app's dump, which matches main.rs:120, but the C harness writes `NAME.<decoder>.raw` (harness.cpp:482).

### DOC-058 [medium] JPEG decoder harness docs contradict harness code on output layout, warm-up and command — PARTIAL

`dec_wuffs.cpp:1` says decodes to RGBA, but it sets Image::RGBX (:45) and names the format `RGBA_NONPREMUL` (:21). The layout is a 4-byte pixel with no real alpha, so the word is loose. harness.cpp:14-15 documents the GPU warm-up of 5 (:147, :233), so the Rust-versus-C difference is stated. The restart.py cite is about the Rust binary, as its docstring says.

### DOC-059 [medium] Profiling tool docs name the wrong build script and removed timing paths — CONFIRMED

profile.ps1:29 says run build.sh first, while profile.py:58 says build.py. build.sh is still in the tree, though build.py:3 says it replaced build.sh. `scan_timing.py:5-6` says renumbered was removed on 2026-11-14, and :113 still runs it.

### DOC-060 [medium] Peak memory script documents peak commit but samples a different counter — CONFIRMED

`peak_mem.ps1:3` says peak commit (private bytes). :47 samples PeakPagedMemorySize64, which is not the private-bytes counter, and :52 reports it as peak commit.

### DOC-061 [medium] tools/README describes the static Qt build and `compare_shots` options incorrectly — CONFIRMED

tools/README.md:57 says the static-qt patches are applied by the static Qt build. No file in tools or the Justfile names app-build.patch or pg-video.patch, and `build_static_qt.py` has no patch step (its functions are extract, configure, build, install). The `compare_shots` --wait option is present at `compare_shots.py:653`; the README row is not a full option list.

### DOC-062 [medium] eval verifier and test-set counts disagree across sections — CONFIRMED

VERIFIER.md gives cross-group pairs as 4637 (:10, :24), 4628 (:161), 4632 (:216, :397, :428), and k-NN as 8450 (:11) and 8,344 (:61, :220, :329) in the same doc. `TEST_SET.md:27` says 678 groups and 702 pairs after fixes, and :36 says 679 and 698 now, with the old line not marked superseded.

### DOC-063 [medium] Desktop QML comments disagree with code, keys or sibling files — CONFIRMED

Main.qml:697 LibraryPanel refocuses with grid.takeFocus(), while Main.qml:813 and :847 use win.refocus() for Log and Inbox. `video_source.h:1` names `src/video_player.rs` for the device, but MediaView.qml:55 and the attach impl (`video_source.rs:23`) name `video_source.rs`. The turn keys (R, Shift+R) in RatingBar.qml:2 were not found in keymap.rs.

### DOC-064 [medium] Grid revision doc does not cover refreshes that change entries — CONFIRMED

grid.rs:138-139 says revision counts list shown and item edited. `refresh_turned` (grid.rs:405-431) changes entries and never assigns self.revision, so the doc misses a change that the revision check depends on.

### DOC-065 [medium] Temporary file naming described differently in the app and the code — PARTIAL

save.rs:47-48 says repository temporaries are dot-names (.pg-write, .pg-crop), which matches the queue temps (queue.rs:562 .b.jpg.turns.pg-write). store.rs:115 writes `<name>.tmp`, a different writer that does not claim dot-names. Two conventions across different writers, not a doc contradiction.

### DOC-066 [medium] Versions doc names the per-crate constants and `pg_library::versions` instead of `pg_model::versions` — PARTIAL

types.rs:277 names `analyze::IMPORT_VERSION`, `borders::BORDERS_VERSION` and `pg_render::THUMBS_VERSION`. Those are the owning crates' re-exports (render lib.rs:14). The code imports from `pg_model::versions` (types.rs:291), as the registry doc at versions.rs:1-4 prescribes. Naming the re-export is acceptable.

### DOC-067 [medium] Version comment names IMPORT as 1 literally in fixture doc — PARTIAL

fixture/raw.rs:180 says rows are at import version 1. versions.rs:13 has `IMPORT_VERSION` = 1 today, and raw.rs binds the constant (:229). The doc is right now but written as a literal that will go stale on a bump.

### DOC-068 [medium] Library runner and scan doc details disagree with code — CONFIRMED

scans.rs:19 says conditions are kept under (root, code). passes.rs:108 replaces them all under one Key::all(scan-note, root), so there is no per-code key. scan/lib.rs:270 names `replaced_ids` and temps as fields, and neither exists in that file.

### DOC-069 [low] Writeback test comment says PNG while the test queues a JPEG — PARTIAL

writeback.rs:1013 says a PNG is rewritten without ExifTool. The test that follows queues the healthy b.png and the poisoned a.jpg (:1014). The comment describes one file and omits the JPEG. Incomplete, not wrong.

### DOC-070 [low] CutStats.unsupported is documented but never set — REFUTED

crop lib.rs:167-169 says unsupported stays 0 and is kept for the report, because every still format is cut now. The field is declared and never set, which is what the doc says.

### DOC-071 [low] Video FFI loader path cited differently in two docs — CONFIRMED

crates/README.md:240 cites crates/video/src/ffi/load.rs, which exists and is where the DLLs are loaded. Justfile:69-70 says pg-video loads its DLLs by full path in crates/video/src/ffi.rs, the module root, so the Justfile points at the wrong file.

### DOC-072 [low] Measured timings in doc comments that the code or benches do not reproduce — PARTIAL

The timing claims (modgraph 115 to 149 s, the HDD order speedup, the cold-read figures, the 0.8 s pairs pass) are measurements recorded in comments. They cannot be reproduced from the repo here, so they are unverified, not contradicted.

### DOC-073 [low] Crop, edit and cut docs use inconsistent function and crate paths — PARTIAL

encode.rs:1-3 names `pg_edit::crop::cut`. The crop module exists in crates/edit, but the cut function was not confirmed in that file. `WEBP_LOSSY_QUALITY` = 90 (encode.rs:96) matches its doc. The path claim is only partly checked.

### DOC-074 [low] Stale or undefined references in the probe and bench doc comments — REFUTED

`pairs_probe.rs:18` lists the prepare field, and the printed line uses t.joins.prepare (:100), which is the same field. `tools/profiling/pairing/set_owed.py` and `tools/profiling/synthetic_library.py` both exist at the cited paths.

### DOC-075 [low] Eval and ANN docs give wrong CLI forms and pair set lists — CONFIRMED

eval/verify.py:3 gives the usage as labeled, `cross_groups` or knn. `PAIR_SETS` at :99-105 also has listed, which eval/ann/run.py:445-447 uses. The usage line is stale. The viewer.py glob point is loose wording.

### DOC-076 [low] Eval JPEG encoder and decoder comments name the wrong counts, stages and venvs — PARTIAL

README:16 says `test_encoder.cpp` has tests 1 to 7, and README:97 documents test 8 as a diagnostic outside pass or fail, so the two are consistent. The `encode_gpu.cu` header list was read only through stage 6. enum Stage (:543) has `ST_COPY`, and the header's full list was not confirmed.

### DOC-077 [low] Tools doc comments name the wrong OS scope, paths or Docker wording — CONFIRMED

`lint_qml.py:2` says just lint-qml on Linux and macOS, while :14 says every OS. :55 says the output is `build/pg-desktop-<hash>/`, but :57 globs `target/*/build`. `fuzz_smoke.py:3` says the fuzzed tree is a git archive of HEAD, but linux.py:187 uses git clone. `few_sweep.ps1:9` says busy runs are not left out, while sweep.ps1:8-9 and :66 leave them out.

### DOC-078 [low] Site and tools docs point at files that are missing or mislabelled — PARTIAL

The hero-icon width is set to 160px in style.css:73, which overrides the 256 attributes in index.html:20, so the rendered size is the CSS. The screenshot comment (index.html:35) is commented out on purpose. pages.yml:39-41 does copy the icon and mascots, as index.html:3 says. The `tools/test_ci_files.py` docstring was not checked.

### DOC-079 [low] Desktop Rust doc comments name wrong symbols, counts or rows — CONFIRMED

notices.rs:73 gives the providers.rs row as (4), but providers.rs has three `pg_log::warn` calls. The `resolver_table.golden` path resolves under crates/model/src/notice/, so that instance is fine.

### DOC-080 [low] QML viewer and gallery comments with stale values or misplaced text — CONFIRMED

ui.ftl:91-92: the log view comment is followed by the inbox comment, with no log entry under it. YandexPreview.qml:26-28: the results comment sits above the dpr property. Main.qml:203 says the cut dialog reports as SaveViewDialog, which is correct. `MAX_ZOOM` = 32 is in client viewport.rs:6, so the zoom comment is right. Gallery and `RATING_SYMBOLS` references resolve. The state.rs query-versus-media MediaType claim was not confirmed.

### DOC-081 [low] Client text and controller doc comments with stale details — PARTIAL

controller/size.rs:1 says pause is timed by deadlines.rs, which exists in crates/client/src/controller, so the file is in the repo. The other instances (tests facets, start.rs:2 on the session's last file) were not contradicted. Some were not read in full.

### DOC-082 [low] Yandex and shell state names differ from the code — CONFIRMED

yandex/state.rs:37-40 has results and frames. The frames doc says indexed by hits, but no hits field exists in that struct. The name is stale.

### DOC-083 [low] Start gate doc on the shown field matches the code (no contrast found) — REFUTED

start.rs:44 says the shown flag means a list was shown and its placement is not yet reported. That matches the flag's use. The list flag's doc at :42 is accurate. No contradiction found.

### DOC-084 [low] Container and codec docs claim behaviour the functions do not have — CONFIRMED

container lib.rs:209 documents `add_extended_chunk` as returning the APP1 data, but the signature (:210) returns nothing and pushes into chunks. lib.rs:3-4 says each walk yields byte ranges, but `png_chunks` (:233) yields Chunk values with slice data. decode.rs:263-264 and encode.rs:431 are accurate.

### DOC-085 [low] Model and metadata doc comments name the wrong module, range or count — CONFIRMED

model state.rs:33 says `poster_ms` is 0 before import, but the field is `Option<i64>`, so no import is None. metadata exif.rs:10 says the format rules are kamadak-exif's `DateTime::from_ascii`, and grep finds no call to `from_ascii` in metadata. The detection black/white bounds (`<=` 40, `>=` 215) match detection.rs:82-88. The square tolerance was not located.

### DOC-086 [low] Third-party product named in project docs and comments — CONFIRMED

ImageRater is named in code docs: app views.rs:2, prefs.rs:29, model view.rs:28 and exiftool lib.rs:129-130. It is a style and naming choice, not a contradiction. Low severity.

### DOC-087 [low] versions.rs `key_version` mark naming rule does not hold for `KEY_ALGO_VERSION` — CONFIRMED

versions.rs:1-3 says each name is the upper-cased mark. `KEY_ALGO_VERSION` (:47-50) is marked `key_version`, so the stated rule has an exception that the header does not list, though its own doc explains it.

### DOC-088 [low] Plan D9 and phase cross-references in code docs point at plans that do not give those numbers — PARTIAL

versions.rs:32 and :44 cite plan D9 for the cache and thumbnail rules. thumbcache schema.rs:113 cites plan D9 and notes no plan file is in the shard. Whether D9 is numbered that way in the plans was not checked.

### DOC-089 [low] Notices reference and catalog docs describe index names and fields that do not match — PARTIAL

lookup.rs:195 names `files_present`, `files_to_import` and `analysis_failed`. Those indexes exist in baseline.sql (:215, :247, :253). Whether status() reads `files_present` was not confirmed. The `kept_events` value (1000) and the rebuild.rs module-doc list were not fully checked.

### DOC-090 [low] Runner, coordinator and lane docs name probes and tools that do not match code — PARTIAL

inner.rs:39 cites `lane::BUSY_RETRY`, which lane.rs:26 defines, so that cite is right. coordinator.rs:107-108 says cuts are queued once confirmed via `Runner::cut_borders`; `cut_borders` was not found in library/src beyond that doc. The other lane and events cites were not fully checked.

### DOC-091 [low] Stub and test docs claim parity with Rust that is not tested in the cited place — PARTIAL

The locale files (cli.ftl:1, text.ftl:1, ui.ftl:3) say a test checks that the ids match the Rust lists. Client src references the ftl files, but the named parity tests were not located in full. Unverified as stated.

### DOC-092 [low] Callgrind and perf narrative numbers in reference docs disagree with each other — PARTIAL

PROFILING.md:320 says 189,518 files (2026-10-31 snapshot), :507 says 189,520 (the pairing data), `REFACTOR_MEASUREMENTS.md:120` says 189,519. The dates differ, so the library may have changed between runs. They are not proven to describe the same folder on the same day.

### DOC-093 [low] Search and crop limits in the desktop app do not match documented bounds — PARTIAL

shell/state.rs:19 says the row bounds are `pg_model::size::ROW_HEIGHT`, which matches RowHeight::MIN and MAX built from it (size.rs:10-23), as recorded for A-027. The rotate.rs:1 claim that R and Shift+R turn images was not found in keymap.rs, so that instance is unverified.

## duplication

### D-001 [high] Folder subtree membership computed two ways — CONFIRMED

removal.rs:351 `folder_ids_under` filters every folder in Rust with `rules.is_under` (rules-aware, so case-insensitive under Windows rules). types.rs:90 `folders_under` selects by byte range through `under_root!`. Two selections of the same subtree with different matching rules.

### D-002 [medium] Lane planning rules split between coordinator and lanes — CONFIRMED

coordinator.rs:484 and lanes.rs:91 both apply the single-lane rule (one lane gets VolumeSet::All, otherwise Only(volumes)). coordinator.rs:858 `drive_label` and lanes.rs:82 both name a group by its first mounted root's drive. The lane rules are split across two modules.

### D-003 [medium] CPU count and budget defaults repeated — CONFIRMED

`available_parallelism().map_or(4`, ...) appears at pipeline.rs:53, coordinator.rs:173 and lanes.rs:129. coordinator.rs:178 repeats Options::budget(8, cpus) with the reader count of 8 that pipeline.rs:55 sets in Options::new.

### D-004 [medium] Pause clock bookkeeping implemented twice — CONFIRMED

control.rs:82 `Running::set_paused` and control.rs:228 PauseClock::set implement the same pause bookkeeping with the same (true, None) and repeat-changes-nothing match arms.

### D-005 [medium] Decode scale rule re-derived in codec and video — REFUTED

video lib.rs:272 calls `MinSize::fit_scale` for the Fit case, so the video crate delegates to the shared rule and does not re-derive it. `scale_for` only adds the clamp to 1 and the other MinSize variants.

### D-006 [medium] Resize computes fit by hand instead of `fit_scale` — CONFIRMED

resize.rs:33 computes side / long by hand, while decode.rs:79-80 says `fit_scale` is the one fit the resizer uses. `to_pixels` is defined at decode.rs:259 and again at resize.rs:125 with the same body.

### D-007 [medium] Atomic file replace implemented twice with different temp naming — CONFIRMED

yandex/save.rs:63 `write_new` replaces by rename with a dot-name temp (see PF-011). app/src/store.rs:117 `write_atomically` writes `<name>.tmp`, syncs and renames. `REFACTOR_LEDGER.md:73` (F-22) records the private helper as a shortcut to move to pg-platform, still open. The cite store.rs:118 is off by one; the function is at :117.

### D-008 [medium] Failure classification repeated in thumbs — CONFIRMED

thumbs lib.rs:112 Failed::of maps MakeError to Failed. placeholders.rs:164 `is_unreadable` and placeholders.rs:177 `is_recorded` each repeat the MakeError and panic checks to classify the same failure.

### D-009 [medium] io::Error downcast sites to be replaced by Unreadable — CONFIRMED

io::Error downcasts appear at borders lib.rs:97 and import hashes.rs:114, and placeholders.rs:171 falls back to `downcast_ref::<io::Error>`. `IMPORT_FAILURES_PLAN.md:198` names these as the copies to replace with Unreadable. The cite thumbs lib.rs:113 is a MakeError match, not an io::Error downcast, so that one cite is wrong.

### D-010 [medium] Import batch size 500 mirrored across crates — CONFIRMED

const BATCH: usize = 500 is defined at import hashes.rs:17, import lib.rs:25 and borders lib.rs:22. The bench catalog.rs:174 `IMPORT_BATCH` = 500 mirrors pg-import's batch by hand, so a change in the import is not picked up by the bench.

### D-011 [medium] Thumbnail and row height default 220 in two types — CONFIRMED

RowHeight::DEFAULT = RowHeight(220) at model size.rs:25 and ThumbHeight::DEFAULT = ThumbHeight(220) at size.rs:69. ThumbHeight's doc says it is the default row height at scale 1, so the two defaults are one fact written twice.

### D-012 [medium] Verbatim-prefix stripping in three platform modules — CONFIRMED

The verbatim prefix is stripped or defined in four places: platform names.rs:127 `without_verbatim`, native.rs:265 `plain_in_place`, picker/shell.rs:33 `shell_path` with its own VERBATIM and UNC unit arrays (:34-35), and native.rs:185 with a VERBATIM constant of the same units.

### D-013 [medium] CreateFileW sharing modes differ across opens — PARTIAL

The CreateFileW sharing combinations differ: file.rs:37 uses READ|WRITE|DELETE, dir.rs:366 the same, drive.rs:293 READ|WRITE, and file.rs:89 READ alone. Each open has its own purpose, and no shared constant exists, but whether these are duplicates or deliberate per-open choices was not settled from the lines read.

### D-014 [medium] XMP namespace URI written twice — CONFIRMED

The XMP namespace is written as bytes with a NUL at container.rs:17 (`JPEG_XMP`) and as a &str at xmp.rs:13 (`NS_XMP`). Both are the same string in the same crate.

### D-015 [medium] BigTIFF magic rule in three places — CONFIRMED

The BigTIFF header rule (43, offset size 8, zero) is in model format.rs:53 `is_bigtiff` and metadata tiff.rs:43-45. codec encode.rs:316 has a test helper `is_big` matching the same header bytes.

### D-016 [medium] JPEG marker walk duplicated in container — CONFIRMED

container lib.rs:70 JpegSegments::next and lib.rs:138 `jpeg_header_end` both skip fill bytes and reject bad segment lengths with the same logic in two loops.

### D-017 [medium] YYYY-MM-DD parsed in three places — PARTIAL

model date.rs:9 `parse_iso_date` and date.rs:64 Day::parse both parse YYYY-MM-DD with range checks in the same file, which is a real duplication. metadata exif.rs:11 `exif_date` parses the colon format YYYY:MM:DD, a different format, so it is not a third copy of the same parse.

### D-018 [medium] Unchanged-source check written twice in crop — CONFIRMED

crop lib.rs:300 `cut_original` compares the file's (size, mtime) with the recorded state, and lib.rs:368 `write_copy` repeats the same (s.size, s.mtime) comparison. The bytes comparison in the cite was not seen; the size and mtime check is repeated.

### D-019 [medium] Width dispatch repeated in dedup join — CONFIRMED

dedup join.rs has four width dispatches over 1, 2 and so on: `first_pass` (:975), `second_pass` (:989), scan (:2031) and `scan_width` (:2043), each matching a width to a const-generic function.

### D-020 [medium] Catalog predicate present and not pending repeated — CONFIRMED

The predicate 'f.missing = 0 AND f.pending = 0' is written inline at work.rs:426 (OWED), :143 (`OWED_PLACEHOLDERS`), :526 (`owed_hashes`) and :565 (`has_owed_hashes`).

### D-021 [medium] `file_writes` and `scan_seen` predicates inline while macros exist — CONFIRMED

removal.rs:74 writes the `file_writes` NOT EXISTS inline, and roots.rs:184 and :204 write the `temp.scan_seen` NOT EXISTS inline. sql.rs:106 (`not_being_written!`) and sql.rs:113 (`not_seen!`) already exist as the shared forms.

### D-022 [medium] Overlap predicate written inline three times — CONFIRMED

The overlap test (same || `is_under` || `is_under`) is written inline at removal.rs:168, and again in `overlapping_marker` (:383) and `overlapping_root` (:391).

### D-023 [medium] `borders_version` lookup repeated — CONFIRMED

writers.rs:720 BordersWriter.mark selects value from meta WHERE key = '`borders_version'`, and `pass_mark_is_ours` at :821-822 reads the same row.

### D-024 [medium] Touch-after-analysis logic written twice — CONFIRMED

writers.rs:339 `touch_analysis` (test and fixture only) and writers.rs:429 `AnalysisWriter::touch_file` each do the same bump-and-touch of a file after an analysis row. The test helper says it mirrors the writer's.

### D-025 [medium] Retry statements repeated between ledger and lifecycle — CONFIRMED

ledger.rs:207-209 and lifecycle.rs:399-401 run the same `next_seq`, `failures_relisted` and DELETE FROM `file_failures` statements for the retry.

### D-026 [medium] Thumbs filter and `pending_borders` SQL copied into test helper — CONFIRMED

The thumbs-pass filter (NOT EXISTS `file_failures` ... pass = 'thumbs' ...) is written at work.rs:145, :463 and :654 (the test helper `borders_statement`), with the same body each time.

### D-027 [medium] VOLUMES statement shared with thumbcache by comment only — CONFIRMED

work.rs:19-21 says thumbcache's `catalog_volumes` is 'the same statement'. thumbcache reader.rs:123 defines `catalog_volumes` separately and reader.rs:74 calls it. The two copies of the SQL are linked only by that comment.

### D-028 [medium] Test catalog builders and versions helpers differ — CONFIRMED

Builders differ: pair.rs:582 catalog(rules) uses Versions::current(), `catalog/tests/other_process.rs:12` and rehearsal.rs:21 each define versions() wrapping Versions::current(). The cited app/src/inbox.rs:455 is clear(scope), not a catalog builder, so that cite is wrong.

### D-029 [medium] Writeback tests hard-code attempt cap 5 — CONFIRMED

writeback.rs:387 uses attempts() `>=` 5 and :404 asserts attempts() == 5, while queue.rs:32 defines `MAX_ATTEMPTS` = 5 once. The tests hard-code the value the constant defines.

### D-030 [medium] Facet kinds enumerated in four client places — PARTIAL

input.rs:19 choice, input.rs:31 `key_of`, facets.rs:43 choices and text.rs:84 `choice_label` each match on the facet or choice enum to produce a different result (a value, a key, a list, a label). Matching one enum in four functions is normal Rust; the enum itself is not duplicated. The count of six kinds was not checked.

### D-031 [medium] Tier names mapped in Rust and Fluent — CONFIRMED

client text/cli.rs:449-451 maps Tier::File and Tier::Pixels to the English strings 'file' and 'pixels' in Rust. cli.ftl has no tier words, so these English words are hard-coded beside a Fluent catalogue.

### D-032 [medium] Two different blues for selection in the desktop UI — CONFIRMED

The selection blue is #4aa3ff at GridCell.qml:133, #2f5f93 at Main.qml:31 (palette highlight), and #4a90e2 for the picked badge at Theme.qml:13. The cited InboxPanel.qml:152 is a Connections block and does not hold #2f5f93, so that cite is wrong. Two or three blues with no token.

### D-033 [medium] Facet and folder kinds as hand-typed integers — CONFIRMED

FilterBar.qml:48 declares facetTypes: 0 and the bar passes it on as a plain integer (:126, :157). `REFACTOR_LEDGER.md:122` (F-51) records the hand-typed integers for facet and folder kinds as open.

### D-034 [medium] Lock order named two ways in web client docs — PARTIAL

RECONCILED.md:582 gives the lock order as fence mutex, then device gates, then write transaction. DESIGN.md:369 says the registry lock serialises additions against the fence. The two may be the same lock under two names, or two locks; the text does not settle which, so the naming drift is real but its meaning is unclear.

### D-035 [medium] Sort ordering in two places in `DECODE_ONCE` plan — PARTIAL

`DECODE_ONCE_PLAN.md:873` says `compare_placed` is 'the existing `compare_keys` logic', so the plan treats the two as one rule, and :875 states the tie rule once. The `grid_sql` ORDER BY is a second written ordering in the plan, which is a planning-stage restatement and not an inconsistency by itself.

### D-036 [medium] Windows build scripts exist in cmd and py forms — CONFIRMED

eval/jpeg-decoders/win-env.cmd exists and parses nvidia-smi `compute_cap` at :29-30, and `win_env.py:1-5` says it was win-env.cmd, which win-build.cmd called. The .cmd is still in the tree beside the .py, so the same parse is kept in two languages.

### D-037 [medium] Exact k-NN sentinel differs between two implementations — CONFIRMED

eval/evaluate.py:31 uses -2.0 as the self-match sentinel, and eval/ann/exact.py:32 uses -inf for the same self-exclusion in the other k-NN implementation.

### D-038 [medium] `bench_ann` constants copied from the bake-off package — CONFIRMED

`eval/bench_ann.py:29` sets FLOOR, K = 0.4, 10 by hand, while eval/ann/data.py:19 defines FLOOR, K = 0.4, 10 with the comment naming the rule. BANDS at `bench_ann.py:194` differs from metrics.py:20 (see U-042).

### D-039 [medium] Library size 70e6 literal in five files — CONFIRMED

The literal 70e6 appears in `bench_thumbs.py:60` (`LIBRARY_SIZE`), `bench_models.py:42`, `bench_semantic.py:152`, `bench_hdd.py:242` and `bench_ann.py:350`, with no shared constant.

### D-040 [medium] sRGB transfer functions duplicated in Python and Rust — REFUTED

A search for the sRGB constants (0.04045, 0.0031308) across crates finds no Rust transfer function. The Rust counterpart named as crates/codec/src/linear.rs does not exist. The Python `_eotf` and `_oetf` in `eval/linear_resize.py` are the only copy, so there is no duplication.

### D-041 [medium] `DISK_MAX_SIDE` kept equal by comment — CONFIRMED

eval/accel.py:246 sets `DISK_MAX_SIDE` = 1024 with the comment '`verify.WORK_SIDE'`, and eval/verify.py:80 sets `WORK_SIDE` = 1024. They are kept equal by a comment, not by an import.

### D-042 [medium] Decoder registry in five places — CONFIRMED

The decoder list is written in `harness_linux.py:25-26` (`CPU_DECODERS` and `IR_DECODERS`), linux.py:133 (a separate list), table.py:16 (DECS) and the Rust enum at eval/jpeg-decoders/src/lib.rs:20-22. harness.cpp:65 does not exist, so that cite is dropped.

### D-043 [medium] Wuffs streaming constants repeated in three files — CONFIRMED

The wuffs streaming chunk constant 16 `<<` 10 is written at `eval/png-decoders/harness/dec_wuffs.cpp:180` (kInChunk) and harness.cpp:298 (kInChunk), in two C++ files. The cited harness/main.rs does not exist.

### D-044 [medium] libjpeg encoders use different error handling — CONFIRMED

`test_encoder.cpp:1135` `libjpeg_encode` uses a plain `jpeg_error_mgr` with no longjmp, while :493 `libjpeg_encode_rgb` uses a JerrMgr with setjmp in the same file, so the two encoders handle libjpeg errors differently.

### D-045 [medium] Viewer pair kinds listed twice — PARTIAL

`VERIFY_TABS` in viewer.html:160 is a subset of TABS (:148) naming three tabs (verify, `verify_cross`, `verify_knn`), not a copy of the same list. `PAIR_FILES` in viewer.py:34 keys the pair kinds, and TABS in viewer.html names tabs. Whether these overlap beyond the verify tabs was not checked.

### D-046 [medium] DCT draft size and denominator rule in four places — CONFIRMED

The DCT denominator rule with (512, 320) is written at eval/jpeg-decoders/gpu.py:47-49 and nvjpeg.py:203-205, and as `DRAFT_SIDES` at eval/embed.py:20 with a repeated note in common.py:70. `bench_decode.py:37` drafts to (320, 320), which differs from the analysis rule, and src/lib.rs:74 has a third denom.

### D-047 [medium] Dotenv loader written five times — CONFIRMED

A dotenv loader with a regex is in tools/pgtools.py:42 `load_env_file` and `tools/lint_qml.py:32` (same regex), and tools/qmltest.ps1:12-15 has its own copy. The cited symbolize.py, profile.ps1 and build.sh do not exist, so those three cites are dropped. The three real copies stand.

### D-048 [medium] Doctor implemented twice — CONFIRMED

doctor.ps1:55 and doctor.py:67 both implement the libclang search and the checks, in PowerShell and Python.

### D-049 [medium] callgraph build implemented twice — CONFIRMED

build.py:62 main compiles and links callgraph.dll, and build.sh (stale, DOC-059) repeats the same flag lists beside it. The two builds are kept side by side.

### D-050 [medium] LLVM default path computed three times — PARTIAL

The LLVM default is computed in build.py:66-68 and build.sh:22-24. The cgtab.py:78 cite is an unrelated callgrind line, not an LLVM default, so only two copies are real.

### D-051 [medium] Newest-version selection in three tools — CONFIRMED

build.py:24 newest() sorts version digits in order, and build.sh:31 uses sort -V to pick the newest clang include folder. The fork-updates.py:49 cite does not exist.

### D-052 [medium] FFmpeg fetch implemented in ps1 and py — REFUTED

tools/setup-ffmpeg.ps1 does not exist. The only FFmpeg fetch-and-check script is `tools/setup_ffmpeg.py`. tools/build-static-ffmpeg.ps1 is a build script, and its duplicated pins are covered by D-055.

### D-053 [medium] QML test staging differs between ps1 and py — CONFIRMED

qmltest.ps1:32 hard-codes the Ratings singleton by name, while qmltest.py:59 detects 'pragma Singleton' in the file. The two stagers decide what a singleton is in different ways.

### D-054 [medium] Toolset pin repeated in ps1 and py build scripts — CONFIRMED

The toolset pin 14.44.35207 is written in build-static-ffmpeg.ps1:24, build-jpeg-turbo.ps1:24 and tools/pgtools.py. The cited `build_static_ffmpeg.py` does not hold it, so that cite is wrong; the ps1 and pgtools copies are the duplicates.

### D-055 [medium] FFmpeg and jpeg-turbo pins duplicated in ps1 and py — CONFIRMED

FFmpeg commit, release and SDK pins are in build-static-ffmpeg.ps1:22-23 and `build_static_ffmpeg.py:33-35`, with the same values. The jpeg-turbo fork URL and commit are in build-jpeg-turbo.ps1:22-23 and `build_jpeg_turbo.py:31-32`.

### D-056 [medium] FFmpeg configure flags in two languages — CONFIRMED

`CONFIGURE_FLAGS` in `build_static_ffmpeg.py:43` and $ConfigureFlags in build-static-ffmpeg.ps1:31 list the same configure flags, with the same comment that each is the pilot's.

### D-057 [medium] System DLL allowlist in two places — CONFIRMED

`build_static_ffmpeg.py:57` `SYSTEM_DLL` regex and build-static-ffmpeg.ps1:194 non-system filter list the same prefixes (api-ms-win-, KERNEL32, USER32, bcrypt, VCRUNTIME140, MSVCP140).

### D-058 [medium] Static FFmpeg library list repeated — CONFIRMED

The five static FFmpeg libraries are listed in tools/static-qt/pg-video.patch:33 and as `STATIC_FFMPEG_LIBS` in app-build.patch:118, with the same names in the same order.

### D-059 [low] OS error-code tables in two places — REFUTED

Only drive.rs:54 `unplugged_code` holds an OS error-code table in code. `IMPORT_FAILURES_PLAN.md:210` proposes a new `classify_raw` table and says it absorbs `unplugged_code`, so the plan removes the second table rather than adding one.

### D-060 [low] Thumbs batch size 200 duplicated — CONFIRMED

const BATCH: usize = 200 is at thumbs lib.rs:31 and placeholders.rs:28, with no shared constant.

### D-061 [low] Drive-or-share test written three times — CONFIRMED

Three drive-or-share predicates: platform dirs.rs:48 `is_absolute_for`, native.rs:215 `form_of`, and picker.rs:19 `has_drive_or_share`, each testing for a drive or UNC prefix in a slightly different way.

### D-062 [low] Hand-typed colours bypass the Theme singleton — CONFIRMED

Theme.qml holds the tokens panelScrim #b0000000 (:9), captionBacking #b3000000 (:17), errorText #e57373 (:24), invalidText #e06060 (:26) and statusWarning #e0a050 (:30). ModalOverlay.qml:26 repeats panelScrim, GridCell.qml:95 repeats errorText, InboxPanel.qml:385 and :295 repeat invalidText and statusWarning. GridCell.qml:151 uses #a0000000, a near-copy of captionBacking.

### D-063 [low] Desktop QML structural duplication — CONFIRMED

enum PanelAction { Close, StepBack, StepForward, Pick } is declared at YandexPanel.qml:90 and identically at YandexPreview.qml:20. The wheel accumulator is at Viewer.qml:302 and YandexPreview.qml:59, and the neighbour preload is at Viewer.qml:396 and YandexPreview.qml:220.

### D-064 [low] Desktop Rust int conversion and level-code helpers repeated — CONFIRMED

`facet_choices.rs:30` count (i64 to i32, clamped), gallery/roles.rs:83 `qt_int`, `failed_files.rs:57` `qt_int`, and the `level_code` mapping at `log_model.rs:64` and notices.rs:227 (0 info, 1 warning, 2 error).

### D-065 [low] Desktop pending batch and save helpers repeated — CONFIRMED

Pending is declared three times: `log_model.rs:53`, notices.rs:215 and pump.rs:13 (generic). `save_prefs` at shell.rs:107 and `change_prefs` at settings.rs:18 have the same body, a prefs change then a worker write.

### D-066 [low] Client text and test helper duplication — PARTIAL

tree.rs:197 `visible_count` repeats the `visible_in` walk, and notice.rs:22 isolated is a bidi wrapper. The controller panels.rs:139 `set_viewer` is one setter, not four visible here. image.rs:109 `view_path` prints the region grammar that `view_of` parses back, which is a round-trip, not a duplicate. replay.rs does not exist and words.rs:97 `ui_entries` was not compared to `entries_of`.

### D-067 [low] Codec and container test helpers repeated — CONFIRMED

`over_white's` formula is in codec convert.rs:160-162 and repeated at decode.rs:510 in a test. `to_pixels` is in decode.rs:259 and resize.rs:125. container lib.rs:1019 writes the PNG signature as a literal in a test while the `PNG_SIGNATURE` constant exists at :251 in the same file.

### D-068 [low] Hash-based pseudo-random generator copied across crates — CONFIRMED

A xorshift-style generator is written in container lib.rs:1201 (noise), dedup join.rs:2078 (Rng::draw), metadata container.rs:216 (xorshift) and model pdq.rs:157 inline. All are test helpers, each with its own shift triple.

### D-069 [low] Dbfile lock and open repeated — CONFIRMED

dbfile lock.rs:29 (Lock::take) and lock.rs:57 (Live::shared) each run the same `LOCK_TRIES` retry loop with `try_lock`, and remove.rs:30 walks PARTS in `move_aside` and `remove_database`.

### D-070 [low] clippy.toml repeats root test allowances — CONFIRMED

model/clippy.toml:3-5 repeats allow-panic-in-tests, allow-print-in-tests and allow-unwrap-in-tests, which root clippy.toml:2-6 sets. The model file's own comment says to keep them in step with the root.

### D-071 [low] Docs restate the same measurements — PARTIAL

PROFILING.md:494 'Moved to `JPEG_DECODERS.md'` is a pointer, not a restatement. STORAGE.md:95 points to PROFILING.md and `START_PROFILE.md:5` gives its own stage list. The restatement claims were only partly checked, so most of the finding is pointers.

### D-072 [low] Historical guard test cited by two plans — CONFIRMED

`BACKGROUND_JOB_AUDIT_FIXES.md:37` cites `carried_is_every_table_of_the_current_schema` at schema.rs:804, and `DECODE_ONCE_PLAN:716` cites it at schema.rs:803-820. The test is at schema.rs:338 now. Both plan cites are stale, and they disagree with each other.

### D-073 [low] Eval timing, metric and callgrind helpers repeated — CONFIRMED

Callgrind counting is repeated in linux.py:111 (ir), `harness_linux.py:29` (`ir_one`) and restart.py:44 (instructions), and the median timing loop in nvjpeg.py:209 (timed). The harness.cpp cites do not exist.

### D-074 [low] Build helper boilerplate repeated across tools — CONFIRMED

checked(what, command, log, cwd) is defined with the same signature in `build_jpeg_turbo.py:36`, `build_static_ffmpeg.py:60` and `build_static_qt.py:47`. sha256 is written at `build_jpeg_turbo.py:64` and elsewhere, and `pg_home_folder` at `build_static_qt.py:66`.

### D-075 [low] Qt kit identifiers repeated — CONFIRMED

The Qt kit names `msvc2022_64` and `gcc_64` are in `tools/check_qt.py:19` and `tools/ci/github_files.py`. The cite `ci/github_files.py:55` is at `tools/ci/github_files.py`.

### D-076 [low] Modgraph and profiling tools repeat small helpers — CONFIRMED

`library_crates` is defined in `tools/modgraph/compiler_graph.py:29` and `unreachable_pub.py`. The test-section cut is written in `module_graph.py:61` and the other modgraph tools, and REPO = parent.parent is in `tools/fuzz_smoke.py:14` beside other path logic.

### D-077 [low] Pairing scripts repeat SQL and invocation — CONFIRMED

The owed-pair delete SQL is written in keep.ps1:38-40 and oracle.ps1:42, with the same DELETE FROM `pdq_pairs` WHERE a IN `pairs_owed` statement. The median is computed in `few_sweep.ps1:26` and keep.py:172 in different ways.

## errors

### E-001 [high] Panics mapped to different outcomes, one of them a fake user stop — CONFIRMED

reader.rs:227 maps a caught panic to ReadError::Panicked. yandex.rs:465 maps a panicked search to Status::Stopped, which reads as a user stop. lanes.rs:145 turns a join failure into a generic lane panic and discards the payload. images.rs:166 is a mutex poison recovery, not a panic mapping, so that instance is wrong. The core claim holds.

### E-002 [high] Exit status reports success on failure — CONFIRMED

screenshot.cpp:186 ends a timed-out-without-window run with QCoreApplication::quit(), which exits 0. The sibling no-window path at :174 uses exit(1). `unreachable_pub.py:48` exits 0 unconditionally after printing error counts. `colour_check.cu:62` returns 0 with no threshold. `REVIEW_LEDGER.md:86` records that the exit mapping has no test.

### E-003 [high] Fluent syntax errors and missing entries are silently accepted — CONFIRMED

words.rs:40 keeps the partial resource on a Fluent syntax error. words.rs:43 discards the `add_resource` result, so duplicate or failed entries are dropped. words.rs:62 returns the raw id when an entry is missing. words.rs:38 discards the `add_builtins` result. LANGUAGES.md:43 documents the same silent drop, so the gap is known but unfixed.

### E-004 [medium] Error chains formatted with {e} in some places and {e:#} in others — CONFIRMED

Plain Display ({e}) and the alternate chain ({e:#}) are both used for the same kind of error text: app.rs:58 and :217 use {e}, app.rs:74 uses {e:#}, desktop app.rs:63 uses `to_string(`), and yandex.rs:122 uses {err:#}. The mix is real.

### E-005 [medium] Stored ledger failure text built four different ways — CONFIRMED

Ledger failure text comes from several forms: hashes.rs:120 uses {e:#}, import lib.rs:179 uses `Cause::of(&e).to_string(`), writeback lib.rs:441 stores `error.to_string(`), thumbs lib.rs:320 stores shown.error, and placeholders.rs:217 uses {e:#}. The app/error.rs:76 cite is a blank line, so that instance is wrong, but the rest holds.

### E-006 [medium] Notice error text tidied and capped only on some paths — CONFIRMED

prefs.rs:153 tidies with `ERROR_CAP`, coordinator.rs:709 calls `error_text` with no tidy while the panic arm at :718 tidies, and tally.rs:93 posts the raw lane error while tally.rs:127 tidies. The tidy and cap are applied on some notice paths only.

### E-007 [medium] Assertions and expects guard invariants inconsistently in library code — CONFIRMED

Invariant guards vary: seam.rs:11 has an assert with a message, pdq.rs:57 has an assert with no message, work.rs:120 uses unreachable!, `union_find.rs:89` uses expect with a clippy reason, and desktop app.rs:54 uses expect with an allow. Low severity: the styles are each defensible, but they are not one convention.

### E-008 [medium] Silent discards of errors in production paths — PARTIAL

Several discards are documented best-effort (thumbcache lib.rs:189 says best-effort; writer.rs:398 says failures stay queued), so they are a choice, not an oversight. The crop.rs:150 cite shows Ok(stats), not the `remove_file` the finding names, so that instance is wrong. The discards are inconsistent in their handling, but not all are bugs.

### E-009 [medium] Fallbacks hide missing or dropped data — CONFIRMED

tree.rs:141-143 leaves a folder out of the tree when its offsets overflow u32, with no report, and the same silent let-else appears at tree.rs:180, 217 and 235. write.rs:95 drops an out-of-range stored value to None. image.rs:41 drops an unreadable version. orientation.rs:17 treats missing EXIF as upright. Several are intended fallbacks, but none is reported.

### E-010 [medium] Documented behaviour not implemented or unbounded decode path — REFUTED

ledger.rs:199-201 says `retry_failures` does nothing for the import, and that the import is retried by gallery import --retry. The code (ledger.rs:203-204) returns Ok early for FilePass::Import, as the doc says. The doc and the code agree, so the documented behaviour is implemented.

### E-011 [medium] Missing-configuration messages worded differently across tools — CONFIRMED

The `PG_HOME` message is worded differently in `build_static_qt.py:69`, `build_jpeg_turbo.py:75` and `build_static_ffmpeg.py:92`. The `QT_DIR` message is worded differently in `compare_shots.py:666`, `check_qt.py:21`, `lint_qml.py:70`, profile.ps1:26, qmltest.ps1:20 and qmltest.py:41.

### E-012 [medium] Same error text duplicated across Python and PowerShell ports — PARTIAL

setup-ffmpeg.ps1 at HEAD has the throws at :45 (SHA mismatch) and :58 (top folders), so the PowerShell side is real. The Python cites (`setup_ffmpeg.py:89` and :62) are blank or argparse lines and do not show the identical wording, so the duplication is not shown by these cites. The file is deleted in the working tree.

### E-013 [medium] The two setup ports apply different safety checks — CONFIRMED

At HEAD, setup-ffmpeg.ps1 has no https check (grep finds none). `setup_ffmpeg.py:42-43` refuses a non-https URL. The finding's python cite at :39 is the docstring; the check is at :42, which is a small cite error that does not change the verdict.

### E-014 [medium] Retry and failure text for builds differ between ports — CONFIRMED

build-jpeg-turbo.ps1:64 retries git fetch three times with no sleep. `build_jpeg_turbo.py:59-60` sleeps 10 times the attempt number. build-static-ffmpeg.ps1:118-119 sleeps, and `build_static_ffmpeg.py:156-157` sleeps too. The failure texts also differ: the py versions include the exit code, the jpeg-turbo ones do not.

### E-015 [medium] Tool error messages inconsistent in name prefix — CONFIRMED

linux.py:67 prefixes its fail() messages with linux.py:, while pgtools.py:35 has no prefix, `lint_qml.py:51` has none, and profile.ps1:26 has none. `win_env.py:50` uses `win_env`: and win-env.cmd uses its own prefix. The `tools/ci_setup.py` cite is missing (the file is `tools/profiling/ci_setup.py`).

### E-016 [medium] Python eval raises different exception kinds for the same class of failure — CONFIRMED

Eval scripts raise different exception kinds for similar failures: `cpu_worker.py:137` raises SystemExit, run.py:304 raises SystemExit, accel.py:202 raises RuntimeError, models.py:56 raises ValueError, and verify.py:696 calls parser.error. Low severity; these are harness scripts.

### E-017 [medium] Per-item failure policy differs across harnesses and sweeps — CONFIRMED

ann/run.py:343 records a failed configuration and continues. accel.py:130 skips a bad calibration image. `bench_decode.py:57` reads outside its try, while pdq.py:47 returns None on any Exception and verify.py:603 records the pair as error. `nvimgcodec_bench.py:87` catches Exception where nvjpeg.py:298 catches RuntimeError only. The failure policies differ across harnesses.

### E-018 [medium] String used as the error type in public functions — CONFIRMED

config.rs:25 `Config::from_env` returns `Result<Config`, `String>`. layout.rs:29 Layout::resolve returns `Result<Layout`, `String>`. size.rs:44, rating.rs:11 and replay.rs:33 also use String error types. query.rs:98 uses u8 as the error type. These sit beside typed errors elsewhere.

### E-019 [medium] Crate error types: anyhow rule in the docs does not match the code — CONFIRMED

`REFACTOR_LEDGER.md:59` (F-14, status decide) records that the plan says anyhow lives only in cli, desktop and gallery, but it appears in 20 crates and no lint enforces the rule. The gap is acknowledged and open, so the inconsistency is real. The lib.rs:27 and rebuild.rs:104 cites show the mixed return types.

### E-020 [medium] Catalog wraps conditions in Error::Other(anyhow) where typed variants exist — CONFIRMED

Catalog code wraps conditions in Error::Other(anyhow) where typed variants exist: borders.rs:47 and :55, work.rs:286, removal.rs:243, and lifecycle.rs:48 for a lock. lifecycle.rs:38 uses the typed Error::PathRules for a similar rules mismatch, so the typed variant is there to use.

### E-021 [medium] Catalog path prefix convention applied inconsistently — CONFIRMED

lifecycle.rs:47 prefixes its lock messages with catalog path, while ledger.rs:269, :301 and lookup.rs:102 have no catalog prefix, and rebuild.rs:95 and :112 use their own forms. core.rs:125 repeats the same context, with the path alone.

### E-022 [medium] Catalog refusal texts for the same condition differ — CONFIRMED

core.rs:139 says the schema version is unknown to this build and was made by another build. core.rs:270 says schema version X, not this build's Y. core.rs:163 says the schema predates `key_version` and to restore a backup. schema.rs:302-313 gives its own mismatch reasons. The refusal for one condition is worded differently in each.

### E-023 [medium] Catalog InvalidColumnType payloads take four shapes — CONFIRMED

InvalidColumnType payloads take several shapes: borders.rs:90 uses the stored key, queue.rs:146 the raw kind text, queue.rs:151 a formatted kind and value, queue.rs:161 a column name, and queue.rs:168 a column name with a Null type. The grid.rs:67 cite is missing; the other five instances hold.

### E-024 [medium] Lossy error conversions flatten the source to a string — CONFIRMED

rebuild.rs:138 turns a `pg_dbfile` error into Error::Other of its display text, which loses the source. roots.rs:75 stores rules.display() strings in RemovalPending. yandex.rs model :70 stores a String error where Download keeps the typed RequestError. refusal.rs:34 stores `error.to_string(`).

### E-025 [medium] Invariant text and error shape for the codec memory limit kind — PARTIAL

tiff.rs:102 returns LimitsExceeded for a missing sample type, and png.rs:111 maps the png decoder's LimitsExceeded to InsufficientMemory, which is the kind the finding says should be used for a real memory limit. gif.rs:58 uses InsufficientMemory for a `checked_mul` overflow, which is arguably the right kind for an overflow. Whether each mapping is right needs a per-case decision; the finding's single 'wrong kind' claim is not shown.

### E-026 [medium] Debug text reaches users in codec and decoder messages — CONFIRMED

jpeg.rs:97 puts the zune-jpeg Debug form of the error into a GenericFeature message, which the user can see. eval png-decoders lib.rs:78 uses Display and lib.rs:126 and main.rs use Debug, so the harness mixes forms too. The user-facing case is the one that matters.

### E-027 [medium] Path rendering in error text uses three helpers — CONFIRMED

Paths in error text come from three helpers: crop/lib.rs uses shown(), dbfile/remove.rs:35 uses Path::display(), and removal.rs:243 uses rules.display(). scan/lib.rs:375 uses catalog.rules().display(). The mix is real; low severity.

### E-028 [medium] Turns bound in `from_stored` duplicates the modulo rule — PARTIAL

write.rs:98 bounds stored turns with a literal small `<=` 3, and returns None above it. turns.rs:14 reduces modulo 4. These are different rules: `from_stored` rejects a damaged stored value, while Turns::new wraps a computed one. The literal duplicates the range but does not duplicate the modulo rule.

### E-029 [medium] Stale last-error buffer in the GPU encoder — CONFIRMED

`encode_gpu.cu:986` `egpu_encode_batch` does not clear `g_error` on entry, and `g_error` is written only on failure (:1000 and :1006), so a later error can read a stale message from an earlier call. The eval harness is the only place this matters.

### E-030 [medium] Unchecked CUDA and FFI return values — CONFIRMED

`dec_gpu.cpp:68` ignores the cudaStreamCreate return, and :49 ignores cudaSetDeviceFlags. `dec_turbo.cpp:11` uses tj3Init's handle with no null check. `test_encoder.cpp:232` and :1094 ignore cudaMemcpy and cudaMalloc results.

### E-031 [medium] A test passes despite a decode failure — CONFIRMED

`test6_quality` at `test_encoder.cpp:973` sets pass as `failed_loads` == 0 at :1035. The decode-failure branch at :1008-1010 prints and continues without incrementing `failed_loads`, so a decode failure does not fail the test. Other branches in the same harness set pass false directly, so the handling is inconsistent.

### E-032 [medium] Verify scoring counts error pairs in two ways — CONFIRMED

`verify_synth.py:68` counts every non-match as incorrect, including error. verify.py:786-787 sets correct only for labels in EXPECTED, so an unlabelled pair is not counted. verify.py:623 records a failed pair as decision error. The two harnesses count error pairs differently.

### E-033 [medium] CLI error text comes from three sources — CONFIRMED

CLI error text comes from three sources: lib.rs:367 bails with the ScanError Display, lib.rs:403-405 bails with the Fluent line, lib.rs:145 maps through `path_words`, and dups.rs:62 builds anyhow from a line. The same command can print text from any of them.

### E-034 [medium] User-visible English literals bypass the message catalogue — CONFIRMED

library.rs:282 gives a user-visible notice the literal the folder list changed: try again, bypassing text::render. providers.cpp:89 returns the English image unavailable. `gpu_device.cpp:121` logs English only. The first two reach the user; the log line is developer-facing.

### E-035 [medium] Start failure is logged but not shown to the user — CONFIRMED

library.rs:157 posts LibraryUnavailable to the inbox when background work fails to start. library.rs:172 logs a change-pump start failure and posts nothing, so the user is not told that changes will not be seen.

### E-036 [medium] Yandex error texts and mappings disagree — CONFIRMED

yandex.rs:493 and images.rs:219 use YandexError::Unexpected with English details. images.rs test at :324 expects the text unexpected Yandex response: no such result. yandex.rs:254 uses Undecodable with {e:#} and :259-261 uses Unreadable with `to_string(`). text.rs:383 renders both Unreadable and Undecodable as one message, so the variants do not show different text.

### E-037 [medium] Replay errors count lines in some places and records in others — CONFIRMED

replay.rs:43 formats parse errors as line N and replay.rs:74 as record N. The test at client tests replay.rs:45 requires the replay error to start with record N:, and parse errors use line N. Two counting units are used in one module.

### E-038 [medium] Replay missing and mismatched format messages — CONFIRMED

Tests fix the two messages: tests replay.rs:55 expects a line 1 prefix on the missing-format error, while :60 expects recorded by format N with no line prefix. The same kind of failure is shaped differently.

### E-039 [medium] Fatal startup path writes the log and error box, not stderr — PARTIAL

The fatal path is deliberate: the doc comment at desktop lib.rs:244-246 says a GUI program has nowhere to print, so it logs and shows a box, and it says the record is lost if the log has no file yet. The catalog-open call at lib.rs:169 follows that design. The gap is the in-memory record loss before the log opens, which the doc already states.

### E-040 [medium] Downcast to io::Error misses the Unopened wrapper — PARTIAL

`BACKGROUND_JOB_AUDIT_FIXES.md:178` says the Unreadable downcast is verified, and :186 amends DO 5.6 to stop using `downcast_ref` of io::Error because it misses Unopened. The plan records its own correction, so the inconsistency is in a historical plan that the same document fixes.

### E-041 [medium] Read failure is classified four different ways — CONFIRMED

Read failure is classified four ways: borders lib.rs:97 and import hashes.rs:114 downcast io::Error, thumbs lib.rs:112 matches MakeError variants, and maker.rs:43 defines MakeError with `of_video` downcasting Unopened. The two downcast sites are the same idiom in two crates and a third classifier beside them.

### E-042 [low] Thumbcache refusal and writer error texts — CONFIRMED

thumbcache schema.rs:297 bails with path: detail even when made anew. schema.rs:310 and :311 give Foreign and Newer the same shape, with Newer adding made by a later build. thumbcache lib.rs:138, :140 and :150 use three wordings for the cache being unavailable.

### E-043 [low] Error conversion idioms differ within the same file or area — CONFIRMED

Conversion idioms differ within one area: library.rs:399 uses `map_err(pg_catalog::Error::Other`), library.rs:423 uses `map_err(failed`), yandex.rs:339 uses error.into(), and catalog listing.rs:40 uses Error::Other(e.into()). Low severity.

### E-044 [low] Source-level invariant and fallback behaviour in the platform layer — PARTIAL

picker.rs:141 holds the `ERROR_CANCELLED` constant, and drive.rs:56 uses the literal 21 for the unplugged code. drive.rs:251 defines `ERROR_MORE_DATA` locally as 234. The finding's crates/platform/src/lock.rs:50 does not exist; the lock test helper is at platform/src/testing/lock.rs. Half the cites are right, and the local-constant issue is real.

### E-045 [low] Planned error names in docs/plans still use pre-rename names — PARTIAL

`BACKGROUND_JOB_AUDIT_FIXES.md:80-84` plans renames: Error::ScanBusy to LeaseBusy and fence to LeaseLost. Code still has ScanBusy (catalog error.rs:28) and LeaseLost (error.rs:33), so the plan's names are pre-rename, which is expected until the work lands. The cli cite at lib.rs:168 does not show ScanBusy, so that instance is wrong.

### E-046 [low] Unknown-format wording repeated with three forms — CONFIRMED

Unknown-format wording comes in three forms: analyze.rs:102 and display.rs:38 use context unknown image format, lookup.rs:178 uses unknown format X inside a SQL conversion error, and writeback lib.rs:580 uses X: unknown format. The three shapes are real.

### E-047 [low] Too-large and damaged wording in the edit crate varies — CONFIRMED

Edit crate messages vary: raster.rs:19 says is too large to decode, webp.rs:53 says the WebP is too large for a RIFF file, png.rs:52 says damaged PNG, and gif.rs:37 says GIF image data is short while raster.rs:40 says the image data is short. The wording is inconsistent.

### E-048 [low] Spelling, case and symbol drift in user-facing messages — CONFIRMED

tiff.rs:132 uses color while jpeg.rs:36 and :66 use colour. tiff.rs:140 capitalises Not enough planes. crop.rs:102 and the size text use x and the multiplication sign inconsistently: fixtures.rs:20 and crop.rs:36 use the multiplication sign.

### E-049 [low] Contraction and modal wording varies — CONFIRMED

borders.rs:47 uses isn't in the catalog and state.rs:83 uses is not in the catalog. inbox.rs:398 and :927 use different wording for the same kind of failure. Low severity.

### E-050 [low] Context strings vary in style and some are missing — CONFIRMED

Context strings vary in style: pipeline.rs:339 is a comment, not a context. coordinator.rs:192 uses start the lane coordinator, borders cli uses reading X, encode.rs:88 uses bare decoding and encoding, and placeholders.rs:131 has no context on fs::read. Low severity.

### E-051 [low] Log level for the same kind of failure varies — CONFIRMED

Log level for similar conditions varies: gallery.rs:69 and state.rs:148 log an error when a command arrives before the model exists, animation.rs:63 logs a warning for a recoverable file error, and controller.rs:45 logs a warning for a bad option. The levels are a judgement, and the mix is real.

### E-052 [low] NUL-in-path error wording varies — CONFIRMED

NUL-in-path messages differ: ledger.rs:269 is a failed file's folder holds a NUL with no prefix, ledger.rs:301 is a failed folder's path holds a NUL, lookup.rs:102 is a stored folder path has a NUL, removal.rs:194 is a removal's folder has a NUL, and dir.rs:255 is the path has a NUL. Four wordings for one condition.

### E-053 [low] Command-line and option error phrasing varies — CONFIRMED

`command_line.rs:233` says does not read as a query, :240 says is not a finite row height, :255 says is not a mascot: 1 to 40, and :291 says is not an option the window reads. The phrasing varies by option, and the error for a bare option is a fourth form.

### E-054 [low] Tool exit and unknown-step conventions vary — CONFIRMED

`bench_data.py:152` and perf-compare.py:105-107 print to stderr and return 1. `ci/pillow_pin.py:31` exits with the pip return code. linux.py:189 and `win_build.py:194` use sys.exit with a message, and `win_build.py` names the program while linux.py does not. Exit conventions differ.

### E-055 [low] Saturating conversions clamp overflow silently — PARTIAL

perf/lib.rs:277 saturates a duration to u64::MAX on purpose, and platform clock.rs:51 documents its u16 fallback for a year past 65535. tiff.rs:96 saturates an entry count to usize::MAX. The saturation is deliberate and commented in the clock case, so only the undocumented ones are a problem.

### E-056 [low] Build script failure style varies — CONFIRMED

video/build.rs:207 uses `unwrap_or_else` with panic! for `read_dir`, and :229-230 use panic! for copy, while :137 returns Err(String) for pkg-config failures. desktop/build.rs:4 returns `Result<(`), `String>` from main. Two failure styles are used in build scripts.

### E-057 [low] Test expect and unwrap messages drift — CONFIRMED

Test messages drift: controller listing.rs:20 expects a row height in range, size.rs:28 expects a factor and `controller_laws.rs:50` expects a positive factor, size.rs:16 expects a positive scale, and listing.rs:180 uses a bare unwrap where others use expect. Low severity.

### E-058 [low] Win32 literal and hand-written Display conventions in platform — CONFIRMED

NormalizeError in names.rs:148-150 has a hand-written Display, as native.rs and picker.rs do. dir.rs:93 builds Unsupported with ErrorKind.into(), while drive.rs:397 uses io::Error::new with a message. Low severity.

### E-059 [low] Turns and count decode limits differ from the documented range — PARTIAL

`union_find.rs:88` says a catalog holds fewer than 2^32 files, and join.rs:70 gives `MAX_PICTURES` as u32::MAX `>>` 4, which is 2^28 pictures. These are different limits on different things (files and pictures), so the documented ranges agree once the units are separated. The finding does not show a conflict.

### E-060 [low] Notice text overwrite on same-bytes refailure — CONFIRMED

`NOTIFICATIONS_PLAN.md:61-65` records the design's undeclared deviations as open rows: N-35 `dismiss_all` deletes events, N-37 the dismissed-entry test moved, N-38 the cascades, N-39 the seven cache events against the designed five. The plan tracks them, so the deviations are known; the same-bytes refailure overwrite is still open per the finding.

### E-061 [low] Unplugged error classifier is duplicated by the import plan — PARTIAL

`IMPORT_FAILURES_PLAN.md:208-212` says there is no existing OS-error classifier to reuse, notes the private `unplugged_code` table in drive.rs:54-68, and says the new table absorbs it. The duplication is planned for removal, not an unnoticed second table. drive.rs:56 is the literal 21, which the plan describes.

### E-062 [low] ExifTool stderr interpolated in messages and English stdout parsed — CONFIRMED

exiftool/lib.rs:153 interpolates ExifTool stderr into the user-visible bail. LANGUAGES.md:102 records that ExifTool runs with no lang option and that success is parsed from English stdout. The English dependency is documented and open.

### E-063 [low] CUDA error text formats differ across harnesses — CONFIRMED

CUDA error text differs across harnesses: gpu.py:42 and `nvimgcodec_bench.py:44` use cuInit returned N, nvjpeg.py:104 uses X: status N, `dec_gpu.cpp:16` uses the macro name plus cudaGetErrorString, and `encode_gpu.cu:40` uses what failed: cudaGetErrorString. Low severity; harness only.

## config

### C-001 [high] dotenv quote handling differs between tools — CONFIRMED

pgtools.py:55 strips the quotes from a .env value, as just does. `lint_qml.py:40` keeps the raw quoted value, symbolize.py:60 keeps the raw value with setdefault, and profile.ps1:22 keeps the raw value. A value written in quotes reads differently in these tools.

### C-002 [high] `SQLITE_DEFAULT_MEMSTATUS` stated with conflicting values across docs — CONFIRMED

`BOUNDED_MEMORY_PLAN.md:167` would force `SQLITE_DEFAULT_MEMSTATUS=1`. `SCAN_OPTIMIZATION_PLAN.md:112-114` says the fork route is resolved and that the two plans must keep one value. The code's test at core.rs:626 asserts `DEFAULT_MEMSTATUS=0`, and PROFILING.md:1090 and scan-optimizations-validation.md:287 use 0. `CATALOG_SIZE_PLAN.md:167` names the same flag. The bounded-memory plan still says 1.

### C-003 [high] Remembered-save debounce: accepted decision contradicts design and code — CONFIRMED

`WEB_CLIENT_TECH_DECISIONS.md:113` sets the remembered-save debounce at 500 ms (B15). `WEB_CLIENT_DESIGN.md:1186` and `WEB_CLIENT_RECONCILED.md:796` say SaveSession debounces 1,000 ms, and deadlines.rs:25 asserts the 1000 ms pause. The accepted decision contradicts the design and the code.

### C-004 [high] Web client security parameters disagree between decisions and design — CONFIRMED

`WEB_CLIENT_TECH_DECISIONS.md:147` accepts Argon2id at 128 MiB, t=3, p=1 (O3, measured at 0.59 s). `WEB_CLIENT_DESIGN.md:174` proposes 64 MiB (m=65536 KiB), and `WEB_CLIENT_RECONCILED.md:180` says 64 MiB as well. The accepted and proposed memory costs differ. The session lifetime also differs: `TECH_DECISIONS` says 90 days absolute at :146, and DESIGN.md:340 is cited at 180 days.

### C-005 [high] Pairing gate defaults differ between PowerShell and Python drivers — PARTIAL

keep.ps1:10 defaults Rule to bounded and passes --rule and --margin explicitly at :129-130 (margin 0.10 unless the rule is margin). keep.py:105 and :107 default to margin 0.01 and rule margin. The defaults differ, but the PowerShell driver always passes its values, so the difference only shows in a direct Python run. The user's pairing memory records the bounded 0.10 rule as the gated one.

### C-006 [medium] `PG_*` environment reads scattered; plan and ledger counts stale — CONFIRMED

`PG_*` environment reads are spread across layout.rs:30-31, exiftool lib.rs:31, desktop lib.rs:157, video build.rs:64 and more. `REFACTOR_LEDGER.md:52` records this as an open shortcut (F-12), so the scatter is known and not yet consolidated.

### C-007 [medium] .env.example omits app variables that code and docs use — CONFIRMED

.env.example lists `PG_HOME` and `PG_CATALOG_DIR` in its app section (lines 29-33) but has no entry for `PG_PREFS`, `PG_THUMB_CAP_GB` or `PG_PERF_LOG`, which the crate README documents (crates/README.md:29, :290, :362). The example file omits app variables that the docs use.

### C-008 [medium] Env var prefix convention not followed for repo-owned keys — CONFIRMED

.env.example names `QT_DIR` (line 12), `LIBCLANG_PATH`, `FFMPEG_DIR`, `JPEG_EVAL` and `CUDA_PATH` without a prefix, while `PG_TEST_DIR` (line 38) and the `PG_` family are prefixed. tools/linux.py:344 and eval/jpeg-decoders/linux.py:42 add more unprefixed variables beside `PG_` ones.

### C-009 [medium] Boolean and flag env values parse differently — CONFIRMED

Flag variables parse differently: desktop trace.rs:25 accepts only the value 1 for `PG_TRACE`, `command_line.rs:329` accepts 1, 0, true and false for `PG_DEV`, `gpu_device.cpp:71` accepts only 0 for `PG_SHARED_DEVICE`, `gpu_device.cpp:43` reads `PG_FORCE_AV1` as an int, and exiftool lib.rs:295 treats `PG_REQUIRE_TOOLS` as on for any value.

### C-010 [medium] FFmpeg executable named two ways — CONFIRMED

`make_test_video.py:22` reads FFMPEG, while doctor.py:146, doctor.ps1:71 and the Dockerfile at :30 use `PG_FFMPEG`. The same executable is named two ways.

### C-011 [medium] JPEG input folder has two variable names — CONFIRMED

The JPEG input folder has two variables: benches/ir.rs:12 reads `JPEG_INPUTS` (default /root/jpeg-inputs), and linux.py:367 reads `JPEG_SRC_DIR` for the inputs step. The eval README (line 3) and docker README (line 56) name `JPEG_SRC_DIR` for the same photos. The roles overlap and are documented in different places.

### C-012 [medium] Eval data-root variable named inconsistently — PARTIAL

`PG_DATA_ROOT` is the dataset folder in common.py:48 and viewer.py:5. `PG_ROOT` is the base folder that the other default paths hang from (common.py `pg_path`). Both exist with different roles. The TIER2 plan line 17 cite does not name `PG_ROOT` as the data root, so that instance is wrong. The use of both is real but not one mistake.

### C-013 [medium] 5-second SQLite busy timeout written as literals in several places — CONFIRMED

The 5-second busy timeout is written as a literal in several places: catalog schema.rs:116 (`busy_timeout` = 5000 in SETTINGS), core.rs:376 (Duration of 5 seconds), thumbcache schema.rs:90 (`BUSY_TIMEOUT`), and dbfile lock.rs:12 (100 tries of 50 ms, documented as about 5 s). inbox.rs:136 uses 100 ms for a different purpose. The `stmt_journal_probe` cite is missing from the working tree.

### C-014 [medium] `cache_size` sign and unit convention inconsistent — PARTIAL

SQLite's `cache_size` takes a negative value for KiB, so the sign convention is the same everywhere. `READER_CACHE_KIB` (roots.rs:336) is a positive u32 and `PREPARE_CACHE_KIB` (types.rs:133) a positive i64, both negated at the pragma, and SETTINGS and schema.rs:430 use negative literals. The types and literals differ, but the units agree. The reader.rs:59 cite is a doc comment, not a `cache_size` value.

### C-015 [medium] `auto_vacuum` INCREMENTAL mode as bare literal 2 — CONFIRMED

The `auto_vacuum` INCREMENTAL mode is the bare literal 2 in schema.rs:178, removal.rs:269 and the test at schema.rs:598. There is no named constant for it, so the magic number is repeated three times.

### C-016 [medium] Preference defaults and names disagree between code, docs and plans — PARTIAL

prefs.rs:122 sets `write_back` to true and `real_size` to false, and the settings bridge documents `detect_borders` as on by default. `DECODE_ONCE_PLAN.md:103` names the field `pregenerate_thumbnails` where the code has `thumb_cache`, which is a stale plan name. LANGUAGES.md:79 says the prefs hold seven fields, and that count was not checked against the struct here.

### C-017 [medium] Hard-coded D:/PhotoGallery defaults in tools and eval — CONFIRMED

build-jpeg-turbo.ps1:12 defaults Work to a D: PhotoGallery static folder, and :13 defaults BuildTools to a Visual Studio 18 path. `test_encoder.cpp:43` hard-codes a D: PhotoGallery ssimulacra2 path, and :41-42 hard-code a per-user scratch path. Python tools derive the same defaults from `PG_HOME` (`build_static_qt.py:182`), so the defaults come from two sources.

### C-018 [medium] FFmpeg default prefix differs by tool and OS — CONFIRMED

The FFmpeg default prefix differs: doctor.py:55 uses .deps/ffmpeg only on Windows and otherwise pkg-config, doctor.ps1:49 always uses .deps/ffmpeg, `compare_shots.py:670` always uses the repo .deps/ffmpeg, and video build.rs:102 uses .deps/ffmpeg on Windows and pkg-config elsewhere.

### C-019 [medium] Two clippy.toml files that must be kept in step — PARTIAL

model/clippy.toml:1-3 repeats the root's hash-collection bans and `allow-*-in-tests` settings, and asks to keep them in step by hand. `CLIPPY_LEDGER.md:15` says clippy.toml bans HashMap for the whole workspace, which is true only through the root file. The repetition is a maintenance risk the file itself names; the two files agree today.

### C-020 [medium] Fixture module gating disagrees with its doc — CONFIRMED

fixture.rs:1 says the fixtures are behind the fixture feature, but fixture.rs:4 gates raw with cfg(test) while the pub mod state is not gated. browse.rs:8 gates its fixtures with cfg(test) only. The gating does not match the module doc.

### C-021 [medium] Ctrl modifier gating differs between grid and viewer zoom — PARTIAL

keys.rs:105-106 uses Ctrl for the grid's Plus and Minus (row resize), keys.rs:134-135 uses plain keys for viewer zoom, and keys.rs:117 gives B no modifier check. The actions differ, so the gating differs by design; the finding does not show a shortcut that is wrong.

### C-022 [medium] Web client numeric limits disagree between design and reconciled docs — CONFIRMED

The web client's numeric limits disagree. DESIGN.md:1297 allows 4 held polls per device, while RECONCILED.md:261 says at most 2. DESIGN.md:1501 has a 2,000-miss queue, and RECONCILED.md:338 has 256 per device and 2,048 globally. DESIGN.md:1506 allows fit sides of 16,384 px, while RECONCILED.md:1067 says 8,192 px. DESIGN.md:1565 gives 64 KiB, and RECONCILED.md:461 gives 128 KiB for selections.

### C-023 [medium] Stop-wait timings differ across app and library — PARTIAL

The stop waits are per purpose: yandex.rs:102 `STOP_WAIT` 2 s, library.rs:34 `JOB_STOP_WAIT` 5 s, inbox.rs:142 `SHUTDOWN_DRAIN` 5 s, and coordinator.rs:47 `RETIRE_WAIT` 10 s. Their names describe different waits, so the values are not meant to match. The naming is mixed, and the values are not a conflict.

### C-024 [low] Batch and bench variable names overlap — PARTIAL

`PG_BENCH_DATA` and `PG_BENCH_JPEG` are read at build time (catalog.rs:28 and linux.rs:23), `PG_BENCH_DATA_SOURCE` is a runtime .env key (docker README:52), and `PG_FFMPEG_BIN` is baked in at build time (load.rs:171). The overlap the finding names is real, but these are build-time versus run-time reads of different variables, not a single conflict.

### C-025 [low] Yandex fixture flag missing from test hooks list — PARTIAL

crates/README.md:111 says Config::yandex is set by a recorded session that --yandex-fixture names. The hook list at README.md:509 is the screenshot hook list and does not include the flag. The flag is documented at :111, so the omission is from a separate list, not a missing doc.

### C-026 [low] Subprocess holder env protocol uses different prefixes per crate — CONFIRMED

The subprocess holder protocol uses a different prefix per crate: catalog `other_process.rs:32-34` sets `PG_CATALOG_HOLDER`, MODE, READY, and dbfile live.rs:31-33 sets `PG_LIVE_DB`, MODE, READY, and platform testing tests.rs:13 uses `PG_PLATFORM_TESTING_CHILD_PREFIX`. The same protocol has three naming schemes.

### C-027 [low] Probe environment variables documented incompletely — CONFIRMED

`scan_probe.rs:6-8` says the probe uses `PG_TEST_DIR` only, but the probe reads `PG_PROBE_AGAIN` and `PG_PROBE_BETWEEN`. pairs.rs:41-44 lists the `PG_PROBE_*` variables it reads, so that doc is complete. The `pairs_load_probe.rs:154` cite is missing from the tree.

### C-028 [low] dotenv container skip only in pgtools — CONFIRMED

pgtools.py:47 skips the .env file when `PG_CONTAINER` is set, but `lint_qml.py:32` has no container check and loads the file anyway. The two `load_env_file` functions differ. Low impact, since the container has no .env.

### C-029 [low] Stale file reference for the cap environment variable — REFUTED

`NOTIFICATIONS_DESIGN.md:323` attributes `PG_THUMB_CAP_GB` to lib.rs run, not app.rs. The finding's claim that the doc says app.rs is not in the doc. The desktop reader is desktop lib.rs:157, which matches the doc.

### C-030 [low] Eval scripts use different random seeds by default — CONFIRMED

Eval scripts use different seeds by default: `make_subset.py:20` uses 1, pdq.py:153, synth.py:51 and `bench_decode.py:146` use 0, `bench_semantic.py:103` uses random.Random(4), and ann/data.py:84 uses `default_rng(0`) while seeds 1 to 5 are used elsewhere. Low severity.

### C-031 [low] Eval worker, batch and budget defaults differ across scripts — CONFIRMED

Worker and budget defaults differ: `bench_decode.py:144` defaults processes to 12, `bench_thumbs.py:334` defaults workers to 10, pdq.py:154 defaults workers to 6, plant.py:54 defaults workers to 12, and gpu.py:31 and `gpu_encoders.py:26` set `BUDGET_S` to 1.5 and 2.0. Low severity.

### C-032 [low] Verifier thresholds repeated as argparse defaults — REFUTED

group.py:160-161 uses the constants `SHORTCUT_PDQ` and `SHORTCUT_SIM` as the argparse defaults, so the values are not repeated; they are referenced. The constants are defined once at :36-38.

### C-033 [low] Profiling tool install defaults repeated across scripts — CONFIRMED

The profiling install defaults repeat: profile.py:52 and profile.ps1:27 both default `PIN_EXE` to a LOCALAPPDATA Programs sde pinkit path, build.py:66 defaults `LLVM_DIR` under PROGRAMFILES, build.sh:22 does the same, and symbolize.py:78 repeats it. Low severity.

### C-034 [low] Windows Qt font folder fallbacks differ — CONFIRMED

qmltest.py:98 falls back to C:/Windows for WINDIR, qmltest.ps1:45 has no fallback, and `gui_app.cpp:29` uses the variable with no fallback. Low severity.

### C-035 [low] PowerShell error preference differs between tools — PARTIAL

doctor.ps1:7 sets ErrorActionPreference to Continue because it reports and keeps going, and profile.ps1:19 sets Stop because a failed profile run should end. The two scripts have different jobs, so the difference is by design.

### C-036 [low] Same-named constants defined in two crates — PARTIAL

`LEDGER_EVERY` is 2 s in thumbs lib.rs:34, and the catalog ledger doc at ledger.rs:52 says 2 s too. The values agree, so the finding's conflict is not there. The inbox.rs:35 doc cites `pg_app::inbox::LISTED`, which the cite does not show, so that part is unchecked.

### C-037 [low] Writeback test asserts retry cap as literal 5 — CONFIRMED

writeback.rs:387 waits for attempts of at least 5, a literal. writeback.rs:664 uses `pg_catalog::queue::MAX_ATTEMPTS` for the same cap. The test uses the literal in one place and the constant in another.

### C-038 [low] Thumbcache shrink threshold differs under test — REFUTED

`SHRINK_ABOVE_PAGES` is 16 under cfg(test) and 16384 otherwise (maintenance.rs:13). A test-only threshold is the usual pattern, so the difference is intended.

### C-039 [low] Retry and wait intervals repeat the 50 ms literal — PARTIAL

The 50 ms value appears as `RETRY_EVERY` (transaction.rs:40), `RETRY_PAUSE` (inbox.rs:139) and a literal at library inner.rs:128. The pipeline.rs:644 cite is missing. Each constant is for its own wait, so the repetition is not one value that must change together.

### C-040 [low] Time constants use mixed types — CONFIRMED

Time constants use mixed types: `TOUCH_AGE_S` is an i64 of seconds (config.rs:67), `TOUCH_FLUSH` is a Duration (config.rs:71), and `SCAN_LEASE_SECS` is an i64 of seconds (types.rs:137). Low severity.

### C-041 [low] CLI defaults repeat pipeline defaults — CONFIRMED

The CLI repeats pipeline defaults as literals: args.rs:34 has `default_value_t` 8 for readers, and pipeline.rs:55 sets readers to 8 and budget to 8. The 12 MB buffer default is stated in the CLI doc and computed in pipeline.rs:75.

### C-042 [low] Similar-pages count differs between crawler and app — PARTIAL

`REVERSE_SEARCH.md:72` says the crawler script sets 20 pages, while the measured runs used 5, and the app's `DEFAULT_MAX_SIMILAR_PAGES` is 5 (yandex client.rs:25). The doc names both figures and says which one was measured, so the difference is recorded, not hidden.

### C-043 [low] Planned gallery.json keys do not match the flat preferences schema — PARTIAL

`BOUNDED_MEMORY_PLAN.md:1537` and :1889 plan nested gallery.json keys (`grid.range_indexes`, `journal.trust_detachable`), and LANGUAGES.md:79 says the prefs schema is flat. The nested keys are proposals for a schema that does not exist yet, so the mismatch is with a plan, not with the shipped file.

### C-044 [low] Eval viewer forces dark-only tokens — REFUTED

eval/viewer.html:8 says Dark theme always, and color-scheme dark at :10 is the intent of that file. site/style.css is the published site and has its own light tokens. Two pages with two themes is deliberate.

### C-045 [low] Data folder names differ by platform — PARTIAL

dirs.rs:32 uses PhotoGallery under LOCALAPPDATA on Windows, and dirs.rs:37 uses photo-gallery under `XDG_DATA_HOME` or HOME on Linux. Each follows its platform's convention, which is intended; the names differ by platform, as in N-033.

### C-046 [low] Yandex preview flag handled two ways in one parser — CONFIRMED

`command_line.rs:263` silently filters the yandex-preview option when find-similar is absent. `command_line.rs:278-279` reports the same condition as a bad option. The same parser handles one condition two ways.

### C-047 [low] Windows setup defaults repeated in cmd and python — CONFIRMED

`win_env.py:52` falls back to C:/Program Files (x86) when PROGRAMFILES(X86) is unset, while win-env.cmd:9 uses the variable with no fallback. The two Windows setup scripts do not share the default.

## units-types

### U-001 [medium] Decimal and binary MB/GB labels mixed in code and tools — CONFIRMED

cli lib.rs:237-238 gigabytes() divides by 1e9 (decimal) for cache totals, while the cli test at lib.rs:986 builds `Cap::from_bytes(1` `<<` 30) (binary GiB). The cite at lib.rs:960 is a different test. lifecycle.rs:268 MB is decimal, query lib.rs:88 MB is binary, report.rs:111 mbps is decimal, rehearsal.rs:46 is MiB, eval/ann/report.py:44 gb is `2**30`. `dhat_top.py` is at tools/profiling/pairing/, not tools/profiling/.

### U-002 [medium] Docs give the same size figures in MB and MiB — CONFIRMED

IO.md:42 labels 240 MB as 12 MiB x (8+12). 12 MiB x 20 = 240 MiB, so the unit is wrong, and IO.md:55 and :64 repeat 240 MB. `CATALOG_SIZE_PLAN.md:1` title uses decimal MB (519 MB) while its body uses MiB.

### U-003 [medium] Comments state units or contracts the code does not follow — PARTIAL

Mixed result across the instances. REFUTED: reader.rs:37 `GRID_CACHE_KIB` = `256*1024` KiB matches its KiB doc, and the 120 MB at :186-189 is a measurement, not the constant's unit. REFUTED: `shell_bridge.rs:182` (1 clockwise, -1 counterclockwise) matches controller.rs:388. CONFIRMED: convert.rs:146-150 says decoders give exactly the expected size and anything else is a bug, but `check_len` (:149-150) accepts len `>=` expected. CONFIRMED: date.rs:33 says impossible times give None, but :44 accepts seconds 0..=60 with no note on leap seconds. CONFIRMED: join.rs:309 documents `FEW_BASE` + `FEW_PER_THREAD` × T as `few_entries_on`, but :338 multiplies by `QUERY_ORIENTATIONS`. PARTIAL: gif.rs:21 cites gif's 50 MB limit, which belongs to the image crate, and codec/src/limits.rs does not exist. `NOTIFICATIONS_DESIGN.md:228` says paths are String, and :240 gives folder as `Vec<u8>`, which is a field-level exception the doc does not mark. verify.py:199 does not exist. Viewport swapsAxes range (0-3) not checked against playerTurns.

### U-004 [medium] Quarter-turn rotation typed as u8, i64, i32 or Turns, with literal modulus 4 — CONFIRMED

`file_rotation` returns u8 (fixture/state.rs:88), `analysis_rotation` returns i64 (:189), `set_analysis_rotation` takes u8, lookup.rs:45 compares via i64::from, model image.rs:24 parses u8, Turns::new takes u8 and reduces mod 4 (turns.rs:13-14), queue.rs:113-114 and browse state.rs:85 repeat % 4 inline, and client controller.rs:113 carries i32 turns.

### U-005 [medium] Pixel dimensions held as u32, usize and i64 across layers — CONFIRMED

Pixel sizes are u32 (analyze.rs:164, analysis.rs:37, CropSource types.rs:443), usize (borders.rs:144 converts with `try_from`, client image.rs:138, render display.rs:77), and i64 (client viewport.rs:15, desktop viewport/state.rs:19). lookup.rs:173 reads them as u32 while thumbcache lib.rs:179 reads height as u32 and reader.rs:73 binds i64 for the same column.

### U-006 [medium] Grid picture height typed differently in app, library, prefs and client — CONFIRMED

yandex.rs:119 takes `grid_height` as usize, prefs.rs:89 deserialises row height as f64, client controller/size.rs:79 uses f64 arithmetic with RowHeight, and model size.rs:10 gives `ROW_HEIGHT` and `THUMB_HEIGHT` as u32 ranges. The cited library/src/library.rs does not exist; ThumbHeight is used in inner.rs, runner.rs and shared.rs.

### U-007 [medium] File size held as u64, i64 or usize across structs and crates — CONFIRMED

Entry.size is u64 (platform listing.rs:17), `size_and_time` returns i64 (file.rs:154), ScannedFile.size is u64 while Prior, FolderRow, WorkItem and CropSource hold i64 (types.rs:321, 338, 363, 407, 439), writers.rs:37 converts with `i64::try_from`, and import lib.rs:100 uses `unwrap_or(0`) on a u64 size. FileState.size is i64 (model state.rs:18).

### U-008 [medium] File and volume identities held as u64 in model and i64 in storage — CONFIRMED

FileIdentity volume and `file_index` are u64 (model ids.rs:37-38) with the doc claiming 64 bits, while writers.rs:36 casts them with `cast_signed` to i64, pair.rs:64 holds i64, roots.rs:274 converts with `cast_signed`, and lanes.rs:22 Place::Mounted carries volume: i64. `MULTI_DRIVE_LANES_PLAN.md:144` keeps u64 with an i64 cast at the SQL edge.

### U-009 [medium] File id typed differently across the Qt and QML boundary — CONFIRMED

`failed_files.rs:83` sends fileId as f64, shell.rs:335 takes `file_id`: f64, `video_source.rs:31` takes i64, and the QML side mixes them: InboxPanel.qml:414 is real, Main.qml:351 revealFile(fileId: real), while Main.qml:372 and :377 use fileId: int. `WEB_CLIENT_DESIGN.md:944` gives `file_id` as i64 and :1479 gives u64.

### U-010 [medium] Count fields typed as usize, u64 or i64 across layers — CONFIRMED

Counts are u64 (ledger/state.rs:65-67), i64 (lookup.rs:199, one()), usize (pair.rs:48 rebound and replaced), u64 with lost (inbox.rs:814), and usize for scan missing (scan lib.rs:279) while the other scan counts are u64 (:277, :281). `writes_remaining` is usize (queue.rs:194) while related rows are i64.

### U-011 [medium] The same integer conversion falls back to different values — CONFIRMED

The same conversion falls back to different values: inbox.rs:1084-1085 count saturates to u64::MAX, catalog inbox.rs:335 unread uses u64::MAX, rebuild.rs:20-21 count uses `unwrap_or(0`) for negatives, removal.rs:195 uses `unwrap_or(0`), hashes.rs:87-88 uses u64::MAX for the item count and 0 for the failed count, scan lib.rs:1002 uses u64::MAX, and desktop `facet_choices.rs:30-31` clamps negatives to 0 and saturates to i32::MAX while shell.rs:61 maps overflow to -1. The cited app/src/pairing.rs does not exist.

### U-012 [medium] Absent or unselected values use different sentinels — CONFIRMED

Absent values use -1 (`command_line.rs:455`, yandex.rs:99, `video_player_bridge.rs:16`, LogPanel.qml:27), -2 (Main.qml:78 loadedIndex), and 0 for zoom and mascot (`command_line.rs:465-472`). selectedSeq is a real -1 next to an int selected (LogPanel.qml:26-27).

### U-013 [medium] `Duration_ms` width, clamp and sentinel differ across model and catalog — CONFIRMED

model grid.rs:24 holds `duration_ms` as u32 with `NO_DURATION` = u32::MAX (:55), model analysis.rs:103-104 holds VideoFacts `duration_ms` and `poster_ms` as i64, and catalog browse grid.rs:65 clamps an i64 column into u32 with a fallback. `WEB_CLIENT_RECONCILED.md:420` lists the GridRecord fields with no duration, while DESIGN:1479 keeps `duration_ms` u32.

### U-014 [medium] Timestamp units and mtime meaning left implicit or mixed — CONFIRMED

model state.rs:15 says mtime: i64 with no unit, while platform listing.rs:19 documents it as FILETIME ticks (100 ns since 1601). file.rs:150 saturates on Windows with `unwrap_or(i64::MAX`). The cited catalog/src/failure.rs, library/src/crop.rs and `stmt_journal_probe.rs` do not exist, so those cites are dropped.

### U-015 [medium] `unix_now` defined three times with different fallbacks — CONFIRMED

`unix_now` is defined three times with the same body: app inbox.rs:1089 with `map_or(0`), borders lib.rs:41 with `map_or(0`), thumbcache maintenance.rs:32-33 with `map_or(0`). The fallback is 0 on a pre-epoch clock in all three, and the same `try_from(...).unwrap_or(i64::MAX`) appears in each. Duplicated, not divergent, so the title's 'different fallbacks' is not borne out.

### U-016 [medium] `process_age_ms` baseline differs by operating system — CONFIRMED

clock.rs:58-59 documents `process_age_ms` on Windows as the age since process creation, and clock.rs:95 gives the age since the first call elsewhere. Both arms document the difference. The same name measures different things, which the doc states.

### U-017 [medium] Benchmark timing helpers return milliseconds or seconds under one name — CONFIRMED

`bench_thumbs.py:212` timed returns milliseconds, `nvimgcodec_bench.py:56` returns ms per image, nvjpeg.py:217-219 returns seconds per item, sweep.ps1:54 converts seconds to ms, `scan_timing.py:55` prints seconds, and `cpu_worker.py:133` stores `build_s` in seconds. The gpu.py cite does not exist as eval/gpu.py; the file is under eval/jpeg-decoders/.

### U-018 [medium] Rounding policy differs in resize, decode and eval scale code — CONFIRMED

JPEG minimum-size fit uses ceil (codec decode.rs:228-229), resize uses round (resize.rs:34-35), and `linear_resize.py:129` uses Python round, which is banker's rounding, with a comment saying round half up. Three rules for the scaled size.

### U-019 [medium] 16-bit to 8-bit conversion uses two formulas by channel count — CONFIRMED

luma16 scales in f32 by 1/65535 (convert.rs:174-178), and rgb16 uses integer (v + 128) / 257 (convert.rs:187). The doc at :185 says round(v × 255 / 65535), which the integer form only approximates. Two formulas for 16-to-8 conversion.

### U-020 [medium] Frame count held as i64 in video and u32 in metadata and model — CONFIRMED

video ffi.rs:231 frames() returns i64, metadata container.rs:25 holds frames as u32 (defaulting to 1), and model analysis.rs:40 holds frames as u32. The `VALUE_PROVENANCE_IDEAS.md:99` cite is accurate.

### U-021 [medium] Rating out of range is rejected in some places and clamped in others — CONFIRMED

shell.rs:257 rejects out-of-range ratings via `u8::try_from` and Rating::new (None means nothing is rated). words.rs:14 clamps to 0..=5, client text.rs:33 shows past 5 as the best symbol, and metadata lib.rs:89 caps EXIF rating at 5 with min(5). Rejected in one place, clamped in others.

### U-022 [medium] Root, folder and file paths typed as String, RulePath or `Vec<u8>` — CONFIRMED

scan lib.rs:319-321 keeps ScanError::Busy with root and holder as String, while the walk uses RulePath. notice.rs:307 FileFailure.path is String, and notice.rs:452 ScanFailed root is RulePath. library state.rs:16-17 keeps `root_ids` and `root_paths` as RulePath. platform args.rs:18 converts Windows args lossily while the Unix form keeps exact bytes.

### U-023 [medium] Ext and Kind held as String in FolderRow but typed elsewhere — CONFIRMED

FolderRow stores ext and kind as String (catalog types.rs:361-362), while ScannedFile (:322), WorkItem and WriteTarget hold Ext and Kind. model media.rs:83 holds Ext as a u8 position in the lists.

### U-024 [medium] Unchecked arithmetic on unit conversions where others use checked — CONFIRMED

cli args.rs:54 shifts mb `<<` 20 with no `checked_mul`, and `TYPE_MISMATCH_VALIDATION.md:68` says so. cli borders.rs:163 uses `checked_mul`. platform memory.rs:35 uses `checked_mul` and volume.rs:42-43 uses `checked_mul`, while platform file.rs:182 does plain multiplication on secs × `10_000_000`.

### U-025 [medium] Screenshot wait in milliseconds, documented in seconds — REFUTED

The seconds-to-milliseconds conversion is made at the boundary: screenshot.rs:60 is seconds × 1000.0 and screenshot.rs:24 passes millis(wait) to the C++ `max_ms`. The user-facing --wait is in seconds as documented (`command_line.rs:147`, TestHooks.qml:27), and the C++ parameter name says ms. The units are consistent at each layer.

### U-026 [medium] Digit grouping and count formatting differ across surfaces — CONFIRMED

Hud.qml:58 concatenates raw numbers, GalleryGrid.qml:150-152 uses Qt.locale().toString for the same kind of count, and LANGUAGES.md:67 says Qt shows 1 234 while Rust shows 1,234 via `group_digits`.

### U-027 [medium] PDQ distance thresholds and metric written as literals in several places — CONFIRMED

join.rs imports `MATCH_DISTANCE` (:58) and uses it in the production path (:154, :724), but the tests use a literal 31 at :2389, :2406 and :2663. The doc at :4-5 states the radius as 31. eval/pdq.py:29 has `MEANINGFUL_D` = 31, eval/group.py:38 has `SHORTCUT_PDQ` = 22, and cli dups.rs:19 hard-codes `PDQ_BANDS` with 31 as its top bound. The Hamming threshold is written in several places.

### U-028 [medium] Verification thresholds written as percent and as fraction — CONFIRMED

VERIFIER.md:124 gives a changed region as 0.3% (percent) and median residual as 0.03 (fraction) in one sentence. VERIFIER.md:256 gives moved as about 1.2% and interior as about 15% while :276-278 use fractions 0.012 and 0.15. verify.py:556 uses fractions, with the comment's 0.05% matching 0.0005.

### U-029 [medium] CUDA kernels use different index and grid widths — CONFIRMED

`encode_gpu.cu:722` launches `k_dct` with a (unsigned)nb cast, while `grid_exact(nb`) (:532, used at :725 and :735) clamps with 64-bit math. `k_gather64` (:460-461) uses a uint32 index. `EGPU_MAX_DIM` is 65500u (`encode_gpu.h:23`) where JPEG allows 65535.

### U-030 [medium] Allocation accounting differs between the C and Rust PNG harnesses — CONFIRMED

`alloc_count.cpp:26` counts live bytes with `_msize` (the usable block size), while the Rust harness main.rs:37 counts layout.size() (the requested size), including realloc `new_size`. The two harnesses count different quantities.

### U-031 [low] Time constants carry units by suffix or by Duration inconsistently — CONFIRMED

thumbcache config.rs:67 `TOUCH_AGE_S` is an i64 of seconds, while `TOUCH_FLUSH` (:71) and `HEAL_INTERVAL` (:74) are Duration. app images.rs:30 `WRITE_WAIT` is a Duration and `DECODES_KEPT` is a usize. catalog inbox.rs:67 `DISMISSED_FOR` is i64 seconds with no suffix.

### U-032 [low] GPU-time projection to 70M in hours or days — CONFIRMED

MODELS.md:7 and :155 give GPU days for 70M, while :205 and :244 give GPU hours for 70M in the same document. `eval/bench_ann.py:186` prints GPU hours for the 70M projection. The cite path `eval/ann/bench_ann.py` is wrong; the file is `eval/bench_ann.py`.

### U-033 [low] Byte budgets typed usize or u64, and 16 MiB caps repeated — CONFIRMED

pipeline.rs:37-38 `buffer_bytes` is usize, governor.rs wraps AtomicUsize, and coordinator.rs converts a u64 to usize with `unwrap_or(usize::MAX`). container lib.rs:107 `GATHER_CAP` is u64 = 16 `<<` 20, and container.rs:139 `MAX_INFLATED` is usize = 16 `<<` 20 under another name. log lib.rs:79 `RING_BYTES` is a usize 4 MiB, and log file.rs:26 `FILE_CAP` is a u64 of 5 MiB.

### U-034 [low] Viewport zoom tolerance written two ways — CONFIRMED

viewport.rs:163 uses a relative tolerance fit × 0.0001, viewport.rs:171 uses the same 0.0001 as an absolute tolerance on (`fit_zoom` - 1.0), and viewport.rs:98 uses zoom `>=` 0.9999 for pixels().

### U-035 [low] Instruction and cycle counts shown in millions and as raw values — CONFIRMED

callgrind/1bb039084.md:19 shows Ir as 93.26 M and 23.3 K per file, while docs/perf/baseline.json stores raw integers. `pairs_probe.rs:77-78` divides cycles by `1_000_000` and prints them as M, while medians.py:27 prints whole M cycles and keep.py reports percent to two decimals.

### U-036 [low] CLI and report precision for seconds differs by output — CONFIRMED

cli lib.rs:214 prints pair timings with two decimals, lib.rs:233 prints secs with one decimal, and report.rs:102, :113 print progress seconds with zero decimals. The cited lib.rs:205 is a return, not the timing print, so the cite is off, but the precision differs by output.

### U-037 [low] Numeric literal style and named constants used inconsistently — CONFIRMED

date.rs:54 writes days × `86_400` + h × 3600 + mi × 60 + s, grouping `86_400` but not 3600. metadata tiff.rs:119 uses bare TIFF type literals 1|2|6|7 etc., while named `TIFF_*` constants exist (:25-31).

### U-038 [low] JPEG width accepted as u32 in dimensions but u16 in decode — CONFIRMED

codecs/jpeg.rs:30-31 dimensions() narrows to u32 with `try_from`, and jpeg.rs:59 decode() narrows to u16 with the comment that a JPEG's sides fit 16 bits. The same dimension has two width types.

### U-039 [low] Encoder quality parameter types differ by codec — CONFIRMED

encode.rs:32 JpegEncoder::new takes quality as i32 on libjpeg-turbo's 1-100 scale, while encode.rs:96 `WEBP_LOSSY_QUALITY` is an f32 90.0 on libwebp's 0-100 scale. Different integer and float types and scales.

### U-040 [low] Tree anchor bits saturate at 63 and alias — PARTIAL

The aliasing is real: tree.rs:69-70 bit(rank) saturates at 63, so anchors 64 and later share bit 63. But the doc at :67-68 states it and gives the reason (no machine has that many drive letters), so it is a documented limit, not a contract violation.

### U-041 [low] Fixed-point scales in SQL sort and filter keys — CONFIRMED

query lib.rs:109-111 compares shape with values scaled by 100, and lib.rs:383 sorts AspectRatio as CAST(1e6 × a.width / a.height AS INTEGER), in millionths, while the Borders sort also uses millionths. Two fixed-point scales in one module.

### U-042 [low] Similarity band upper bound differs between eval tools — CONFIRMED

eval/ann/metrics.py:20 has BANDS ending at 1.01 as edges, while `eval/bench_ann.py:194` has BANDS as (lo, hi) tuples ending at 1.0 (:194 is 0.8, 1.0). The cite path `eval/ann/bench_ann.py` is wrong. The two tools cut the similarity range differently.

### U-043 [low] Hand-written mouse wheel notch divisor — CONFIRMED

WheelListView.qml:50, Viewer.qml:306 and YandexPreview.qml:65 each divide angleDelta by the literal 120 with no named constant.

### U-044 [low] ThumbHeight construction uses bare and unwrapped literals — CONFIRMED

bench catalog.rs:45 defines `THUMB_HEIGHT` as u32 = 256 and wraps it in ThumbHeight::new. bench linux.rs:59 passes 256 as a bare literal to maker.make. library runner.rs:216 uses ThumbHeight::new(64).expect, while lanes.rs:75 uses .unwrap() on the same constructor.

## naming

### N-001 [high] Web client design doc and reconciled doc disagree on wire-level names — CONFIRMED

`WEB_CLIENT_DESIGN.md` contradicts itself on the session cookie: :182 says SameSite=Strict, :715 and :1336 say Lax, :1691 rejects Strict. `WEB_CLIENT_RECONCILED.md:165` and :545 say Lax. Its cookie is `__Host-gallery_session` at :182 and `__Host-pg_sign` at :340. The CSRF header is X-Gallery-Request (design :1689), X-Pg-Client (reconciled :166) or X-Pg-Bundle (reconciled :59). Error codes are `snake_case` in design :1568 and kebab-case in reconciled :313 and :170.

### N-002 [high] Web server on-disk file names and lock location differ between docs — CONFIRMED

The server lock is `<home>/server.lock` at design :377, `<catalog>.server.lock` at :1754, and `<catalog_dir>/server.lock` at reconciled :515. The state database is server.sqlite at design :330 and server.db at design :31, :798 and reconciled :512. The design doc uses both names. The token and json files are at reconciled :308 and design :798.

### N-003 [high] Tool script names disagree across docs for the same gate and the same tool — CONFIRMED

`tools/perf_check.py` and tools/perf-compare.py both exist. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:57` names `perf_check.py` as the 1% gate. `SCAN_OPTIMIZATION_PLAN.md:1020` names perf-compare.py as the scan gate. tools/README.md:24 describes perf-compare.py as the comparison, which `perf_check.py` calls. budgets.json:2 names both. tools/lint-qml.ps1 does not exist; the file is `tools/lint_qml.py`, which REVIEW.md:68 and `REFACTOR_LEDGER.md:149` cite as the old name.

### N-004 [medium] CLI flag spellings for the same action differ across docs and code — CONFIRMED

The code's flag is --retry-errors (cli args.rs:90-92), and `IMPORT_FAILURES_PLAN.md:480` uses that. The doc comment at crates/catalog/src/sql.rs:6-7 says --retry, which is not a flag. The --again and --no-dups flags match the code. `DECODE_ONCE_PLAN.md:1132` sketches the field as `no_duplicates`, which is a design sketch and not the shipped name.

### N-005 [medium] Crate and package names are written three ways across code and docs — PARTIAL

crates/dedup/Cargo.toml names the package pg-dedup, so dups.rs:2 is right to write the hyphenated package name. The hyphen and underscore split is the Rust convention: package names are hyphenated and imports underscored. The real mixing is in tools/crate-layers.txt:32 (jobs-testkit against bare names). The finding's manifest instances (pg-cli, pg-log, pg-metadata, pg-model, pg-edit) are consistent.

### N-006 [medium] Notice name for a refused folder differs between the error map and the test — REFUTED

Both notice variants exist (notice.rs has FolderNotAdded and FolderRefused). error.rs:22 correctly names the refusal notice FolderRefused. The test at app.rs:274 posts FolderNotAdded as a generic sample notice about the shutdown, not as a refused folder, so it is not a naming mismatch.

### N-007 [medium] Found::Offline name does not cover undecodable originals — CONFIRMED

images.rs:59-66: Found::Offline(Error, `Option<ThumbHeight>`) is documented as covering the original that can't be read (offline) or that doesn't decode. The name covers only half of what the variant holds, as the finding says.

### N-008 [medium] Two different types named Listing are imported side by side in the app — CONFIRMED

Two types named Listing are in the pg-app crate: inbox.rs:149 (the inbox's listing) and `pg_model::view::Listing`, imported at views.rs:8 (the saved session listing). Both are real and in use in one crate.

### N-009 [medium] Scratch folder base names are reused across unrelated tests — PARTIAL

testkit's `run_folder(scratch`, prefix) makes distinct folders per call, and platform/src/testing/tests.rs:286 tests that calls made together are distinct. Reusing a scratch base is therefore a grouping label, not a collision. The real inconsistency is that `scratch_rate` is shared by unrelated tests (crop, `crop_tags`, `crop_roots`, writeback) while `scratch_crop` and `scratch_cli` are used elsewhere.

### N-010 [medium] Stored turns count is called rotation in some places and turns in others — CONFIRMED

FileState.rotation (model state.rs:16) is the stored turns, and FileState.turns (state.rs:21) is the pending turns. write.rs:31 says the Turns write kind is stored under the key rotation. The same concept has two field names. failure.rs:18 (the finding's cite) does not exist, so that instance is wrong, though the rest holds.

### N-011 [medium] Catalog schema object names use inconsistent prefixes and are written in two files — CONFIRMED

Prefixes are mixed in baseline.sql: the table is `file_failures` (:120), the index `file_failures_live` (:233), the triggers `files_failures_stale` and `files_failures_hidden` (:433, :440), and `file_failures_stamp` (schema.rs:380). The triggers `files_changed` (:424) and the index `files_changed` (:241) share one name.

### N-012 [medium] Thumbs output version key and constant are named three ways — PARTIAL

The three names are for different things, which the finding overstates. meta '`thumbs_version'` (baseline.sql:463) is the thumbnailer's version, and `THUMBS_VERSION` is its constant (`pg_render`). '`thumb_output_version'` (thumbcache schema.rs:156) is the cache's output format, and `THUMB_OUTPUT_VERSION` is its constant. writer.rs:393 is a comment, not a key, so that instance is wrong. The similar names for different values are a real hazard.

### N-013 [medium] `PLACEHOLDER_VERSION` is imported from the analysis module, not the version registry — REFUTED

model/src/analysis.rs:134 re-exports `PLACEHOLDER_VERSION` from `crate::versions::PLACEHOLDER_VERSION` (versions.rs:29). The import from `pg_model::analysis` is a documented re-export of the registry value, not a separate definition.

### N-014 [medium] Perf span names mix dotted and undotted forms — REFUTED

The documented rule (perf lib.rs:182) is for span names such as scan.walk and phase.import. The undotted 'pipeline' at perf lib.rs:587 is a record name (jobs pipeline.rs:170 records it with `pg_perf::record`), and a single word has no dot to add. The span names checked (pass.scan, scan.setup, pending.placeholders, thumbs.pending) are all dotted.

### N-015 [medium] Budget keys and baseline keys use different separators for the same metrics — CONFIRMED

budgets.json uses scan-walk.again, scan-walk.first and pending-thumbs (hyphens). baseline.json uses `scan_walk.again` (:20) and `pending_thumbs.at_height` (:12) (underscores) for the same benches.

### N-016 [medium] Test names follow several styles within the same crates — CONFIRMED

Test names in the same crates mix styles: bare nouns (borders.rs:324 `too_deep`), sentences (borders.rs:383, analyze.rs:221), a `_windows` and `_posix` suffix pattern (catalog browse tests) and a plain-name test with a comment (browse/read/tests.rs:99). The mix is real within crates, though low severity.

### N-017 [medium] QML user-action signal names use several verbs for the same kind of event — CONFIRMED

QML user-action signals use playAsked, volumeAsked, rateAsked, borderMarkAsked, copyRequested, dismissRequested, retryRequested, writeBackToggled, detectBordersToggled, recursiveToggled and toggleRequested. The same kind of event is named with Asked, Requested and Toggled.

### N-018 [medium] C++ member naming mixes trailing underscore and `m_` prefix — CONFIRMED

C++ members use a trailing underscore (`frame_sink.cpp:10` `state_`, `video_source.h` `player_` and `device_`) and an `m_` prefix (providers.cpp:124 `m_kind`, `gui_app.cpp:53` `m_text`, `live_reload.cpp:32` `m_prefix`). Both conventions appear in one desktop/cpp tree.

### N-019 [medium] `cxx_name` renames are applied unevenly across bridges — CONFIRMED

`cxx_name` renames are applied unevenly. `library_bridge.rs:25` renames writes to writeStatus and `write_failed` to writeFailed, while roots and busy have none. `notices_bridge.rs:37` renames `toast_level`. `viewport_bridge.rs:52` renames `zoom_at` to setZoom. `facet_choices_bridge.rs:32` has no `cxx_name`, and `shell_bridge.rs:169` has redundant renames that equal the Rust name.

### N-020 [medium] Bench folder root literals differ between bench cases — PARTIAL

catalog.rs:121 and :241 use different literal roots: `r'PhotoGallery\synthetic\library'` for the list bench and `r'D:\bench\library'` for the scan bench. These are separate fixtures and may be meant to differ. The duplicated literal is a style issue, not a clear error.

### N-021 [medium] Environment variable prefixes are mixed across tools and build scripts — CONFIRMED

tools/doctor.ps1:71 reads `PG_FFMPEG` and `PG_EXIFTOOL`, while doctor.ps1:49 reads `FFMPEG_DIR`, and the same script's `QT_DIR`, `LIBCLANG_PATH`, `LLVM_DIR`, `PIN_ROOT` and `PIN_EXE` are unprefixed. eval/jpeg-decoders/linux.py:12 mixes `JPEG_*` and `PG_*` in one environment list. `imagereduce_switch.py:14` mixes `IMAGEREDUCE_VIPS_BIN`, `JPEGLI_BUILD` and `PG_ROOT`.

### N-022 [medium] Temp-file suffixes differ by write kind and between golden data and code — CONFIRMED

The notice fixtures use a.jpg.pg-tmp (golden.jsonl:18 and notice.rs:1397). The code's temp suffixes are .rating.pg-write and .turns.pg-write (writeback file.rs:60-62, queue.rs:562, removal.rs:576), and the cut temp is .cut.pg-write (inplace.rs:83). No code path produces .pg-tmp, so the fixture names a suffix the app never writes. The golden's folder-removal temp is a.jpg.pg-write, which matches the code.

### N-023 [medium] Notification design doc names differ from the code — PARTIAL

`NOTIFICATIONS_DESIGN.md` is a design doc that records its own overrides: each 'Cache events (overrides ...)', 'Keeper close (overrides ...)' and 'Dev mode (overrides ...)' bullet says which earlier name it replaces, and the overridden names are called out. The differences the finding lists are therefore declared, not accidental. The doc has no stale claims left unmarked at the instances checked.

### N-024 [medium] Eval folder and label names for similar-but-not-duplicate pairs disagree — PARTIAL

The same set is named differently by tool: eval/viewer.html:152 says Hardest negatives, verify.py:100 uses `similar_folder`, evaluate.py:130 uses `hard_negative_pairs_flagged`, and evaluate.py:209 uses similar-folder pairs flagged. The folder on disk is `similiar_but_not_dups` everywhere (common.py:91, `TEST_SET.md:29`), so that spelling is consistent, though misspelled.

### N-025 [medium] Eval key and kind names differ between the eval scripts — CONFIRMED

`bench_ann.py:207` returns recall10 and metrics.py:165 writes `recall_at_10` for the same measure. score() is defined twice with different signatures: `bench_ann.py:201` and metrics.py:129. The kinds are labelled dups2 in metrics.py:21 and duplicate in run.py:488. The report.py:109 cite does not exist, though report.py:120 uses `recall_at_10`.

### N-026 [medium] Historical plan and reference doc references are stale — REFUTED

`NTFS_MFT_SCAN_PLAN.md:779-781` is the plan's migration checklist '(Doc references to `pg_scan` change with the move'), so naming `pg_scan` there is intentional. PROFILING.md:350 names scan-timing.sh and then gives `tools/profiling/scan_timing.py` as its successor, which is an explicit history note.

### N-027 [medium] Module file name does not match its main type — PARTIAL

No type Coordinator exists in the library crate. The main type in coordinator.rs:130 is Lanes, and coordinator.rs:731 has a fn coordinate. `OPEN_ITEMS.md:157` cites `Coordinator::request_crop`, which does not exist, so that doc is stale. lanes.rs:65 has pub struct Group, which is a separate concept in a separate file. The file name itself is defensible.

### N-028 [low] Action and event names differ across the client controller layers — PARTIAL

ListShown appears in three enums (controller.rs:83, input.rs:17, start.rs:286), which are in different layers and carry the same event name for the same fact. Event::FilesChanged mapping to Command::RefreshTurned (controller.rs:327) and Event::Reload mapping to `open_list` (controller.rs:326) are real naming gaps. The repeated ListShown is layering, not a clash.

### N-029 [low] Same type name used for different types in different modules — CONFIRMED

The same type name names different types: Writing in thumbs/lib.rs:178 and thumbcache schema.rs:255, Status in model yandex.rs:85 and model query.rs:42 (both in `pg_model`), Blur in detection.rs:48 and :69, and Grid as an enum in keys.rs:37 and a struct in client grid.rs:111. Each is in its own module, so this is a readability cost, not a compile error.

### N-030 [low] Accessors for stored codes are named differently across the model — CONFIRMED

Accessors for stored codes differ: detection.rs and media.rs Kind use key() and `from_key(`), format.rs:46 uses name(), media.rs:96 Ext::name, and notice.rs:790 uses code() on Op. Those pairs are real, if not wrong. Resolver::name (notice.rs:753) returns a String, which differs in shape from the others.

### N-031 [low] Decision, finding and item identifiers are numbered inconsistently in the plans — PARTIAL

Each plan numbers its own decisions, so Q3 (`BACKGROUND_JOB_AUDIT_FIXES`) and Q-S1 (`BOUNDED_MEMORY`) differ by plan and are not wrong. D1 and D2 do appear in both `CATALOG_SIZE_PLAN` and `BOUNDED_MEMORY_PLAN` with different meanings, which is real. `REFACTOR_LEDGER's` skipped F-56 and F-57 are a gap, and REVIEW.md:19 says IDs are never reused, so the gap is consistent.

### N-032 [low] Decoder and feature names differ in small ways across the crates — PARTIAL

Naming here is style variance, not meaning drift: container `png_chunks`, `png_` and `jpeg_` prefixes against unprefixed riff and find, and codecs.rs:43 writing ::png::Scratch with a leading :: while the webp type has none. The features (pg-codec fixtures, pg-catalog fixture) are documented in crates/README.md:138 as a deliberate test-support rule.

### N-033 [low] Product and data-folder spellings differ across docs and code — PARTIAL

The product is named four ways in LANGUAGES.md:85 (Photo Gallery, gallery, pg-gallery, PhotoGallery), and the spelling of the UI title is documented there. layout.rs:151 and :154 assert different default folders per platform (PhotoGallery on Windows, photo-gallery under .local/share on Linux), which follows platform convention and is intended. eval/viewer.html:6 is a separate tool titled NDD Bake-off.

### N-034 [low] Inbox method names for the same operations differ by layer — CONFIRMED

Inbox operations are named singular at the app layer and plural at the store layer: `thumb_failed` and `thumb_made` (inbox.rs:509), `thumbs_failed` and `thumbs_made` on Store (inbox.rs:641), and ThumbsFailed and ThumbsMade on Step (inbox.rs:718). Thumb::Failed and Thumb::Made (inbox.rs:214) add a third form.

### N-035 [low] Phase and pass labels in code comments are stale project labels — PARTIAL

Comments cite project labels a reader can't resolve: roots.rs:557 'N-13', schema.rs:85 'Phase 4', schema.rs:624 'N-13'. The finding's work.rs:909 'Phase 3' cite is past EOF (the file has 761 lines), so that instance can't be checked. The labels are project tracking names, stale for code readers, though not wrong.

### N-036 [low] Dups pass and owed concept have several names across the plans — CONFIRMED

`DECODE_ONCE_PLAN.md:531` names the pass Dups (Phase::Dups) and also FilePass::Duplicates in the same bullet. `IMPORT_FAILURES_PLAN.md:74` decides FilePass names Duplicates and Placeholders. The code's FilePass (notice.rs:375) has only Thumbs, Borders and Import, so those names are planned and not built. Within the plans, Dups and Duplicates name the same pass.

### N-037 [low] Plan documents use the same word for different concepts — CONFIRMED

'Placeholder' names two concepts: the cloud-file or reparse placeholder (`OPEN_ITEMS.md:143-144`, dir.rs:410) and the ThumbHash placeholder (crates/README.md:435-445, analysis `PLACEHOLDER_VERSION`). The same word is used for both in the plans and the code.

### N-038 [low] QML dialog title and removal keys are not uniform — CONFIRMED

Dialog titles are ui-removal-discard-title (DiscardRemovalDialog.qml:32), ui-remove-folder-title (RemoveFolderDialog.qml:35) and ui-cut-borders-title (CutBordersDialog.qml:30). Buttons are ui-removal-discard (LibraryPanel.qml:101) and ui-remove (LibraryPanel.qml:69). The removal and remove naming differs for the same kind of action.

### N-039 [low] Fluent id prefixes and digit suffixes are not uniform — CONFIRMED

cli.ftl ids all begin cli-; text.ftl ids have no file prefix (text.ftl:1 is the header). Hint ids use digit suffixes, as in keys.rs:273 ui-hint-up-4 and :274 ui-hint-down-1. The mix is real but is documented per file.

### N-040 [low] Constants and helper names differ in small ways across tools and crates — PARTIAL

sort.rs:7 uses KEYS where facets, panels, keys and input use ALL, a real split. The others are cross-language or unrelated: Get-Median (sweep.ps1:30) against Median (`few_sweep.ps1`), `sha256_of` (`setup_ffmpeg.py`) against Get-Sha256 (PowerShell), and `FILE_FLAG` constants in `png_read_bench.rs` against bare literals in `jpeg_read_bench.rs`. Those are different languages or tools, so they are not a codebase inconsistency.

## versions

### V-001 [high] Thumbnail cache doc says there is no pipeline version, the code stores one — CONFIRMED

`THUMBNAIL_RESAMPLING.md:31` (T-01) says the cache stores no filter or pipeline version. thumbcache/schema.rs:156 writes meta `thumb_output_version` and :211 refuses a newer one, so the doc's statement is stale at the cache level. T-01's narrower gap (no per-entry version in `get_nearest`) is not checked here. schema.rs:211 and the doc's claim are as described.

### V-002 [medium] Replay recording format: u32 plan item and duplicated mismatch and missing-format text — CONFIRMED

replay.rs:13 re-exports the registry's i64 `REPLAY_FORMAT_VERSION`; :28 writes it into Init.format; :46 missing-format text; :52 and :69 repeat the mismatch text (parse and replay). tests/replay.rs:57 tests formats 0 and 2. The title's u32 is stale: the constant is already i64, so only the duplicated text remains.

### V-003 [medium] Bench catalog LAYOUT strings are hand-written literals — CONFIRMED

catalog.rs:36 `BENCH_LAYOUT='v1-windows-rules'` and :40 `BENCH_LARGE_LAYOUT='v1-windows-rules-large'` are hand-written literals. The cited doc is at lines 32-35, not 7-9 as the finding says.

### V-004 [medium] `CATALOG_SIZE_PLAN` quotes a bench LAYOUT value the code does not have — CONFIRMED

`CATALOG_SIZE_PLAN.md:312` quotes LAYOUT = v10-windows-rules to bump. The code has `BENCH_LAYOUT` = v1-windows-rules (catalog.rs:36). Line 43 cited in the plan is RULES, not a layout constant.

### V-005 [medium] Test code hard-codes thumbs, `user_version` and output-version literals — CONFIRMED

thumbcache lib.rs:465, 479, 1072, 1085, 1342 pass literal 1 as the thumbs version; :746 sets `user_version` = 7; :757 sets `user_version` = 0; :787 writes literal 0 to `meta.thumb_output_version`. `tests/other_process.rs:49` and :61 pass literal 1; writeback/src/lib.rs:806 passes literal 1.

### V-006 [medium] Test constructors for Versions differ: Versions::current, versions(), `pg_library::versions(`) — CONFIRMED

bench catalog.rs:72 fn versions() returns Versions::current(). pair.rs:583 and rehearsal.rs:21-23 do the same; rehearsal.rs:20 doc names `pg_library::versions`. `other_process.rs:12` local fn. library/src/runner.rs:273 calls crate::versions(). library/tests/pairs.rs:65 overrides import on `pg_library::versions(`). borders.rs:33 and lanes.rs:59 cited without a directory are library/tests/borders.rs:33 and library/tests/lanes.rs:59, both calling `pg_library::versions(`) as stated. Claim holds; paths incomplete.

### V-007 [medium] Version constants imported from different paths for the same value — CONFIRMED

lifecycle.rs:485 (`pg_model::versions::IMPORT_VERSION`), :618 (`pg_analysis::analyze`), :500 (BORDERS/THUMBS from `pg_model::versions`). raw.rs:12-14 mixes `pg_analysis::analyze`, `pg_analysis::borders` and `pg_model::analysis`. `test_support.rs:7` `pg_analysis::analyze`. writers.rs:921 imports `IMPORT_VERSION` from `pg_analysis::analyze` (finding says :920). import/src/lib.rs:168 passes `pg_analysis::analyze::IMPORT_VERSION`. `PAIRING_VERSION` is not at writers.rs:1711; it is used as `pg_model::pdq::PAIRING_VERSION` at :1742 and :2015/:2018, so the line is wrong but the source is as stated.

### V-008 [medium] Bump rule restated in five places with different wording — CONFIRMED

Five restatements read: analyze.rs:23-25 header-only import not bumped; params.rs:3 bump when Params changes; ledger.rs:80 `THUMBS_VERSION` by hand; `NOTIFICATIONS_DESIGN.md:375` THUMBS by hand, codec upgrades not detected; `DECODE_ONCE_PLAN.md:23` rule rewritten. The wording differs in each.

### V-009 [medium] Schema and migration numbers in docs and code comments contradict each other and the registry — CONFIRMED

Stale numbers confirmed: `CATALOG_SIZE_PLAN.md:5` (v20), PLAN.md:315 (version 11, ten steps), `SCAN_OPTIMIZATION_PLAN.md:57` (ends at 15), `DECODE_ONCE_PLAN.md:400` (`TO_10` next), `IMPORT_FAILURES_PLAN.md:48` and :376 (`TO_18`), reader.rs:19 (schema 19), schema.rs:82 (MIGRATIONS empty). Path error: `REFACTOR_LEDGER` is `docs/plans/REFACTOR_LEDGER.md:128`, not docs/reference. crates/README.md:297 says version 1, which matches the registry, so it is not a contradiction.

### V-010 [medium] Historical migration and backup references in the named plan docs — CONFIRMED

PROFILING.md:510-511 (pre-v9.bak before migration v9), :619 (migration 13 from a 209.4 MB v12 copy) and :985 (migration 14 introduces `meta.pairing_epoch`) are as cited. `NTFS_MFT_SCAN_PLAN.md:296` and :544 and `BACKGROUND_JOB_AUDIT_FIXES.md:32` match too. The numbers are historical and unmarked as such.

### V-011 [medium] `IMPORT_VERSION` HEAD values in plan docs do not match the registry — CONFIRMED

`EXIFTOOL_PORT_PLAN.md:77-78` says `IMPORT_VERSION` at HEAD is 2 (bumped 2026-11-13); `IMPORT_READS_PLAN.md:12` says HEAD raised it `2->3`; `THUMBNAIL_RESAMPLING.md:8` lists it as done at 2; `NOTIFICATIONS_PLAN.md:14`; `DECODE_ONCE_PLAN.md:436` `DUPS_VERSION` beside `IMPORT_VERSION`. versions.rs:13 is 1. These are dated snapshots, so 'stale' is right against the current registry, which the reset (plan 3.6) changes.

### V-012 [medium] Bundled SQLite version stated three ways — CONFIRMED

`INCREMENTAL_SYNC.md:278` measured on SQLite 3.49.1; `WEB_CLIENT_RECONCILED.md:499` says bundled sqlite3.c 3.50.2; scan-optimizations-validation.md:64 says 3.54.1 bundled by the rusqlite fork. Three versions are named, each in a different doc.

### V-013 [medium] Tool text restates pinned versions as literals — CONFIRMED

doctor.ps1:46 and :52 and doctor.py:121 and video/build.rs:112 repeat literal pins as text (Qt 6.12.0, FFmpeg 9.0). doctor.py:113 reads `qt_version` from the Justfile, so the Qt case is already handled there. `build_static_ffmpeg.py:34` `FFMPEG_RELEASE` = 9.0.2. The literal text agrees with the pins today; the duplication is real.

### V-014 [medium] Perf log program identifiers differ across binaries — CONFIRMED

cli/src/lib.rs:109 perf meta program 'gallery'. desktop/src/trace.rs:31 perf meta program 'gallery-app'. desktop/src/lib.rs:148 log header 'gallery {version}'. Three different program identifiers for the same binary family.

### V-015 [medium] Eval scripts disagree on the default model name — CONFIRMED

`check_groups.py:43` and group.py:159 default `sscd_fp16`. ann/data.py:16 MODEL = `sscd_trt_fp16_trim_draft`. embed.py:39 builds the name from --model plus `_trim` and `_draft`. The name is built in different ways.

### V-016 [medium] TensorRT major version stated inconsistently in eval — CONFIRMED

`bench_semantic.py:7` docstring says TensorRT 11 fp16 (finding's :5 is 2 lines off). :70 builds the plan name from `accel.trt_major(`). accel.py:40 `TRT10_DIR` defaults to tools/tensorrt-10.16.1.11-cu13. The docstring's major version disagrees with the runtime folder.

### V-017 [medium] Two different dates given for the same default-off switch — REFUTED

verify.py:653 says the --strong-inliers default has been off since 2026-10-29. viewer.html:269 says hard-negative pairs have mostly matched since 2026-10-28. These are different facts with different dates. The cited verify.py:83 and :91 contain no date, so 'since 2026-10-29' is not repeated there.

### V-018 [medium] Pairing version seed is a literal in baseline.sql while the other marks seed 0 — CONFIRMED

baseline.sql:455/458/463 seed borders, import and thumbs at 0, and :460 seeds `pairing_version` at 1. schema.rs:43 and :55 state the literal and assert it. versions.rs:19-20 says the baseline seed is this value. The difference is intentional (D2 in the plan), so 'inconsistent' describes a deliberate exception.

### V-019 [medium] Thumbnail cache newer-version handling — REFUTED

Instances are accurate (schema.rs:202 and :211 return Found::Newer; :231 starts over an earlier or missing output version). The policy is uniform: both newer markers are refused and left alone. The start-over for an earlier version is by design (D9). No inconsistency shown.

### V-020 [medium] Preferences file has no format version and migrates a float on read — CONFIRMED

prefs.rs:19-20 says missing fields take defaults and unknown fields are ignored (serde default at :22). :88-89 rounds and clamps a legacy float `row_height` on read. GOLDEN at :212 has no version key.

### V-021 [medium] `THUMBS_VERSION` is wired into the thumb cache at five call sites — PARTIAL

Production site lib.rs:39 (`open_thumbs`) and test call runner.rs:370 are correct. The cited src paths are wrong: library/src/video.rs and writeback.rs do not exist, and library/src/lanes.rs:73 has no `THUMBS_VERSION`. The matching calls are in library/tests/lanes.rs:73, tests/video.rs:241 and tests/writeback.rs:103, so the line numbers hold but the directory is wrong. The duplication is real.

### V-022 [low] Catalog lowers newer marks while refusing newer schemas — CONFIRMED

core.rs:138-143 refuses found `>` current as made by another build (the finding's :136 is the enclosing match, 2 lines off). core.rs:227-237 sets lowered marks back and logs them via `pg_log::warn`.

### V-023 [low] `key_version` mark name differs between plan and code — CONFIRMED

`CATALOG_SIZE_PLAN.md:66` says `meta.name_key_version` holds the table hash plus a key-format number. The code writes meta `key_version` (lifecycle.rs:231; core.rs:157 reads '`key_version'`). The name differs; the finding's 'plan' side is the wrong name.

### V-024 [low] Test fixture uses a `taken_zone` value that is not in the real format — CONFIRMED

lifecycle.rs:1390 writes literal 'another-zone' to `meta.taken_zone`. :1397 asserts the real fingerprint has 4 comma fields. :1388 comment states a UTC system's fingerprint is '0,0,0,0'.

### V-025 [low] Test pins a decode cap value owned by `pg_codec` — CONFIRMED

edit/src/rewrite.rs:60 asserts `pg_codec::max_decode_bytes(`) == 512 `<<` 20. The test pins the owner's value as a literal, though the comment says it is the decoders' own cap.

### V-026 [low] Win32 enum values hand-copied, with a test in only one file — CONFIRMED

platform/src/dir.rs:137-140 declares `DRIVE_REMOVABLE/FIXED/REMOTE` as local literals. grep finds no test in dir.rs comparing them with windows-sys. drive.rs:170-178 `BUS_*` literals; the windows-only test at drive.rs:739-742 asserts them against `fs::BusType*`, as the finding says.

### V-027 [low] Image cache key format defined in two desktop files — CONFIRMED

desktop/src/gallery/items.rs:39-40 documents the ImageKey format '`<file` `id>@<mtime>.<rotation>.<pending` `turns>'`. desktop/src/providers.rs:1-2 documents the same layout as '`<file` `id>@<mtime>.<rotation>.<turns>'` in a second place.

### V-028 [low] Qt version claims outside the READMEs — REFUTED

The Qt prose matches the pin. Justfile:105 sets `qt_version` := 6.12.0. site/index.html:49 and `tst_gridcell_fade.qml:3` say Qt 6.12. Nothing is inconsistent. The cited tools/static-qt/.. path is not a real file.

### V-029 [low] FilterBar facet numbers share integers across two namespaces — CONFIRMED

FilterBar.qml:46-53 list facets use integers 0-5 (type through exts), order pinned by tests/facets.rs. :202-219 range Bound facets use 0-4 (size, megapixels, duration, and two dates) with the same integers in a second namespace.

### V-030 [low] nvJPEG header version differs from the pinned wheel — CONFIRMED

nvjpeg.py:6 says declarations come from nvjpeg.h 13.2. requirements.txt:8 pins nvidia-nvjpeg==13.3.2.43. The stated header and the pinned wheel differ.

### V-031 [low] Batch byte budget duplicated in two eval scripts — CONFIRMED

`nvimgcodec_bench.py:23` and nvjpeg.py:36 both define `MAX_BATCH_BYTES` = 2.0e9 with a comment about the GPU's memory. The constant is repeated, with slightly different comments.

### V-032 [low] Library Cargo.toml calls versions a module — PARTIAL

library/Cargo.toml:11 lists versions among items, and the comment mixes module names (runner, folders, scans) with functions (`open_catalog`, `open_thumbs`, versions). lib.rs:44 has pub fn versions(), so it is a fn. The manifest never calls it a module; the finding's reading is an inference. The line is stale only in that ambiguity.

### V-033 [low] Version owner named inconsistently in docs — CONFIRMED

ledger.rs:80 says `THUMBS_VERSION` (`pg_render`) bumped by hand; :84 names `BORDERS_VERSION` with no crate. lifecycle.rs:393 names `pg_render::THUMBS_VERSION` while the import is from `pg_model::versions` (:500). thumbcache/schema.rs:379-380 names `pg_render::THUMBS_VERSION` and `pg_catalog::Versions::thumbs`.

### V-034 [low] `cache_version!` macro duplicates the registry value as a literal — CONFIRMED

config.rs:12-16 macro `cache_version!(`) yields the literal 1 (:14). :19 `CACHE_VERSION` is the registry re-export and :21 is the const assert that ties the literal to it.

### V-035 [low] Tools keep legacy version layouts alongside current ones — CONFIRMED

`set_owed.py:6` (cited as :7) names the draft v12 `dups_state` layout and the v13 `pairs_owed` layout. :33 picks the layout by table presence. Both layouts are still handled.

### V-036 [low] Compare-shots manifest reads a legacy key — CONFIRMED

`compare_shots.py:275-277` reads the legacy fixture key or the new fixtures dict. :453 writes fixtures. The legacy read is kept alongside the new write.

### V-037 [low] ExifTool version given as an example and a probe version in code — PARTIAL

`install_exiftool.py:8` uses --version 13.59, which matches the Justfile pin (`exiftool_version` 13.59.0, `exiftool_source_version` 13.59). The 13.50 probe at exiftool/src/lib.rs:187 is a dated test note, not a conflicting pin. So the 'inconsistent' example does not hold; the probe note is only a historical reference.

### V-038 [low] Gate baseline commit named two ways — PARTIAL

PROFILING.md:401-402 says the baseline was last re-measured at e7576f0. `REFACTOR_MEASUREMENTS.md:82` says HEAD's counts are baseline.json at a22fbad, in a dated 2026-11-04 section. Both are accurate for their dates, so they are not contradictory. The doc does not say which is current.

### V-039 [low] Argon2 implementation named differently between design and measurement — PARTIAL

`WEB_CLIENT_DESIGN.md:174` proposes the argon2 Rust crate 0.5.3 with m=65536, t=3. `TECH_DECISIONS.md:16` states the measurement used argon2-cffi 25.1.0 (Python). The library difference is stated in `TECH_DECISIONS`, so 'undocumented' overstates it; only the design has no pointer to the measurement's library.

## persisted-formats

### PF-001 [high] PDQ dihedral blob size documented as 256 bytes but stored as 128 — CONFIRMED

baseline.sql:30 says 8 orientations of 32 bytes (256). pdq.rs:8 ORIENTATIONS = 4 and pdq.rs:118 `BLOB_BYTES` = ORIENTATIONS × `HASH_BYTES`, `HASH_BYTES` = 32 (pdq.rs:4), so the stored blob is 128 bytes. The `DECODE_ONCE_PLAN:526` figure of 256 MB also assumes 8. The `grow_catalog.py:78` cite is at `tools/profiling/pairing/grow_catalog.py`.

### PF-002 [high] ExFat refusal spelled 'ex-fat' in serde and 'exfat' in code() — CONFIRMED

refusal.rs:8 serde uses `rename_all` kebab-case, so ExFat serialises as ex-fat, while refusal.rs:79 code() returns exfat. Both are strings for the same variant; the serde one is the persisted tag.

### PF-003 [high] Unrated stored two ways and ratings CHECK forbids the file-side form — PARTIAL

Unrated is stored two ways: the ratings table (baseline.sql:81) has CHECK BETWEEN 1 AND 5, so the user side deletes the row for 0 stars (writers.rs:545-546), while `file_ratings` (baseline.sql:85-88) allows 0..5 and takes Some(value) unchanged. The title's claim that the ratings CHECK forbids the file-side form is muddled: the CHECK on ratings applies to the user side. The writers.rs:380 and :383 cites are the pair-delete and analysis insert, not ratings writes.

### PF-004 [high] Web client design docs disagree on thumbnail caching and key contents — CONFIRMED

`WEB_CLIENT_DESIGN.md:1322` says the thumbnail URL includes turns and the response is public, immutable. :1495 says no turns component and private. :687 says private. `WEB_CLIENT.md:317` says public and keyed without turns. The design doc contradicts itself and the design doc contradicts `WEB_CLIENT.md`.

### PF-005 [high] Session token digest algorithm disagrees within `WEB_CLIENT_DESIGN` — CONFIRMED

`WEB_CLIENT_DESIGN.md:178` and :340 store the BLAKE3 digest of the token. :1694 (labelled Proposal) says the server stores its SHA-256. The same document gives two digests for the same stored token.

### PF-006 [high] QML Settings keys in camelCase vs gallery.json `snake_case` keys — CONFIRMED

Main.qml:39 still declares Settings { id: appSettings }, and Main.qml:690-691 feed writeBack and detectBorders from it. Rust reads the same preferences from prefs().get() (settings.rs:26-27), whose on-disk keys are `snake_case` (prefs.rs:212 GOLDEN). `REFACTOR_DESIGN.md:34` says gallery.json is the only prefs file. The QML Settings store is a live second store that contradicts that, not just a key-case difference.

### PF-007 [medium] `file_failures` pass set documented as thumbs and borders only — CONFIRMED

`NOTIFICATIONS_DESIGN.md:268-270` documents pass as thumbs | borders, but FilePass (model notice.rs:375-382) has Thumbs, Borders and Import. The cited schema.rs:114 comment is about `cache_size` and does not list pass values, so that cite is wrong; the real stale doc is `NOTIFICATIONS_DESIGN`.

### PF-008 [medium] Ledger pass and kind codes spelled several ways across catalog, bridge and inbox — CONFIRMED

Ledger pass is numeric in the bridge (`failed_files_bridge.rs:60`: 0 thumbs, 1 borders, 2 import, 3 scans), text in notices.rs:535 (thumbs, borders, import, scan-folders), and pass.code() or kind.code() bound into SQL (ledger.rs:423-424), and the inbox uses the text code. Three encodings for the same ledger identity.

### PF-009 [medium] Notice level code mapping duplicated in three places — CONFIRMED

Level mapping Info=0, Warning=1, Error=2 is written three times: inbox.rs:424 `level_code`, `log_model.rs:64` `level_code` and notices.rs:227 `level_code`. The inbox version is i64, the desktop ones i32. baseline.sql:109 states the same mapping as the schema comment.

### PF-010 [medium] Hard-coded SQL literals where shared constants or macros exist — PARTIAL

The mask 0xFFFFFFFFFFFF is repeated as a literal in sql.rs:91 and at least seven places in work.rs (57, 64, 465, 483, 505, 656, 665), with no named constant, so the repetition is real. The premise that a shared constant exists is wrong for this literal. `live_failures!` (sql.rs:130) is already a macro and its literal is documented as deliberate, because the partial index needs the literal condition. The thumbcache `auto_vacuum` literal 2 (schema.rs:229) has no named constant, a real case.

### PF-011 [medium] Temp-file suffix convention diverges — CONFIRMED

Temp names differ: writeback file.rs:46 uses a leading dot and .pg-write; yandex save.rs:50-52 uses `.<name>.pg-save`; crop lib.rs:402 uses .pg-crop; app store.rs:118 uses `<name>.tmp`, not dot-prefixed; eval plant.py:38 uses .tmp. The model notice.rs:1397 test fixture uses .pg-tmp, which matches no real writer.

### PF-012 [medium] Version meta key names differ for the same thumbnail version — PARTIAL

`thumb_output_version` is written to thumbcache meta (thumbcache schema.rs:156), and `thumbs_version` is the catalog meta mark (core.rs:225, fixture/raw.rs:270). They are different databases and different constants (`THUMB_OUTPUT_VERSION` and `THUMBS_VERSION`). The writer.rs:393 cite is a comment about eviction and does not name a key. The cited key names are not the same value under two names.

### PF-013 [medium] Missing value encoded as -1 in some places and null or NULL in others — PARTIAL

Persisted -1 is the prefs `last_file` sentinel (prefs.rs:104, 'a negative number reads as none') and the eval ids .npy (`cpu_worker.py:54`). Most other -1 cites (LaneRow.qml:17, VideoLog.qml:9) are in-memory UI sentinels that are never written. SQL uses NULL for unknown created (baseline.sql:208). The persisted mix is real but narrower than the finding says.

### PF-014 [medium] Image URL grammar differs between the Rust parser and QML — REFUTED

Viewport.qml:32 does not exist. A search for a fit/ image URL form in the QML and in client image.rs found nothing. The shown image URLs are `image://full/<key>/full`, `image://full/`... region and `image://thumb/<key>/<height>` (items.rs:72-81, client image.rs:159), and they parse back to the same request. No second grammar is in use.

### PF-015 [medium] Web client thumbnail URL shape differs between two docs — CONFIRMED

`WEB_CLIENT_RECONCILED.md:320` has `/api/thumbs/{catalog_id}/t{THUMBS_VERSION}/{file_id}@`... ; `WEB_CLIENT_DESIGN.md:691` has `/api/thumbs/{file_id}@{mtime}.{rotation}.{thumbs_version}/{height}` with no `catalog_id` and a different version position. Two documents specify the same URL differently.

### PF-016 [medium] Web client device cookie name and path differ — CONFIRMED

`WEB_CLIENT_DESIGN.md:340` names the device cookie `pg_device` with Path=/api/login and SameSite=Strict. `WEB_CLIENT_RECONCILED.md:543` names `gallery_device` with Path=/api/auth/login and SameSite=Lax. The name, path and SameSite all differ between the two docs.

### PF-017 [medium] Quarantine location and migration number disagree in web client docs — CONFIRMED

`WEB_CLIENT_DESIGN.md:941` keeps a per-volume `_quarantine` folder and Recycle Bin is rejected; `WEB_CLIENT_TECH_DECISIONS.md:200` says quarantine is dropped and deletes go to the Recycle Bin. The migration number also differs: DESIGN:952 says migration 10 adds quarantine tables, RECONCILED.md:440 says migration 11.

### PF-018 [medium] README data folder list omits thumbs.sqlite — PARTIAL

crates/README.md:288-290 lists the catalog folder as holding catalog.sqlite and the thumbnail cache, without the file name. The same README names thumbs.sqlite at line 71 and at 360. layout.rs:75 puts thumbs.sqlite in `catalog_dir`. The data-folder list omits the file name, but does name the cache, so this is a wording gap, not a missing file.

### PF-019 [medium] `io_attempts` columns and triggers differ between two plan docs — CONFIRMED

`DECODE_ONCE_PLAN.md:409` defines `io_attempts(file_id`, pass, attempts) with no `next_try_at`. `IMPORT_FAILURES_PLAN.md:385` defines `io_attempts` with `next_try_at` DEFAULT 0 and an `io_attempts_due` index. The two plans give different column sets for the same table name.

### PF-020 [medium] `work_leases` and `root_removals` columns differ across `TO_10` and the full design — CONFIRMED

`BACKGROUND_JOB_AUDIT_FIXES.md:49` gives `work_leases` six columns. `BOUNDED_MEMORY_PLAN.md:462` gives twelve columns. `root_removals` also differs: `BACKGROUND_JOB_AUDIT_FIXES.md:52` (id, path, hidden, report, finished, `cursor_path`, `cursor_id`) against `BOUNDED_MEMORY_PLAN.md:434` (which adds `key_lo` and `key_hi`).

### PF-021 [medium] AUTOINCREMENT declared inconsistently across plans and catalog — REFUTED

The docs agree with the built catalog. baseline.sql:204 and :208 declare roots, folders and files with AUTOINCREMENT, and scans (baseline.sql:7-8) has no AUTOINCREMENT, which matches `BACKGROUND_JOB_AUDIT_FIXES.md:140`. DATABASE.md:33 lists files, folders and roots and does not mention scans, which is accurate. The mfolders table exists only in `CATALOG_SIZE_PLAN` as a planned table.

### PF-022 [medium] Moved-aside catalog and backup file names — REFUTED

The names are one scheme across code and docs. rebuild.rs:26 uses .building, rebuild.rs:31 uses .pre-v{version}.bak, and rebuild.rs:44 uses .partial beside the backup. The moved-aside names (.replaced, .bad, numbered .1) come from one function, dbfile remove.rs:14. inbox.rs:44 and rehearsal.rs:79 describe the same names. The cited lifecycle.rs:752 is a test that uses the same scheme.

### PF-023 [medium] Analysis row version uses `IMPORT_VERSION` — PARTIAL

The analysis version is `IMPORT_VERSION` by design: versions.rs:10 says the analysis pipeline's version is `pg_analysis::analyze::IMPORT_VERSION`, so an analysis made at another version is redone. The value is documented. Naming it `IMPORT_VERSION` is confusing, but the code and the doc agree, so this is a naming issue, not a stored-value defect.

### PF-024 [medium] Manifest key case, path style and encoding differ across build tools — CONFIRMED

`build_static_ffmpeg.py:253` writes CRT entries with keys Name, MSVCRT, LIBCMT, while :294 writes libraries with name, bytes, sha256. `build_jpeg_turbo.py:143` stores an absolute library path, and `build_static_qt.py:226` stores a path relative to the prefix. Key case and path style differ across the three manifests.

### PF-025 [medium] Fixture hash input separators differ inside one tool — CONFIRMED

`compare_shots.py:258` prefixes each name with os.sep, and :267 uses `as_posix` for broken files. The docstring at :256 explains the leading separator for compatibility with the PowerShell version, so that part is intentional. The two functions still use different separators inside one tool.

### PF-026 [low] Rotation write named Turns, rotation and turns — CONFIRMED

The Rust kind is WriteKind::Turns (writeback.rs:470, :836, :875), but the queue key is the string 'rotation' (writeback.rs:928 and queue.rs:70 via `write_rotation!`). queue.rs:562 uses the temp name .b.jpg.turns.pg-write with 'rotation' as the kind.

### PF-027 [low] Span name differs for the pairing pass — CONFIRMED

pairs.rs:136 opens the span pass.pairing, while control.rs:248 maps Phase::Pairing to phase.pairing. The cited import/src/control.rs:248 is wrong; the real file is jobs/src/control.rs. The span and phase names for the same work differ.

### PF-028 [low] Tombstone retention and listing key proposals conflict with decided docs — PARTIAL

`INCREMENTAL_SYNC.md:60` proposes replacing 90-day retention and marks it Recommended, not accepted. `TECH_DECISIONS.md:207` keeps 90 days and marks it as a proposal. Both documents treat retention as undecided, so they do not contradict each other on the decision. The conflict with the bounded-memory plan's Phase 2 freeze is noted in `INCREMENTAL_SYNC.md:58` and is real, but it is one open item, not a split decision.

### PF-029 [low] Historical thumbs-v3 cache name in profiling notes — PARTIAL

`START_PROFILE.md:219` is a historical note that the first run of a new build made the empty thumbs-v3.sqlite. That is a dated record, not a live name. The PROFILING.md:170 cite does not exist: grep for thumbs-v3 in docs finds it only in this audit. PROFILING.md:171 names thumbs.sqlite, which is the current name.

### PF-030 [low] Catalog `application_id` checked only when `user_version` is positive — PARTIAL

schema.rs:310 checks `application_id` only when `user_version` is above 0. judge.rs:131 treats id 0 as not foreign and any other differing id as foreign. The two rules differ for id 0 and for version 0. Neither doc states which is intended, so this is a real divergence whose intent is unrecorded.

### PF-031 [low] JSON indentation varies across persisted tool outputs — CONFIRMED

eval/ann/run.py:497 writes `verify_meta.json` with no indent, :300 uses indent=1, report.py:178 uses indent=1, `bench_decode.py:207` and verify.py:901 use indent=2, and tools/perf-compare.py:102 uses indent=2 with `sort_keys`. Five indent choices across eval and tools.

### PF-032 [low] CSV schemas differ for the same kind of output — CONFIRMED

`pairs_knn.csv` has header a,b,label with integer ids (run.py:500). `misses_*.csv` has `similarity,path_a,path_b` with file paths (evaluate.py:156). `unpaired_pairs` has a fourth column layout (evaluate.py:177). `imagereduce_timing.py:27` has a fixed FIELDS list that `imagereduce_switch.py` does not match. `bench_ann.py:190` writes exact.npz keys ids, sims, rate while the other npz keys differ.

### PF-033 [low] pairKey implemented twice in the eval viewer — CONFIRMED

viewer.html:165 defines pairKey as sort-and-join with |, and viewer.html:607 defines keyOf as the same expression. The viewer's own comment at :164 says it is the same as viewer.py's `pair_key`, so there are two copies in one file plus the Python one.

### PF-034 [low] Analysis row column lists disagree with the plan — PARTIAL

writers.rs:382-383 INSERT OR REPLACE names 18 columns, and no `dups_state` appears in the catalog source. `DECODE_ONCE_PLAN.md:475` describes a 29-column upsert with `dups_state`, which is a planned change. The plan is not built, so the drift is expected unless the plan says it is done. The 27-column `analysis_rewritten` trigger (plan :716) is also a planned statement that was not checked in code.

### PF-035 [low] Research manifest has double .html extensions — CONFIRMED

docs/research/MANIFEST.tsv has five rows ending .html.html (grep count 5), matching the rows 08, 36, 44, 45 and 48.

### PF-036 [low] Inbox key separator differs between the catalog and QML fixture — PARTIAL

The catalog's `key_text` (inbox.rs:394-396, used at :366) joins with U+001F. The desktop InboxEntryId key is row:{id}:{uid} and ledger:{code} (notices.rs:156-163), a different identity used for selection. The QML fixture (`tst_inbox_panel.qml:109`, row:2:2) follows the desktop key, not the catalog one. The cited fixture therefore agrees with the desktop side. Whether the catalog key and the desktop key should match was not settled, so the finding names the wrong pair.

## architecture

### A-001 [high] thumbcache reads catalog tables directly despite views-only claim — CONFIRMED

thumbcache reader.rs reads the catalog's tables directly through ATTACH: `cat.sqlite_sequence` at :33 and cat.files in `catalog_volumes` at :121-123. The crate's own comment (Cargo.toml:11-13) says it reads through the views alone. The pg-catalog entry at Cargo.toml:24 is the dev dependency for the tests, which the comment also says.

### A-002 [high] `REFACTOR_DESIGN` states a wrong desktop dependency set and a non-existent query path — PARTIAL

`REFACTOR_DESIGN.md:11` says the desktop depends on seven pg crates at runtime. desktop Cargo.toml:20-22 lists eight, including pg-perf, so the count is wrong. `REFACTOR_DESIGN.md:46` says `pg_catalog::query` does not exist and that `from_json` and `to_json` are in pg-query. pg-query has both (lib.rs:319 and :325), so that line is correct, and the finding misreads it.

### A-003 [medium] NoticeSink described three ways and held as trait object and concrete type — CONFIRMED

`NOTIFICATIONS_DESIGN.md:309` describes NoticeSink as Arc of a Fn in pg-library. :139 says it is a trait, and :69 says it moved to pg-jobs. The code has a trait in jobs `notice_sink.rs:1`, library holds Arc of dyn NoticeSink (library.rs:55), and LibraryService and PrefStore hold a concrete Arc of Inbox (library.rs:273, prefs.rs:142). Three descriptions and two holding styles.

### A-004 [medium] Model holds display text against its own rule — CONFIRMED

model tree.rs:311-313 returns FolderTree::anchors as a String of display text, D: E:. `REFACTOR_LEDGER.md:98` (F-37) records it as a shortcut: the plan puts display text in pg-client. notice.rs:1-4 says nothing in the model is English, so the rule is stated and this method breaks it.

### A-005 [medium] Picker purpose is a number in one place and an enum in another — CONFIRMED

The picker's purpose is a Rust enum (edits.rs:166, Purpose::SaveCrops), a number in the bridge doc (`gallery_bridge.rs:163` says purpose 1), and a bare number from QML (YandexPanel.qml:168 calls savePicker.pick(2)). The same value is named three ways.

### A-006 [medium] Desktop imports the same app items from different paths — CONFIRMED

Desktop imports app items from two paths: notices.rs:93 and `failed_files/state.rs:4` and filters.rs:16 use `pg_app::reexport`, while notices.rs:94, `lane_model.rs:23` and `failed_files.rs:17` import the same kind of item from `pg_app` directly. The reexport module is the documented boundary, so the direct imports are inconsistent with it.

### A-007 [medium] Re-export allowlist differs between design doc and boundary tool — CONFIRMED

`NOTIFICATIONS_DESIGN.md:197` says a production pub use naming another crate is allowed only in app/src/reexport.rs. boundaries.py `REEXPORT_BOUNDARIES` (lines 33-35) also allows crates/testkit/src/lib.rs. The design doc and the check disagree on the allowlist.

### A-008 [medium] Module docs name crates outside their dependency set — PARTIAL

client image.rs:51, render transcode.rs:1-2, thumbs placeholders.rs:7 and log file.rs:9 name crates outside their dependency set. These are prose references, not imports, so no build depends on them. They only become broken links if written as intra-doc links, and the cites show plain prose. The finding is about wording, not dependency.

### A-009 [medium] Test fixtures come from four different places — CONFIRMED

Test fixtures come from four places: pg-testkit (edit Cargo.toml:10-11 says so), `pg_codec::fixtures` (edit crop.rs:156 and rotate.rs:89), and a file from another crate, `include_bytes` of ../../video/tests/sample.mp4 (library tests writeback.rs:291). `REFACTOR_LEDGER.md:63` (F-18) says the image dev-dependency should move to pg-testkit.

### A-010 [medium] Platform path rules reached directly instead of injected or wrapped — PARTIAL

Direct uses of `PATH_RULES` are real: schema.rs:269 and :291 pass it into `bring_to`, which takes rules as a parameter (:146-148), and desktop folders.rs:49, `failed_files.rs:76` and gallery model.rs:99 use it directly. The desktop uses the native rules by design, as catalogs.rs:17-21 documents, so only the places that skip the wrapper are open.

### A-011 [medium] Catalog module docs omit or contradict their imports — CONFIRMED

lifecycle.rs:2-3 says the file uses types and error and that its SQL macros come from sql. Line 14 imports crate::progress::FreeSpace, which the doc does not name. writers.rs:5 does not list the lease and lifecycle imports at lines 136-137. The module docs are out of step with their imports.

### A-012 [medium] Browse read side reaches into borders and writers despite leaf claim — CONFIRMED

browse/read/details.rs:46 calls `crate::borders::border_sides_on`, so the read side reaches the borders module. borders.rs:12-14 imports core, current, error and the writers. scope.rs:2 says the read-side scope is a leaf that uses nothing of its siblings, which the details call contradicts.

### A-013 [medium] Library lane and inner module docs contradict their imports and calls — CONFIRMED

lanes.rs:5-6 says the module is pure and tested without a disk. groups at :74-76 calls SystemDrives, and `probe_place` at :177 calls `pg_scan::root_volume`. The module is not pure, and it depends on pg-scan, which the plans say is being folded into pg-library.

### A-014 [medium] Platform rule enforced through different mechanisms per crate — PARTIAL

The platform rule uses different mechanisms by design: boundaries.py checks the platform words with an empty `PLATFORM_GLUE` (boundaries.py:40), and clippy `disallowed_types` is the mechanism for process spawning. `CLIPPY_LEDGER.md:299` lists the process ban as warn with adoption still to do. The mechanisms are not one rule in one place, but each rule has a check; the gap is the unadopted ban, not a conflict.

### A-015 [medium] Dependency and library-crate counting differs between module-graph tools — CONFIRMED

The module-graph tools count library crates and dependencies in different ways: `compiler_graph.py:38` uses any lib in kind, `unreachable_pub.py:27` uses kind equal to ['lib'], `api_surface.py:28` reads normal dependencies only, `make_html.py:35` reads all packages, and edges.py:43 reads normal dependencies only. The same question gets different answers across the tools.

### A-016 [medium] Qt objects invoked by string name across the C++ and Rust boundary — CONFIRMED

Qt objects are reached by string name across the boundary: screenshot.cpp reads the QML property shellCore (Main.qml:118) and invokes pending by name, and deadline.cpp:28 calls invokeMethod with the name timeout. A rename in QML or Rust breaks these silently at run time.

### A-017 [medium] Shortcut policy location and QML controller logic conflict — CONFIRMED

GlobalShortcuts.qml:4 and Main.qml say what a shortcut does is the controller's, and the controller decides. `REFACTOR_LEDGER.md:116` (F-45) records 106 functions of controller logic still in QML, with Main 25, Viewer 17, InboxPanel 15 and FilterBar 11. The stated policy and the code disagree, and the ledger records it as open.

### A-018 [medium] Version registry location differs from the planned location — PARTIAL

`tools/test_versions.py:14` scans crates/model/src/versions.rs as the registry, and versions.rs:1-3 is the registry (commit 991f5119 made the hand-written versions its). The test and the file agree. The finding cites a planned location that the instances do not show, so the mismatch is with the plan, which is not verified here.

### A-019 [medium] Catalog-free command path and catalog folder creation in the CLI — PARTIAL

cli lib.rs:130-139 runs borders of piped pixels or of outside files without a catalog and creates the catalog folder for every other command. The split is intended and documented in the function's own doc comment, so the two paths differ on purpose.

### A-020 [medium] Python tooling uses different import and run conventions — CONFIRMED

Python tooling uses different import conventions: flat imports (eval/accel.py:33, ann/data.py:13), package imports (ann/exact.py:12), sys.path.insert (`github_files.py:18` and `test_linux.py:11`), a local run function (linux.py:85), and an import from pgtools (`win_build.py:26`) that `linux_build.py` does not use.

### A-021 [low] Historical layering claims in plan docs without status markers — PARTIAL

`BACKGROUND_JOB_AUDIT_FIXES.md:176` says the analysis to platform layer is to be added to crate-layers.txt, and crate-layers.txt:31 still lacks platform for analysis, so the plan is not executed. The finding wants status markers on the plan's layering claims. The plan does state its change as a plan item, which is the status marker the finding asks for.

### A-022 [low] Plan layer maps predate the current crate-layer file — PARTIAL

`DECODE_ONCE_PLAN.md:1176` gives an import map of analysis catalog dedup jobs model platform render thumbcache trace. crate-layers.txt:40 gives analysis catalog container dedup jobs model perf platform, so the plan's map is stale. `GUI_THREAD_PLAN.md:111` (catalog may depend on dbfile, log, model, platform, query) matches crate-layers.txt:25, and `MULTI_DRIVE_LANES_PLAN.md:53` (jobs not on catalog or scan) matches crate-layers.txt:28. Only one of the four plan maps is stale.

### A-023 [low] Eval scripts import private helpers and tool internals from other scripts — CONFIRMED

`bench_thumbs.py:51` imports the private `_vips` from `bench_decode`, and `verify_synth.py:23` imports ROTATIONS, Checkpoint and PairRunner from verify. Scripts reach into another script's internals.

### A-024 [low] App accessor list omits thumbs() — CONFIRMED

app.rs:6-8 lists the accessors inbox, catalogs, prefs, images, reading, views, edit, library and yandex. The thumbs accessor at :121-123 is missing from the list, though it exists.

### A-025 [low] Import and accessor path style varies within crates — CONFIRMED

Import and call style varies: notices.rs:95 imports text and calls render bare, desktop lib.rs:169 calls the full path, providers.rs uses app::get() at :45 and crate::app::get() at :99 in the same file, and library passes.rs:9 imports some crates and calls others by full path.

### A-026 [low] Desktop window modules read app state directly, bypassing wrappers — CONFIRMED

`failed_files.rs:144-145` calls the notices helper and reaches app::get().reading().session() directly. library.rs:31 wraps app::get().library().running() in running(), and `lane_model.rs:33` uses that wrapper. The window modules use the wrapper in some places and the app directly in others.

### A-027 [low] Row height constants named by a doc that the code does not use — REFUTED

The doc at shell/state.rs:19 names `pg_model::size::ROW_HEIGHT`. size.rs:10 defines `ROW_HEIGHT` as the range 60 to 800, and RowHeight::MIN and MAX (size.rs:21 and :23) are built from its ends. The code uses the bounds that come from the named constant, so the doc is accurate.

### A-028 [low] Leaf-below-core rationale repeated and reverse edges exist — CONFIRMED

The same leaf-below-core rationale is repeated across the catalog's module headers (dups/identity.rs:1, inbox/id.rs:1, and in failure.rs, ledger state.rs, listing.rs, coverage.rs, lease.rs). roots.rs:44 imports ledger::failure and removal, which shows reverse edges from the module that the core reaches.

### A-029 [low] Catalog comments tie to other crates' internals and phase numbers — PARTIAL

The two probe cites (`stmt_journal_probe.rs:9` and `pairs_write_probe.rs:11`) do not exist at those paths, so those instances cannot be checked. core.rs:109 says Test-only until the crash tests (Phase 4), a stale phase label that a reader cannot resolve. The finding has one real instance and two unchecked ones.

### A-030 [low] Pairing tooling couples to the v13 queue table — CONFIRMED

keep.ps1:38-40 deletes from `pdq_pairs` where the rows are in `pairs_owed`. Both tables exist in the schema and the probes (`crates/catalog/examples/pairs_write_probe.rs`). The profiling script depends on the table names and the pairing state of schema v13, so a schema rename breaks it without a check.

### A-031 [low] Static Qt wiring lives in patch files that edit crate sources — CONFIRMED

The static Qt build is wired through patch files that edit crate sources: tools/static-qt/app-build.patch:19-21 adds QuickControls2, QuickLayouts and Svg to desktop build.rs under `PG_QT_STATIC`, and pg-video.patch:63-65 adds a cfg-gated use of `pg_platform`. The crate sources differ from what the patches assume if they change.

### A-032 [low] Writeback holds a leaked static thumbnail cache — CONFIRMED

writeback lib.rs:165-167 holds thumbs as a leaked static reference, and the test at :812 creates it with Box::leak. The production code and the test use the same pattern, so the leak is in the design and in the tests.

### A-033 [low] Desktop and app module boundary details — PARTIAL

keymap.rs:68 imports a cxx-generated MediaKind from `gallery_bridge` in a non-bridge module, and `yandex_bridge.rs:1-2` says the allow is on this module and lib.rs. The shell state.rs:9 carries its own expect. These are small module boundary differences, and the finding does not show that any is wrong.

### A-034 [low] CLI manifest dependency order — CONFIRMED

cli Cargo.toml:11-13 lists pg-library before pg-borders and pg-dedup, so the sixteen pg dependencies are not in alphabetical order. Low severity.

## manifests-lints

### ML-001 [high] Client clippy.toml drops the `temp_dir` ban that root and model carry — PARTIAL

The root clippy.toml:25 bans `std::env::temp_dir`, and crates/client/clippy.toml has no `temp_dir` entry, so the client's gap is real. crates/model/clippy.toml does not ban `temp_dir` either (it bans `read_to_string`, write and `read_dir` at :20-23), so the finding's claim that model carries it is false. The cite clippy.toml:21 is a comment line; the ban is at :25.

### ML-002 [high] Three clippy.toml files repeat root settings by hand — CONFIRMED

The root clippy.toml:2-4 sets the `allow-*-in-tests` keys, and crates/model/clippy.toml:3-5 and crates/client/clippy.toml:3-5 repeat them by hand. The model file's own comment at :1-2 says to keep them in step with the root file, so the duplication is acknowledged and manual.

### ML-003 [high] Workspace lint comment says warn while clippy lints are deny — PARTIAL

The comment at Cargo.toml:60 says the lints are kept at warn so work in progress builds, and it sits above the [workspace.lints.rust] table, which is the rust lint group. The clippy group at :71-98 is deny, under its own comment at :79-80. The comment is accurate for the table it sits on and does not cover the clippy group, so the conflict is in reading, not in the lint levels.

### ML-004 [medium] `CLIPPY_LEDGER` level column disagrees with its decision column — REFUTED

`CLIPPY_LEDGER.md:230` and :298 use the Level column for the current level (warn) and the Work column for the target (adopt at deny). Current and target are different columns by design, so the rows do not contradict each other.

### ML-005 [medium] `CLIPPY_LEDGER` warn-first rollout conflicts with the -D warnings gate — CONFIRMED

`CLIPPY_LEDGER.md:16` says just lint runs clippy with -D warnings, so a warn-level lint fails the gate. Line 77 says the 171 names are to be added at warn with no code change, and their status is not started. Adding them at warn as the ledger says would fail the gate that the same document describes.

### ML-006 [medium] eval/png-decoders standalone workspace has no dependabot entry or lockfile check — CONFIRMED

.github/dependabot.yml has entries for / (grouped), /eval/jpeg-decoders, /fuzz, /tools/shots and /tools/lint, and none for /eval/png-decoders, which is a standalone workspace. lint.py:23-24 runs cargo metadata --locked on fuzz and eval/jpeg-decoders only. The .gitignore:5-6 note says the PNG workspace has its own Cargo.lock, which nothing checks for staleness.

### ML-007 [medium] Crate-root print-lint attribute present in some crates and absent in others — CONFIRMED

The print-lint crate attribute is present in catalog lib.rs:12, client lib.rs:3, codec lib.rs:7, container lib.rs:9, dedup lib.rs:6, metadata lib.rs:12 and perf lib.rs:17, and absent from dbfile lib.rs, crop lib.rs and thumbs lib.rs. The workspace deny at Cargo.toml:81-82 and the allow-print-in-tests key already cover it, so the per-crate attribute is partly redundant and its presence is uneven.

### ML-008 [medium] No rust-version declared while newer syntax and APIs are used — CONFIRMED

No Cargo.toml sets rust-version (verified at P-007 and here). `LINT_TOOLS.md:31` says the MSRV is not set anywhere. The code uses let-chains (dups.rs:85, desktop app.rs:60, scan lib.rs:562) and `is_none_or` (browse tests.rs:579), which need a recent toolchain, so the minimum is not declared.

### ML-009 [medium] Feature names mix singular and plural test-support forms — CONFIRMED

The test-support feature is fixture in catalog Cargo.toml:14 and cli/library dev-dependencies, and fixtures in codec Cargo.toml:29, with library also enabling codec fixtures at Cargo.toml:39. The singular and plural forms are both used for the same kind of thing.

### ML-010 [medium] allow(...) used where the convention is expect with reason — CONFIRMED

dedup join.rs:288 and :290 use `cfg_attr` with `allow(dead_code`), and the workspace sets `allow_attributes` at warn (Cargo.toml:77). The convention in this repo is expect with a reason, and these two sites have neither.

### ML-011 [medium] QML files missing pragma ComponentBehavior: Bound — CONFIRMED

WheelListView.qml:1 and PanelKind.qml:1 have no pragma, Ratings.qml:2 and Theme.qml:4 carry only pragma Singleton, and Viewer.qml:18 has pragma ComponentBehavior: Bound. The pragma is present in some component files and absent in others, which is the inconsistency the finding names.

### ML-012 [medium] `unreachable_pub` expect applied to every bridge module except two — CONFIRMED

desktop lib.rs:76 puts the `unreachable_pub` expect on the screenshot bridge, and :81 and :86 on other bridges. `settings_bridge` (lib.rs:79) and `trace_bridge` (lib.rs:84) are bridge modules with no such expect. The two listed bridges are not the only exceptions, so the bridge list is incomplete in the same way.

### ML-013 [medium] Python noqa suppressions mix bare codes and parenthesised reasons — CONFIRMED

accel.py:131 and run.py:343 use a parenthesised reason after noqa, `nvimgcodec_bench.py:87` uses one too, and accel.py:298 and report.py:59 use noqa E731 with no reason. The reason convention is applied unevenly.

### ML-014 [medium] `REFACTOR_LEDGER` says no wasm32 check exists, but lint.py runs one — CONFIRMED

`REFACTOR_LEDGER.md:62` (F-17) says there is no wasm32 check in just lint or CI. tools/lint.py:21 runs cargo check for pg-model and pg-client on wasm32-unknown-unknown, and `NOTIFICATIONS_DESIGN.md:367` describes that check. The ledger is stale.

### ML-015 [medium] Boundary bans described as enforced by deny.toml are not all present — PARTIAL

`REFACTOR_LEDGER.md:61` (F-16) records the planned windows-sys and cxx bans as missing, and that is an open item, not a claim that deny enforces them. codec Cargo.toml:10 says image decoders are linked here and nowhere else (deny.toml), but deny.toml:56 lets image be wrapped by pg-library, pg-testkit and pdqhash as well as pg-codec. The codec comment overstates the ban.

### ML-016 [medium] Ruff repo-wide errors outside the lint scope — REFUTED

The `TIER2_ANN_BAKEOFF_PLAN.md:117` note recorded 9 ruff errors and one format diff on 2026-11-14. Run today, ruff check . reports All checks passed and ruff format --check . reports 182 files already formatted, with ruff 0.17.3. The repo-wide check is clean now, so the plan's note is stale and nothing is outside the lint scope.

### ML-017 [low] Plan docs cite stale Cargo.toml and deny.toml line numbers — CONFIRMED

Cargo.toml:69 is `undocumented_unsafe_blocks`; `GUI_THREAD_PLAN.md:1719` cites line 65, which is a different lint. `VALUE_PROVENANCE_IDEAS.md:33` cites the cast lints at 86-94, and they are at 88-98. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:192` and :143 cite lines that moved. The stale line citations are real.

### ML-018 [low] WTFPL allowance in deny.toml matches no crate — PARTIAL

deny.toml:34 allows WTFPL. Whether any crate uses it needs cargo-deny to run, and I did not run it, so the claim that the entry matches no crate is unverified. LANGUAGES.md:426 is a phase table row, not a cite of this entry, so it does not show the entry is unused.

### ML-019 [low] quick-xml entries left in manifests after the planned removal — PARTIAL

`EXIFTOOL_PORT_PLAN.md:378-380` plans the removal of legacy and quick-xml, including fuzz/Cargo.toml:82, deny.toml:53 and :84, and the lock lines. The entries are still present, which is expected while the removal is a plan, not done. The finding treats the planned removal as overdue, but the plan does not set a date.

### ML-020 [low] Windows feature additions planned but not in the workspace list — PARTIAL

LANGUAGES.md:51, `EXIFTOOL_PORT_DESIGN.md:471` and `MULTI_DRIVE_LANES_PLAN.md:135` name Win32 features (Globalization, `System_Time`, `System_Ioctl`) that are not in Cargo.toml:49-54. These are planned additions, so the gap is between a plan and the manifest, not a bug.

### ML-021 [low] Hash-container guidance in plan docs contradicts the clippy ban — CONFIRMED

`SCAN_OPTIMIZATION_PLAN.md:967` says item 13 makes the walk's seen set a HashSet, which the clippy ban forbids. The validation note at :1051 says the hash types were replaced with BTreeSet and BTreeMap. The plan contradicts itself, and the NIGHTLY appendix states the ban correctly.

### ML-022 [low] Cargo build-script directive written with one colon and with two — CONFIRMED

tools/static-qt/pg-video.patch:11 uses cargo:rerun-if-env-changed with one colon, and :12 uses cargo::rustc-check-cfg with two. app-build.patch:11 uses cargo:: with two colons. Cargo accepts both, but the patches mix the two forms.

### ML-023 [low] Windows macro gating differs between C++ files — CONFIRMED

`video_source.cpp:7` and its other branches use the compiler macro `_WIN32`, while `gpu_device.cpp:22` and its other branches use Qt's `Q_OS_WIN`. The same platform test is written two ways across the desktop C++ files.

### ML-024 [low] Shebang lines present on some Python tools and absent on others — PARTIAL

The shebang mix is real: tools/profiling/bench.py has a shebang line, `bench_data.py` has none, and cgtab.py and `ci_setup.py` have one. Two of the cited files do not exist at the cited paths (`tools/profiling/grow_catalog.py`, keep.py and medians.py are missing; keep.py is at tools/profiling/pairing/keep.py).

### ML-025 [low] Manifest dependency spelling and ordering vary — CONFIRMED

desktop Cargo.toml:31 writes cxx-qt-lib = { workspace = true, features = [...] } inline while the lines above use name.workspace = true. catalog Cargo.toml:21 lists pg-platform before pg-dbfile, out of order with the rest of the list. Low severity.

### ML-026 [low] Edition written literally in two standalone manifests — REFUTED

fuzz and eval/png-decoders are standalone workspaces with their own lockfiles, so they cannot inherit edition from the root with edition.workspace. The literal edition is the only option for those manifests, so the difference is required by their layout.

### ML-027 [low] Expect reason text repeated across state files with escaped apostrophe — CONFIRMED

The same reason, with an escaped apostrophe, is repeated in the desktop state files: `lane_model/state.rs:41`, yandex/state.rs:11, animation/state.rs:11, and the finding counts 11 files in all. The repetition is real. The escape is valid Rust but unusual, and the text should be shared or shortened.

### ML-028 [low] Platform-gated tests named differently in catalog and client — CONFIRMED

lifecycle.rs:812 and :838 use plain cfg(unix) and cfg(windows) tests with descriptive names, while the browse tests use `under_both_rules!` with `_windows` and `_posix` suffixes. The two catalog test styles for the same platform split are real.

### ML-029 [low] Test-file lint allowance style differs between test files — CONFIRMED

rehearsal.rs:12 sets a file-level expect for `unwrap_used` with a reason, while writers.rs:1 relies on clippy.toml allow-unwrap-in-tests. The two test files use different ways to allow the same lint.

### ML-030 [low] Module declarations out of order in desktop lib.rs — CONFIRMED

desktop lib.rs:95-101 lists mod declarations that are not in order: worker sits before a blank line and yandex and `yandex_bridge` come after it. The module list is not kept sorted. Low severity.

### ML-031 [low] Feature combinations missing from the clippy gate — PARTIAL

dhat-heap is a feature of crates/gallery (Cargo.toml:28-29), off by default, so the clippy gate with default features does not compile it. The specta dependency is at the root (Cargo.toml:42), and whether a model feature gates it was not checked here.

### ML-032 [low] Crates listed under both dependencies and dev-dependencies in library — PARTIAL

library Cargo.toml lists pg-catalog and pg-writeback, and pg-borders, under both dependencies and dev-dependencies (with fixture and fault features). Cargo allows this, and the dev-dependency entry is how a test enables a feature. The duplication is a Cargo pattern, not an error.

### ML-033 [low] Gate content documented differently from lint.py — CONFIRMED

`NOTIFICATIONS_DESIGN.md:367` says just lint runs cargo deny. lint.py's STEPS (lines 18-30) have no deny step, and the Justfile at :134 says the deny run is not in lint. The design doc's description of the gate is wrong.

## tooling

### TL-001 [high] Probe scripts resolve their output root outside the checkout — CONFIRMED

`wsl_probe.sh:10` computes ROOT from ../../.. relative to docs/probes, which is the parent of the checkout (`D:\GitHub`), so the default probe folder is outside the repo. `wsl_probe.sh:3` says the folder is in the checkout, so the comment is wrong. `v1_symlink.ps1:3` has the same climb to `D:\GitHub\target`.

### TL-002 [high] Docs disagree on what just lint runs — CONFIRMED

`NIGHTLY_AND_BUILD_FLAGS_PLAN.md:232` and LANGUAGES.md:129 both say just lint runs cargo deny. tools/lint.py has no deny step (a grep for deny finds nothing), and the Justfile deny recipe is outside lint. The docs state a step that does not run.

### TL-003 [high] WSL and container policy stated inconsistently across docs — PARTIAL

PLAN.md:19 says WSL is not used and Linux runs in pg-dev. IO.md:91 describes an ExifTool probe run in WSL and `TIER2_ANN_BAKEOFF_PLAN.md:102` says CPU indexes ran in WSL until 2026-11-11 and then moved, so both are dated history. The current policy is consistent. The TIER2 doc names pg-eval where PLAN names pg-dev, which is a real naming difference but both are containers, not WSL.

### TL-004 [medium] Python interpreter command differs by call site — CONFIRMED

ci.yml:77 runs python on Windows and ci.yml:238 runs python3 on Linux. The Justfile py default (:21) picks python or python3 by OS, but `compare_shots.py:675` falls back to a literal 'python' whatever the OS.

### TL-005 [medium] Cargo invocations in Justfile disagree on --locked — CONFIRMED

Justfile build (:50), run (:63), and dev (:67) call cargo without --locked. test (:72) passes --locked, and modgraph's cargo install (:149) passes --locked. The cite lines are off by two or three, not the claim.

### TL-006 [medium] Runner image for Pages deploy floats — CONFIRMED

pages.yml:30 uses runs-on: ubuntu-latest, while ci.yml:224 pins ubuntu-26.04. The cite is ci.yml:224, not :223.

### TL-007 [medium] eval/jpeg-decoders is a separate workspace that no CI step builds — CONFIRMED

eval/jpeg-decoders/Cargo.toml:9 declares its own [workspace]. lint.py:22-24 runs cargo metadata on that manifest, but no CI or Justfile step builds the crate, which matches `REVIEW_LEDGER.md:89`.

### TL-008 [medium] Callgraph pin build and profile logic duplicated between Python and shell/PowerShell — CONFIRMED

The MSVC compile and link flags are written out in build.py (:86 onwards) and again in build.sh (:42-54), and build.sh is the file build.py says was replaced (DOC-059). The knob list (-o, -stop, -start) is built separately in profile.py:61 and profile.ps1:32. Duplicated in Python, shell and PowerShell.

### TL-009 [medium] Build parallelism differs between Linux and Windows eval builds — CONFIRMED

`linux_build.py:21` uses JOBS = `os.cpu_count(`). `win_build.py` has -j 4 five times (lines 59 and others) and win-build.cmd has -j 4 five times. The Windows build parallelism is hard-coded and the Linux build is not.

### TL-010 [medium] Clone-and-patch helper copied three times in eval build scripts — CONFIRMED

A clone-and-patch helper is written three times: `linux_build.py:44` clone, `win_build.py:31` clone, and win-build.cmd:20 :clone. Each applies the patch and keeps an existing folder, in its own language.

### TL-011 [medium] Executable and interpreter path handling repeated per OS — CONFIRMED

accel.py:38 chooses the venv interpreter path by os.name, and `gpu_encoders.py:7` documents Scripts versus bin per OS. encoders.py:35 uses EVAL/'bin' with no suffix, `bench_thumbs.py:55` and imgload.py:45 each pick .exe or '' by os.name. The per-OS suffix logic is repeated across tools.

### TL-012 [medium] Env file loading implemented differently in PowerShell and Python tools — CONFIRMED

qmltest.ps1:13-15 parses `..\.env` with its own regex and sets unset variables. qmltest.py:115 calls `pgtools.load_env_file`. eval/README.md:12 says PowerShell does not read .env, which the ps1 contradicts. `build_jpeg_turbo.py:85-87` runs its platform guard before `load_env_file`.

### TL-013 [medium] Text files read and written without an encoding — CONFIRMED

perf-compare.py:61 and :109 call `read_text` with no encoding, and :102 calls `write_text` with no encoding, while `lint_qml.py:37` passes encoding='utf-8'.

### TL-014 [medium] CPU load measured by two different WMI queries — CONFIRMED

sweep.ps1:29 reads `Win32_PerfFormattedData_PerfOS_Processor` PercentProcessorTime. keep.ps1:97 and `few_sweep.ps1:50` read `Win32_Processor` LoadPercentage. Two WMI classes measure CPU load.

### TL-015 [medium] Probe scripts capture stderr differently — CONFIRMED

sweep.ps1:52 merges stderr with `2>&1`. `few_sweep.ps1:44` and oracle.ps1:74 take stdout only. The probe's output is captured differently across the three scripts.

### TL-016 [medium] Probe binaries disagree on scratch cleanup and duplicate scheduling — PARTIAL

Scratch cleanup differs: `pairs_load_probe.rs:108` and `pairs_write_probe.rs:114` call `remove_dir_all`, while `stmt_journal_probe.rs:206` creates its `run_folder` and never removes it. The cites are at crates/catalog/examples/, not src/, which is the real location. The rotation claim ((k + run) % len at :90 and :86) was not located by a search for % len, so it is unverified.

### TL-017 [medium] Screenshot comparison exists as both a PowerShell and a Python tool — PARTIAL

The premise that a PowerShell tool exists beside the Python one is false: tools/compare-shots.ps1 does not exist, only `tools/compare_shots.py` does. `BOUNDED_MEMORY_PLAN.md:1567` still names the .ps1, so the stale reference is the real defect. The 25-scenario claim at `compare_shots.py:104-129` was not confirmed (:104 is a blank line in the view).

### TL-018 [medium] Eval scripts parse command lines in three styles — PARTIAL

Two styles are seen: encoders.py:238 dispatches on sys.argv[1] through a dict, and eval/ann/run.py:524-525 reads stage names from sys.argv. linux.py does not use argparse (no match), so the cited comparison does not hold for linux.py. Some eval scripts use argparse; the claimed third style was not established.

### TL-019 [medium] Stale reference to `make_bench_jpeg.py` in bench source and docs — REFUTED

`tools/profiling/make_bench_jpeg.py` exists, and linux.rs:21 and tools/README.md:33 name it correctly. `DECODE_ONCE_PLAN.md:1221` introduces `make_strata.py` as a new tool and does not say it replaces `make_bench_jpeg`; the plan at :1163 extends `make_bench_jpeg`, which is consistent. No stale reference was found.

### TL-020 [low] Bench run command documented two ways — REFUTED

ir.rs:3 says the benches run with just perf in the Linux container. `DECODE_ONCE_PLAN.md:1165` says cargo bench -p pg-bench --bench ir through bench.py:95. Both are true: just perf runs bench.py, which runs cargo bench. Consistent, not two conflicting run methods.

### TL-021 [low] Probe shell and PowerShell strictness differs — CONFIRMED

`win_probe2.ps1:111` sets ErrorActionPreference to Continue, while `v1_symlink.ps1`, `v1b_junction.ps1`, `v2_volumes.ps1` and `win_probe.ps1` use Stop. `v34_wsl.sh` has no set -u (grep count 0), while `v4_drvfs_superblock.sh:4`, `wsl_probe.sh:5` and `wsl_probe2.sh:5` use set -u.

### TL-022 [low] Benchmark harness error handling mixes check and returncode — CONFIRMED

bench.py:115 and :122 use subprocess.run with check=True, while bench.py:98 uses check=False and tests returncode itself. Two error styles in one file.

### TL-023 [low] Exit-code convention documented only in a plan — CONFIRMED

The 0/1/2/3 exit-code convention is written in `DECODE_ONCE_PLAN.md:1219`. The tools use sys.exit with ad hoc messages (`compare_shots.py:177-283` and others) and no shared convention is in code, so the plan is the only place the convention is written down.

### TL-024 [low] Desktop-only flags tested in the CLI detector — REFUTED

cli lib.rs:745-746 asserts wants(...) is true for the CLI's own subcommands and help. The second loop at :747-749 asserts !wants for --screenshot, --view and --viewer-keys. The test checks that the CLI does not claim desktop flags, which is the intended behaviour.

### TL-025 [low] CI leaves a debug variable and sets a tool-missing flag — PARTIAL

ci.yml:59-60 sets `CARGO_LOG` with the comment 'Diagnostic (temporary)', so the debug variable is left in CI with a temporary label. `PG_REQUIRE_TOOLS` at ci.yml:51 is an intended setting (its comment says so), not a leftover.

### TL-026 [low] Tool-availability probes differ across library tests — CONFIRMED

library crop.rs:87 probes exiftool with -ver and then calls `pg_testkit::missing_tool`. video.rs:38 probes ffmpeg with -version through a filter, and writeback.rs:136 probes exiftool inline without `missing_tool`. Three probe patterns.

### TL-027 [low] Code cites PROFILING.md by line number — REFUTED

The crate code cites PROFILING.md by section heading (join.rs:84 names 'The scan kernel'), not by line. A grep for PROFILING.md with a line number in crates finds nothing.

### TL-028 [low] Docs name eval scripts without paths — CONFIRMED

MODELS.md:22 names evaluate.py without a path, while VERIFIER.md:35 names eval/verify.py. The eval scripts are named without a consistent path convention across docs.

### TL-029 [low] Desktop C++ include path style differs — CONFIRMED

deadline.cpp:5 and screenshot.cpp:13 include "`pg-desktop/cpp/<name>.h"`. `frame_sink.cpp:1` includes "`frame_sink.h"` bare, and screenshot.cpp:29 and providers.cpp:22 include generated bridge headers by the pg-desktop/src path. Three include styles in one C++ tree.

### TL-030 [low] Bench data regeneration names the old backup and the generator — CONFIRMED

`CATALOG_SIZE_PLAN.md:312` says bump LAYOUT = "v10-windows-rules" at catalog.rs:43. The bench code has `BENCH_LAYOUT` = "v1-windows-rules" at catalog.rs:36. The constant name, the line and the version all differ from the plan's instruction, so the plan is stale.

### TL-031 [low] Version lint script referenced from the versions registry — PARTIAL

`tools/test_versions.py` exists and does the regex scan of `*VERSION` constants described in its docstring (lines 1-4 and 16-17). The versions.rs:7 claim that it fails on a constant no owner reads was not confirmed in the parts read, so only the outside-module check is verified.

## pins

### P-001 [high] FFmpeg BtbN build tag in ci.yml header comment is stale — REFUTED

ci.yml:11 names no BtbN tag; the comment only says the FFmpeg pin lives in the Justfile. The only tag is Justfile:17 autobuild-2026-10-31-12-59, not the autobuild-2026-11-19-12-39 or -10-30-13-01 values the finding cites. The cited Justfile line is 17, not 15.

### P-002 [high] turbojpeg-sys fork rev disagrees between the build script comment, eval harness and fuzz/root manifests — CONFIRMED

`build_jpeg_turbo.py:32` `FORK_COMMIT` 945d853b, and its comment at :29-30 says it is the commit the root Cargo.toml pins. Root Cargo.toml:148 pins 28b8b86a. fuzz/Cargo.toml:84 and fuzz/Cargo.lock:660 pin 28b8b86a too. `linux_build.py:95` clones 945d853b. `STATIC_BUILD.md:20` says 28b8b86. The comment in `build_jpeg_turbo.py` is false.

### P-003 [high] Probe constants claim to mirror `pg_import` but are copied as literals — CONFIRMED

The probes are in crates/catalog/examples/, not src/. `pairs_write_probe.rs:39` has WRITE = `2_000`, documented as mirroring `pg_import::pairs::WRITE`, which is `4_000` at crates/import/src/pairs.rs:39: the mirror is already out of date. `pairs_load_probe.rs:38` `CACHE_KIB` = `16*1024` matches pairs.rs:34. `pairs_write_probe.rs:37` has PRAGMA `cache_size` -16384 as a literal.

### P-004 [medium] parajpeg git rev differs between production manifests and eval harness — CONFIRMED

Cargo.toml:34 and fuzz/Cargo.lock:353 pin parajpeg 8e2c53a. eval `linux_build.py:158`, `win_build.py:96`, win-build.cmd:50 and `JPEG_DECODERS.md:489` use f45081d. The harness clones upstream anniigr/parajpeg, while the product uses the darktorres fork, so the two revisions differ.

### P-005 [medium] fuzz manifest claims the root patch list is complete, but it is not — CONFIRMED

fuzz/Cargo.toml:74 claims the root patch table is the full list. The root has 14 entries (Cargo.toml:137-150). fuzz/Cargo.toml has 9 (lines 77-86). parajpeg is git-pinned outside both tables, and fuzz/Cargo.lock:353 has no patch entry for it.

### P-006 [medium] Plan doc cites the old png fork rev and Cargo.toml line — CONFIRMED

`NIGHTLY_AND_BUILD_FLAGS_PLAN.md:159` says the png fork is at f7129d3 at Cargo.toml:132. Cargo.toml:141 pins 2f176edb. `PNG_DECODERS.md:3` says pinned at 2f176ed. The plan's rev and line are stale.

### P-007 [medium] Workspace declares no rust-version, and API and SQLite floors are undeclared — CONFIRMED

Cargo.toml:1-8 [workspace.package] has no rust-version. The cited features are real: let-chain at crates/client/src/grid.rs:312 (the finding says controller/grid.rs:310, which does not exist), `split_at_checked` at results.rs:63, `is_multiple_of` at text.rs:291, `repeat_n` at words.rs:86, UPDATE..RETURNING at pair.rs:277, unixepoch() at lease.rs:28. No SQLite version check exists (grep for `sqlite_version` finds nothing).

### P-008 [medium] Runner image labels differ between workflows — CONFIRMED

ci.yml:56 windows-2025 and :224 ubuntu-26.04 (the finding's :223 is the job name line). fork-updates.yml:17 ubuntu-26.04. pages.yml:30 ubuntu-latest, which floats.

### P-009 [medium] gungraun-runner installed without --locked while other cargo installs use it — PARTIAL

Dockerfile:115 gungraun-runner has no --locked. Dockerfile:113-114 gives the reason in a comment: the published lock pins yoke-derive 0.8.3, which was yanked. :116 and :87-90 use --locked. The omission is deliberate and commented, so inconsistent describes a documented exception.

### P-010 [medium] Rust components and the installer are declared in both `ci_setup.py` and rust-toolchain.toml — CONFIRMED

The file is `tools/profiling/ci_setup.py`, not `tools/ci_setup.py`. Line 51 fetches `https://sh.rustup.rs` with urllib and pipes it to sh with no checksum. Line 55 runs rustup component add clippy rustfmt on the default toolchain. rust-toolchain.toml:4-5 pins nightly-2026-11-06 with clippy and rustfmt components.

### P-011 [medium] Static-build tool pins copied between PowerShell and Python — CONFIRMED

The same Build Tools path literal is in `build_jpeg_turbo.py:33`, `build_static_ffmpeg.py:39`, `build_static_qt.py:42`, build-jpeg-turbo.ps1:13 (the finding says :24) and build-static-ffmpeg.ps1:14. `build_static_ffmpeg.py:36` and build-static-ffmpeg.ps1:26 both pin make 4.4.1-3 and diffutils 3.12-1.

### P-012 [medium] ExifTool version in docs and platform code disagrees with the 13.59 pin — CONFIRMED

Justfile pins are at :98 (`exiftool_version` 13.59.0) and :102 (`exiftool_source_version` 13.59). The finding's :96 and :100 are off by two. platform/native.rs:15-16 says checked with ExifTool 13.60. IO.md:84 says 13.60 and :91 says 13.50 under WSL. `EXIFTOOL_PORT_PLAN.md:10` names 13.60 as the port's reference. The docs' reference version differs from the pinned 13.59.

### P-013 [medium] Bundled SQLite version stated three different ways — CONFIRMED

DATABASE.md:101 says the app ships 3.54.0. `UROCISSA_COMPARISON.md:174` says 3.54. budgets.json:14 says 3.54. `CATALOG_SIZE_PLAN.md:357` says 3.54.1 with libsqlite3-sys 0.39.1. `DECODE_ONCE_PLAN.md:852` says 3.54.1. Three different precisions for the same bundled build.

### P-014 [medium] Eval decoder build scripts repeat commit pins instead of sharing one list — CONFIRMED

`win_build.py:64`, :79, :117 and `linux_build.py:158` pin jpeggpu 3ae7ac0d, GPUJPEG 37ab4810, GPU-JPEG-Decoder 61cfb372 and parajpeg f45081d. win-build.cmd:36, :42, :50, :59 carry the same commits as literals. win-build.cmd:30 builds libjpeg-turbo from vendor/ with no commit pin, while `win_build.py:48` clones `FORK_COMMIT`. The pins are repeated and one is missing.

### P-015 [medium] Python 3.12.10 literal in ci.yml, Justfile and `STATIC_BUILD.md` — CONFIRMED

ci.yml:72 hard-codes python-version 3.12.10. Justfile:96 `python_version` is 3.12.10 (the finding's :94 is its comment). `STATIC_BUILD.md:27` names 3.12.10. The literal in ci.yml is deliberate: setup-python runs before just is installed, and the Justfile comment says the tools test asserts equality. The duplication is documented and tested.

### P-016 [medium] MSVC toolset literal duplicated in PowerShell scripts and `STATIC_BUILD.md` — CONFIRMED

build-jpeg-turbo.ps1:24 and build-static-ffmpeg.ps1:24 hard-code Toolset 14.44.35207. `STATIC_BUILD.md:22` names it again. The Python recipes read `MSVC_TOOLSET` from pgtools.

### P-017 [low] Measurement records cite GPU and image library versions that differ from the current pins — PARTIAL

Accurate for nvImageCodec (`JPEG_ENCODERS.md:53` measured 0.9.0.20; pin 0.10.0.9), nvJPEG (`JPEG_DECODERS.md:492` measured 13.2; requirements.txt:8 pins 13.3.2.43) and Pillow 12.2.0 (`PNG_DECODERS.md:47`). Not accurate for Pillow 13 (`JPEG_DECODERS.md:51`), which matches requirements.txt:11 pillow 13.0.0, or for libvips (no pin exists). The docs state measurement versions by design.

### P-018 [low] FFmpeg and mpv versions in docs disagree with the shipped FFmpeg 9.0.2 — PARTIAL

mpv 0.41 does not appear in `VIDEO_PLAYER.md` (the doc says 0.42.0 at :261 and :289). FFmpeg: PROFILING.md:126 labels its Linux build as FFmpeg 8 (Ubuntu) and says the Windows app ships avcodec-63, so that is a stated separate build. `JPEG_DECODERS.md:54` names FFmpeg 8 mjpeg in a decoder table without a build label. No mpv version is pinned anywhere.

### P-019 [low] Eval torch CUDA tag in MODELS.md is stale against requirements — PARTIAL

requirements.txt:65 pins torch 2.15.0+cu134. `TEST_SET.md:13` says CUDA 13.4 (matches). MODELS.md:190 says cu132 then cu134, which measures the same. MODELS.md:152 says cu132 for the driver-621.01 measurement, a dated record. Only the dated measurement uses cu132; the rest agrees with the pin.

### P-020 [low] TensorRT version in eval README and accel.py differs from requirements pin — REFUTED

requirements.txt:60-62 pins TensorRT 11.4.0.106. accel.py:40 defaults `TRT10_DIR` to tools/tensorrt-10.16.1.11-cu13. eval/README.md:86-87 documents that folder as TensorRT 10 DLLs for ONNX Runtime's TensorRT provider, which is built against TensorRT 10. The difference is deliberate and documented.

### P-021 [low] Justfile comment says bump cargo-modules in two places, but CI reads it from the Justfile — CONFIRMED

Justfile:142 says bump cargo-modules in ci.yml and the container reads it from here. `tools/ci/github_files.py:81` maps cargo-modules to `cargo_modules_version` and reads it from the Justfile through `just_value`. So CI no longer needs a separate bump, and the bump-both instruction is stale. The cite Justfile:140 is a comment line; :143 is the 0.28.0 pin.

### P-022 [low] Two Windows binding crates in one platform crate, and a stale windows-sys line reference in a plan — PARTIAL

Both binding crates are in platform: picker.rs:129 uses the windows crate, dir.rs:122 and drive.rs:240 use `windows_sys`. That part holds. `DECODE_ONCE_PLAN.md:299` does not mention windows-sys at all, so the plan-reference instance is wrong. Cargo.toml:54 windows-sys 0.61 is at line 54, as the finding says.

### P-023 [low] GitHub Action pin comments use different version granularity — CONFIRMED

ci.yml:66 comment # v7, :70 # v7.1.0, :82 # v2; pages.yml:43 # v6.0.0 and :48 # v5.0.1. The comment granularity varies as stated.

### P-024 [low] Apt packages are unversioned in three places, against a digest-pinned base image — CONFIRMED

Dockerfile:7 pins the base image by digest. bench.py:81-82 says apt-packages.txt is not pinned to a snapshot. video/build.rs:52-53 lists `libav*-dev` with no versions. `tools/profiling/ci_setup.py` (the finding's `tools/ci_setup.py:41`) appends mold unversioned.

### P-025 [low] GPU compute capability hard-coded in one build script and detected in others — CONFIRMED

eval/jpeg-encoder/build.ps1:11 hard-codes Arch = `sm_75` (the finding's eval/jpeg-decoders/build.ps1 does not exist). `win_env.py:74-75` and win-env.cmd:29-30 detect the compute capability from nvidia-smi unless `CUDA_ARCH` is set.

### P-026 [low] Bench data default falls back to a container path — CONFIRMED

catalog.rs:28-31 falls back to /root/pg-bench when `PG_BENCH_DATA` is unset (the finding's :30 is the None arm, correct).

### P-027 [low] Yandex User-Agent pinned to Chrome 108, a browser build from 2022 — CONFIRMED

yandex/src/http.rs:11-14 sets Chrome/108.0.0.0 and says it is the same User-Agent as an external crawler. The cited line and the stated reason both match.

### P-028 [low] Qt 6.12 cited as a version in code comments, not only READMEs — PARTIAL

Qt 6.12 is named in platform/audio.rs:1 and desktop/qml/GridCell.qml:77 as the finding says. `gpu_device.cpp:4` cites Qt's source file qrhid3d11.cpp, not a Qt version. Two of the three instances match the claim.

### P-029 [low] Key version tied to toolchain Unicode version — PARTIAL

types.rs:272 combines `KEY_ALGO_VERSION` with `char::UNICODE_VERSION`, as stated. The toolchain dependency is already documented in the function's doc comment at types.rs:268-270 (recomputed on another toolchain). The key-version doc the finding asks for does not exist as a separate doc.

### P-030 [low] SHA-256 comparison uses two different case conventions — PARTIAL

tools/setup-ffmpeg.ps1 is deleted in the working tree (uncommitted) and present at HEAD. The Python side also compares case-insensitively: `setup_ffmpeg.py:96` lowercases the pin and :109 compares `sha256_of(zip`). The cited line 82 is `top_folder`, not the SHA comparison. Both sides are case-insensitive, so the two-conventions claim describes style, not behaviour.

## ui-text

### UI-001 [high] MB labelled where the value is MiB — CONFIRMED

`pairs_write_probe.rs:103` labels the column 'WAL after (MB)', and :110 computes the value as bytes `>>` 10 divided by 1024.0, which is MiB. The label is wrong. The lifecycle.rs:270 cite is decimal MB with a decimal divisor (`1_000_000`), so it is consistent and not a defect.

### UI-002 [high] Rating symbols defined in Rust and again in the QML test stub — CONFIRMED

client text.rs:30 `RATING_SYMBOLS` and the QML test stub Words.qml:8 hold the same six symbols, with the same clamp to 0..5 (stub :10). The list is duplicated across Rust and a test stub, so it can drift.

### UI-003 [medium] Same id-less word under two Fluent ids in ui.ftl — CONFIRMED

ui.ftl has 'Loading…' at :110 (ui-inbox-loading) and :166 (ui-loading). 'Log' is at :116 (ui-log) and :117 (ui-log-title), and 'Inbox' at :94 and :97 (ui-inbox-title). The same words are under two ids with no shared key.

### UI-004 [medium] Writer stop reported with different words in GUI and CLI — CONFIRMED

The GUI notice says 'Writing into the files has stopped' (text.ftl:261) and the CLI says 'the file writer stopped with N changes still to write' (cli.ftl:70). The same event is worded differently in the two surfaces and with different terms (writing into the files vs file writer).

### UI-005 [medium] Kept-change wording differs across three Fluent files — CONFIRMED

Kept-change wording: text.ftl:282 'A change to a file in {folder} was kept', ui.ftl:71 'kept: a change is waiting to be written', cli.ftl:61 'removal not finished, rows kept'. Three phrasings of one state.

### UI-006 [medium] Failure notice levels follow no stated rule — CONFIRMED

notice.rs:1049 gives Cropped and BordersCut Warning when failed `>` 0 and Info otherwise. ThumbsFailed, BordersFailed and ImportFailed are Warning (:1073). WriteFailed is Error (:1087), CacheDisabled is Error (:1086) while CacheFull is Warning (:1060). `NOTIFICATIONS_DESIGN.md:208-209` says warnings and errors stay until dismissed but gives no rule for which variant gets which level.

### UI-007 [medium] Status word for Failed differs between filter chip and status — PARTIAL

The test at text.rs:821-825 expects the Failed status to read 'Unreadable', which matches text.ftl:14 (status-failed = Unreadable). The filter chip's label was not located, so whether the chip and the status word actually differ is unverified.

### UI-008 [medium] Dialog buttons come from two label sources — CONFIRMED

SaveViewDialog.qml:38 uses standardButtons Dialog.Save | Dialog.Cancel, so its labels come from Qt's platform theme. CutBordersDialog, DiscardRemovalDialog, FolderPickerDialog and RemoveFolderDialog use words.text('ui-cancel'). LANGUAGES.md:113 records the one exception.

### UI-009 [medium] words.phrase count argument passed in several shapes — CONFIRMED

The count argument to words.phrase is passed as a fixed 0 (YandexPreview.qml:129, FilterBar.qml:271, TopBar.qml:85-86), as the unread count (StatusBar.qml:114), as failedTotal (InboxPanel.qml:462), as dialog.count (CutBordersDialog.qml:42), as index (RatingBar.qml:76) and as pickedCount (YandexPanel.qml:164). The shapes differ and the fixed 0 is not explained.

### UI-010 [medium] Key-name notation differs across hint lines and strings — CONFIRMED

ui.ftl:135 writes Shift+R, :218 writes R / Shift+R, :230 writes ⇧R, and text.rs:790-799 asserts the hint strings in both forms. Key notation differs across the hint lines.

### UI-011 [medium] Client-facing English Display text bypasses the catalogue — PARTIAL

The English Display impls exist: names.rs:153 NormalizeError, scan lib.rs:344 ScanError::Removed, catalog error.rs:37 RootRemoved, library folders.rs:17 AddError::Refused, and writeback lib.rs:98 WriteError, which its doc says the app words through `pg_client::text`. The CLI maps NormalizeError itself (cli lib.rs:149-151). Whether the English Display text reaches a GUI user was not confirmed, so the finding is only partly shown.

### UI-012 [medium] CLI and log refusal text bypasses cli.ftl — CONFIRMED

gallery main.rs:50 prints 'gallery was built without its app window...' with eprintln, a user-facing English message outside cli.ftl. desktop lib.rs:161 and shell.rs:71 write English through `pg_log`. The yandex fetch.rs:44, :53 and parse.rs:10 Unexpected details are English literals.

### UI-013 [medium] Refusal advice text drifts from the test and the other messages — CONFIRMED

The same refusal advice is worded three ways: lifecycle.rs:27 'made with the {name} path rules', lifecycle.rs:28 'made before its path rules were recorded', error.rs:20-21 'open it on the platform that made it, or move it aside and scan again', and core.rs:164 'restore a `.pre-v*.bak` or start fresh'. The test at lifecycle.rs:1030 asserts the error.rs wording on Error::PathRules, which is correct.

### UI-014 [medium] Doubled and mismatched strings in doctor and qmltest scripts — CONFIRMED

doctor.py:174 and :177 print the same strings as doctor.ps1:88 and :92. The `QT_DIR` not-set message is written in qmltest.ps1:20, qmltest.py:41 and `lint_qml.py:70`, with the same wording in each.

### UI-015 [medium] Viewer colours are literals, one a magenta border — CONFIRMED

Viewer.qml:443 uses a literal border colour #ff00ff and Viewer.qml:154 a literal 'black' background, while YandexPanel.qml:1 and other panels use Theme tokens. The viewer's colours are outside the theme.

### UI-016 [medium] Catalog and database used for the same file — CONFIRMED

dbfile judge.rs:49 says 'it is not a database', dbfile remove.rs:35 says 'can't move {} aside', and catalog lifecycle.rs:60 says 'catalog {}: another program or connection has it open'. Catalog and database are both used for the same file in the refusal messages.

### UI-017 [low] Product name spelled as a Rust literal and in Fluent — PARTIAL

The em-dash claim is refuted: ui.ftl:122 and LANGUAGES.md:85 both use 'Photo Gallery — {text}' with an em dash, so no hyphen is in use. The real defect stands: alert.rs:6 passes the literal 'Photo Gallery' to the error box instead of going through a catalogue key (ui-app-name at ui.ftl:121 exists), and text.ftl:172 holds the crash box text.

### UI-018 [low] Window and dialog strings use different scripts and separators — CONFIRMED

Separators differ in the GUI: FilterBar.qml:156 uses two spaces, Hud.qml:90-92 three spaces, SaveViewDialog.qml:51 ' · ', YandexPanel.qml:77 ' '( and ')', TopBar.qml:101 ' ▴' and ' ▾', and StatusBar.qml:100 '⚠ ' prefix. ResultTile.qml:92 uses '×' in the size caption.

### UI-019 [low] Plain-text rule not applied in test fixtures that carry markup — REFUTED

The fixtures with `<b>` are deliberate: `tst_inbox_panel.qml:110` and :234 and `tst_log_panel.qml:100` assert that the markup is shown as literal text (selectedText equals the string with `<b>`). The production panels use textFormat PlainText (InboxPanel.qml:462-464), so the test checks the plain-text rule.

### UI-020 [low] Panel and status text on the failed-file ledger is inconsistent — REFUTED

Both failed-file link keys read 'Show the failed files': ui.ftl:103 (ui-inbox-show-files) and :113 (ui-failed-link). ledger.rs:88 names the same entry action. The wording is consistent, and the other cited comments describe different status messages.

### UI-021 [low] Plan docs state UI text differently from the code — CONFIRMED

`REVIEW_LEDGER.md:87` says the 'All folders' label comes from text::ui('ui-all-folders') in Rust. The ledger itself records the deviation as undeclared: the code returns an empty name and FolderTree.qml holds the words. The plan and code state the label's source differently.

### UI-022 [low] Historical plan docs name keys and facts the code no longer uses — PARTIAL

'round-failed' is still a live key in the catalog doc (ledger.rs:58) and RoundFailed is still a Notice in the client (text/notice.rs:74), so one cited key is still in use. The 'scan-failed' key and the RoundPanicked-as-State claim were not checked, so the historical-drift claim is only partly shown.

### UI-023 [low] Sentence-final periods differ between messages and labels — PARTIAL

yandex-nothing-picked (text.ftl:137) and yandex-no-folder (:138) end with a period, and ui.ftl labels (:8 onward) do not. Labels and sentences are different kinds of string, so a missing period on labels is expected. Whether messages are consistent with each other was not checked across the file.

### UI-024 [low] Apostrophe and quote style differs across UI copy — CONFIRMED

text.ftl:40 wraps the name in curly quotes '(name has “{ $name }”'), while text.ftl:135 uses straight apostrophes in "Can't open". Tests at client text.rs:485, :667 and :736 assert straight apostrophes in "couldn't" and "can't".

### UI-025 [low] CLI and GUI sentence case and tool casing differ — CONFIRMED

CLI lines start lowercase '(imported 12 files...' at client text/cli.rs:613), GUI lines start capitalised, and cli.ftl:14 begins with a stray comma ', { $count } changed while read'. Tool messages mix sentence case too: pgtools.py:35 starts lowercase and `lint_qml.py:70` starts uppercase.

### UI-026 [low] Eval and bench output uses x and multiplication sign for sizes — CONFIRMED

YandexPanel.qml:272 and ResultTile-style captions use '×', as do nvjpeg.py:278 and video.rs:122. The png harness harness.cpp:205 and the jpeg harness use an ASCII x in sizes. The cited yandex.rs:309 is a `data_changed` call, not the file-name suffix, but the mixed separators are real.

### UI-027 [low] Bench output labels for units and references differ — CONFIRMED

nvjpeg.py:246 prints 'ms/image' and `nvimgcodec_bench.py:90-92` prints 'median wall ms per image' and 'CPU ms per image'. nvjpeg.py:296 says 'vs Pillow' while the harness says 'vs libjpeg-turbo'. The unit and reference labels differ across the bench outputs.

### UI-028 [low] Test assert messages contradict checks or each other — CONFIRMED

writeback.rs:769 asserts (status.error, `status.error_file`) == (None, None) but its message says 'the failure of a dropped write is still shown'. The message contradicts the check. borders.rs:86 and writeback.rs:377 give two skip reasons for the same permission case with different wording.

### UI-029 [low] Notice id and source-word naming is documented two ways — CONFIRMED

notice.rs:110 documents source words as '`source-<code>'`, while `NOTIFICATIONS_DESIGN.md:344` says '`source-<name>'` and the id arm uses `notice-<code>-title` (notice.rs:941). The doc comment and the design name the key part differently.

### UI-030 [low] Inbox and version-mismatch messages are worded in different voices — CONFIRMED

app inbox.rs:446 says 'the inbox's queue is full: notices are not kept' and :927 says 'the inbox couldn't keep {notices} notices'. thumbcache schema.rs:203 says 'layout version {version}, this build's is `{CACHE_VERSION}'` and dbfile schema.rs:45 says 'differs from this build's'. The cited desktop/src/inbox.rs paths are wrong; the messages are in app/src/inbox.rs.

## logging

### LG-001 [high] Thumbnail events logged under two targets — CONFIRMED

app.rs:208 logs the thumbnail cache's started-over event under target thumbnails. images.rs:147 logs thumbnail failures under target thumbs, and the test at images.rs:566 asserts thumbs. One kind of thumbnail event is filed under two targets.

### LG-002 [high] Perf-log rotation cap in docs does not match code — REFUTED

The perf log has its own cap: crates/perf/src/lib.rs:31 sets `FILE_CAP` to 16 MiB, and lib.rs:85 opens `pg_log::LogFile` with it. PROFILING.md:1331 says 16 MiB and two older files, which matches. The 5 MiB figure in pg-log's `FILE_CAP` (crates/log/src/file.rs:26) is the app log's cap, and `NOTIFICATIONS_DESIGN.md:330` describes that one correctly. The finding compared the wrong cap.

### LG-003 [medium] Notice written to the log in two formats — CONFIRMED

app.rs:182 logs a notice as JSON and falls back to the error text. inbox.rs:622 logs the same notice as JSON and falls back to a Debug form with the error. inbox.rs:1064 falls back to the bare error. The same notice is written to the log in three fallback shapes.

### LG-004 [medium] Ledger entries always logged at warn, notices mapped by level — CONFIRMED

inbox.rs:625-627 logs a notice at its own level (info, warning or error). inbox.rs:1069 logs every ledger entry at warn, whatever its level.

### LG-005 [medium] Inbox logs under two target schemes — CONFIRMED

The inbox logs under a literal target inbox (inbox.rs:446, and at 553, 844, 851, 860 and 927). `write_log` at inbox.rs:623 uses notice.source().code() as the target, and the ledger path at inbox.rs:1069 uses the same source code. Two target schemes are used in one file.

### LG-006 [medium] CLI summary written to stderr in one mode and stdout in another — PARTIAL

In --stdin mode the borders command writes its data to stdout (lib.rs:715) and so writes its summary to stderr (lib.rs:716), which keeps stdout clean for the data. The CSV mode prints its summary to stdout because there is no data stream. The two modes differ for a reason, so the mismatch is a choice, not a fault.

### LG-007 [medium] Environment version line on stderr in one eval script, stdout in others — CONFIRMED

encoders.py:237 prints the environment line to stderr. gpu.py:89, `gpu_encoders.py:45` and debesheedas.py:41 print it to stdout. The eval scripts disagree on where the environment line goes.

### LG-008 [medium] Desktop startup diagnostics bypass `pg_log` — CONFIRMED

The desktop trace.rs:33 writes a failure to open the perf log with eprintln. The desktop screenshot.rs:1 is a hook that uses eprintln as well. `NOTIFICATIONS_PLAN.md:57` records that the GUI build never shows stderr, so these lines are lost in the app that is meant to log them.

### LG-009 [medium] Perf span names do not follow the pass./pending. pattern — CONFIRMED

Span names do not follow one pattern: borders lib.rs:63 and import lib.rs:79 use pass. and pending. pairs, thumbs lib.rs:212 uses pass.thumbs with thumbs.pending (reversed), scan lib.rs:2252 looks up scan.walk, passes.rs:75 uses scan.after, roots.rs:169 uses bare `mark_missing`, lane.rs:93 uses bare round, transaction.rs:18 uses `catalog.tx.*`, and crop lib.rs:240 has pass.cut beside pass.crop. The prefix rule is not applied.

### LG-010 [medium] Pairing perf names split across two prefixes — CONFIRMED

pair.rs:88 uses `pair.after_scan` and pair.setup, while import pairs.rs:136 uses pass.pairing and pairs.rs:228 uses pairs.load, pairs.store and pairs.join. Pairing spans use two prefixes.

### LG-011 [medium] Error chains formatted with {e} and {e:#} in different sites — CONFIRMED

app library.rs:421 logs a failed temp removal with {e}, which drops the cause chain. inbox.rs:1064 uses {e} too. desktop `log_model.rs:124` uses {e}. gallery main.rs:31 prints {e:#}, and codec decode.rs:165 keeps both errors. The chain format is not consistent.

### LG-012 [medium] Print-lint attribute repeated in each library crate — CONFIRMED

The print lint attribute is repeated in catalog lib.rs:12, render lib.rs:8, thumbcache lib.rs:28 and analysis lib.rs:6, and analysis duplicates the workspace print lints (Cargo.toml:81-82). thumbs lib.rs:1 has no print lint at all, so the set of crates that carry it is not uniform.

### LG-013 [medium] Session record documented but not written by the desktop app — REFUTED

`pg_log::init` (log lib.rs:86) calls LOG.init, which calls `init_capped` (lib.rs:266-267), and `init_capped` logs the session record with target session and message start (lib.rs:272). So the desktop run does write a session record. The doc at notices.rs:53 is accurate.

### LG-014 [medium] Log file name in user text is hard-coded in some places and a value in others — CONFIRMED

text.ftl:161 names gallery.log directly in a message, while text.ftl:174 takes the log path as a variable $log through DetailsInLog and DetailsNotLogged (message.rs:247). The log file name is hard-coded in one message and passed in others.

### LG-015 [medium] Hard-coded log call counts in docs disagree — CONFIRMED

The call counts in the docs disagree: `NOTIFICATIONS_DESIGN.md:118` says 36 log sites, :182 and :323 say 34, and LANGUAGES.md:99 says about 51 `pg_log` calls with a breakdown. Each count is for a different thing or a different date, but none is marked as the same count.

### LG-016 [medium] Skip message duplicated between exiftool and platform helper — CONFIRMED

exiftool/lib.rs:298 prints ExifTool not found: skipped. platform testing.rs:77 prints the same message shape with a tool name. The message is written twice, in two crates, with the same wording.

### LG-017 [low] Print lint allows a stderr line that uses Debug level names — PARTIAL

plain() (log record.rs:59) uses the Debug form of the level for the human stderr line, so the line reads Warning. The doc at log lib.rs:18 describes the JSON level field as lowercase info, warning or error, and that is the stored form. The human line is a different output, so the Debug form there is not a contradiction. The finding's point is that the two forms differ, which is true but deliberate.

### LG-018 [low] Identical warning text repeated in two places — CONFIRMED

lifecycle.rs:298 and lifecycle.rs:317 use the same phrase it is tried again at the next start, in two warnings with the same meaning.

### LG-019 [low] Lane model log category uses a space — CONFIRMED

`lane_model.rs:225` and :232 log under the category lane model, with a space. Every other `pg_log` category in the set is a single word.

### LG-020 [low] Warning wording uses contractions in some places and full forms in others — CONFIRMED

Warnings use contractions in some places (`lane_model.rs:225` can't be read, :232 don't parse, notices.rs:514 couldn't) and full forms in others (library.rs:421 wasn't deleted). Low severity; the mix is real.

### LG-021 [low] Migration warnings use a different target from catalog warnings — CONFIRMED

Catalog lifecycle warnings use the target catalog at lifecycle.rs:198, and the migration warnings at :297 and :315 use the target migration. The same module files two kinds of catalog warning under two targets.

### LG-022 [low] Same user-visible message written in PowerShell and Python twins — PARTIAL

tools/setup-ffmpeg.ps1:22 is missing from the working tree (deleted, present at HEAD), so that cite can't be checked here. The python cite at `setup_ffmpeg.py:75` is a docstring and does not print the sentence. The twin-message claim is not shown by these cites.

### LG-023 [low] Tool status words use different vocabularies — CONFIRMED

doctor.ps1:17 uses ok and MISSING, perf-compare.py:113 uses SKIPPED, NO-BENCH, MISSING, OVER and UNDER in uppercase, and fork-updates.py:99 uses lowercase newer release and current. The status words for a check's result use three vocabularies.

### LG-024 [low] Command echo present in some Python tools but not the shell build — PARTIAL

docker/check.py:27 and pgtools.py:128 echo each command with a + prefix. build.sh does not exist in the repository (no such file in tools), so the shell-build comparison in the finding cannot be checked. The Python echo pattern is real.

### LG-025 [low] Progress cadence and flushing differ between eval scripts — CONFIRMED

Progress cadence differs: verify.py:791 prints every 100 pairs, `verify_synth.py:74` every 260 pairs, `imagereduce_switch.py:184` every 50 items, and `bench_thumbs.py:345` prints without flush while :231 flushes. The eval scripts pick their own intervals.

### LG-026 [low] Probe examples format plan and header lines differently — CONFIRMED

`pairs_load_probe.rs:131` writes plan of the first four words, `pairs_write_probe.rs:153` writes plan of the first verb, and `pairs_write_probe.rs:103` writes a header that differs from `pairs_load_probe.rs:99`. The probes format plan and header lines differently.

### LG-027 [low] Test diagnostics printed with eprintln or println inconsistently — CONFIRMED

Test diagnostics use eprintln in browse tests.rs:415 and codec decode.rs:471, and println in join.rs:2912, `rules_reference.rs:126` and rehearsal.rs:102. Low severity.

### LG-028 [low] Superseded `PG_METRICS` body left in `DECODE_ONCE_PLAN` — PARTIAL

`DECODE_ONCE_PLAN.md:1227` carries a superseded note that says `PG_PERF_LOG` replaces `PG_METRICS` and that nothing was built under the old name. The body below the note still describes it, which is how superseded design text usually stays in a plan. The note is there, so the staleness is marked, not hidden.

### LG-029 [low] Trace stage function names repeat the crate prefix — REFUTED

pg-trace (crates/trace) is a real crate, and its stage functions use plain C names such as `pg_stage_main`, so the `pg_stage_` prefix is required by the SDE control string quoted in crates/trace/src/lib.rs. The repeated prefix is deliberate.

## tests

### T-001 [medium] Scratch folder cleanup differs: let `_`, unwrap, .ok() or none — CONFIRMED

Scratch cleanup is written five ways in the cited files: let `_` = `remove_dir_all` (app.rs:265, images.rs:483, inbox.rs:1430, dir.rs:691, video lib.rs:388), unwrap (app.rs:279, images.rs:420, file.rs:359, `other_process.rs:108`), and .ok() (player.rs:182). The replay.rs:197 cite is missing.

### T-002 [medium] Scratch root names do not follow the `scratch_<crate>` pattern — CONFIRMED

The scratch root names do not follow one pattern. `scratch_store`, `scratch_listing`, `scratch_app` and `scratch_rate` name areas or features, not the crate. `scratch_catalog` and `scratch_app` match their crate, and the second argument mixes pg-layout, kebab-case and `snake_case`. The replay.rs:167 and :202 cites are missing.

### T-003 [medium] Wait helpers are copied per file with different bounds and intervals — CONFIRMED

`wait_for` is copied in `catalog/tests/other_process.rs:17`, dbfile/tests/live.rs:17, library/src/runner.rs:395, library/tests/lanes.rs:90 and `thumbcache/tests/other_process.rs:14`, each with its own bound and 20 ms poll. render/src/player.rs:161 and writeback.rs:110 are variants. Several cites are off (lanes.rs:90 is not a wait, and cli lib.rs:980 is not a recv).

### T-004 [medium] Timing-window assertions that can flake under load — CONFIRMED

Timing asserts with fixed windows exist: roots.rs:984 asserts under 4 s, transaction.rs:219 under 1 s, inbox.rs:1690 sleeps 1 s to hold a lock, writeback.rs:403 sleeps a fixed 3 s. `MULTI_DRIVE_LANES_PLAN.md:455` records a 3 s deadline test that failed once in a parallel run.

### T-005 [medium] Known flaky tests recorded in plans but left unfixed — PARTIAL

`IMPORT_FAILURES_PLAN.md:185` records a flaky test that fails 2 of 5 alone. The test still exists under its name at thumbs/src/lib.rs:699, not at the cited :687 (which is a different test). The 2-of-5 rate was not reproduced here, so whether it is still flaky is unknown.

### T-006 [medium] Small test helpers duplicated across modules — CONFIRMED

edit crop.rs:105 and rotate.rs:93 have an identical four-line pattern() body. catalog roots.rs:449 and removal.rs:456 both define library(rules) with the same first three lines, then diverge.

### T-007 [medium] Catalog setup helper repeated across app and thumbs test modules — CONFIRMED

The catalog setup (`run_folder`, `Layout::in_home`, `create_dir_all`, `open_catalog`) is repeated in app inbox.rs:1103 `catalog_in`, reader.rs:388 catalog and library.rs:485 library. thumbs lib.rs:528 imported() repeats it inline in its tests at 701 and 759.

### T-008 [medium] Test fixture images come from `pg_testkit` and `pg_codec::fixtures` — CONFIRMED

Test images come from `pg_testkit` (`png_rgb`, `bmp_rgb`, `tiff_rgb` in edit crop.rs:146-148, `png_rgb` in analyze.rs:235, apng and gif in metadata lib.rs:115) and from `pg_codec::fixtures` (`tiff_rgb8` crop.rs:156, `webp_lossy_rgb` crop.rs:264, orient in rotate.rs:89).

### T-009 [medium] Path spelling in test fixtures varies by method — CONFIRMED

Path spelling varies: browse fixtures.rs:56 uses a raw `r"E:\p"` literal, where its neighbours at :54 and :58 use spelled(rules, ...). ledger.rs:502 branches on rules, inbox.rs:461 builds `r"D:\{}"`, pair.rs:578 uses spelled, and notices.rs:634 writes PathRules::Windows directly.

### T-010 [medium] Under-both-rules coverage differs between test modules — CONFIRMED

lifecycle.rs uses `under_both_rules!` in five places (804, 936, 975, 1005, 1020), but `posix_paths_are_bytes_under_case_sensitive_rules` at :1043 is a plain #[test]. inbox.rs:564, :636 and :663 are plain tests beside its `under_both_rules` tests, and queue.rs:407 runs library(PathRules::Windows) only.

### T-011 [medium] Test error handling mixes unwrap, expect, anyhow Result and panic — CONFIRMED

Error handling is mixed: `crop_folders.rs:87` returns anyhow::Result, crop.rs and the library tests use unwrap, catalog fixture/raw.rs:23 unwraps, fixture/state.rs:13-15 returns Result with ?, grid.rs:469 unwraps, shell.rs:628 expects, and borders.rs:23, pairs.rs:23 and `listing_laws.rs:23` put expect lints on helpers only. listing.rs:3 has a file-level expect.

### T-012 [medium] Test-end invariant checks applied to some suites only — CONFIRMED

writeback.rs:265 calls quiet.check() at the end, but runner.rs (test module, around :201 and :294) binds its Quiet as `_quiet` and never checks it. borders.rs:103 and :145 remove their folders with let `_`, so the end-of-test invariant is applied to some suites only.

### T-013 [medium] Golden comparison normalises line endings in one test and not the other — CONFIRMED

notice.rs:1824 normalises CRLF with `replace("\r\n"`, "`\n"`) before comparing `resolver_table.golden`, but notice.rs:1586 compares `include_str` of golden.jsonl with no normalisation. prefs.rs:212 uses an inline GOLDEN const, a third pattern.

### T-014 [medium] Doc claims tests never touch the connection, but many do — CONFIRMED

fixture/raw.rs:1-5 says the catalog's tests never touch its connection, except the layout tests. browse/read/tests.rs:741, writers.rs:1433, ledger.rs:602 and thumbcache lib.rs:1262 run raw SQL on c.conn or a fresh connection, and thumbcache reader.rs:180 builds a stub inline. The doc is not kept.

### T-015 [medium] Doc cites a test helper that does not exist — CONFIRMED

`NOTIFICATIONS_DESIGN.md:363` says catalog tests use `pg_testkit::test_folder`. No `test_folder` function exists anywhere in crates. The crate exposes `run_folder` (testkit lib.rs:16).

### T-016 [medium] Test-section cut rule differs across the module graph tools — CONFIRMED

Five modgraph tools cut the test section three ways: `module_graph.py:63` only at an inline mod, `api_surface.py:49`, `split_check.py:72` and edges.py:18 at any line-start #[cfg(test)], and dupes.py:26 by startswith. `module_graph.py:56` defines tests by filename, a fourth rule.

### T-017 [medium] Timing methodology differs across eval benchmarks — CONFIRMED

Benchmark timing differs: gpu.py:31, 53-58 takes the median of at least 5 calls over `BUDGET_S` = 1.5 s after 3 warm-ups. debesheedas.py:52 takes the median of 3 whole runs, and encoders.py:150 takes reps = 3 (1 for guetzli). The methods are not one shared timer.

### T-018 [medium] Tests that check version literals in ignore reasons and fixtures — CONFIRMED

writeback lib.rs:800-806 builds its writer with the literal version 1 and cap 1 `<<` 20, while thumbs lib.rs:493-545 uses `pg_render::THUMBS_VERSION` and 1 `<<` 30. The ignore reason in pairs.rs:246-247 is a separate, documented reason.

### T-019 [medium] Proptest case counts and API spelling differ — CONFIRMED

Proptest case counts differ: `listing_laws.rs:30` uses 12 cases, container lib.rs:1295 uses 200, and `search_laws.rs` uses the default. Spelling also differs: container uses proptest::collection::vec and `search_laws` uses prop::collection::vec.

### T-020 [medium] Skip and ignore mechanisms differ across tests — CONFIRMED

Skip mechanisms differ: exiftool lib.rs:288-290 returns None and `PG_REQUIRE_TOOLS` makes it a failure, `pg_testkit::skip` only prints (testing.rs:83-84), codec decode.rs:434 is #[ignore] behind an env var, dedup join.rs:2900 is #[ignore] with env vars, and writeback.rs:127-128 returns early when ExifTool is absent.

### T-021 [low] Case-name spelling in `run_folder` second argument mixes pg- prefix and hyphen or snake forms — CONFIRMED

Case names mix the pg- prefix, hyphens and `snake_case`: pipeline.rs:947 uses pg-pipeline, pipeline.rs:1172 uses pipeline-gather, thumbcache `other_process.rs:48` uses other-process, transaction.rs:201 uses kebab-case, and library layout.rs:183 uses pg-layout. The cited crop lib.rs:450 is a doc comment, not a `run_folder` name.

### T-022 [low] Serialising static lock declared in several test binaries — CONFIRMED

The same serialising static, static `ONE_PASS`: `Mutex<()>`, is declared in library/tests/borders.rs:19, library/tests/pairs.rs:19 and thumbs/src/lib.rs:448, with near-identical comments.

### T-023 [low] Test naming and doc comment styles vary — CONFIRMED

Test doc style varies: analyze.rs:219 puts a /// doc on a test, app.rs:227 and images.rs:388 and others use // line comments above tests, tally.rs:200 has no doc, and names vary between sentences (video.rs:134 is videos, thumbcache lib.rs:595 is eviction).

### T-024 [low] Stale or wrong test doc comments — CONFIRMED

scan lib.rs:1345-1347 has two stacked doc comments on type Tree describing the same fake tree. thumbcache lib.rs:880 says a few hundred entries and the loop writes 400. scan lib.rs:2129-2131 has a doc about a 200 000-file walk placed above a different test. `NOTIFICATIONS_DESIGN.md:355` is a stale list. The stacked duplicate is clearly wrong.

### T-025 [low] Assertions on exact counts and message text couple tests to data — CONFIRMED

Tests pin exact counts and text: codecs.rs:467 asserts files.len() == 72, notice.rs:1538 sets KINDS = 50, `rules_reference.rs:87` asserts 2033 rows, query.rs:520 asserts SortKey count 14, and `test_benches.py:49` asserts an exact error message.

### T-026 [low] Platform gate style differs between attribute, crate-level and runtime checks — CONFIRMED

Platform gates use four styles: `crop_folders.rs:4` has a crate-level #![cfg(windows)], dir.rs:707 and others use #[cfg(windows)] on the item, file.rs:347 uses a runtime cfg!(windows), and refusal.rs:208 uses #[cfg(all(test, windows))].

### T-027 [low] Hand-rolled pseudo-random generators in tests — CONFIRMED

Hand-rolled generators: browse/read/tests.rs:452 defines struct Rng with xorshift seeds, thumbcache lib.rs:1007 uses xorshift with a fixed seed, and desktop `tests/qml/tst_gridrow.qml:344` uses an LCG. catalog writers.rs:1858 has a SplitMix64 generator. The cited container lib.rs:239 is a length parse, not a generator.

### T-028 [low] Known-gap statements in plans about test coverage — PARTIAL

The plan's coverage gaps are real: LANGUAGES.md:130 says the pseudo scenarios run but are not compared, and `REFACTOR_LEDGER.md:90` (F-34) says only doctest = false is set. But LANGUAGES.md:129 lists cargo deny in just lint, which lint.py does not run (see DOC-001). eval/jpeg-decoders/linux.py:176-177 runs only clippy and fmt, as stated. Mixed: most gaps are accurate, one is stale.

### T-029 [low] Ignored test reason and invocation name different crates — PARTIAL

`compares_real_jpegs` is in crates/codec/src/decode.rs:438, and its invocation (cargo test -p pg-codec) is correct. `JPEG_DECODERS.md:226` calls it the app's ignored test. The app is the whole product, so the wording is loose rather than wrong.

## concurrency-unsafe

### CU-001 [medium] Lease heartbeat cadence: plan says 5 s thread, code renews every 20 s in write transactions — PARTIAL

`BACKGROUND_JOB_AUDIT_FIXES.md:82` describes a heartbeat thread that beats every 5 s. scan lib.rs:205 has `RENEW_EVERY` = 20 s, scan lib.rs:45 renews with each batch and every 20 s, and catalog lease.rs:21 writes the heartbeat as the first statement of each write transaction. The code documents a different mechanism. Whether the plan is the decided design or a proposal was not settled, so the plan is stale against the code.

### CU-002 [medium] Cancel and stop flags use Relaxed in some places and SeqCst or Release/Acquire in others — CONFIRMED

app reader.rs:150, :170 and :243 store the cancel flag with Relaxed. catalog core.rs:304 loads its progress-handler flag with Relaxed. transaction.rs:76 and jobs control.rs:342 use SeqCst, import pairs.rs:127 uses SeqCst, and thumbcache lib.rs:129 stores Release and reads Acquire (:134). The same kind of stop flag uses three orderings.

### CU-003 [medium] Lifecycle opens write transactions directly, bypassing `write_transaction` and its busy retry — CONFIRMED

lifecycle.rs:112 and :168 open a transaction with `Transaction::new_unchecked(conn`, Immediate), which skips `write_transaction`. `write_transaction` (transaction.rs:87-97) retries on DatabaseBusy, so these two writes have no busy retry.

### CU-004 [medium] Pre-check outside the write lock, then write in a second transaction — PARTIAL

The read-then-write pattern is real: inbox.rs:142 checks before `write_transaction` at :145, ledger.rs:189 checks before the write at :192, and pair.rs:393 probes outside the transaction. lifecycle.rs:108 reads outside, but :169 checks again inside the transaction, so that one is safe. inbox.rs and ledger.rs were not confirmed to re-check inside.

### CU-005 [medium] `CLIPPY_LEDGER` unsafe-lint policy is not implemented; `NOTIFICATIONS_DESIGN` says no forbid exists — CONFIRMED

`CLIPPY_LEDGER.md:153` sets the `unsafe_code` policy (forbid where there is no unsafe, deny with per-item expect elsewhere). Cargo.toml has no `unsafe_code` lint and no crate has `forbid(unsafe_code`). The plan is not implemented. `NOTIFICATIONS_DESIGN.md:363` states no forbid exists, which matches the code.

### CU-006 [medium] Three docs give three unsafe-site totals with different units — CONFIRMED

`CLIPPY_LEDGER.md:147` gives 472 unsafe sites (446 desktop, 20 platform). `NIGHTLY_AND_BUILD_FLAGS_PLAN.md:153` gives unsafe lines by crate (video 104, platform 72, desktop 68). `VALUE_PROVENANCE_IDEAS.md:52` gives about 245 lines. The three use different units, and platform is 20 in one and 72 in another with no unit stated.

### CU-007 [medium] `GUI_THREAD_PLAN` describes an unsafe block in pg-catalog that the crate source does not contain — CONFIRMED

A grep for unsafe in crates/catalog/src returns no file. `GUI_THREAD_PLAN.md:238` says one documented unsafe block exists in pg-catalog, so the plan describes code that is not there. `GUI_THREAD_PLAN.md:1719` cites Cargo.toml:65 for `undocumented_unsafe_blocks`, but the lint is at Cargo.toml:69 and :65 is `unreachable_pub`.

### CU-008 [medium] Two channel libraries used for the same job across the workspace — CONFIRMED

crossbeam-channel is used in two files (jobs pipeline.rs:21 and control.rs area, plus its manifest), while 14 files use std::sync::mpsc (for example jobs control.rs:785-787). The workspace mixes two channel libraries for the same worker-to-caller handoff. The cli lib.rs:971 cite is a blank line, so the cli example is off.

### CU-009 [medium] C++ cross-thread flag is a plain bool while sibling bridges use std::atomic — CONFIRMED

`desktop/cpp/frame_sink.h:26` declares bool open = true as a plain bool, and `frame_sink.cpp:28-29` writes it from close(), which runs on the sink's thread, while the posted lambda reads it on the app thread. providers.cpp:34 uses `std::atomic<bool>` cancelled for the same cross-thread job, and screenshot.cpp:134 uses std::atomic for `last_frame_ms`.

### CU-010 [medium] Benchmark cleanup removes every `thumbs_*` folder, while its comment says only this process's — CONFIRMED

`bench_thumbs.py:377` globs `thumbs_*` under `CACHE_DIR/tmp` and removes every match with rmtree. The comment on the same line says this process's and the workers' folders. The glob takes all of them, so the comment describes a narrower cleanup than the code does.

### CU-011 [low] Unnamed production threads, where most spawn sites use Builder::name — CONFIRMED

Production threads are unnamed in places: cli report.rs:23 and :78 use std::thread::spawn with no name. Across crates there are 28 thread::spawn calls against 17 Builder::new() uses, so naming is common but not universal. The cited cli lib.rs:972 is a `run_folder` call, not a spawn.

### CU-012 [low] Mutex poisoning recovered with two closure forms; one test module unwraps — CONFIRMED

Poison recovery uses `unwrap_or_else(|poisoned|` `poisoned.into_inner(`)) in app yandex.rs:270 and thumbcache writer.rs:95, and `unwrap_or_else(PoisonError::into_inner`) in the desktop. thumbs lib.rs:486 unwraps `PANIC_ON.lock(`) in its test module, while :457 uses `PoisonError::into_inner`.

### CU-013 [low] Desktop defines the same poison-tolerant lock helper in three files — CONFIRMED

The same poison-tolerant lock helper body appears in desktop `log_model.rs:59-61` and notices.rs:222-224, and pump.rs:28-30 is a generic variant with a comment. Three copies of one helper.

### CU-014 [low] Thread-start failure handled two ways in the Yandex fetch pipeline — CONFIRMED

Yandex images.rs:108-110 handles a thread that cannot start by leaving its results queued and decrementing the fetcher count. yandex.rs:394-396 `spawn_worker` clears busy and status and returns the error. Two strategies for the same failure.

### CU-015 [low] Counters and flags in jobs/pipeline.rs mix Relaxed and SeqCst — CONFIRMED

jobs pipeline.rs mixes orderings: the clock adds and reads use Relaxed (:134, :138), the reader index uses Relaxed (:245), and the sent, dropped, stop and failed counters use SeqCst (:275, :301-302, :367-368, :456-490). The cited :242 is a priority call and :272 a break, so those two cites are off.

### CU-016 [low] Hash-container ban is enforced only in two crates' clippy.toml — REFUTED

The hash-container ban is in the root clippy.toml:14-15 as well as crates/client/clippy.toml:13-14 and crates/model/clippy.toml:12-13. Crates with no own clippy.toml inherit the root ban, so it is not enforced in only two crates. scan lib.rs:923 relies on it, consistent with that.

### CU-017 [low] Process-wide PATH, environment and DLL search mutated without restore — CONFIRMED

Process-wide state is changed without restore: eval/accel.py:295 prepends `TRT10_DIR` to os.environ PATH, `test_encoder.cpp:979` calls `_putenv_s` to prepend the DLL folder to PATH, symbolize.py:130 calls SetDllDirectoryW with no restore, and tools/linux.py:290 sets os.environ `PG_HEAD_SHA` and never removes it.

### CU-018 [low] Pairing scripts restore probe environment variables inconsistently — CONFIRMED

keep.ps1:127 removes its probe variables at script end. sweep.ps1:43, :48 and :49 set or remove per run, and no end-of-script cleanup exists. `peak_mem.ps1:39` sets `PG_PROBE_WORKERS` and its end removes only the work folder. The three scripts restore the environment in three ways.

### CU-019 [low] Pointer provenance API differs for the same job of rebuilding an OS handle — CONFIRMED

platform picker.rs:215 rebuilds the owner HWND with `std::ptr::with_exposed_provenance_mut`, and process.rs:129 rebuilds the job handle with `without_provenance_mut`. Two APIs for the same job of turning a kernel handle back into a value. The testing/tests.rs:330 cite is beyond the file's end.

### CU-020 [low] Return values of queue, join and `begin_move` calls discarded — CONFIRMED

desktop shell.rs:341 and notices.rs:520 discard queue results with let `_`, `lane_model.rs:317` discards the bool from `begin_move_rows`, and cli lib.rs:300, :387, :417, :444, :538 and report.rs:87 discard printer.join() results. The cli cite at :87 is a match, but the discards are real.

### CU-021 [low] Unsafe functions in the SIMD kernel have no Safety section; others do — CONFIRMED

dedup join.rs:1423 (`scan_popcnt`) and :1435 (`scan_avx2`) are unsafe fns with no # Safety section, while their call sites carry SAFETY comments. desktop animation.rs:43-44 documents # Safety on `set_sink`.

### CU-022 [low] Perf capture flags restored without a drop guard — CONFIRMED

perf lib.rs:453-457 sets ENABLED and `CAPTURE_HERE`, runs f(), and restores them with no drop guard, so a panic in f skips the restore. import pairs.rs:120 uses AbortOnDrop, which restores on drop, as the contrast.

### CU-023 [low] Atomic-write temp-file naming differs across eval and tools — CONFIRMED

Atomic-write temp names differ: eval encoders.py:127-128 uses a fixed .tmp suffix, encoders.py:63 uses a uuid name, bench.py:58 uses .partial, and `setup_ffmpeg.py:37` documents a .partial download.

### CU-024 [low] Download integrity checked in one tool and not in another — CONFIRMED

The `tools/ci_setup.py` cite is at `tools/profiling/ci_setup.py`. There it fetches `https://sh.rustup.rs` with urlopen (:51) and runs it with no checksum. `setup_ffmpeg.py` verifies a SHA-256 before it installs the archive. The cite at `setup_ffmpeg.py:80` is `top_folder`, but the check exists in that file.

### CU-025 [low] Eval job tag uses Python hash(), which is salted per process — CONFIRMED

eval/ann/run.py:166 builds a job tag from abs(hash(json.dumps(...))) % `10**8`. Python's str hash is salted per process, so the tag changes between runs and the cache or the folder name is not stable.

### CU-026 [low] CUDA blocking-sync flag set through two APIs, and one return value ignored — CONFIRMED

The blocking-sync flag is set two ways: `dec_gpu.cpp:49` uses cudaSetDeviceFlags and ignores its return value, nvjpeg.py:91 uses cudaSetDeviceFlags, and `nvimgcodec_bench.py:39` uses cuDevicePrimaryCtxSetFlags through the driver DLL.

### CU-027 [low] SQL function marked deterministic but reads shared mutable state — CONFIRMED

The cited file is at `tools/profiling/pairing/grow_catalog.py`. moved() at :121 is registered with deterministic=True and reads current, a bytearray that the copy loop mutates between copies (:115-116). The cite path `tools/profiling/grow_catalog.py` does not exist.

## api-docs

### AD-001 [high] eGPU `pixel_budget` default documented as 64 Mpx but coded as 64 Mi pixels — CONFIRMED

`encode_gpu.h:40` documents `pixel_budget` as 0 = 64 Mpx. `encode_gpu.cu:935` uses 64ull `<<` 20, which is 67,108,864 pixels, or 64 Mi pixels. The doc uses the decimal mega prefix and the code the binary one, a difference of about 4.9 percent.

### AD-002 [medium] Analyze scale rule (512x320 minimum) held in Rust and Python — CONFIRMED

The analyze scale rule is held in two places: jpeg-decoders src/lib.rs:54 documents it and :78 codes it as (512, 320) or (320, 512). nvjpeg.py:204-206 restates the same rule and cites pg-analysis. A change to one needs a change to the other.

### AD-003 [medium] Unsafe fn without a # Safety section where neighbours have one — CONFIRMED

`video_player_bridge.rs:15` declares an unsafe fn `new_video_player` with a one-line doc and no # Safety section. `video_source.rs:23` and `live_reload.rs:26` document their unsafe contracts under # Safety, so the neighbours do have one.

### AD-004 [medium] Public types, traits and consts without an item doc where neighbours have one — CONFIRMED

Public types without an item doc include Pool (app pool.rs:7), Inbox (inbox.rs:372), JsonStore (store.rs:21), Catalog (catalog core.rs:37), Control (jobs control.rs:202) and Runner (library runner.rs:30), while neighbouring items in the same files carry docs.

### AD-005 [medium] Public functions and methods without docs where neighbours have them — CONFIRMED

Public functions beside documented ones are undocumented in many files: `set_paused` in app library.rs:230, new in pool.rs:13, details in catalog browse read details.rs:12, `finish_scan` in roots.rs:145, and `clear_stop` in jobs control.rs:349. The gap is spread through the crates.

### AD-006 [medium] qproperty and qinvokable doc coverage uneven across desktop bridges — CONFIRMED

`gallery_bridge.rs:41-79` has qproperty items count, revision, anchor and `viewed_*` without docs while items at 42-62 have them. `viewport_bridge.rs:86-100` has undocumented qinvokables, and `library_bridge.rs:18` has roots with no doc. `command_line_bridge.rs:19` documents every property, so the reference pattern exists.

### AD-007 [medium] Doc comments attached to the wrong item or duplicated — PARTIAL

`library_bridge.rs:19` is directly above `kept_removals` (line 20), and its text (waits for an unfinished write) reads as that property, so the attachment is right and roots at line 18 has no doc. The claim that each doc is shifted one property off would need the property at line 22 to confirm, and that line was not read. The writeback test docs at :71-73 and the crop.rs:68-70 doc are real duplications of the same paragraph, which is the stronger part of the finding.

### AD-008 [medium] Python docstring and argparse description conventions diverge — PARTIAL

`install_exiftool.py:46` takes the first paragraph of the docstring, `make_test_video.py:18` takes line 1 of the docstring split by lines, and cgtab.py:325 uses the whole docstring with a raw formatter. The three conventions are real. Most of the other cites (`bench_data`, perf-compare, `make_bench_jpeg`, `setup_ffmpeg`, `synthetic_library`) are at paths that do not match the cite, so they are not checkable as written.

### AD-009 [low] Public fields and variants with missing docs while siblings have them — CONFIRMED

Fields are undocumented beside documented ones: catalog fixture.rs:15 (File), inbox.rs:72-73 (Entry), inbox.rs:92-96 (Mark), ledger.rs:143 (FailedFile), model details.rs:13 (Details.size), model view.rs:15-16 (Listing), and the others listed. The gap is common across the model and catalog.

### AD-010 [low] Doc comments on public items name private machinery or other-crate variants — PARTIAL

join.rs:516 mentions `run_team`, which the doc says is private; that is an internal name in a public doc. app error.rs:25-26 names client message variants, which the app is allowed to name because it depends on the client's text layer. Only the join doc is an internal-name leak, and the finding's second instance is an intended cross-layer reference.

### AD-011 [low] Visibility and re-export pattern varies for public surface — CONFIRMED

Visibility and re-export patterns vary: dedup lib.rs:8-9 exposes join and `union_find` as pub mod with only UnionFind re-exported, client controller.rs:17-23 makes every submodule pub mod, catalog lease.rs:15 uses pub(crate) fields, pair/coverage.rs:8 has fully public fields, and metadata xmp.rs:32 is pub with pub(crate) fields. Each is a choice, but no single rule holds across the crates.

### AD-012 [low] Derive sets differ across public data types — CONFIRMED

Derives differ across public data types: model state.rs:12 FileState derives Copy and Eq, state.rs:28 FileSource lacks Eq and Copy, state.rs:39 Placeholder derives nothing, import lib.rs:40 ImportStats derives nothing while hashes.rs:19 derives Clone, Copy and Debug, edit crop.rs:24 Cropped derives nothing, and codec pixels.rs:5 Rgb derives Clone and Default only.

### AD-013 [low] Return-value documentation style differs across write methods — CONFIRMED

Return-value docs end differently: writers.rs:191 ends with a trailing fragment, Whether a row was written, and :436 has the same fragment. inbox.rs:100 states the bool meaning once in the module doc and again per method. Low severity.

### AD-014 [low] Public statics documented with plain comments; a heading without a blank line — CONFIRMED

maker.rs:20-22 documents the public statics DECODE, RESIZE, ENCODE and THUMBHASH with plain // line comments rather than rustdoc. notices.rs:45-46 has a ## heading directly after a paragraph with no blank //! line, unlike the other headings in that module doc.

### AD-015 [low] Mode lists in the two decoder harness tools are documented differently — PARTIAL

Both cites are missing at the paths given: the harness is at eval/jpeg-decoders/harness/harness.cpp, and the main.rs cite does not exist as written. The mode-list difference between the two tools cannot be checked from these cites.

### AD-016 [low] Script-header conventions: PowerShell help blocks and first-line verb form — CONFIRMED

Script headers differ: `few_sweep.ps1:2` and the other pairing scripts use .SYNOPSIS help blocks, while `set_owed.py:1` opens with an imperative docstring and keep.py:1 with a noun phrase, and `setup_ffmpeg.py:2` opens third person. Low severity.

### AD-017 [low] Duplicated boilerplate disclaimer across modgraph tools — CONFIRMED

dupes.py:4, edges.py:6 and `api_surface.py:8` repeat the same boilerplate disclaimer, A report for a review, not a gate. The wording is copied verbatim into three modgraph tools.

### AD-018 [low] Doc states a caller contract the body does not enforce — CONFIRMED

inbox.rs:297 documents `mark_all_read` as marking the entries displayed and never one the window was not given. The body at :298-299 calls only `self.mark_read(ids`, now), and nothing in it checks what the window was given. The contract is stated and left to the caller.

### AD-019 [low] TestHooks.qml says options are read in the file but also says Rust reads them — CONFIRMED

TestHooks.qml:29 says The options are read here, while the comment at :64 says they are read by Rust in `command_line.rs` and given to this component as typed properties. The file's own header contradicts its later comment.

### AD-020 [low] Plan docs cite API shapes that differ from the code — PARTIAL

The plans already mark their own API shapes as not verified against current code: `CATALOG_SIZE_PLAN.md:225`, `DECODE_ONCE_PLAN.md:380` and `BOUNDED_MEMORY_PLAN.md:121` each carry a not verified marker in the finding text. The divergence is expected for a plan, and the plans say so. The finding is accurate about the gap but not about a hidden error.
