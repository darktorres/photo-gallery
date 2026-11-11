# Storage: drive kinds and file systems

How the scanner and the read passes behave on drives other than the library's HDD, investigated 2026-11-01.
**Decision: only NTFS is supported for now.** The file-system section records what would break elsewhere and
the fix, should that change. The behaviour described there is how Windows file systems are documented and
known to behave; none of it was tested against this code.

## What assumes NTFS

The scanner (`crates/scan/src/lib.rs`; the listing is in `crates/platform/src/dir.rs`) lists folders with
`GetFileInformationByHandleEx(FileIdBothDirectoryInfo)` and takes each entry's file ID as the file's identity:
`files(volume, file_ref)` is `UNIQUE` (`crates/catalog/src/schema.rs`), and the scan upserts on it (`ON CONFLICT(volume,
file_ref)`), so a moved or renamed file keeps its row and its analysis. `volume` is the root's 64-bit `FileIdInfo`
serial number, not the 32-bit `dwVolumeSerialNumber`: two volumes can share the low 32 bits (S-17.7 decision 1).

The passes over the originals (import, thumbnails) read in on-disk order, `ORDER BY f.volume, f.file_ref &
0xFFFFFFFFFFFF` (`crates/catalog/src/lib.rs:460,878`, `crates/thumbcache/src/lib.rs:804`): the low 48 bits of an NTFS file ID are its MFT record
number. Reads go through `pipeline::run` with 8 readers (`crates/jobs/src/pipeline.rs:47`) using plain `std::fs::File` reads.

A reparse point is not followed. A file with a row keeps it, and gets no content from the listing; a folder is not descended, so its files are kept and not marked missing. An entry with `FileId == 0` makes its folder unreadable, which is reported with the other unreadable folders (S-17.7 decisions 9 and 10).
The unreadable folders, the refused mounts and a refused library folder are kept one row per folder in the catalog's
folder ledger (`folder_failures`), which the inbox shows as one entry; the schema and its rules are in
[NOTIFICATIONS.md](NOTIFICATIONS.md) ("The folder ledger", and "Storage, as `TO_8_NOTIFY` leaves it").

## Drive kinds (on NTFS)

The two storage-specific choices, MFT order and 8 reads in flight, come from the HDD bench (FINDINGS "I/O":
either gives ~3× path order).

| Drive | Behaviour |
|---|---|
| HDD | What it's tuned for: MFT order and queue depth each reach the disk's sequential speed. |
| SATA SSD / NVMe | MFT order is irrelevant but free. 8 in flight is shallow for NVMe, but FINDINGS "I/O" already has decoding as the bottleneck above ~1,600 files/s, so an SSD runs at decode speed. A deeper queue (picked from `IOCTL_STORAGE_QUERY_PROPERTY` / `StorageDeviceSeekPenaltyProperty`) is only worth it if a measurement shows readers waiting on the disk. |
| USB HDD | MFT order still helps. Behind a bridge without UAS (USB mass storage "bulk-only"), there's no NCQ, so the drive can't reorder the 8 reads and the queue-depth gain shrinks; the MFT order is what's left. |

## Other file systems (unsupported)

| File system | What goes wrong | Severity |
|---|---|---|
| FAT32 / exFAT (SD cards, USB sticks) | Refused when added or scanned (not NTFS; on Linux, `vfat`, `msdos` and `exfat` by type). The file ID comes from the directory entry, so it is not stable, and FAT's local-time mtimes shift by an hour at each DST change. | Refused |
| ReFS | Refused when added or scanned (not NTFS). Its file IDs are 128-bit, and the 64-bit `FileId` collides when one doesn't fit. | Refused |
| Network shares (SMB; on Linux, NFS, CIFS and SMB) | Refused when added or scanned: on Windows a UNC path by its text and a mapped network drive by its drive type, as a removable drive is (`crates/scan/src/refusal.rs`). The file IDs come from the server, which may give none or unstable ones. | Refused |
| An accepted root that reports zero file IDs | `FileId` is 0. The folder is unreadable and reported with the other unreadable folders, not dropped silently (S-17.7 decision 10). | Reported |
| OneDrive / cloud-synced folders (on NTFS) | Their files are reparse points (cloud tags) even when local. A placeholder file keeps its row and gets no content from the listing. A folder of placeholders is not descended, so its files are kept but not refreshed, and the scan counts the reparse points it indexed without following (S-17.7 decision 9). | Kept, not refreshed |

On the non-NTFS volumes, masking to 48 bits makes the read order meaningless, which is harmless: queue depth
still helps.

## Linux

The identity is the volume and the inode. The volume is the file system's UUID (`pg_platform::dir`: from the kernel,
or from udev's database, or an ext file system's `f_fsid`, which the kernel builds from it), so a device renumbered at
a restart keeps its files. It is found per entry, because a mount has a device of its own (`/` and `/proc` are two
devices in the tests). A device without one (procfs, tmpfs, a FAT volume udev doesn't know), or a clone of another's,
keeps its device number, which can change across a restart; the scan counts it (`ScanNote::VolumeFallback`). The Linux tests run in CI on each commit (the Linux job runs the
catalog, scan and platform tests).

- **Birth time** is `statx` btime (`std::fs::Metadata::created()`). A file system that gives none (drvfs) gives NULL,
  and NULL matches any creation time: the file is still the same file by its identity (S-17.7 decision 2).
- **Refused:** NFS, CIFS and SMB roots, by their `statfs` magic numbers (`pg_scan::refusal`, from `pg_platform::dir::fs_type`). Their inode numbers come
  from the server and can be unstable or duplicated. FAT (`vfat`, `msdos`) and exFAT roots, as on Windows: they store
  no inode numbers, so the kernel derives each one while the volume is mounted, and a file's number is not the same
  after a remount. The kernel's pseudo file systems (`/proc`, `/sys`, ...). A mount under a root is checked the same
  way (`refusal::check_mount`). Every other type is accepted, `9p` (WSL's drvfs) included. The other Unix systems refuse every
  root (`refusal::check_root` fails closed). Each refusal is a `pg_model::refusal::RefusedReason`, worded by the client.
- **Bind mounts:** a folder whose identity is already on the walk's path is not descended and is counted as a loop
  (`ScanStats::loops`, S-17.7 decision 11).
- **Open files:** a catalog or thumbnail cache that another process has open is not rebuilt or started over. Each holds
  `<db>.live` shared while it is open, and a rebuild or a start-over takes it exclusively (`crates/schema`). Before
  S-17 Unit F, Linux moved an open catalog aside and the other process went on writing to the moved file, while Windows
  refused the rename; both now refuse.
- **Tests:** the Linux job runs the whole workspace (`just test`), with the five library tests that were ignored on
  Linux before S-17 Unit F. Two tests need a file that can't be read or written (the borders file, the writeback copy);
  as root that can't be made, so they are skipped there, and the container runs them as the user `pg` (`docker/check.py`), where they check.
- **Timed:** a 10,000-file library scans in about 1.6 s warm and 4.8 s cold on WSL's ext4 disk, and a rescan after
  renumbering pairs every file with its row (`docs/PROFILING.md`, Measurements, 2026-11-07). The Windows run (10,000
  files on `D:`, first scan 1.316 s) and the cold VM after `wsl --shutdown` (1.353 s) are in the same section.

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
6. Count entries dropped for `FileId == 0` as errors, so an omission is at least visible. (Done in S-17.7: the folder is unreadable and reported.)

## Reading the MFT instead of listing folders (proposed, 2026-11-01)

What Everything and WizTree do: read the volume's `$MFT` directly, in large sequential chunks, and rebuild the
tree from each record's parent reference.

**Admin rights.** It needs a volume handle (`\.\E:`), which only an elevated process gets: unelevated,
`fsutil fsinfo ntfsinfo E:` is refused ("Access is denied"). On this machine elevation is silent: the user is an
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
