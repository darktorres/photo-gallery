# Plan: scanning NTFS volumes from the MFT, kept current by the USN journal

## Context

The scanner (`crates/scan/src/lib.rs`, over `pg_platform::dir::Lister`) lists every folder with
`GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`. Every rescan walks everything again.

**E: measured (2026-11-01, elevated):**

- the MFT holds 99.99 GB of valid data at 1 KiB records: ~97.6M records, in 78 extents (about 1.3 GB each on
  average), spread across the disk;
- in its first 2M records, 3.3% are directories, which extrapolates to **~3.2M folders**. A cold walk pays a seek
  for each one: hours;
- an unbuffered sequential read of the MFT ran at **116 MB/s** (one 4 MiB read at a time, the first 2 GB): ~14 min
  for the whole MFT. Whether 2 reads in flight gain more is measured in step 8.

After a full read, the NTFS USN journal gives just the changes. It is active on the NTFS volumes checked (C:, D:, E:,
F: 320 MB; G: 954 MB). An unlettered NTFS volume was not checked.

The raw read needs an elevated volume handle. On this machine elevation is silent: the account is an administrator,
`EnableLUA = 1`, `ConsentPromptBehaviorAdmin = 0`; `Start-Process -Verb RunAs` ran without a prompt. Processes still
start with the filtered token. Only NTFS is supported (`docs/reference/STORAGE.md`).

Outcome: whole-volume scans at sequential speed with a real progress bar, rescans in seconds from the journal, the
same catalog rows either way. Rows are keyed by `(volume serial, NTFS file ID)`. The 64-bit file ID is the MFT record
number with the sequence number in the top 16 bits (checked: the live ID `ce00000014dd1f` is record `14dd1f`, sequence
`ce`, as stored in that record).

## Status

As of 2026-11-14 (`493116c`; the Design and Steps are the plan):

- **Done:** the 64-bit volume serial is read and carried as `FileIdentity::volume: u64` (`crates/platform/src/dir.rs:299–311`,
  `crates/model/src/ids.rs:37`). Non-NTFS roots are refused by `check_root` (`crates/scan/src/refusal.rs:14`, which calls
  `check_ntfs` on Windows), reached through `pg_scan::check_folder` (`crates/scan/src/lib.rs:312`, which calls
  `check_root` at `:313`), when a folder is added (`crates/library/src/folders.rs:31`) and when it is scanned
  (`crates/scan/src/lib.rs:321`). UNC shares are refused as `NetworkPath` (`refusal.rs:42`). An entry with no file ID
  makes its folder unreadable (`crates/scan/src/lib.rs:201`).
- **Not in the tree:** the `links`, `volumes` and `content_gen` tables; volume-relative paths (`folders.path` still
  holds full paths); `move_folder`, `mark_missing_ref` and `own_usn`; `crates/volume`, `pg-volume.exe`, the MFT reader
  and any USN journal call; `crates/library/src/{listing,walk,helper}.rs`. The entry filter and the walker still live
  in `crates/scan/src/lib.rs`.
- **Anchors:** as of `493116c`, except the writeback lines (`:502`–`:642`), which are the uncommitted working tree's
  (HEAD: `:499`–`:639`). The `4e1ad4d` anchors are history (see the Validation record).

## Design

### Unit of work: the volume

One MFT read covers **every library root on that volume**; the journal position is per volume. The runner groups
queued roots by volume (`GetVolumePathNameW` → `GetVolumeNameForVolumeMountPointW` → `\\?\Volume{GUID}`).

Which source a scan uses:

- **Non-NTFS roots are refused** (`GetVolumeInformationByHandleW`), when added and when scanned, by file system type.
  Z: (Google Drive's virtual drive, presented as FAT32) gave `FileId == 0` for every entry when first measured; the
  2026-11-14 re-measurement found nonzero IDs (see the Validation record). The refusal stands because it is by file
  system type. Done (see Status).
- **Local NTFS:**
  - a stored journal position that is valid (same journal ID, USN ≥ `LowestValidUsn`) → **journal pass**;
  - otherwise, or a root new to the volume → **full MFT read** (elevation available), else the walker.
  - Cost: a full read covers the whole volume, so it wins for drive roots and large roots; a small folder is cheaper to
    walk. Rescans are where the MFT read matters most, since the journal then gives only the changes.
- **SMB shares are refused** (decided 2026-11-01). A share has no MFT, no journal and no mount GUID. Two shares of one
  server volume report the same serial with overlapping trees. It doesn't fit volume-relative paths or the
  duplicate-serial rule, and supporting it properly would be a design of its own.
- **The walker** (for a non-admin, and for the differential tests) is exact, not fast. A listing's size and mtime come
  from the directory index, whose copy is stale for a file with several links (verified), and a listing can't tell the
  link count. So the walker lists names and IDs only, then opens each media file by ID (in ID order, 8 in flight) for
  its live size, mtime, attributes and link count (`FileStandardInfo`, `FileBasicInfo`). It is the fallback, so
  correctness comes first.
- **Flags:** `gallery scan --full` forces the MFT read; `--walk` forces the walker (the differential test).

**Volume serial collisions:** a cloned disk keeps its serial, and `files.volume` would merge two drives' files. Refuse
a volume whose serial equals another mounted volume's. This is a flaw in today's identity too; none collide here
(checked).

### Privilege levels and the helper

**Journal reads.**

- Live following (step 7) and step 6's passes use the documented `FSCTL_READ_USN_JOURNAL` in the helper.
- Without elevation (a user who isn't an admin, or a prompt declined), the parent falls back to
  **`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`**. It is not on Microsoft Learn; it is declared in the SDK's
  `winioctl.h:11446` (10.0.26100.0, `CTL_CODE(9, 234, METHOD_NEITHER, ANY_ACCESS)` = `0x903AB`), and BuildXL uses it.
  Verified from an unelevated process (a prototype, not kept in the repo), on D: and E:
  - it works on a handle to the volume's **root directory** (`E:\`, `FILE_READ_ATTRIBUTES`,
    `FILE_FLAG_BACKUP_SEMANTICS`), and so does `FSCTL_QUERY_USN_JOURNAL`;
  - a handle to the volume device (`\\.\E:`) fails with error 1;
  - **names come back blank**; the file ref, parent ref, reason and attributes are present;
  - the start USN must be one the journal returned: a USN mid-record fails with 87;
  - the privileged `FSCTL_READ_USN_JOURNAL` fails unelevated with 5.

  The pass is state-based and needs no names (see Journal passes), so both reads feed the same code.

**The raw MFT read runs in a separate helper,** `pg-volume.exe`: a bin target in a new crate `crates/volume`, depending
on `windows-sys` alone (no `pg-*` crate; the progress count is a plain counter the caller reads). It is not
`gallery.exe`, for three reasons:

- `run_command` (`crates/cli/src/lib.rs:131`) opens, and would create, the catalog (`:136`) before dispatching;
- Qt and FFmpeg DLLs would load elevated (`just deploy` puts them next to `gallery.exe`);
- an elevated UI can't receive drops from Explorer.

The media extensions come in the request (`pg_model::media` keeps the list). An elevated CLI runs the same code
in-process.

**Launch:** `ShellExecuteExW` (`runas`, `SW_HIDE`).

- The parent's environment is not passed (verified: a variable set in the parent was empty in the elevated child), so
  everything comes in the request.
- A user who isn't an admin gets the walker for a full scan. A rescan with a valid journal position still gets a
  journal pass, run unprivileged by the parent, since no helper starts (inferred from the rules above; confirm before
  step 6).
- On a machine that would prompt, the helper starts only for a scan the user asked for. Live following uses the
  unprivileged read, so it never prompts.

**Pipe (prototype, not in the tree):**

- The parent creates it with `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, `nMaxInstances` 1, a
  random 128-bit name, and a DACL granting `FILE_GENERIC_READ | FILE_GENERIC_WRITE` to `<user SID>`, without
  `WRITE_DAC`.
- Measured 2026-11-14 (`PipeFirstProbe2.cs` and `PipeFirstProbe.cs` in the session scratchpad, not in the repo):
  - a second create with the flag, instance limit not reached: `ERROR_ACCESS_DENIED` (5);
  - a second create with the limit reached: 231 (`ERROR_PIPE_BUSY`, the instance limit);
  - limit 1 without the flag: 231 (`PipeFirstProbe.cs`);
  - without the flag, a same-name create succeeds as another instance up to the limit;
  - a squatter created without the flag, then a flagged create at limit 2: refused with 5 (case C2).
- At limit 1, the design's limit, the limit alone refuses a second create. The flag refuses it whatever the limit, so
  it is a backstop.
- The DACL keeps other accounts out. A process running as the same user reaches the pipe whatever the DACL says, so the
  PID checks below are what refuse it.
- The elevated helper gets through the DACL when it runs as the same account (true here; a different admin account's
  elevation would be refused).
- Each side checks the other: the helper checks `GetNamedPipeServerProcessId` against `--parent`, and the parent checks
  `GetNamedPipeClientProcessId` against the child's PID from `ShellExecuteExW` (`GetProcessId` on its handle). A
  connection from any other process is refused: the parent disconnects it and listens again, within a bounded connect
  wait. A declined UAC prompt or a helper that never starts ends the wait.
- The helper exits when the pipe breaks, and it waits on the parent's handle.
- Lifetime: one helper per app session, started on first need. Each launch gets its own pipe (a new random name and a
  new first instance), so no instance outlives its helper. The helper connects as a client, which needs no
  instance-creation right.

**Requests are closed:** `FullRead { volume, roots, extensions }`, `ReadJournal { volume, journal_id, from_usn }`,
`Follow { volume }` (live following). The volume must be local NTFS and the roots must be on it.

The helper sends every directory under the roots (the catalog keeps them all) and the media files under them, and
nothing else. Wire format: length-prefixed little-endian frames, 1 MiB writes; pipe backpressure is the pause. Each file
frame carries its reparse tag (read from `$REPARSE_POINT`, step 4). The parent applies the same filter as the walker
(`listing.rs`, step 2), so the MFT and walk paths agree; the helper applies no filter of its own.

**DLL planting is closed:** `pg-volume.exe` links with `/DEPENDENTLOADFLAG:0x800` (`LOAD_LIBRARY_SEARCH_SYSTEM32`;
set in `build.rs` as `cargo:rustc-link-arg-bin=pg-volume=/DEPENDENTLOADFLAG:0x800`, so no other target gets it). Its
static imports resolve only from System32, never from its own folder. The flag does not restrict run-time
`LoadLibrary` calls, so the helper must make none.

**Known limit, flagged:** the exe itself in a user-writable folder could be replaced. On this machine, while silent
elevation is on, any process running as this administrator account can elevate without a prompt, so a replaced exe
adds nothing here. On a machine that prompts, it is live. Elsewhere it needs an install to an admin-only folder: out of
scope until there is an installer.

### The MFT read (in the helper)

- **Geometry:** `FSCTL_GET_NTFS_VOLUME_DATA`, offsets checked against live output: `BytesPerCluster` 0x2C,
  `BytesPerFileRecordSegment` 0x30, `MftValidDataLength` 0x38, `MftStartLcn` 0x40. Record 0's unnamed `$DATA` runs give
  the `$MFT`'s extents (78 on E:). The low 32 bits of `VolumeSerialNumber` equal `dwVolumeSerialNumber` (checked on D:,
  E:, F: and G:).
- **Consistency:**
  1. Query the journal → U0.
  2. `FlushFileBuffers` on a **separate, short-lived** volume handle opened `GENERIC_WRITE` (required by the call;
     verified working with a prototype, 939 ms on C:, one run, not kept in the repo). The reading handle stays
     read-only.
  3. Read raw.
  4. Store U0 in `volumes`; the journal pass from U0 replays what changed during the read.

  The flush writes MFT records that are dirty in the cache: tested 2026-11-14 over a 160 ms window (see the Validation
  record). A longer-wait control is step 4's test.
- **A record that can't be read is never a missing file.** That covers a fixup mismatch, a parse error, a read error and
  a parent cycle (0 torn in E:'s first 2M records).
  - It is read again after the pass; if it still fails, its ref is resolved live (`OpenFileById`).
  - If that fails too, the record goes on the pass's **unknown** list: nothing under an unknown directory, and no
    unknown file, is marked missing or loses a link, and the scan reports them as errors.
  - The walker has the same rule. At `4e1ad4d`, `mark_missing` ran after folders that couldn't be listed and hid their
    files (`crates/ingest/src/scan.rs:203` counts the error, then `:239` runs `catalog.mark_missing` anyway). That bug is
    fixed: `scan` passes its skipped folders to `mark_missing(root, scan, skipped)` (`crates/scan/src/lib.rs:591`,
    defined at `crates/catalog/src/roots.rs:150`), which leaves the files under unreadable folders alone. Step 2 adds
    the walker's unknown list to the scan's report and the MFT read's refs.
- **An aborted read sweeps nothing.** A pass that is stopped, fails, or loses its helper (a crash or a broken pipe)
  mid-stream commits no missing marks and no link removals. `mark_missing` runs only at the end of a complete pass, as
  today (`crates/scan/src/lib.rs` module doc, HEAD `:36–37`; `crates/catalog/src/roots.rs:27`: an incomplete walk sweeps
  nothing). Partial rows may be kept, since they are true. The journal position is not stored, so the next pass replays
  it.
- **Reads:** `FILE_FLAG_NO_BUFFERING` with overlapped IO, 4 MiB sector-aligned chunks, 2 in flight (a choice step 8
  checks against one in flight, the measured 116 MB/s baseline), in extent order. Low IO priority as a per-handle hint
  (`SetFileInformationByHandle(FileIoPriorityHintInfo, low)`; verified accepted on a volume handle).
- **Parser: our own, pure safe Rust** (no `unsafe`; the input is untrusted data handled by an elevated process). The
  crates don't fit:
  - `mft` 0.9.1 gives each `MftEntry` its own `Vec<u8>`, an allocation per record. It is kept as a dev-dependency oracle
    on a captured `$MFT`;
  - `ntfs` reads every record through `Read + Seek` (`Ntfs::file(fs, record_number)`).

  Per record (layout checked by a prototype on C: and E:):
  - fixups; the in-use and directory flags; the base reference;
  - `$STANDARD_INFORMATION`: mtime at +8, attributes at +32;
  - `$FILE_NAME`: parent +0, name length +64, namespace +65, name +66. Keep POSIX (0), Win32 (1) and Win32+DOS (3); skip
    DOS (2). `CreateHardLink` made POSIX names in the measurement: a link was namespace 0;
  - unnamed `$DATA`: `DataSize` +0x30 if non-resident, the value length if resident;
  - `$REPARSE_POINT`: the tag.

  The prototype's `$STANDARD_INFORMATION` mtime and `$DATA` size equal `GetFileInformationByHandle`'s exactly. Extension
  records (63 per 2M on E:) are merged into their base at the end. Attribute lists (128 per 2M) are never read, since
  every extension record names its base.
- **Tree:** directories by record number → `(parent, sequence, name)`: ~3.2M on E:, a few hundred MB. Each directory's
  place is memoized. **Cycles** in the parent chain are detected; those records go on the unknown list, not dropped, so
  nothing under them is marked missing.
- **Streaming, in bounded memory:** a media file whose ancestry is resolved is sent at once, as
  `Folder { dir_ref, rel_path, root }` once per folder, then `File`. Unresolved files wait, up to a budget (default
  512 MiB, ~40 B per file plus its name). Past it, the deferred files of a chunk are dropped and the chunk's number is
  kept. After the pass the tree is complete, so only those chunks are read again, and every file in them resolves. The
  deferred files stay within the budget whatever the layout; the second read costs only what overflowed. The directory
  tree (a few hundred MB on E:) grows with the directories and sits outside that budget.
- **Progress:** the reader reports records read against `MftValidDataLength / record size`, and `pg-library` shows them
  through `Control::begin` (`crates/jobs/src/control.rs:219`).

### Entry filter, reparse points and hard links

The filter moves to a new file, `crates/library/src/listing.rs` (today it is in `crates/scan/src/lib.rs`, `:195–229`).

- **Reparse points:**
  - Skip name surrogates: `IsReparseTagNameSurrogate`, bit 29. That covers mount points `0xA0000003`, symlinks
    `0xA000000C` and WSL links `0xA000001D`.
  - The cloud tags (`0x9000001A`, with the provider index in bits 12–15), WOF `0x80000017` and dedup `0x80000013` are
    not surrogates, so this rule keeps them. The cloud rule (below) excludes the cloud tags.
  - Keep the other reparse files, but skip ones not local (`OFFLINE`, `RECALL_ON_DATA_ACCESS`). That is not in the tree.
  - The tag is **not** read yet. The walker takes only the reparse attribute (`crates/platform/src/dir.rs:398`). A WOF
    file shows no reparse attribute to directory enumeration or `GetFileAttributes` (checked on `notepad.exe`), so the
    listing can't see WOF. `fsutil file layout` still shows the attribute and a 24-byte `$REPARSE_POINT` stream, and the
    tag's value is read from the MFT record. MS-FSCC `FileIdBothDirectoryInformation` carries the tag in `EaSize` when
    the reparse attribute is set; nothing reads it.
  - The reparse branch is `crates/scan/src/lib.rs:502–514` (checked 2026-11-14): a reparse folder is not descended, and
    a reparse file gets a row only if it already has one, so a new placeholder is not added. The name-surrogate filter
    is not in the tree either.
- **Hard links are modelled, not collapsed.** Every name of a file under a root is a link (see Catalog). A hard-linked
  photo appears in each of its folders, as in Explorer; ratings and analysis stay per file. Every pass, journal passes
  included, has a `scans` row, and a link not seen by a full pass is gone.
  - A listing's sizes and mtimes can be wrong for hard-linked files (verified): the `$FILE_NAME` copy of the other link
    still said size 0 and an older mtime. So step 2's walker takes every value from the file itself, and the MFT read
    takes them from `$STANDARD_INFORMATION` and `$DATA`. Today the walker still takes size and mtime from the listing
    (`crates/platform/src/dir.rs:394`, `:401`); step 2 changes that.
- **`FileId == 0`** is counted in `ScanStats::errors`. In the tree, the folder is treated as unreadable instead
  (`crates/scan/src/lib.rs:201–226`).

### Cloud folders (decided 2026-11-13)

**Decision: cloud folders are excluded from the library.** Cloud entries are recognised by the reparse tag, not by their
path, except for the OneDrive folder itself, which the tag can't see.

- **Classification by the tag.** The listing reads the tag from `EaSize` of `FILE_ID_BOTH_DIR_INFO` when the reparse
  attribute is set (today `platform/src/dir.rs` keeps only the bool). An entry is a cloud entry when
  `tag & !0x0000F000 == 0x9000001A` (the Cloud Files tags, `IO_REPARSE_TAG_CLOUD`). This is from the Windows SDK and has
  not been seen on a real placeholder.
- **Excluded from the walk.** A cloud file is not added and gets no row. A cloud folder is not descended. The scan
  reports them as "cloud" (for example "N cloud files excluded"), apart from links and junctions, which get their own
  count. The message no longer says "indexed."
- **The folder rule is a path check.** The OneDrive folder is not a reparse point (`C:\Users\Torres\OneDrive` has
  attributes `ReadOnly, Directory, Archive`), so the tag can't exclude it. Refuse adding a root inside the folder named by
  the `OneDrive`, `OneDriveConsumer` (personal) or `OneDriveCommercial` (business) environment variable, each when set,
  with a message. A search result says `OneDrive` may name either account; not checked on this machine, where `OneDrive`
  is `C:\Users\Torres\OneDrive`. Other providers' roots (Dropbox, iCloud, Google Drive) are not covered: their sync roots
  would need the registry (`SyncRootManager`), not decided.
- **The walk skips it too.** A root above the OneDrive folder (the profile, say) still reaches it, since it is not a
  reparse point. So the walk also skips any directory whose path is one of those folders (the same environment
  variables, read at scan time). Before step 1 that is a full-path comparison; after it, the volume-relative form of the
  same folder. Decided here; the owner has not confirmed it.
- **Not decided here:** rows already indexed under a cloud folder stay as they are. Whether a hydrated placeholder keeps
  the reparse attribute is open: if it does, the listing's `FILE_ATTRIBUTE_OFFLINE` and `RECALL_ON_DATA_ACCESS` would be
  needed to tell online-only from downloaded files.
- **Tests:** the scan's fake listing gets cloud, symlink and junction tags, and the reparse cases in
  `crates/scan/src/lib.rs` check the counts. The `OneDrive` root refusal is a unit test with a fake environment. The
  real-placeholder check needs a machine with a placeholder; it is not in the tree's tests. The cloud-tag and OneDrive
  tests do not exist yet; they are new work in step 2.

### Catalog

Schema changes are migrations (`crates/catalog/src/schema.rs:43–62`). The baseline stays frozen at version 1, and
`MIGRATIONS` reaches version 11 today.

- **Migrations.** Every schema change below is a migration from version 12 on, in one transaction with its version. A
  migration converts the rows whose meaning changes, with Rust steps:
  - `links`: one per old file row;
  - `volumes`: from the old serial;
  - `content_gen`: from mtime;
  - `own_usn` and `own_from` are added in step 6, not here.

  Each Rust step checks the old shape by its columns and is idempotent (`schema.rs:15–16, 30`), since the rebuild
  (`rebuild.rs`) runs the steps again on carried rows. The rebuild runs only when a catalog's schema differs from this
  build's at its version. Existing catalogs keep their rows.
- **Volumes of the old rows.**
  - `roots`: the volume their files store, matched by path, so a re-lettered drive keeps its identity. A root with no
    files takes its current volume through the volume APIs (no file reads), and stays NULL until that volume is
    mounted. On a build without those APIs, the fallback is NULL.
  - `folders` keep their ids, since `links` point at them. A folder takes the volume its files store, else its root's.
    A folder with neither stays NULL until a scan, and paths resolve once its volume is set.
  - `file_ref` and `parent_id` of a migrated folder are NULL until the next scan of its root fills them in.
- **Volume identity: the 64-bit NTFS serial.** Done: the serial is read as 64 bits (`FILE_ID_INFO`,
  `crates/platform/src/dir.rs:299–311`) and carried as `FileIdentity::volume: u64` (`crates/model/src/ids.rs:37`). What
  changes is storage. `files.volume` is `INTEGER` (`crates/catalog/src/schema.rs:553`), and the `volumes` table is new.
  `FSCTL_GET_NTFS_VOLUME_DATA` in the helper gives the same value. The serial is stored as `volume.cast_signed()`
  (`crates/catalog/src/writers.rs:32`), so `volumes(serial PRIMARY KEY)` holds the signed form. Clones still copy the
  serial, so the duplicate refusal stays.
- **Paths relative to their volume; the drive letter is never stored.**
  - `folders.path` becomes the path inside the volume, and `roots` becomes `(volume, rel_path)`.
  - `volumes` keeps the GUID path. Where each volume is mounted now is resolved at start, at each run, and on device
    arrival or removal (`WM_DEVICECHANGE`; `GetVolumePathNamesForVolumeNameW`).
  - A full path is the current mount + `rel_path` + name, built in one place: an SQL function `pg_path(volume, rel_path,
    name)` over a process-wide mount table, registered in `query::register_functions` (`crates/query/src/lib.rs:393`).
    That is called from `Catalog::prepare` (`crates/catalog/src/core.rs:67`, its call at `:120`) and `Catalog::open`
    (`core.rs:133`, its call at `:153`). Full paths are built in Rust today, not in SQL: no `|| '\' ||` concatenation
    exists in the tree (checked 2026-11-14), so `pg_path` replaces the Rust joins. The call sites must be listed by grep
    before step 1; the list in earlier drafts was wrong.
  - The thumbnail cache's writer and file connections (`open_writer` and `open_file`,
    `crates/thumbcache/src/schema.rs:239` and `:378`) do not attach the catalog; the reader does
    (`crates/thumbcache/src/reader.rs:28`). So the writer connections must gain an attach. `pg_path` must be registered on
    each of them, since registration is per connection. `query::register_functions` is called from production code only
    at `core.rs:120` and `:153`; `pg-query`'s tests call it at `lib.rs:500` and `:527`.
  - **Decided:** the mount table lives in `pg-platform`, which calls the mount APIs. `pg-query` depends on `pg-platform`
    and owns `pg_path`. `pg-thumbcache` depends on `pg-query` and registers `pg_path` on its connections. The crate graph
    allows it: `pg-platform` and `pg-query` each depend only on `pg-model` (plus `pg-platform` for the query), and nothing
    depends back on `pg-thumbcache` from either.
  - The rebuild connection (`rebuild`, `crates/catalog/src/rebuild.rs:53`, opened at `:105`; `carry_rows` attaches the old
    catalog as `carried` at `:175`) must have `pg_path` too, when it copies paths.
  - `file_by_path` (`crates/catalog/src/lookup.rs:98`, used by `crates/cli/src/borders.rs:191` and
    `crates/cli/src/lib.rs:398`; tests at `crates/catalog/src/lifecycle.rs:688–689`) resolves a full path to
    `(volume, rel_path)` first.
  - Roots by `(volume, rel_path)`: `Catalog::roots`, `add_root` and `remove_root` (`crates/catalog/src/roots.rs:55`,
    `:70`, `:89`). `is_under` (`PathRules::is_under`, `crates/model/src/rules.rs:281`) has these users:
    `crates/catalog/src/roots.rs:75`, `:78`, `:121`; `crates/catalog/src/types.rs:80`;
    `crates/library/src/runner.rs:225–230` and `:461` (`:519` in the working tree); `crates/crop/src/lib.rs:84` and `:94`
    at HEAD. The library panel in the app (`crates/desktop/src/library.rs`, over `crates/app/src/library.rs`) shows roots
    with their current letter.
  - **The folder tree's drive marks.** Each node carries one bit per drive *letter* it is on. Today the mechanism is
    `anchors: u64` in `crates/model/src/tree.rs:27`: one bit per anchor (a drive root) by its rank in text order, and the
    64th and later anchors share bit 63 (`tree.rs:67–70`). The rows come from `crates/catalog/src/browse/read/tree.rs`.
    The new unit is the volume, so the bits become the node's volumes, shown by current letter or, when offline, by
    volume label. Keep `u64` and the shared-bit rule: a `u32` would cut the capacity to 32 and drop the rule.
  - **Offline volumes.** A volume that isn't mounted is **offline**: its files show as unavailable, not missing, and come
    back as they were when it returns, under whatever letter. In the app, the grid and viewer show cached thumbnails with
    an "offline" mark, opening the original says the drive is offline, and the import, borders, thumbnail and writeback
    passes skip it (writes stay queued).
  - The merge that drops the drive letter is `PathRules::merge` (`crates/model/src/rules.rs:154`, "its anchor dropped").
    Its stored form is `PathRules::join_merged` (`rules.rs:160`), with the `Merged` type at `:51` and the tests around
    `:481`. Folder keys and `order_key` don't change. Nothing else stores letters: `session` keeps a file ID and a merged
    folder path, and `views.folder` a merged path (`crates/catalog/src/schema.rs:729`, the baseline's comment).
- **Folders are a tree.** `parent_id` + `name` are the truth. `rel_path` is derived and kept indexed for `under_root!`
  range queries. `move_folder` changes one `parent_id`/`name`, then rewrites the derived paths and `order_key`s under it,
  one statement each.
- **`folders`: a row for every directory under a root.** Step 1 keeps today's rule (a row only for a folder that holds
  media: `crates/scan/src/lib.rs:566–576`); every directory comes with step 2's walker and step 4's MFT read. The key is
  `volume`, `file_ref`, `parent_id`, `UNIQUE(volume, file_ref)`.
  - A rename or move is always found by ref → **`Catalog::move_folder(old, new)`**, which rewrites the paths under it and
    its links' `order_key`. `pg_order_key` is a SQL function over `PathRules::order_key(folder_key, name)`
    (`crates/model/src/rules.rs:257`), registered like `pg_path`. It is not in the tree.
  - Uniqueness is `UNIQUE(parent_id, name)` and the derived `UNIQUE(volume, rel_path)`. A stale row already at the new
    place is merged first.
  - The tree in the UI is unaffected: its counts come from links (today `FOLDER_COUNTS`,
    `crates/catalog/src/browse/read/tree.rs:10`, from `files`), so folders without media still don't show.
- **Identity and location split.**
  - `files` keeps identity (`volume`, `file_ref`), `size`, `mtime`, `kind`, `missing`, `pending`, `seq` and the per-file
    data (analysis, ratings, writes).
  - A new **`links(id, file_id, folder_id, name, ext, order_key, last_scan)`**, `UNIQUE(folder_id, name)`, takes
    `folder_id`, `name`, `ext` and `order_key` off `files`. `files_order` and `files_present` move to `links`. The grid,
    the folder counts and the order list read links. A file reads and writes through any present link
    (`crates/writeback/src/lib.rs`, the import's paths). A file is missing when it has no present link.
  - Everything that reads `files.folder_id`, `name` or `order_key` moves to `links`: 57 lines reference `folder_id` in
    `crates/catalog/src` and one in `crates/thumbcache/src` (the grep count of 2026-11-14; recount before step 1). Also
    the name filter `pg_contains(f.name, ?)` (`crates/query/src/lib.rs:308`), the name order `f.order_key` in
    `crates/catalog/src/browse/read/grid.rs:166` and `scope.rs:14`, and the tests that query them (`crates/library/tests/video.rs`,
    `writeback.rs`). The migration to `links` makes one link per old row.
- **A content generation, not mtime, says whether derived data is current.** `stale!` (`crates/catalog/src/sql.rs:8`;
  analysis: size, mtime, rotation, kind) and the thumbnail cache's `Source` (`crates/thumbcache/src/types.rs:17`; mtime,
  rotation) both trust mtime. A rewrite that keeps the mtime, which the journal now detects, would leave the analysis and
  the cached thumbnails current forever.
  - **`files.content_gen`** goes up on a change of size or mtime, and on a journal data-change reason that isn't ours.
  - `analysis` records the `content_gen` it was made from, and `stale!` compares it (with rotation and kind).
  - The thumbnail cache's `Source` becomes `(content_gen, rotation)`. The cache schema is edited in place, at version 1.
  - Writeback's own writes and `borders_cut` bump it too. `borders_cut` updates size and mtime only today, so it changes;
    its analysis is redone as now. The metadata-only carry-over in `file_rewritten` moves `analysis.content_gen` to the new
    generation in the same transaction, so a rating write keeps its analysis current.
  - `retag` (`crates/thumbcache/src/lib.rs:209`) moves the thumbnails from the old generation to the new one, as it moves
    them between mtimes today.
  - The import's check that a file didn't change while it was read (`AnalysisWriter`'s `current`) compares `content_gen`.
- **`volumes(serial PRIMARY KEY, guid_path, journal_id, next_usn, synced)`** is created in step 1, since offline mounts
  need it. Its journal columns are written by the full read (step 4 stores U0 there) and used by the journal pass in step
  6. Until step 6, a full read misses changes made during it, and the next full scan catches them.
  `Catalog::mark_missing_ref(volume, file_ref)` is new.
- **`mark_missing(root, scan, skipped)`** (`crates/catalog/src/roots.rs:150`; today its third argument is the skip list,
  called from `crates/scan/src/lib.rs:591`) becomes: links under `root` that this scan didn't see are deleted, except
  under the pass's unknown directories. Then files left without a present link are marked missing. In an MFT read each
  root gets its own scan ID.
- **`ScanWriter`** (`crates/catalog/src/writers.rs`, built by `scan_writer` (`crates/catalog/src/core.rs:212`, which
  calls `ScanWriter::new` at `:213`)) is rewritten. It writes files and links, records folders by ref (moving them), and
  takes the unknown list. Every source writes through it, unelevated.

### Journal passes (state-based)

1. Read from the saved USN (in the helper, or unprivileged as the fallback). Group the records by file ref, keeping each
   record's USN and reason, since the own-write rule needs each record's USN.
2. Resolve each ref unelevated with `OpenFileById` on the root's handle (verified with a prototype):
   - a wrong sequence number, older or newer, fails with 87, so a reused record can't pass for the old file;
   - `FileNameInfo` gives one link's path, without the drive;
   - `FindFirstFileNameW`/`FindNextFileNameW` list every link (both found);
   - `FileStandardInfo` and `FileBasicInfo` give size, mtime and attributes.
3. Keep a ref if it is in the catalog (`files` or `folders`), or it is now a directory, or one of its live names (from
   item 2) is a media name. A ref that doesn't resolve is kept only if it is in the catalog, which then goes missing or
   loses its links. A `.jpg` renamed to `.txt` is in the catalog, so it is kept and marked missing.
4. **Directories first, shallowest old path first:**
   - moved under a root → `move_folder`;
   - moved out of the roots or deleted → delete the links under the old path (files left without one go missing);
   - new, or moved in → walk it.
5. **Then files:**
   - upsert the file and its links whose names are media names (every name from `FindFirstFileNameW`, through the media
     test of item 3; a link renamed to a non-media name is removed);
   - delete the file's links the list no longer has (a removed hard link; the file keeps its other links, and one left
     with none goes missing), or `mark_missing_ref`;
   - store the next USN returned by the read, in the same transaction as the pass's writes. A pass that stops or fails
     stores none, so the next pass replays it.
6. **Content changes come from the journal, not only from size and mtime.** A tool that rewrites a file and restores its
   mtime, at the same size, is invisible to `(size, mtime)`. A ref whose reasons include `DATA_OVERWRITE`,
   `DATA_EXTEND` or `DATA_TRUNCATION` bumps its `content_gen`. That makes it pending, and gives it a new `seq` for the
   thumbnails, whatever its size and mtime.
7. **Our own writes are not external changes:** the bracket rule, below.
8. **Journal unusable** (entry deleted, ID changed, inactive) → a full read.

### Our own writes: the bracket rule

Writeback's own writes must not show up as external changes, and a real external change must not be hidden by them.

Every in-place write of an original that completes records through one own-write function in the catalog. The function
takes the bracket start and the stored size and mtime.

- **Before the first stat of the original:** read the volume's `NextUsn` (`FSCTL_QUERY_USN_JOURNAL`, which works
  unelevated on the root-directory handle). That is the bracket start.
- **After the copy's stat:** read the file's latest USN on a fresh handle (`FSCTL_READ_FILE_USN_DATA`). Writeback's own
  handle is gone by then, since `write_into` takes it by value. That is `files.own_usn`.
- **Record,** through `file_rewritten` (`crates/catalog/src/queue.rs:331`): `files.own_usn`, the size and mtime written,
  and `files.own_from`, the earliest bracket start not yet consumed by a journal pass. `own_from` is set when it is empty
  and cleared by the pass.

**The callers.** Writeback's copy and `finish_copy` call the function through `write_done` (in `queue.rs`, which calls
`file_rewritten` at `:232` and `:236`). `crop::cut` calls it through `borders_cut`, because `replace`
(`crates/writeback/src/inplace.rs`, which `crop::cut` calls at `crates/crop/src/lib.rs:308`) has no catalog and returns
only the new size and mtime. Both callers are tested together in step 6, so neither path can skip the record.
`finish_copy` keeps its unchecked copy on purpose, since its original may already be partly the temp's. A write that fails
before its record is taken is left to the journal, which counts its records as external. That costs one extra re-import,
and it is safe.

**The rule.** A journal record for the file is ours only if its USN is at or above `own_from` and at or below
`own_usn`, and the file's size and mtime still equal what the write stored. Otherwise it is an external change and bumps
`content_gen`. Without this test, every rating write would re-import the file.

**What the bracket does not cover.** The bracket closes the span before the write. The windows left are listed below.
None is a journal issue.

- **W0, from the `NextUsn` read to the stat at `:502`.** A change in this gap has a USN inside the bracket. If its final
  size and mtime match the stored ones, it is counted as ours, and its thumbnails can carry forward through `retag`. The
  bracket is read as early as the code allows, but the gap is not zero.
- **W1, from the stat at `:502` to the check at `:630`.** A change that restores size and mtime is not seen by the check.
  For a rating it is copied in, and the analysis carry-over keeps it as ours (`queue.rs:334–339`). For a turn the copy
  overwrites it, as in W2.
- **W2, from the check at `:630` to the end of the copy's `sync_all` (`file.rs:65`).** A writer that already holds a
  write handle with share-write can still change the file, and the copy overwrites it. A turn's copy writes the bytes
  read at `:503` (or, when ExifTool writes the orientation, the temp it made from the file by path at `:546`). A rating's
  copy writes the temp ExifTool made at `:525`. Nothing detects it. `FILE_SHARE_READ` blocks only new writers. This limit
  of writeback predates this plan.
  - **Accepted as a limit (2026-11-14), recorded in this list;** a code comment at the copy site is added in step 6.
  - An atomic replace is ruled out as the fix. `ReplaceFile` gives the original path the replacement's file index, and a
    hard link keeps the old content (measured 2026-11-14, `ReplaceIdProbe.cs`, session scratchpad). That breaks the
    identity the catalog keys on (`crates/writeback/src/inplace.rs:1–3`). No fix that keeps identity is designed here.
- **W3, from the copy's end to the stat at `:642`.** An external write here is included in `after`. The catalog records
  it as ours and carries the analysis over with our hash. The journal sees the stored size and mtime and ignores the
  record. It is narrow, a few milliseconds, and USN reasons cannot separate it from our write.
- **W4, after the stat at `:642`.** A write after the `own_usn` read has a larger USN, so it counts as external and is
  caught, whatever its size and mtime. Only a write between the stat at `:642` and the `own_usn` read can be ignored, and
  only if it restores both size and mtime. Then it is ignored like W3. Otherwise the size or mtime differs from what was
  stored, and the rule bumps `content_gen`.
- **Crop path, from the read to the locked check in `prepare` (`crates/writeback/src/inplace.rs:60`).** The bytes are read
  by the pipeline before `cut_original` runs. `cut_original` and `prepare` compare the stat with the scan's stored size and
  mtime, not with a stat taken with the read. Between them run `crop::cut`, `with_tags` (ExifTool), and the temp write and
  sync (`inplace.rs:57–58`). A same-size, same-mtime change anywhere in that span is cut from the old bytes and overwrites
  the file. This is the same class as W1, with a longer span.

**Tested for the rule:** a rating written by the app is not pending again; a rating written while an external change sits
inside the bracket is caught when the change moves size or mtime (step 6).

### The runner and live following

**The runner: a full read isn't interrupted by unrelated work.** Today any request stops the running pass
(`crates/library/src/runner.rs:268–270` at HEAD: "the queue has a new request: stop the round in progress"). A
non-resumable ~14 min read would restart each time. So requests that only add work queue behind a running volume read.
Only a removed root, quitting or the user's stop end it.

**Live following (step 7).** On a machine that prompts, following uses the unprivileged read, run by the parent, so it
never prompts. Whether that read can wait for new records (`BytesToWaitFor`) is not verified for
`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`; step 7 checks it first, and if it can't, the parent polls at an interval. Where
elevation is silent, the helper keeps a waiting journal read per volume (`BytesToWaitFor`) and tells the job a volume
changed. The runner debounces and runs a journal pass. The rescan button becomes a fallback; "full rescan" forces the MFT
read.

## Steps

Each numbered step is committed on its own. Step 1 is five commits.

1. **Catalog model.**
   - the `files`/`links` split, with every reader moved to it (`crates/catalog`, `crates/thumbcache`, `dups.rs`,
     `writeback.rs`, the import, and the name filter at `crates/query/src/lib.rs:308`);
   - `ScanWriter` writes files and links, as in the Catalog section (the walker of step 2 and the read of step 4 both
     write through it);
   - the 64-bit serial in the catalog columns (the type is already done, see Status);
   - `content_gen` in `files`, `analysis` and the thumbnail cache;
   - volume-relative paths, roots by volume, mounts resolved at run time and refreshed on device arrival and removal,
     offline volumes, and the `volumes` table (moved here from step 6);
   - folders as a tree (`parent_id` + `name`, derived `rel_path`), with `volume/file_ref` for the folders that hold media
     (today's rule), and `move_folder` with the merge.

   The five are committed separately (links; serial; `content_gen`; relative paths and offline; folder tree), each with
   its tests. Step 1 touches `crates/catalog/src/browse/`, which other work is changing. That work is committed first, and
   the step is confirmed with the owner. The crop cut work (see Critical files) is committed before step 1 starts, since
   step 1 edits the same files.

   Tests: catalog unit tests (links, missing, `move_folder`, `rebuild` from an old catalog), the existing suites, and the
   app's grid and folder tree by screenshot (`run` skill). Also the detach/reattach offline test (Verification), and the
   shared drive-letter bit test for anchors 64 and later (a unit test on `tree.rs`'s `bit()` with 65 anchors).

2. **`listing.rs`:** the entry filter, the `FileId == 0` rule, the unknown list, the duplicate-serial refusal and the cloud
   rule (see Cloud folders). The non-NTFS and share refusals already exist (`crates/scan/src/refusal.rs`; `NetworkPath`
   at `:42`) and stay.
   - The walker moves to `walk.rs`, records links, takes live values by ID, and no longer hides the files of folders it
     couldn't list (already fixed; see the walker rule above).
   - **Decided:** the walk moves to `crates/library`, and `pg-scan` is folded into `pg-library`. The `Listing` trait, the
     filter, the refusals and the synthetic bench tree move with it. `pg_scan::scan`, `check_folder` and `ScanError`
     become `pg_library` items. The callers in `cli`, `bench` and `library` change with them: the `bench` feature moves
     with `crates/bench/Cargo.toml:18`; `library` is `src/runner.rs`, `src/scans.rs`, `src/folders.rs` and the
     `tests/*.rs` files.
   - **Decided: no test moves.** `thumbs` keeps its three tests (`crates/thumbs/src/lib.rs:414, :468, :618`, working-tree
     lines; at HEAD the scan test is at `:582`) and its dev-dependency on the scan, which is now `pg-library`. Its
     `pg_scan` dev-dependency is `crates/thumbs/Cargo.toml:25`. Two things block a move: the `PANIC_ON` seam
     (`:311–312`, under `#[cfg(test)]`) cannot be reached from another crate, and the `imported` helper (`:459–470`,
     its scan call at `:468`) is shared by four more tests (`:488, :508, :522, :543`). The dev-only cycle (`pg-library`
     depends on `pg-thumbs` normally) is accepted by cargo for dev-dependencies. Step 2 confirms it with
     `cargo test -p pg-thumbs`.
   - Doc references to `pg_scan` change with the move: `crates/platform/src/dir.rs:7`, `crates/model/src/refusal.rs:1`,
     `crates/library/src/lib.rs:4`, `crates/README.md:73, 89`, `docs/reference/STORAGE.md:60`,
     `docs/reference/PROFILING.md:160–161`, `docs/reference/NOTIFICATIONS_DESIGN.md:143`,
     `crates/library/src/folders.rs:15`, `crates/library/Cargo.toml:12`.
   - Step 2 is one commit, and it includes this move.
   - Tests: the existing `crates/library/tests/*.rs`, plus hard-link, reparse and unreadable-folder cases (a folder with a
     deny-list ACL). New: the cloud tags on a fake listing, the OneDrive root refusal with a fake environment, duplicate
     serials on two fake volumes, the exact-walker test (after editing a file through its second link, the walker records
     the new size and mtime), and the dedup keep-rule unit test (tag `0x80000013` on a fake listing).

3. **`crates/volume`: the parser (`record.rs`, `runs.rs`).** Tests:
   - records from a builder with fixups, plus real records captured by a capture step in this step, from C: and E: (the
     prototype's records are not kept in the repo);
   - a mutation test: errors, never panics or hangs (the cycle rule is in step 4);
   - the `mft` crate as an oracle on a captured `$MFT`, as a dev-dependency. `just lint` runs `cargo deny` (licenses,
     duplicates; "a new license is a decision", `deny.toml`) and `cargo machete`. Whatever it pulls in must pass them, or
     the oracle is replaced by records cross-checked with `GetFileInformationByHandle`.

4. **The MFT reader (`mft.rs`),** in-process when elevated, storing U0 in `volumes`. The `--full`/`--walk` flags arrive in
   step 5, so step 4's differential test drives the reader through a test entry point. Tests: a parent loop goes to the
   unknown list, not followed (a tree unit test); the flush-consistency test (a file created just before the read); the
   bounded-memory test (a few-KiB deferral budget gives the same catalog as an unbounded read); the unreadable-record test
   (a record made unreadable in the image is resolved live, and goes to the unknown list if that fails too); the progress
   test, in `pg-library` (records read against `MftValidDataLength / record size`, through `Control::begin`). Differential
   test on the test VHD, then on the real drives.

5. **`pg-volume.exe` and the pipe.** `crates/library/src/helper.rs` (the parent side: `ShellExecuteExW` launch, pipe
   server, request frames), `crates/volume/build.rs` (the `/DEPENDENTLOADFLAG` link arg), and `crates/library/src/runner.rs`
   (volume grouping, source choice, uninterrupted reads). `crates/cli/src/lib.rs` gets the flags (`--full`, `--walk`).
   Tests: the pipe protocol with a fake client in-process (first-instance refusal, DACL, PID check, request frames); the
   built exe's imports checked (no DLL outside System32 is imported); two roots on one volume scheduled as one read; an
   added root queued behind a running full read, which it does not interrupt; a removed root stops it; changing the
   thumbnail height during a full read does not restart it.

6. **Journal passes** (privileged read in the helper, unprivileged fallback). The journal columns of `volumes` (the table
   is in step 1); the replay after a full read; `own_usn` and `own_from` with the bracket rule, including their columns
   and migration; and the W2 comment at the copy site. Tests: the unprivileged fallback run unelevated on D: or E:,
   compared to the privileged read (names blank, a USN mid-record fails); the journal list (create, rename, move, rewrite,
   delete; a rename to a non-media name; nested folder renames; a folder out of the root and back; a renamed folder
   without direct media, found by ref; add and remove a hard link; record reuse; journal wrap falling back to a full
   read), each compared to a fresh walk; the content change without mtime (rewrite at the same size, restore the mtime:
   the thumbnails are made again; a rating written by the app is not pending again); both own-write callers (writeback
   and `borders_cut`) record through the same function.

7. **Live following.** Checks first whether `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` can wait for new records
   (`BytesToWaitFor`), with the polling fallback if not. Test: a journal change while following gives one debounced
   journal pass.

8. **Measure once, at the end.**
   - On E:, a cold walk vs a warm walk vs the MFT read (1 vs 2 in flight) vs a journal pass, and the helper's peak memory.
     The journal pass is measured on C: as well, since most changed refs there lie outside the roots, so the resolution
     cost shows.
   - Results in `docs/reference/IO.md` ("I/O"), `docs/reference/STORAGE.md`, PLAN M1 (`docs/plans/PLAN.md:274`) and
     `crates/README.md`.
   - `docs/reference/STORAGE.md` has no Z: text today (checked 2026-11-14). Add the 2026-11-14 Z: result as a note beside
     the FAT row (line 40). The zero-file-ID row (line 43) stays. The FAT row's claim that file IDs are unstable is not
     measured, and the row should say so.

## Critical files

- **Changed:**
  - the crop cut work (`crates/crop/src/lib.rs`, `crates/edit/src/crop.rs`, `crates/catalog/src/borders.rs`,
    `crates/writeback/src/inplace.rs`, the desktop QML): another owner's uncommitted work, committed before step 1;
  - `crates/scan/src/` (all of it, folded into `pg-library` in step 2), `crates/library/src/runner.rs`,
    `crates/library/src/folders.rs` (adding a root: the refusal), and `crates/cli/src/lib.rs`;
  - `crates/catalog/src/`: `schema.rs` (migrations from version 12, `links`, `volumes`), `writers.rs` (`ScanWriter`),
    `roots.rs` (`mark_missing`, `mark_missing_ref`, roots by volume), `rebuild.rs` (the copy from old catalogs), `queue.rs`
    (`file_rewritten`, `own_usn`, `own_from`), `sql.rs` (`stale!`), `core.rs` (`prepare`/`open`), `lookup.rs`
    (`file_by_path`), `dups.rs`, `borders.rs`, and the new `move_folder`;
  - `crates/catalog/src/browse/`, `crates/query/src/lib.rs` (`pg_path`, `register_functions` at `:393`; `Cargo.toml` gains
    `pg-platform`), `crates/platform/src/` (the process-wide mount table), `crates/thumbcache/src/` (`Cargo.toml` gains
    `pg-query`; `types.rs` `Source`, `lib.rs` `retag`, `schema.rs` `open_writer`/`open_file`, `reader.rs` the attach;
    links, `pg_path`, drive marks, `content_gen`), `crates/model/src/tree.rs` (`anchors`, the drive marks), and
    `crates/model/src/rules.rs` (`merge`, `join_merged`, `is_under`);
  - `crates/thumbs/` (its `pg_scan` dev-dependency is repointed to `pg-library`; no test moves),
    `crates/platform/src/dir.rs` (the tag from `EaSize`, the 64-bit ID path), `crates/crop/src/lib.rs` (`is_under` users),
    `crates/import/src/`, `crates/writeback/src/lib.rs` (links, paths, roots, the own-write call);
  - `crates/desktop/src/library.rs` and `crates/app/src/library.rs` (roots by volume), plus the offline mark in the grid
    and viewer (`crates/desktop/src/gallery.rs`, `providers.rs`, QML);
  - `Cargo.toml` (member `crates/volume`; windows-sys features to add: `Win32_System_Ioctl`, `Win32_System_Pipes`,
    `Win32_UI_Shell`, `Win32_Security_Authorization`; the current list has none of these).
- **New:**
  - `crates/volume/build.rs` and `crates/volume/src/{lib.rs, record.rs, runs.rs, mft.rs, usn.rs, protocol.rs,
    bin/pg-volume.rs}`;
  - `crates/library/src/{listing.rs, walk.rs}` (step 2) and `crates/library/src/helper.rs` (step 5);
  - `pg_order_key` and `pg_path` (SQL functions; both planned, not in the tree), and `verbatim_wide` (ported from
    `crates/core/src/paths.rs:22` at `4e1ad4d`; absent at `493116c`).
- **Reused:**
  - the catalog's statement style (constants, `concat!` macros) and `rebuild` (`crates/catalog/src/rebuild.rs:53`). The
    schema change is a migration with Rust steps (`MIGRATIONS`, `schema.rs:51–62`), not a baseline edit;
  - `Ext::of_name_bytes` (`crates/model/src/media.rs:108`; the older name `of_name_utf16` at `4e1ad4d`);
  - `Control` (`crates/jobs/src/control.rs`), `normalize_path` (`crates/platform/src/names.rs:171`).
- **Prototypes:** the raw MFT read, the pipe and the USN probes were checked with throwaway C# and PowerShell prototypes,
  which are not kept in the repo.

## Verification

- **Test VHD** (elevated, made by the test with `diskpart`, detached at the end). It holds:
  - paths over 260 characters and non-BMP names;
  - hard links across folders (and enough of them for an `$ATTRIBUTE_LIST`);
  - a fragmented file whose `$DATA` spills into extension records;
  - junctions, symlinks and a mount point (skipped); WOF and sparse files (kept). WOF files are made with
    `FSCTL_SET_EXTERNAL_BACKING` (see the 2026-11-14 table);
  - a file being written; a file created and closed just before the read (the flush test).
- **Elevated tests** are gated with a name prefix (`#[ignore = "elevated shell and a test VHD"]`; every elevated test name
  starts with `elevated_`). Steps 3–4 tests are in `pg-volume`, except step 4's progress test, which is in `pg-library`.
  Steps 5–6 tests are in `pg-library`, and so is step 1's offline test. Run them from an elevated shell with
  `cargo test -p pg-volume -- --ignored elevated_` and `cargo test -p pg-library -- --ignored elevated_`.
  `cargo test --workspace` runs unelevated and does not include them.
- **Differential:** on the VHD and on D:, E:, F: and G: (scratch catalogs, `PG_CATALOG_DIR`), `--walk` and `--full` must
  agree exactly on files, links, sizes and mtimes. The walker takes live values, so any mismatch is a bug in one of them,
  settled against the live file.
- **Journal, each compared to a fresh walk:** listed under step 6.
- **Runner:** listed under step 5 (the thumbnail height, queued roots, removed root).
- **Volumes:** detach the test VHD → its files show offline, not missing; reattach it under another letter → the same
  rows, ratings and analysis, with no rescan and nothing marked moved.
- **Bounded memory:** listed under step 4. **Exact walker:** listed under step 2.
- **Content change without mtime:** listed under step 6.
- **Own writes:** the bracket rule and its residual windows are tested in step 6.
- **Unknowns:** a folder the walker can't list (deny ACL) and an MFT record corrupted in a VHD image read offline: their
  files stay present, and the scan reports the error. The unreadable-record live resolution is listed under step 4.
- **Dedup:** not testable on this machine (no dedup). The filter's keep rule for tag `0x80000013` is a unit test with a
  fake tag (step 2).
- **Progress:** listed under step 4. **Shared bit:** listed under step 1.
- **Existing suites:** `cargo test --workspace` and `just lint`.
- **Builds:** `crates/scan/src/lib.rs`, `crates/catalog/src/` and `crates/library/src/runner.rs` are edited by other work;
  each build phase runs only when no other edit is in progress.

## Validation record

Re-checked 2026-11-14. Each claim is marked with how it was checked.

**Verified, by experiment on this machine (prototypes, which are not kept in the repo unless a path says so) or by
Microsoft's docs:**

| Claim | Evidence |
|---|---|
| Silent elevation; `runas` doesn't pass the environment | registry; elevated runs without a prompt; a variable empty in the child |
| E: 1 KiB records, 99.99 GB (decimal; `fsutil` prints 93,12 GB, i.e. 93.12 GiB) of MFT, 78 extents; journals active everywhere | `fsutil fsinfo ntfsinfo`, `fsutil usn queryjournal` (elevated) |
| Raw read at 116 MB/s, one read at a time; 3.3% directories, 0 torn, 128 lists, 63 extensions per 2M records | prototype sweep of E:'s first 2 GB |
| SI mtime and `$DATA` size equal the live values; record layout offsets; record and sequence = file ID | prototype on C: |
| Hard links: POSIX names; the other link's `$FILE_NAME` copy is stale (size 0) | prototype on C: |
| Serial = low 32 bits of the NTFS serial | `fsutil` vs `vol`, 4 volumes |
| Unprivileged journal: root-directory handle, names blank, parent/reason/attributes present, boundary USN required | prototype on D: and E: |
| `OpenFileById` rejects a wrong sequence (87); `FileNameInfo` one link; `FindFirstFileNameW` all links | prototype on C: |
| `FileIdInfo` gives the 64-bit serial (`2cac2355ac2318ba`, low half = `vol`) and the 128-bit ID unelevated; `FSCTL_READ_FILE_USN_DATA` works unelevated | prototype on C: |
| User-SID DACL passes the elevated client; server PID check; a second first-instance create is refused with `ERROR_ACCESS_DENIED` (5) when the instance limit is not reached (231 is the limit) | prototype, both sides; `PipeFirstProbe2.cs` run 2026-11-14 (session scratchpad, not in the repo) |
| Volume flush works (`GENERIC_WRITE`, admin); IO priority hint accepted on a volume handle | prototype on C:; docs |
| `ReplaceFile` gives the original path the replacement's file index; a hard link keeps the old content | `ReplaceIdProbe.cs` (session scratchpad), 2026-11-14 |
| A write made after a USN position is taken lands in a journal record at or above it, for writes through a handle that is still open (one handle; two writes in one session, then one in a new session). Folding into an older record from another handle is not reproduced | `UsnFoldProbe.cs` (session scratchpad), unprivileged read `0x903AB`, 2026-11-14 |
| `EaSize` holds the reparse tag; `FileId` 0 without 64-bit IDs; ReFS -1 | MS-FSCC |
| Z: (Google Drive's FAT32 drive) gave `FileId == 0` for all entries | prototype listing, earlier. Superseded by the 2026-11-14 re-measurement (below) |
| `mft` allocates per entry; `ntfs` reads per record | docs.rs |
| No duplicate serials among the mounted volumes | `Get-CimInstance Win32_Volume` |

**From our code, at `4e1ad4d` (2026-11-02; history).** The commit is not reachable from any branch: the history was
squashed into `88e780b`. It survives in the reflog (`HEAD@{755}`) until that expires, and its objects are still in this
clone, so the anchors below can be read with `git show 4e1ad4d:<path>` until `git gc` prunes them. The earlier claim
that every other anchor was "checked and current" is withdrawn. Re-checked at `493116c` (2026-11-14):

| Anchor at 4e1ad4d | At 493116c |
|---|---|
| catalog opened first (`cli.rs:151–153`) | `crates/cli/src/lib.rs:64` creates the catalog folder; `run_command` opens the catalog at `:136` |
| every request stops the pass (`runner.rs:150–154`) | `crates/library/src/runner.rs:268–270` (`:280–282` in the uncommitted working tree) |
| folder rows only with media (`scan.rs:216`) | still so, at `crates/scan/src/lib.rs:566–576` |
| all reparse points skipped (`scan.rs:128`) | no longer: `crates/scan/src/lib.rs:502–514` keeps a reparse file that has a row |
| the errors of listing a folder are counted (`scan.rs:203`), and `mark_missing` still runs (`:239`) | errors counted at `crates/scan/src/lib.rs:602–612`; `mark_missing` now takes the skip list (`crates/scan/src/lib.rs:591`, `crates/catalog/src/roots.rs:150`) |
| full paths concatenated in six places | built in Rust; no SQL concatenation is in the tree |
| the drive-letter bits in the folder tree (`browse.rs:142–143`) | `crates/model/src/tree.rs:27` (`anchors: u64`) |
| thumbnail and analysis staleness trust mtime (`thumbs.rs:146–150`, `:658–659`; `stale!`) | `stale!` at `crates/catalog/src/sql.rs:8`; thumbnail `Source` at `crates/thumbcache/src/types.rs:17` |
| writeback's writes both record through `file_rewritten` (`crates/ingest/src/writeback.rs:241`, `:314` at 4e1ad4d) | `file_rewritten`, a free function at `crates/catalog/src/queue.rs:331`, called at `:232` and `:236` |
| `session` and `views` keep merged paths and file IDs, never drive letters | `views.folder` at `crates/catalog/src/schema.rs:729` (`join_merged`); `session` not re-checked |
| `Ext::of_name_utf16` (`media.rs:61`) | renamed `Ext::of_name_bytes`, `crates/model/src/media.rs:108` |

The rest of the 4e1ad4d list (`paths.rs:22`, `control.rs:112`, `main.rs:6`, the `Justfile` lint and deploy) was not
re-checked.

**Verified on 2026-11-14** (elevated shell for the flush and WOF probes; a throwaway C# probe in the session scratchpad,
not in the repo; the flush probe's output was not saved, so its figures are from the session's console):

| Claim | Evidence |
|---|---|
| A volume flush writes MFT records dirty in the cache | 8 rounds of 200 new files on C:. Read raw (unbuffered handle) right after the files closed, before `FlushFileBuffers` on a `GENERIC_WRITE` volume handle: 0 to 28 of 200 records read as `present` (the name on disk). The rest were classed `free`, `in-use-other-name` or `unreadable`; the probe does not separate a deleted name from a free record. After the flush: 200 of 200 `present` in every round; the flush took 149–190 ms. Control: the same files with no flush and a 160 ms wait left the disk state unchanged in all 4 rounds. That excludes the lazy writer for a 160 ms window only; a longer-wait control is needed before saying more |
| WOF: the unnamed `$DATA` size equals the logical size, and WOF files can be made | `C:\Windows\System32\notepad.exe` (re-read 2026-11-14): WOF tag `0x80000017`; the named `$DATA` `WofCompressedData`; the unnamed `$DATA` in extension record `0x1516D`, non-resident, real size 200704 = the logical length. A WOF file made from a scratch copy with `FSCTL_SET_EXTERNAL_BACKING` (`WOF_PROVIDER_FILE`, `FILE_PROVIDER_COMPRESSION_XPRESS8K` = 2, per `winioctl.h`): tag `0x80000017`, unnamed `$DATA` real size = logical length, `WofCompressedData` present, content SHA-256 unchanged. `compact /EXE` refused every file type and algorithm tried. The WOF filter hides the reparse point from directory enumeration, `GetFileAttributes` and `fsutil reparsepoint query`; `fsutil file layout` still shows the reparse attribute, a 24-byte `$REPARSE_POINT` and the `WofCompressedData` stream (checked on `notepad.exe`) |
| Z: (FAT32, Google Drive) gives nonzero file IDs, so the earlier `FileId == 0` finding does not hold | `FileIdBothDirectoryInfo` on `Z:\Meu Drive`: 13 of 13 entries nonzero (`0x12`, `0x23`, …); `FileIdInfo` on one file gives `0x2B`. Control on NTFS, `C:\Windows\System32\drivers`: 466 of 467 nonzero; the one zero is the `..` entry, which the scanner exempts (`crates/scan/src/lib.rs`, the `is_dot` check). The comment at `crates/platform/src/dir.rs:301` ("`FileIdInfo` fails on FAT32") does not hold for this volume; not changed. Probe source: `ZFileIdProbe.cs` in the session scratchpad |

**Still unverified, and where each gets settled:**

| Claim | Status |
|---|---|
| `$STANDARD_INFORMATION` carries `OFFLINE`/`RECALL_ON_DATA_ACCESS`; OneDrive files are reparse points even when local | Re-checked 2026-11-14: `C:\Users\Torres\OneDrive` has 1 hidden file and 6 directories, and no `.url` files (this supersedes the earlier count of 4 files; reparse status not re-checked). No local placeholder to test. Open: the cloud rule's fake-listing tests are step 2; the real-placeholder check is a manual step-2 check on a machine with a placeholder |
| Throughput with 2 reads in flight; warm-walk time; the helper's peak memory; the journal-pass cost on a busy volume | Step 8, the measurement step. Not measurable until the helper exists |
| The `BytesToWaitFor` behaviour of `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` | Step 7, the first check of live following |
| Folding of a write into an older journal record across handles | Not reproduced (see the verified table); a test in step 6 is the place to settle it |
| The longer-wait flush control | Step 4's flush-consistency test |
| FAT's DST shift (local-time mtimes) | Partly supported: the Linux kernel's vfat docs treat FAT times as local time, with a DST caveat. The Microsoft spec text was not reachable (a 2008 mailing-list post says it never mentions time zones). Not tested here. No step: out of scope for this plan |
| FAT file IDs are unstable | Not tested. A third-party note says `FileIdInfo` fails on FAT with `ERROR_INVALID_PARAMETER`; it succeeds on Z: (FAT32), so that note does not hold for this driver. Stability needs a test on a FAT volume; no step (out of scope for this plan) |
| Dedup sizes from `$DATA` equal the logical size | Not testable here: `Enable-DedupVolume` is absent on this Windows 10 Pro install |
| Unprivileged journal, `OpenFileById` and pipe results | From prototypes not kept in the repo; the probe sources named above are in the session scratchpad |
