# I/O

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).


- **HDD read order (`eval/bench_hdd.py`, 2026-10-29): read in MFT order, or keep ≥ 8 reads in
  flight. Either gives ~3× path order and reaches the disk's sequential speed.** Unbuffered reads
  (no Windows cache), each order on its own block of 7,500 consecutive files of the test folder on E:
  (`--dense 7500`):

  | Order | files/s | MB/s | 70M files, I/O only |
  |---|---|---|---|
  | path (sorted walk), 1 read at a time | 557 | 64 | 1.5 days |
  | **MFT record number** | **1,699** | **228** | **0.5 days** |
  | physical cluster (LCN) | 1,657 | 234 | 0.5 days |
  | path, 8 reads in flight (drive NCQ reorders) | 1,603 | 235 | 0.5 days |
  | physical cluster, 8 in flight | 1,399 | 252 | 0.6 days |
  | random | 188 | 28 | 4.3 days |

  - Even though path, MFT and cluster order almost coincide in this folder (rank correlation
    0.996–0.999), path order one read at a time loses 2/3 of the speed. Small local reorderings
    each cost a disk rotation.
  - **Caveat: this folder was copied in one go, so it is nearly ideal** (1.2% of files fragmented).
    Older parts of the 70M library are likely more scattered, which favors MFT or cluster order
    even more. The Rust scanner should read the MFT (or USN journal), sort by record number, and
    keep a deep read queue.
  - Sparse sample (every 25th file, `--stride 25`): every order runs at ~140 files/s (random 125),
    and 8 in flight gives ~245. When files are far apart, each read pays a seek whatever the order;
    only queue depth helps.
  - At ~1,600 files/s the disk is no longer the bottleneck for a 70M pass; decoding is.
- **Read buffers (`pipeline::Options::budget`, 2026-11-01): the pass's file buffers must cover a file per
  read in flight and per worker, since a worker holds its file while it decodes it.** `gallery import`,
  8 readers, 12 workers, on the first 1,000 files (6.3 GB, 24 MP Sony A6000 JPEGs, 6 MB average) of
  a camera folder on another drive, in on-disk order, purged from the file cache before each run (an unbuffered
  open drops a file's cached pages), three runs each in rotating order, medians:

  | Buffers | MB/s |
  |---|---|
  | no pool (HEAD 2e1b6ca: a channel of 32 files, a `Vec` per file) | 255 |
  | 64 MB | 221 |
  | 128 MB | 250 |
  | 240 MB (12 MiB × (8 + 12), the default) | 268 (HEAD in the same rounds: 270) |
  | 256 MB | 268 |
  | 512 MB | 266 |

  - The disk, not the CPU, sets the pace: ~40 files/s. Below a file per slot, readers wait for workers to
    hand buffers back.
  - Without a pool, the channel bounded files, not bytes: 52 camera files in flight is 300–600 MB, and a
    folder of 100 MB panoramas held 5 GB.
  - One pass per block of files read once (no purge) was useless: the same build read 175–248 MB/s on
    different blocks of the folder, more than the budgets differ.
  - **Again with the pool that replaces a too-small idle buffer** (145ec5b; the runs above were of a pool that
    only reused a buffer already large enough, and otherwise made a new one while the budget had room), at
    1d3685b, the same files and method, 2026-11-02. Medians: no pool (2e1b6ca) 278 MB/s, 64 MB 222,
    128 MB 267, 240 MB 277, 256 MB 278, 512 MB 277. The knee is where it was, so the default stays at 240 MB. The same
    build without a pool read 278 MB/s here and 255 MB/s in the first session, so only rounds of one session
    compare.
  - That pool still made a buffer for nearly every file: only bytes bound the files in flight, so readers read
    ahead until the budget was full (HEAP_PROFILE.md "The read pool still makes a buffer per file").
  - **Again with the files in flight bounded by count too** (`Options::slots`; the pool then holds at most that
    many buffers, reused and grown in place, and the readers' channel has that many places), 2026-11-02. The
    first 400 of the same files (2.6 GB), the same method, three rounds of slots × budget, medians in MB/s:

    | Slots \ budget | 128 MB | 240 MB | 512 MB |
    |---|---:|---:|---:|
    | 20 (readers + workers) | 249 | 271 | 272 |
    | 32 (readers + 2 × workers, the default) | 261 | 273 | 273 |
    | 56 (readers + 4 × workers) | 265 | 274 | 273 |
    | no bound by count | 261 | 273 | 273 |

    The runs of a configuration were within 0.4 s of each other (9.6–10.1 s, but for 20 at 128 MB). The bound by
    count costs nothing from 32 slots on, and 240 MB is still the knee, so both defaults are those. A first try
    on the 1,000 files was dropped: its third round ran 30% slower in every configuration, from some other load
    on the machine.
- **Paths over 260 characters.** std makes a long path verbatim (`\\?\`) itself, and the scanner's `CreateFileW` is
  handed the verbatim form (`crates/platform/src/native.rs`, `\\?\UNC\` for a share). FFmpeg reads its files through
  std (`crates/video/src/ffi.rs`), so it gets the long form too. Probed (ExifTool 13.60, a 375-character path): from
  the command line, a write and a read reach such a file with the plain path, and with `WindowsLongPath=1` too;
  through the stay-open argument file, with the plain path. The option isn't turned on.
- **Folders ending in a dot or a space.** A plain path drops the dot or space from a folder's name on Windows, so
  `D:\a.\x.jpg` opens `D:\a\x.jpg`; the verbatim form keeps it. Every stored path reaches the disk verbatim
  (`pg_platform::names::disk_path`). Probed (2026-11-10, Windows 10, folders `a.` and `a` side by side):
  - ExifTool 13.60 takes the verbatim form, through the stay-open argument file and from the command line, with a
    non-ASCII name too: `-Rating=5` on `\\?\…\a.\x.jpg` wrote `a.\x.jpg`, and on the plain path it wrote `a\x.jpg`.
    The write-back and the crop give it the verbatim path. (An earlier note here said ExifTool did not read `\\?\`:
    wrong for 13.60.)
  - The shell takes neither form: `SHCreateItemFromParsingName` parses the plain `a.` as `a`, and fails on any
    verbatim folder with `E_INVALIDARG`. The folder dialog starts at the nearest folder whose names survive.
  - Qt's `QFile` normalizes the path with `GetFullPathNameW`, which drops the dot even behind `\\?\`. The video
    player is handed the file already open (`pg_platform::file::player_handle`).
- **ExifTool's argument file and odd names.** Probed in WSL (Ubuntu-26.04, ExifTool 13.50), with absolute paths, as a
  rating write (`-charset filename=utf8`, `-XMP:Rating=4 -o out in`) through `-@`. The argument file carries a name
  exactly, except one with a line break (split at it) or one ending in a space, tab, vertical tab, form feed or CR
  (trimmed: "Can't create PNG files from scratch"). A leading space or tab, a space, tab or CR inside a name, `#`, `-`,
  a quote, `[CSTR]` and raw bytes all come through. The process-argument route carries a name's bytes exactly, and
  `crates/exiftool/src/lib.rs` (`needs_argv`) sends the names the argument file can't carry through it. But a name
  ending in whitespace still can't be written by either route: ExifTool takes the file's type from its extension
  (`png`, followed by a space), refuses it ("Can't create PNG files from scratch"), so the write fails with an error and leaves no copy.
  Its argument file lines are absolute paths: a relative name starting with `#` is a comment and one starting with `-`
  is an option. Windows can't hold any of these names; its surrogate names were probed separately (above).
  A `#[CSTR]` line (ExifTool's C-escape form, `\n`, `\r`, `\t`, `\"`, `\\`) carries the same names except those with
  `$` or `@`, which it rewrites (probed on both: WSL 13.50 and Windows 13.60). So it isn't a route for every name, and
  the code doesn't use it: a plain line carries `$` and `@` exactly, and the argument route carries the rest on Unix.
  On Windows the argument route can't carry a lone surrogate (Windows' argv is lossy), so the plain line is its route.
- **HDD read, path order (robocopy E:→D:): 70 MB/s, ~410 files/s** (consistent with the 557 files/s
  above, as robocopy also writes).
- **The harness embeds at only 80–117 img/s against 410 img/s of GPU capacity**, so decoding
  (PIL, Python DataLoader) is the bottleneck. The Rust decode path matters more than the model.
