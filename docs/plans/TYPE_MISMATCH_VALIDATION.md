# Type mismatch findings: open items

Five findings from the type-mismatch review are still open. The findings were first written at `489f3d31`. Item 1 was adversarially validated at `0e6e1fd9`, and its corrections are applied. Item 2 was checked at HEAD (`fd5e727a`), and its design is below. Item 4 was re-validated at `b621641e`, and item 5 at `b621641e`. The Qt sentinel sweep is no longer in item 1; it is its own plan, `docs/plans/QT_SENTINEL_SWEEP.md`. Items 1, 2, 4 and 5 are rated low, except 4b, which is medium. Item 5 is not reached at the 1B-file target, and it must be fixed before Phase 7 of the bounded-memory plan ships. Items 1 and 5 overlap steps of Phase 7 in `docs/plans/BOUNDED_MEMORY_PLAN.md`. Each overlap is noted under its item. The design is not finalized. Each entry gives the current location, the defect, its impact, and the planned fix. Fixed and stale findings are not listed. Item 1 is open: its row-id defect is fixed, and its entry lists the defects that remain and their planned fixes. The Exif IFD pointer and Rating findings are complete and have been removed from this file.

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

1. **The stamp is identity and creation time.** `FileStamp` (`crates/model/src/ids.rs:55`) drops size and mtime and keeps `identity` and `created`. The catalog sets `created` from the listing and fills it only when it is NULL (`crates/catalog/src/writers.rs:181`, `coalesce(created, …)`, backfill at `:306-307`). It changes only when a row takes a new identity (`take_identity`, `crates/catalog/src/pair.rs:475-500`), where it moves with the identity. Two forms exist. A rebind (`REBIND`, `coalesce(?4, created)`) keeps the row's old value when the new file has no creation time, so the stamp then carries a creation time that is not the new file's. A replacement (`REPLACE`, `?4`, `crates/catalog/src/pair.rs:500`) takes the new file's value, NULL included, so the row is not remembered until a scan backfills it. Both cases need a file with no creation time. Identity can be reused after a delete, but a new file gets a new creation time, so the creation time refuses a reused identity. A file whose platform gives no creation time (`created` NULL, `crates/platform/src/dir/listing.rs:27`) is not remembered. There is no fallback to size and mtime, because that brings back defect 1. `find_by_stamp` (`crates/catalog/src/lookup.rs:122`) matches the identity and `created` on a row that is not `missing`, and `stamp_of` (`lookup.rs:103`) returns no stamp for a NULL `created`. The catalog displaces a row whose identity matches but whose creation times differ (`crates/catalog/src/writers.rs:300-305`), so the creation time already decides identity there. Its pairing does not use that rule alone: the tiers that pair a file to a row on another volume or in another folder require size and mtime, with `created` only as an optional check (`crates/catalog/src/pair.rs:115`, `:165`). The stamp does not follow a move across volumes, because the identity includes the volume. A NULL `created` is backfilled by the next scan that reads the file (`writers.rs:306-307`), so such a file becomes rememberable then, not never.
   - *Documentation to change with the stamp:* `crates/model/src/ids.rs` says size and mtime are how the scanner tells a row is a different file (lines 30-31 and 52-53). That text moves to creation time.
   - *Known trade-off:* the stamp does not refuse a file that another program rewrote in place. Measured on a synthetic file: a copy over an existing file kept its identity and creation time. The catalog treats that file as the same one too, and the old size-and-mtime stamp refused it. The restore therefore keeps the selection on a file whose content changed under the same name.
   - *Volume types:* whether a FAT or exFAT file's identity survives a rename is unconfirmed. A search summary said that a FAT identity follows the directory entry and can change on a rename to a longer name. Microsoft's [`FILE_ID_INFO` page](https://learn.microsoft.com/en-us/windows/desktop/api/winbase/ns-winbase-file_id_info) does not say that (it was fetched, and says only that the identifier and volume serial "uniquely identify a file on a single computer"). If FAT identities do change on rename, the stamp would fail to restore after such a rename, and nothing else would change. exFAT is not known. No FAT or exFAT volume was measured. The other non-system drives were not used: `Z:` is a Google Drive virtual drive, which was deliberately not written to (writes sync to the cloud), and the network share `Y:` is NTFS and was not written to.
2. **The start takes the resolved file.** `start_state` takes `last_file: Option<FileId>` as a parameter, and the caller resolves the stamp in `Core::new` (`crates/desktop/src/controller.rs:59-65`). The 11 test calls in `controller.rs` (lines 233 to 320) and the one in `crates/desktop/src/shell.rs:752` pass the argument.
3. **A catalog error is reported.** A new notice `LastFileNotKept { error }` (key `last-file-not-kept`, `Level::Error`, `Source::Catalog`) follows the steps for adding a kind in `crates/model/src/notice.rs:50-59`:
   - the variant and its `ids!` string (`notice.rs:800`, `:1168`), and the `level` (`:1285`), `source` (`:1313`) and `lifecycle` arms;
   - its sample in `Notice::samples()` (`:1617`), its line in `notice/golden.jsonl`, and its `ordinal` arm. The ordinal is 51: `ChangesUnseen` is 50 (`:1702`), so `KINDS` (`= 51`) becomes 52;
   - its row in `notice/resolver_table.golden`, its words in `crates/wording/locales/en/text.ftl` (`msg-notice-last-file-not-kept-title` and `-body`), and its `error` argument in `crates/wording/src/notice.rs` (the arm at `:95`, as `SessionNotRead` has). The `notice.rs` doc comment names `client/src/text/notice.rs` for the args; that file does not exist, and the doc is stale.
   - Its resolver is `Resolver::NextSuccess(Op::LastFile)`. `Op` (`notice.rs:931-952`) has no such variant, so one is added. A save reports success through `app::observe(Op::LastFile, …)` (`crates/desktop/src/app.rs:29`), which the prefs save does not do today. A successful save of no file clears the notice too, and so does a start whose lookup finds no row: `Ok(None)` is a success, and only a failed lookup is an error.
   - The notice is posted where the outcome is, in the desktop save and start. A notice raised at the flush is not shown: the inbox closes in `App::shutdown` (`crates/app/src/app.rs:210-211`), after the flush. It is posted and, at most, logged. Whether it reaches the log file was not checked.
4. **A save whose lookup fails writes nothing.** `app::observe(Op::LastFile, catalog.stamp_of(file), …)` returns `None` on failure (`crates/desktop/src/app.rs:29`, failure arm at `:35-38`), and the save skips the `save_prefs` call on `None`. This is what stops defect 2 from erasing the stored stamp.
5. **The save also runs at flush, at most once.** A flush pushes `Command::SaveLastFile` when the start is over, unless `fire` already pushed one because a `SaveSession` was armed (`crates/client/src/controller.rs:606-620`, `:629-631`). This covers defect 3, which arms nothing. `fire`'s `is_started` guard (`controller.rs:629`) keeps a start's options unsaved. The stamp is read on the window thread before the window's writes stop (`crates/desktop/src/lib.rs:187`), so the catalog is still open. When nothing is shown at quit, the flush writes `None`: nothing shown means nothing remembered. That is intended, and it is the one way a successful save clears the stored file on purpose. A list that is still loading at quit does not change what is shown (`open_listing` only sets the loading state), so the save keeps the file that was shown before the load.
   - *Open decision:* a start whose lookup failed shows nothing, and a later successful save of nothing writes `None`, which erases the stored stamp. Keeping the old stamp until a file is shown would avoid that, but then a file the user really closed on would also be kept. Not decided here.

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
- a new file with no creation time rebinds a row and keeps the row's old creation time, and replaces a row and leaves its creation time NULL (`take_identity`, `crates/catalog/src/pair.rs:498-500`);
- a lookup that finds no row is a success for the notice's resolver, and only a failed lookup posts `LastFileNotKept`;
- a failed start reports `LastFileNotKept`, and the notice's own checks pass: the sample, its golden line, the `ordinal`, the resolver table and the words (`crates/model/src/notice.rs`, `crates/wording`).

**Out of scope.** The Qt sentinel sweep (`docs/plans/QT_SENTINEL_SWEEP.md`); counts and positions (item 5); and the overlap with `docs/plans/BOUNDED_MEMORY_PLAN.md` step 7.9 (line 1508), which names a `save_last_file(Option)` that this item does not add.

**Not verified.** The NTFS behaviour is measured on synthetic files on the local NTFS volumes, not on the library. FAT, exFAT and network identity: see the volume-types trade-off above, which is unconfirmed for FAT and exFAT. A notice raised at the flush is logged: `inbox.post` writes it to the log before anything else (`crates/app/src/inbox.rs:433-434`), and the log is an appended line (`crates/log/src/file.rs:60`). The viewer's and the grid's paths to the save were traced from the code and not run.

## 2. CLI buffer and parallelism flags clamp or saturate silently (`crates/cli/src/args.rs:54-58`)

**Current state, checked at HEAD.** `--buffer-mb` is an `Option<u64>` (`args.rs:45`). `options()` computes `self.buffer_mb.map_or(budget, |mb| mb.max(1).saturating_mul(1 << 20))` (`args.rs:58`). The wrap that SEC-043 reported (`docs/plans/SECURITY_FLOW_REVIEW.md:753`, CONFIRMED: `mb.max(1) << 20` wrapped to 0 for multiples of 2^44) is gone, because the multiply saturates. A saturated value is `u64::MAX`, which `Budget::new` keeps (`crates/jobs/src/governor.rs:76`), and the pool reads as `usize::MAX` (`crates/jobs/src/pipeline.rs:813`). Three more flags clamp silently the same way: `--readers`, `--workers` and `--slots` take `.max(1)` (`args.rs:55, 56, 59`). So `--buffer-mb 0` becomes 1 MB, and `--workers 0` becomes one worker.

**Impact, corrected by validation.**

- No hang. A file larger than the budget is read alone: the production doc says so (`crates/jobs/src/pipeline.rs:800-801`), and the rule is `st.held == smallest` at `pipeline.rs:836`. A small budget only serializes. (The earlier citation, `:924`, is a comment in the test module that starts at `:896`.)
- A huge budget removes the target the flag sets, since the budget is soft; the pass then holds what its slots and files need. The pool never holds more buffers than there are slots (`pipeline.rs:803`), where slots are `readers + 2 × workers` (`pipeline.rs:81`), and each buffer grows to the largest file it reads. With the default 8 readers and 16 workers that is 40 slots, against a default budget of 24 × 12 MB = 288 MB. Unbounded, 40 slots of 200 MB files could hold about 8 GB. The earlier "no data or hang impact" is correct only for data and hangs.
- The user's value is silently replaced: 0 becomes 1, and an overflow becomes unlimited.
- The lanes cap their buffers at a quarter of physical memory (`crates/library/src/coordinator.rs:178`, `MEMORY_SHARE` at `:53`). The CLI pass applies no cap, and nothing refuses a value above physical memory.

**Shared design (both options).**

1. **The type stays pure; the platform is read at the CLI edge.** `BufferBytes` lives in `crates/model`, beside `ThumbCap` (`crates/model/src/cache.rs`). Its constructor is `from_mb(mb: NonZeroU64) -> Option<BufferBytes>`. It refuses only a byte count past `u64` (a checked multiply). It knows nothing about memory: the budget is a soft limit (fix 5), so no physical size enters the model, and the model's tests need no platform.
2. **Clap parses it, through one parser per flag.** `#[arg(long, value_parser = buffer_mb)]` on an `Option<BufferBytes>` field, next to `thumb_height` and `cap_gib` (`args.rs:131`, `:135`). `buffer_mb` calls `from_mb`, and reads the platform only to decide whether to warn (fix 5). `options()` becomes `self.buffer.map_or(budget, BufferBytes::bytes)`, with no arithmetic and no `max(1)`.
3. **Readers, workers and slots use non-zero types, decided.** The CLI fields become `NonZeroUsize`, so `options()` loses its three `.max(1)` (option 1 uses the same types in `pipeline::Options`). The defaults need a `NonZeroUsize` constant, since `pipeline::READERS` is a plain `usize` (`crates/jobs/src/pipeline.rs:56`); `default_value_t` takes that constant. `pipeline::Options`'s fields stay `usize`, so `options()` passes `.get()`. clap 4.6.7 does not parse `NonZero` types itself (checked in `clap_builder`'s source: no `NonZero` impl), so each flag gets a small parser that refuses zero, in the style of `thumb_height`. `--readers 0`, `--workers 0` and `--slots 0` are errors from now on, which changes behaviour for anyone who passes zero today. The default is no flag at all, and the error text says so, because `0` is a common way to mean "automatic".
4. **Refusals say why, and none is silent.** Zero: "--buffer-mb takes at least 1 MB; leave it out for the default". Overflow: "{text} MB is too large to count in bytes". Not a number: "{text} isn't a number of megabytes". Above physical memory is not a refusal; it is a warning (fix 5). A zero no longer means 1 MB.
5. **Decided: the budget is a soft limit, not a hard cap.** The flag is a target the pass aims for. No value is refused for being large. A value above physical memory is accepted, and the CLI warns through its own `stderr_line` (`crates/cli/src/out.rs:15`), the channel progress already uses. The pass may exceed the budget: one file larger than the budget is read alone, and slots still hold whole buffers (`crates/jobs/src/pipeline.rs:836`). That is today's behaviour and is kept. The lanes' quarter (`coordinator.rs:178`) stays their default and is not applied to the CLI. If physical memory cannot be read (`physical_bytes()` returns `None`), no warning is given.
6. **Help text states the range (confirmed).** The current doc comment at `crates/cli/src/args.rs:43` says the flag is "the megabytes of file buffers the pass holds at most", which the soft limit contradicts, so it changes. The confirmed text: `--buffer-mb`: "Megabytes of file buffers the pass aims to hold (at least 1; a target, not a hard limit). Default: 12 per reader and per worker."; `--readers`, `--workers` and `--slots`: "at least 1" added to their current text. The help is part of the flag's contract, so it changes with the type.
7. **SEC-043 is closed by this change.** The entry at `docs/plans/SECURITY_FLOW_REVIEW.md:753` gets its status updated when the change lands; the wrap was already fixed by the saturating multiply, and the typed parser removes the saturation as well. Its trigger is out of date: it names `gallery borders --buffer-mb`, but the flag is on `import`, `hash`, `pair` and `thumbs` (`crates/cli/src/args.rs:92, 115, 121, 137`); `borders` takes no pipeline flags. The entry should name those four commands.

**Option 1: typed limits in the library (accepted).** The rules move into the type the pipeline takes, so no caller can build an invalid pool.

- `BufferBytes` (shared item 1) is used by `pg-jobs`, which already depends on `pg-model` (`crates/jobs/Cargo.toml`). `pipeline::Options` takes `readers`, `workers` and `slots` as `NonZeroUsize` and the buffer as `BufferBytes`. The budget is built from it.
- `Pool::with_budget` takes `NonZeroUsize` slots, so the `slots.max(1)` at `crates/jobs/src/pipeline.rs:793` goes.
- The CLI parses into those types (shared items 2 and 3), and `options()` only copies them.
- Scope: every construction of `pipeline::Options` changes. Known sites are the pipeline's test module (`buffer_bytes: 1 << 20` from `pipeline.rs:1039` on) and the library and bench crates. A full count is needed before implementation.
- Behaviour changes only by the shared refusals already decided. The budget's meaning is unchanged.
- Tests: zero cannot be built, so no test is needed for it; the pool tests keep their current assertions; the shared CLI tests cover the parsing.

**Option 2: a strict memory bound (not chosen).** The soft-limit decision (fix 5) rules out a hard ceiling, which is what a strict bound would be. The policies below are kept for reference, since they are the only way to make the bound strict.

- Today's exception (`crates/jobs/src/pipeline.rs:836`, `st.held == smallest`, documented at `:800-801`) lets one such file through alone. Strict enforcement must choose:
  - (a) keep the exception and state the bound as the budget plus the largest file. No behaviour change, but a weaker bound.
  - (b) refuse files larger than the budget, and report each one as a notice. The pass skips them, so results change for those files.
  - (c) read such files in pieces. `process` takes the whole file's bytes (`pipeline.rs:96`), so this needs a codec change, and it is the largest option.
- The default budget (12 MB per slot, `pipeline.rs:53`) decides how often (a) or (b) matters. The plan has no measurement of library file sizes, so measure before choosing a default.
- The pool tests at `pipeline.rs:1460-1481` assert the exception ("a budget of one byte still grants a file larger than it, alone"). They change under (b) and (c).
- Tests: under (b) and (c), the held total is at most the budget. Under (a), it is at most the budget plus the largest file. Under (b), each skipped file is reported.
- Decided: option 1, with the soft limit (fix 5). Option (a) is confirmed: a file larger than the budget is read alone, as today. (b) and (c) would make the limit hard, and are not planned.

**The lanes' memory policy (assessed, from the code; not run; not part of item 2).** The lanes never read the CLI's pipeline flags: `PipelineArgs` is referenced only in `crates/cli`, and the lanes' options come from `pipeline::Options::new` (`crates/library/src/inner.rs:65`). Its findings are item 3 of this file. Decided: each process keeps its own pool and cap, and sharing a budget across processes is out of scope. The library's lanes use a governor, not the CLI's pool budget. Each lane gets one budget: the knee, `(readers + cpus) × 12 MB` (`crates/jobs/src/governor.rs:92`), or the cap split evenly among the lanes when the cap binds (`governor.rs` `split`). The cap is a quarter of physical memory per process (`crates/library/src/coordinator.rs:178`), and the governor rebalances when a lane retires (`coordinator.rs:555`). Each lane's pool is soft, like the CLI's: one file larger than the budget is read alone (`pipeline.rs:836`).

- **The cap almost never binds.** With 16 CPUs the knee is 288 MB per lane, so two drives need 576 MB. The cap binds only on a machine with under about 2.3 GB of RAM, or with about 28 lanes on a 32 GB machine.
- **Slots are not governed.** Each lane's options take the default slot count, `readers + 2 × cpus` (`pipeline.rs:70`, inherited through `inner.rs:65`), so total slots grow with the lane count while the budgets are split. Slots hold whole buffers, which grow to the largest file each one reads.
- **Nothing is shared across processes.** The CLI pass runs its own pool with no governor, and the CLI notes that another process can scan at the same time (`crates/cli/src/args.rs:70`). The app's lanes and a CLI pass can each use their full budget at once.
- **Assessment.** The per-lane knee is a reasonable measured default, and the soft semantics are consistent. The cap is coherent in principle, but it is a rule that rarely fires, and it doesn't cover slots. The most proper version would count slots against the same budget (divide each lane's slots by the lane count, or count total slots against the cap), keep the cap as a documented per-process target, and state that sharing across processes is out of scope.

**Tests (shared).** The model tests (no platform needed): `from_mb` refuses zero; `1` gives 2^20 bytes; `17592186044416` (SEC-043's value, 2^64 bytes) is refused as overflow; `u64::MAX` is refused; a count above physical memory is accepted, because the limit is soft. The CLI tests use the existing `Cli::try_parse_from` pattern (`crates/cli/src/lib.rs:860`): `--buffer-mb 0`, `--workers 0`, `--readers 0` and `--slots 0` are refused with their messages; `--buffer-mb 1` gives exactly 2^20 bytes in `options()`; surrounding spaces are trimmed; the default with no flags is unchanged. The above-physical-memory warning is tested through an inner function that takes the physical size as an argument, so no platform is needed; with no size, no warning is given.

*Closed since the first list:* the thumbnail height entry is fixed. `ThumbHeight` (`crates/model/src/size.rs:82`) holds the 16 to 2048 range (`THUMB_HEIGHT`, `size.rs:12`), and the CLI parses it with a clap `value_parser` (`crates/cli/src/args.rs:131`, parser at `args.rs:190`). `pg_thumbs::run` and `ImageRequest::thumb` take it. The plan's `ThumbHeight::clamp(u32)` was not built. The UI clamp in `client/src/image.rs` is gone, and `ThumbHeight::for_row(RowHeight, Scale)` (`size.rs`) does that job instead.

## 3. The lanes' slots and memory cap are not bounded by the budget (`crates/library/src/inner.rs:65`, `crates/library/src/coordinator.rs:178`)

**Current state, checked in the code.** One lane runs per physical drive (`crates/library/src/coordinator.rs:563`, `Inner::new` with `governor.lane()`). Each lane's pool gets a budget from the governor, the knee `(readers + cpus) × 12 MB` (`crates/jobs/src/governor.rs:92`, `pipeline.rs:53`), or the cap split evenly among the lanes when the cap binds. Each lane's pool also takes the default slot count, `readers + 2 × cpus` (`crates/jobs/src/pipeline.rs:70`, inherited through `inner.rs:65`), whatever the number of lanes. The cap is a quarter of physical memory per process (`coordinator.rs:178`, `MEMORY_SHARE` at `coordinator.rs:53`).

**Problems.**

1. Slots are not divided among lanes. The total grows with the number of drives, and each slot's buffer can grow to the largest file its lane reads, while the budgets are split.
2. The cap almost never binds. With 16 CPUs the knee is 288 MB, so two drives need 576 MB. The cap binds only on a machine with under about 2.3 GB of RAM (two lanes), or with about 28 lanes on 32 GB.

**The knee.** The knee is the budget one lane needs to run at the measured speed. It is `(readers + workers) × 12 MB`: a buffer for each read in flight and for each worker. The measurement in `pipeline.rs:84-87`: on 24 MP JPEGs on an HDD, with 8 readers and 12 workers, 128 MB at the default slots was 4% slower than the default, and 512 MB was no faster than 240 MB.

**Options for the cap.**

- Keep it: a documented process-wide target that rarely binds.
- Lower it: a fraction that binds on realistic systems. A smaller budget costs little speed, given the measurement above.
- Remove it: memory is then the sum of the lanes' knees, plus files larger than the budget and the slots, with no process-wide limit. Example: six drives at 288 MB each is about 1.7 GB, and the cap would have no effect.

**Options for slots.**

- Divide each lane's slots by the number of lanes, so readers and workers shrink as drives are added.
- Count the total slots against the cap, so the total is bounded.

**Status.** Open. The cap and the slots are decided together.

## 4. XMP packets: wide encodings are decoded as UTF-8, and a GIF packet needs its wrapper (`crates/metadata/src/xmp.rs:62`, `crates/metadata/src/container.rs:210`)

**Where it stands (checked at `b621641e`).** `xmp_properties` (`crates/metadata/src/xmp.rs:62`) decodes every packet with `String::from_utf8_lossy`. Nothing in `crates/metadata/src` or `crates/container/src` detects a BOM or a wide encoding. The GIF path finds a packet's end by searching for the ASCII `<?xpacket end=` in two places: `gif_xmp` (`crates/metadata/src/container.rs:210`) and the block walker's `block()` (`crates/container/src/lib.rs:983`). The commits since `9d167729` changed `xmp.rs` and `tiff.rs`, but not these lines.

**Validation.** Run with a throwaway synthetic test, since reverted; nothing from it remains in the tree. The packets were built from the rating packet and read through the public `read` path:

| Container and encoding                           | Rating    | Frames (animated GIF, 3 frames) |
|--------------------------------------------------|-----------|---------------------------------|
| PNG iTXt, UTF-8 (control)                        | `Some(3)` | n/a                             |
| PNG iTXt, UTF-16LE with BOM                      | `None`    | n/a                             |
| PNG iTXt, UTF-16BE with BOM                      | `None`    | n/a                             |
| PNG iTXt, UTF-32LE with BOM                      | `None`    | n/a                             |
| PNG iTXt, UTF-16LE without BOM                   | `None`    | n/a                             |
| GIF XMP application extension, UTF-16LE with BOM | `None`    | **1** (expected 3)              |
| GIF XMP application extension, UTF-8 (control)   | `Some(3)` | 3                               |

The wrapper-less GIF case (4b below) was not probed; it rests on code reading. `gif_xmp` returns `None` when the marker is missing, and `block()` returns `None` through `?` at `crates/container/src/lib.rs:983`. `next()` then sets `walk = Broken`, so `complete()` is false, the frame count stays 1, and the rating is dropped.

**Spec.** The text below was extracted from two Adobe PDFs: [XMP Part 1](https://raw.githubusercontent.com/adobe/XMP-Toolkit-SDK/main/docs/XMPSpecificationPart1.pdf) (April 2012, ISO 16684-1:2011) and [XMP Part 3](https://dl.photoprism.app/pdf/specifications/20120101-Adobe_XMP_Specification_Part_3.pdf). The Part 3 text is the January 2020 edition; the file name says 2012.

- Part 1 §7.1: the RDF serialization "shall use Unicode text"; the choice of UTF-8, UTF-16 or UTF-32 "is beyond the scope of this document" and other embedding standards may specify it. An XMP reader "shall recognize and honour a leading Unicode U+FEFF character as a byte-order marker". A UTF-16 or UTF-32 writer "should include" one.
- Part 3 requires UTF-8 for the packet in each embedding it checked. GIF §1.1.2: "must be encoded as UTF-8". JPEG tables: "Must be encoded as UTF-8". PNG and TIFF: UTF-8. So a UTF-16 or UTF-32 packet is non-conforming wherever it sits.
- Part 1 §7.3.2: the `<?xpacket` wrapper "may be placed" around the packet, and file formats "can forbid" it. Its NOTE 2 says the wrapper "has no meaning once the XMP has been located" and that the recommended practice is "to use format knowledge and locate the XMP by other means whenever possible".
- Part 3 §1.1.2 (GIF): after the packet comes a 258-byte magic trailer that starts `0x01, 0xFF, 0xFE, 0xFD`.
- Part 3 §2.1.2: "XMP-aware software should be smarter and more robust than presumably older XMP-unaware software".
- Not found in the Adobe text: any rule for detecting a BOM-less UTF-16 or UTF-32 packet. ExifTool's pattern detection (`XMP.pm` 4593–4645) is an implementation leniency, not a spec rule.

**Findings.**

**4a. Wide-encoded packets lose their rating and dates.** Validated above. The input is non-conforming, and the EXIF fallback covers the rating and dates. Priority: low.

**4b. A conforming GIF without a wrapper loses its rating and frame count.** Part 1 makes the wrapper optional, so a wrapper-less GIF with UTF-8 XMP is conforming, and the path above misreads it. The frame count is wrong, not only the rating. Priority: medium. This is the more important of the two, and it has nothing to do with encoding.

**Design.**

*Encoding (4a).* Decide by the BOM alone, as the Part 1 rule requires:

- `EF BB BF`: UTF-8. The BOM is dropped, and the path is otherwise unchanged.
- `FF FE 00 00`: UTF-32LE.
- `00 00 FE FF`: UTF-32BE.
- `FF FE`: UTF-16LE.
- `FE FF`: UTF-16BE.

The 4-byte forms are checked first, so `FF FE 00 00` is not read as a UTF-16LE BOM followed by U+0000. A packet with no BOM stays on the UTF-8 path.

*Decoding (4a).* Decode lossily: `char::decode_utf16` with `REPLACEMENT_CHARACTER` for UTF-16, `char::from_u32` with the same fallback for UTF-32, and a trailing partial unit dropped. This matches the UTF-8 path, which keeps the values and replaces bad bytes, and Part 3 §2.1.2 asks readers to be robust. Damaged values then fail to parse as a rating or date, so no partial value is read. The alternative, a strict decode that skips the packet on any error, is not chosen: it would make a wide packet stricter than a UTF-8 one with the same damage.

*Allocation (4a).* `MetaScratch` promises no allocation once its buffers have grown (`xmp.rs:27-28`). Add a `text: String` field reused across packets, and pass it to `xmp_properties`. The UTF-8 path keeps `Cow::Borrowed`, so its behaviour is unchanged.

*GIF packet location (4b).* Find the packet's end by the trailer's own bytes, not the wrapper: search for `01 FF FE FD` from the start of the application data, and take the packet to be the bytes before it. The trailer must lie wholly in the file (258 bytes), so a truncated GIF is still incomplete. The search is unambiguous for a conforming packet, because valid UTF-8 never contains `FE` or `FF`. The two sites, `gif_xmp` (`crates/metadata/src/container.rs:210`) and `block()` (`crates/container/src/lib.rs:983`), must agree on the end, so the search goes in one function in `pg_container` that both call. This also removes the encoding dependence: a wide packet in a GIF is read leniently, with no special case.

*Out of scope.* A BOM-less wide packet has no spec rule, so it is left to `docs/plans/EXIFTOOL_PORT_PLAN.md:295`, where ExifTool parity is the target. The PNG, WebP and JPEG paths do not search for markers, so they need no change.

**Tests.** All synthetic, built in the test code (per the synthetic-fixture rule). Nothing is searched for on disk.

- `xmp_properties`: UTF-16LE, UTF-16BE, UTF-32LE and UTF-32BE packets with a BOM each read the rating. A UTF-8 packet with a BOM reads as it does now.
- Lossy behaviour: an odd trailing byte and a lone surrogate, each pinned to its documented outcome.
- GIF: a wrapper-less animated UTF-8 GIF asserts `(Some(3), 3)`. Run it before the change and expect the 4b failure; that is the first step, because 4b is not probed yet. A UTF-16 GIF with the wrapper asserts `(Some(3), 3)`, a lenient read rather than a conforming one. Both sit beside `a_gif_is_read_only_whole`. The truncation cases must still pass.

**Cross-references.** `docs/plans/EXIFTOOL_PORT_PLAN.md:295` plans BOM-less detection. When 4a lands, tick its BOM part there and leave the BOM-less part open.

**Priority and status.** 4a is low, and 4b is medium. Open. Design only; not implemented.

*Closed since the first list:* the version counter entry (old item 7) is gone. No version role, `cast_signed()` or `>= 0` version gate remains in the desktop, the QML or `client/src/grid.rs`. The last commit that touched `ROLE_VERSION` is `d28e2f72`.

## 5. Counts and positions are 32-bit at the Rust↔QML boundary (`crates/desktop/src/rows.rs:31`)

**Status.** Open, rated low. Validated at `b621641e`. Two parts. The rules and a gate can land now. The windowed model that removes the gate belongs to Phase 7 of `docs/plans/BOUNDED_MEMORY_PLAN.md`, and this item does not build it.

**Relation to the bounded-memory plan.** Decision D29 (exact `f64` ranks and counts, line 131) and step 7.9 (`qt_int` and `to_i32` deleted, `qt-edge-bound` reasons, line 1510) agree with the rules below. They conflict on file ids: D29 says decimal strings, while the code and step 7.9 use `real` (`crates/desktop/src/rows.rs:9-19`, `gallery_bridge.rs:187, 192`). This item assumes `real`. The plan has to settle it.

**Current state (`b621641e`).**

- **Overflow conventions.** Two, and they disagree.
  - Saturate to `i32::MAX`: `qt_int` (`crates/desktop/src/rows.rs:31`), the one shared copy, used by `failed_files.rs`; `notices.rs` has no copy of its own. Also `to_row` (`rows.rs:26`), `index_for_qml` (`crates/desktop/src/command_line.rs:439`), and 13 inline `i32::try_from(..).unwrap_or(i32::MAX)` sites: `filters.rs:31, 70`, `folders.rs:139, 168`, `gallery/items.rs:96, 100, 157`, `log_model.rs:87, 166, 167, 173`, `lane_model.rs:53`, `facet_choices.rs:117`.
  - Become -1: `to_i32` (`crates/desktop/src/shell.rs:63`), which QML reads as "no item". It converts the viewer step (`shell.rs:167`) and the list indices (`shell.rs:502-585`). `docs/plans/CODEBASE_REVIEW_VALIDATION.md` RV-0206 records the disagreement.
- **Bridge properties are `i32`.** Gallery `count`, `current`, `viewing`, `anchor` (`gallery_bridge.rs:42, 56, 61, 63`); filters `total` and `matching` (`filters_bridge.rs:37-38`); failed files `total` (`failed_files_bridge.rs:27`); the facet count (`facet_choices_state.rs:11`); `list_shown(count: i32)` (`bridges/shell_bridge.rs:61`, `shell.rs:129`).
- **`rowCount` is `i32`** in eight bridges: `facet_choices_bridge.rs:63`, `failed_files_bridge.rs:50`, `folders_bridge.rs:50`, `gallery_bridge.rs:132`, `lane_model_bridge.rs:44`, `log_model_bridge.rs:53`, `notices_bridge.rs:73`, `yandex_bridge.rs:98`.
- **The list is all in RAM.** `Grid.entries` is a `Vec<Entry>` (`crates/client/src/grid.rs:118`), `Entry` is asserted at 56 bytes (`crates/model/src/grid.rs:45`), and the model has no window. The row role `row_field` returns no data for a row whose first item does not fit `i32` (`crates/desktop/src/gallery/model.rs:41`), so those cells go blank with no message.
- **Already correct, and the pattern to follow.** Folder totals cross as `i64` in a `QVariant` (`folders.rs:142`), and QML declares them `double` (`FolderTree.qml:36`). File ids cross as `real` and `i64`. The facet count saturates on purpose, because its property is an int (`facet_choices.rs:27-29`). That decision is reversed here for counts.

**Reach.** The stated target is 1B files (`BOUNDED_MEMORY_PLAN.md` §1.1, §2). 1B is below `i32::MAX` (2,147,483,647), so nothing clamps at the target.

- **Lists (items, rows, list length).** A list needs more than 2^31 − 1 items before anything clamps. Rows partition the items and each holds at least one (`crates/client/src/layout.rs:40-57`), so rows never outnumber items. The list is in RAM: 2^31 entries of 56 bytes are 112 GiB for `Entry` alone, before rows and QML. The current design does not reach that in practice.
- **Catalog-wide counts** (filter totals, facet counts, failed files, notice totals). SQL computes them with no list loaded, so they clamp once a catalog holds more than 2^31 − 1 matching files: 2.1 times the target. This is the one path that is reachable without a rewrite.
- **The bounded plan's own tests go past 2^31**: a window scroll at `2^31+2^20+7` (`BOUNDED_MEMORY_PLAN.md` lines 1550 and 1952) and round trips to 2^48 − 1 (line 352). Correct behaviour above 2^31 is therefore required by Phase 7, even though the target sits below it.

**Severity: low.** It was rated medium because it fails silently at scale. The scale is 2.1 times the target, and the list path is not reached on the current design.

**Measured facts (`b621641e`).**

Measured in this pass. Method under Verification.

- **QML `real`** returns every integer tested exactly: 2^31 − 1, 2^40 + 3, 2e11 and 2^48 − 1. (2^53 + 1 cannot be tested this way: the JS literal already rounds to 2^53.)
- **QML `int` wraps; it does not clamp.** 2^31 reads back as −2^31, 2^31 + 2^20 + 7 as −2146435065, and 2e11 as −1863462912. A count that reaches an `int` property is wrong with no signal, so every count or index property changes type on both sides.
- **A panic in a bridge call aborts the process.** cxx wraps each `extern "Rust"` function in `prevent_unwind`, and cxx-qt emits `extern "Rust"` blocks for invokables. A panic inside the guard aborts. An unguarded panic is caught.
- **Float32 rounding in the scene graph.** A red marker inside a `Flickable`'s content item, at absolute y = contentY + offset, lands off its row by up to half the float32 spacing at that y. The software backend (`GraphicsInfo.api` 1) and D3D11 (api 4) gave identical rows. Offset 60: exact at contentY 2^20, 2^24 and 2^25, and 4 px off at 2^26 and 2^28. Offsets 61 and 63: 1 px off at 2^24 and 2^25, and up to 3 px off at 2^26 and above. At 2^28 the rows fall on a 32 px grid, which is why the markers appear to collapse. A blue marker outside the content item was exact at every offset tested, up to 2e11. So a position that reaches the scene graph must be viewport-relative and sit outside the moving content item. The error does not start at a clean 2^24 for every offset, as the earlier probe text implied.
- **Window size.** The probe's figures (4,140 items at 3840×2160, 11,462 at 7680×4320 on the user's aspects) depend on the row height, which the probe does not state. A synthetic port of `layout.rs`'s greedy pack (uniform 3:2, 4 px gap, ±1500 px cache) gives, at 3840×2160: 3,362 items at 60 px rows, 903 at 120, 300 at 220 (the default) and 112 at 400. At 1920×1080: 1,407, 396, 120 and 56. The probe's figures are the right order of magnitude for the 60 px minimum. They are not reproduced, because the user's aspects and the probe script are not in the repo. The 32-bit problem is in the total count, not in the window.
- **Unexplained.** At contentY 2e11 the red marker is at row 0 for both offsets tested (60 and 61). The float32 model puts it 4,096 px above the viewport, where it would be invisible.

**Fix.** Every number that crosses the bridge takes one of three types, and each site follows the rule for its kind. No site picks its own convention.

1. **Counts of files** (filter totals, facet counts, failed files, notice totals). `i64` in Rust, from SQL; `double` in QML; a `QVariant` in model roles. Exact to 2^53, about 9.0 × 10^15. A value that does not fit a `double` exactly is refused with a notice, never rounded. Changes: `filters_bridge.rs:37-38` (`total` and `matching`; `FilterBar.qml:23-24` become `real`; `active` stays `int`), `failed_files_bridge.rs:27`, `facet_choices_state.rs:11` with `facet_choices.rs:29` (which stops saturating), and `notices.rs:248`.
2. **Positions and list lengths** (item index, row, list length, `rowCount`). The Qt model API takes `int`, and the model holds the whole list, so these cannot widen before the window exists.
   - *Now: a gate.* A list of more than `i32::MAX` items is refused with a new notice kind, `ListTooLarge` (added by the steps under "Adding a kind" in `crates/model/src/notice.rs`), and no list is shown. The gate sits where a list enters the model (`gallery/model.rs`, `apply`), and before the rows are read if the catalog gives the count for the same query first. Behind the gate every item index, row index and the list length is below 2^31, so the `qt_int` calls on them are exact and the clamp is unreachable. Rows never outnumber items, so the row checks follow.
   - *Phase 7: the window* (§7.9 and §7.10 of the bounded plan). Qt rows are window-relative `int`s, below 2^31 by the window's size. Absolute positions are `i64` in Rust and `real` in QML (`current`, `viewing`, `anchor`, `count`), with an `i64` base and an `int` offset. The gate is removed when the window lands.
   - *Scene graph.* Any position that reaches the scene graph is viewport-relative and sits outside the content item (see Measured facts).
3. **Small bounded values** (rating 0 to 5, levels, passes, depth, kind, lane numbers, pixel sizes, key codes). They stay `int`. Each conversion names its bound in a `qt-edge-bound` comment, and a test pins the bound. Saturation is kept only for these, since they cannot reach it.

Retired: the -1 overflow sentinel in `to_i32`, the saturating helpers `to_row` and `index_for_qml`, and the inline sites. Each call moves to rule 1, 2 or 3.

Bridge functions must not panic (see Measured facts). "Fails loudly" means a refused value and a notice, never a panic. The workspace already denies `clippy::panic` (`Cargo.toml:87`).

The Qt sweep's selection (`docs/plans/QT_SENTINEL_SWEEP.md`) must address a window row by its absolute `i64` position, or by the file's identity. A Qt-side row handle inherits the 32-bit row limit.

**Tests.** Each check takes its value as an argument, so none needs a 2^31-item allocation.

- The gate accepts `i32::MAX` and refuses `i32::MAX + 1`.
- `real_count` is exact at 2^53 and refuses 2^53 + 1.
- `to_i32` is gone: a list index past `i32::MAX` is refused, not turned into -1.
- A count above `i32::MAX` shows its exact digits in the filter bar (a QML test under `crates/desktop/tests/qml`, with a stub count of 2^31 + 5).
- Phase 7: the window laws at `2^31+2^20+7` and `2^40` (`BOUNDED_MEMORY_PLAN.md` line 1550), and the id round trips at line 352.
- Phase 7: a position that reaches the scene graph stays viewport-relative, checked with the P10 marker harness.

**Verification method.**

- *QML:* QtTest cases run with Qt 6.12.0's `qmltestrunner` (`C:/Qt/6.12.0/msvc2022_64/bin`). `qml.exe` is a GUI binary and drops `console.log`, so the cases report through QtTest. The P10 case renders a `Window`, grabs it, and reads the marker rows from the saved PNG.
- *cxx:* a scratch crate on cxx 1.0.205, built offline. The cxx-qt generator output was read in `cxx-qt-gen` 0.10.0 (crates.io). The git fork the lockfile pins was not read.
- *Window size:* a throwaway port of `layout.rs`'s greedy pack.

**Open.**

- The 2e11 row-0 result. It needs an explanation before the Phase 7 scene-graph design is settled.
- The user's-aspect window figures and the 8K figure need the library and the probe script, which are not in the repo.
- The git-forked `cxx-qt-build` has not been read.
- Each `int` declaration in `crates/desktop/qml` (about 30 files) must be sorted into the three rules at implementation.

**Decisions for the user.**

- Add the gate now, or wait for Phase 7. Recommended: now. It is one check and one notice kind, and it turns a silent error into a loud one.
- Keep the rating at low, or raise it to medium.
- File-id type: the bounded plan must settle D29's decimal strings against `real`.
