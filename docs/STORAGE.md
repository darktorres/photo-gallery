# Storage: drive kinds and file systems

How the scanner and the read passes behave on drives other than the library's HDD, investigated 2026-11-01.
**Decision: only NTFS is supported for now.** The file-system section records what would break elsewhere and
the fix, should that change. The behaviour described there is how Windows file systems are documented and
known to behave; none of it was tested against this code.

## What assumes NTFS

The scanner (`crates/ingest/src/scan.rs`) lists folders with `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`
and takes each entry's 64-bit `FileId` as the file's identity: `files(volume, file_ref)` is `UNIQUE`
(`crates/core/src/catalog.rs:90,104`), and the scan upserts on it (`ON CONFLICT(volume, file_ref)`,
`catalog.rs:965`), so a moved or renamed file keeps its row and its analysis. `volume` is the root's
`dwVolumeSerialNumber`.

The passes over the originals (import, thumbnails) read in on-disk order, `ORDER BY f.volume, f.file_ref &
0xFFFFFFFFFFFF` (`catalog.rs:585,813`, `thumbs.rs:660`): the low 48 bits of an NTFS file ID are its MFT record
number. Reads go through `pipeline::run` with 8 readers (`pipeline.rs:41`) using plain `std::fs::File` reads.

Entries are skipped when they are reparse points or have `FileId == 0` (`scan.rs:128`).

## Drive kinds (on NTFS)

The two storage-specific choices, MFT order and 8 reads in flight, come from the HDD bench (FINDINGS "I/O":
either gives ~3× path order).

| Drive | Behaviour |
|---|---|
| HDD | What it's tuned for: MFT order and queue depth each reach the disk's sequential speed. |
| SATA SSD / NVMe | MFT order is irrelevant but free. 8 in flight is shallow for NVMe, but FINDINGS "I/O" already has decoding as the bottleneck above ~1,600 files/s, so an SSD runs at decode speed. A deeper queue (picked from `IOCTL_STORAGE_QUERY_PROPERTY` / `StorageDeviceSeekPenaltyProperty`) is only worth it if a measurement shows readers waiting on the disk. |
| USB HDD | MFT order still helps. Behind a bridge without UAS (USB mass storage "bulk-only"), there's no NCQ, so the drive can't reorder the 8 reads and the queue-depth gain shrinks; the MFT order is what's left. |
| Network share (SMB) | Latency-bound: reads in flight help, the order hardly matters. Long paths are handled (`\\?\UNC\`, `pg_core::paths`). Identity depends on the server's file IDs (below). |

## Other file systems (unsupported)

| File system | What goes wrong | Severity |
|---|---|---|
| FAT32 / exFAT (SD cards, USB sticks) | The file ID is derived from the directory entry's position, so it isn't stable: a moved file becomes a new row and the old one is marked missing (analysis redone). FAT stores local time, which Windows converts with the *current* DST offset, so every mtime shifts by an hour at each DST change: every file goes pending again (the triggers in `catalog.rs`) and the library re-imports twice a year. | Wasted work |
| ReFS | File IDs are 128-bit. When one doesn't fit, the 64-bit `FileId` is a sentinel, so distinct files would collide on `(volume, file_ref)` and the upsert would overwrite one with another. | Wrong catalog |
| SMB shares without file IDs (some NAS / Samba setups) | `FileId` is 0 and `scan.rs:128` drops the entry silently, not even counted in `ScanStats::errors`: whole folders could be missing with nothing reported. | Silent omission |
| OneDrive / cloud-synced folders (on NTFS) | Their files are reparse points (cloud tags) even when local, and `scan.rs:128` skips every reparse point (meant for junctions and symlinks), so the folder scans as empty. | Silent omission |

On the non-NTFS volumes, masking to 48 bits makes the read order meaningless, which is harmless: queue depth
still helps.

## If support is ever needed

1. Read the volume's file system once per scan: `GetVolumeInformationByHandleW` on the root handle the scan
   already opens.
2. NTFS: today's path.
3. ReFS: list with `FileIdExtdDirectoryInfo`, which gives the full `FILE_ID_128`, and store it whole.
4. FAT / exFAT, or a share returning `FileId == 0`: identity from path + size + mtime, an mtime tolerance of ±1 h
   on FAT, and a log line that the volume fell back.
5. Reparse points: skip by tag (junctions `IO_REPARSE_TAG_MOUNT_POINT`, symlinks `IO_REPARSE_TAG_SYMLINK`), which
   `FILE_ID_BOTH_DIR_INFO` carries in `EaSize` when the reparse attribute is set, instead of skipping all of
   them. This one matters on NTFS too, for OneDrive folders.
6. Count entries dropped for `FileId == 0` as errors, so an omission is at least visible.

## Reading the MFT instead of listing folders (proposed, 2026-11-01)

What Everything and WizTree do: read the volume's `$MFT` directly, in large sequential chunks, and rebuild the
tree from each record's parent reference.

**Admin rights.** It needs a volume handle (`\.\E:`), which only an elevated process gets: unelevated,
`fsutil fsinfo ntfsinfo E:` is refused ("Acesso negado"). On this machine elevation is silent: the user is an
administrator, and UAC is on (`EnableLUA = 1`) with `ConsentPromptBehaviorAdmin = 0` (elevate without
prompting). Processes still start with the filtered token (Administrators deny-only, medium integrity), so the
app has to ask for elevation (`ShellExecuteExW`, verb `runas`): silent here, a UAC prompt on a machine with the
default settings.

**What it gains (estimates; the walk hasn't been timed on E:).**

- An MFT record is 1 KiB: at 70M files, ~70 GB, ~5 min at the HDD's ~230 MB/s (FINDINGS "I/O"). A cold folder
  walk pays a few seeks per folder: ~20 min per 100k folders, 3–4 h at 1M. Both are small next to the first
  import (~1.5 days at decode speed). Rescans are where the time matters.
- An MFT read always covers the whole volume, so a small folder is far cheaper to walk. Use it for drive roots
  and large roots, and keep the walk for the rest.
- Size and mtime come from the file's own record (`$STANDARD_INFORMATION`, unnamed `$DATA`). A listing returns
  the copies duplicated in the parent's index, which can lag (hard links, files still being written).
- The reparse tag comes with the record (`$REPARSE_POINT`), which fixes the OneDrive omission above.
- Record number | sequence << 48 *is* the NTFS file ID the catalog already stores: rows don't change, both
  scanners can coexist, and on the same folder they must agree, which is the test.

**Rescans: the USN journal.** The same volume handle reads the journal (`FSCTL_QUERY_USN_JOURNAL`,
`FSCTL_READ_USN_JOURNAL`): only what changed since the saved `(journal ID, next USN)`, in seconds. A full read is
needed again when the journal ID changed or the saved USN fell below `LowestValidUsn` (the journal wrapped). An
elevated process kept running could also follow it live, as Everything does.

**Shape.** Only the volume reads run elevated: a `gallery` subcommand started with `runas` that streams
`(file_ref, parent_ref, name, attributes, size, mtime, reparse tag)` over a named pipe the app creates (DACL: the
user's SID). The app stays unelevated (an elevated UI can't receive drops from Explorer) and so do the catalog
writes, through the existing `ScanWriter`, so the catalog's files keep their owner.

**Parsing.** Record 0 gives the `$MFT`'s own data runs; read them in order, in chunks of megabytes. Per record:
apply the update-sequence fixups; skip records not in use and extension records (follow `$ATTRIBUTE_LIST` from
the base record when `$DATA` lives in one); take the Win32 (or POSIX) `$FILE_NAME`, not the DOS 8.3 one; a hard
link is one `$FILE_NAME` per parent. Keep the folders (record → parent, name) in memory, rebuild paths, and keep
the media files under the scanned roots. Crates to evaluate first: `ntfs` (read-only NTFS) and `mft` (record
parser).
