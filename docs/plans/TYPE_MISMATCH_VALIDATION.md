# Type mismatch findings: open items

Six findings from the type-mismatch review are still open. The findings were first written at `489f3d31`. Item 1 was adversarially validated at `0e6e1fd9`, and its corrections are applied. Item 2 was checked at HEAD (`fd5e727a`), and its design is below. Item 4 was re-validated at `b621641e`, and item 6 at `b621641e`. The Qt sentinel sweep is no longer in item 1; it is its own plan, `docs/plans/QT_SENTINEL_SWEEP.md`. Item 6 is not reached at the 1B-file target. Items 1 and 6 overlap steps of Phase 7 in `docs/plans/BOUNDED_MEMORY_PLAN.md`. Each overlap is noted under its item. The design is not finalized. Each entry gives the current location, the defect, its impact, and the planned fix. Fixed and stale findings are not listed. Item 1 is open: its row-id defect is fixed, and its entry lists the defects that remain and their planned fixes. The Exif IFD pointer and Rating findings are complete and have been removed from this file.

## 1. The saved last file is lost after a rating, and a catalog error reads as no file (`crates/desktop/src/shell.rs:95`)

**Status.** The row-id defect is fixed in code (`fab0512c`). `Prefs.last_file` is an `Option<FileStamp>` (`crates/model/src/ids.rs:55`), written as `null` or an object (`crates/app/src/prefs.rs:114-157`), and a number of either sign reads as none. The stamp is identity, size and mtime today; the change below replaces size and mtime. The save takes the shown file's stamp with `Catalog::stamp_of` (`crates/desktop/src/shell.rs:95-98`). The start resolves it with `Catalog::find_by_stamp` in `Core::new` (`crates/desktop/src/controller.rs:61-65`), with the catalog open. `start_state` (`controller.rs:102`) is a function of the prefs and leaves `last_file` to that caller. Tests: `crates/catalog/src/lookup.rs` (`stamp_tests`) and `crates/app/src/prefs.rs`. The desktop controller tests run with the Qt bin on `PATH` (Qt at `C:/Qt/6.12.0/msvc2022_64`), and the six start-state tests pass at this revision. The Qt sentinel sweep is in `docs/plans/QT_SENTINEL_SWEEP.md`.

**Defects, in the code as committed.**

1. **The saved stamp goes stale after a rating.** The stamp is identity, size and mtime. The app's own write-back changes size and mtime: the catalog takes the new values after the copy (`crates/catalog/src/queue.rs:376`). A rating therefore changes the stamp the prefs hold, and the restore finds nothing. The identity and the creation time do not change, because the write copies in place and does not rename a temp file over the original (`crates/writeback/src/lib.rs:5-6`, `crates/writeback/src/inplace.rs:1-6`). Measured on a synthetic file: an in-place rewrite changed size and mtime, and kept the creation time and the NTFS file ID.
2. **A failed catalog lookup erases the stored file.** The save writes `stamp_of(…).ok().flatten()` (`crates/desktop/src/shell.rs:95-98`), so a lookup error writes `None` over the stored stamp. The start (`crates/desktop/src/controller.rs:61-65`) resolves a lookup error to `None`, and the next save then writes whatever is shown over the stored stamp, which is `None` if nothing is. Either way a transient catalog error loses the remembered file, and nothing tells the user.

3. **A shown file can change without a save.** The shown file is the viewer's while it is open, and the selection's otherwise (`crates/desktop/src/gallery.rs:41-56`). It is published again on every revision change (`crates/desktop/src/gallery/model.rs:301`, `:309`). The session save is armed only by a change of the current index: `CurrentChanged` (`crates/client/src/controller.rs:363`), which the QML `onCurrentChanged` handler sends when the grid's current item changes (`crates/desktop/qml/Main.qml:184`). `set_current_item` (`gallery/model.rs:251-253`) returns early when the index is unchanged, so the handler does not fire. A file deleted from disk is marked missing by the folder's scan sweep (`crates/catalog/src/roots.rs:206`). A re-list then removes the selected file (`onScanned` after the scan, or `onImported`, `crates/desktop/qml/Main.qml:155-156`, then `Event::Reload`, `controller.rs:413`), `selection_after` keeps the index (`crates/client/src/grid.rs:456`), or moves it to the last item if the list got shorter. Only the kept index is the case here: it now holds a neighbouring file, nothing is armed, and the stored stamp keeps the removed file. (A moved index changes the current item and arms the save.) The restore then finds nothing, though the neighbour was on screen. Traced from the code; not run. A grid rating changes no shown file and arms nothing, and viewer navigation does arm the save, through the viewer's `onIndexChanged` (`Main.qml`, the viewer element, which calls `shell.currentChanged`).

*Related, not reconciled:* `docs/plans/BOUNDED_MEMORY_PLAN.md` step 7.9 (line 1508) names `save_last_file(Option)` and `loadedAnchor`, which overlap this item's proposed Rust-side ownership.

*History:* before `fab0512c`, the prefs held an `Option<FileId>`, stored on disk as `-1` for none. A row id names a file only in the catalog that assigned it. The rebuild keeps ids (`crates/catalog/src/rebuild.rs:188`, and the high-water mark carried over, `rebuild.rs:284-297`), but a catalog made anew hands ids out again from 1, so a saved id could name a different file. The stamp replaced the id for that reason.

**Impact.** A file that was rated, if it is the remembered file, is not selected again at the next start, because its size and mtime change and the stored stamp no longer matches. A failed catalog lookup at the save or the start loses the remembered file, and nothing says so. A selected file that a re-list removes (deleted, or moved out of the folder) leaves its neighbour on screen unremembered, so the next start restores nothing. No rating is lost: the write-back and the catalog keep it. Only the restored selection is affected.

**Fix (planned).**

1. **The stamp is identity and creation time.** `FileStamp` (`crates/model/src/ids.rs:55`) drops size and mtime and keeps `identity` and `created`. The catalog sets `created` from the listing and fills it only when it is NULL (`crates/catalog/src/writers.rs:181`, `coalesce(created, …)`, backfill at `:306-307`). It changes only when a row takes a new identity (`take_identity`, `crates/catalog/src/pair.rs:475-500`), where it moves with the identity. Two forms exist. A rebind (`REBIND`, `coalesce(?4, created)`) keeps the row's old value when the new file has no creation time, so the stamp then carries a creation time that is not the new file's. That breaks the creation-time refusal. The fix is for REBIND to take the new file's value, as REPLACE does, so a stamp never carries another file's creation time; the row is then not remembered until a scan backfills it. A replacement (`REPLACE`, `?4`, `crates/catalog/src/pair.rs:500`) takes the new file's value, NULL included, so the row is not remembered until a scan backfills it. Both cases need a file with no creation time. Identity can be reused after a delete, but a new file gets a new creation time, so the creation time refuses a reused identity. A file whose platform gives no creation time (`created` NULL, `crates/platform/src/dir/listing.rs:27`) is not remembered. There is no fallback to size and mtime, because that brings back defect 1. `find_by_stamp` (`crates/catalog/src/lookup.rs:122`) matches the identity and `created` on a row that is not `missing`, and `stamp_of` (`lookup.rs:103`) returns no stamp for a NULL `created`. The catalog displaces a row whose identity matches but whose creation times differ (`crates/catalog/src/writers.rs:300-305`), so the creation time already decides identity there. Its pairing does not use that rule alone: the tiers that pair a file to a row on another volume or in another folder require size and mtime, with `created` only as an optional check (`crates/catalog/src/pair.rs:115`, `:165`). The stamp does not follow a move across volumes, because the identity includes the volume. A NULL `created` is backfilled by the next scan that reads the file (`writers.rs:306-307`), so such a file becomes rememberable then, not never.
   - *Documentation to change with the stamp:* `crates/model/src/ids.rs` says size and mtime are how the scanner tells a row is a different file (lines 30-31 and 52-53). That text moves to creation time.
   - *Known trade-off:* the stamp does not refuse a file that another program rewrote in place. Measured on a synthetic file: a copy over an existing file kept its identity and creation time. The catalog treats that file as the same one too, and the old size-and-mtime stamp refused it. The restore therefore keeps the selection on a file whose content changed under the same name.
   - *Volume types.* The scanner refuses any library folder whose file system is not NTFS (`crates/scan/src/refusal.rs:60-63`), so FAT and exFAT are out of scope. The stamp is designed for NTFS only.
2. **The start takes the resolved file.** `start_state` takes `last_file: Option<FileId>` as a parameter, and the caller resolves the stamp in `Core::new` (`crates/desktop/src/controller.rs:59-65`). The 11 test calls in `controller.rs` (lines 233 to 320) and the one in `crates/desktop/src/shell.rs:752` pass the argument.
3. **A catalog error is reported.** A new notice `LastFileNotKept { error }` (key `last-file-not-kept`, `Level::Error`, `Source::Catalog`) follows the steps for adding a kind in `crates/model/src/notice.rs:50-59`:
   - the variant and its `ids!` string (`notice.rs:800`, `:1168`), and the `level` (`:1285`), `source` (`:1313`) and `lifecycle` arms;
   - its sample in `Notice::samples()` (`:1617`), its line in `notice/golden.jsonl`, and its `ordinal` arm. The ordinal is 51: `ChangesUnseen` is 50 (`:1702`), so `KINDS` (`= 51`) becomes 52;
   - its row in `notice/resolver_table.golden`, its words in `crates/wording/locales/en/text.ftl` (`msg-notice-last-file-not-kept-title` and `-body`), and its `error` argument in `crates/wording/src/notice.rs` (the arm at `:95`, as `SessionNotRead` has). The `notice.rs` doc comment names `client/src/text/notice.rs` for the args; that file does not exist, and the doc is stale.
   - Its resolver is `Resolver::NextSuccess(Op::LastFile)`. `Op` (`notice.rs:931-952`) has no such variant, so one is added. A save reports success through `app::observe(Op::LastFile, …)` (`crates/desktop/src/app.rs:29`), which the prefs save does not do today. A successful save of no file clears the notice too, and so does a start whose lookup finds no row: `Ok(None)` is a success, and only a failed lookup is an error.
   - The notice is posted where the outcome is, in the desktop save and start. A notice raised at the flush is not shown: the inbox closes in `App::shutdown` (`crates/app/src/app.rs:210-211`), after the flush. It is logged synchronously (`crates/app/src/inbox.rs:433-434`), and it is also persisted (decided): when the save at quit fails, the prefs record the error in a field of their own, written even when the stamp is not. The next start posts `LastFileNotKept` from that field, and the next successful save clears it.
4. **A save whose lookup fails writes nothing.** `app::observe(Op::LastFile, catalog.stamp_of(file), …)` returns `None` on failure (`crates/desktop/src/app.rs:29`, failure arm at `:35-38`), and the save skips the stamp write on `None`. The error field of item 3 above is written separately, so the two do not conflict. This is what stops defect 2 from erasing the stored stamp.
5. **The save also runs at flush, at most once.** A flush pushes `Command::SaveLastFile` when the start is over, unless `fire` already pushed one because a `SaveSession` was armed (`crates/client/src/controller.rs:606-620`, `:629-631`). This does not cover defect 3, which arms nothing: today the flush sends `SaveLastFile` only through a `SaveSession`, so nothing is saved for it. The fix is for the flush to send `SaveLastFile` whenever the start is over, whether or not a timer was armed. The flush test in the list below must fail before this change. `fire`'s `is_started` guard (`controller.rs:629`) keeps a start's options unsaved. The stamp is read on the window thread before the window's writes stop (`crates/desktop/src/lib.rs:187`), so the catalog is still open. When nothing is shown at quit, the flush writes `None` only if a file was shown during this run (Decided B below); otherwise it keeps the stored stamp. A list that is still loading at quit does not change what is shown (`open_listing` only sets the loading state), so the save keeps the file that was shown before the load.
   - *Decided (B):* a save does not overwrite the stored stamp with `None` unless a file was shown during this run. A start whose lookup failed keeps the stored stamp, and a later save of an empty session keeps it too. The stored stamp is checked again on restore, so keeping one that no longer matches is harmless; erasing a good one is not. The run records whether a file was shown (a run-local flag, set when the shown file is first known), and the save checks it before it writes `None`.

**Disk format.** `null` for none; an object with the stamp's fields for a file; a number of either sign reads as none. An object written before this change, with size and mtime and no `created`, fails to read as a stamp and so reads as none. The untagged read falls through to `Other`, so the last file is forgotten once and the prefs file still loads. A row id from an earlier catalog is never resolved, because it has no meaning here.

**Why not the alternatives.** Size and mtime change with the app's own writes (defect 1). A path breaks on a rename, which the restore should survive. A content hash changes with every write. The catalog itself would lose the value on a rebuild, and the prefs file is where it outlives one (`crates/app/src/prefs.rs:1-4`).

**Tests.** Written, against the current size-and-mtime stamp (they change with step 1): a reused identity with other size or mtime selects nothing (`a_reused_identity_is_a_different_file`); a renamed file keeps its stamp (`a_renamed_file_keeps_its_stamp`); a missing row selects nothing (`a_missing_file_is_not_selected`); a displaced row has no stamp (`a_displaced_row_has_no_stamp`) (all in `crates/catalog/src/lookup.rs`); `null` reads as none, and a number of either sign reads as none and is not resolved (`the_last_file_is_its_stamp_and_a_number_reads_as_none`, `crates/app/src/prefs.rs`).

**Planned, not written:**

- a rated file is still found after the app changes its size and mtime (`lookup.rs`);
- a file whose `created` is NULL has no stamp and is not remembered (`lookup.rs`);
- a stamp written and read back as one value (`prefs.rs`);
- an object saved with size and mtime reads as none, and the prefs file still loads (`prefs.rs`);
- a fresh catalog that holds the saved file under other row ids restores it, by identity and `created`; a fresh catalog that does not hold it selects nothing. The test `a_stamp_finds_its_file_by_identity_whatever_its_row_id` (`lookup.rs`) is named for the row id, but it changes `file_ref`, so it is renamed or replaced by these two cases. The first case is the design's intent: a rebuild of the same library keeps the selection;
- a file rewritten in place by another program keeps its stamp (the known trade-off above), so the test documents it rather than refuses it;
- a save whose lookup fails writes nothing and posts `LastFileNotKept` (desktop, needs a seam to inject the failure);
- a grid rating changes no shown file and arms no save, and the stamp still restores the file after it (`crates/client/tests/controller_laws.rs`, beside `a_pending_session_save_is_made_at_the_close_with_the_last_file`, plus the size-and-mtime case above);
- a selected file removed by a re-list keeps its index, nothing is armed, and the flush saves the file on screen (`controller_laws.rs`);
- a new file with no creation time rebinds a row and leaves its creation time NULL, as a replace does (`take_identity`, `crates/catalog/src/pair.rs:498-500`);
- a lookup that finds no row is a success for the notice's resolver, and only a failed lookup posts `LastFileNotKept`;
- a failed start reports `LastFileNotKept`, and the notice's own checks pass: the sample, its golden line, the `ordinal`, the resolver table and the words (`crates/model/src/notice.rs`, `crates/wording`).

**Out of scope.** The Qt sentinel sweep (`docs/plans/QT_SENTINEL_SWEEP.md`); counts and positions (item 6); and the overlap with `docs/plans/BOUNDED_MEMORY_PLAN.md` step 7.9 (line 1508), which names a `save_last_file(Option)` that this item does not add.

**Not verified.** The NTFS behaviour is measured on synthetic files on the local NTFS volumes, not on the library. Network identity is not measured: the share `Y:` is NTFS and was not written to. A notice raised at the flush is logged: `inbox.post` writes it to the log before anything else (`crates/app/src/inbox.rs:433-434`), and the log is an appended line (`crates/log/src/file.rs:60`). The viewer's and the grid's paths to the save were traced from the code and not run.

## 2. Typed pipeline options: zero counts and overflow are representable (`crates/jobs/src/pipeline.rs:33-50`)

**Scope changed in phase 0 of the plan.** The CLI flags this item was written for (`--buffer-mb`, `--readers`, `--workers`, `--slots`) are removed with the command line (`docs/plans/HEADLESS_SESSION_PLAN.md`, phase A2). What remains is the typing of `pipeline::Options`, which is decided in `docs/plans/STORAGE_AND_CLEANUPS_PLAN.md` section 5 (phase B3). The readers field follows with `Readers` in `docs/plans/ADAPTIVE_READERS_PLAN.md` (phase C9).

**Current state, checked at HEAD.** `Options` is a public type of `pg-jobs` with plain `usize` and `u64` fields: `readers`, `workers`, `slots`, `buffer_bytes`. Any caller can set them to zero. The pool clamps with `.max(1)` (`pipeline.rs:793`), and `Budget::new` clamps too (`crates/jobs/src/governor.rs:76-77`). The wrap that SEC-043 reported (`docs/plans/SECURITY_FLOW_REVIEW.md:753`) is gone, because the megabyte multiply saturates. A saturated budget is `u64::MAX`, which `Budget::new` keeps, and the pool reads it as `usize::MAX` (`pipeline.rs:813`). That is an unlimited budget, not an error.

**Impact, corrected by validation.**

- No hang. A file larger than the budget is read alone: the production doc says so (`crates/jobs/src/pipeline.rs:800-801`), and the rule is `st.held == smallest` at `pipeline.rs:836`. A small budget only serializes. (The earlier citation, `:924`, is a comment in the test module that starts at `:896`.)
- A huge budget removes the target the flag sets, since the budget is soft; the pass then holds what its slots and files need. The pool never holds more buffers than there are slots (`pipeline.rs:803`), where slots are `readers + 2 × workers` (`pipeline.rs:81`), and each buffer grows to the largest file it reads. With the default 8 readers and 16 workers that is 40 slots, against a default budget of 24 × 12 MB = 288 MB. Unbounded, 40 slots of 200 MB files could hold about 8 GB. The earlier "no data or hang impact" is correct only for data and hangs.
- A zero count is silently clamped to one (`pipeline.rs:793`), and an overflowing budget becomes unlimited. Both are at the API, so any caller can hit them, not only the command line.

**Decided design.**

1. **The type stays pure.** `BufferBytes` lives in `crates/model`, beside `ThumbCap` (`crates/model/src/cache.rs`). Its constructor refuses zero and refuses a byte count past `u64` (a checked multiply). It knows nothing about memory: the budget is a soft limit, so no physical size enters the model, and its tests need no platform.
2. **The pipeline takes the typed values.** `Options::buffer_bytes` becomes `BufferBytes`. `Options::budget` and `Options::slots` use checked arithmetic, and no caller can build a zero count.
3. **Non-zero types are the rule.** `workers` and `slots` become `NonZeroUsize`. `readers` becomes the typed `Readers` of the adaptive plan (`docs/plans/ADAPTIVE_READERS_PLAN.md`); until then it is `NonZeroUsize`. The defaults become `NonZeroUsize` constants.
4. **Refusals say why, and none is silent.** A zero count, a budget that overflows, and a slot count above the stated maximum are each refused with a named reason. Nothing is clamped silently in `Options`. `Budget::new`, `Budget::set` and `Governor::new` keep their clamps (`crates/jobs/src/governor.rs:76-77, 87`), and `pipeline.rs:1465` pins `Budget::new(0) == 1`, so "no silent clamp" covers `Options`, not the `Budget` type. `Pool::with_budget` preallocates `Vec::with_capacity(slots)` (`pipeline.rs:794`), so a huge slot count is a huge allocation. The maximum for slots is to be chosen.
5. **Decided: the budget is a soft limit, not a hard cap.** The budget is a target. No value is refused for being large. One file larger than the budget is read alone, and slots still hold whole buffers (`crates/jobs/src/pipeline.rs:800-801, 836`). That is today's behaviour and is kept.
6. **Tests.** Zero cannot be built, so none is needed for it. The pool tests keep their assertions. `BufferBytes` refuses zero and overflow, and a slot count above the maximum is refused.
7. **SEC-043** is closed by the removal of the flags and by these types. Its entry is updated in `docs/plans/SECURITY_FLOW_REVIEW.md`.

**Scope (accepted).** The rules move into the type the pipeline takes, so no caller can build an invalid pool.

- `BufferBytes` is used by `pg-jobs`, which already depends on `pg-model` (`crates/jobs/Cargo.toml`). `pipeline::Options` takes `workers` and `slots` as `NonZeroUsize`, and the buffer as `BufferBytes`.
- `Pool::with_budget` takes `NonZeroUsize` slots, so the `slots.max(1)` at `crates/jobs/src/pipeline.rs:793` goes.
- Every construction of `pipeline::Options` changes. The sites are listed in `docs/plans/STORAGE_AND_CLEANUPS_PLAN.md` section 5.
- The budget's meaning is unchanged.

**Considered and not chosen: a strict memory bound.** The soft-limit decision (fix 5) rules out a hard ceiling, which is what a strict bound would be. The policies below are kept for reference, since they are the only way to make the bound strict.

- Today's exception (`crates/jobs/src/pipeline.rs:836`, `st.held == smallest`, documented at `:800-801`) lets one such file through alone. Strict enforcement must choose:
  - (a) keep the exception and state the bound as the budget plus the largest file. No behaviour change, but a weaker bound.
  - (b) refuse files larger than the budget, and report each one as a notice. The pass skips them, so results change for those files.
  - (c) read such files in pieces. `process` takes the whole file's bytes (`pipeline.rs:96`), so this needs a codec change, and it is the largest option.
- The default budget (12 MB per read in flight and per worker, `pipeline.rs:53`) decides how often (a) or (b) matters. The plan has no measurement of library file sizes, so measure before choosing a default.
- The pool tests at `pipeline.rs:1460-1481` assert the exception ("a budget of one byte still grants a file larger than it, alone"). They change under (b) and (c).
- Tests: under (b) and (c), the held total is at most the budget. Under (a), it is at most the budget plus the largest file. Under (b), each skipped file is reported.
- Decided: the soft limit. Option (a) is confirmed: a file larger than the budget is read alone, as today. (b) and (c) would make the limit hard, and are not planned.

**The lanes' memory policy (assessed from the code; see section 3).** The lanes' options come from `pipeline::Options::new` (`crates/library/src/inner.rs:65`), not from the command line. Decided: each process keeps its own pool and cap, and sharing a budget across processes is out of scope. The library's lanes use a governor, not the CLI's pool budget. Each lane gets one budget: the knee, `(readers + cpus) × 12 MB` (`crates/jobs/src/governor.rs:92`), or the cap split evenly among the lanes when the cap binds (`governor.rs` `split`). The cap is a quarter of physical memory per process (`crates/library/src/coordinator.rs:178`), and the governor rebalances when a lane retires (`coordinator.rs:555`). Each lane's pool is soft, like the CLI's: one file larger than the budget is read alone (`pipeline.rs:836`).

- **The cap almost never binds.** With 16 CPUs the knee is 288 MB per lane, so two drives need 576 MB. The cap binds only on a machine with under about 2.3 GB of RAM, or with about 28 lanes on a 32 GB machine.
- **Slots are not governed.** Each lane's options take the default slot count, `readers + 2 × cpus` (`pipeline.rs:70`, inherited through `inner.rs:65`), so total slots grow with the lane count while the budgets are split. Slots hold whole buffers, which grow to the largest file each one reads.
- **Nothing is shared across processes.** The CLI pass runs its own pool with no governor, and the CLI notes that another process can scan at the same time (`crates/cli/src/args.rs:70`). The app's lanes and a CLI pass can each use their full budget at once.
- **Assessment.** The per-lane knee is a reasonable measured default, and the soft semantics are consistent. The cap is coherent in principle, but it is a rule that rarely fires, and it doesn't cover slots. The most proper version would count slots against the same budget (divide each lane's slots by the lane count, or count total slots against the cap), keep the cap as a documented per-process target, and state that sharing across processes is out of scope.

**Tests.** The `BufferBytes` tests need no platform: `1` gives 2^20 bytes; `17592186044416` (SEC-043's value, 2^64 bytes) is refused as overflow; `u64::MAX` is refused; zero is refused. A count above physical memory is accepted, because the limit is soft. The pipeline's own tests keep their assertions.

*Closed since the first list:* the thumbnail height entry is fixed. `ThumbHeight` (`crates/model/src/size.rs:82`) holds the 16 to 2048 range (`THUMB_HEIGHT`, `size.rs:12`), and the CLI parses it with a clap `value_parser` (`crates/cli/src/args.rs:131`, parser at `args.rs:190`). `pg_thumbs::run` and `ImageRequest::thumb` take it. The plan's `ThumbHeight::clamp(u32)` was not built. The UI clamp in `client/src/image.rs` is gone, and `ThumbHeight::for_row(RowHeight, Scale)` (`size.rs`) does that job instead.

## 3. The lanes' slots and memory cap are not bounded by the budget (`crates/library/src/inner.rs:65`, `crates/library/src/coordinator.rs:178`)

**Current state, checked in the code.** One lane runs per physical drive (`crates/library/src/coordinator.rs:563`, `Inner::new` with `governor.lane()`). Each lane's pool gets a budget from the governor, the knee `(readers + cpus) × 12 MB` (`crates/jobs/src/governor.rs:92`, `pipeline.rs:53`), or the cap split evenly among the lanes when the cap binds. Each lane's pool also takes the default slot count, `readers + 2 × cpus` (`crates/jobs/src/pipeline.rs:70`, inherited through `inner.rs:65`), whatever the number of lanes. The cap is a quarter of physical memory per process (`coordinator.rs:178`, `MEMORY_SHARE` at `coordinator.rs:53`). If physical memory cannot be read (`crates/platform/src/memory.rs`, on a platform other than Windows or Linux), the cap is `None`, and there is no ceiling.

**Problems.**

1. Slots are not divided among lanes. The total grows with the number of drives, and each slot's buffer can grow to the largest file its lane reads, while the budgets are split.
2. The cap almost never binds. With 16 CPUs the knee is 288 MB, so two drives need 576 MB. The cap binds only on a machine with under about 2.3 GB of RAM (two lanes), or with about 28 lanes on 32 GB.
3. The overshoot is in the reads, not in the slot count. `read_gather` admits only the hint, then calls `grow_to(bytes.len())`, and `read_the_rest` grows to the full size with no budget check (`crates/jobs/src/pipeline.rs:667, 697`). `grow_to` only counts the bytes it reserves (`pipeline.rs:764`), so nothing blocks a read that exceeds the budget. The budget is not limited to files larger than itself. The cap already bounds the sum of the lanes' budgets (`crates/jobs/src/governor.rs:136-140`), and slots hold no bytes beyond the budget.

**The knee.** The knee is the budget one lane needs to run at the measured speed. It is `(readers + workers) × 12 MB`: a buffer for each read in flight and for each worker. The measurement in `pipeline.rs:84-87`: on 24 MP JPEGs on an HDD, with 8 readers and 12 workers, 128 MB at the default slots was 4% slower than the default, and 512 MB was no faster than 240 MB. These figures appear only in doc comments. No harness or dated record of the run is in the repository, so they are reported here, not reproduced.

**Options for the cap.**

- Keep it: a documented process-wide target that rarely binds.
- Lower it: a fraction that binds on realistic systems. A smaller budget costs little speed, given the measurement above.
- Remove it: memory is then the sum of the lanes' knees, plus files larger than the budget and the slots, with no process-wide limit. Example: six drives at 288 MB each is about 1.7 GB, and the cap would have no effect.

**Options for slots.**

- Divide each lane's slots by the number of lanes, so readers and workers shrink as drives are added. Refuted for the reads: a drive needs at least 8 reads in flight to reach sequential speed (`crates/jobs/src/pipeline.rs:4-6`), and dividing slots by lanes can starve a drive.
- Count the total slots against the cap, so the total is bounded. Refuted: slots hold no bytes beyond the budget, so counting them against a byte cap changes nothing about memory.

**Status (phase 0 of the plan).** Problems 1 and 2 are decided and closed by `docs/plans/ADAPTIVE_READERS_PLAN.md` (memory governance, phase C8): the cap is `min(physical / 4, held + available − headroom)`, a soft per-process target recomputed on a timer. Slots are not divided among lanes. Each lane's budget is static, and the reads in flight are bounded by the reader gate, not by a slot count. Problem 3 is **still open**: `read_gather` grows past the budget with no check (`pipeline.rs:667, 697`), and the adaptive plan does not address it.

## 4. XMP packets: wide encodings are decoded as UTF-8, and a GIF packet needs its wrapper (`crates/metadata/src/xmp.rs:62`, `crates/metadata/src/container.rs:210`)

**Where it stands (`b621641e`; no later commit touches these files).** `xmp_properties` (`crates/metadata/src/xmp.rs:62`) decodes every packet with `String::from_utf8_lossy`. Nothing in `crates/metadata/src` or `crates/container/src` detects a BOM or a wide encoding. The GIF path finds a packet's end by searching for the ASCII `<?xpacket end=` in two places: `gif_xmp` (`crates/metadata/src/container.rs:210`) and the block walker's `block()` (`crates/container/src/lib.rs:983`). The commits since `9d167729` changed `xmp.rs` and `tiff.rs`, but not these lines.

**Reproduced behaviour.** Synthetic packets built from the rating packet, read through the test helper `meta_of` in temporary tests that were reverted:

| Container and encoding                           | Rating    | Frames (animated GIF, 3 frames)                     |
|--------------------------------------------------|-----------|-----------------------------------------------------|
| PNG iTXt, UTF-8 (control)                        | `Some(3)` | n/a                                                 |
| PNG iTXt, UTF-16LE with BOM                      | `None`    | n/a                                                 |
| PNG iTXt, UTF-16BE with BOM                      | `None`    | n/a                                                 |
| PNG iTXt, UTF-32LE with BOM                      | `None`    | n/a                                                 |
| PNG iTXt, UTF-16LE without BOM                   | `None`    | n/a                                                 |
| GIF XMP application extension, UTF-16LE with BOM | `None`    | **1** (a wide packet is refused; see the decisions) |
| GIF XMP application extension, UTF-8 (control)   | `Some(3)` | 3                                                   |

A wrapper-less animated UTF-8 GIF reads `rating=None, frames=1`, and the same packet with its wrapper reads `Some(3)` and `3`. A UTF-8 packet with a leading BOM reads `Some(3)`, whether the BOM bytes come before `<?xpacket` or U+FEFF is in the `begin` attribute. The GIF failure has this cause: `gif_xmp` returns `None` when the marker is missing; `block()` returns `None` through `?` at `crates/container/src/lib.rs:983`; `next()` then sets `walk = Broken`, so `complete()` is false, the frame count stays 1, and the rating is dropped.

**Sources.** Adobe XMP Part 1 (April 2012) from [Adobe's repository](https://raw.githubusercontent.com/adobe/XMP-Toolkit-SDK/main/docs/XMPSpecificationPart1.pdf). Adobe lists it as ISO 16684-1:2012; its PDF footer reads 2011(E). Adobe XMP Part 3 (January 2020) from [Adobe's repository](https://raw.githubusercontent.com/adobe/XMP-Toolkit-SDK/main/docs/XMPSpecificationPart3.pdf). XML 1.0 (Fifth Edition) from [w3.org/TR/xml](https://www.w3.org/TR/xml/). A third-party copy of Part 3 is byte-identical to Adobe's file. ExifTool 13.64 (the local clone at `D:\GitHub\exiftool`) is the behaviour reference. Its results below were checked on synthetic files built for this item.

- Part 1 §7.1: the RDF serialization "shall use Unicode text"; the choice of UTF-8, UTF-16 or UTF-32 "is beyond the scope of this document" and other embedding standards may specify it. An XMP reader "shall recognize and honour a leading Unicode U+FEFF character as a byte-order marker". A UTF-16 or UTF-32 writer "should include" one.
- Part 3 requires UTF-8 for the packet in these embeddings: GIF §1.1.2 ("must be encoded as UTF-8"); JPEG, including the XMP UUID box of JPEG 2000 (the tables say "Must be encoded as UTF-8"); PNG; TIFF; Photoshop (its XMP resource, "must be encoded as UTF-8"); and PDF and PostScript/EPS ("must be encoded as UTF-8"). AVI/WAV, ID3 and MP4 are described as UTF-8. So a UTF-16 or UTF-32 packet in these is non-conforming. The claim does not extend to other embeddings: Part 3's XML section says "It is recommended that the file be encoded as Unicode using UTF-8 or UTF-16" (extracted text, line 2166).
- Part 1 §7.3.2: the `<?xpacket` wrapper "may be placed" around the packet, and file formats "can forbid" it. Its NOTE 2 says the wrapper "has no meaning once the XMP has been located" and that the recommended practice is "to use format knowledge and locate the XMP by other means whenever possible".
- Part 3 §1.1.2 (GIF): after the packet comes a 258-byte magic trailer that starts `0x01, 0xFF, 0xFE, 0xFD`.
- Part 3 §1.1.2 (GIF) gives the reason for the UTF-8 rule: "When XMP is encoded as UTF-8, there are no zero bytes in the XMP packet. Therefore, software that is unaware of XMP views packet data bytes as sub-block lengths". A packet with a zero byte breaks that framing.
- Not in the Adobe text: any rule for a BOM-less UTF-16 or UTF-32 packet. Appendix F of XML 1.0 (non-normative) gives byte patterns for detecting such encodings. ExifTool's own detection is at `XMP.pm` 4599–4649.
- XML 1.0 §4.3.3: a UTF-16 entity "must" begin with the BOM. It is a fatal error for an entity to contain byte sequences that are not legal in its encoding, and for an entity with neither a BOM nor an encoding declaration to use any encoding other than UTF-8. The Terminology section says that after a fatal error the processor "must not continue normal processing".

**Findings.**

**4a. Wide-encoded packets lose their rating and dates.** The input is non-conforming. The rating and dates are lost unless the file also has an EXIF value to fall back on. The rating order is `microsoft.or(exif_percent).or(xmp_rating).or(exif_stars)` (`crates/metadata/src/lib.rs:89`), so a file whose only rating is XMP stars loses it.

**4b. A conforming wrapper-less GIF loses its rating and frame count, and rotate and crop refuse it.** Part 1 makes the wrapper optional, so a wrapper-less GIF with UTF-8 XMP is conforming, and the path above misreads it. An animated GIF also loses its frame count; a still one loses only its rating. The walker is shared with the lossless rewriter: `crates/codec/src/rewrite/gif.rs:57` refuses an incomplete walk as `damaged GIF`, so rotate and crop refuse such a file today. For an animated file, `rewrite` returns `Ok(None)` (`gif.rs:45-47`, and `gif.rs:60` for a file with no image), which rotate reports as the catalog's outcome. Other consumers are `crates/edit/src/crop.rs:67`, which goes through `rewrite`, and the fuzz target `fuzz/fuzz_targets/container_chunks.rs:14-16`. The test-only uses at `crates/codec/src/rewrite/gif.rs:128,173` and `crates/edit/src/rotate.rs:186` read image offsets only, so they need review if `gif_blocks` changes.

**4c. Entity-split values are read from their first part only.** Within one element, only the first text part is read, so `3&#x20;7` reads as `3`. XML's value is `3 7`, which is not a rating. This is a defect in the same reader.

**Design.**

*Encoding (4a).* The BOM decides, as Part 1 §7.1 requires readers to honour it:

- `EF BB BF`: UTF-8. A UTF-8 packet with a BOM already reads correctly today, so this case changes nothing; the test pins it.
- `FF FE 00 00`: UTF-32LE.
- `00 00 FE FF`: UTF-32BE.
- `FF FE`: UTF-16LE.
- `FE FF`: UTF-16BE.

The 4-byte forms are checked first, so `FF FE 00 00` is not read as a UTF-16LE BOM followed by U+0000. Without a BOM, the packet is matched on the wide form of `<?xpacket begin=`, as ExifTool does (`XMP.pm` 4611–4647). The first four bytes of each wide form are the patterns in XML's Appendix F, which is non-normative: `00 3C 00 3F` UTF-16BE, `3C 00 3F 00` UTF-16LE, `00 00 00 3C` UTF-32BE, and `3C 00 00 00` UTF-32LE. Any other packet stays on the UTF-8 path. Reading BOM-less packets goes beyond XML's fatal-error rule. It is a decision, recorded below, because ExifTool reads them and `docs/plans/EXIFTOOL_PORT_PLAN.md:295` asks for that parity.

*Decoding (4a).* Decode lossily, as ExifTool does: each bad byte becomes `?`. For a UTF-16 unit the bytes are those of its UTF-8 form, so an unpaired surrogate, which is three bytes, becomes `???`. ExifTool reads an invalid UTF-8 byte in `a` + byte + `b` as `a?b`, and an unpaired UTF-16 surrogate, which is three UTF-8 bytes, as `a???b`. Each value is then parsed, so a damaged rating or date does not parse and is not read. This departs from XML's fatal-error rule (§4.3.3), which says a processor must not continue normal processing. That departure is a decision, recorded below. The decoder is written by hand, not `from_utf8_lossy`, because that substitutes U+FFFD, and ExifTool uses `?`. The UTF-8 path therefore changes its replacement from U+FFFD to `?` as well.
*Allocation (4a).* `MetaScratch` promises no allocation once its buffers have grown (`xmp.rs:27-28`). Add a `text: String` field to `MetaScratch`, and pass it to `xmp_properties`. Only a wide packet writes into it, and a refused one leaves it empty. A valid UTF-8 packet returns `Cow::Borrowed` and never touches it, so its behaviour is unchanged. The `MetaScratch` promise holds for valid UTF-8. A damaged packet is rebuilt with `?` replacements, which allocates, as the current `from_utf8_lossy` also does.

*Entity split (4c).* Join every text part and entity reference of one value before parsing. Then `3&#x20;7` yields `3 7`, which fails to parse, so no rating is read.

*GIF packet location (4b).* Walk the extension's sub-blocks from the start of the application data, keeping each length byte, as ExifTool does (`GIF.pm` 445–455, `IncludeLengthBytes`). The bytes walked, up to and including the block terminator, are the packet and the magic trailer, because the packet's bytes double as sub-block lengths (Part 3 §1.1.2). The last 258 bytes must be the trailer: they start `01 FF FE FD` and end with the block terminator `00`. The packet is everything before them. The wrapper is not needed, and the packet is now everything before the trailer, so any padding after `<?xpacket end=` is included, where the old code ended at `?>`. The walk ends at a zero length byte, not at any zero byte: a zero in the data is just data. A UTF-16 packet has zero bytes at length positions in practice, so the walk ends inside it, the trailer check fails, and the packet is absent. That matches ExifTool, which gives `Empty XMP` for a UTF-16 GIF. A truncated trailer leaves the file incomplete, as now. `gif_xmp` (`crates/metadata/src/container.rs:210`) and `block()` (`crates/container/src/lib.rs:983`) call one function in `pg_container` that returns the packet and the extension's end, so they agree by construction.

**Tests.** All synthetic, built in the test code (per the synthetic-fixture rule). Nothing is searched for on disk.

- `xmp_properties`: UTF-16LE, UTF-16BE, UTF-32LE and UTF-32BE packets, each with a BOM and each without one, read the rating. A UTF-8 packet with a BOM reads as it does now.
- Lossy behaviour, as ExifTool: an invalid UTF-8 byte in a value reads as `a?b`, and an unpaired UTF-16 surrogate reads as `a???b`.
- Entity split (4c): the element form `3&#x20;7` must read `None`. It reads `Some(3)` today, so this test must fail before the fix. The attribute form reads `None` today and must stay `None`.
- Wide GIF packet: refused by the trailer check, so `(None, 1)`, as now, and the walk stays incomplete. ExifTool gives no rating here either.
- GIF (4b): a wrapper-less animated UTF-8 GIF must read `(Some(3), 3)`. It reads `(None, 1)` today, so this test must fail before the fix. A long wrapper-less packet (about 2,000 bytes, with a description) must read `Some(4)` with its description intact, as ExifTool reads it. A wrapped one reads `(Some(3), 3)` and must not change. A truncated trailer reads `(None, 1)`, as now. A wide GIF with its wrapper reads `(None, 1)`, as now, because its packet fails the trailer check. A packet containing a zero byte reads `(None, 1)`. These sit beside `a_gif_is_read_only_whole`, and the truncation cases must still pass.
- Rewriter (`crates/codec/src/rewrite/gif.rs`): a still wrapper-less GIF with XMP is rotated and its XMP bytes are checked unchanged, where today it is refused as `damaged GIF`. An animated wrapper-less GIF rotated reports the catalog's outcome, where today it is refused. A wide GIF is still refused as `damaged GIF`, which pins the gate.

**Cross-references.** `docs/plans/EXIFTOOL_PORT_PLAN.md:295` covers the BOM and BOM-less detection. That line has no checkboxes, so when 4a lands, note there that both are done. The GIF UTF-16 row of the validation table stays `(None, 1)` after 4a, because its packet fails the trailer check.

**Status.** Open. Design only; not implemented.

**Decisions.**

- Wide GIF packets are refused, and the walk stays incomplete. ExifTool gives no rating for a UTF-16 GIF packet, so this follows it.
- Wide packets in PNG, WebP, JPEG and TIFF are read: the BOM is honoured, and BOM-less packets are read by Appendix F's patterns. ExifTool reads them too, with warnings (checked on synthetic files: PNG with and without a BOM, TIFF and WebP with a BOM, and JPEG). This goes beyond the Adobe rule that these containers use UTF-8, and beyond XML's fatal-error rule for BOM-less entities. WebP's container specification gives no encoding rule for its `XMP` chunk, so Part 1 governs there.
- Wrapper-less GIF packets are read, by the sub-block walk that ExifTool uses. This fixes 4b, and ExifTool reads these files (a rating of 3 for a short one, and 4 with its description for a long one).
- Decoding is lossy, with `?` for each bad byte, as ExifTool does. This departs from XML's fatal-error rule, and it changes the UTF-8 path's replacement from U+FFFD to `?`.
- Entity-split values are joined before parsing (4c). ExifTool reads `3&#x20;7` as the string `3 7`, and XML gives the same value, so the rating is not read.

*Closed since the first list:* the version counter entry (old item 7) is gone. No version role, `cast_signed()` or `>= 0` version gate remains in the desktop, the QML or `client/src/grid.rs`. The last commit that touched `ROLE_VERSION` is `d28e2f72`.

## 5. Windows strings are converted to `String` with lossy replacement (`crates/platform/src/dir.rs:348`, `crates/platform/src/drive.rs:267`, `crates/platform/src/drive.rs:284`)

**Current state, checked at `bde0476a`.** Three Windows calls return UTF-16 that is converted to a Rust `String` with `String::from_utf16_lossy`, which replaces each unpaired surrogate with U+FFFD:

- `dir.rs:348`: the file system name (`NTFS`, `FAT32`, ...), returned by `file_system`. Its caller, `crates/scan/src/refusal.rs:61`, refuses folders on file systems it does not want. The file system name is ASCII in every case the code sees.
- `drive.rs:284`: the volume GUID path (`\\?\Volume{...}\`), built in `volume_guid` and used to reopen the volume (`drive.rs:260`, `drive.rs:288`).
- `drive.rs:267`: the drive's root text. It is compared with `X:\` to pick the drive's label, and falls back to the GUID path (`drive.rs:268-270`). The label is stored in `Drive.label` (`drive.rs:145-160`).
**Defect.** A lossy conversion changes a value without any signal. On the volumes the code sees, the values are ASCII, so the replacement is not expected to fire. That is not measured on unusual volumes. The defect is that the code would be wrong silently if it did fire. The decisions made from these values do not change at the three sites, so the change there is to the text shown and the values compared. One comparison does change its outcome: `under()` (`crates/platform/src/dir.rs:183-186`) compares folder paths after lossy conversion, and it decides the OneDrive and Dropbox sync-folder caution (`dir.rs:173, 178`). Two folders that differ only by an unpaired surrogate and U+FFFD compare equal.

**Design.**

1. `dir.rs:348`: compare the file system name's units with the units of `NTFS`, with no conversion. A name with an unpaired surrogate is not NTFS, so the refusal is unchanged. Converting strictly would turn a readable non-NTFS name into an unreadable one at `refusal.rs:61`, so that is not the fix.
2. `drive.rs:284`: convert strictly. The GUID is ASCII by format, so an error means the volume is unreadable, not that its path was changed.
3. `drive.rs:267`: compare the exact units instead of converting. A root is a drive-letter root when it is three units: an ASCII letter, `:` and `\`. This is stricter than today's test, which accepts any three bytes ending in `:\`. The label is then the letter and `:`, as now. Any other root uses the GUID path. Nothing is converted, so nothing is lossy.
4. No change to names and paths. `crates/model/src/name.rs` and `crates/platform/src/names.rs` already keep exact UTF-16 units and WTF-8.
5. `under()` (`crates/platform/src/dir.rs:183-186`) compares the folder and the root by their exact units, folded for case and separators by the same rule as today for ASCII, so no two distinct paths compare equal.
6. Exception: `arg_bytes` (`crates/platform/src/args.rs:17-19`) converts command-line arguments lossily on Windows, on purpose, because Qt takes them that way. It stays.

**Tests.**

- `dir.rs`: the file system name with an unpaired surrogate is an error. The existing ASCII test (`dir.rs:889`) must still pass.
- `drive.rs`: move the label choice into a pure function that takes the root units and the GUID path. It is tested with `D`, `:`, `\` giving `D:`; with a high surrogate in place of `D` giving the GUID path; and with a GUID-only root giving the GUID path.
- `volume_guid`: an ASCII GUID round trip.
- `under()`: two paths that differ only by an unpaired surrogate and U+FFFD do not compare equal.
- Seam: `RootFacts::file_system` returns `String` (`dir.rs:124`), so a test cannot supply an unpaired surrogate. The trait returns the exact units instead, and the test uses them.

**Status.** Open. Design only; not implemented.

## 6. Counts, positions and overflow at the Rust↔QML boundary (`crates/desktop/src/rows.rs:31`)

**Status.** Open. Classified exhaustively at `b621641e`: 182 QML lines with `int`, 200 bridge crossings and 179 non-test conversion sites. The rules and the list gate can land now. The window that removes the gate for list positions is Phase 7 of `docs/plans/BOUNDED_MEMORY_PLAN.md`.

**Relation to the bounded-memory plan.** D29 (exact `f64` ranks and counts, line 131; counts below 2^48, inside the 2^53 limit used here) and step 7.9 (`qt_int` and `to_i32` deleted, `qt-edge-bound` reasons, line 1510) agree with rules 1 to 3. File ids are out of scope: `docs/plans/QT_SENTINEL_SWEEP.md` (Proposed design, item 1) decides decimal strings, while the code passes them as `i64` in Rust and `real` in QML (`crates/desktop/src/rows.rs:9-19`, `gallery_bridge.rs:187, 192`).

**What overflows, and when.** At the 1B-file target nothing bounded by the file count reaches 2^31 − 1 (2,147,483,647). Three inputs overflow at the target, and none of them is a count: a CLI index with no upper bound (`command_line.rs:356`), a CLI wait (`screenshot.rs:60`), one image's header width or height (`gallery/items.rs:95-100`, `providers.rs:79-80`), and a remote Yandex size (`yandex.rs:353-354, 396-397`; `yandex_bridge.rs:56-57`). Above about 2.15e9 files, catalog counts and every list position overflow. The bounded plan's tests go past 2^31 (a window scroll at `2^31+2^20+7`, `BOUNDED_MEMORY_PLAN.md` lines 1550 and 1952; round trips to 2^48 − 1, line 352), so the design has to hold there.

The list path needs 112 GiB at that scale (2^31 entries of 56 bytes, `crates/model/src/grid.rs:45`) and exhausts memory as it fills, before any clamp. The sorted path is larger: it reserves `(Value, Entry)` tuples of 88 bytes (a 32-byte `Value` and the 56-byte `Entry`), about 176 GiB at 2^31 rows. The target-reachable overflows need unusual input, and the count and list paths need 2.1e9 files.

**The overflow forms in the code** (179 non-test conversions):

- **Saturate to `i32::MAX`, silently: 72 rows.** `qt_int` (`rows.rs:31`; 47 call sites in 8 files), `to_row` (`rows.rs:26`), `index_for_qml` (`command_line.rs:439`), and 13 inline `i32::try_from(..).unwrap_or(i32::MAX)` sites (`filters.rs:31, 70`, `folders.rs:139, 168`, `gallery/items.rs:96, 100, 157`, `log_model.rs:87, 166, 167, 173`, `lane_model.rs:53`, `facet_choices.rs:117`).
- **-1 used for overflow: 17 rows.** `to_i32` (`shell.rs:63`; six callers; overflow reads as "no item"), `row_index` (`folders.rs:40-42`, used at `folders.rs:255`; overflow reads as "not in tree"), and `log_model.rs:211`. -1 is also the legitimate "none" in codes and in `to_row(None)`; only the overflow uses are defects.
- **Silent return: 4 rows, plus input-side returns.** `folders.rs:205` (a toggle does nothing), `gallery/model.rs:41` (cells go blank when a row's first item or end does not fit `i32`), and `test_hooks.rs:83, 87` (a CLI index past `i32` is dropped).
- **Not overflows: 86 rows.** Signatures, FFI declarations, literal codes, exit statuses and pass-throughs.

Qt's model API is C++ `int` (rowCount, index, begin/insert/remove/move rows). It cannot widen without the window, so list positions stay `int` behind the gate.

**Classification.** Every QML `int` (182 lines containing `int` as a word, 108 of them `property int` declarations) and every bridge crossing (200) is classed by what it counts:

- **COUNT:** a count from the catalog or a SQL scope, not from the in-RAM listing. Six QML lines: `TopBar.qml:27`, `FilterBar.qml:23, 24, 82`, `InboxPanel.qml:29, 182`. Rust: `filters_bridge.rs:37-38`, `failed_files_bridge.rs:27`, `facet_choices_state.rs:11`, `notices.rs:248`, and `words_bridge.rs:34`, which formats the counts for display.
- **LISTPOS:** a position or length in a list held in RAM, and any count derived from the listing: the gallery `count` (`gallery_bridge.rs:42`), `croppable`, the cut-borders and saved-copies counts (`gallery/edits.rs:161-188`), picked counts, folder rows, Yandex results. 79 QML lines.
- **EVENT:** an unbounded counter that is not a count of files: `GalleryGrid.qml:49` (`pendingRevision`), `Viewer.qml:105` and `MediaView.qml:25` (`playRequests`), `Main.qml:373` (`saveViewRequests`), `SaveViewDialog.qml:18` (`requests`), and the Rust property `gallery_bridge.rs:48` (`revision`). Five QML lines and one Rust property.
- **SMALL:** bounded by construction: ratings 0 to 5, quarter turns 0 to 3, levels, row heights 60 to 800, sort keys, and fixed tables. 62 QML lines.
- **CODE:** enum-like codes and sentinels. 30 QML lines.
- **INPUT:** values from outside the catalog (see above).

**Measured facts (`b621641e`).** Measured in this pass; method under Evidence.

- **QML `real`** round-trips every integer tested exactly: 2^31 − 1, 2^40 + 3, 2e11 and 2^48 − 1. (2^53 + 1 cannot be tested this way: the JS literal already rounds to 2^53.)
- **A JS number past the `int` range wraps; it does not clamp.** Through a real cxx-qt 0.10.0 invokable built from the pinned versions here, `takeInt(2^32 + 1)` received 1, and `takeInt(2^31 + 5)` received −2147483643. QML `int` properties wrap the same way: 2^31 reads as −2^31, and 2e11 as −1863462912. So `words.count(node.total)` shows a wrapped count. Qt's built-in `ListModel.remove` wraps the same way.
- **A panic in a bridge call aborts the process.** cxx wraps each `extern "Rust"` function in `prevent_unwind`. Through a real cxx-qt 0.10.0 invokable, a panic printed "panic in ffi function …, aborting." and the process exited with 0xC0000409, the Windows code for Rust's abort.
- **The list reservation succeeds here.** `Vec::with_capacity` for 2^31 entries of 56 bytes (120 GB) returned the full capacity. The failure would come as the list fills. The fill was not run, since it would page this machine.
- **Float32 rounding in the scene graph.** A red marker inside a `Flickable`'s content item, at content y = contentY + offset, is drawn at `fround(contentY + offset) − fround(contentY)`, rounded to whole pixels. That model matches all 18 red rows measured, and six further points predicted before measuring. Offset 60 is exact at 2^20, 2^24 and 2^25, and 4 px off from 2^26. Offsets 61 and 63 are 1 px off at 2^24 and 2^25, and up to 3 px off at 2^26 and above. From 2^26 the red row is 64 for every offset tested. A blue marker outside the content item was exact at every offset up to 2e11. So a position that reaches the scene graph must be viewport-relative and sit outside the moving content item. At 2e11 both positions round to the same float32 value, so the red marker is drawn at row 0. The software backend (`GraphicsInfo.api` 1) and D3D11 (api 4) gave identical rows.
- **Window size.** The probe's figures (4,140 items at 3840×2160, 11,462 at 7680×4320 on the user's aspects) depend on the row height, which the probe does not state. A synthetic port of `layout.rs`'s greedy pack (uniform 3:2, 4 px gap, ±1500 px cache) gives 3,362 items at 3840×2160 with 60 px rows, 300 at the 220 px default, and 112 at 400 px. The probe's figures are the right order for the 60 px minimum, but they are not reproduced: the user's aspects and the probe script are not in the repo. The 32-bit problem is in the total count, not in the window.

**Fix.** Each value takes the type for its kind, and every overflow form is replaced. No site picks its own convention.

1. **COUNT: `i64` in Rust, `real` in QML.** Catalog and SQL totals are exact to 2^53 as `real`. The conversion from `u64` ledger and SQL counts is checked (`i64::try_from`), with a notice on failure.
   - QML: `FilterBar.qml:23-24` (`total`, `matching`), `FilterBar.qml:82` (the chip's `count`), `TopBar.qml:27` (`filterTotal`), `InboxPanel.qml:29` (`failedTotal`) and `InboxPanel.qml:182` (`count`) become `real`. `active` stays `int`.
   - Rust: `filters_bridge.rs:37-38` and `failed_files_bridge.rs:27` become `i64`; `facet_choices_state.rs:11` and the role at `facet_choices.rs:102` become `i64`; `facet_choices.rs:28-30` and `failed_files.rs:194` stop clamping; `notices.rs:248` (the repeat count, `u64`) becomes a checked `f64` role.
   - `words.count(n: i32)` (`bridges/words_bridge.rs:34`, `words.rs:19-21`) becomes `double`. Every count display goes through it (`TopBar.qml:77`, `FilterBar.qml:193, 222, 341, 366`, `Hud.qml:58`, `GalleryGrid.qml:166`, `CutBordersDialog.qml:42`, `FolderTree.qml:74`), and an `int` parameter wraps a JS number past its range (Measured facts). This change is needed for every count to be correct.
2. **LISTPOS: `i32` in Rust and QML now, `real` with the window.** These are the positions and lengths of lists built in RAM. The gate is what makes the `i32` safe.
   - **Gate.** A list of more than `i32::MAX` items is refused with the new notice `ListTooLarge` (the steps under "Adding a kind" in `crates/model/src/notice.rs`), and nothing is shown. Four list builders can grow past the target, and each gets a gate:
     - the catalog grid in `list_grid` (`crates/catalog/src/browse/read/grid.rs:135`), before its allocation (`grid.rs:149`, `:159`), reserving `Vec::with_capacity` from the scope count (`grid.rs:139-141`), not the filtered count. The gate tests the filtered count for the same `condition(query)` only when the scope count exceeds `i32::MAX`. So a scope above `i32::MAX` with a filtered count below it passes the gate and still reserves a scope-sized list. The fix is to reserve from the filtered count and gate on it;
     - the folder tree build (`folders.rs`). Its rows are folders, not files, so the file count does not bound them;
     - the picker's folder list (`picker.rs:55`, `names_of`);
     - the Yandex results, at parse time (`yandex/src/client.rs:25`, `parse.rs`), which is capped at 5 pages (`DEFAULT_MAX_SIMILAR_PAGES`, `crates/yandex/src/client.rs:25`, used at `crates/app/src/yandex.rs:300`), so its gate guards a case that the cap makes unreachable today.
   - `apply` (`crates/desktop/src/gallery/model.rs:128-142`) is a second check: every route into the model passes it (`grid.arrive`, line 128; the held list, line 118). `swap_entries` is called from nowhere else in the desktop crate.
   - **Conversion.** After the gate every position converts with `list_index(usize) -> Result<i32, ListTooLarge>`. On `Err` a notice is posted and nothing changes. This replaces `qt_int`, `to_row`, `to_i32`, `row_index`, `index_for_qml` and `log_model.rs:211` on list positions. -1 remains only as the "none" value, never as an overflow.
   - **Small lists** (notices at most 200, the log ring at most 5,000, lane rows equal to the drive count, failed files at most 200) need no gate, but their conversions still go through `list_index`.
   - **Phase 7.** The window (§7.9 and §7.10 of the bounded plan) makes the Qt rows window-relative `int`s below 2^31, and the absolute positions `i64` in Rust and `real` in QML. The gate is then removed.
3. **SMALL: `i32`, bound named.** Each conversion names its bound in a `qt-edge-bound` comment (for example ratings at most 5, quarter turns at most 3, row height at most 800), and a test pins the bound. The saturating fallback is kept only here, because the value cannot reach it.
4. **CODE: `i32`, unchanged.** -1 means "none" and is documented as such. A named enum is a later change.
5. **EVENT: `real` in QML, `i64` in Rust.** These counters are not counts of files, and nothing bounds them. `real` is exact to 2^53, so they cannot wrap in practice; the wrap that `GalleryGrid.qml:98` would show at 2^31 is removed. `gallery_bridge.rs:48` (`revision`) has no QML reader: make it `i64` with a checked increment, or delete it if no Rust reader needs it.
6. **INPUT: validated at the source.** The CLI index parser (`command_line.rs:356`) refuses a value above `i32::MAX` with a parse error, so the `i32::try_from(..).ok()` in `test_hooks.rs:83, 87` can no longer drop one silently. The CLI wait (`screenshot.rs:60`) refuses a value whose milliseconds do not fit. One image's width or height, and a remote Yandex size, are `u32` at the source. An overflow makes the size unknown (0, the "none" value the UI already shows) and posts a notice (`items.rs:95-100`, `providers.rs:79-80`, `yandex.rs:353-354, 396-397`, `yandex_bridge.rs:56-57`).
7. **Silent returns.** `folders.rs:205` and `gallery/model.rs:41` post a notice before they return. `words.rs:16` maps negative seconds to 0 with `unwrap_or(0)`: a silent clamp, not a wrap, since `timecode` takes an `i32`.
8. **Revision: `i64`.** `gallery/model.rs:265` computes `self.revision + 1` on an `i32` (`crates/desktop/src/bridges/gallery_state.rs:23`), so the counter overflows at 2^31 revisions. It becomes `i64`.

Retired: the -1 overflow sentinel in `to_i32`, `row_index` the `log_model.rs:211`, `folders.rs:41`, `ledgers.rs:10` and `keymap.rs:127` sentinels (`command_line.rs:448` also returns -1 on overflow, but its -1 for a random mascot is a real value and stays); the saturating helpers `to_row` and `index_for_qml`; `facet_choices::count` (which `filters.rs:195` uses for the filter totals); `qt_int` itself; the inline sites; and the silent returns. Each call moves to rule 1, 2, 3 or 6.

Bridge functions must not panic (see Measured facts): a panic in a bridge call aborts the app, and `clippy::panic` is denied (`Cargo.toml:87`). "Fails loudly" means a refused value and a notice. `expect` is not used on these conversions.

The Qt model API (rowCount, index, begin/insert/remove/move) stays `int` and is bounded by the gate. The Qt sweep's selection (`docs/plans/QT_SENTINEL_SWEEP.md`) must address a window row by its absolute `i64` position, or by the file's identity. A Qt-side row handle inherits the 32-bit row limit.

**Tests.** Each check takes its value as an argument, so none needs a 2^31-item allocation.

- `list_index` accepts `i32::MAX` and refuses `i32::MAX + 1`, and the refusal posts `ListTooLarge`.
- The `list_grid` reservation, as a pure function of the filtered count: a scope above `i32::MAX` with a filtered count below it reserves the filtered count and is accepted, and a filtered count above `i32::MAX` is refused. No test passes a scope-sized reservation.
- The count conversion: `2^53` is exact as `real`, `2^53 + 1` is refused, and a `u64` ledger count converts with `i64::try_from`.
- `words.count` takes a `double`: a count of `2^31 + 5` formats as `2,147,483,653` (a QML test under `crates/desktop/tests/qml`).
- An EVENT counter keeps counting past `2^31 − 1` (a QML test).
- A CLI index above `i32::MAX` is refused with a message (next to the existing `index` tests, `command_line.rs`).
- A width or height above `i32::MAX`, or a remote `u32` maximum, yields "unknown" and a notice.
- Each SMALL bound has a pinned test. Existing: `command_line.rs:596`, `ledgers.rs:36`, `keymap.rs:229`.
- Phase 7: the window laws at `2^31+2^20+7` and `2^40` (`BOUNDED_MEMORY_PLAN.md` line 1550), the id round trips at line 352, and a position that reaches the scene graph stays viewport-relative (the P10 harness).

**Evidence.**

- *QML:* QtTest cases run with Qt 6.12.0's `qmltestrunner` (`C:/Qt/6.12.0/msvc2022_64/bin`). `qml.exe` printed no `console.log` output here, although its PE subsystem is console (3); the cause is not established. The `GraphicsInfo.api` values were read from the running runtime: 1 is `Software`, 4 is `Direct3D11`. The P10 case renders a `Window`, grabs it, and reads the marker rows from the saved PNG.
- *cxx:* a scratch crate built on this machine with the pinned cxx 1.0.205, cxx-qt 0.10.0 and the repo's `cxx-qt-build` fork (patched in the repo's `[patch.crates-io]`). The cxx-qt generator output was read in `cxx-qt-gen` 0.10.0 (crates.io).
- *Window size:* a throwaway port of `layout.rs`'s greedy pack.
- *Classification:* every QML `int`, bridge crossing and conversion site was read line by line by a separate reviewer, from the code, under the rules above. Their tables are in the scratchpad (`scratchpad/inv/out_A.md` to `out_E.md`), not in the repo. Three verdicts changed on review: the gallery `count` and the cut-borders count are LISTPOS (they measure a list), not COUNT; the EVENT counters are not SMALL; and the file-count premise does not overflow at 1B.
- *Adversarial review:* a fresh reviewer tried to refute the earlier design from its text alone. Its valid findings are in this section: the gate is not in the code; the list is reserved before it is checked; `words.count` is a shared sink.

**Open.**

- The notice unread count (`notices.rs:391`) is COUNT if the marks it counts span the catalog, or SMALL if they are the 200 listed entries. The mark source (`app/src/inbox.rs:1069`) is not traced to its query.
- The decoders' dimension caps are not traced. Image sizes are treated as INPUT until they are.
- The folder-tree and picker gates are not implemented. Only the catalog grid's gate has a measured design (its allocation).
- `ListTooLarge` needs its notice steps (`crates/model/src/notice.rs`, "Adding a kind"). The argument wording lives in `crates/wording/src/notice.rs:63`; the doc comment in `notice.rs` names `client/src/text/notice.rs`, which does not exist.
- The user's-aspect window figures and the 8K figure need the library and the probe script, which are not in the repo.
- Whether the list fill exhausts memory on the user's machine. The reservation was measured here; the fill was not.

**Decisions.**

- *Decided:* the list gate goes at all four list builders: the catalog grid (`list_grid`), the folder tree, the picker's folder list, and the Yandex results.
- *Decided:* EVENT counters become `real` in QML, so they cannot wrap. Image and remote sizes become "unknown" with a notice on overflow, instead of saturating. This is decided only once the decoders' dimension caps are traced (see Open).
- *Open:* file ids. D29 and `QT_SENTINEL_SWEEP.md` both already decide decimal strings, so the open question is the code: check its file-id type against that decision.
