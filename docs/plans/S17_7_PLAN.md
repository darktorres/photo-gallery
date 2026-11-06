# Plan: S-17.7, file identity across device renumbering

Written against `main` at 53ff12c. Depends on S-17.3/5 (`docs/plans/S17_REMAINDERS_PLAN.md`); see Sequencing. The refactor plan is `docs/REFACTOR_DESIGN.md`.

Status: implemented in steps 1 to 7; step 8 (the WSL benchmark and the perf ledger) was not run, so no timing is claimed. The deviations from this plan are recorded in the S-17.7 row of `docs/REFACTOR_PLAN.md`.

Code references (`file:line`) are as of `53ff12c`, before the change, unless a note says otherwise. For the current place of a symbol, grep for its name. Notes marked "Superseded" give the state as of 2026-11-06.

## Problem

`FileIdentity { volume, file_index }` (`crates/model/src/ids.rs:32`) keys `files` by `UNIQUE(volume, file_ref)` (`crates/catalog/src/lib.rs:144`; the baseline schema is at `:113`, `files` at `:126-145`). Both identity columns are `NOT NULL`. On Linux `volume` is `st_dev as u32` (`crates/platform/src/dir.rs:183`); on Windows it is the 32-bit `dwVolumeSerialNumber` (`dir.rs:86-91`).

On WSL2 (Ubuntu-26.04), `/mnt/d`'s device number changes. Across restarts it went 214 → 146 → 74 and then stayed at 74. Within one VM boot (boot id `229876ad`, no restart) it went 74 → 117 → 160. Each drvfs mount has its own device number, so a change means `/mnt/d`'s mount was replaced. The trigger is unknown.

When the number changes, every file gets an identity miss. Ratings, rotation and queued writes hang off `files.id`, so they would be lost without the rules below. The design handles any renumbering, including none.

## Verified facts

Probe scripts are in `docs/plans/probes/` and their results in `docs/plans/S17_7_PROBES.md`. The scripts are committed (`8904aac`).

Windows (NTFS on D:):

- `GetVolumeInformationByHandleW` works on a directory listing handle (access 0 or `FILE_LIST_DIRECTORY`). D: and C: report `NTFS`. `Z:` reports `FAT32`, serial `0x19831116`.
- `GetDriveTypeW` returns 3 (fixed) for D:, C: and `Z:`. A drive-type check alone cannot catch `Z:`; the file-system name does.
- The 64-bit `FileIdInfo` volume serial on D: is `0x16cc492ecc490987`. Its low 32 bits equal `dwVolumeSerialNumber`, `0xcc490987`. Seven volumes on this machine were checked: no two share low 32 bits. `FileIdInfo` fails on FAT32 (error 87), so the identity read must come after the file-system check.
- The listing's `CreationTime` matches the handle's creation time.
- In-place overwrite keeps the file ID and creation time. Save-by-rename (`MoveFileEx`) and `File.Replace` give a new file ID and a new creation time, so they arrive as an identity miss.
- Delete and recreate gives a new file ID every time (5 of 5). Rename keeps the file ID and creation time.
- UNC (`\\localhost\D$\`): `GetDriveTypeW` returns 4 (remote), while `GetVolumeInformationByHandleW` still reports `NTFS`. The drive-type check is what refuses an SMB share.
- Reparse roots. A junction on D: to `C:\Users\<you>`: `GetVolumePathNameW` follows it and returns `C:\`, and the volume checks pass. A symlink on D: to `\\localhost\D$\...`: `GetVolumePathNameW` does not follow it (returns `D:\`), `GetDriveTypeW` returns fixed, and the handle reports `NTFS`. Only the final path (`GetFinalPathNameByHandleW`) shows the target, `\\?\UNC\localhost\D$\...`. So the volume checks alone pass both links; refusal depends on order items 4 and 5 (decision 6).
- Zero file IDs on local NTFS: 0 of 400,000 non-dot entries (a sample of a development folder on D:, `C:\Users\<you>` and `C:\Windows\System32`, capped by the probe; not a census). 19 folders were unreadable. The zero-ID error in decision 10 is a guard for an unexpected value, not evidence that NTFS can return zero.

WSL (ext4 `$HOME`; `/mnt/d` is `9p`):

- ext4 reused the inode of a just-deleted file every time: 500 of 500, and 200 of 200 with neighbours present.
- ext4 birth time (`statx`) changes on rename-over and not on in-place rewrite.
- drvfs has no birth time (`stat -c %w` prints `-`), so `created` is NULL there.
- drvfs never reused an inode: 0 of 2000 delete-then-create cycles in an otherwise empty folder. The 95% upper bound is about 0.15% per cycle. This is evidence, not proof.
- Distinct devices had distinct `st_dev`: `/` 2096, `/proc` 132, `/dev/shm` 137, `/tmp` 166, `/mnt/c` 159, `/mnt/d` 160. On the GitHub runner `/` 66305, `/proc` 25, `/dev/shm` 27, and `/tmp` shares `/`'s device (66305). `/` and `/proc` are separate mounts on any Linux, so they are the two-device test pair (decision 11 and the tests).
- Restarting `docker-desktop` did not change `/mnt/d`'s device number. The within-boot change is unexplained.

## Decisions

1. **Volume is `u64` on both platforms. Catalogs from before this change are moved aside and rebuilt by a scan; no rows are converted.** Windows: `FileIdInfo`'s 64-bit serial from the root handle. Unix: `st_dev` whole. Binds are `as i64` and reads `as u64`; every bind site is listed in step 3. Rebuilding is exact, where converting a stored 32-bit serial to its 64-bit value would depend on the low 32 bits being unique on each machine.

2. **`created` matching is NULL-safe, and a changed `created` displaces the old row.** Two values are compatible when `(a IS NULL OR b IS NULL OR a = b)`.
   - Identity **hit**, `created` NULL on either side: the same file. If the stored value is NULL and the scanned value is known, write the scanned value (backfill). Rows from a rebuilt catalog get their `created` this way.
   - Identity **hit**, both known and equal: the same file. Update name and folder.
   - Identity **hit**, both known and different: a different file now has this identity. The walk **displaces** the hit row. It keeps its `id`, `folder_id`, `name`, ratings and content; it loses its identity (`volume` and `file_ref` set to NULL, `missing = 1`); and it is marked `unpaired`. A fresh row for the identity is inserted, also `unpaired`. Pairing decides what the displaced row becomes (decision 4).
   - Identity **miss**: a fresh row, `unpaired`.
   - On drvfs `created` is NULL, so identity hits keep the row. Inode reuse on drvfs was not observed (0 of 2000); that is the accepted risk there.

3. **Schema commit (step 3): `created`, `unpaired`, nullable identity, layout bump, catalog move-aside.**
   - Add `files.created INTEGER NULL` and `files.unpaired INTEGER NOT NULL DEFAULT 0`. The name `pending` is taken: `files.pending` (`:141`) is the import-work flag.
   - Make `volume` and `file_ref` nullable.
   - Add both new columns to `LAYOUT_CHECK` (`catalog/src/lib.rs:421-433`) and bump `LAYOUT_PARTS` (`:434`, 25 → 26; 26 → 27 if S-17.3/5 lands first). The bump is what makes `prepare` see the old catalog.
     Superseded (2026-11-06): `LAYOUT_CHECK` and `LAYOUT_PARTS` no longer exist. The schema is `BASELINE` in `crates/catalog/src/schema.rs` (`BASELINE_VERSION` 1, no migrations yet). A catalog whose schema differs from this build's is rebuilt (`carry_rows` in `schema.rs`). `files.created` and `files.unpaired` are in the baseline (`schema.rs:234` and `:251`).
   - `prepare` does not copy a catalog from an earlier layout in this step. It moves the catalog set aside (`catalog.sqlite`, `-wal`, `-shm` and `thumbs.sqlite` to `*.pre-s17-7`) and creates new ones, which the next scan fills. The moved files are kept until the user deletes them. This is the only step that does not copy rows.
   - Superseded (2026-11-06): the catalog file is moved aside by `move_aside` (`crates/catalog/src/lib.rs`, around line 316) to `<name>.replaced`, or `<name>.replaced.N` when that name is taken. The suffix is not `*.pre-s17-7`. No move-aside of `thumbs.sqlite` was found by grep in `crates/catalog/src` or `crates/thumbcache/src`.
   - Check the `files_to_import` index (`:153`), which keys on `volume` and `file_ref & 0xFFFFFFFFFFFF`, for the 64-bit change.
   - Back up the catalog before its first run anyway (see Rollback).

4. **Pairing at the end of every scan, driven by the persistent `unpaired` flag.**
   - `unpaired = 1` marks a row not yet resolved: a fresh row from a miss, or a displaced row (decision 2). Unpaired rows of this root are processed at the end of every scan, whether or not they were hit in it. An interrupted or deferred pairing is retried on the next scan. The list lives in the catalog, not in memory, so a rollback cannot lose it.
   - Pass order: tier 1, then tier 2, then replacement. Tiers 2 and replacement run only on a **complete** scan (no stop, no error). Each binding runs in one transaction.
   - **Tier 1 (same folder and name).** An unpaired row F pairs with exactly one row on another volume under the root that has the same `folder_id`, `name`, `file_ref`, `size`, `mtime` and a compatible `created`. "Exactly one" counts every such row, missing or not. Tombstones have NULL volume and never match. Zero or several: bind nothing and count it as ambiguous.
   - **Tier 2 (moved while closed).** F pairs with exactly one row on another volume under the root by `(file_ref, size, mtime, ext)` and a compatible `created`, again counting every row. The candidate's old volume must have no rows seen in this scan, and the candidate's folder must be readable and not a reparse point.
   - **Replacement (save-over, same name).** F with no children pairs with exactly one candidate from: (a) rows in the same folder with the same name that are displaced or still unpaired, or (b) rows in the same folder with the same name, `missing = 0`, `last_scan < scan`, a non-NULL identity, and a readable folder. `created` must be changed or NULL on either side. Missing rows are never candidates. The candidate takes F's identity, and its content state resets (decision 7).
   - An unpaired row **with children** (rating, rotation, analysis) is never paired by any tier. Pairing deletes the fresh row, which would lose its children. It stays unpaired and is counted in `ScanStats.unpaired_with_data` on every scan.
   - **Clearing.** At the end of a complete scan with no skipped folders on this root, unpaired rows that were not paired and have no children are cleared (`unpaired = 0`). They are genuinely new files. Displaced rows that found no replacement (a deleted file, or ext4 inode reuse under a different name) stay as tombstones. A scan with a skipped folder clears nothing, so the deferred rows are retried.
   - Ambiguous candidates bind nothing and are reported (decision 12). Nothing is guessed.

5. **Duplicates and hard links.** Within one root, the walk is deterministic for an unchanged tree, so the same path wins on every scan. Later paths with the same identity are skipped and counted in `ScanStats.duplicates`. Pairing does not depend on walk order. Across roots, a row that belongs to another root (`under_root!`) is not written by this root's scan, and a missing row does not claim its identity for another root.

6. **NTFS-only refusal (Windows), enforced at add time, at scan time, and in the CLI.**
   - `check_root(path) -> Result` lives in `crates/platform/src/dir.rs`, behind a `RootFacts` trait (the API calls), so the decision tests need no SMB share or FAT32 drive. It checks, in order, all before the path is opened:
     1. Refuse a UNC path by string (`\\` or `\\?\UNC\`).
     2. `GetVolumePathNameW` gives the volume root. An error refuses.
     3. `GetDriveTypeW` on the volume root must be `DRIVE_FIXED` (3). Anything else refuses. This is an allow-list, so removable drives are refused too.
     4. Open the path with `FILE_FLAG_OPEN_REPARSE_POINT` and refuse if it is itself a reparse point (a junction or symlink root).
     5. `GetFinalPathNameByHandleW` on that handle. Refuse unless the result is a local path on a drive letter, and repeat step 3 on its volume root.
     6. `GetVolumeInformationByHandleW` must report exactly `NTFS`.
     Any API error refuses (fail closed).
   - Unix: decision 13.
   - Placement: `add_folder` (`crates/app/src/library.rs:157-162`) and the CLI add path (`crates/cli/src/lib.rs:258`) call `check_root` before `add_root`. `pg_scan::scan` calls it first, before `open_folder`, `volume_serial` (`crates/scan/src/lib.rs:118-119`) and `begin_scan` (`:121`). A refused scan writes no `scans` row, and `mark_missing` never runs.
   - A refused root stays in the library, with a notice on each scan. Removal (`remove_folder`, `gallery remove`) is the way out, and the notice names it. `remove_root` keeps the rows (`catalog/src/lib.rs:1074-1085`).
   - The refusal is its own error variant. The runner maps every scan error to `Notice::ScanFailed` today (`crates/library/src/runner.rs:360-364`); a refusal maps to a new `Notice::RootRefused`. That needs a `text::Message` variant (`crates/client/src/text.rs`), a Fluent entry (`crates/client/locales/en/text.ftl`), and the mapping in `crates/desktop/src/library.rs:162`.
   - CLI: the add loop (`cli/src/lib.rs`, around `:250-258`) and the scan loop (`:267`) match on each root's result. A refused root prints its message and the other roots continue; the `?` in both loops is removed.

7. **Reset rules on replacement.** The candidate takes F's identity, and its content state resets completely:
   - `rotation` = 0; delete the file's `analysis`, `pdq_pairs` and `border_marks` rows.
   - Delete the file's thumbnail entries in `thumbs.sqlite`, after the catalog transaction commits (it is a separate file, so the two cannot share a transaction). A crash between the two leaves stale thumbnails and no rating is affected; the next scan of that file re-keys it.
   - Delete the `file_ratings` row, so the first sync treats the file as having no known rating.
   - Delete the queued content writes (`file_writes` rows of kind `rotation`, `catalog/src/lib.rs:272`) and their temp files.
   - Keep the queued **rating** writes, so the rating still reaches the new file. A rating row can also carry a `temp` (`writeback/src/lib.rs:289` records one for every non-video write, rating included). That temp is the old file's content with the old file's rating, so copying it in would put the old bytes into the new file. Delete the temp file and set `temp` and `temp_value` to NULL on the row. It keeps `value` and is written again from the new file.
   - Keep `ratings`. Set `size`, `mtime`, `created`, `last_scan` and `missing = 0`, and let the existing `UPDATE OF` triggers fire for the changed columns (`catalog/src/lib.rs:320-331`).

8. **Ratings.**
   - Replacement: the kept `ratings` value stands. Because `file_ratings` was deleted, the first sync sees `known = None`: it adopts the file's own rating if it has one, and otherwise the kept value stands.
   - Every other row: the existing three-way `sync_file_rating` (`catalog/src/lib.rs:1600-1617`) is unchanged. A file that loses its rating clears the catalog's rating, unless a write of ours is queued. The existing test at `:2536-2538` covers this.

9. **Reparse entries are indexed, not dropped.**
   - A reparse **file** is recorded from its listing metadata (identity, name, size, mtime, `created`, attributes). Its row stays present and `last_scan` advances. It gets no content work: no thumbnail, no analysis, no read of its contents. The listing does not hydrate a cloud placeholder. Counted in `ScanStats.skipped_reparse`.
   - A reparse **folder** is not descended into. Its subtree goes in the skip set, so `mark_missing` (`catalog/src/lib.rs:1013-1033`) keeps its rows. It is counted and noticed.
   - This replaces the silent drop at `crates/scan/src/lib.rs:61`.

10. **Zero file ID.** A NTFS listing entry with `file_id == 0` (other than `.` and `..`) makes `list()` return an error for its folder, which marks the folder unreadable. Its sibling entries are not updated; that is the stated behaviour. `list()` takes a `Listing` trait (`each` returns `Result`), so tests can supply entries; the real `Lister` implements it.

11. **Unix per-entry device and bind-mount loops.** `Entry.volume: Option<u64>` is `Some(dev)` from each entry's own `lstat` on Unix and `None` on Windows. The walk is recursive with an explicit ancestor list of `(dev, ino)`. A folder already on the ancestor list is skipped and counted (a bind-mount cycle). Entry metadata errors (`dir.rs:201`) become errors for the folder, not silent `continue`s.

12. **Reporting.** `ScanStats` gains `rebound`, `replaced`, `duplicates`, `skipped_reparse`, `unpaired_with_data` and `ambiguous`. The CLI prints them; the runner raises one notice for each non-zero count.

13. **Network filesystems are refused on Linux.** On Unix, `check_root` calls `statfs` on the root and refuses NFS, CIFS and SMB by their `f_type` magic numbers (the constants are checked against the `statfs` headers in step 1). Their inode numbers come from the server and can be unstable or duplicated, so inode identity is not trustworthy there. Every other type is accepted, including `9p`, which is WSL's drvfs: it is the path to NTFS that the tests use. Its inode numbers were not reused (0 of 2000), and each mount has its own device number (see Problem). A refusal uses the same notice, placement and CLI handling as decision 6.

Not built, on purpose: a fallback identity for FAT32, exFAT or ReFS; repointing a fresh row's children; a repair pass for duplicates in an old catalog (an old catalog is moved aside, not repaired).

## Steps

Each step is a commit, made when asked.

1. **Refusal, zero ID, listing seam (decisions 6, 10 and 13).** `crates/platform/src/dir.rs` (`check_root`, `RootFacts`), `crates/scan/src/lib.rs` (`scan`, `list`, the `Listing` trait), `crates/app/src/library.rs` (`add_folder`), `crates/cli/src/lib.rs` (the add and scan loops), and the notices (`runner.rs`, `text.rs`, `text.ftl`, `desktop/src/library.rs`). Docs: `docs/STORAGE.md` (the NTFS decision is enforced; the SMB row at `:32`; the stale `crates/library/src/scan.rs` references at `:10, 20, 41`; the SMB and ReFS rows at `:38-40`); `docs/plans/NTFS_MFT_SCAN_PLAN.md:39-41` says `Z:` scans silently as empty, which this step's refusal changes.
2. **Reparse entries indexed (decision 9).** `crates/scan/src/lib.rs` (`list`, the skip set), and the reparse notice. No schema change.
3. **Schema, volume `u64`, catalog move-aside (decisions 1 and 3).** One commit, described in decision 3. Touches `BASELINE` (`catalog/src/lib.rs:113`, `files` at `:126-145`), `LAYOUT_CHECK` and `LAYOUT_PARTS`, `FILLED` if it needs the new columns, and the move-aside in `prepare`. Also `crates/model/src/ids.rs`, `dir.rs` (`volume_serial` returns `u64`), `scan/src/lib.rs`, `catalog/src/lib.rs` (`upsert_file`, `known`, `thumb_sources`, the `ORDER BY volume` reads, `fixture.rs:55`), and `thumbcache/src/lib.rs:672`. `dir.rs` provides `created` on both platforms: `CreationTime` on Windows and `statx` birth time on Linux (NULL where unsupported; a `statx` error maps to NULL, never to a failed scan).
4. **Per-entry device and the recursive walk (decision 11).** `dir.rs` (`Entry.volume`), `scan/src/lib.rs` (the walk and the ancestor list).
5. **Upsert: hits, backfill, displacement (decision 2).** `catalog/src/lib.rs` upsert (around `:1385-1436`): `RETURNING id`, `created` written and compared in `same`, the `unpaired` marks.
6. **Pairing, replacement reset, rating rules (decisions 4, 7 and 8).** `pg_scan::scan` and the catalog pairing functions. `under_root!` and `under_params` (`catalog/src/lib.rs:88-111`) scope the pairing to the root. Apply the queued-write rules of decision 7 (the kind column is `rating` or `rotation`).
7. **Duplicates, stats, notices (decisions 5 and 12).**
8. **Bench, ledger and docs.** Commit the wall-clock scan bench first. Then run `just perf --update` and commit the baseline separately (`wsl-bench.sh` benches committed HEAD, so the order matters). Update `docs/REFACTOR_PLAN.md` S-17.7 (row 823) and `docs/STORAGE.md` (a Linux section).

## Tests

Each step lands with its tests.

- **Step 1.**
  - `check_root` through `RootFacts` fakes: a remote drive is refused; a removable drive is refused; `DRIVE_FIXED` with FAT32 is refused; `NTFS` is allowed; each API error is refused; a reparse root is refused; a final path on another volume or a remote drive is refused.
  - A real UNC string is refused before any API call (no share needed).
  - The `Z:` test uses `testkit::missing_tool`, not a silent skip (CI sets `PG_REQUIRE_TOOLS`).
  - Linux: `statfs` results for NFS, CIFS and SMB are refused through a fake; local types and `9p` are accepted. The refused `f_type` values are `NFS_SUPER_MAGIC` 0x6969, `CIFS_SUPER_MAGIC` 0xFF534D42, `SMB2_SUPER_MAGIC` 0xFE534D42 and `SMB_SUPER_MAGIC` 0x517B; `9p` is `V9FS_MAGIC` 0x01021997 (checked against the kernel headers, see Not verified).
  - A zero-ID entry from a fake `Listing`: the folder is unreadable and its siblings are not updated.
  - CLI: three roots, the middle one refused: the first and third are added and scanned.
  - The runner maps a refusal to `RootRefused`, not `ScanFailed`.
- **Step 2.** A reparse file from a fake listing keeps its row and advances `last_scan`, with no content job created. A reparse folder's subtree survives `mark_missing`.
- **Step 3.**
  - A catalog from the previous layout is moved aside to `*.pre-s17-7` and a new, empty one is created; the moved files are untouched.
  - `u64` volumes above `i64::MAX` round-trip through every bind site (enumerated in the step).
  - NULL-safety: `created` NULL on one side with the same identity is the same row.
- **Step 4.**
  - Linux, two real devices, `/` and `/proc`: two `Lister::each` calls on two directories; each `Entry.volume` equals that directory's own `st_dev`, read by an independent `lstat`. A bug that copies the first device to both fails this.
  - A cycle from a fake listing (a child with an ancestor's `(dev, ino)`) is skipped and counted.
- **Step 5.**
  - Hit, unchanged `created`: the same row.
  - Hit, stored NULL `created`, scanned value known: `created` is backfilled.
  - Hit, changed `created`: the displaced row keeps `id`, folder, name and ratings, with NULL identity and `missing = 1`; the fresh row is inserted and unpaired.
- **Step 6.**
  - Rename on a complete scan keeps the row (tier 2).
  - Rename while closed, on an **interrupted** scan: tier 1 pairs it; tier 2 does not; the next complete scan pairs the rest (retry).
  - Tier 1 with zero candidates, and with two, binds nothing and counts as ambiguous.
  - Two swapped names across a volume change are not cross-bound.
  - A same-inode, same-size, same-mtime file with a different name and extension does not pair under tier 2.
  - **A missing row is never a replacement candidate**: a deleted, rated photo and a new file with the same name in the same folder leave the rating on the deleted row.
  - Save-over (rename-over, same name): the old row keeps its rating, resets its content, loses its `file_ratings` row, and keeps its queued rating write.
  - Two unseen rows with the same name: nothing binds.
  - ext4 reuse under a different name: the displaced row stays a tombstone, and the new file inherits no rating.
  - ext4 reuse under the same name (accepted, decision 4): the new file takes the rating, and the test records that it does.
  - An unpaired row with a rating, rotation or analysis is not paired at any tier; `unpaired_with_data` counts it on every scan.
  - An unpaired row is cleared after a clean complete scan without children; it is not cleared when a folder was skipped.
  - A pairing that crosses the 10,000-file batch boundary (`scan/src/lib.rs:39`) still finds its unpaired rows, since they are in the catalog.
  - The existing `sync_file_rating` test (`catalog/src/lib.rs:2536-2538`) passes unchanged.
- **Step 7.** A hard-link second path is skipped and counted; the row does not flip between two scans; a missing row in another root does not claim the identity.
- **Step 8.** The bench runs on committed HEAD only; the wall-clock timing is recorded in the ledger, not gated.
- **Manual, WSL.** A tmpfs mounted under a scanned root scans twice with no row flipping.
- **Manual, end to end.** Build the catalog with the old binary (`main`), rate a file, run `wsl --shutdown`, and scan with the new binary. Check `stat -c %d` before and after: if the device number did not change, the run proves nothing and is repeated. The rows keep their ids, the rating is on a present row, and analysis is kept. Stop `docker-desktop` first and restart it afterwards.

## Verification

- `just lint` exits 0.
- `just test` with `$env:PG_TEST_DIR = '<repo>\target\pg-test'` (PowerShell syntax) exits 0.
- WSL: a copy of the **working tree** (not HEAD; `tools/wsl-check.sh` and `tools/perf-check.sh` check committed HEAD only), with `. ~/.cargo/env` and `FFMPEG_DIR` set as the tool does, and `LIBCLANG_PATH` discovered rather than hard-coded. Run `cargo clippy --workspace --exclude pg-desktop --exclude pg-gallery --all-targets -- -D warnings` and the same `cargo test`. Linux CI runs the catalog, scan and platform tests (`.github/workflows/ci.yml:232`, the `linux` job's `cargo test`), so they must pass there.
- **Perf.** `just perf` is callgrind (`crates/bench/benches/ir/linux.rs`). It counts instructions and does not see drvfs I/O. `tools/perf-compare.py` reads only `callgrind.*.out` files, so a wall-clock bench is not gated and needs no `--update`. Add a wall-clock scan timing (a 10k-file fixture: first scan and renumbered rescan, before and after) and record it in the ledger. The catalog bench's `LAYOUT` constant is at `crates/bench/benches/ir/catalog.rs:30` and changes with the schema; its dataset is regenerated with `bench_data.py`.

## Rollback

- Back up the catalog directory before the first run of each step: `catalog.sqlite` with its `-wal` and `-shm` (`crates/app/src/config.rs:28`), and `thumbs.sqlite` (`crates/thumbcache/src/lib.rs:130`). Step 3 also moves these files aside itself (decision 3).
- Each step is a commit. Reverting a commit restores the code, not the catalog.
- Step 3 changes the catalog in a way the older code does not detect. The older layout check does not count the new columns, so it still matches and opens the catalog silently. Its `u32` volume binds then miss the 64-bit identities, and tombstones with NULL volume are misread. Going back past step 3 therefore means restoring the moved-aside files. The step 3 commit message says so.
- Steps 1, 2 and 4 to 8 keep the catalog readable by the code of the same commit. Steps 1 and 2 do not change the catalog layout.

## Sequencing and conflicts

- Files shared with `docs/plans/S17_REMAINDERS_PLAN.md`: `crates/catalog/src/lib.rs` (`LAYOUT_CHECK`, `LAYOUT_PARTS`, `FILLED`, the test helper; the first two and `FILLED` no longer exist, see decision 3), `crates/cli/src/lib.rs`, and `docs/REFACTOR_PLAN.md` (rows 820-824). `crates/app/src/library.rs` is not shared; the companion plan touches `views.rs`, `images.rs` and `reader.rs` in that crate.
- S-17 step 2 (25 → 26, `views.folder`) lands before step 3 here (26 → 27). S-17 step 8 (the catalog helper) lands before step 6's catalog tests.
  Superseded (2026-11-06): the layout is no longer numbered (decision 3), so there is no 25, 26 or 27 to bump. S-17.7 has landed; `views.folder` is still `TEXT` (the S-17.3 row, `docs/REFACTOR_PLAN.md` row 819), so the companion plan's change is still open.
- S-17's decision 3 says `drives: u32` becomes `anchors: u64`. That is already done (`crates/model/src/tree.rs:23`, ledger row 820), so it is not this plan's work, and the S-17 plan should drop it.

## Settled questions

- **Re-key or rebuild (decisions 1 and 3).** Rebuild: catalogs from before the change are moved aside and rebuilt by a scan. Ratings in those catalogs are not carried over.
- **Ambiguous candidates (decision 4).** Bind nothing; report the count.
- **Reparse placeholders (decision 9).** Index from listing metadata with no content work; the rows stay present.
- **In-place rating sync (decision 8).** The existing three-way sync stays. A file that loses its rating clears the catalog's rating. Only replacement adopts the file's own rating.
- **Removable drives (decision 6).** Refused by the drive-type allow-list.
- **Same-name reuse inherits the rating (decision 4, candidates a and b).** Accepted and documented. A rename-over and an inode reuse under the same name look the same to the scanner, so the signals used are name, folder and changed identity.
- **Non-NTFS roots on Linux (decision 13).** Network filesystems (NFS, CIFS, SMB) refused. Local types and `9p` accepted.

## Not verified yet

- The trigger that replaces `/mnt/d`'s mount. The mechanism is known (each drvfs mount has its own device number). Not reproduced: within boot `229876ad`, `/mnt/d` went from 160 to 195 and `/mnt/c` from 159 to 194, and the kernel log shows no 9p event. The design does not depend on it.
