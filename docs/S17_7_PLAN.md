# S-17.7: file identity across device renumbering (as built)

Planned against `main` at 53ff12c and built in steps 1 to 8. This is the record of what was built, as of 2026-11-11: each passage states the code as it is, and keeps the plan's intent where it explains a decision. Deviations lists where the build departed from the plan. The ledger row is S-17.7 in `docs/REFACTOR_PLAN.md`. The companion work, S-17.3/5, is recorded in its ledger rows; `docs/S17_REMAINDERS_PLAN.md` keeps its probes. The refactor design is `docs/REFACTOR_DESIGN.md`.

Code is cited by symbol and file; grep for a name to find it. A `file:line` cite names the code at 53ff12c, before the change.

## Problem

`FileIdentity { volume, file_index }` (`crates/model/src/ids.rs`) keys `files` by `UNIQUE(volume, file_ref)`. At 53ff12c both identity columns were `NOT NULL` and the volume was 32 bits: `st_dev as u32` on Linux, and `dwVolumeSerialNumber` on Windows.

On WSL2 (Ubuntu-26.04), `/mnt/d`'s device number changes. Across restarts it went 214 → 146 → 74 and then stayed at 74. Within one VM boot (boot id `229876ad`, no restart) it went 74 → 117 → 160. Each drvfs mount has its own device number, so a change means `/mnt/d`'s mount was replaced. The trigger is unknown.

When the number changes, every file gets an identity miss. Ratings, rotation and queued writes hang off `files.id`, so without the rules below they would be lost. The design handles any renumbering, including none.

## Verified facts

Probe scripts are in `docs/probes/` and their results in `docs/S17_7_PROBES.md`. The scripts are committed (`8904aac`).

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

1. **Volume is `u64` on both platforms, and no stored identity is converted.** Windows: the 64-bit `FileIdInfo` serial, read from the root handle (`Dir::volume`, `crates/platform/src/dir.rs`). Unix: the volume is read per entry (decision 11). On Linux it is the file system's UUID, from the kernel, udev's database, or an ext file system's `f_fsid`, resolved once per device (`Lister::volume_of_device`); a device with none (procfs, tmpfs, drvfs) keeps its device number, and the scan counts it (`ScanStats.volume_fallbacks`, the note `ScanNote::VolumeFallback`). The UUID came in 94a2530 (2026-11-09); until then the volume was `st_dev` whole. Binds are `cast_signed()` to `i64` and reads go back to `u64`. A catalog from before the change is rebuilt (decision 3), with its stored identities as they were: converting a stored 32-bit serial to its 64-bit value would depend on the low 32 bits being unique on each machine.

2. **`created` matching is NULL-safe, and a changed `created` displaces the old row** (`ScanWriter::upsert_file`, `crates/catalog/src/writers.rs`). Two values are compatible when `(a IS NULL OR b IS NULL OR a = b)`.
   - Identity **hit**, `created` NULL on either side: the same file. If the stored value is NULL and the scanned value is known, the scanned value is written (backfill). Rows carried into a rebuilt catalog get their `created` this way.
   - Identity **hit**, both known and equal: the same file. Name and folder are updated.
   - Identity **hit**, both known and different: a different file now has this identity. The walk **displaces** the hit row. It keeps its `id`, `folder_id`, `name`, ratings and content; it loses its identity (`volume` and `file_ref` set to NULL, `missing = 1`); and it is marked `unpaired`. A fresh row for the identity is inserted, also `unpaired`. Pairing decides what the displaced row becomes (decision 4).
   - Identity **miss**: a fresh row, `unpaired`.
   - On drvfs `created` is NULL, so identity hits keep the row. Inode reuse on drvfs was not observed (0 of 2000); that is the accepted risk there.

3. **Schema: `created`, `unpaired`, nullable identity; a catalog of another schema is rebuilt and moved aside.**
   - `files.created INTEGER NULL` (FILETIME ticks, NULL when unknown) and `files.unpaired INTEGER NOT NULL DEFAULT 0` are in `BASELINE` (`crates/catalog/src/schema.rs`). The name `pending` was taken: `files.pending` is the import-work flag.
   - `volume` and `file_ref` are nullable.
   - There is no layout number. `Catalog::prepare` (`crates/catalog/src/core.rs`) compares the file's schema with the one this build makes for its version. A catalog of this build's schema at an earlier version is migrated forward by `MIGRATIONS` (the baseline was frozen on 2026-11-08; the first entry is version 2, the change stamps for borders, CODE_REVIEW C-8). Any other catalog, every one from before S-17.7 included, is rebuilt: a new catalog is made beside it, the old rows are copied into it with their ids by `carry_rows` (`crates/catalog/src/rebuild.rs`), and a copy that fails leaves the new catalog empty, with the reason said.
   - The old file, with its `-wal`, `-shm` and `-journal`, is moved aside by `pg_dbfile::move_aside` (`crates/dbfile/src/remove.rs`) to `catalog.sqlite.replaced`, then `.replaced.1`, and so on. Nothing is deleted; the user removes the moved files. `thumbs.sqlite` is not moved aside with the catalog: the same function moves a damaged one to `thumbs.sqlite.bad`, and one of another layout or catalog is started over.
   - The `files_to_import` index keys on `volume` and `file_ref & 0xFFFFFFFFFFFF`; it is unchanged by the 64-bit volume.
   - Back up the catalog before a first run anyway (see Rollback).

4. **Pairing at the end of every scan, driven by the persistent `unpaired` flag** (`Catalog::pair_after_scan`, `crates/catalog/src/pair.rs`).
   - `unpaired = 1` marks a row not yet resolved: a fresh row from a miss, or a displaced row (decision 2). The rows paired are the unpaired rows of this root that this scan saw (`last_scan = scan`), which are in folders it listed; an unpaired row the scan did not see waits for a scan that sees it. An interrupted or deferred pairing is retried on the next scan. The list lives in the catalog, not in memory, so a rollback cannot lose it.
   - The pairing takes a `Coverage { complete, skipped }` (`crates/catalog/src/pair/coverage.rs`). Tier 1 runs on any scan that reaches the pairing. Tier 2, replacement and the clearing run on a complete scan (not stopped). The folders the walk did not list (unreadable ones, refused mounts and reparse folders, with their subfolders: the same set `mark_missing` keeps) are left out of every tier; a row there is neither a candidate nor cleared. A scan with an unreadable or reparse folder still pairs the rest of the root (CODE_REVIEW C-40).
   - Pass order: tier 1, then tier 2, then replacement. Each tier runs over the fresh rows in batches of at most `WRITE_BATCH` by id, each batch in a short write transaction of its own that reads the batch and runs the tier's queries inside it. A batch is a seek on the `files_fresh` index. The last tier's batches also clear the rows nothing paired.
   - **Tier 1 (same folder and name).** An unpaired row F pairs with exactly one row on another volume under the root that has the same `folder_id`, `name`, `file_ref`, `size`, `mtime` and a compatible `created`. "Exactly one" counts every such row, missing or not. Tombstones have NULL volume and never match. Several candidates bind nothing and count the row as ambiguous; no candidate binds nothing and is not counted.
   - **Tier 2 (moved while closed).** F pairs with exactly one row on another volume under the root by `(file_ref, size, mtime, ext)` and a compatible `created`, again counting every row. The candidate's old volume must have no rows seen in this scan, and the candidate must not be in a skipped folder.
   - **Replacement (save-over, same name).** F with no children pairs with exactly one candidate from: (a) rows in the same folder with the same name that are displaced or still unpaired, or (b) rows in the same folder with the same name, `missing = 0`, `last_scan < scan` and a non-NULL identity, outside the skipped folders. `created` must be changed or NULL on either side. Missing rows are never candidates. The candidate takes F's identity, and its content state resets (decision 7).
   - An unpaired row **with children** (rating, rotation, analysis) is never paired by any tier. Pairing deletes the fresh row, which would lose its children. It stays unpaired and is counted in `Pairing.unpaired_with_data` on every scan.
   - **Clearing.** At the end of a complete scan, unpaired rows that were not paired, have no children and are outside the skipped folders are cleared (`unpaired = 0`). They are genuinely new files. Displaced rows that found no replacement (a deleted file, or ext4 inode reuse under a different name) stay as tombstones. Rows in a skipped folder are not cleared, so they are retried.
   - Ambiguous candidates bind nothing and are reported (decision 12). Nothing is guessed.

5. **Duplicates and hard links.** Within one root, the walk is deterministic for an unchanged tree, so the same path wins on every scan. Later paths with the same identity are skipped and counted in `ScanStats.duplicates`. Pairing does not depend on walk order. Across roots (`under_root!`, `crates/catalog/src/sql.rs`), a present row of another root keeps its identity: the file here is a second path to it, recorded as `Recorded::Foreign` and counted as a duplicate. A missing row of another root does not claim its identity: `ScanWriter::upsert_file` treats it as this root's hit, so the row moves here with its data as a move within the root would (or is displaced, if the creation times differ). This half was not built until 2026-11-11; before then a missing row of another root kept the file out.

6. **NTFS-only refusal (Windows), enforced at add time, at scan time, and in the CLI.**
   - `check_root` (`crates/scan/src/refusal.rs`) works through the `RootFacts` trait (`crates/platform/src/dir.rs`, the API calls; `SystemFacts` is the real one), so the decision tests need no SMB share or FAT32 drive. `pg_scan::check_folder` is its public entry. It checks, in order, all before the path is opened:
     1. Refuse a UNC path by string (`\\` or `\\?\UNC\`).
     2. `GetVolumePathNameW` gives the volume root. An error refuses.
     3. `GetDriveTypeW` on the volume root must be `DRIVE_FIXED` (3). Anything else refuses. This is an allow-list, so removable drives are refused too.
     4. Open the path with `FILE_FLAG_OPEN_REPARSE_POINT` and refuse if it is itself a reparse point (a junction or symlink root).
     5. `GetFinalPathNameByHandleW` on that handle. Refuse unless the result is a local path on a drive letter, and repeat step 3 on its volume root.
     6. `GetVolumeInformationByHandleW` must report exactly `NTFS`.
     Any API error refuses (fail closed).
   - Each refusal is a typed `RefusedReason` (`crates/model/src/refusal.rs`) that the scanner decides and the client words (`NetworkPath`, `NetworkDrive`, `RemovableDrive`, `OtherDrive`, `Link`, `RealNotLocal`, `RealOnOtherVolume`, `NotNtfs`, `Unreadable`, and the Linux reasons of decision 13).
   - Unix: decision 13.
   - Adding: `pg_library::folders::add_folder` (`crates/library/src/folders.rs`) checks the folder first. A refused folder is not added; the add fails with `AddError::Refused { folder, reason }`, which the app reports as `Notice::FolderRefused { folder, reason }` (`crates/model/src/notice.rs`) and the CLI prints as that notice's line.
   - Scanning: `pg_scan::scan` calls `check_folder` before the walk. A refused scan writes no `scans` row and `mark_missing` never runs. The refusal is recorded in the folder ledger (`Catalog::folder_refused`, `FolderFailure::Refused`; `docs/NOTIFICATIONS.md`), which the next complete walk of the folder, or of one above it, sweeps, and the scan returns `ScanError::Refused`. The runner posts nothing for it: it is the ledger's, not a failed scan.
   - A root refused after it was added stays in the library, and each scan records the refusal again. Removal (`remove_folder`, `gallery remove`) is the way out. `Catalog::remove_root` keeps the rows.
   - CLI (`crates/cli/src/lib.rs`): the add loop and the scan loop match on each root's result. A refused root prints its notice line and the other roots continue; any other error stops the command.

7. **Reset rules on replacement.** The candidate takes F's identity, and its content state resets completely:
   - `rotation` = 0; the file's `analysis`, `pdq_pairs` and `border_marks` rows are deleted.
   - The file's thumbnail entries in `thumbs.sqlite` are deleted by the caller after the catalog transaction commits (`Pairing.replaced_ids`; it is a separate file, so the two cannot share a transaction). A crash between the two leaves stale thumbnails and no rating is affected; the next scan of that file re-keys it.
   - The `file_ratings` row is deleted, so the first sync treats the file as having no known rating.
   - The queued content writes (`file_writes` rows of kind `rotation`) are deleted, and their temp files with them.
   - The queued **rating** writes are kept, so the rating still reaches the new file. A rating row can also carry a `temp` (the write-back records one for every non-video write, rating included). That temp is the old file's content with the old file's rating, so copying it in would put the old bytes into the new file. The temp file is deleted by the caller after the commit (`Pairing.temps`), and `temp` and `temp_value` are set to NULL on the row. It keeps `value` and is written again from the new file.
   - `ratings` are kept. `size`, `mtime`, `created`, `last_scan` and `missing = 0` are set, and the existing `UPDATE OF` triggers fire for the changed columns.

8. **Ratings.**
   - Replacement: the kept `ratings` value stands. Because `file_ratings` was deleted, the first sync sees `known = None`: it adopts the file's own rating if it has one, and otherwise the kept value stands.
   - Every other row: the three-way `sync_file_rating` (`crates/catalog/src/writers.rs`) is unchanged. A file that loses its rating clears the catalog's rating, unless a write of ours is queued. The test `three_way_rating_sync` covers it.

9. **Reparse entries are indexed, not followed.**
   - A reparse **file** that has a row keeps it: the row stays present and `last_scan` advances. It gets no content work: no thumbnail, no analysis, no read of its contents. The listing does not hydrate a cloud placeholder. A new reparse file gets no row (see Deviations). Counted in `ScanStats.skipped_reparse`.
   - A reparse **folder** is not descended into. Its subtree goes in the skip set, so `mark_missing` (`Catalog::mark_missing`, `crates/catalog/src/roots.rs`) keeps its rows, and the pairing leaves it out of every tier (`Coverage.skipped`, decision 4). It is counted and noted.
   - This replaces the silent drop of reparse entries at 53ff12c (`crates/scan/src/lib.rs:61`).

10. **Zero file ID.** A NTFS listing entry with `file_id == 0` (other than `.` and `..`) makes `list()` return an error for its folder, which marks the folder unreadable. Its sibling entries are not updated; that is the stated behaviour. `list()` takes a `Listing` trait (`crates/scan/src/lib.rs`), so tests can supply entries; the real `Lister` implements it.

11. **Unix per-entry device and bind-mount loops.** `Entry.volume: Option<u64>` (`crates/platform/src/dir/listing.rs`) is, on Unix, the volume of each entry's own device from its `lstat` (decision 1), and `None` on Windows, where every entry is on its folder's volume. The walk keeps an explicit ancestor list of `(dev, ino)`. A folder already on the ancestor list is skipped and counted (`ScanStats.loops`, a bind-mount cycle). Entry metadata errors become errors for the folder, not silent `continue`s.

12. **Reporting.** `ScanStats` (`crates/scan/src/lib.rs`) has `duplicates`, `skipped_reparse`, `loops`, `refused_mounts` and `volume_fallbacks`, and the pairing's counts in `ScanStats.pairing` (`Pairing`: `rebound`, `replaced`, `ambiguous`, `unpaired_with_data`). The CLI prints them (`scan_report`); the runner raises one `ScanNote` notice for each non-zero count (`crates/library/src/scans.rs`).

13. **Network, FAT and pseudo file systems are refused on Linux.** On Unix, the check calls `statfs` on the root and refuses by `f_type` (`FS_TYPES`, `crates/scan/src/refusal.rs`): NFS (`NFS_SUPER_MAGIC` 0x6969), SMB and CIFS (`SMB_SUPER_MAGIC` 0x517B, `CIFS_SUPER_MAGIC` 0xFF534D42, `SMB2_SUPER_MAGIC` 0xFE534D42), FAT (`MSDOS_SUPER_MAGIC` 0x4D44), exFAT (0x2011BAB0), and the kernel's pseudo file systems (`proc`, `sysfs`, `devpts`, `cgroup2`, `debugfs`, `tracefs`, `securityfs`, `bpf`, `pstore`, `configfs`). Network inode numbers come from the server and can be unstable or duplicated. FAT and exFAT store no inode numbers: the kernel derives them at each mount. The pseudo file systems hold kernel state, not photos. Every other type is accepted, including `9p` (`V9FS_MAGIC` 0x01021997), which is WSL's drvfs: it is the path to NTFS that the tests use. Its inode numbers were not reused (0 of 2000), and each mount has its own device number (see Problem). A refusal uses the same reasons, placement and CLI handling as decision 6. A mount under a scanned root whose file system is refused is not walked: its rows stay, and it is counted (`ScanStats.refused_mounts`) and recorded in the folder ledger.

Not built, on purpose: a fallback identity for FAT32, exFAT or ReFS; repointing a fresh row's children; a repair pass for duplicates in an old catalog (an old catalog is rebuilt, not repaired).

## Deviations

Where the build departed from the plan:

- **No ambiguity for no candidate.** A fresh row with no candidate in a tier is not ambiguous. The plan counted zero candidates as ambiguous, which would flag every new file. Several candidates still count.
- **A new reparse file gets no row.** The plan indexed every reparse file. Only one that already has a row keeps it, since the flag that would keep a new row out of the import is not in this step.
- **A refused folder is not added.** The plan kept a refused root in the library with a notice. The add fails with the reason; a root refused after it was added stays and is reported on each scan.
- **The refusal notice.** The plan's `Notice::RootRefused` is `Notice::FolderRefused { folder, reason }`, with a typed `RefusedReason` that the scanner decides and the client words (Review 4, step 2). At scan time a refusal is a row of the folder ledger, and the runner posts no notice.
- **`check_root`'s place.** It is in `crates/scan/src/refusal.rs`, behind `RootFacts` in `crates/platform/src/dir.rs`, not in `dir.rs` itself.
- **The `Z:` test is a fake.** The FAT32 refusal is tested through a `RootFacts` fake (`a_fixed_fat32_drive_is_refused_by_its_file_system`), because the real drive is not on CI. The plan named `testkit::missing_tool`.
- **No layout number.** `LAYOUT_CHECK` and `LAYOUT_PARTS` (and the 25 → 26 → 27 bumps) were not built. The schema is compared whole, a catalog of another schema is rebuilt with its rows carried by `carry_rows`, and later changes are `MIGRATIONS` steps.
- **The move-aside suffix.** The old catalog goes to `catalog.sqlite.replaced` (then `.replaced.1`), not `*.pre-s17-7`. `thumbs.sqlite` is not moved aside with it.
- **The Linux volume.** Since 94a2530 (2026-11-09) the volume on Linux is the file system's UUID, falling back to the device number, counted as `ScanNote::VolumeFallback`. The plan used `st_dev` whole.
- **More refused file systems on Linux.** Decision 13 refused only NFS, CIFS and SMB. FAT, exFAT and the kernel's pseudo file systems are refused too, and a refused mount under a root is skipped and kept in the folder ledger.
- **The pairing's transactions.** The plan ran each binding in one transaction. Each tier runs in short batches of fresh rows, each batch one write transaction.
- **Skipped folders.** The plan skipped tier 2, replacement and the clearing for the whole root when a folder was skipped. The skipped folders and their subfolders are left out of every tier, and the rest of the root is paired (CODE_REVIEW C-40).
- **Decision 5 across roots.** "A missing row does not claim its identity for another root" was not implemented until 2026-11-11 (`ScanWriter::upsert_file`).
- **The upsert.** `upsert_file` has no `RETURNING id`, and `created` is compared in its hit logic rather than in `same`.
- **The bench script.** The wall-clock scan timing is `tools/profiling/scan_timing.py` (first written as `scan-timing.sh`).

## Steps

Each step was a commit.

1. **Refusal, zero ID, listing seam (decisions 6, 10 and 13).** `check_root` and `RootFacts`; `pg_scan::scan`, `list` and the `Listing` trait; the add path; the CLI's add and scan loops; the refusal's reasons and their wording. Docs: `docs/STORAGE.md` (the NTFS decision is enforced; the SMB, ReFS and other file-system rows), and `docs/NTFS_MFT_SCAN_PLAN.md`, whose sentence that `Z:` scans silently as empty this step's refusal changes.
2. **Reparse entries indexed (decision 9).** `crates/scan/src/lib.rs` (`list`, the skip set) and the reparse note. No schema change.
3. **Schema, volume `u64`, catalog move-aside (decisions 1 and 3).** `BASELINE` (the new columns, nullable identity), the rebuild and move-aside in `Catalog::prepare`, `crates/model/src/ids.rs`, `Dir::volume` (returns `u64`), and the catalog's binds and reads of `volume` (`upsert_file`, `known`, `thumb_sources`, the `ORDER BY volume` reads). The thumbnail cache reads the volume in `pending_sql` (`crates/thumbcache/src/lib.rs`). The listing provides `created` on both platforms: `CreationTime` on Windows and the `statx` birth time on Linux (NULL where unsupported; an error maps to NULL, never to a failed scan).
4. **Per-entry device and the recursive walk (decision 11).** `Entry.volume`, and the walk with its ancestor list.
5. **Upsert: hits, backfill, displacement (decision 2).** `ScanWriter::upsert_file`: `created` written and compared in the hit logic, and the `unpaired` marks.
6. **Pairing, replacement reset, rating rules (decisions 4, 7 and 8).** `Catalog::pair_after_scan`, called by `pg_scan::scan`. `under_root!` and `under_params` scope the pairing to the root. The queued-write rules of decision 7 (the kind column is `rating` or `rotation`).
7. **Duplicates, stats, notices (decisions 5 and 12).**
8. **Bench, ledger and docs.** The wall-clock scan timing (`tools/profiling/scan_timing.py`) was written in S-17 Unit G and run on 2026-11-07; the results are in `docs/PROFILING.md` (Measurements). It is not gated, so it needed no `just perf --update` (Verification). `docs/REFACTOR_PLAN.md` S-17.7 and `docs/STORAGE.md` (its Linux section) were updated.

## Tests

Each step landed with its tests. The tests named below that did not exist when the steps landed were added on 2026-11-11: in `crates/catalog/src/pair.rs`, `tier_one_binds_nothing_with_no_candidate_or_two`, `tier_two_does_not_pair_a_file_of_another_extension`, `swapped_names_across_a_volume_change_are_not_cross_bound`, `a_missing_row_is_never_a_replacement_candidate`, `two_unseen_rows_with_one_name_bind_nothing`, `an_inode_reused_under_another_name_leaves_a_tombstone_and_no_rating` and `an_inode_reused_under_the_same_name_takes_the_rating`; in `crates/scan/src/lib.rs`, `a_pairing_across_the_batch_boundary_still_finds_its_rows` and `a_missing_row_in_another_root_does_not_claim_the_identity`; `a_refused_folder_is_in_the_folder_ledger_not_a_failed_scan` (`crates/library/src/runner.rs`); and `a_refused_folder_between_two_others_does_not_stop_the_scan` (`crates/cli/src/lib.rs`). The pairing tests run under both path rules (`under_both_rules!`).

- **Step 1.**
  - `check_root` through `RootFacts` fakes (`crates/scan/src/refusal.rs`): a remote drive, a removable drive and any other non-fixed drive are refused (`each_drive_that_is_not_a_fixed_disk_is_refused_for_its_kind`); `DRIVE_FIXED` with FAT32 is refused (`a_fixed_fat32_drive_is_refused_by_its_file_system`); a reparse root is refused (`a_reparse_root_is_refused`); a final path on another volume or a remote drive is refused (`a_real_location_off_a_local_drive_or_on_another_volume_is_refused`). `NTFS` is allowed (`a_fixed_local_ntfs_folder_is_allowed`), and each API error is refused (`each_failed_call_refuses_with_the_fact_it_could_not_read`).
  - A real UNC string is refused before any API call, with no share needed (`a_unc_path_is_refused_before_any_call`).
  - The `Z:` case is the FAT32 fake above (Deviations).
  - Linux: `statfs` types through `check_fs_type`: NFS, CIFS and SMB are refused and local types and `9p` accepted (`network_file_systems_are_refused_and_local_ones_accepted`); FAT and exFAT are refused (`fat_and_exfat_are_refused`); the pseudo file systems are refused by name (`the_kernels_pseudo_file_systems_are_refused_by_name`); on the real system `/` is accepted and `/proc` refused (`this_systems_root_is_accepted_and_proc_is_refused`).
  - A zero-ID entry from a fake `Listing`: the folder is unreadable and its siblings are not updated (`an_entry_with_no_file_id_makes_its_folder_unreadable`, `an_entry_with_no_file_id_in_the_walk_keeps_its_siblings_unchanged`).
  - CLI: three roots, the middle one refused: the first and third are scanned (`a_refused_folder_between_two_others_does_not_stop_the_scan`).
  - The runner: a refusal is a row of the folder ledger and posts no notice, not `ScanFailed` (`a_refused_folder_is_in_the_folder_ledger_not_a_failed_scan`).
- **Step 2.** A reparse file from a fake listing keeps its row and advances `last_scan`, with no content job created (`a_reparse_file_keeps_its_row_and_gets_no_content`). A reparse folder's subtree survives `mark_missing` (`a_reparse_folder_is_not_descended_and_its_rows_are_kept`).
- **Step 3.**
  - A catalog of an earlier layout is moved aside to `catalog.sqlite.replaced` and a new one is made; the moved files are untouched (`an_earlier_layout_is_moved_aside_and_a_new_catalog_is_made`, `crates/catalog/src/lifecycle.rs`).
  - `u64` volumes above `i64::MAX` round-trip (`a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again`, `crates/catalog/src/writers.rs`).
  - NULL-safety: `created` NULL on one side with the same identity is the same row.
- **Step 4.**
  - Linux, two real devices, `/` and `/proc`: each entry's volume follows its own device, one identity per device; a bug that copies the first device to both fails this (`each_entry_carries_its_own_device_across_a_mount`, `crates/platform/src/dir.rs`).
  - A cycle from a fake listing (a child with an ancestor's `(dev, ino)`) is skipped and counted (`a_folder_already_on_the_path_is_skipped_and_counted`).
- **Step 5.** Hit with unchanged `created`: the same row. Hit with stored NULL `created` and a known scanned value: `created` is backfilled. Hit with a changed `created`: the displaced row keeps `id`, folder, name and ratings, with NULL identity and `missing = 1`; the fresh row is inserted and unpaired (`a_creation_time_fills_in_once_and_a_different_one_is_a_different_file`, `crates/catalog/src/writers.rs`).
- **Step 6** (`crates/catalog/src/pair.rs` unless named otherwise).
  - A renumbered volume rebinds every row and loses none (`a_renumbered_volume_rebinds_every_row_and_loses_none`, `crates/scan/src/lib.rs`).
  - Rename while closed: tier 2 pairs it on a complete scan, not on an interrupted one, and the next complete scan pairs the rest (`a_file_renamed_while_closed_is_paired_on_a_clean_scan_only`).
  - Tier 1 with zero candidates, and with two, binds nothing (`tier_one_binds_nothing_with_no_candidate_or_two`); several candidates count as one ambiguous row (`several_candidates_bind_nothing_and_count_as_one_ambiguous_row`).
  - Two swapped names across a volume change are not cross-bound (`swapped_names_across_a_volume_change_are_not_cross_bound`).
  - A same-inode, same-size, same-mtime file with a different name and extension does not pair under tier 2 (`tier_two_does_not_pair_a_file_of_another_extension`).
  - A missing row is never a replacement candidate: a deleted, rated photo and a new file with the same name in the same folder leave the rating on the deleted row (`a_missing_row_is_never_a_replacement_candidate`).
  - Save-over (rename-over, same name): the old row keeps its rating, resets its content, loses its `file_ratings` row, and keeps its queued rating write (`a_file_saved_over_keeps_its_row_and_rating_but_loses_its_content`).
  - Two unseen rows with the same name: nothing binds (`two_unseen_rows_with_one_name_bind_nothing`).
  - ext4 reuse under a different name: the displaced row stays a tombstone, and the new file inherits no rating (`an_inode_reused_under_another_name_leaves_a_tombstone_and_no_rating`).
  - ext4 reuse under the same name (accepted, decision 4): the new file takes the rating, and the test records that it does (`an_inode_reused_under_the_same_name_takes_the_rating`).
  - An unpaired row with a rating, rotation or analysis is not paired at any tier; `unpaired_with_data` counts it on every scan (`a_displaced_row_with_data_is_counted_and_never_paired`).
  - An unpaired row without children is cleared after a complete scan (`a_new_file_is_cleared_only_by_a_clean_scan`); only the rows in a skipped folder are left, and the rest of the root is cleared (`a_skipped_folders_rows_are_not_paired_or_cleared`; and in `crates/scan/src/lib.rs`, `a_complete_scan_with_a_reparse_folder_still_pairs_the_rest_of_the_root` and `an_unreadable_folder_is_skipped_and_the_rest_of_the_root_is_paired`).
  - A pairing whose rows fall in different batches of the walk still finds its unpaired rows, since they are in the catalog (`a_pairing_across_the_batch_boundary_still_finds_its_rows`, `crates/scan/src/lib.rs`; the batch size is a parameter of `walk_in_batches`, `BATCH` in the real walk).
  - The existing `sync_file_rating` test passes unchanged (`three_way_rating_sync`, `crates/catalog/src/writers.rs`).
- **Step 7.** A hard-link second path is skipped and counted, and the row does not flip between two scans (`a_second_path_to_a_file_is_skipped_and_counted_and_the_row_does_not_flip`); a missing row in another root does not claim the identity (`a_missing_row_in_another_root_does_not_claim_the_identity`). Both in `crates/scan/src/lib.rs`.
- **Step 8.** The wall-clock timing is recorded in `docs/PROFILING.md`, not gated.
- **Manual, WSL.** A tmpfs mounted under a scanned root scans twice with no row flipping. Done, recorded in `docs/PROFILING.md` (Measurements): 40 rows and 0 missing after each scan.
- **Manual, end to end.** The plan: build the catalog with the old binary (`main`), rate a file, run `wsl --shutdown`, and scan with the new binary; check `stat -c %d` before and after, and repeat the run if the device number did not change; the rows keep their ids, the rating is on a present row, and analysis is kept; stop `docker-desktop` first and restart it afterwards. The run across `wsl --shutdown` is recorded in `docs/PROFILING.md` (Measurements): the device number changed twice (201 to 145, then 74), and every row kept its id, rating and analysis. Both scans used HEAD's binary, and the run predates 94a2530. A run from a catalog built by an older binary has not been done.

## Verification

- `just lint` exits 0.
- `just test` with `$env:PG_TEST_DIR = '<repo>\target\pg-test'` (PowerShell syntax) exits 0.
- Linux: CI's `linux` job runs clippy and `cargo test` over the whole workspace on each commit, so the catalog, scan and platform tests pass there. Locally the same checks run in the pg-dev container (`tools/linux.py check`, `docker/README.md`). The container has no drvfs mount, so the renumbering checks above use WSL, and only to run binaries.
- **Perf.** `just perf` is callgrind (`crates/bench/benches/ir/linux.rs`). It counts instructions and does not see drvfs I/O. `tools/perf-compare.py` reads only `callgrind.*.out` files, so a wall-clock bench is not gated and needs no `--update`. The wall-clock scan timing (a 10,000-file library: first scan, unchanged rescan, renumbered rescan) is in `docs/PROFILING.md`. The catalog bench's `LAYOUT` constant (`crates/bench/benches/ir/catalog.rs`) changes with the schema; its dataset is regenerated with `bench_data.py`.

## Rollback

- Back up the catalog directory before a first run: `catalog.sqlite` with its `-wal` and `-shm`, and `thumbs.sqlite`. A rebuild moves only the catalog file and its sidecars aside (decision 3).
- Each step is a commit. Reverting a commit restores the code, not the catalog.
- Code from before step 3 does not detect a catalog made after it: its layout check did not count the new columns, so it still matched and opened the catalog silently. Its `u32` volume binds then miss the 64-bit identities, and tombstones with NULL volume are misread. Going back past step 3 therefore means restoring the moved-aside files.
- Steps 1, 2 and 4 to 8 keep the catalog readable by the code of the same commit. Steps 1 and 2 do not change the catalog layout.

## Sequencing and conflicts

- The files shared with the S-17.3/5 work were the catalog crate, `crates/cli/src/lib.rs` and `docs/REFACTOR_PLAN.md`. `crates/app/src/library.rs` was not shared.
- The plan ordered the layout bumps (S-17's 25 → 26 for `views.folder`, then 26 → 27 here). With no layout number there was nothing to order. `views.folder` is a BLOB in `BASELINE` (S-17 Units A and B, the S-17.3 row), and a catalog with a TEXT `views.folder` has its saved views converted as `carry_rows` copies them.
- S-17's decision 3 (`drives: u32` becomes `anchors: u64`) was already done (`crates/model/src/tree.rs`, the S-17.4 row), so it was not this work.

## Settled questions

- **Re-key or rebuild (decisions 1 and 3).** Rebuild: a catalog from before the change is rebuilt, and no stored identity is converted.
- **Ambiguous candidates (decision 4).** Bind nothing; report the count.
- **Reparse placeholders (decision 9).** Index from listing metadata with no content work; the rows stay present.
- **In-place rating sync (decision 8).** The existing three-way sync stays. A file that loses its rating clears the catalog's rating. Only replacement adopts the file's own rating.
- **Removable drives (decision 6).** Refused by the drive-type allow-list.
- **Same-name reuse inherits the rating (decision 4, candidates a and b).** Accepted and documented. A rename-over and an inode reuse under the same name look the same to the scanner, so the signals used are name, folder and changed identity.
- **Non-NTFS roots on Linux (decision 13).** Network, FAT, exFAT and pseudo file systems refused. Other local types and `9p` accepted.

## Not verified yet

- The trigger that replaces `/mnt/d`'s mount. The mechanism is known (each drvfs mount has its own device number). Not reproduced: within boot `229876ad`, `/mnt/d` went from 160 to 195 and `/mnt/c` from 159 to 194, and the kernel log shows no 9p event. The design does not depend on it.
- The end-to-end run from a catalog built by an older binary (Tests, Manual, end to end).
