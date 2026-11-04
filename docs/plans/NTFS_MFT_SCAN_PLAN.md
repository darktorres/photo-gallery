# Plan: scanning NTFS volumes from the MFT, kept current by the USN journal

## Context

The scanner (`crates/library/src/scan.rs`, over `pg_platform::dir::Lister`) lists every folder with
`GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`. Every rescan walks everything again.

**E: measured (2026-11-01, elevated):**

- the MFT holds 99.99 GB of valid data at 1 KiB records: ~97.6M records, in 78 extents of ~200 MB spread across
  the disk;
- in its first 2M records, 3.3% are directories, which extrapolates to **~3.2M folders**. A cold walk pays
  seeks for each one: hours;
- an unbuffered sequential read of the MFT ran at **116 MB/s** (one 4 MiB read at a time, the first 2 GB). That
  is ~14 min for the whole MFT; whether 2 reads in flight gain more is measured in step 8.

After a full read, the NTFS USN journal gives just the changes. It is active on every volume (D:, E:, F: 320 MB;
G: 954 MB).

The raw read needs an elevated volume handle. On this machine elevation is silent: the user is an administrator,
`EnableLUA = 1`, `ConsentPromptBehaviorAdmin = 0`; `Start-Process -Verb RunAs` ran without a prompt. Processes
still start with the filtered token. The user wants the most proper design, and churn is no concern. Only NTFS is
supported (docs/STORAGE.md).

Outcome: whole-volume scans at sequential speed with a real progress bar, rescans in seconds from the journal,
the same catalog rows either way. Rows are keyed by `(volume serial, NTFS file ID)`; the 64-bit file ID is the
MFT record number with the sequence number in the top 16 bits (checked: the live ID `ce00000014dd1f` is record
`14dd1f`, sequence `ce`, as stored in that record).

## Design

### Unit of work: the volume

One MFT read covers **every library root on that volume**; the journal position is per volume. The runner groups
queued roots by volume (`GetVolumePathNameW` → `GetVolumeNameForVolumeMountPointW` → `\\?\Volume{GUID}`).

Which source a scan uses:

- **The root's file system isn't NTFS (`GetVolumeInformationByHandleW`) → refuse it with a message,** when it is
  added and when it is scanned. Z: (Google Drive's virtual drive, presented as FAT32) returns `FileId == 0` for
  every entry, so today it scans silently as empty.
- **Local NTFS:**
  - stored journal position valid (same journal ID, USN ≥ `LowestValidUsn`) → **journal pass**;
  - otherwise, or a root new to the volume → **full MFT read** (elevation available), else the walker.
- **SMB shares are refused, like non-NTFS volumes** (the user agreed, 2026-11-01). A share has no MFT, no
  journal and no mount GUID. Two shares of one server volume report the same serial with overlapping trees. It
  doesn't fit volume-relative paths or the duplicate-serial rule, and supporting it properly would be a design of
  its own.
- **The walker (for a non-admin, and the differential tests) is exact, not fast:** the listing's size and mtime come from the directory index, whose copy is
  stale for a file with several links (verified), and a listing can't tell the link count. So the walker lists
  names and IDs only, then opens each media file by ID (in ID order, 8 in flight) for its live size, mtime,
  attributes and link count (`FileStandardInfo`, `FileBasicInfo`). It is the fallback, so correctness comes
  first.
- **Flags:** `gallery scan --full` forces the MFT read; `--walk` forces the walker (the differential test).

**Volume serial collisions:** a cloned disk keeps its serial, and `files.volume` would merge two drives' files.
Refuse a volume whose serial equals another mounted volume's. That's a flaw in today's identity too; none
collide here (checked).

### Two privilege levels, kept apart

- **Journal passes use the documented `FSCTL_READ_USN_JOURNAL` in the helper,** which is running anyway for
  live following. When elevation isn't available (a user who isn't an admin, or a prompt declined), they fall
  back to **`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`**. It is undocumented; BuildXL uses it. Verified on D: and E:
  from an unelevated process:
  - it works on a handle to the volume's **root directory** (`E:\`, `FILE_READ_ATTRIBUTES`,
    `FILE_FLAG_BACKUP_SEMANTICS`), and so does `FSCTL_QUERY_USN_JOURNAL`;
  - a handle to the volume device (`\\.\E:`) fails with error 1;
  - **names come back blank**, while the file ref, parent ref, reason and attributes are present;
  - the start USN must be one the journal returned: a USN mid-record fails with 87;
  - the privileged `FSCTL_READ_USN_JOURNAL` fails unelevated with 5.

  The pass is state-based and needs no names (below), so both reads feed the same code.
- **The raw MFT read runs in a separate small helper,** `pg-volume.exe`: a bin target in a new crate
  `crates/volume`, depending on `windows-sys` alone. It is not `gallery.exe`, for three reasons:
  - `cli::main` opens (and would create) the catalog before dispatching (`run_command`, `crates/cli/src/lib.rs:241`);
  - Qt and FFmpeg DLLs would load elevated (`just deploy` puts them next to `gallery.exe`);
  - an elevated UI can't receive drops from Explorer.

  The media extensions come in the request (`pg_model::media` keeps the list). An elevated CLI runs the same code
  in-process.
- **Launch:** `ShellExecuteExW` (`runas`, `SW_HIDE`).
  - **The parent's environment is not passed** (verified: a variable set in the parent was empty in the elevated
    child), so everything comes in the request.
  - A user who isn't an admin gets the walker.
  - On a machine that would prompt, the helper starts only for a scan the user asked for.
- **Pipe (verified end to end with an elevated client):**
  - the parent creates it with `FILE_FLAG_FIRST_PIPE_INSTANCE` (a second create is refused, 231),
    `PIPE_REJECT_REMOTE_CLIENTS`, a random 128-bit name, and a DACL `D:P(A;;GA;;;<user SID>)`;
  - the elevated helper gets through that DACL;
  - `GetNamedPipeServerProcessId` equals the parent's PID, and the helper checks it against `--parent`;
  - the helper exits when the pipe breaks, and it waits on the parent's handle;
  - lifetime: one helper per app session, started on first need.
- **Requests are closed:**
  - `FullRead { volume, roots, extensions }`
  - `ReadJournal { volume, journal_id, from_usn }`
  - `Follow { volume }` (live following)

  The volume must be local NTFS and the roots must be on it. The helper sends every directory under the roots
  (the catalog keeps them all) and the media files under them, and nothing else. Wire format: length-prefixed little-endian frames, 1 MiB writes. Pipe backpressure is the pause.
- **DLL planting closed:** `pg-volume.exe` links with `/DEPENDENTLOADFLAG:0x800`
  (`LOAD_LIBRARY_SEARCH_SYSTEM32`, a `build.rs` `cargo:rustc-link-arg`). Its load-time imports then come only
  from System32, never from its own folder. It loads nothing at run time.
- **Known limit, flagged:** the exe itself in a user-writable folder could be replaced. This is moot here
  (silent elevation lets any process elevate anyway), and elsewhere it needs an install to an admin-only folder:
  out of scope until there is an installer.

### The MFT read (in the helper)

- **Geometry:** `FSCTL_GET_NTFS_VOLUME_DATA`, offsets checked against live output: `BytesPerCluster` 0x2C,
  `BytesPerFileRecordSegment` 0x30, `MftValidDataLength` 0x38, `MftStartLcn` 0x40. Record 0's unnamed `$DATA`
  runs give the `$MFT`'s extents (78 on E:). The low 32 bits of `VolumeSerialNumber` equal
  `dwVolumeSerialNumber` (checked on D:, E:, F: and G:).
- **Consistency:**
  1. Query the journal → U0.
  2. `FlushFileBuffers` on a **separate, short-lived** volume handle opened `GENERIC_WRITE` (required by the call;
     verified working, 939 ms on C:). The reading handle stays read-only.
  3. Read raw.
  4. Store U0; the journal pass from U0 replays what changed during the read.

  Whether the flush writes MFT records that are dirty in the cache remains to be tested.
- **A record that can't be read is never a missing file.** That covers a fixup mismatch, a parse error, a read
  error, and a parent cycle (0 torn in E:'s first 2M records).
  - It is read again after the pass; if it still fails, its ref is resolved live (`OpenFileById`).
  - If that fails too, the record goes on the pass's **unknown** list: nothing under an unknown directory, and
    no unknown file, is marked missing or loses a link, and the scan reports them as errors.
  - The walker gets the same rule: today `mark_missing` runs after folders that couldn't be listed and hides
    their files (`scan.rs:203` counts the error, then `scan.rs:239` runs `catalog.mark_missing` anyway). That was a
    bug in the code at the time, to be fixed in step 2. *Note: it is fixed since (R-45): `scan` passes its
    `unreadable` folders to `mark_missing(root, scan, unreadable)` (`scan.rs:175`, `crates/catalog/src/lib.rs:912`),
    which leaves the files under them alone. What step 2 still adds is the walker's unknown list in the scan's
    report and the MFT read's refs.*
- **Reads:** `FILE_FLAG_NO_BUFFERING` with overlapped IO, 4 MiB sector-aligned chunks, 2 in flight, in extent
  order. Low IO priority as a per-handle hint (`SetFileInformationByHandle(FileIoPriorityHintInfo, low)`;
  verified accepted on a volume handle).
- **Parser: our own, pure safe Rust (no `unsafe`; the input is untrusted data handled by an elevated process).**
  The crates don't fit:
  - `mft` 0.9.1 gives each `MftEntry` its own `Vec<u8>`, an allocation per record. It is kept as a
    dev-dependency oracle on a captured `$MFT`;
  - `ntfs` reads every record through `Read + Seek` (`Ntfs::file(fs, record_number)`).

  Per record (layout checked by a prototype on C: and E:):
  - fixups; the in-use and directory flags; the base reference;
  - `$STANDARD_INFORMATION`: mtime at +8, attributes at +32;
  - `$FILE_NAME`: parent +0, name length +64, namespace +65, name +66. Keep POSIX (0), Win32 (1) and Win32+DOS
    (3); skip DOS (2). **`CreateHardLink` makes POSIX names**: a link was namespace 0;
  - unnamed `$DATA`: `DataSize` +0x30 if non-resident, value length if resident;
  - `$REPARSE_POINT`: the tag.

  The prototype's `$STANDARD_INFORMATION` mtime and `$DATA` size equal `GetFileInformationByHandle`'s exactly.
  Extension records (63 per 2M on E:) are merged into their base at the end; attribute lists (128 per 2M) are
  never read, since every extension record names its base.
- **Tree:** directories by record number → `(parent, sequence, name)`: ~3.2M on E:, a few hundred MB. Each
  directory's place is memoized. **Cycles** in the parent chain are detected and those entries dropped.
- **Streaming, in bounded memory:** a media file whose ancestry is resolved is sent at once, as
  `Folder { dir_ref, rel_path, root }` once per folder, then `File`. Unresolved files wait, up to a budget
  (default 512 MiB, ~40 B per file plus its name). Past it, the deferred files of a chunk are dropped and the
  chunk's number kept. After the pass the tree is complete, so only those chunks are read again, and every file
  in them resolves. Memory stays bounded whatever the layout; the second read costs only what overflowed.
- **Progress:** records read out of `MftValidDataLength / record size`, through `Control::begin`.

### One entry filter and one hard-link rule (`crates/library/src/listing.rs`)

- **Reparse points:**
  - skip name surrogates: `IsReparseTagNameSurrogate`, i.e. bit 29. That covers mount points `0xA0000003`,
    symlinks `0xA000000C` and WSL links `0xA000001D`, but not cloud `0x9000001x`, WOF `0x80000017` or dedup
    `0x80000013`;
  - keep the other reparse files, but skip ones not local (`OFFLINE`, `RECALL_ON_DATA_ACCESS`);
  - the walker takes the tag from `EaSize` (MS-FSCC FileIdBothDirectoryInformation: it holds the reparse tag
    when the reparse attribute is set);
  - this fixes `scan.rs:46` (`!e.reparse`) skipping every reparse point.
- **Hard links are modelled, not collapsed:** every name of a file under a root is a link (catalog below). A
  hard-linked photo appears in each of its folders, as in Explorer; ratings and analysis stay per file. Every
  pass, journal passes included, has a `scans` row, and a link not seen by a full pass is gone.
  - **A listing's sizes and mtimes can be wrong for hard-linked files** (verified): the `$FILE_NAME` copy of the
    other link still said size 0 and an older mtime. That's why the walker takes every value from the file
    itself (above), and the MFT read from `$STANDARD_INFORMATION` and `$DATA`.
- **`FileId == 0`** is counted in `ScanStats::errors`.

### Catalog (baseline edited in place, version 1, rebuilt on open: `crates/catalog/src/lib.rs:44–50`, `FILLED`)

- **Volume identity: the 64-bit NTFS serial.** `files.volume` and `volumes.serial` change from
  `dwVolumeSerialNumber` (32 bits) to the full serial: `FileIdInfo` gives it unelevated,
  `FSCTL_GET_NTFS_VOLUME_DATA` in the helper. `FileIdentity::volume` (`pg_model::ids`; `ScannedFile` carries a `FileIdentity`)
  becomes `u64`. Clones still copy it, so the
  duplicate refusal stays.
- **Paths relative to their volume; the drive letter is never stored.**
  - `folders.path` becomes the path inside the volume, and `roots` becomes `(volume, rel_path)`;
  - `volumes` keeps the GUID path, and where each volume is mounted now is resolved at start and on device
    arrival or removal (`GetVolumePathNamesForVolumeNameW`);
  - a full path is the current mount + `rel_path` + name, built in one place: an SQL function `pg_path(volume,
    rel_path, name)` over a process-wide mount table, registered in `query::register_functions`
    (`crates/catalog/src/query.rs:363`, called by `Catalog::prepare` and `Catalog::open`,
    `crates/catalog/src/lib.rs:830`, `:851`). The thumbnail cache's connections (`open_writer`, `open_file`,
    `crates/thumbcache/src/lib.rs:543`, `:587`) attach the catalog (`:619`) and don't call it today, so they must;
    so must the rebuild connection (`rebuild`, `lib.rs:513`), when it copies paths. It replaces the five
    concatenations `path || '\' || name`: `browse/read/details.rs:18`, `browse/read/state.rs:48`, and
    `lib.rs:878`, `:1100`, `:1149` (the test fixture's `fixture/state.rs:59` also builds one). `file_by_path`
    (`lib.rs:1052`, used by `crates/cli/src/borders.rs:181` and `crates/cli/src/lib.rs:412`) resolves a full path to
    `(volume, rel_path)` first;
  - roots by `(volume, rel_path)`: `Catalog::roots`/`add_root`/`remove_root` (`lib.rs:934–964`), `is_under`
    (`pg_model::path`, `crates/model/src/path.rs:113`) and its users (`lib.rs:951/954`,
    `crates/library/src/runner.rs:341`, `crates/library/src/crop.rs:52/60`), and the library panel in the app
    (`crates/desktop/src/library.rs`, over `crates/app/src/library.rs`, which shows them with their current letter);
  - **the folder tree's drive marks:** each node carries one bit per drive *letter* it is on (`drives: u32` in
    `crates/model/src/tree.rs:22`, one bit per row at `:52`, `:160`; the rows come from
    `crates/catalog/src/browse/read/tree.rs`). They become the node's volumes, shown by current letter, or by
    volume label when offline;
  - a volume that isn't mounted is **offline**: its files show as unavailable, not missing, and come back as
    they were when it returns, under whatever letter. In the app: the grid and viewer show cached thumbnails
    with an "offline" mark, opening the original says the drive is offline, and the import, borders, thumbnail
    and writeback passes skip it (writes stay queued);
  - this fits the merged tree: `pg_model::path::merged_path` (`crates/model/src/path.rs:10`) already drops the
    drive letter, so folder keys and `order_key` don't change. Nothing else stores letters: `session` keeps a file
    ID and a merged folder path, `views.folder` a merged path (`crates/catalog/src/lib.rs:263`, the baseline's
    comment).
- **Folders are a tree:** `parent_id` + `name` are the truth; `rel_path` is derived and kept indexed for
  `under_root!` range queries; `move_folder` changes one `parent_id`/`name`, then rewrites the derived paths and
  `order_key`s under it in one statement each.
- **`folders`: a row for every directory under a root,** not only those holding media (today `scan.rs:147`):
  `volume`, `file_ref`, `parent_id`, `UNIQUE(volume, file_ref)`.
  - A rename or move is always found by ref → **`Catalog::move_folder(old, new)`**, which rewrites the paths
    under it and its links' `order_key` (`pg_order_key`);
  - uniqueness is `UNIQUE(parent_id, name)` (and the derived `UNIQUE(volume, rel_path)`); a stale row already at
    the new place is merged first.
  - The tree in the UI is unaffected: its counts come from links (today `FOLDER_COUNTS`,
    `crates/catalog/src/browse/read/tree.rs:10`, from `files`), so folders without media still don't show.
- **Identity and location split:**
  - `files` keeps identity (`volume`, `file_ref`), `size`, `mtime`, `kind`, `missing`, `pending`, `seq` and the
    per-file data (analysis, ratings, writes);
  - a new **`links(id, file_id, folder_id, name, ext, order_key, last_scan)`**, `UNIQUE(folder_id, name)`, takes
    `folder_id`, `name`, `ext` and `order_key` off `files`;
  - `files_order` and `files_present` move to `links`. The grid, the folder counts and the order list links; a
    file reads and writes through any present link (`crates/library/src/writeback.rs`, the import's paths).
  - Everything that reads `files.folder_id`/`name`/`order_key` moves to `links`: about 30 uses of `folder_id` in
    `crates/catalog/src/lib.rs`, `browse/read/` and `fixture/`, and in `crates/thumbcache/src/lib.rs`; the name
    filter `pg_contains(f.name, ?)` (`crates/catalog/src/query.rs:289`) and the name order `f.order_key` in
    `query.rs`; and the tests that query them (`crates/library/tests/video.rs`, `writeback.rs`). `rebuild` gets a
    `FILLED`-style copy from old catalogs: one link per old row.
  - A file is missing when it has no present link.
- **A content generation, not mtime, says whether derived data is current.** `stale!` (analysis: size, mtime,
  rotation, kind) and the thumbnail cache's `Source` (mtime, rotation: `crates/thumbcache/src/lib.rs:46–51`,
  checked at `:665–666`) both trust mtime. A rewrite that keeps the mtime, which the journal now detects, would leave the
  analysis and the cached thumbnails current forever.
  - **`files.content_gen`** goes up on a change of size or mtime, and on a journal data-change reason that isn't
    ours;
  - `analysis` records the `content_gen` it was made from, and `stale!` compares it (with rotation and kind);
  - the thumbnail cache's `Source` becomes `(content_gen, rotation)`. The cache schema is edited in place, at
    version 1;
  - writeback's own writes bump it too, and `retag` (`crates/thumbcache/src/lib.rs:426`) moves the thumbnails from the
    old generation to the new one, as it moves them between mtimes today;
  - the import's check that a file didn't change while it was read (`AnalysisWriter`'s `current`) compares
    `content_gen`.
- **New `volumes(serial PRIMARY KEY, guid_path, journal_id, next_usn, synced)`;
  `Catalog::mark_missing_ref(volume, file_ref)`.**
- `mark_missing(root, scan, unreadable)` (`crates/catalog/src/lib.rs:912`) becomes: links under `root` that this
  scan didn't see are deleted, except under the pass's unknown directories; then files left without a present link are marked
  missing. In an MFT read each root gets its own scan ID.
- `ScanWriter` is rewritten: it writes files and links, records folders by ref (moving them), and takes the
  unknown list. Every source writes through it, unelevated.

### Journal passes (state-based)

1. Read from the saved USN (in the helper, or unprivileged as the fallback); coalesce by file ref.
2. Keep a ref if it is in the catalog (`files` or `folders`), or it is now a directory, or its live name is a
   media name. A `.jpg` renamed to `.txt` is in the catalog, so it is kept and marked missing.
3. Resolve each ref unelevated with `OpenFileById` on the root's handle (verified):
   - a wrong sequence number, older or newer, fails with 87, so a reused record can't pass for the old file;
   - `FileNameInfo` gives one link's path, without the drive;
   - `FindFirstFileNameW`/`FindNextFileNameW` list every link (both found);
   - `FileStandardInfo` and `FileBasicInfo` give size, mtime and attributes.
4. **Directories first, shallowest old path first:**
   - moved under a root → `move_folder`;
   - moved out of the roots or deleted → delete the links under the old path (files left without one go missing);
   - new, or moved in → walk it.
5. **Then files:** upsert the file and its current links (every name from `FindFirstFileNameW`), or
   `mark_missing_ref`. Store the next USN returned by the read.
   - **Content changes come from the journal, not only from size and mtime.** A tool that rewrites a file and
     restores its mtime, at the same size, is invisible to `(size, mtime)`. A ref whose coalesced reasons
     include `DATA_OVERWRITE`, `DATA_EXTEND` or `DATA_TRUNCATION` bumps its `content_gen` (catalog above).
     That makes it pending, and gives it a new `seq` for the thumbnails, whatever its size and mtime.
   - **Our own writes don't count:** after writeback writes a file, it reads the file's latest USN
     (`FSCTL_READ_FILE_USN_DATA` on its handle, verified unelevated) and records it as `files.own_usn` through
     `file_rewritten` (`crates/catalog/src/writes.rs:306`), which both write paths already go through (`:211`, `:216`, `:282`). A journal
     record for that file at or below `own_usn` is ours, and is ignored. Otherwise every rating write would
     re-import the file.
6. **Journal unusable** (entry deleted, ID changed, inactive) → a full read.

### The runner: a full read isn't interrupted by unrelated work

Today any request stops the running pass (`crates/library/src/runner.rs:219–221`). A non-resumable ~14 min read would
restart each
time. Requests that only add work queue behind a running volume read; only a removed root, quitting or the
user's stop end it.

### Live following (step 7)

The helper keeps a waiting journal read per volume (`BytesToWaitFor`) and tells the job a volume changed; the runner debounces and runs a journal
pass. The rescan button becomes a fallback; "full rescan" forces the MFT read.

## Steps (each verified and committed on its own)

1. **Catalog model:**
   - the `files`/`links` split, with every reader moved to it (`crates/catalog`, `crates/thumbcache`, `dups.rs`,
     `writeback.rs`, the import);
   - the 64-bit volume serial;
   - `content_gen` in `files`, `analysis` and the thumbnail cache;
   - volume-relative paths, roots by volume, mounts resolved at run time, offline volumes;
   - folders as a tree (`parent_id` + `name`, derived `rel_path`) for every directory, with `volume/file_ref`,
     and `move_folder` with the merge.

   Committed as separate steps (links; serial; `content_gen`; relative paths and offline; folder tree), each with
   its tests. It
   touches `crates/catalog/src/browse/`, which the other session is changing, so it needs that work committed first and the
   user's go-ahead. Tests: catalog unit tests (links, missing, `move_folder`, `rebuild` from an old catalog),
   the existing suites, and the app's grid and folder tree by screenshot (`run` skill).
2. **`listing.rs`** (the filter, `FileId == 0`, the unknown list); refusing non-NTFS roots, shares and duplicate
   serials. The walker moves to `walk.rs`, records links, takes live values by ID, and no longer hides the files
   of folders it couldn't list. Tests: the existing `crates/library/tests/*.rs`, plus hard-link, reparse and
   unreadable-folder cases (a folder with a deny-list ACL).
3. **`crates/volume`: the parser (`record.rs`, `runs.rs`).** Tests:
   - records from a builder with fixups, plus real records captured in step 4;
   - a mutation test: errors, never panics, hangs or cycles;
   - the `mft` crate as an oracle on a captured `$MFT`, as a dev-dependency. `just lint` runs `cargo deny`
     (licenses, duplicates; "a new license is a decision", `deny.toml`) and `cargo machete`. Whatever it pulls
     in must pass them, or the oracle is replaced by records cross-checked with `GetFileInformationByHandle`.
4. **The MFT reader (`mft.rs`)** in-process when elevated, with `--full/--walk`. Differential test on the test
   VHD, then on the real drives.
5. **`pg-volume.exe` and the pipe.** `crates/library/src/runner.rs`: volume grouping, source choice, uninterrupted reads. `crates/cli/src/lib.rs`:
   the flags.
6. **Journal passes** (privileged read in the helper, unprivileged fallback); `volumes`; the replay after a
   full read.
7. **Live following.**
8. **Measure once, at the end:**
   - on E:, a cold walk vs a warm walk vs the MFT read (1 vs 2 in flight) vs a journal pass, and the helper's
     peak memory;
   - results in FINDINGS "I/O", STORAGE.md, PLAN M1 and `crates/README.md`.

   Also fix STORAGE.md: the Z: finding (FileId 0, silently empty), and "only NTFS, others refused".

## Critical files

- **Changed:**
  - `crates/library/src/scan.rs` (the orchestrator), `runner.rs`, and `crates/cli/src/lib.rs`;
  - `crates/catalog/src/lib.rs` (schema, `ScanWriter`, `move_folder`, `mark_missing_ref`, `volumes`,
    `pg_path`);
  - `crates/catalog/src/browse/`, `crates/catalog/src/query.rs`, `crates/thumbcache/src/lib.rs` (links, `pg_path`,
    drive marks, `content_gen`), and `crates/model/src/tree.rs` (the drive marks);
  - `crates/library/src/{import.rs, dups.rs, crop.rs, borders.rs, writeback.rs}` (links, paths, roots,
    `own_usn`);
  - `crates/desktop/src/library.rs` and `crates/app/src/library.rs` (roots by volume), plus the offline mark in the
    grid and viewer (`crates/desktop/src/gallery.rs`, `providers.rs`, QML);
  - `Cargo.toml` (member `crates/volume`; windows-sys `Win32_System_Ioctl`, `Win32_System_Pipes`,
    `Win32_UI_Shell`, `Win32_Security_Authorization`).
- **New:**
  - `crates/volume/src/{lib.rs, record.rs, runs.rs, mft.rs, usn.rs, protocol.rs, bin/pg-volume.rs}`;
  - `crates/library/src/{listing.rs, walk.rs, helper.rs}`.
- **Reused:**
  - the catalog crate's (`crates/catalog/src/lib.rs`) statement style (constants, `concat!` macros) and
    `rebuild`/`FILLED` for the schema change;
  - `pg_platform::verbatim::verbatim_wide` (`crates/platform/src/verbatim.rs:26`);
  - `Ext::of_name_utf16` (`crates/model/src/media.rs:55`);
  - `Control` (`crates/jobs/src/control.rs`), `normalize_path` (`crates/library/src/scan.rs:70`), `pg_order_key`.
- **The prototypes behind these checks** (C# through PowerShell, in the session scratchpad):
  - `C:\Users\Torres\AppData\Local\Temp\claude\C--Users-Torres-Documents-GitHub-photo-gallery\b0042dab-511d-417c-bafa-1a31bf7c518c\scratchpad\elev.ps1`
    (raw MFT);
  - `C:\Users\Torres\AppData\Local\Temp\claude\C--Users-Torres-Documents-GitHub-photo-gallery\b0042dab-511d-417c-bafa-1a31bf7c518c\scratchpad\pipe_client.ps1`
    (pipe).

## Verification

- **Test VHD (elevated, made by the test with `diskpart`, detached at the end).** It holds:
  - paths over 260 characters and non-BMP names;
  - hard links across folders (and enough of them for an `$ATTRIBUTE_LIST`);
  - a fragmented file whose `$DATA` spills into extension records;
  - junctions, symlinks and a mount point (skipped); WOF and sparse files (kept);
  - a file being written; a file created and closed just before the read (the flush test).
- **Differential:** on the VHD and on D:, E:, F: and G: (scratch catalogs, `PG_CATALOG_DIR`), `--walk` and
  `--full` must agree exactly on files, links, sizes and mtimes. The walker takes live values, so any mismatch
  is a bug in one of them, settled against the live file.
- **Journal, each compared to a fresh walk:**
  - create, rename, move, rewrite, delete;
  - rename to a non-media name;
  - nested folder renames; a folder out of the root and back;
  - a renamed folder without direct media (found by ref, no walk);
  - add and remove a hard link;
  - record reuse;
  - journal wrap, falling back to a full read.
- **Runner:** changing the thumbnail height during a full read doesn't restart it.
- **Volumes:** detach the test VHD → its files show offline, not missing; reattach it under another letter → the
  same rows, ratings and analysis, with no rescan and nothing marked moved.
- **Bounded memory:** a full read of the VHD with the deferral budget set to a few KiB takes the re-read path
  and gives the same catalog as an unbounded read.
- **Exact walker:** after editing a file through its second link, `--walk` records the new size and mtime.
- **Content change without mtime:** rewrite a file at the same size, restore its mtime → the journal pass makes it
  pending, and its thumbnails are made again (not served stale). A rating written by the app → not pending
  again, and its thumbnails kept.
- **Unknowns:** a folder the walker can't list (deny ACL) and an MFT record corrupted in a VHD image read
  offline: their files stay present, and the scan reports the error.
- **Existing suites:** `cargo test --workspace` and `just lint`.
- **Builds:** another session is committing in `scan.rs`, `catalog.rs` and `runner.rs`. Ask the user before
  each build phase.

## Validation record (2026-11-01)

**Verified, by experiment on this machine (prototypes above) or by Microsoft's docs:**

| Claim | Evidence |
|---|---|
| Silent elevation; `runas` doesn't pass the environment | registry; elevated runs without a prompt; a variable empty in the child |
| E:: 1 KiB records, 99.99 GB of MFT, 78 extents; journals active everywhere | `fsutil fsinfo ntfsinfo`, `fsutil usn queryjournal` (elevated) |
| Raw read at 116 MB/s, one read at a time; 3.3% directories, 0 torn, 128 lists, 63 extensions per 2M records | prototype sweep of E:'s first 2 GB |
| SI mtime and `$DATA` size equal the live values; record layout offsets; record and sequence = file ID | prototype on C: |
| Hard links: POSIX names; the other link's `$FILE_NAME` copy is stale (size 0) | prototype on C: |
| Serial = low 32 bits of the NTFS serial | `fsutil` vs `vol`, 4 volumes |
| Unprivileged journal: root-directory handle, names blank, parent/reason/attributes present, boundary USN required | prototype on D: and E: |
| `OpenFileById` rejects a wrong sequence (87); `FileNameInfo` one link; `FindFirstFileNameW` all links | prototype on C: |
| `FileIdInfo` gives the 64-bit serial (`2cac2355ac2318ba`, low half = `vol`) and the 128-bit ID unelevated; `FSCTL_READ_FILE_USN_DATA` works unelevated | prototype on C: |
| Pipe: first-instance refusal, user-SID DACL passes the elevated client, server PID check | prototype, both sides |
| Volume flush works (`GENERIC_WRITE`, admin); IO priority hint accepted on a volume handle | prototype on C:; docs |
| `EaSize` holds the reparse tag; `FileId` 0 without 64-bit IDs; ReFS -1 | MS-FSCC |
| Z: (Google Drive's FAT32 drive) gives `FileId == 0` for all entries | prototype listing |
| `mft` allocates per entry; `ntfs` reads per record | docs.rs |
| No duplicate serials among the mounted volumes | `Get-CimInstance Win32_Volume` |

**From our code (revalidated at `4e1ad4d`, 2026-11-02, after the other session's commits).** The anchors below are
those of `4e1ad4d`, before the crate split; where the code is now: `cli.rs` is `crates/cli/src/lib.rs`, `scan.rs` and
`runner.rs` are in `crates/library/src/`, `catalog.rs` is `crates/catalog/src/lib.rs`, `browse.rs` is
`crates/catalog/src/browse/`, `thumbs.rs` is `crates/thumbcache/src/lib.rs`. The sections above give the current
lines. At `e669f2d` the `mark_missing` item (R-45) is fixed; the rest still holds.

- catalog opened first (`cli.rs:151–153`);
- every request stops the pass (`runner.rs:150–154`);
- folder rows only with media (`scan.rs:216`): replaced by a row per directory;
- all reparse points skipped (`scan.rs:128`);
- the errors of listing a folder are counted (`scan.rs:203`), and `mark_missing` still runs (`:239`);
- full paths concatenated in six places (five now), and the drive-letter bits in the folder tree (`browse.rs:142`);
- thumbnail and analysis staleness trust mtime (`thumbs.rs:146–150`, `:658–659`; `stale!`);
- writeback's temp is `.<name>.pg-write`, not a media name (`writeback.rs:396`, now `:333`), and both writes record
  through `file_rewritten`;
- `session` and `views` keep merged paths and file IDs, never drive letters;
- every other anchor in the plan checked and current: `cli.rs:151–153`, `runner.rs:150–154`,
  `catalog.rs:32–33/69/338/741/989`, `browse.rs:18/164`, `media.rs:61`, `paths.rs:22`, `control.rs:112`,
  `main.rs:6`, the workspace members and the `windows-sys` features in `Cargo.toml`, the `Justfile`'s `lint`
  and `deploy`.

**Still unverified, and where each gets settled:**

| Claim | Where |
|---|---|
| A volume flush writes MFT records dirty in the cache | VHD test: a file created just before the read |
| `$STANDARD_INFORMATION` carries `OFFLINE`/`RECALL_ON_DATA_ACCESS`; OneDrive files are reparse points even when local | not testable here: OneDrive has 4 files, none a reparse point. Stays flagged |
| Throughput with 2 reads in flight; warm-walk time; the helper's peak memory | step 7 |
| FAT's DST shift and unstable FAT IDs (STORAGE.md) | not tested; unsupported file systems, kept as stated |
| WOF and dedup sizes from `$DATA` equal the logical size | VHD test (WOF); dedup not available on client Windows |
