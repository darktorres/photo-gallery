# Security and data-integrity flow review

This report lists the findings of a flow-based review of the photo-gallery repository. The review started from each untrusted input and each persistent or destructive write, then traced it end to end across the files it touches. It was run as a read-only workflow and produced 189 findings across 37 flows.

**Status: validated.** Each finding was checked against the code it cites. Verdicts: 175 confirmed, 13 partial, 1 refuted. A confirmed verdict means the mechanism is in the cited code. Where a finding also claims a consequence that was not traced, the note says so. Duplicates are marked and point to the finding they repeat.

## Method

1. A mapping step enumerated the untrusted inputs (files being decoded, metadata parsing, the catalog file, environment variables, command-line options, network responses) and the persistent or destructive writes (the user's originals and sidecars, the catalog and its migrations, caches, temp files, removals). It also recorded the process and FFI boundaries.
2. One agent per flow then read each file on the path in full and traced callers and callees.
3. Findings are graded by severity (high, medium, low) and category (security, data-integrity), with a confidence level. Confidence is high only where the trigger was traced through every step.

## Coverage

| Measure | Value |
|---|---:|
| Flows traced | 37 |
| Findings | 189 |
| Sites recorded as not flows | 17 |

Sites recorded as not flows are mostly test-only code (test modules and integration tests).

Coverage gap: 106 of the 199 files that the mapping step listed for the flows were not read by any trace. Those files may hold further defects, and a targeted pass is still needed.

## Summary

| Severity | Count |
|---|---:|
| high | 4 |
| medium | 56 |
| low | 129 |

| Category | Count |
|---|---:|
| data-integrity | 148 |
| security | 41 |

## Verdicts

| Verdict | Count |
|---|---:|
| CONFIRMED | 175 |
| PARTIAL | 13 |
| REFUTED | 1 |

## Flows

| ID | Kind | Name | Findings |
|---|---|---|---:|
| F01 | untrusted-input | Library folder scan: directory listing and file identity | 5 |
| F02 | untrusted-input | Image decode for thumbnails and grid pictures | 3 |
| F03 | untrusted-input | Header and metadata parsing: EXIF, TIFF, XMP, PNG/JPEG/WebP containers during import | 1 |
| F04 | boundary | Video container and frame reading through FFmpeg FFI | 5 |
| F05 | untrusted-input | Catalog database file opened and read (untrusted local SQLite file) | 8 |
| F06 | persistent-write | Thumbnail cache file open, read and write | 5 |
| F07 | untrusted-input | Environment and default paths resolving the catalog, prefs and home folders | 4 |
| F08 | untrusted-input | Desktop command-line options that trigger file writes or network calls | 7 |
| F09 | untrusted-input | CLI arguments and borders tool file reads and writes | 5 |
| F10 | untrusted-input | Folder picker result and start folder | 2 |
| F11 | untrusted-input | QML file dialogs, drag and drop and external URL opens | 3 |
| F12 | network | Yandex reverse-image search: upload and similar-page requests | 4 |
| F13 | untrusted-input | Yandex response parsing (JSON and HTML state) | 6 |
| F14 | network | Yandex result image download and decode check | 4 |
| F15 | persistent-write | Yandex save: writing a result into the user's chosen folder | 4 |
| F16 | untrusted-input | Recorded Yandex session replay (fixture folder) | 5 |
| F17 | persistent-write | Original file write-back (ratings and tags written into user photos) | 5 |
| F18 | boundary | ExifTool process spawn and stdin command protocol | 6 |
| F19 | persistent-write | Crop and copy export to an output folder | 5 |
| F20 | persistent-write | Catalog migration backup, rebuild and replacement | 5 |
| F21 | persistent-write | Catalog sidecar deletion and move-aside | 5 |
| F22 | persistent-write | Lock and live sidecar files beside catalog and log | 3 |
| F23 | persistent-write | Log file append, rotation and perf-log sinks | 6 |
| F24 | persistent-write | Preferences and unreadable-file quarantine (JSON store) | 8 |
| F25 | persistent-write | Recording and screenshot writes to caller-named paths | 5 |
| F26 | boundary | Qt and cxx-qt bridge boundary (QML, images, file watcher) | 3 |
| F27 | boundary | Unsafe Windows API calls (file, volume, directory, console, dialog, message box) | 7 |
| F28 | untrusted-input | Linux and POSIX system file reads (sysfs, udev, /proc, statfs) | 3 |
| F29 | boundary | Audio backend library load (PipeWire) | 1 |
| F30 | boundary | FFmpeg and ExifTool discovery by file path | 5 |
| F31 | boundary | Build scripts reading environment and resource folders | 5 |
| F32 | network | Python tool: ExifTool download, verify and extract | 3 |
| F33 | network | Python tool: FFmpeg archive download and zip extraction | 6 |
| F34 | boundary | Python tools: source download and build outputs (Qt, FFmpeg, libjpeg-turbo) | 9 |
| F35 | persistent-write | PowerShell scripts: deletes, moves and writes under repo or chosen folders | 13 |
| F36 | persistent-write | Eval harnesses: decoded output and benchmark file writes (C++ and Python) | 13 |
| F37 | persistent-write | Developer tool: catalog-copy probe examples (copy and delete trees) | 2 |

## F01: Library folder scan: directory listing and file identity

- Kind: untrusted-input
- Entry: crates/platform/src/dir.rs:23 (std::fs::read\1dir) and crates/platform/src/dir.rs:363 (CreateFileW folder open, Windows Lister)
- Endpoint: crates/scan/src/lib.rs:371 scan() writes file rows through crates/catalog/src/writers.rs
- Summary: Folder names and entry names from the user's disk (including reparse points, junctions, symlinks and case variants) are listed, identified by volume and file id, checked against path rules and root volume (scan/src/lib.rs:354-361), then stored as catalog rows. Untrusted names and FS metadata cross into SQL writes.

### SEC-001 [medium/security] Child and root folders opened by path after the reparse check, so a swapped junction or symlink is listed — CONFIRMED

- Verdict: CONFIRMED. Open-by-path follows links (dir.rs:369 backup semantics, no reparse flag; dir.rs:505, 517). Reparse test is on the listing only. Race is real.

- Confidence: medium
- Kind: traversal
- Trigger: A library subfolder is listed as a plain folder, then replaced before the walk or a worker opens it by a junction (Windows, creatable without admin) or a symlink (Unix) pointing outside the library. The walk opens the folder by path (dir.rs:369 has no FILE\1FLAG\1OPEN\1REPARSE\1POINT; dir.rs:505 uses fs::metadata; dir.rs:517 uses read\1dir, all following links), and lists it. The reparse test is only the parent's listing value (lib.rs:680). Workers list up to AHEAD (50 000) entries ahead, so the window can be long. The same race applies to the root: refusal.rs:48-49 checks the link on one handle, and lib.rs:835 opens the root again by path.
- Consequence: Media files from the link target are recorded under the library path. On Windows no mount check runs for them (device is None, so other\1device is false and refusal::check\1mount is Ok), and their identity is (library volume, target file id). A target file id equal to a real library row's id is treated as that row (record\1file, writers.rs:230), so ratings and the row's name and folder are moved to a file that is not the library's. Later analysis and rating write-back reach the file through the same stored path (names.rs:74-82, disk\1path). The documented guarantee that a reparse folder is never followed does not hold under this race.
- Suggested fix: Open every folder the walk descends into with FILE\1FLAG\1OPEN\1REPARSE\1POINT (Windows) and refuse it when the opened handle is a reparse point or its final path differs from the expected path; on Unix open with O\1DIRECTORY\1O\1NOFOLLOW (or walk with openat) and compare fstat dev/ino with the lstat the parent listing gave. Apply the same check to the root in walk\1paced, not only in refusal::check\1ntfs.
- Locations:
  - `crates/platform/src/dir.rs:369` FILE\1FLAG\1BACKUP\1SEMANTICS,
  - `crates/platform/src/dir.rs:505` let meta = std::fs::metadata(&path)?;
  - `crates/platform/src/dir.rs:517` for entry in std::fs::read\1dir(&dir.path)? {
  - `crates/scan/src/lib.rs:680` && !e.reparse
  - `crates/scan/src/lib.rs:701` None =\1 match open\1folder(lister, rules, &job.path) {
  - `crates/scan/src/lib.rs:835` let root\1dir = open\1folder(lister, rules, root).with\1context(\1\1 format!("can't open {}", rules.display(root)))?;
  - `crates/scan/src/lib.rs:1062` stack.push(Frame { job: next, dir: None, pooled: !other });

### SEC-002 [low/data-integrity] Non-regular files (FIFO, socket, device) with media names are recorded and queued for analysis — PARTIAL

- Verdict: PARTIAL. Listing keeps FIFOs and devices with media names (dir.rs:527, scan lib.rs:238); the write path has the is\1file check. Blocking open not checked.

- Confidence: medium
- Kind: wrong-target
- Trigger: A FIFO, socket or device node named like a media file (for example a FIFO named photo.jpg) is in a library folder on Unix. list() classifies it only by is\1dir and is\1symlink, so it is kept as an ordinary file (ext Some), gets a row and is queued for analysis. The analysis open is a plain open with no O\1NONBLOCK, which blocks on a FIFO with no writer.
- Consequence: An analysis worker blocks indefinitely on the FIFO, stalling the import pass and holding its buffer. A device node may be opened with side effects. No data is lost, but the walk records a row for a non-file. The write-back path has an is\1file check (writeback/src/file.rs:50) that the scan lacks.
- Suggested fix: In Unix Lister::each, classify by the lstat file type and treat anything that is not a regular file or directory as a skipped entry (like a reparse point), so it is never recorded or queued.
- Locations:
  - `crates/platform/src/dir.rs:527` folder: meta.is\1dir(),
  - `crates/platform/src/dir.rs:528` reparse: meta.file\1type().is\1symlink(),
  - `crates/scan/src/lib.rs:238` let ext = if e.folder { None } else { Ext::of\1name\1bytes(name.as\1bytes()) };
  - `crates/writeback/src/file.rs:50` if !std::fs::symlink\1metadata(&disk)?.is\1file() {

### SEC-003 [low/security] A folder name holding U+0000 is cut short by CreateFileW and lists another folder — CONFIRMED

- Verdict: CONFIRMED. Wide path set with an embedded NUL (dir.rs:360); CreateFileW stops at the NUL. Name check missing at scan lib.rs:234.

- Confidence: medium
- Kind: wrong-target
- Trigger: An NTFS folder whose name contains U+0000, which NTFS can hold when created through the native API but Win32 rejects. list() keeps any name (lib.rs:238, no NUL check; names.rs:15-18 Name::from\1wide accepts it). open\1folder (lib.rs:267) builds the path with the NUL inside, and Lister::open (dir.rs:360) passes it to CreateFileW, which reads only up to the first NUL.
- Consequence: The walk lists the folder named by the text before the NUL, or the parent itself when the name starts with NUL, and records its files under the NUL-containing path that does not exist on disk. Each parent-level file is already in seen, so most duplicates are skipped, but the walk re-enters folders under the bogus path and can record rows under paths that do not exist.
- Suggested fix: Reject a listed name that contains U+0000 in list() (the folder is then unreadable and skipped, or the entry is ignored) so a NUL never reaches a Win32 path.
- Locations:
  - `crates/platform/src/dir.rs:360` self.wide.set(&\1path, &\1SEP\1\1);
  - `crates/scan/src/lib.rs:234` if e.file\1id == 0 && !name.is\1dot() {
  - `crates/scan/src/lib.rs:267` lister.open(&units\1of(&rules.join(path)))
  - `crates/platform/src/names.rs:15` pub fn name\1from\1native(units: &\1Unit\1) -\1 Name {

### SEC-004 [low/data-integrity] Volume identity for a symlink on another device can come from its target's file system — CONFIRMED

- Verdict: CONFIRMED. volume\1id falls through to ext\1fsid, which calls statfs on the probe path (dir.rs:611), following a symlink.

- Confidence: low
- Kind: wrong-target
- Trigger: On Linux, an entry on a device other than its folder's is a symlink, and its device has no UUID in the kernel or udev (so the first two lookups fail). The probe opens the link with O\1NOFOLLOW, which fails, and then ext\1fsid calls statfs on the same path, which follows the link to its target. The probe is made before the reparse test (lib.rs:1010-1017), so a symlink is probed even though it is then skipped.
- Consequence: The link's device is given the f\1fsid of the target's ext file system. If that equals the identity of a volume the library uses, entries on the link's device share identities with that volume's files, and the dedupe or record\1file logic can merge two different files. This is rare: it needs no udev UUID and an ext target on another device.
- Suggested fix: Use an fstatfs on the O\1NOFOLLOW probe fd (or skip ext\1fsid for a symlink), so the probe reads the entry itself as its comment says, and do not probe reparse entries at all.
- Locations:
  - `crates/platform/src/dir.rs:567` uuid\1from\1kernel(probe).or\1else(\1\1 uuid\1from\1udev(dev)).or\1else(\1\1 ext\1fsid(probe))
  - `crates/platform/src/dir.rs:575` .custom\1flags(libc::O\1NONBLOCK \1 libc::O\1NOFOLLOW)
  - `crates/platform/src/dir.rs:611` if unsafe { libc::statfs(c.as\1ptr(), st.as\1mut\1ptr()) } != 0 {
  - `crates/scan/src/lib.rs:1012` lister.volume\1of\1device(device, &probe)

### SEC-005 [low/security] The identity probe opens an entry on another device before any refusal — PARTIAL

- Verdict: PARTIAL. Probe opens foreign-device entries before refusal (dir.rs:573, scan lib.rs:1010 before 1040). Device side effects are speculative.

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A library root contains a bind mount of another file system with device nodes or FIFOs (for example devtmpfs /dev or a tape or serial device node). The first entry on each foreign device is probed by volume\1of\1device, which opens it with read and O\1NONBLOCK (dir.rs:573-576) before the walk runs check\1mount (lib.rs:1040) or the reparse test (lib.rs:1017).
- Consequence: Opening a device node can have side effects (a tape rewinds on close, a serial line changes state), and the open happens for an entry the walk then refuses. The entry is not read as content, so no catalog row is affected.
- Suggested fix: Probe only directories and regular files (lstat first, skip non-regular entries), or read the identity from the folder's own mount point (statfs on the folder) instead of opening an entry.
- Locations:
  - `crates/platform/src/dir.rs:573` let file = std::fs::OpenOptions::new()
  - `crates/scan/src/lib.rs:1010` let probe = units\1of(&rules.join(&job.path.child(&e.name)));
  - `crates/scan/src/lib.rs:1040` let verdict = match mounts.get(&volume) {

## F02: Image decode for thumbnails and grid pictures

- Kind: untrusted-input
- Entry: crates/platform/src/file.rs:33 (CreateFileW open of the original) feeding crates/jobs/src/pipeline.rs:185 run()
- Endpoint: crates/codec/src/decode.rs:118 decode\1into and crates/thumbcache/src/writer.rs:358 put (thumbnail row written to cache)
- Summary: Untrusted JPEG/PNG/TIFF/WebP/GIF bytes from the user's originals are decoded (with limits.rs size checks) into pixels, resized, and stored as thumbnails. Hostile dimensions or chunk structure reach hand-written decoders; output goes into the thumbnail cache file.

### SEC-006 [medium/security] Whole original read into memory with no size cap before any decoder check — CONFIRMED

- Verdict: CONFIRMED. Whole files read with no cap before decode: pipeline.rs:588-589, render maker.rs:131, thumbs placeholders.rs:131.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: A library original of several GB (for example an uncompressed multi-GB scanned TIFF), or a file whose metadata size is huge. The thumbnail pass (Read::File for every still) and the on-demand grid path read it entirely before any dimension or decode-limit check runs.
- Consequence: The whole file is allocated and held for the duration of the decode. The 512 MiB decode cap bounds only the decoded image, not the compressed input. Vec::reserve\1exact in Pool::take aborts the process on allocation failure instead of returning an error, so a large original can crash the running app mid-pass. The pool budget is also exceeded by one file (a file larger than the budget is granted alone).
- Suggested fix: Check the file's size against a cap (or a per-file budget) before reading, and refuse or skip over-cap originals with an Undecodable-style error. Use try\1reserve for the lease and for read\1to\1end so that a failed allocation is an error, not an abort. Apply the same rule to Maker::read\1file and the placeholders fallback.
- Locations:
  - `crates/jobs/src/pipeline.rs:588` let mut lease = pool.take(i, size)?;
  - `crates/jobs/src/pipeline.rs:589` Some(file.read\1to\1end(&mut lease.bytes).map(\1\1\1 lease))
  - `crates/jobs/src/pipeline.rs:789` bytes.reserve\1exact(size);
  - `crates/render/src/maker.rs:131` let result = std::fs::File::open(path).and\1then(\1mut f\1 f.read\1to\1end(&mut self.file)).map(drop);
  - `crates/thumbs/src/placeholders.rs:131` read = std::fs::read(&disk)?;

### SEC-007 [medium/security] Per-decode pixel buffers (up to 512 MiB each) sit outside the pipeline's memory budget — CONFIRMED

- Verdict: CONFIRMED. Pipeline budget counts read buffers only (pipeline.rs:228). Decode output of up to 512 MiB sits outside it (limits.rs:14, convert.rs:52-58).

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A folder of small hostile PNG, GIF, WebP or TIFF files, each declaring dimensions just under the 512 MiB decode cap. A deflate-bomb PNG of about 1 MB can declare an RGBA image of roughly 11,500 px square. The pipeline runs one worker per logical CPU (pipeline.rs:53-56), and all of them decode at once.
- Consequence: The pipeline budget (pipeline.rs:228, Budget::new(opts.buffer\1bytes)) counts only file read buffers, not decode output. A worker decoding an 8-bit RGBA image at the cap holds the RGBA buffer (512 MiB, in out.pixels) and the flattened RGB buffer (384 MiB, in raw) at the same time, so about 900 MiB peak per worker. With 12 workers the aggregate can reach several GB, outside any bound, and can abort the process at allocation. The buffers are released only by trim after each file.
- Suggested fix: Account for decode output in the pass's budget (reserve the decoded size of each worker's decode from the shared Budget before decoding, as Codecs already does for the file bytes), or lower the per-decode cap for thumbnail passes. Alternatively, reduce peak memory by flattening RGBA in place without a second full-size buffer.
- Locations:
  - `crates/codec/src/codecs/limits.rs:14` pub(super) fn reserve(color: ColorType, w: u32, h: u32) -\1 ImageResult\1Limits\1 {
  - `crates/codec/src/codecs/convert.rs:52` ColorType::Rgba8 =\1 {
  - `crates/codec/src/codecs/convert.rs:55` let mut flat = Rgb { width: 0, height: 0, pixels: std::mem::take(raw) };
  - `crates/codec/src/codecs/png.rs:51` reserve(color, w, h)?;
  - `crates/jobs/src/pipeline.rs:228` let budget = opts.budget.clone().unwrap\1or\1else(\1\1 Arc::new(Budget::new(opts.buffer\1bytes)));

### SEC-008 [low/data-integrity] Thumbnail stored in the cache, then failure cleared in the catalog in a separate commit — CONFIRMED

- Verdict: CONFIRMED. put\1batch (thumbs lib.rs:337) and clear\1failures (lib.rs:342) are separate stores, so a crash between them leaves a stale failure row. The test cite was not checked.

- Confidence: medium
- Kind: crash-window
- Trigger: The process is killed or loses power after cache.put\1batch commits the thumbnail rows (thumbs/src/lib.rs:337) and before catalog.clear\1failures commits (thumbs/src/lib.rs:342). The two databases are separate, so there is no shared transaction.
- Consequence: The file has a stored thumbnail, but its retry=false row stays in the Thumbs failure ledger. The thumbs test at thumbs/src/lib.rs:756-800 states that such a row keeps the file out of the owed-placeholders list, so its placeholder is not made until the user retries or the file changes. The ledger shows a stale failure for a file that now thumbnails correctly. Nothing is lost and the next Retry repairs it.
- Suggested fix: Clear the ledger rows after a successful store and make the ordering explicit, or re-check on the next pass that a stored thumbnail's file has no ledger row (clear it lazily). A cross-database atomic write is not available, so an idempotent recheck is the practical fix.
- Locations:
  - `crates/thumbs/src/lib.rs:337` let stored = cache.put\1batch(thumbs, Some(thumb))?;
  - `crates/thumbs/src/lib.rs:342` match catalog.clear\1failures(FilePass::Thumbs, &made\1known) {

## F03: Header and metadata parsing: EXIF, TIFF, XMP, PNG/JPEG/WebP containers during import

- Kind: untrusted-input
- Entry: crates/container/src/lib.rs:179 jpeg\1import\1bytes and crates/container/src/lib.rs:278 png\1import\1bytes (and metadata reads from bytes)
- Endpoint: crates/metadata/src/lib.rs:55 Meta::read result stored by crates/import/src/lib.rs:58 import into catalog rows (crates/catalog/src/writers.rs)
- Summary: EXIF date, orientation, XMP ratings and TIFF IFD offsets are parsed from attacker-controlled header bytes with manual offset arithmetic (tiff.rs entries/uint/ascii/bytes; xmp.rs xmp\1properties). Parsed values (dates, orientation, rating, pair keys) become catalog columns.

### SEC-009 [low/data-integrity] BigTIFF Exif IFD pointer narrowed to 32 bits, dropping the Exif dates — CONFIRMED

- Verdict: CONFIRMED. tiff.rs:161 narrows IFD8 offsets to u32, so exif.rs:61 drops the Exif IFD above 4 GiB.

- Confidence: high
- Kind: overflow
- Trigger: A BigTIFF file (magic 43) larger than 4 GiB whose IFD0 Exif pointer (an IFD8 entry) holds an offset at or above 2^32, for example a custom directory a writer appended at the end of the file. IFD0 itself is read with a full 64-bit offset (tiff.rs offset\1at), so only the pointer to the Exif IFD is affected.
- Consequence: exif.rs reads exif\1ifd through tiff.uint, which returns None for an IFD8 above u32::MAX, so no Exif IFD is walked. DateTimeOriginal and DateTimeDigitized are then absent: taken falls back to XMP or stays NULL in analysis.taken, and date filters miss the picture. Orientation and rating from IFD0 still read.
- Suggested fix: Read the Exif IFD pointer as a 64-bit offset in BigTIFF (convert IFD8 with the same path as offset\1at) and keep the u32 narrowing only for the numeric value reads (orientation, rating, percent).
- Locations:
  - `crates/metadata/src/exif.rs:61` EXIF\1TAG\1EXIF\1IFD =\1 exif\1ifd = tiff.uint(&e),
  - `crates/metadata/src/tiff.rs:161` TIFF\1LONG8 \1 TIFF\1IFD8 =\1 self.u64\1of(v).and\1then(\1n\1 u32::try\1from(n).ok()),
  - `crates/metadata/src/tiff.rs:84` if self.big { usize::try\1from(self.u64\1at(i)?).ok() } else { Some(pg\1model::num::index(self.u32\1at(i)?)) }

## F04: Video container and frame reading through FFmpeg FFI

- Kind: boundary
- Entry: crates/video/src/ffi/io.rs:34 read\1source (C callback filling FFmpeg's buffer from a Rust Source) driven by crates/video/src/ffi.rs:135 avformat\1open\1input
- Endpoint: crates/video/src/ffi.rs:191 av\1read\1frame / :388 avcodec\1receive\1frame frames returned to crates/thumbs/src/lib.rs and crates/render/src/player.rs
- Summary: Untrusted video files are opened by an FFmpeg build loaded at runtime (unsafe function-pointer table, ffi/load.rs:167-176) and read through a Rust callback (io.rs:34-40, raw slice from C-supplied size). Frame data and metadata dictionaries come back across the unsafe FFI boundary.

### SEC-010 [medium/data-integrity] Mid-file read error is taken as end of stream and a stale frame is cached — CONFIRMED

- Verdict: CONFIRMED. read\1source returns EIO (io.rs:44); read\1packet returns false (ffi.rs:191); the frame loop ends and picks the last earlier frame (lib.rs:207, 227).

- Confidence: high
- Kind: missing-check
- Trigger: A video whose file is read through a drive or share that fails after the open succeeded (the drive drops during the packet loop). read\1source returns AVERROR\1EIO at io.rs:44, av\1read\1frame returns negative, and read\1packet returns false. The loop in frame\1into ends as if the file had ended, and the poster-frame target (10% in) is not reached. The thumbs pass then keeps the last earlier frame, or records Undecodable if none decoded.
- Consequence: The poster thumbnail and its ThumbHash are built from a frame earlier than the one intended, and the thumbnail is written to the cache as a success. The size and mtime check still passes, so it is not retried until the file changes. A transient I/O failure is silently converted into a permanent wrong result.
- Suggested fix: Make read\1packet return a Result that distinguishes AVERROR\1EOF from other negative codes, and propagate non-EOF errors out of frame\1into as an I/O error (the Unopened kind, or a new one) so the ledger classifies the file as Unreadable and retries it, instead of returning a frame.
- Locations:
  - `crates/video/src/ffi/io.rs:44` Err(\1) =\1 AVERROR\1EIO,
  - `crates/video/src/ffi.rs:191` unsafe { (self.api.av\1read\1frame)(self.ptr, packet.ptr) \1= 0 }
  - `crates/video/src/lib.rs:207` 'packets: while input.read\1packet(&mut packet) {
  - `crates/video/src/lib.rs:223` if picker.found.is\1none() {
  - `crates/video/src/lib.rs:227` let f = picker.found.or(picker.last).context("no frame decoded")?;
  - `crates/thumbs/src/lib.rs:293` Ok(data) =\1 {

### SEC-011 [medium/data-integrity] I/O error during open or probe is classified as Undecodable and recorded until the file changes — CONFIRMED

- Verdict: CONFIRMED. Open and probe errors become plain bail strings (ffi.rs:138, 144); MakeError::of\1video only downcasts Unopened (maker.rs:58), so they are Undecodable.

- Confidence: high
- Kind: missing-check
- Trigger: A read error inside avformat\1open\1input or avformat\1find\1stream\1info, such as a drive that drops while the header or moov box is read. read\1source returns AVERROR\1EIO, and Input::open turns the FFmpeg code into a plain bail! string.
- Consequence: The thumbs pass classifies the failure as Undecodable, which is recorded in the failure ledger with retry=false and does not hold the checkpoint back. The video is then not retried until the file changes or the user retries it, even though the failure was an I/O failure that a later pass would likely clear. The unopened-file case (Unopened) is handled correctly, so only the mid-open I/O case is misclassified.
- Suggested fix: Keep the I/O error from read\1source in the Source (or a thread-local set by the callback) and, when avformat\1open\1input or find\1stream\1info fails with a negative code and the source recorded an error, return it as Unopened or an io::Error so of\1video maps it to Unreadable.
- Locations:
  - `crates/video/src/ffi.rs:135` let ret = unsafe { (api.avformat\1open\1input)(&mut ptr, name.as\1ptr(), ptr::null(), ptr::null\1mut()) };
  - `crates/video/src/ffi.rs:138` bail!("{}", error\1text(ret));
  - `crates/video/src/ffi.rs:144` bail!("{}", error\1text(ret));
  - `crates/render/src/maker.rs:58` match e.downcast::\1pg\1video::Unopened\1() {
  - `crates/thumbs/src/lib.rs:126` Failed::Panicked \1 Failed::Undecodable =\1 Some(false),

### SEC-012 [medium/security] Sample aspect ratio from the file sizes the RGB destination buffer without a cap — PARTIAL

- Verdict: PARTIAL. The poster path uses MinSize::Full (video lib.rs:177), so the destination is the full display width. FFmpeg buffer allocation returns an error (ffi.rs:425), not an abort. Thumbnail scales are smaller. The magnitude is overstated.

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A video whose stream sets a large sample aspect ratio (H.264 VUI SAR up to 65535:1, or an MP4 pasp box with a large value) and a moderate coded width. display\1width multiplies the width by the SAR, and to\1rgb allocates a destination of tw x th x 3 for the scaled size, which can be several hundred MB up to FFmpeg's own buffer-size limit (about 2 GiB), per worker. The poster extraction then fails or succeeds with a large transient allocation.
- Consequence: An untrusted file in an imported folder, which the import thumbs pass decodes automatically, forces a very large transient allocation in every worker, possibly exhausting memory or causing the pass to fail. The thumbnail is then discarded by the encoder, so the allocation buys nothing.
- Suggested fix: Reject or clamp an SAR outside a sane range before display\1width, and cap the destination's pixel count (for example at a few tens of megapixels) before Frame::video allocates it, returning an error.
- Locations:
  - `crates/video/src/lib.rs:102` fn display\1width(width: u32, sar: AVRational) -\1 u32 {
  - `crates/video/src/lib.rs:290` let dw = display\1width(w, src.sample\1aspect\1ratio());
  - `crates/video/src/lib.rs:302` let (tw, th) = (scaled(dw, scale), scaled(h, scale));
  - `crates/video/src/lib.rs:303` let mut dst = Frame::video(AVPixelFormat\1AV\1PIX\1FMT\1RGB24, tw, th)?;
  - `crates/video/src/ffi.rs:425` let ret = (frame.api.av\1frame\1get\1buffer)(frame.ptr, 0);

### SEC-013 [low/data-integrity] Frame time subtraction wraps in release builds — CONFIRMED

- Verdict: CONFIRMED. Time subtraction in lib.rs:254 is unchecked, and the release profile sets no overflow-checks (Cargo.toml).

- Confidence: medium
- Kind: overflow
- Trigger: A crafted video whose best-effort timestamp and stream start\1time are both extreme with opposite signs, so that t - start exceeds i64 range. Release profiles have no overflow checks (Cargo.toml \1profile.release\1 sets only debug).
- Consequence: The wrapped value makes a frame look earlier or later than it is, so the wrong frame is chosen as the poster or as the first frame at or after the target. Result is wrong but memory-safe.
- Suggested fix: Use t.saturating\1sub(self.start) (or checked\1sub with a fallback to at\1ms) so the difference cannot wrap.
- Locations:
  - `crates/video/src/lib.rs:254` .map\1or(self.at\1ms, \1t\1 pg\1model::num::f64\1to\1i64(ticks\1ms(t - self.start, self.time\1base)));
  - `crates/video/src/lib.rs:197` let start = if stream.start\1time() == NOPTS\1VALUE { 0 } else { stream.start\1time() };

### SEC-014 [low/security] Animation player reads the whole file into memory with no size check — CONFIRMED

- Verdict: CONFIRMED. player.rs:76-77 reads the whole animation file into memory.

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A GIF, APNG or WebP file of very large size (hundreds of MB to several GB) opened in the viewer, which calls Player::start and then play().
- Consequence: The whole file is allocated and held in memory for the life of the player, unbounded by the file size. This is the animation path rather than FFmpeg, and it is listed on this flow only as an endpoint.
- Suggested fix: Check the file's length against a cap (or use a bounded reader) before reading, and show the poster instead of loading an oversize animation.
- Locations:
  - `crates/render/src/player.rs:76` let Ok(bytes) = std::fs::read(path) else { return };
  - `crates/render/src/player.rs:77` let bytes: Arc\1\1u8\1\1 = bytes.into();

## F05: Catalog database file opened and read (untrusted local SQLite file)

- Kind: untrusted-input
- Entry: crates/catalog/src/core.rs:258 Catalog::open (Connection::open\1with\1flags, no CREATE flag) and crates/catalog/src/core.rs:125 / :177 Connection::open on rebuild or migration
- Endpoint: crates/catalog/src/browse/read/\1.rs and crates/catalog/src/work.rs queries; schema checks in crates/catalog/src/schema.rs
- Summary: The catalog file (and its -wal sidecar) is opened and queried; user\1version, schema objects and key\1version marks are read back from the file before use. A file edited by another program is accepted or refused by schema and version checks (core.rs:262-270), and its rows drive listing and write queries.

### SEC-015 [high/security] Stored temp path is read, copied over the original and deleted unchecked — CONFIRMED

- Verdict: CONFIRMED. Stored temp is read and removed without a check (writeback lib.rs:732-748). Needs a hostile or corrupted catalog row. Severity should be medium.

- Confidence: high
- Kind: missing-check
- Trigger: Another program (or a corrupted catalog) sets a file\1writes row for an existing file X: kind='rating', value=3, queued=0, attempts=0, temp = the bytes of an unrelated readable file such as C:\Users\u\Documents\other.docx, temp\1value=3. next\1write returns it with temp, and process() routes it to finish\1copy.
- Consequence: The writer overwrites library photo X with the bytes of other.docx, then deletes other.docx (remove\1file on the same stored path). An arbitrary user file is read, written over a photo and deleted.
- Suggested fix: Before reading, copying or removing a recorded temp, require it to equal the sibling temp\1of(original, kind) and to be a regular '.\1name\1.\1kind\1.pg-write' file, as writeback/file.rs remove\1temp already does. Or store the temp's identity at record time and compare it on finish.
- Locations:
  - `crates/catalog/src/queue.rs:154` let temp: Option\1Vec\1u8\1\1 = r.get(3)?;
  - `crates/writeback/src/lib.rs:732` let temp\1path = disk\1path(rules, temp);
  - `crates/writeback/src/lib.rs:733` match std::fs::read(&temp\1path) {
  - `crates/writeback/src/lib.rs:748` let \1 = std::fs::remove\1file(&temp\1path);

### SEC-016 [high/security] Stored folder or file name is used as a path without checking separators or dot-dot parts — CONFIRMED

- Verdict: CONFIRMED. Name::from\1raw does no validation (model name.rs:57) and file\1path\1of pushes the raw name (lookup.rs:103). Needs a corrupted catalog row. Severity should be medium.

- Confidence: high
- Kind: traversal
- Trigger: A files row in a library folder has name = \..\..\Pictures\other.jpg (the leading separator keeps the generated temp name valid), and a queued rating or turn exists for it in file\1writes (queued=0). The row's folder need not be altered.
- Consequence: The writer rewrites the file that the traversed path reaches: a rating write produces an ExifTool copy and copies it over the target, and a turn rewrites its pixels. The importer, thumbnail and placeholder passes read and analyze any readable file the path reaches, and the result is stored and shown.
- Suggested fix: In file\1path\1of and folder\1path, treat a stored name containing a separator, or equal to '.' or '..', and a folder part equal to '..', as a damaged row and return an error. The OS scanner never produces them.
- Locations:
  - `crates/catalog/src/lookup.rs:103` parts.parts.push(Name::from\1raw(name));
  - `crates/catalog/src/lookup.rs:116` Ok(WriteTarget { path: self.file\1path\1of(&folder, &name)?, format, kind })
  - `crates/writeback/src/lib.rs:546` let path = disk\1path(rules, &path\1rule);
  - `crates/import/src/lib.rs:121` let disk = pg\1platform::names::disk\1path\1of(&work.path(item.path, sep));
  - `crates/platform/src/native.rs:237` if (part.is\1empty() && !last) \1\1 part == \1DOT\1 \1\1 part == \1DOT, DOT\1 {

### SEC-017 [medium/data-integrity] Rebuild moves the catalog aside before its replacement takes the name, with no file in between — CONFIRMED

- Verdict: CONFIRMED. move\1aside at rebuild.rs:94 precedes the rename at :96, with no file in between. core.rs:125 creates an empty file on the next open.

- Confidence: high
- Kind: crash-window
- Trigger: Process is killed or power is lost after pg\1dbfile::move\1aside moves catalog.sqlite to .replaced and before the rename of .building onto the catalog path, during a rebuild triggered by a schema change.
- Consequence: On the next start there is no file at the path. Catalog::prepare creates an empty file and makes a fresh baseline catalog with no notice. The carried replacement sits unused at .building and the old catalog at .replaced, so the library appears empty until the user recovers it by hand.
- Suggested fix: Close the window: on prepare, if .building exists and the path is missing, finish the rename (or refuse with a message); alternatively make the replacement's name carry the swap state so a restart can complete it.
- Locations:
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/catalog/src/core.rs:177` conn = Connection::open(db\1file(path)?).with\1context(\1\1 format!("catalog {}", path.display()))?;

### SEC-018 [medium/data-integrity] A missing recorded temp is treated as already copied and the write is recorded as done — CONFIRMED

- Verdict: CONFIRMED. NotFound on the recorded temp is treated as copied (writeback lib.rs:742), then write\1done runs (:747).

- Confidence: medium
- Kind: lost-update
- Trigger: A copy started and its temp was recorded (record\1write\1temp), then the process stopped. Before the restart, the .pg-write temp was deleted by another program or a cleanup. The restart's finish\1copy hits NotFound.
- Consequence: write\1done records the rating as held by the file (file\1ratings and the queue row are cleared), but the original was never written. Later set\1rating finds wanted == in\1file and never queues the write again, so the rating is silently lost from the file.
- Suggested fix: On NotFound, do not record the write as done: fail the job with a reason and keep it queued, or verify the original against a content hash stored with the temp before recording it.
- Locations:
  - `crates/writeback/src/lib.rs:742` Err(e) if e.kind() == std::io::ErrorKind::NotFound =\1 {}
  - `crates/writeback/src/lib.rs:747` catalog.write\1done(&written, Self::rewrote(&written, rewrite))?;

### SEC-019 [low/data-integrity] Upgrade backup is renamed into place without syncing its contents — CONFIRMED

- Verdict: CONFIRMED. Backup copy is renamed into place with no sync (rebuild.rs:45-46). Latent: MIGRATIONS is empty.

- Confidence: medium
- Kind: partial-write
- Trigger: Power loss after the copy is renamed to .pre-vN.bak and before its data reaches disk, followed by the migration committing. Latent: MIGRATIONS is empty in this build, so the path is unreachable until the first migration is added.
- Consequence: The restore point .pre-vN.bak is zero-length or truncated, while the upgrade notice tells the user it holds the old catalog.
- Suggested fix: Sync the partial copy (File::sync\1all on it) before the rename, as the writer's sync() does for its temps.
- Locations:
  - `crates/catalog/src/rebuild.rs:45` std::fs::copy(db\1file(path)?, &partial)
  - `crates/catalog/src/rebuild.rs:46` .and\1then(\1\1\1 std::fs::rename(&partial, &backup))

### SEC-020 [low/data-integrity] Catalog::open writes the WAL setting before it checks what it is opening — CONFIRMED

- Verdict: CONFIRMED. SETTINGS, which switch the file to WAL, run at core.rs:265 before the version check at :268.

- Confidence: high
- Kind: missing-check
- Trigger: A file that another program replaced, or a non-catalog SQLite file at the catalog path, opened by Catalog::open after prepare.
- Consequence: The file header is switched to WAL and -wal and -shm sidecars are created, before the file is refused for its schema or application id. A file the owner must not write is changed.
- Suggested fix: Run the version and schema checks before SETTINGS, or classify the file read-only as dbfile judge does, so a refused file is never written.
- Locations:
  - `crates/catalog/src/core.rs:265` conn.execute\1batch(SETTINGS)?;
  - `crates/catalog/src/core.rs:268` if version != current {
  - `crates/catalog/src/schema.rs:116` pub(crate) const SETTINGS: &str = "PRAGMA journal\1mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign\1keys = ON;

### SEC-021 [low/security] One row with an unknown extension or kind fails the whole folder listing and the import list — CONFIRMED

- Verdict: CONFIRMED. Unknown ext makes grid.rs:67 return an error for the whole listing.

- Confidence: high
- Kind: missing-check
- Trigger: A files row (not missing) written by another program with an ext the build doesn't hold, or a kind it doesn't read.
- Consequence: list\1grid returns an error for any folder that contains the row, and the import work list fails for every file. Listing is unavailable until the row is repaired, with no per-row isolation.
- Suggested fix: Skip or report such rows individually (filter them in the query or log and omit them on decode) so one damaged row does not fail a whole list.
- Locations:
  - `crates/catalog/src/browse/read/grid.rs:67` ext: Ext::of(ext).ok\1or\1else(\1\1 rusqlite::Error::InvalidColumnType(5, format!("ext {ext}"), Type::Text))?,
  - `crates/catalog/src/work.rs:128` kind: crate::types::kind\1at(r, 6)?,

### SEC-022 [low/security] Every open reads all meta keys and all schema SQL from the file into memory — CONFIRMED

- Verdict: CONFIRMED. Schema::read loads every key and SQL text in full (dbfile schema.rs:79-81).

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A catalog file with a very large meta table or very large sqlite\1schema SQL text, as written by another program.
- Consequence: Memory and time on every Catalog::open (once per connection) grow with the file's own row count, with no bound, because Schema::read loads each key and each object's SQL in full.
- Suggested fix: Compare the seeded keys with a count and an EXISTS check per expected key, instead of materializing every key of the table.
- Locations:
  - `crates/dbfile/src/schema.rs:79` let mut keys = conn.prepare(&format!("SELECT quote({column}) FROM main.{table} ORDER BY {column}"))?;
  - `crates/catalog/src/core.rs:274` if let Some(why) = mismatch(&conn, version, current\1expected()?)? {

## F06: Thumbnail cache file open, read and write

- Kind: persistent-write
- Entry: crates/thumbcache/src/lib.rs:95 ThumbCache::open (cache path from cache\1path at lib.rs:396)
- Endpoint: crates/thumbcache/src/lib.rs:284 Connection::open\1with\1flags(READ\1WRITE); writes at crates/thumbcache/src/writer.rs:161 write and :439 total\1and\1cap / :460 evict deleting rows
- Summary: A persistent cache file beside the catalog is opened read-write, healed (maintenance.rs heal), and receives thumbnail BLOBs with LRU eviction and deletes (writer.rs evict/forget). Its size cap comes from PG\1THUMB\1CAP\1GB or --cap-gb (desktop/src/lib.rs:157; cli/src/args.rs cap\1gb). Corrupt cache files are kept aside as bad copies (per the tests at thumbcache/src/lib.rs:861-956).

### SEC-023 [low/data-integrity] Stale cache deleted before its replacement exists; its cap is lost — CONFIRMED

- Verdict: CONFIRMED. remove\1database runs before the replacement is ready (thumbcache schema.rs:331, 339). make\1layout uses the default cap (:443). Doc at lib.rs:84 says the cap is kept.

- Confidence: high
- Kind: crash-window
- Trigger: A cache judged stale (another build's layout or another catalog) that holds a user-set cap from PG\1THUMB\1CAP\1GB or --cap-gb. The process crashes or loses power, or ready() fails (disk full, make\1layout error, heal error), between remove\1database and the new file's commit.
- Consequence: The replacement is created with default\1cap\1bytes (up to 50 GB) instead of the user's cap, or open fails and the cache stays disabled for the session. The doc at lib.rs:84 says the cap is kept, which is not true on this path. Only derived thumbnails are lost otherwise.
- Suggested fix: Build the replacement at a temporary name beside the file, with the cap read from the stale file, and rename it over the stale one (after the sidecars are removed). Alternatively store the cap in a sidecar that survives the rebuild. Either way, the cap must be read before any deletion and written before the old file goes.
- Locations:
  - `crates/thumbcache/src/schema.rs:331` pg\1dbfile::remove\1database(path).map\1err(\1e\1 {
  - `crates/thumbcache/src/schema.rs:339` let (conn, drift) = match ready(path, catalog\1id, cap\1bytes.or(cap), thumbs\1version, !usable) {
  - `crates/thumbcache/src/schema.rs:443` make\1layout(&tx, catalog\1id, cap\1bytes.unwrap\1or\1else(\1\1 default\1cap\1bytes(dir)))?;
  - `crates/thumbcache/src/lib.rs:84` /// A cache of another catalog is deleted and made anew (its cap kept); one another process

### SEC-024 [medium/data-integrity] move\1aside moves the WAL before the main file, contrary to its own safety comment — CONFIRMED

- Verdict: CONFIRMED. move\1aside renames the WAL first (remove.rs:30-37), so a crash before the main file moves leaves the main file without its WAL changes. Rebuild checkpoints first (rebuild.rs:63), which limits the catalog case.

- Confidence: medium
- Kind: crash-window
- Trigger: A crash, power loss, or a failed rename of the main file occurs after the -wal (or -shm/-journal) has been renamed to \1name\1.bad-wal (or .replaced-wal) and before the main file is renamed. The main file is left at its original path without the committed-but-not-checkpointed pages that were in its WAL.
- Consequence: The database at the original path is judged on its checkpointed content only. Recent commits are stranded in the moved WAL, which SQLite will not reattach. For the thumbnail cache this loses derived thumbnails. The same function is used on the catalog rebuild path (catalog/src/rebuild.rs:94, move\1aside(path, ".replaced")), where the live catalog's recent commits (ratings, queued writes, scans) can be separated from it, and the next rebuild proceeds from the older main file.
- Suggested fix: Roll back the already-moved parts (rename them back) if any later rename fails, and correct the doc comment. Or move the main file first and keep the WAL at a name that is not replayed into a new file at the old path (the reason sidecars move first). Either way the claimed invariant must match the order used.
- Locations:
  - `crates/dbfile/src/remove.rs:16` /// the moved files when they are no longer wanted. The sidecars move first, so the main file is never left at 'path'
  - `crates/dbfile/src/remove.rs:30` for part in PARTS {
  - `crates/dbfile/src/remove.rs:34` rename\1no\1replace(&from, &to)
  - `crates/thumbcache/src/schema.rs:406` pg\1dbfile::move\1aside(path, ".bad").with\1context(\1\1 format!("{}: {why}, and can't be moved aside", path.display()))

### SEC-025 [low/data-integrity] Daily heal is skipped until the clock passes a future healed timestamp — CONFIRMED

- Verdict: CONFIRMED. saturating\1sub with a future healed timestamp keeps now less than last, so the heal check is skipped (maintenance.rs:41).

- Confidence: high
- Kind: wrong-target
- Trigger: meta.healed was written with a clock reading ahead of the real time (the clock was wrong, then corrected backwards, or a cache was copied from a machine with a later clock). On every later open now \1 last, so now.saturating\1sub(last) is negative.
- Consequence: The check of totals against entries and the damage scan that heal performs does not run until the wall clock passes that stored time (possibly years). Totals drift stays uncorrected, and damage in the entries is found only by a reader, not at open.
- Suggested fix: Treat a stored time in the future as due: run heal when now \1 last or now - last \1= the interval (clamp a future last to now).
- Locations:
  - `crates/thumbcache/src/maintenance.rs:41` if now.saturating\1sub(last) \1 i64::try\1from(HEAL\1INTERVAL.as\1secs()).unwrap\1or(i64::MAX) {
  - `crates/thumbcache/src/maintenance.rs:45` conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('healed', ?1)", \1now\1)?;

### SEC-026 [low/data-integrity] Open-time judge connection has no busy timeout, so a transient lock fails the open — PARTIAL

- Verdict: PARTIAL. Open-time judge connection has no busy\1timeout (schema.rs:167 versus :420). The BUSY trigger was not traced, as the finding itself says.

- Confidence: low
- Kind: missing-check
- Trigger: Another process holds a connection to the cache file in a state that makes this read return SQLITE\1BUSY (for example during WAL recovery or a checkpoint) when judge() runs at open. The writer's own connections use BUSY\1TIMEOUT, but this one is opened with rusqlite's default of no wait.
- Consequence: open\1writer returns Err (not a damage classification), so ThumbCache::open fails and the app posts CacheDisabled for the session, with no retry. No data is lost. I could not confirm from the code alone which SQLite steps return BUSY in WAL mode, so the trigger is plausible rather than traced.
- Suggested fix: Set conn.busy\1timeout(BUSY\1TIMEOUT) on the judge connection right after Connection::open, as open\1file and the readers already do.
- Locations:
  - `crates/thumbcache/src/schema.rs:167` let conn = Connection::open(path)?;
  - `crates/thumbcache/src/schema.rs:420` conn.busy\1timeout(BUSY\1TIMEOUT)?;

### SEC-027 [low/data-integrity] Damaged cache copies accumulate without bound beside the catalog — CONFIRMED

- Verdict: CONFIRMED. Each damage event moves the whole file to .bad with a new suffix (remove.rs:23-34). Nothing in the code removes them.

- Confidence: medium
- Kind: leak
- Trigger: A cache file is judged damaged more than once (for example a failing drive, or repeated daily-check damage). Each detection moves the whole file (up to the cap, 50 GB by default and more when set by the user) to \1name\1.bad, .bad.1, and so on. Nothing ever removes them.
- Consequence: The catalog's volume fills with full-size copies. A full disk then makes catalog and cache writes fail, and the cache reports NotStored. The user is told only once, by a start-over notice.
- Suggested fix: Keep at most one or two .bad generations (delete the older ones when moving aside), or cap the total size kept, and say so in the notice. The derived-data argument allows deleting them.
- Locations:
  - `crates/thumbcache/src/schema.rs:406` pg\1dbfile::move\1aside(path, ".bad").with\1context(\1\1 format!("{}: {why}, and can't be moved aside", path.display()))
  - `crates/dbfile/src/remove.rs:23` let base = loop {

## F07: Environment and default paths resolving the catalog, prefs and home folders

- Kind: untrusted-input
- Entry: crates/app/src/config.rs:25-26 Config::from\1env via Layout::resolve(\1n\1 env::var\1os(n)) reading PG\1HOME, PG\1CATALOG\1DIR, PG\1PREFS and platform default folder
- Endpoint: crates/library/src/layout.rs (catalog\1dir and file locations) used by crates/catalog/src/core.rs:258 Catalog::open and crates/app/src/store.rs (prefs file)
- Summary: Environment variables choose where every persistent file lives (catalog, prefs, logs, cache). The values are taken without validation beyond OsString conversion and feed directory creation (cli/src/lib.rs:138 create\1dir\1all) and all later writes.

### SEC-028 [medium/data-integrity] Relative or empty PG\1HOME / PG\1CATALOG\1DIR resolve against the working directory — CONFIRMED

- Verdict: CONFIRMED. PG\1CATALOG\1DIR and PG\1HOME are taken without checks (layout.rs:30-38). A relative value resolves against the working directory.

- Confidence: high
- Kind: wrong-target
- Trigger: PG\1CATALOG\1DIR=catalog (relative) with PG\1HOME unset, or PG\1HOME=data, or either variable set to an empty string, and the app started from any folder other than the intended one. Layout::resolve accepts these values with no check.
- Consequence: The catalog, thumbs.sqlite, gallery.json, gallery.log and sqlite-aliases are created under whatever folder the process was started from (for a relative catalog dir, the home is the empty path, so prefs and log land in the working directory root). The same install started from another folder opens or creates a different catalog, so the library appears empty. A catalog.sqlite already in that folder that is not this build's catalog is moved to catalog.sqlite.replaced (rebuild.rs:94) and replaced by an empty catalog. Nothing is deleted, but the user's data is shown as lost.
- Suggested fix: Treat an empty value as unset, and require an absolute path for PG\1HOME, PG\1CATALOG\1DIR and PG\1PREFS (reuse dirs.rs is\1absolute\1for), returning an error the way default\1home does. Apply the same rule to the --catalog-dir flag.
- Locations:
  - `crates/library/src/layout.rs:30` let catalog\1dir = catalog\1dir.or\1else(\1\1 var("PG\1CATALOG\1DIR").map(PathBuf::from));
  - `crates/library/src/layout.rs:32` Some(home) =\1 PathBuf::from(home),
  - `crates/library/src/layout.rs:34` Some(dir) =\1 dir.parent().unwrap\1or(dir).to\1path\1buf(),
  - `crates/library/src/layout.rs:38` let prefs = var("PG\1PREFS").map\1or\1else(\1\1 home.join("gallery.json"), PathBuf::from);
  - `crates/platform/src/dirs.rs:30` let absolute = \1name: &str\1 var(name).filter(\1value\1 is\1absolute\1for(windows, &value.to\1string\1lossy())).map(PathBuf::from);

### SEC-029 [medium/data-integrity] PG\1PREFS may name the catalog or a folder, and the quarantine renames it — CONFIRMED

- Verdict: CONFIRMED. PG\1PREFS can name the catalog file, and quarantine renames the named path (app.rs:78, store.rs:51, 111). Platform outcomes were not checked.

- Confidence: high
- Kind: wrong-target
- Trigger: PG\1PREFS set to the catalog file (for example PG\1PREFS=\1catalog folder\1\catalog.sqlite) or to a folder. JsonStore::open fails to parse or read it and quarantines the named path. On Linux the open catalog's main file is renamed to catalog.sqlite.bad while its -wal stays under the old name; the next save writes prefs JSON at the catalog path.
- Consequence: On Linux the catalog's main file and its WAL are separated, and the next start sees a non-database file at catalog.sqlite. rebuild.rs moves that JSON aside to .replaced and creates an empty catalog, so the library appears lost. Rows remain only in the .bad and .replaced files. A named folder is renamed to \1name\1.bad. On Windows SQLite keeps the file without share-delete, so the rename is refused and the file stays, but every prefs save then fails with a notice.
- Suggested fix: Refuse a PG\1PREFS that names a directory or a catalog, thumbs, log or lock file, or check the layout's paths for collisions when it is resolved. Quarantine only a file whose content failed to parse, and never a directory.
- Locations:
  - `crates/app/src/app.rs:78` let prefs = Arc::new(PrefStore::open(config.layout.prefs().to\1path\1buf(), inbox.clone()));
  - `crates/app/src/store.rs:51` Err(e) =\1 (T::default(), Some(quarantine(&path, e.to\1string()))),
  - `crates/app/src/store.rs:54` Err(e) =\1 (T::default(), Some(quarantine(&path, e.to\1string()))),
  - `crates/app/src/store.rs:111` let kept = pg\1platform::file::keep\1aside(path, ".bad").map\1err(\1e\1 e.to\1string());
  - `crates/platform/src/file.rs:273` match rename\1no\1replace(path, &to) {
  - `crates/library/src/layout.rs:38` let prefs = var("PG\1PREFS").map\1or\1else(\1\1 home.join("gallery.json"), PathBuf::from);

### SEC-030 [low/data-integrity] Any read error quarantines a readable preferences file — CONFIRMED

- Verdict: CONFIRMED. Any read error other than NotFound goes to quarantine (store.rs:54).

- Confidence: medium
- Kind: missing-check
- Trigger: gallery.json exists and is readable in content, but fs::read fails with a non-NotFound error (for example PermissionDenied or EIO) on a transient condition, and the rename that follows succeeds.
- Consequence: The user's preferences file is renamed to gallery.json.bad and defaults are used. The next change writes the defaults to gallery.json. The content is kept, so nothing is lost, but the preferences appear reset.
- Suggested fix: Return the read error without renaming the file for any error other than a parse failure, and let the notice say the file could not be read.
- Locations:
  - `crates/app/src/store.rs:53` Err(e) if e.kind() == io::ErrorKind::NotFound =\1 (T::default(), None),
  - `crates/app/src/store.rs:54` Err(e) =\1 (T::default(), Some(quarantine(&path, e.to\1string()))),

### SEC-031 [low/data-integrity] A failed preferences save leaves gallery.json.tmp behind — CONFIRMED

- Verdict: CONFIRMED. Failed rename leaves the temp (store.rs:118-123). There is no guard.

- Confidence: high
- Kind: leak
- Trigger: The rename of the temp file over the preferences path fails (the target is held open by another program, or the path is empty, as with PG\1PREFS="" which gives a temp file .tmp in the working directory). The temp file is not removed.
- Consequence: A stale gallery.json.tmp (or .tmp in the working directory) stays beside the preferences after each failed save. It is overwritten by the next save, so the stale temp is the only effect; the prefs file itself is not corrupted, since its old content is kept.
- Suggested fix: Arm pg\1platform::file::TempFile (which exists for this purpose and is unused here) before File::create, and call persist() after a successful rename.
- Locations:
  - `crates/app/src/store.rs:118` let temp = beside(path, ".tmp");
  - `crates/app/src/store.rs:122` drop(file);
  - `crates/app/src/store.rs:123` std::fs::rename(&temp, path)
  - `crates/platform/src/file.rs:110` /// A temporary file that is removed when this is dropped, unless \1'TempFile::persist'\1 said it was kept (renamed

## F08: Desktop command-line options that trigger file writes or network calls

- Kind: untrusted-input
- Entry: crates/desktop/src/command\1line.rs:198-222 init/read\1of parsing args and PG\1DEV; gallery entry crates/gallery/src/main.rs:27 env::args\1os
- Endpoint: crates/desktop/src/screenshot.rs:35 (--screenshot PATH writes files), crates/desktop/src/yandex.rs:130 (--find-similar PATH sends the photo to Yandex), crates/desktop/src/command\1line.rs:519-529 (--add-folder, --save-crops actions)
- Summary: Command-line paths choose which files are read and where outputs go (screenshot PNG and .settle.txt log, find-similar input photo, log-view and lane-rows files, yandex-fixture folder). Bad option values are collected and reported; the paths are used as given.

### SEC-032 [medium/data-integrity] Crop copy renamed into place without a sync: a crash can leave an empty copy under the final name — CONFIRMED

- Verdict: CONFIRMED. publish writes and renames with no sync (crop lib.rs:399-401). The Yandex save path does sync (save.rs:80).

- Confidence: high
- Kind: crash-window
- Trigger: A --save-crops run writes a copy: publish writes the temp, sets its mtime, and renames it to the final name. Power is lost (or the OS is reset) after the rename reaches the directory but before the temp's data is flushed.
- Consequence: The copy at its final name is zero-length or truncated, though the pass reported it as saved. The doc comment's promise ('complete or not there') does not hold under power loss. The Yandex writer syncs before its rename, so the two writers are inconsistent.
- Suggested fix: Open the temp, write it, call sync\1all() before the rename (as save.rs write\1new does), and sync the folder where the platform allows.
- Locations:
  - `crates/crop/src/lib.rs:399` std::fs::write(temp, bytes).with\1context(\1\1 format!("writing {}", shown(out)))?;
  - `crates/crop/src/lib.rs:400` keep\1time(source, temp)?;
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;
  - `crates/app/src/yandex/save.rs:80` file.sync\1all().map\1err(at\1temp)?;

### SEC-033 [medium/security] --find-similar reads the whole photo into memory with no size cap — CONFIRMED

- Verdict: CONFIRMED. read\1photo reads the whole file with no size cap (yandex.rs:238-251).

- Confidence: high
- Kind: unbounded-alloc
- Trigger: --find-similar=\1path\1 names a very large file (for example a multi-GB TIFF, or a file with a JPEG/PNG magic prefix and a huge body). The first 16 bytes pass the still-image sniff, so read\1to\1end runs over the whole file.
- Consequence: The app allocates the full file (the Vec grows by doubling, so the peak can be about twice the file size) before any decode or size check, and can abort with out-of-memory, losing unsaved session state. The entire file is then uploaded to Yandex.
- Suggested fix: Check the file's length before reading it (a cap in bytes that Yandex accepts, or the TIFF transcode's own limit), and reserve the buffer from that length.
- Locations:
  - `crates/app/src/yandex.rs:238` fn read\1photo(path: &Path) -\1 Result\1Vec\1u8\1, SearchError\1 {
  - `crates/app/src/yandex.rs:251` file.read\1to\1end(&mut bytes).map\1err(unreadable)?;

### SEC-034 [low/security] Recorded session's downloads.tsv accepts a Windows drive-relative name and reads outside the session folder — CONFIRMED

- Verdict: CONFIRMED. The name check (replay.rs:56) does not catch a drive prefix, and join replaces the base directory (replay.rs:59).

- Confidence: medium
- Kind: traversal
- Trigger: A --yandex-fixture folder whose downloads.tsv has a line such as '`https://x/y.jpg\1TAB\1C:secret.bin'.` The name has no '..', no '/' and no '\', so the guard passes. On Windows, Path::join with a name carrying a drive prefix replaces the base directory, so the read goes to drive C's current directory.
- Consequence: A file the user can read, outside the fixture folder, is read into memory and, when that result is saved, written into the folder the user picked under the photo's name. This needs the test-only flag and a hostile fixture folder.
- Suggested fix: Reject any name with a prefix or colon (or require Path::new(name).components() to be a single Normal component) before joining, and resolve the joined path against the folder.
- Locations:
  - `crates/app/src/yandex/replay.rs:41` let file = \1name: &str\1 dir.join(name);
  - `crates/app/src/yandex/replay.rs:56` if name.contains("..") \1\1 name.chars().any(\1c\1 c == '/' \1\1 c == '\\') {
  - `crates/app/src/yandex/replay.rs:59` let bytes = std::fs::read(file(name)).map\1err(\1e\1 format!("{}: {e}", file(name).display()))?;

### SEC-035 [low/data-integrity] --log-view also reads the log's rotation siblings, not only the named file — CONFIRMED

- Verdict: CONFIRMED. read\1all includes the aside and rotated files (log file.rs:152-154).

- Confidence: high
- Kind: wrong-target
- Trigger: --log-view=\1dir\1\gallery.log while \1dir\1 also holds gallery.log.1, .2 or .next. The screenshots' fixed log then shows the records of those siblings too.
- Consequence: The log view shows records from files the command line did not name, so a screenshot or log check can show the wrong records. The doc says the view 'shows that file'.
- Suggested fix: Give --log-view a reader that takes only the named file (no sibling reads), or document that rotated siblings are included.
- Locations:
  - `crates/desktop/src/log_model.rs:117` if let Some(path) = crate::command\1line::log\1view() {
  - `crates/log/src/file.rs:152` let mut files: Vec\1PathBuf\1 = (1..FILES).rev().map(\1n\1 older(path, n)).collect();
  - `crates/log/src/lib.rs:125` let mut records = parse(&file::read\1all(path));

### SEC-036 [low/data-integrity] --screenshot overwrites any existing file at the given path, and the PNG write is not atomic — CONFIRMED

- Verdict: CONFIRMED. screenshot.rs:55 writes the path with no check for an existing file.

- Confidence: high
- Kind: missing-check
- Trigger: --screenshot=\1path\1 names an existing file (for example a photo the user keeps). The settle log is written beside it first, then std::fs::write truncates the file and writes the PNG. A crash during the write leaves a truncated image.
- Consequence: An unrelated file is replaced by the screenshot PNG with no check or warning, and a crash mid-write leaves a truncated image. This is a test hook, so the risk is limited to a mistyped path.
- Suggested fix: Refuse a path that already exists unless it is a previous screenshot, or write to a temp and rename it into place as the crop and Yandex writers do.
- Locations:
  - `crates/desktop/src/screenshot.rs:50` std::fs::write(log, format!("{report}\n"))?;
  - `crates/desktop/src/screenshot.rs:55` std::fs::write(path, png)

### SEC-037 [low/data-integrity] Crop copies replace any existing file of the same name in the chosen folder, unlike the Yandex saves — CONFIRMED

- Verdict: CONFIRMED. crop rename replaces the target (crop lib.rs:401). The Yandex path uses create\1new (save.rs:38).

- Confidence: high
- Kind: overwrite
- Trigger: --save-crops=\1folder\1 where a file with the copy's name (the library's place plus the file's path) already exists in that folder, for example one the user placed there. The rename replaces it silently.
- Consequence: A pre-existing file in the copies folder is replaced without a '(2)' name, so its content is lost. Repeat crop saves are meant to overwrite, so this is partly intended, but it is not limited to earlier crop copies.
- Suggested fix: Check the copy's path for an existing file that is not an earlier crop copy (for example by a recorded marker) before the rename, or claim the name with create\1new as the Yandex save does.
- Locations:
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;
  - `crates/app/src/yandex/save.rs:38` match OpenOptions::new().write(true).create\1new(true).open(&path) {

### SEC-038 [low/security] Command-line options whose name is not valid Unicode are dropped without an error — CONFIRMED

- Verdict: CONFIRMED. arg\1of returns None for a non-UTF-8 name (command\1line.rs:213), and read\1of skips it (:223).

- Confidence: high
- Kind: missing-check
- Trigger: An argument like --\1non-UTF-16 name\1=value on Windows (or a non-UTF-8 name on Unix). arg\1of returns None for it, and read\1of's bare-word loop skips it, so nothing is recorded as a bad option.
- Consequence: A misspelt or odd-byte test hook does nothing, silently. This is the case the module's stated rule (another --name is an error) is meant to catch, and a --screenshot run is not refused for it.
- Suggested fix: Record a BadOption (with the lossy name) for any argument that starts with '--' but whose name is not valid Unicode, in both places.
- Locations:
  - `crates/desktop/src/command_line.rs:213` let name = name.to\1str()?.strip\1prefix("--")?.to\1string();
  - `crates/desktop/src/command_line.rs:223` let Some(word) = word.to\1str().and\1then(\1w\1 w.strip\1prefix("--")) else { continue };

## F09: CLI arguments and borders tool file reads and writes

- Kind: untrusted-input
- Entry: crates/cli/src/args.rs:17-26 (--catalog-dir, env PG\1CATALOG\1DIR) and clap options in crates/cli/src/args.rs:34-175
- Endpoint: crates/cli/src/borders.rs:35 read\1dir of user folder, crates/cli/src/borders.rs:136 write CSV, crates/cli/src/borders.rs:182 read review file
- Summary: The CLI reads a folder of images for border analysis, reads a user-written review file, and writes a CSV to a path from the command line. Catalog folder creation (lib.rs:138) also happens at start.

### SEC-039 [medium/data-integrity] CSV target checked only at the end, so an unwritable --csv discards a full pass — CONFIRMED

- Verdict: CONFIRMED. The CSV path is checked only as present (cli lib.rs:719). It is written after the whole pass (borders.rs:136).

- Confidence: high
- Kind: missing-check
- Trigger: gallery borders D:/photos --csv C:/reports/out.csv where C:/reports does not exist, or out.csv is open in Excel (sharing violation on the write), so the write at the end fails after every file has been analyzed.
- Consequence: The whole analysis (possibly hours on a large folder) is discarded and the command ends with a 'writing ...' error. Nothing is saved, and the work is repeated on each retry. The error comes after the pass, not before it, so the user learns about the wrong path only after the cost is spent.
- Suggested fix: Before the pass, create or open the CSV's temp sibling (and check its parent exists) so an unwritable target fails up front. Write the report to that temp file and rename it over the target at the end.
- Locations:
  - `crates/cli/src/lib.rs:719` let csv = csv.with\1context(\1\1 line(&Line::CsvMissing))?;
  - `crates/cli/src/borders.rs:74` let files = image\1files(paths)?;
  - `crates/cli/src/borders.rs:136` std::fs::write(csv, out).with\1context(\1\1 format!("writing {}", csv.display()))?;

### SEC-040 [medium/data-integrity] One unreadable subfolder aborts the whole folder walk and the report — CONFIRMED

- Verdict: CONFIRMED. read\1dir errors propagate with ? (borders.rs:35-38), so one unreadable subfolder ends the report.

- Confidence: high
- Kind: missing-check
- Trigger: gallery borders D:/ (or any folder containing a folder the process cannot read, e.g. 'System Volume Information' on a drive root for a non-elevated user): read\1dir or file\1type fails inside the walk.
- Consequence: The error propagates with ? out of image\1files and report, so no CSV is written and no per-file rows are produced for the readable folders. Readable images elsewhere are never reported. The skip-and-report behaviour the per-file pipeline has for unreadable files is not applied to folders.
- Suggested fix: Treat a subfolder that cannot be read as a skipped entry: record it (with its error) and continue the walk, as the pipeline does for unreadable files, rather than returning the error with ?.
- Locations:
  - `crates/cli/src/borders.rs:35` for entry in std::fs::read\1dir(dir).with\1context(\1\1 format!("reading {}", dir.display()))? {
  - `crates/cli/src/borders.rs:36` let entry = entry?;
  - `crates/cli/src/borders.rs:38` if entry.file\1type()?.is\1dir() {

### SEC-041 [low/security] Explicit non-image paths and walked files are read whole before any size or type check — CONFIRMED

- Verdict: CONFIRMED. Explicit non-image paths are pushed unchecked (borders.rs:58). The pipeline reads the whole file before any sniff (pipeline.rs:573-589).

- Confidence: high
- Kind: unbounded-alloc
- Trigger: gallery borders D:/downloads/movie.mkv (an explicit file path, which skips the is\1image test at line 58), or a folder holding a multi-GB file with an image extension and garbage content. The pipeline reads the whole file into memory before Format::sniff can refuse it.
- Consequence: Memory use grows to the file's size for that one read. Allocation failure becomes an I/O error for that file (try\1reserve), but on a machine with less memory the process can be pushed into memory pressure. A file that is still growing during the read can exceed the pool's counted size, since read\1to\1end grows the buffer past the size taken from metadata.
- Suggested fix: Apply the same is\1image test to explicit file arguments (or reject non-image explicit files with a per-file error row). Check the file size against a cap before the whole read, or read only the header first, as the library's Gather mode does.
- Locations:
  - `crates/cli/src/borders.rs:58` out.push(p);
  - `crates/cli/src/borders.rs:40` } else if is\1image(&path) {
  - `crates/jobs/src/pipeline.rs:573` let size = usize::try\1from(file.metadata()?.len())
  - `crates/jobs/src/pipeline.rs:586` Some(file.read\1to\1end(&mut lease.bytes).map(\1\1\1 lease))

### SEC-042 [low/data-integrity] Report written by truncate-then-write, so a crash mid-write destroys the previous report — CONFIRMED

- Verdict: CONFIRMED. std::fs::write truncates the target first (borders.rs:136).

- Confidence: high
- Kind: crash-window
- Trigger: A kill, crash or power loss during the std::fs::write of a large report (tens of thousands of rows take visible time to write). The target is truncated first, so the old report is gone and the new one is partial.
- Consequence: A truncated or partial CSV sits at the user's --csv path, and the previous report is lost. The success line is not printed, but a later reader of the file may take the partial data as complete.
- Suggested fix: Write to a temp file beside the target, flush it, then rename it over the target, so the target is either the old report or the complete new one.
- Locations:
  - `crates/cli/src/borders.rs:136` std::fs::write(csv, out).with\1context(\1\1 format!("writing {}", csv.display()))?;

### SEC-043 [low/data-integrity] --buffer-mb shift silently wraps to 0 for multiples of 2^44 — CONFIRMED

- Verdict: CONFIRMED. mb.max(1) \1\1 20 wraps to 0 for large values (args.rs:54).

- Confidence: high
- Kind: overflow
- Trigger: gallery borders --buffer-mb 17592186044416 (2^44 MB). mb.max(1) \1\1 20 drops the high bits, so buffer\1bytes is 0.
- Consequence: Every file counts as larger than the budget, so the pass reads and processes one file at a time. There is no error, only a silent loss of parallelism, and the user's stated budget is not what runs.
- Suggested fix: Use checked\1mul (or saturating\1mul) for the megabyte-to-byte conversion, and reject or clamp values that do not fit.
- Locations:
  - `crates/cli/src/args.rs:54` opts.buffer\1bytes = self.buffer\1mb.map\1or(budget, \1mb\1 mb.max(1) \1\1 20);

## F10: Folder picker result and start folder

- Kind: untrusted-input
- Entry: crates/platform/src/picker.rs:99 pick() (IFileOpenDialog through COM) and crates/platform/src/picker.rs:44 start\1folder reading env names via var callback
- Endpoint: crates/platform/src/picker.rs:46 canonicalize of the chosen path, then crates/desktop/src/picker.rs:53 and crates/catalog/src/roots.rs add-root writes
- Summary: The folder path returned by the Windows shell dialog (IShellItem file\1system\1path) is canonicalized and stored as a library root. The start folder comes from environment variables and an optional --name argument.

### SEC-044 [medium/data-integrity] Volume-mounted start folder makes the picker fail instead of opening — REFUTED

- Verdict: REFUTED. start\1folder keeps the requested folder when the canonical form has no drive or share (picker.rs:47-48). The verbatim volume path is not used.

- Confidence: medium
- Kind: missing-check
- Trigger: The Yandex save dialog starts in the saved folder (or the photo's folder) and that folder is on a volume mounted at a folder with no drive letter. canonicalize gives the verbatim volume path \\?\Volume{GUID}\dir. start\1folder keeps it (no drive or share prefix, and it is a directory), shell\1path then strips the \\?\ prefix to the relative text Volume{GUID}\dir, and SHCreateItemFromParsingName is given that relative text.
- Consequence: start\1item returns an error before the dialog is shown. pick returns Err, so the user gets Notice::PickerFailed and the failed signal and no dialog, and the start folder is not replaced by the user's folder, although start\1folder is meant to return a folder the shell takes. The save cannot proceed while the saved-folder preference points there.
- Suggested fix: In start\1folder, return the user's folder (or None) when the canonical path has no drive or share prefix, or keep the \\?\Volume{GUID} prefix in shell\1path (strip only \\?\X:\ and \\?\UNC\). Add a shell\1path test for a volume-GUID path.
- Locations:
  - `crates/platform/src/picker/shell.rs:37` let Some(rest) = units.strip\1prefix(&VERBATIM) else { return units };
  - `crates/platform/src/picker.rs:47` Ok(real) if has\1drive\1or\1share(&real) =\1 real,
  - `crates/platform/src/picker.rs:193` let mut units = shell\1path(start);
  - `crates/platform/src/picker.rs:199` let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(units.as\1ptr()), None) }
  - `crates/desktop/src/picker.rs:85` match pg\1platform::picker::pick(start.as\1deref(), owner) {

### SEC-045 [low/data-integrity] Chosen volume-mounted folder refused with a misleading reason — CONFIRMED

- Verdict: CONFIRMED. without\1verbatim turns the volume path into a relative one (names.rs:136). check\1folder then refuses it.

- Confidence: medium
- Kind: wrong-result
- Trigger: The user picks, in the Windows dialog, a folder on a volume mounted at a folder with no drive letter, so the shell returns \\?\Volume{GUID}\dir. Add-folder then canonicalizes it to the same verbatim path.
- Consequence: without\1verbatim strips the prefix and the RulePath becomes the relative Volume{GUID}\dir. check\1folder then fails to open the relative text and refuses it as an unreadable folder. Nothing is written, since the refusal comes before add\1root, so the library is not corrupted, but the user is told the folder is unreadable instead of that it has no drive letter.
- Suggested fix: Refuse a canonical path that has no drive or share prefix before it is parsed into a RulePath (in normalize\1path or real\1path), with a reason that names the missing drive letter.
- Locations:
  - `crates/platform/src/names.rs:136` } else if let Some(rest) = bytes.strip\1prefix(br"\\?\") {
  - `crates/platform/src/names.rs:175` let real = real\1path(root).map\1err(\1error\1 NormalizeError::Unreadable { path: root.to\1path\1buf(), error })?;
  - `crates/platform/src/dir.rs:161` wide.set(&\1path, &\1SEP\1\1);
  - `crates/scan/src/lib.rs:354` pub fn check\1folder(rules: PathRules, root: &RulePath) -\1 Result\1(), RefusedReason\1 {

## F11: QML file dialogs, drag and drop and external URL opens

- Kind: untrusted-input
- Entry: crates/desktop/qml/Main.qml:220 RemoveFolderDialog (the only dialog in QML; no FileDialog, DropArea or openUrlExternally found)
- Endpoint: catalog folder removal via crates/desktop/src/library\1bridge.rs and crates/catalog/src/removal.rs
- Summary: The QML layer shows a folder removal confirmation; no drag-and-drop or external-open paths exist in the QML tree searched. Folder removal goes to the catalog, not the disk (removal.rs header: files stay, marked missing).

### SEC-046 [medium/data-integrity] Remove confirms by row; a reload while the dialog is open removes another folder — CONFIRMED

- Verdict: CONFIRMED. The dialog stores a folder, but the removal uses the row index (Main.qml:216-222, library.rs:268). A reload can change which folder that row names.

- Confidence: medium
- Kind: wrong-target
- Trigger: RemoveFolderDialog is open for folder X at row r. The catalog's roots change out of band (for example the CLI begin\1removal at crates/cli/src/lib.rs:432 removes an earlier folder). A background walk then publishes Change::Scanned, and on\1pass calls load\1roots. X moves to another row, and row r now holds folder Z. The user clicks Remove, and the dialog still shows X.
- Consequence: Folder Z leaves the library: its rows (ratings, analyses, queued writes) are deleted with the removal, and the removal is not what the dialog displayed. Files on disk are untouched, but the catalog data is lost. The expected-path check in begin\1removal passes because the path comes from the reloaded list, so the guard does not catch it.
- Suggested fix: Capture the folder's RootId and RulePath when the dialog opens, pass both to remove\1folder, and make the dialog's confirm act on that identity (or close the dialog when the roots list reloads) instead of on the row index.
- Locations:
  - `crates/desktop/qml/Main.qml:216` removeDialog.folder = library.roots\1row\1 ?? ""
  - `crates/desktop/qml/Main.qml:222` onConfirmed: library.removeFolder(removeDialog.row)
  - `crates/desktop/src/library.rs:202` self.as\1mut().load\1roots();
  - `crates/desktop/src/library.rs:268` (self.rust().root\1ids.get(index).copied(), self.rust().root\1paths.get(index).cloned())
  - `crates/catalog/src/removal.rs:94` if !rules.same(&path, expected) {

### SEC-047 [low/data-integrity] A failure after the removal is committed is reported as not removed — CONFIRMED

- Verdict: CONFIRMED. finish errors after begin\1removal committed are returned as failure (app library.rs:389-398, desktop library.rs:284).

- Confidence: high
- Kind: partial-write
- Trigger: The library is not running (library start failed) and the inline removals::finish call in pg\1app fails after begin\1removal has committed. Its failure sources are thumbs.forget\1many, sweep\1orphans or shrink\1all, and any of them returns Err.
- Consequence: The folder is already out of the catalog's library and its marker is set. The UI shows FolderNotRemoved and does not reload the roots, so the folder stays listed with its old id and the pending removal is not shown. A retry gets NotFound and then a reload. The message is wrong, and the operation is not reported as partly done.
- Suggested fix: After Begun::Started, reload the roots and report the removal as begun, with the finish error as a separate notice, so a failure after the commit is not presented as a failed removal.
- Locations:
  - `crates/app/src/library.rs:389` let begun = self.catalogs.with\1catalog(\1c\1 c.begin\1removal(id, expected))?;
  - `crates/app/src/library.rs:398` pg\1library::removals::finish(c, self.thumbs, &self.catalog\1path, &mut \1\1\1 true)
  - `crates/desktop/src/library.rs:284` Err(e) =\1 app::post(Notice::FolderNotRemoved { error: app::error\1text(&e) }),

### SEC-048 [low/data-integrity] Discard deletes the catalog rows, then unlinks the temps in a separate step, so a crash leaks the temps — CONFIRMED

- Verdict: CONFIRMED. Commit at removal.rs:259 runs before discard\1temps (app library.rs:420). A crash between them leaves the temps.

- Confidence: medium
- Kind: crash-window
- Trigger: The user discards a kept removal, and the process is killed or loses power after the catalog transaction commits (removal.rs:259) and before discard\1temps unlinks the dot-prefixed .pg-write temps (app/src/library.rs:420-422).
- Consequence: The temp files stay in the user's photo folder with no catalog row referring to them, and no step in this flow sweeps them. This is a leak, not a loss of the original file.
- Suggested fix: Record the temp paths for cleanup in the same transaction (or a small pending-cleanup table) and unlink them at the next start, so a crash after the commit does not leave them unreferenced.
- Locations:
  - `crates/app/src/library.rs:417` let Some(discarded) = self.catalogs.with\1catalog(\1c\1 c.discard\1removal(path))? else { return Ok(false) };
  - `crates/catalog/src/removal.rs:259` tx.commit()?;
  - `crates/app/src/library.rs:420` for (temp, e) in pg\1writeback::discard\1temps(rules, &temps) {

## F12: Yandex reverse-image search: upload and similar-page requests

- Kind: network
- Entry: crates/app/src/yandex.rs and crates/app/src/yandex/source.rs (Live source) calling crates/yandex/src/client.rs:48 upload(image bytes)
- Endpoint: crates/yandex/src/client.rs:11-12 UPLOAD\1URL and SEARCH\1URL over ureq (crates/yandex/src/http.rs:30 agent)
- Summary: The user's photo bytes are sent to yandex.ru endpoints, with a cookie built from the clock (client.rs:110-113), a fixed user agent, and paced requests. Uses rustls via ureq TLS config (http.rs:6). The network response is the untrusted input to flow F13.

### SEC-049 [low/security] Photo upload reads the whole file into memory with no size cap — CONFIRMED

- Verdict: CONFIRMED. Upload reads the whole file (yandex.rs:251) and sends it (client.rs:50-51). The download side has a cap (fetch.rs:12); upload does not.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: The user searches a still image that is many GB (for example a large BMP or TIFF scan). photo\1bytes sniffs 16 bytes, then calls read\1to\1end on the whole file, and Client::upload sends that entire Vec as the POST body.
- Consequence: The process allocates the full file size (plus Vec growth slack) before any request is made. A failed allocation in Rust aborts the whole app process, and catch\1unwind does not cover it. The request can also run for the full 15 minute body timeout before Yandex refuses it. The download side has a 100 MB cap (fetch.rs:12) and the upload side has none.
- Suggested fix: Check the file's length (File::metadata) before reading and refuse above an upload cap, mirroring MAX\1DOWNLOAD\1BYTES, or read with take(cap + 1) and return an error when the cap is exceeded.
- Locations:
  - `crates/app/src/yandex.rs:251` file.read\1to\1end(&mut bytes).map\1err(unreadable)?;
  - `crates/yandex/src/client.rs:50` let req = transfer\1timeouts(self.agent.post(UPLOAD\1URL).header("User-Agent", USER\1AGENT));
  - `crates/yandex/src/client.rs:51` parse\1upload(&read\1response(req.send(image))?)

### SEC-050 [low/data-integrity] similar suffix adds 1 to an unchecked u32 position from the response — CONFIRMED

- Verdict: CONFIRMED. pos + 1 is unchecked (yandex.rs:319). parse.rs:153 accepts u32::MAX.

- Confidence: high
- Kind: overflow
- Trigger: A similar-page entity with pos = 4294967295 (u32::MAX). parse.rs:153 accepts it, and find\1similar computes s.pos + 1 at yandex.rs:319.
- Consequence: In a debug or test build this panics inside the search thread. The panic is caught, so the whole search, including the other sizes, ends as Status::Stopped with no results. In a release build (no overflow-checks set) it wraps to 0, and the saved file is named with the suffix 'yandex similar 0', which is wrong. Both need a malformed upstream value.
- Suggested fix: Use s.pos.saturating\1add(1) (or checked\1add with a fallback) for the suffix, so no value of pos can panic or wrap.
- Locations:
  - `crates/app/src/yandex.rs:319` suffix: format!("yandex similar {}", s.pos + 1),
  - `crates/yandex/src/parse.rs:153` let pos = u32::try\1from(entity\1"pos"\1.as\1u64()?).ok()?;

### SEC-051 [low/data-integrity] Captcha check is a substring match on every response body, including successful ones — CONFIRMED

- Verdict: CONFIRMED. is\1captcha\1page is a substring test (parse.rs:21), applied to every body before the status check (http.rs:72).

- Confidence: high
- Kind: wrong-result
- Trigger: A successful Yandex response (the results page or a similar-page JSON) whose body contains the text 'showcaptcha', for example inside a source page's URL or title that the results JSON carries. http.rs:72 runs is\1captcha\1page on every body before the status check.
- Consequence: A legitimate answer is reported as YandexError::Captcha. On the results page or the upload, the whole search fails with a captcha error and the photo's results are lost. On a later similar page, the list is cut short with stopped = Captcha, so results are silently missing.
- Suggested fix: Test for the captcha only on non-success statuses, or match the captcha page's own markup (for example a form action or a specific element) rather than a bare substring of the whole body.
- Locations:
  - `crates/yandex/src/parse.rs:21` body.contains("showcaptcha")
  - `crates/yandex/src/http.rs:72` if is\1captcha\1page(&body) {

### SEC-052 [low/security] Similar image URLs are taken from the response with no scheme or host check — CONFIRMED

- Verdict: CONFIRMED. img\1url is taken with no scheme or host check (parse.rs:154) and fetched (fetch.rs:36).

- Confidence: high
- Kind: missing-check
- Trigger: A similar-page entity whose url query holds img\1url=`http://127.0.0.1/...` or a LAN or other internal host (only possible if the response is hostile or Yandex's answer is compromised, since TLS uses the platform verifier). parse.rs:154 and 166-170 accept it, and find\1similar stores it as the Hit url, which the fetcher (fetch.rs:35-57) then GETs.
- Consequence: The app issues GET requests to any http or https host that the response names, from the user's machine (a request-forgery path into the local network), and reads up to 100 MB of each reply. Only fetch.rs's content-type and sniff checks limit what is kept. The cookies are not sent to those hosts, since the fetcher's jar holds only what those hosts set.
- Suggested fix: Accept an image\1url only when its scheme is https and its host is a public name (the host must match the Yandex image CDN or a result host that is not an IP literal or loopback), and drop the others in similar\1item.
- Locations:
  - `crates/yandex/src/parse.rs:154` let image\1url = query\1param(entity\1"url"\1.as\1str()?, "img\1url")?;
  - `crates/yandex/src/parse.rs:166` fn query\1param(url: &str, key: &str) -\1 Option\1String\1 {
  - `crates/app/src/yandex.rs:314` url: s.image\1url,

## F13: Yandex response parsing (JSON and HTML state)

- Kind: untrusted-input
- Entry: crates/yandex/src/parse.rs:9 parse\1upload, :107-109 parse\1other\1sizes (HTML entity decode of data-state), :137 parse\1similar\1page, :166 query\1param
- Endpoint: crates/app/src/yandex.rs result list and crates/app/src/yandex/images.rs:96 prefetch of image URLs
- Summary: Remote JSON (serde\1json::Value) and HTML containing embedded JSON are parsed with manual percent and entity decoding. Image URLs taken from the page become fetch targets (F14).

### SEC-053 [low/security] Entity decode is quadratic on '&' with no nearby ';' — CONFIRMED

- Verdict: CONFIRMED. find(';') scans the whole tail before the end\1=10 filter (parse.rs:72).

- Confidence: high
- Kind: unbounded-work
- Trigger: A results page whose data-state attribute (up to the 10 MiB body cap) has many raw '&' characters and no ';' within 10 bytes, for example millions of '&' and no ';'. A compromised or MITM-ed Yandex response is needed.
- Consequence: decode\1entities runs find(';') over the whole rest of the attribute for every '&', so the search thread burns CPU quadratically (roughly 5e13 byte compares for 10 MB). The search stays busy, and Yandex.stop returns false after its 2 s wait. Memory is not affected.
- Suggested fix: Limit the search for ';' to the next 10 bytes, for example by slicing tail to min(len, 11) before find, so each '&' costs O(1).
- Locations:
  - `crates/yandex/src/parse.rs:72` tail.find(';').filter(\1&end\1 end \1= 10).and\1then(\1end\1 decode\1entity(&tail\11..end\1).map(\1c\1 (c, end)));

### SEC-054 [low/security] Percent-decode accepts '+' as a hex sign, so %+1 becomes 0x01 — CONFIRMED

- Verdict: CONFIRMED. u8::from\1str\1radix accepts a leading '+' (parse.rs:180).

- Confidence: high
- Kind: wrong-target
- Trigger: An img\1url in a similar entity's url containing the sequence %+1 (or %+A and similar). u8::from\1str\1radix("+1", 16) returns Ok(1).
- Consequence: The decoded image URL differs from the one Yandex sent, so the fetch targets the wrong path (usually a dead result), or a different file on the same host. This is silent and wrong, not a crash.
- Suggested fix: Require both hex digits to be ASCII hex digits (check is\1ascii\1hexdigit on hi and lo) before calling from\1str\1radix.
- Locations:
  - `crates/yandex/src/parse.rs:180` && let Some(v) = std::str::from\1utf8(&\1\1hi, \1lo\1).ok().and\1then(\1h\1 u8::from\1str\1radix(h, 16).ok())

### SEC-055 [low/security] Result URLs from the page are fetched with no scheme or host check — CONFIRMED

- Verdict: CONFIRMED. Result URLs are fetched with no https or host check (parse.rs:123, 154; fetch.rs:36).

- Confidence: high
- Kind: missing-check
- Trigger: A similar or other-size result whose img\1url or url is http:// (common for small third-party sites) or points at a loopback or private address (e.g. `http://192.168.1.1/x.jpg).` The fetch runs from the search's Images.prefetch with no https\1only and no host check. Substitution needs a network attacker on the plain-HTTP path. A private-address URL needs an indexed hostile link.
- Consequence: A network attacker can swap the picture the user saves for a plain-HTTP result. A private-address URL makes the app issue GET requests to the user's LAN; the response is kept only if it sniffs as an image, and redirects are followed up to 10 hops. The app does not restrict the destinations it fetches.
- Suggested fix: Accept only https URLs (or set https\1only on the Fetcher agent) and reject loopback, link-local and private-range hosts, after resolving the name, before the request is sent.
- Locations:
  - `crates/yandex/src/parse.rs:154` let image\1url = query\1param(entity\1"url"\1.as\1str()?, "img\1url")?;
  - `crates/yandex/src/parse.rs:123` out.push(OtherSize { group: group.clone(), width, height, url: url.to\1string() });
  - `crates/yandex/src/fetch.rs:36` let resp = transfer\1timeouts(self.agent.get(url).header("User-Agent", USER\1AGENT))
  - `crates/yandex/src/http.rs:30` pub(crate) fn agent() -\1 ureq::Agent {

### SEC-056 [low/data-integrity] Replay fixture name check misses drive-prefixed names on Windows — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-066 and SEC-067: the drive-prefix name check gap in replay.rs:56.

- Confidence: medium
- Kind: traversal
- Trigger: A replay folder whose downloads.tsv has a name such as D:secret.png or C:x (no '..', '/' or backslash). The user has to pass --yandex-fixture pointing at that folder, so this is a local, user-chosen input.
- Consequence: Recording::read reads a file outside the session folder (Path::join with a drive prefix replaces the base) and stores its bytes as the result file. Those bytes can then be saved into the user's folder. Nothing is sent over the network.
- Suggested fix: Require the name to be a plain file name: Path::new(name).components() must be exactly one Normal component, and reject any ':' as well.
- Locations:
  - `crates/app/src/yandex/replay.rs:56` if name.contains("..") \1\1 name.chars().any(\1c\1 c == '/' \1\1 c == '\\') {
  - `crates/app/src/yandex/replay.rs:59` let bytes = std::fs::read(file(name)).map\1err(\1e\1 format!("{}: {e}", file(name).display()))?;

### SEC-057 [low/security] Unchecked pos + 1 for the similar suffix wraps or panics — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-050: the unchecked pos + 1 at yandex.rs:319.

- Confidence: high
- Kind: overflow
- Trigger: A similar entity with pos = 4294967295 (u32::MAX). The entity passes u32::try\1from, and the add happens in find\1similar.
- Consequence: In release (no overflow-checks in Cargo.toml \1profile.release\1) the saved file name gets 'yandex similar 0' (wrong label). In debug or test builds the add panics, the whole search, including other sizes, ends as Stopped, and no note explains it.
- Suggested fix: Use s.pos.saturating\1add(1) (or checked\1add with a fallback label) so the label is correct in every profile and the add cannot panic.
- Locations:
  - `crates/app/src/yandex.rs:319` suffix: format!("yandex similar {}", s.pos + 1),

### SEC-058 [low/data-integrity] Every fetched result file is kept until the next search, with no total cap — CONFIRMED

- Verdict: CONFIRMED. Each Ready result keeps its full bytes until the search is replaced (images.rs:158, 176). No total cap.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: A search with many results (up to 5 similar pages, plus other sizes, which the page can list without a count limit) where many files are large, for example 100+ results at a few MB each. Each Ready result keeps its full bytes (up to 100 MB) until the search's Images is replaced.
- Consequence: Resident memory grows with the sum of all result file sizes, not with what is on screen. Several GB is possible on a large search, and it is only released when the next search starts. No failure is reported.
- Suggested fix: Keep only the grid picture and the format for results on screen, and re-fetch the original on save, or add a total byte budget with eviction of Ready entries that are not picked.
- Locations:
  - `crates/app/src/yandex/images.rs:158` Ok(Arc::new(Fetched { file, format, grid }))
  - `crates/app/src/yandex/images.rs:176` Ok(download) =\1 State::Ready(download),
  - `crates/yandex/src/fetch.rs:12` pub const MAX\1DOWNLOAD\1BYTES: u64 = 100 \1 1024 \1 1024;

## F14: Yandex result image download and decode check

- Kind: network
- Entry: crates/yandex/src/fetch.rs:35 Fetcher::download(url) reading up to MAX\1DOWNLOAD\1BYTES (fetch.rs:12, :49-53)
- Endpoint: crates/app/src/yandex/images.rs:154 fetch\1checked, :231 grid\1picture decode check, then crates/app/src/makers.rs for the grid picture
- Summary: Remote image bytes (up to 100 MB, content type checked by plausibly\1image at fetch.rs:66) are fetched from URLs parsed out of the Yandex page and decoded for the grid. Undecodable files are marked dead and not saved.

### SEC-059 [medium/security] JPEG grid decode allocates from header dimensions with no reservation — PARTIAL

- Verdict: PARTIAL. The parajpeg path at decode.rs:209 resizes to the full header size with no reservation first, as claimed. The grid path also reaches a reservation at jpeg.rs:78, which the finding does not address.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: A result file that is a JPEG whose SOF header declares up to 65535 x 65535 (a few bytes of header, or a valid flat image of about 15 MB). fetch\1checked passes it to grid\1picture; libjpeg read\1header accepts the size; the 1/8 DCT scale yields 8192 x 8192 x 3 bytes, which out.pixels.resize zeroes (about 201 MB) before decompress. Up to four fetchers run at once, so about 800 MB transient. A file that decodes at the grid scale is saved (images.rs:158, yandex.rs:336), and the viewer's full decode of the saved file takes the parajpeg branch (decode.rs:209, resize to header.rgb\1len(), about 12.9 GB) or the libjpeg Full branch (decode.rs:238, same size). The second part was traced from the code but not run.
- Consequence: Each malicious result costs about 200 MB of zeroed memory during the grid check, enough to exhaust memory on a constrained machine with several in parallel. The viewer path can request about 12.9 GB from a saved file, which aborts the process on allocation failure (Vec::resize does not return an error). Other formats are capped at 512 MiB (codecs/limits.rs:14-18), but this path is not.
- Suggested fix: Apply the same limits::reserve check to the libjpeg header size (and to the parajpeg rgb\1len) before resizing, as codecs/jpeg.rs does on its fallback. Refuse a JPEG over max\1decode\1bytes with the same LimitError.
- Locations:
  - `crates/codec/src/decode.rs:215` let header = self.jpeg.read\1header(bytes)?;
  - `crates/codec/src/decode.rs:238` out.pixels.resize(scaled.width \1 scaled.height \1 3, 0);
  - `crates/codec/src/decode.rs:209` out.pixels.resize(header.rgb\1len(), 0);
  - `crates/codec/src/codecs/jpeg.rs:78` if let Err(e) = reserve(color, w, h) {

### SEC-060 [medium/data-integrity] A transient fetch failure marks a result dead for the whole search — CONFIRMED

- Verdict: CONFIRMED. A fetch error propagates with ? (images.rs:155) and the slot is marked Dead (:177-180). get returns that error for the whole search (:198-199).

- Confidence: high
- Kind: transient-failure-as-permanent
- Trigger: One result fetch fails for a reason that a retry would fix: a 30 s stall or connection reset (Transport), or an HTTP 429 or 503 (Status). fetch\1checked propagates it with ? (images.rs:155), and settle marks the slot Dead (images.rs:177-180).
- Consequence: The result is removed from the grid (Poll::dead, yandex.rs:487), get returns the error for every later call in this search (images.rs:198-199), and a save of that hit fails with SaveFailure::Download. Nothing retries until a new search replaces the Images. A network blip permanently drops good images from the search.
- Suggested fix: Keep Transport and 429/5xx outcomes retryable: leave the slot re-queued with a bounded attempt count, or return the error without settling Dead. Reserve Dead for decode and format failures and for 4xx answers.
- Locations:
  - `crates/yandex/src/fetch.rs:36` let resp = transfer\1timeouts(self.agent.get(url).header("User-Agent", USER\1AGENT))
  - `crates/yandex/src/fetch.rs:39` if !resp.status().is\1success() {
  - `crates/app/src/yandex/images.rs:155` let file = self.fetch.fetch(url)?;
  - `crates/app/src/yandex/images.rs:177` Err(error) =\1 {
  - `crates/app/src/yandex/images.rs:198` Some(Slot { state: State::Dead(error), .. }) =\1 return Err(error.clone()),

### SEC-061 [low/data-integrity] Every fetched result file is kept in full for the life of the search — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-058: the same Ready state holds the full Download (images.rs:52, 176).

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A search whose results include many large files. Each file may be up to 100 MiB (fetch.rs:12), and up to 5 similar pages plus the other sizes are fetched (client.rs:25), with no total byte budget. Each Ready state holds the whole Vec and not only the grid picture.
- Consequence: Resident memory grows with the sum of the original file sizes, not with the grid pictures. Dozens of 50 MB results can reach gigabytes until the next search replaces the Images (yandex.rs:449-454). The cap is per file, not per search.
- Suggested fix: Keep the grid picture resident and re-fetch the original on save, or keep originals under a byte budget with eviction. The save path already falls back to a fetch when a slot is missing.
- Locations:
  - `crates/app/src/yandex/images.rs:52` Ready(Arc\1Fetched\1),
  - `crates/app/src/yandex/images.rs:176` Ok(download) =\1 State::Ready(download),
  - `crates/yandex/src/fetch.rs:49` .take(MAX\1DOWNLOAD\1BYTES + 1)
  - `crates/app/src/yandex.rs:449` let pictures = Images::new(fetch, check, changes.clone());

### SEC-062 [low/data-integrity] A crash between claim and rename leaves an empty file under the saved name — CONFIRMED

- Verdict: CONFIRMED. claim creates the empty final name (save.rs:38) before the rename (:84).

- Confidence: high
- Kind: crash-window
- Trigger: Process kill or power loss after claim() creates the empty final name (save.rs:82) and before rename() moves the synced temporary onto it (save.rs:84).
- Consequence: A 0-byte file named like a saved result stays in the folder. The next save takes ' (2)' (claim is create\1new), so the empty file is never replaced. The temporary also stays until the next save of that name. The doc comment (save.rs:61-62) records this window, but the window is avoidable.
- Suggested fix: Publish with fs::hard\1link(temp, final), which fails if the name exists and makes the full file appear atomically, then remove the temporary. This drops the empty placeholder, so a crash leaves either nothing at the name or the whole file.
- Locations:
  - `crates/app/src/yandex/save.rs:38` match OpenOptions::new().write(true).create\1new(true).open(&path) {
  - `crates/app/src/yandex/save.rs:82` let path = claim(folder, stem, ext).map\1err(\1error\1 (folder.join(numbered(stem, ext, 1)), error))?;
  - `crates/app/src/yandex/save.rs:84` rename(temp.path(), &path).map\1err(\1error\1 (path.clone(), error))?;

## F15: Yandex save: writing a result into the user's chosen folder

- Kind: persistent-write
- Entry: crates/app/src/yandex/save.rs:63 write\1new (folder and stem from prefs yandex\1folder, crates/app/src/prefs.rs) and crates/app/src/yandex/images.rs cached bytes
- Endpoint: crates/app/src/yandex/save.rs:38 OpenOptions create\1new claim, :78 File::create temp (.\1stem\1.\1ext\1.pg-save), :84 rename of temp onto the claim
- Summary: Downloaded bytes are written as new files named after the user's photo plus a suffix returned by the search (saved\1stem at save.rs:13). The temp and claim names are predictable; the claim uses create\1new and the final rename uses std::fs::rename (save.rs:64), which replaces the claim.

### SEC-063 [low/data-integrity] Claim-then-replace leaves an empty file at the result's name after a crash — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-062 with a different path (save.rs:38, 84).

- Confidence: high
- Kind: crash-window
- Trigger: The process is killed or power is lost after the empty claim is created (save.rs:82) and before the temp is renamed onto it (save.rs:84). A concurrent writer that puts bytes into the claim in that instant is also overwritten, since the rename replaces rather than refusing an existing name.
- Consequence: The user's folder keeps a zero-byte file named like the saved result, and the full result bytes sit only in the hidden temp '.\1name\1.pg-save'. The next save of that name takes ' (2)', so the empty file is never cleaned up, and the hidden temp is overwritten only by the next save of the same name. The code comment at save.rs:61-62 names this state, but the claim exists only to make the save non-replacing, which pg\1platform::file::rename\1no\1replace already does in one call.
- Suggested fix: Drop the claim: loop over the candidate names and rename the synced temp onto each with pg\1platform::file::rename\1no\1replace, moving on to the next name on AlreadyExists. This leaves no empty file at any point and removes the replace-over-claim race.
- Locations:
  - `crates/app/src/yandex/save.rs:38` OpenOptions::new().write(true).create\1new(true).open(&path)
  - `crates/app/src/yandex/save.rs:64` write\1new\1with(folder, stem, ext, bytes, \1from, to\1 std::fs::rename(from, to))
  - `crates/app/src/yandex/save.rs:84` rename(temp.path(), &path).map\1err(\1error\1 (path.clone(), error))?;

### SEC-064 [low/data-integrity] Fixed temp name is opened by truncating create, so writers sharing it interleave and a foreign file there is truncated and then deleted — CONFIRMED

- Verdict: CONFIRMED. The fixed temp name is opened with File::create (save.rs:78) and removed on failure (file.rs:141). Two processes can interleave.

- Confidence: medium
- Kind: race
- Trigger: (a) Two app instances (for example, on two catalogs) save a result with the same stem into the same folder at the same moment: both open '.\1stem\1.\1ext\1.pg-save' with File::create, so one writer's bytes land in the other's file and the first rename publishes a mixed file as a success. (b) A file the user already has named exactly '.\1stem (yandex ...)\1.\1ext\1.pg-save' is truncated by File::create (save.rs:78), and any later failure in that write deletes it through the guard (file.rs:141).
- Consequence: A saved result can be a corrupt image reported as saved, or an unrelated file is truncated and then removed. Within one process the busy flag serializes saves, so the risk needs a second process or a pre-existing file of that exact name. I found no process-level guard in the crates (only the catalog's lock file), so the two-instance case is not ruled out.
- Suggested fix: Create the temp with create\1new under a name unique to this call (process id plus a random or counter suffix), and retry on AlreadyExists, so a writer never opens a file it did not make and the guard only removes its own file.
- Locations:
  - `crates/app/src/yandex/save.rs:49` fn temp\1path(folder: &Path, stem: &OsStr, ext: &str) -\1 PathBuf {
  - `crates/app/src/yandex/save.rs:78` let mut file = File::create(temp.path()).map\1err(at\1temp)?;
  - `crates/platform/src/file.rs:141` let \1 = std::fs::remove\1file(&self.path);

### SEC-065 [low/security] Temp open follows a link at the fixed temp name and writes the photo through it — CONFIRMED

- Verdict: CONFIRMED. File::create follows a link at the temp name (save.rs:78). The rename then moves the link (:84).

- Confidence: medium
- Kind: traversal
- Trigger: A symlink (or on Windows a reparse point) named '.\1stem (yandex ...)\1.\1ext\1.pg-save' exists in the user's chosen folder. Only someone who can already create entries in that folder can plant it.
- Consequence: File::create follows the link and truncates and writes the result's bytes into its target, which can be outside the chosen folder. The rename at save.rs:84 then moves the link itself to the result's name. By contrast the claim at save.rs:38 uses create\1new, which does not follow links, so the two opens of the same save are inconsistent.
- Suggested fix: Open the temp with create\1new (no truncate, no link follow) under a per-call unique name, as the claim already does.
- Locations:
  - `crates/app/src/yandex/save.rs:78` let mut file = File::create(temp.path()).map\1err(at\1temp)?;
  - `crates/app/src/yandex/save.rs:38` OpenOptions::new().write(true).create\1new(true).open(&path)

### SEC-066 [low/security] Replay fixture's file-name check misses Windows drive-prefixed names, so a read escapes the fixture folder — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-056 and SEC-067 on the same replay.rs check.

- Confidence: high
- Kind: traversal
- Trigger: A recorded session (chosen with --yandex-fixture) has a downloads.tsv line whose file name is 'C:boot.ini' or similar: it has no '/', no '\\' and no '..', so it passes the check. On Windows, dir.join with a drive-prefixed name replaces dir, so the read goes to that file on drive C (or another drive).
- Consequence: The file's bytes are read. If they sniff as a supported image (JPEG, PNG, WebP, GIF, BMP or TIFF), the save writes them into the user's chosen folder under the photo's name, so an image file from elsewhere on disk can be copied out. Non-image content is dead and not saved. The trigger needs the user to open a crafted fixture folder.
- Suggested fix: Reject any name that is absolute, has a prefix, or has more than one normal component (for example, require Path::new(name).components() to be a single Component::Normal), instead of matching separator characters.
- Locations:
  - `crates/app/src/yandex/replay.rs:56` if name.contains("..") \1\1 name.chars().any(\1c\1 c == '/' \1\1 c == '\\') {
  - `crates/app/src/yandex/replay.rs:59` let bytes = std::fs::read(file(name)).map\1err(\1e\1 format!("{}: {e}", file(name).display()))?;

## F16: Recorded Yandex session replay (fixture folder)

- Kind: untrusted-input
- Entry: crates/desktop/src/command\1line.rs:438-440 yandex\1source() from --yandex-fixture PATH
- Endpoint: crates/app/src/yandex/replay.rs:44 read\1to\1string, :49 similar-N.json loop, :53 downloads.tsv, :59 read of recorded image bytes, which feed the search results
- Summary: A folder named on the command line stands in for Yandex. Its JSON, TSV and image files are read and parsed the same way as live responses, so the parsers in F13 and F14 see local untrusted data.

### SEC-067 [low/security] Drive-relative TSV names escape the session folder on Windows — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-056 and SEC-066: a drive-relative name escapes the session folder (replay.rs:56, 59).

- Confidence: high
- Kind: traversal
- Trigger: downloads.tsv line '`https://a.example/x.png\1TAB\1C:secret.png'` on Windows. The name has no '..', '/' or '\\', so it passes replay.rs:56. dir.join("C:secret.png") replaces dir (std path.rs:1306: a path with a prefix but no root replaces self), so replay.rs:59 reads a file relative to drive C's current directory.
- Consequence: The fixture reads an image file from outside its folder (the drive's current directory). It is kept and shown in the grid if it decodes, and a save copies it into the chosen folder. Only images are kept, and it needs a fixture folder the user named on the command line.
- Suggested fix: Accept a TSV name only when Path::new(name).components() is exactly one Component::Normal (no prefix, root, or '..'), or reject any ':' as well as the separators. Keep the check next to the join.
- Locations:
  - `crates/app/src/yandex/replay.rs:56` if name.contains("..") \1\1 name.chars().any(\1c\1 c == '/' \1\1 c == '\\') {
  - `crates/app/src/yandex/replay.rs:59` let bytes = std::fs::read(file(name)).map\1err(\1e\1 format!("{}: {e}", file(name).display()))?;

### SEC-068 [medium/data-integrity] Unreadable similar page or downloads.tsv silently ends the recorded results — CONFIRMED

- Verdict: CONFIRMED. while let Ok ends the loop on any error (replay.rs:49), and downloads.tsv read errors are taken as absent (:53).

- Confidence: high
- Kind: missing-check
- Trigger: A recorded similar-0.json or similar-1.json that is not valid UTF-8, such as a UTF-16 file with a BOM (FF FE) from Windows PowerShell 5 Out-File, or any read error other than not-found. read\1to\1string returns Err, the 'while let Ok' loop ends, and no error is raised. For downloads.tsv, the same error is taken as the file being absent.
- Consequence: A similar list recorded as UTF-16 shows zero similar results with no stop note, so the replay looks like a complete recording, and the read error is never returned or logged. A bad downloads.tsv makes every result fail as 'no recorded result file' instead of naming the real read error.
- Suggested fix: Stop the loop only on ErrorKind::NotFound and return other errors with the path, as text() does at line 44. Apply the same to downloads.tsv.
- Locations:
  - `crates/app/src/yandex/replay.rs:49` while let Ok(page) = std::fs::read\1to\1string(file(format!("similar-{}.json", similar.len()).as\1str())) {
  - `crates/app/src/yandex/replay.rs:53` if let Ok(list) = std::fs::read\1to\1string(file("downloads.tsv")) {

### SEC-069 [low/security] Recorded folder is read whole, with no count or size cap — CONFIRMED

- Verdict: CONFIRMED. Replay reads every similar page and download with no cap (replay.rs:49, 59). The startup timing was not checked.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: A fixture folder with a multi-GB file named in downloads.tsv, or thousands of contiguous similar-N.json files. Recording::read runs on the main thread during app open (desktop lib.rs:167-168, yandex.rs:105-113), before any search.
- Consequence: Every similar page and every downloaded file is loaded into memory at startup, and the window's start blocks while it reads. The live path caps downloads at MAX\1DOWNLOAD\1BYTES (100 MiB) and similar pages at DEFAULT\1MAX\1SIMILAR\1PAGES = 5, but the replay has neither cap and reads pages past 5 that are never used.
- Suggested fix: Stop the similar loop at DEFAULT\1MAX\1SIMILAR\1PAGES, and read each download with take(MAX\1DOWNLOAD\1BYTES + 1) as fetch.rs does, or read downloads on demand in fetch().
- Locations:
  - `crates/app/src/yandex/replay.rs:49` while let Ok(page) = std::fs::read\1to\1string(file(format!("similar-{}.json", similar.len()).as\1str())) {
  - `crates/app/src/yandex/replay.rs:59` let bytes = std::fs::read(file(name)).map\1err(\1e\1 format!("{}: {e}", file(name).display()))?;

### SEC-070 [low/data-integrity] Empty --yandex-fixture= reads the recording from the working directory — CONFIRMED

- Verdict: CONFIRMED. An empty value gives Replay with an empty path (command\1line.rs:439-440). Path::new("").join reads from the working directory (replay.rs:41).

- Confidence: high
- Kind: wrong-target
- Trigger: Launch with '--yandex-fixture=' (empty value). path() returns Some("") (command\1line.rs:418-420 keeps the exact empty value), so yandex\1source gives Replay(""). replay.rs:41 then joins "upload.json" onto "", which is the process's current directory.
- Consequence: A search answers from whatever upload.json, similar-N.json and downloads.tsv are in the working directory, and the log line names an empty folder. No request is sent, so the effect is a misleading recording, not a leak.
- Suggested fix: Treat an empty or non-directory --yandex-fixture value as a bad option in command\1line::read\1of, and fail the recording with a message naming the value.
- Locations:
  - `crates/desktop/src/command_line.rs:439` match path("yandex-fixture") {
  - `crates/app/src/yandex/replay.rs:41` let file = \1name: &str\1 dir.join(name);

### SEC-071 [low/data-integrity] Captcha check matches the text anywhere in the page's JSON — CONFIRMED

- Verdict: CONFIRMED. The captcha check runs on the whole recorded body (replay.rs:92; parse.rs:21).

- Confidence: high
- Kind: false-positive
- Trigger: A recorded similar page 0 whose JSON contains the string showcaptcha, for example a result whose img\1url is https%3A%2F%2Fa.example%2Fshowcaptcha.png (percent-encoding leaves the letters as they are). The whole body is searched.
- Consequence: Page 0 becomes YandexError::Captcha and read\1similar\1pages returns Err, so the entire similar list fails with 'Yandex asked for a captcha' although no captcha was served. On a later page the list is cut short with a Captcha note. The live path has the same check (http.rs:72), so this is a live defect too.
- Suggested fix: Detect the captcha page by its markers in the page's structure (for example its HTML title or form action), not a substring of the whole body, and apply it in both the live and replay paths.
- Locations:
  - `crates/yandex/src/parse.rs:21` body.contains("showcaptcha")
  - `crates/app/src/yandex/replay.rs:92` Some(body) if is\1captcha\1page(body) =\1 Err(YandexError::Captcha),

## F17: Original file write-back (ratings and tags written into user photos)

- Kind: persistent-write
- Entry: crates/writeback/src/lib.rs:228 start (writer thread); queued files from crates/catalog/src/writers.rs
- Endpoint: crates/writeback/src/lib.rs:603 std::fs::write temp, :615 read-back check, :642-672 remove\1file of temp; crates/writeback/src/file.rs:74-82 write\1all, set\1len, sync\1all on the original; crates/writeback/src/inplace.rs:57 write temp
- Summary: The user's original JPEG, PNG or WebP is rewritten in place or via a temp file beside it: bytes are read, tags are changed through ExifTool (F18), the result is written to a temp, read back, then written over the original (file.rs:80-82 truncates to length). Removes of temp and original-path files happen in file.rs:50-53. Failure paths clean up temp files.

### SEC-072 [medium/data-integrity] Missing recorded temp is treated as a finished copy, so a lost write is recorded as done — CONFIRMED

- Verdict: CONFIRMED. NotFound on the recorded temp is treated as copied (writeback lib.rs:742), then write\1done runs (:747).

- Confidence: high
- Kind: missing-check
- Trigger: A copy is recorded (lib.rs:650) and the process stops before the copy finishes (crash, kill, or a failed copy that is retried later). The recorded temp is then gone when the writer resumes: a folder holding it was moved or renamed (folder\1id changes but file\1writes.temp keeps the old path; queue.rs:235 is the only writer of that column), or the hidden temp was deleted by another program, or a second writer process removed it (see the cross-process finding). finish\1copy reads the temp (lib.rs:733), gets NotFound (742), skips the copy, and calls write\1done with before None (745-747).
- Consequence: The catalog records the rating or turn as in the file (file\1ratings or the turns state is updated) while the original still holds the old bytes. The user's edit is silently lost and no failure is reported. For turns, the analysis rotation is also updated for pixels that never turned.
- Suggested fix: When the recorded temp is missing, do not record success: keep the row and fail the attempt (or verify the original against the expected content, for example the rating read back from it, before write\1done). Also store the temp's path relative to the file, or re-derive it from the current folder, so a moved folder does not leave a stale path.
- Locations:
  - `crates/writeback/src/lib.rs:742` Err(e) if e.kind() == std::io::ErrorKind::NotFound =\1 {}
  - `crates/writeback/src/lib.rs:745` let after = stat(path)?;
  - `crates/writeback/src/lib.rs:746` let rewrite = Rewrite { before: None, after, metadata\1only: None };
  - `crates/writeback/src/lib.rs:747` catalog.write\1done(&written, Self::rewrote(&written, rewrite))?;
  - `crates/catalog/src/queue.rs:235` "UPDATE file\1writes SET temp = ?3, temp\1value = ?4 WHERE file\1id = ?1 AND kind = ?2 AND \

### SEC-073 [medium/data-integrity] A second writer process deletes the temp another process recorded — CONFIRMED

- Verdict: CONFIRMED. Two writer processes share one catalog. The remove on the temp path (lib.rs:587) and the Err-arm delete (:642) follow the code as described. The shared-hold claim was not checked.

- Confidence: medium
- Kind: race
- Trigger: The app and the CLI 'write' command (cli/src/lib.rs:613) run at once on one catalog (only a shared hold, lifecycle.rs:66-69). Process B picks a fresh job for file F before process A records F's temp. B runs remove\1file on the same temp path (lib.rs:587), then writes its own. A records the temp (lib.rs:650), holds the original open with FILE\1SHARE\1READ (platform/src/file.rs:89). B's open\1for\1copy fails with a sharing violation (lib.rs:639) and the Err arm deletes the temp A recorded (lib.rs:642). If A's copy then fails or A crashes, the temp is gone.
- Consequence: A's recorded temp, the only whole copy of the edit, is destroyed. The next start finds no temp and records success (the F1 defect), leaving a torn or unchanged original marked as written. The window is narrow, but the result is silent damage to the user's photo.
- Suggested fix: Make writes exclusive across processes (an exclusive hold on the catalog or a per-file claim in the queue, taken before the temp is touched), or give each attempt a unique temp name and delete only the temp the attempt itself created, never a recorded one.
- Locations:
  - `crates/writeback/src/lib.rs:587` let \1 = std::fs::remove\1file(&temp);
  - `crates/writeback/src/lib.rs:642` let \1 = std::fs::remove\1file(&temp);
  - `crates/writeback/src/lib.rs:650` let recorded = match catalog.record\1write\1temp(job, &temp\1rule, job.write) {
  - `crates/platform/src/file.rs:89` FILE\1SHARE\1READ,
  - `crates/cli/src/lib.rs:613` let writer = Writer::start(catalog\1path.clone(), cache, Arc::new(Bus::new(16)), Arc::new(PrintingSink))?;

### SEC-074 [medium/data-integrity] Resumed copy overwrites an external save without comparing the original — CONFIRMED

- Verdict: CONFIRMED. finish\1copy writes the stored temp with no unchanged() check (lib.rs:735-737). copy\1and\1done consumes the handle first (:709).

- Confidence: medium
- Kind: lost-update
- Trigger: A copy into the original fails part-way (disk full, I/O error): copy\1and\1done has consumed the handle (lib.rs:709), so the file is released with the temp recorded. Another program then saves the photo. The row is pinned by the recorded temp, so the scanner cannot record the change (writers.rs:178, not\1being\1written). At the retry (next start via lib.rs:343, or the next change to the file), finish\1copy calls open\1for\1copy and write\1into with the stored temp, with no unchanged() check (lib.rs:735-737).
- Consequence: The external save is overwritten by the stale temp, and the user's other-program edit is lost with no notice beyond the failure text.
- Suggested fix: Before the resumed copy, compare the original's (size, mtime) with the values recorded at record time (or with the expected pre-write content hash). If they differ and the original is not already the temp's content, fail the attempt and keep the temp, rather than overwrite.
- Locations:
  - `crates/writeback/src/lib.rs:709` write\1into(copy.file, copy.data)?;
  - `crates/writeback/src/lib.rs:735` let file =
  - `crates/writeback/src/lib.rs:737` write\1into(file, &data)?;
  - `crates/catalog/src/writers.rs:178` not\1being\1written!(files)

### SEC-075 [low/data-integrity] Failure paths before the copy leave the partial temp file beside the user's photo — CONFIRMED

- Verdict: CONFIRMED. The ? at lib.rs:593 and the fallible write at :603 return before any cleanup, so the temp stays.

- Confidence: high
- Kind: leak
- Trigger: ExifTool's write\1rating fails (lib.rs:593, the ? returns before any cleanup), the turns temp write fails midway (lib.rs:603), or the orientation read-back fails (lib.rs:615). The temp exists but is never recorded, so the failure branch at lib.rs:642 and lib.rs:654 does not run.
- Consequence: A dot file (.name.rating.pg-write or .name.turns.pg-write) with partial or wrong content stays in the user's folder until the next attempt for that file removes it (lib.rs:587). If the row is dropped (for example the file is removed from the library), the file stays indefinitely. The module doc says failure paths clean up their temps.
- Suggested fix: Wrap the temp in a guard that removes it on any error before recording (for example platform::file::TempFile armed until record\1write\1temp succeeds), as the open\1for\1copy error path already does.
- Locations:
  - `crates/writeback/src/lib.rs:593` tool.write\1rating(&path, &temp, rating.get())?;
  - `crates/writeback/src/lib.rs:603` std::fs::write(&temp, &new).with\1context(\1\1 format!("writing {shown\1temp}"))?;
  - `crates/writeback/src/lib.rs:615` Ok(()) =\1 Some(std::fs::read(&temp)?)

### SEC-076 [low/data-integrity] Temp name can exceed the file-name limit for names near 255 characters — CONFIRMED

- Verdict: CONFIRMED. with\1affixes adds 16 characters with no length check (writeback file.rs:67, model name.rs:63). Medium confidence.

- Confidence: medium
- Kind: missing-check
- Trigger: A library file whose name is about 239 to 255 characters. temp\1of adds a dot and a 16-character kind suffix (.rating.pg-write or .turns.pg-write), with no length check. The temp name exceeds 255 units, so the temp create fails.
- Consequence: Every rating or turn write of that file fails with an error. The write counts as failed, retries up to the cap, and is reported as a failure, so the user's edits for that file never reach the file. It does not corrupt data, but the feature silently does nothing beyond the error text.
- Suggested fix: Check the temp name's length against the file system's limit before writing, and report it as a clear per-file error, or shorten the temp name (for example a hash-based name with the original kept in the catalog).
- Locations:
  - `crates/writeback/src/file.rs:67` temp.parts.push(name.with\1affixes(".", suffix));
  - `crates/model/src/name.rs:63` pub fn with\1affixes(&self, prefix: &str, suffix: &str) -\1 Name {
  - `crates/writeback/src/lib.rs:582` let temp\1rule = temp\1of(&path\1rule, job.kind());

## F18: ExifTool process spawn and stdin command protocol

- Kind: boundary
- Entry: crates/exiftool/src/lib.rs:31 program() (PG\1EXIFTOOL env, else exiftool on PATH)
- Endpoint: crates/exiftool/src/lib.rs:61 Command::new(program) with -stay\1open and piped stdin for file paths; crates/exiftool/src/lib.rs:201 oneshot Command::new(program).args(...).output()
- Summary: Spawns a child process chosen by environment variable and passes file paths and tag values over stdin (-@ -) or argv. The child's stdout and stderr are parsed by the pump thread (exiftool/src/lib.rs:216). Child process identity and PATH resolution are not verified by the program. Job object assignment in process.rs:131.

### SEC-077 [medium/data-integrity] Any ExifTool failure on orientation write is recorded as a turn the file cannot hold — CONFIRMED

- Verdict: CONFIRMED. write\1orientation errors map to None (writeback lib.rs:614-617), and that goes to keep\1turn\1in\1catalog (:623).

- Confidence: high
- Kind: missing-check
- Trigger: A turn on a JPEG or other file with no EXIF orientation tag (patch\1orientation returns None), while the kept ExifTool fails the write\1orientation command: it times out at 120 s, the process dies, or the pipe breaks. The Err is mapped to None.
- Consequence: The job is marked done as WriteEffect::KeptInCatalog with no retry and no failure notice. The file keeps its old orientation on disk while the catalog shows the turn, and other programs show it unrotated. Only the catalog-only outcome for formats that cannot take EXIF was meant to produce this state.
- Suggested fix: Propagate the ExifTool error with ? so the job fails and is retried and reported. Keep the turn in the catalog only when ExifTool succeeded and the orientation does not read back (the format cannot hold it).
- Locations:
  - `crates/writeback/src/lib.rs:614` let written = match tool.write\1orientation(&path, &temp, o.get()) {
  - `crates/writeback/src/lib.rs:617` Err(\1) =\1 None,
  - `crates/writeback/src/lib.rs:623` return self.keep\1turn\1in\1catalog(catalog, job, turns);

### SEC-078 [medium/data-integrity] Crop save publishes untagged bytes over the earlier copy, then deletes them if tagging fails — CONFIRMED

- Verdict: CONFIRMED. publish overwrites the earlier copy (crop lib.rs:382, 401). tag\1copy failure removes out (:125-128), so neither copy remains.

- Confidence: high
- Kind: crash-window
- Trigger: Saving a TIFF or compressed BMP crop copy to a folder where a copy from an earlier save already exists, while ExifTool is missing or its copy\1tags fails (ExifTool error at tag\1copy, line 125). A crash between the publish and the tag step has the same starting state.
- Consequence: The earlier good copy is replaced by the new untagged bytes at line 382 and then removed at line 128, so the user is left with neither copy. The job reports failure. After a crash the untagged copy is left at the destination with no failure reported.
- Suggested fix: Copy the tags into the temp before publish, as cut's with\1tags does (ExifTool on the temp path, read back, then rename). Publish only the complete, tagged bytes, so a failure leaves the earlier copy in place.
- Locations:
  - `crates/crop/src/lib.rs:382` publish(&temp\1path, &out\1path, &cropped.bytes, &source\1path)?;
  - `crates/crop/src/lib.rs:125` if let Err(e) = tag\1copy(&mut exiftool, &source, &out) {
  - `crates/crop/src/lib.rs:128` let \1 = std::fs::remove\1file(&out);
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;

### SEC-079 [medium/data-integrity] Copy path through an existing junction or symlink can rename over a library original — CONFIRMED

- Verdict: CONFIRMED. The lexical containment check (crop lib.rs:79) does not see a junction. create\1dir\1all (:375) and the rename (:401) follow it. Needs a link in the user's chosen folder.

- Confidence: medium
- Kind: wrong-target
- Trigger: The user's chosen copy folder contains a junction or symlink at a component of the copy path below it, for example C:\Out\D pointing to D:\, so that C:\Out\D\Photos\2019\a.jpg resolves into library root D:\Photos\2019. The lexical overlap check at line 79 does not follow it. Requires write access to the chosen folder.
- Consequence: create\1dir\1all follows the junction, and the temp is renamed over the resolved path, which is a library original. The original is replaced by the cropped copy (new NTFS identity, rating lost). ExifTool's -overwrite\1original on the same path would also reach it.
- Suggested fix: Before writing, resolve the existing components of the copy path (or reject reparse points below the chosen folder) and refuse when the resolved destination is under a library root, as the lexical check does for the path as typed.
- Locations:
  - `crates/crop/src/lib.rs:79` if let Some(r) = roots.iter().find(\1r\1 rules.is\1under(&tree, r) \1\1 rules.is\1under(r, &tree)) {
  - `crates/crop/src/lib.rs:90` let out = rules.join(&RulePath { anchor: tree.anchor.clone(), parts });
  - `crates/crop/src/lib.rs:375` std::fs::create\1dir\1all(dir).with\1context(\1\1 format!("creating {}", shown(dir)))?;
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;

### SEC-080 [low/security] Bare exiftool name resolves to the app's own directory before PATH — CONFIRMED

- Verdict: CONFIRMED. Bare exiftool resolves through std's Windows search, which checks the app directory first (std process/windows.rs:550). Needs write access to the install folder.

- Confidence: medium
- Kind: wrong-target
- Trigger: An exiftool.exe placed in the directory of the app's executable (which Rust's Windows Command search checks before the system directories and PATH) and PG\1EXIFTOOL unset. The program is then run with the user's rights, and it receives every file path and tag value the writer sends.
- Consequence: A planted program runs and can alter the photo files it is given. This needs write access to the install folder, so the impact is limited to that case; the program identity is not checked anywhere in the flow.
- Suggested fix: Resolve the program once to an absolute path on PATH, skipping the app directory, or verify it at start (for example that '-ver' reports the pinned version) before its first write.
- Locations:
  - `crates/exiftool/src/lib.rs:31` std::env::var\1os("PG\1EXIFTOOL").unwrap\1or\1else(\1\1 "exiftool".into())
  - `crates/exiftool/src/lib.rs:61` let mut cmd = Command::new(program);
  - `C:/Users/Torres/.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/src/rust/library/std/src/sys/process/windows.rs:550` if let Ok(mut app\1path) = env::current\1exe() {

### SEC-081 [low/data-integrity] The argument-route ExifTool run has no deadline and cannot be killed — CONFIRMED

- Verdict: CONFIRMED. The argv route calls cmd.output() with no deadline (exiftool lib.rs:122-123, 204).

- Confidence: high
- Kind: missing-check
- Trigger: A name or folder with a line break, or a file name ending in space, tab, CR, VT or FF (possible on Unix only), so the command takes the argv route at line 122, and ExifTool then hangs on it, for example on an unresponsive network share.
- Consequence: The writer thread blocks in output() with no bound. The queue stalls, Writer::stop returns false, and dropping the writer waits for the thread, which can hang app shutdown. The kept process has a 120 s deadline and a kill; this path has neither.
- Suggested fix: Spawn the one-shot child, poll try\1wait against the same COMMAND\1TIMEOUT, and kill it on expiry, as the kept process does.
- Locations:
  - `crates/exiftool/src/lib.rs:122` if needs\1argv(args) {
  - `crates/exiftool/src/lib.rs:123` return oneshot(&self.program, args);
  - `crates/exiftool/src/lib.rs:204` let out = cmd.output().with\1context(\1\1 format!("starting {} (ExifTool)", program.to\1string\1lossy()))?;

### SEC-082 [low/security] ExifTool output has no cap per line or per answer — PARTIAL

- Verdict: PARTIAL. Output is read with no per-line or per-answer cap (exiftool lib.rs:221-256). The volume claim was not checked, as the finding says.

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A library file whose metadata makes ExifTool write a very long line, or a very large number of warning lines, to stdout or stderr before the marker. The stderr pump keeps reading and buffering even when no command is waiting for it.
- Consequence: Memory grows with ExifTool's output for up to the command's 120 s deadline, held in an unbounded channel and an unbounded answer String. Plausibility depends on ExifTool's output volume, which was not verified.
- Suggested fix: Cap the bytes kept per line and the total answer size, counting and discarding the excess, and fail the command when the cap is exceeded.
- Locations:
  - `crates/exiftool/src/lib.rs:221` match reader.read\1until(b'\n', &mut line) {
  - `crates/exiftool/src/lib.rs:231` if tx.send(String::from\1utf8\1lossy(&line).into\1owned()).is\1err() {
  - `crates/exiftool/src/lib.rs:256` out.push\1str(&line);

## F19: Crop and copy export to an output folder

- Kind: persistent-write
- Entry: crates/crop/src/lib.rs:50 save(catalog, ids, folder, opts) and crates/crop/src/lib.rs:209 cut
- Endpoint: crates/crop/src/lib.rs:339 write temp beside output, :375 create\1dir\1all, :396-401 publish (rename temp to out), :407-410 keep\1time on the copy
- Summary: Crop and rotate write new files into a chosen folder: bytes are rewritten by the edit crate (per-format byte surgery on PNG, GIF, WebP and BMP), tags are copied with ExifTool (tags\1temp\1beside, tag\1copy), then renamed into place. The original's modification time is copied to the output (keep\1time).

### SEC-083 [high/data-integrity] Copy path checked lexically; reparse point sends write into library — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-079 with severity high. A junction in the user-chosen folder is required, so medium is the fitting severity.

- Confidence: medium
- Kind: wrong-target
- Trigger: The chosen output folder contains a junction or symlink at a component of the mirrored place (for example \1out\1\D\Photos) that points into a library folder, and the library holds a file whose name matches a cropped file. save computes tree and out textually, the containment check at lib.rs:79 compares paths as text, create\1dir\1all (374-376) follows the link, and publish writes the temp and renames it onto the link path (382, 401).
- Consequence: A library original is replaced by the cropped copy (rename over it) and a temp file is written into the library. The code comment claims a folder whose copies would land in the library is refused up front; through a reparse point that guarantee does not hold, so the original is lost.
- Suggested fix: After create\1dir\1all, resolve the real path of the copy's parent (names::real\1path) and repeat the is\1under check against the library roots on that real path; refuse any output tree that crosses a reparse point below the chosen folder.
- Locations:
  - `crates/crop/src/lib.rs:57` let folder = pg\1platform::names::real\1path\1lenient(folder).context("the folder for the copies isn't a path")?;
  - `crates/crop/src/lib.rs:79` if let Some(r) = roots.iter().find(\1r\1 rules.is\1under(&tree, r) \1\1 rules.is\1under(r, &tree)) {
  - `crates/crop/src/lib.rs:374` if let Some(dir) = out\1path.parent() {
  - `crates/crop/src/lib.rs:375` std::fs::create\1dir\1all(dir).with\1context(\1\1 format!("creating {}", shown(dir)))?;
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;

### SEC-084 [medium/data-integrity] Copy renamed into place without fsync of its contents — CONFIRMED

- Verdict: CONFIRMED. publish writes and renames with no sync (crop lib.rs:399-401).

- Confidence: high
- Kind: crash-window
- Trigger: Power loss or OS crash after the rename's metadata is committed but before the temp's data is flushed. publish writes the temp with std::fs::write (399), sets its time (400), and renames it over out (401), with no sync\1all in between.
- Consequence: The name at out holds a zero-length or partly written file, and the earlier good copy at out has already been replaced. The comment's guarantee 'complete or not there' does not hold across a crash. The writeback path does this correctly (sync before the copy).
- Suggested fix: Sync the temp's contents (open it for write and call sync\1all, as writeback's sync helper does) before the rename, and sync the directory on Unix after it.
- Locations:
  - `crates/crop/src/lib.rs:399` std::fs::write(temp, bytes).with\1context(\1\1 format!("writing {}", shown(out)))?;
  - `crates/crop/src/lib.rs:401` std::fs::rename(temp, out).with\1context(\1\1 format!("writing {}", shown(out)))?;

### SEC-085 [medium/data-integrity] Untagged copy published before its tags are written — CONFIRMED

- Verdict: CONFIRMED. publish (crop lib.rs:382) runs before tag\1copy (:125). A crash between them leaves the untagged copy.

- Confidence: high
- Kind: crash-window
- Trigger: A crash (kill or power loss) after publish at line 382 and before tag\1copy at line 125, for TIFF or compressed-BMP copies (metadata false). The tag step runs only after the copy is already at out.
- Consequence: A complete-looking copy without its EXIF, XMP or ICC data sits at out and replaced any earlier copy there. Nothing records it (save writes no catalog row), so the loss is silent until the next save. The in-process failure path removes out, but a crash does not run it.
- Suggested fix: Tag the temp (as cut's with\1tags does, with the extension kept) before the rename, so only a fully tagged copy is ever published at out.
- Locations:
  - `crates/crop/src/lib.rs:382` publish(&temp\1path, &out\1path, &cropped.bytes, &source\1path)?;
  - `crates/crop/src/lib.rs:125` if let Err(e) = tag\1copy(&mut exiftool, &source, &out) {

### SEC-086 [medium/data-integrity] Crash during in-place cut leaves a corrupt original; full cut sits in an unread temp — CONFIRMED

- Verdict: CONFIRMED. write\1into writes, sets length, then syncs (writeback file.rs:79-83, inplace.rs:46). A crash mid-write corrupts the original.

- Confidence: high
- Kind: crash-window
- Trigger: Process kill or power loss during replace, after write\1all of the new bytes into the original (file.rs:80) and before set\1len and sync\1all (81-82). A cut that is shorter than the original leaves the old tail after the new prefix; a longer one leaves a half-written new prefix over old bytes.
- Consequence: The original file is corrupt, with no backup. The catalog still records the old size and mtime, so a later scan re-analyses the corrupt file. The complete cut exists only in .\1name\1.cut.pg-write beside it, which nothing reads back after a crash (discard\1temps covers only queued-write temps) and nothing reports. The in-place design is documented as non-atomic, but the docs cover only reported failures, not crashes.
- Suggested fix: Record the cut's temp path in the catalog before the copy starts, so the next start can finish or restore it; or keep the original intact until the whole content is durable (not possible with identity-preserving in-place copy, so the record is the minimum).
- Locations:
  - `crates/writeback/src/inplace.rs:46` write\1into(file, bytes).map\1err(incomplete)?;
  - `crates/writeback/src/file.rs:80` file.write\1all(data)?;
  - `crates/writeback/src/file.rs:81` file.set\1len(u64::try\1from(data.len())?)?;

### SEC-087 [low/data-integrity] Catalog-only turn not carried into compressed-BMP copies — CONFIRMED

- Verdict: CONFIRMED. BMP rotate returns Catalog when rewrite gives None (edit rotate.rs:76-78). Crop then reads stored pixels. Medium confidence.

- Confidence: medium
- Kind: wrong-result
- Trigger: A compressed (RLE) BMP is turned with R. rotate returns Outcome::Catalog (rotate.rs:76-79), so the turn lives only in the catalog. save later crops the stored pixels (crop.rs:373 via reencoded) and writes a BMP with no orientation, so the copy shows unturned while the app shows it turned.
- Consequence: The saved copy differs in orientation from what the user sees and turned before the crop. No error or notice is produced.
- Suggested fix: Either carry the catalog turn into the copy's bytes (not possible for BMP, so state the limit to the user) or refuse to save a copy of a file whose turn is catalog-only.
- Locations:
  - `crates/edit/src/rotate.rs:76` Format::Bmp =\1 match rewrite::bmp::rewrite(bytes, Op::Orient(total))? {
  - `crates/crop/src/lib.rs:373` let cropped = crop(bytes, s.format, s.width, s.height, s.cut)?;
  - `crates/catalog/src/lookup.rs:175` let (width, height) = if orientation.swaps\1axes() { (h, w) } else { (w, h) };

## F20: Catalog migration backup, rebuild and replacement

- Kind: persistent-write
- Entry: crates/catalog/src/core.rs:188-192 (migration path calls back\1up and ensure\1room\1to\1migrate) and crates/catalog/src/rebuild.rs:45 rebuild entry
- Endpoint: crates/catalog/src/rebuild.rs:45-46 std::fs::copy to .partial then rename to catalog.pre-vN.bak; crates/catalog/src/rebuild.rs:96 std::fs::rename(replacement, path); pg\1dbfile::move\1aside of the old file to .replaced
- Summary: On a version change or unreadable schema, the catalog is copied aside, checkpointed, and rebuilt into a .building file, then the old file is moved aside (kept, never deleted) and the new file renamed into the old name. Row carry-over reads the old database (F05). Replaced files stay on disk for the user to remove.

### SEC-088 [medium/data-integrity] Catalog name left vacant between move-aside and rename — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-017: the empty name between move\1aside and rename (rebuild.rs:94-96, core.rs:180).

- Confidence: high
- Kind: crash-window
- Trigger: Process dies (power loss, kill) between rebuild.rs:94 (move\1aside moves catalog.sqlite to .replaced) and rebuild.rs:96 (rename of .building into catalog.sqlite). The same state follows a failed rename at line 96 (Windows sharing violation, for example from a scanner holding the just-written .building file), which returns Err without moving .replaced back.
- Consequence: Nothing is at the catalog path. The next start's Connection::open (core.rs:125) creates an empty file, version 0 is treated as a fresh catalog (core.rs:180 and 198), and no Rebuilt is returned, so no rebuild notice is posted (app.rs:64). The user sees an empty library with no explanation. The old rows are intact in catalog.sqlite.replaced and the carried rows sit in catalog.sqlite.building, which nothing promotes; .building is only deleted by a later rebuild (rebuild.rs:76). Data is not destroyed, but recovery is manual and silent.
- Suggested fix: Never leave the name vacant: keep the old file by a no-replace link or copy to .replaced first, then replace path in one rename of the .building file over it, and on failure move the kept file back. Alternatively, when path is absent and .building (or .replaced) exists at open, finish or roll back the rebuild instead of creating an empty catalog.
- Locations:
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/catalog/src/core.rs:180` if version == Some(0) && rebuilt.is\1none() {

### SEC-089 [low/data-integrity] Pre-migration backup copy is never synced and a truncated copy is kept forever — CONFIRMED

- Verdict: CONFIRMED. back\1up copies without sync and renames (rebuild.rs:45-46). Latent: MIGRATIONS is empty.

- Confidence: high
- Kind: partial-write
- Trigger: Power loss after the copy at rebuild.rs:45 reaches the .partial name and the rename at line 46 publishes it as .pre-vN.bak, but before the copied data is flushed. Nothing calls sync\1all on the copy before the rename. Later runs then see the existing backup at line 40 and return early, so the zero-length or partial copy is never remade. The path is latent: it needs a migration, and MIGRATIONS is empty (schema.rs:82), so core.rs:191 never reaches back\1up.
- Consequence: Once a migration is added, the only pre-migration copy can be truncated after a crash. If the migration then fails or the power is lost mid-migration, the copy the user is told to restore from is unusable and the early return hides that it was never remade.
- Suggested fix: Open the .partial file, call sync\1all before the rename (as app/src/store.rs:121 does), and compare the copy's length with the source's; when the existing backup does not match, remake it rather than returning early.
- Locations:
  - `crates/catalog/src/rebuild.rs:40` if backup.exists() {
  - `crates/catalog/src/rebuild.rs:45` std::fs::copy(db\1file(path)?, &partial)
  - `crates/catalog/src/rebuild.rs:46` .and\1then(\1\1\1 std::fs::rename(&partial, &backup))

### SEC-090 [low/data-integrity] A stale pre-migration backup is kept and the error points at it — CONFIRMED

- Verdict: CONFIRMED. back\1up returns early when the backup exists (rebuild.rs:40-41). Latent: no migration yet.

- Confidence: medium
- Kind: wrong-target
- Trigger: A catalog.sqlite.pre-v1.bak from an earlier migration exists, and the catalog at v1 is later a different file (a manual restore of an older copy, or an older build writing to a v1 file after a downgrade), and then migrates. The backup check at rebuild.rs:40 returns early, so the current file is never copied. Latent: back\1up is not reachable until a migration exists.
- Consequence: The copy that the upgrade message names as the catalog as it was (core.rs:199-203) is an earlier file, and the changes made in between are in no copy. A user who restores from it after a failed migration loses those changes without being told.
- Suggested fix: Do not trust a backup by its name alone: keep the existing one aside (no-replace rename to a numbered name, as move\1aside does) and make a fresh copy of the current file, or check the backup's identity (size and mtime or catalog\1id) against the current file before reusing it.
- Locations:
  - `crates/catalog/src/rebuild.rs:40` if backup.exists() {
  - `crates/catalog/src/rebuild.rs:41` return Ok(());
  - `crates/catalog/src/core.rs:194` back\1up(&conn, path, found)?;

### SEC-091 [low/data-integrity] std::fs::rename replaces its target where the layer's contract is no-replace — CONFIRMED

- Verdict: CONFIRMED. Both renames (rebuild.rs:46, 96) use std::fs::rename, which replaces its target.

- Confidence: medium
- Kind: overwrite
- Trigger: A file appears at the backup name between the exists check (rebuild.rs:40) and the rename at line 46, or at catalog.sqlite between move\1aside (line 94) and the rename at line 96 (another program creating the catalog file in that window, which the exclusive .live hold does not stop). std::fs::rename then overwrites it silently.
- Consequence: A file that appeared is replaced without a trace. The dbfile module documents that its renames never replace a file (remove.rs:1-2, :20), and these two renames do not follow that rule.
- Suggested fix: Use pg\1platform::file::rename\1no\1replace for the backup rename. For line 96, pair it with the fix for the vacant-name finding, so the replacement is a deliberate replace of a file this process just moved aside, or fail with AlreadyExists when the name is taken.
- Locations:
  - `crates/catalog/src/rebuild.rs:46` .and\1then(\1\1\1 std::fs::rename(&partial, &backup))
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/platform/src/file.rs:216` /// Rename 'from' to 'to' unless 'to' exists, which is then 'AlreadyExists' and nothing moves. std's 'rename' replaces

### SEC-092 [medium/data-integrity] Any carry error replaces the catalog with an empty one, with no room check before the copy — CONFIRMED

- Verdict: CONFIRMED. The fallback at rebuild.rs:78-86 runs on any carry error. ensure\1room\1to\1migrate covers only migrations (core.rs:192).

- Confidence: high
- Kind: missing-check
- Trigger: The volume has room for an empty catalog but not for the copy (for example, a large catalog on a nearly full drive). The copy at rebuild.rs:78 fails with SQLITE\1FULL inside its single transaction, which rolls back. The fallback at line 83 then succeeds with an empty file, and move\1aside at line 94 takes the real catalog aside. Nothing checks free space before the copy: ensure\1room\1to\1migrate (lifecycle.rs:260) runs only on the migration path (core.rs:192).
- Consequence: The app opens with an empty library. The rows (folders, roots, ratings, analyses) are kept in catalog.sqlite.replaced and the reason is in the notice, so nothing is lost, but restoring them is manual. A transient I/O error is treated the same as a row the layout refuses.
- Suggested fix: Make the rebuild check free space as the migration does (about the catalog's size) before the copy, and treat I/O or full-disk errors as a failed rebuild that leaves the catalog in place. Reserve Carried::Nothing for refused rows (CHECK, NOT NULL, UNIQUE, rules mismatch).
- Locations:
  - `crates/catalog/src/rebuild.rs:78` Ok(()) =\1 match make\1replacement(&replacement, set, rules, Some(path)) {
  - `crates/catalog/src/rebuild.rs:83` make\1replacement(&replacement, set, rules, None)?;
  - `crates/catalog/src/rebuild.rs:84` Carried::Nothing { reason: format!("{e:#}") }
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")

## F21: Catalog sidecar deletion and move-aside

- Kind: persistent-write
- Entry: crates/dbfile/src/remove.rs:47 remove\1database (deletes main file and -wal/-shm sidecars) called from crates/catalog/src/rebuild.rs:52 and :54
- Endpoint: crates/dbfile/src/remove.rs:20-45 move\1aside via rename\1no\1replace (platform/src/file.rs:221 MoveFileExW on Windows) of each part to a free .N name
- Summary: Deletes the building file and its sidecars before a rebuild (std::fs::remove\1file, NotFound ignored). Moves aside the live catalog with no-replace renames. Both run only under the exclusive lock (remove.rs doc comment), but the deletion targets are derived from the catalog path.

### SEC-093 [medium/data-integrity] Crash between move-aside and promote leaves an empty live catalog — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-017 and SEC-098: the empty catalog path after move\1aside (rebuild.rs:94-96).

- Confidence: high
- Kind: crash-window
- Trigger: Process killed or power lost (or the rename at rebuild.rs:96 fails, e.g. a Windows sharing violation) after move\1aside (rebuild.rs:94) has moved the old catalog to .replaced and before std::fs::rename(&replacement, path) (rebuild.rs:96) completes. The next start is the trigger for the consequence.
- Consequence: The next Catalog::prepare creates an empty file at path (core.rs:125, Connection::open with CREATE), sees version 0 and an empty schema, so why is None (core.rs:136-147). No rebuild runs and rebuilt stays None (no notice). bring\1to then makes a fresh empty baseline. The fully carried replacement sits unpromoted at catalog.sqlite.building. The user's rows are not lost on disk (the original is at .replaced), but the app silently starts with an empty catalog. Recovery needs manual steps.
- Suggested fix: Open an existing catalog path without SQLITE\1OPEN\1CREATE in prepare, so a missing catalog is detected as missing. On start, promote a complete .building (or report it) when the path is absent and .replaced exists, and record the rebuild's step in a marker so the promotion is done or reported rather than silently skipped.
- Locations:
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/catalog/src/core.rs:125` let mut conn = Connection::open(db\1file(path)?).with\1context(\1\1 format!("catalog {}", path.display()))?;
  - `crates/catalog/src/core.rs:180` if version == Some(0) && rebuilt.is\1none() {

### SEC-094 [medium/data-integrity] Any carry error empties the live catalog, including disk full — CONFIRMED

- Verdict: CONFIRMED. Carry errors take the same empty-catalog fallback (rebuild.rs:80-86, 94). Related to SEC-092 with a different trigger.

- Confidence: high
- Kind: lost-update
- Trigger: The copy into catalog.sqlite.building fails for any reason (SQLITE\1FULL when the drive fills during carry\1rows, SQLITE\1BUSY, an I/O error), or the read of rules\1mismatch fails for a transient reason. The rebuild has no room check (ensure\1room\1to\1migrate runs only for migrations, core.rs:192), so a large catalog on a nearly full drive reaches this.
- Consequence: The error branch removes the replacement, makes an empty one, and moves the live catalog aside (rebuild.rs:94). The active catalog becomes empty and the full library is kept only at .replaced. The fallback is taken even though the failure is not a schema or rules problem, so a transient failure silently demotes the user's catalog, and the user must restore it by hand.
- Suggested fix: Fall back to an empty replacement only for the reasons the old file is unusable (a schema or rules difference, Damage::of). For any other error from make\1replacement or rules\1mismatch, return the error and leave the catalog at path untouched, and check free space before the copy as the migration path does.
- Locations:
  - `crates/catalog/src/rebuild.rs:69` Err(e) =\1 Err(format!("its path rules can't be read: {e}")),
  - `crates/catalog/src/rebuild.rs:80` Err(e) =\1 {
  - `crates/catalog/src/rebuild.rs:82` remove\1database(&replacement)?;
  - `crates/catalog/src/rebuild.rs:84` Carried::Nothing { reason: format!("{e:#}") }

### SEC-095 [low/data-integrity] Sidecars move before the main file; a failed main move strands committed WAL changes — CONFIRMED

- Verdict: CONFIRMED. move\1aside renames the sidecars first (remove.rs:30-37). A failed main rename leaves the WAL orphaned.

- Confidence: medium
- Kind: partial-write
- Trigger: The WAL holds committed frames when move\1aside runs, which needs the TRUNCATE checkpoint (rebuild.rs:63) to have been incomplete, for example a busy reader that does not take the shared .live hold. Then the main rename at remove.rs:34 fails or the process dies after the -wal rename at remove.rs:34 and before the main rename.
- Consequence: The main file stays at path without its WAL. The rebuild returns an error. On the next start the rebuild copies the WAL-less main, so the frames in the moved .replaced-wal are never carried. move\1aside's base search then selects .replaced.1 because .replaced-wal is taken, so the orphan WAL is never reattached. The doc comment claims the opposite ordering protects the main file's changes, so it misleads the reader.
- Suggested fix: Check the checkpoint's busy result (query the wal\1checkpoint row and fail when busy is non-zero) before any file is moved, so a WAL with frames stops the rebuild rather than moving it. Correct the doc comment at remove.rs:15-16 to state what a partial move leaves behind.
- Locations:
  - `crates/dbfile/src/remove.rs:15` /// changes its sidecars held. The parts share one name, so they are found free together; no connection to
  - `crates/dbfile/src/remove.rs:30` for part in PARTS {
  - `crates/dbfile/src/remove.rs:34` rename\1no\1replace(&from, &to)
  - `crates/catalog/src/rebuild.rs:63` old.execute\1batch("PRAGMA wal\1checkpoint(TRUNCATE)")?;

### SEC-096 [low/data-integrity] Derived .building name is deleted without an ownership check — CONFIRMED

- Verdict: CONFIRMED. remove\1database deletes the .building name unconditionally (rebuild.rs:76, 136-138). The doc says the file is always this build's, and nothing checks that.

- Confidence: medium
- Kind: wrong-target
- Trigger: A file named catalog.sqlite.building (or its -wal, -shm, -journal) exists beside the catalog, for example a user's file or one left by another tool, when a rebuild starts.
- Consequence: remove\1database deletes it unconditionally before the rebuild, so a file this build did not make is lost. The doc comment says it is only ever called on a file this build made, but nothing checks that.
- Suggested fix: Move the stale .building aside with move\1aside (no-replace, never deleted), or verify it by application\1id and schema before deleting it.
- Locations:
  - `crates/catalog/src/rebuild.rs:76` remove\1database(&replacement)?;
  - `crates/catalog/src/rebuild.rs:136` /// build made (a replacement left by an interrupted rebuild), never on the catalog itself.
  - `crates/catalog/src/rebuild.rs:138` pg\1dbfile::remove\1database(path).map\1err(\1e\1 Error::Other(anyhow!("{e}")))
  - `crates/dbfile/src/remove.rs:47` match std::fs::remove\1file(&file) {

### SEC-097 [low/data-integrity] Final promote replaces any file that appears at the catalog path — CONFIRMED

- Verdict: CONFIRMED. The rename at rebuild.rs:96 replaces any file at the catalog path, unlike the rename\1no\1replace calls elsewhere.

- Confidence: medium
- Kind: race
- Trigger: A process that does not take the .lock or the shared .live hold (for example a tool that opens catalog.sqlite with CREATE) creates a file at path between move\1aside (rebuild.rs:94) and the rename at rebuild.rs:96.
- Consequence: std::fs::rename replaces the existing file on Windows (MOVEFILE\1REPLACE\1EXISTING) and on Unix, so that file is overwritten. Every other move in this flow uses rename\1no\1replace, which refuses to replace.
- Suggested fix: Use pg\1platform::file::rename\1no\1replace for the promote step and return an error when the target exists, instead of std::fs::rename.
- Locations:
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/platform/src/file.rs:226` if unsafe { MoveFileExW(from.as\1ptr(), to.as\1ptr(), 0) } == 0 {

## F22: Lock and live sidecar files beside catalog and log

- Kind: persistent-write
- Entry: crates/dbfile/src/lock.rs:28 OpenOptions create(true) for \1db\1.lock and crates/dbfile/src/lock.rs:82 for \1db\1.live
- Endpoint: crates/log/src/file.rs:105 \1log\1.lock creation; lock state used by crates/catalog/src/lifecycle.rs:45-66 take\1lock / try\1exclusive / take\1shared
- Summary: Lock files are created beside the catalog and log files and never removed by this code path. They are the cross-process mutual exclusion for the catalog (F20 rebuild depends on them).

### SEC-098 [medium/data-integrity] Rebuild leaves no catalog at its path between the two renames — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-017: no catalog file between move\1aside and the rename (rebuild.rs:94-96).

- Confidence: high
- Kind: crash-window
- Trigger: Crash, power loss, or a failed rename in rebuild.rs between move\1aside(path, '.replaced') at line 94 and std::fs::rename(&replacement, path) at line 96. The catalog path is then missing and the rebuilt copy sits at \1db\1.building.
- Consequence: The next Catalog::prepare creates an empty file at \1db\1 (core.rs:125) and brings it up as a new catalog. Nothing reads .replaced or .building to recover, so the library appears empty. The old catalog is still at \1db\1.replaced and the carried rows are at \1db\1.building, but the app does not find either. The data is kept, not destroyed, but it is not reachable through the app.
- Suggested fix: Have prepare refuse to create a catalog when \1db\1 is missing and \1db\1.replaced or \1db\1.building exists, and finish the rename of the replacement into place under the exclusive hold. Alternatively, rename the replacement over the original in one step, keeping the original as a copy first.
- Locations:
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/catalog/src/core.rs:125` let mut conn = Connection::open(db\1file(path)?).with\1context(\1\1 format!("catalog {}", path.display()))?;

### SEC-099 [low/data-integrity] A failing shift of the older log files stops rotation for good, so the log grows without bound — CONFIRMED

- Verdict: CONFIRMED. If rename of the older files fails, rotate returns early (log file.rs:78-80). The copy fallback at :92 is never reached.

- Confidence: high
- Kind: missing-check
- Trigger: gallery.log.1 or gallery.log.2 is held open by another program without delete sharing, so the rename in finish() fails with a non-NotFound error. The first rename (log to .next) succeeds, so .next remains. Every later append sees .next and calls rotate, which returns early at line 79.
- Consequence: The log is never rotated again while the older file stays held. The copy-and-empty fallback at line 92 is never reached, because it sits after the early return. gallery.log grows past its 5 MB cap without limit, which can fill the drive.
- Suggested fix: Do not return when finish() fails. Let the size check (and the copy fallback past twice the cap) run anyway, or make the fallback apply whenever the file is past twice its cap regardless of the shift state.
- Locations:
  - `crates/log/src/file.rs:78` if aside(&self.path).exists() && self.finish().is\1err() {
  - `crates/log/src/file.rs:79` return;
  - `crates/log/src/file.rs:88` let \1 = self.finish();
  - `crates/log/src/file.rs:92` Err(\1) if self.len() \1 self.cap.saturating\1mul(2) =\1 {

### SEC-100 [low/data-integrity] Thread-local thumbnail read connections outlive the cache's shared hold — PARTIAL

- Verdict: PARTIAL. READERS is cleared only on the calling thread (thumbcache lib.rs:378). Whether the worker's connection blocks removal depends on thread lifetimes not traced here.

- Confidence: medium
- Kind: race
- Trigger: In one process, a worker thread reads a thumbnail, so with\1reader pushes a read connection to thumbs.sqlite into its READERS. The cache is then dropped on another thread, and close() clears only the calling thread's READERS. A new ThumbCache::open on the same path later finds the file stale or damaged (for example the catalog id changed), so start-over takes the exclusive hold, which succeeds because no Live hold remains.
- Consequence: On Windows, remove\1database fails with a sharing violation (the open read connection), so the cache cannot be started over and open returns an error until the worker thread exits. On Unix, the delete succeeds, but the worker thread's stale connection still reads the unlinked file, so get\1nearest\1into can return thumbnails keyed by old file ids for the same path.
- Suggested fix: Clear or invalidate the read connections of every thread on close (for example, a generation counter checked by with\1reader), or have with\1reader take a shared Live hold that the cache's close releases with the connection.
- Locations:
  - `crates/thumbcache/src/reader.rs:238` readers.push((path.to\1path\1buf(), conn));
  - `crates/thumbcache/src/lib.rs:378` READERS.with(\1readers\1 readers.borrow\1mut().clear());
  - `crates/thumbcache/src/schema.rs:331` pg\1dbfile::remove\1database(path).map\1err(\1e\1 {

## F23: Log file append, rotation and perf-log sinks

- Kind: persistent-write
- Entry: crates/log/src/file.rs:59 append (OpenOptions create+append) and crates/log/src/file.rs:105 lock creation
- Endpoint: crates/log/src/file.rs:129 std::fs::copy to aside, :130 truncate, :136 rename rotation, :188 rename; PG\1PERF\1LOG target written by crates/desktop/src/trace.rs:30 and crates/cli/src/lib.rs:106
- Summary: The app log grows by append; past a cap it is copied, truncated and rotated to .N names. PG\1TRACE and PG\1PERF\1LOG (env) choose whether and where a perf log is written; the log view reads it back via --log-view (desktop/src/command\1line.rs:425).

### SEC-101 [medium/data-integrity] A torn last line glues the next record onto it, so that record is lost as structure and shows twice — CONFIRMED

- Verdict: CONFIRMED. LogFile::append never checks for a final newline (log file.rs:59). parse treats the joined line as a plain record (record.rs:83, 98).

- Confidence: high
- Kind: partial-write
- Trigger: The process stops (power loss, kill, or a write that fails part-way such as disk full) after part of a record's bytes are written but before its newline. The next append (the next run's session record from init, or any later record) writes straight after the fragment, because LogFile::append never checks for a missing final newline. On NTFS, power loss can also leave a NUL-filled tail, with the same result.
- Consequence: The joined line starts with '{' but is not valid JSON, so parse() turns it into one plain record with no t, pid, session or seq. The record's identity is lost. Feed::merge keeps file records whose pid differs from this run, so the run's session record shows twice in the log view: once from the in-memory ring and once as a plain text row. perf's parse\1sessions drops the same glued line silently.
- Suggested fix: Before writing, check whether the file is non-empty and does not end in '\n', and if so write a leading newline so the fragment stays its own line. Alternatively, make parse() split a line at a '}{' boundary and parse each part.
- Locations:
  - `crates/log/src/file.rs:59` OpenOptions::new().create(true).append(true).open(&self.path)?.write\1all(line.as\1bytes())
  - `crates/log/src/record.rs:83` if line.starts\1with('{')
  - `crates/log/src/record.rs:98` (None, \1) =\1 records.push(plain\1record(String::new(), line.to\1string())),

### SEC-102 [medium/data-integrity] A rotation whose shift keeps failing stops rotating altogether, so the log grows without bound — CONFIRMED

- Verdict: CONFIRMED. A failing shift keeps the aside file (file.rs:78-79, 120). The copy fallback is never reached.

- Confidence: high
- Kind: unbounded-growth
- Trigger: Another program holds gallery.log.2 (or .1) open without FILE\1SHARE\1DELETE, so rename(.1, .2) fails every time. After the first failed shift, gallery.log.next is left behind. From then on every append sees the aside file, calls rotate(), takes the lock and runs finish(), which fails at the .1-to-.2 step and returns. The copy fallback is never reached, because it only sits after the rename of the live file.
- Consequence: gallery.log (and the perf file) grows without limit, contrary to the module doc's 'runs a few lines past its cap'. read\1all and perf's read\1to\1string then load every byte into memory each time the view opens or a summary is read.
- Suggested fix: Apply the past-twice-the-cap copy-and-empty fallback to the finish() failure path too, so the cap still holds while a shift keeps failing, instead of returning from rotate() with no fallback.
- Locations:
  - `crates/log/src/file.rs:78` if aside(&self.path).exists() && self.finish().is\1err() {
  - `crates/log/src/file.rs:79` return;
  - `crates/log/src/file.rs:120` Err(e) =\1 return Err(e),
  - `crates/log/src/file.rs:92` Err(\1) if self.len() \1 self.cap.saturating\1mul(2) =\1 {

### SEC-103 [medium/data-integrity] The perf summary reads only the current file, so records rotated into .1 or .2 are missing — CONFIRMED

- Verdict: CONFIRMED. summarize\1session reads only the named file (perf lib.rs:472) while rotation moves records to .1 (cli lib.rs:77).

- Confidence: high
- Kind: incomplete-read
- Trigger: PG\1PERF\1LOG names a file appended to across runs (the documented use), or one run writes past the 16 MiB cap. The rotation renames the file to .1, so the run's earlier records (its session record and first steps) are in .1. summarize\1session then reads only the named path.
- Consequence: The end-of-run table and 'gallery perf' under-report: steps are missing, the wall time (computed from the records present) is wrong, and a run that fell entirely in rotated files prints 'no perf records for this run'.
- Suggested fix: Have pg\1perf read the rotated files as well (the aside file, .2, .1, then the live file, in that order), for example through a public pg\1log read helper, before parsing the sessions.
- Locations:
  - `crates/cli/src/lib.rs:77` match pg\1perf::summarize\1session(&file, pg\1perf::session\1id()) {
  - `crates/perf/src/lib.rs:472` let text = std::fs::read\1to\1string(path)?;
  - `crates/perf/src/lib.rs:466` let text = std::fs::read\1to\1string(path)?;

### SEC-104 [low/data-integrity] The perf summary fails on the whole file when one torn UTF-8 byte is present, against its documented skip — CONFIRMED

- Verdict: CONFIRMED. read\1to\1string fails on any invalid UTF-8 (perf lib.rs:466, 472), against the skip the doc promises (:464).

- Confidence: high
- Kind: missing-check
- Trigger: A torn write ends inside a multi-byte UTF-8 character. perf session meta holds catalog\1dir, which may be non-ASCII. The torn bytes stay mid-file once the next line is glued to them (finding 1). read\1to\1string then fails on the whole file.
- Consequence: 'gallery perf' reports 'can't read' and the end-of-run summary prints an error instead of its table, for every later run, until the file is rotated out. The doc says a torn last line is skipped, which is not what happens.
- Suggested fix: Read the bytes and use String::from\1utf8\1lossy (as file::read\1all does) before splitting lines, so a bad line is skipped and the rest is kept.
- Locations:
  - `crates/perf/src/lib.rs:464` /// Lines that aren't records (a torn last line, a line another version wrote) are skipped.
  - `crates/perf/src/lib.rs:466` let text = std::fs::read\1to\1string(path)?;
  - `crates/cli/src/lib.rs:79` Err(e) =\1 eprintln!("perf log: can't read {}: {e}", file.display()),

### SEC-105 [low/data-integrity] The log view reads the files without the rotation lock, so a rotation between reads can drop a whole file's records from the view — CONFIRMED

- Verdict: CONFIRMED. read\1all reads without the rotation lock and in an order that can miss a rotated file (log file.rs:152-157). Medium confidence.

- Confidence: medium
- Kind: race
- Trigger: The log view's reader thread calls read\1all while a rotation runs, in this process or another instance. read\1all reads .2, .1, the aside file, then the live file, without the lock. Content moves file to .next to .1 to .2, which is backwards against the read order. Example: the reader reads .2 = A, then rotation runs (.1 = B moves to .2, the live file C moves to .next then .1), then the reader reads .1 = C, finds no .next, and reads the new live file D. The view shows A, C, D and B is missing.
- Consequence: The view (not the disk) omits a rotated file's records, and Feed::merge cannot detect the gap. The on-disk log is unaffected.
- Suggested fix: Hold the rotation lock (blocking, which the rotation's try\1lock then skips) around read\1all, or re-list the file set and retry until the names are stable.
- Locations:
  - `crates/log/src/file.rs:157` if let Ok(bytes) = std::fs::read(&file) {
  - `crates/log/src/file.rs:85` match (self.rename)(&self.path, &aside(&self.path)) {
  - `crates/desktop/src/log_model_bridge.rs:133` let from\1files = path.map(\1p\1 pg\1log::read(&p, pg\1log::RING\1RECORDS)).unwrap\1or\1default();

### SEC-106 [low/data-integrity] A crash inside the copy-and-empty fallback duplicates history and drops an older file early — CONFIRMED

- Verdict: CONFIRMED. copy\1aside writes the aside copy before the truncate (file.rs:129-130). A crash between them duplicates history on the next run.

- Confidence: high
- Kind: crash-window
- Trigger: The live file is past twice the cap and its rename fails persistently. The process dies after std::fs::copy has written the aside file and before the in-place truncate. On the next run the aside file (a full copy) is shifted to .1 by finish(). The live file still holds the same text, so it is copied and shifted again, and the older .1 is overwritten at .2.
- Consequence: The history holds the same text twice (once in .1 and once in .2), and one or two older history files are dropped early. Records are duplicated in the log view. Nothing current is lost.
- Suggested fix: Copy to a temporary name and rename it to the aside name only after the live file is emptied, or drop an aside file at startup when it matches the live file, instead of shifting it.
- Locations:
  - `crates/log/src/file.rs:129` std::fs::copy(&self.path, aside(&self.path))?;
  - `crates/log/src/file.rs:130` OpenOptions::new().write(true).truncate(true).open(&self.path).map(drop)
  - `crates/log/src/file.rs:93` if self.copy\1aside().is\1ok() {

## F24: Preferences and unreadable-file quarantine (JSON store)

- Kind: persistent-write
- Entry: crates/app/src/store.rs:48 std::fs::read of the prefs or store file and serde\1json parse
- Endpoint: crates/app/src/store.rs:119-123 write\1atomically (File::create \1name\1.tmp, sync\1all, rename over the file); quarantine at crates/app/src/store.rs:~112 via pg\1platform::file::keep\1aside (renames unreadable file to \1name\1.bad.N)
- Summary: Preferences are read from a user-writable JSON file (folder values may be hex-encoded non-UTF-8 paths, prefs.rs:72-113). Unreadable content is renamed aside and defaults are used. Writes replace the file atomically.

### SEC-107 [medium/data-integrity] Transient read error quarantines a readable prefs file — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-030: any non-NotFound read error quarantines (store.rs:54).

- Confidence: high
- Kind: wrong-target
- Trigger: gallery.json exists and is readable, but std::fs::read fails with a non-NotFound error at launch (sharing violation or access denied, for example another program holding the file without share-read). store.rs:54 then calls quarantine, which renames the file to gallery.json.bad and uses defaults.
- Consequence: The user's active preferences are reset to defaults. The next change writes defaults plus that change to gallery.json. The original content survives only in gallery.json.bad, and the notice says the file did not parse, not that it could not be read.
- Suggested fix: Only quarantine on a parse error. For other read errors, return the defaults with a notice that says the file could not be read, and leave the file in place so the next open can retry.
- Locations:
  - `crates/app/src/store.rs:54` Err(e) =\1 (T::default(), Some(quarantine(&path, e.to\1string()))),

### SEC-108 [low/data-integrity] Older successful save clears the notice of a newer failed save — CONFIRMED

- Verdict: CONFIRMED. A successful older save calls inbox.resolve(Save) (prefs.rs:197), and NextSave resolves PrefsNotSaved (notice.rs:840). Medium confidence.

- Confidence: high
- Kind: lost-update
- Trigger: A worker-queued save of change 5 is pending. On the Qt thread, Search::save runs prefs.update for change 6 and its write fails (transient disk error). PrefsNotSaved is posted. The worker then writes change 5 successfully, and PrefSave::write calls inbox.resolve(Trigger::Save), which clears PrefsNotSaved.
- Consequence: The file holds change 5, memory holds change 6, and no notice remains. Change 6 is silently lost on the next restart unless another change happens.
- Suggested fix: Clear the failure notice only when the write reached the latest change, for example by having Save::write report whether it wrote the newest sequence number. Alternatively, route the Yandex save through the same worker queue.
- Locations:
  - `crates/desktop/src/shell.rs:109` worker::run(move \1\1 {
  - `crates/app/src/prefs.rs:197` Ok(()) =\1 self.inbox.resolve(Trigger::Save),
  - `crates/app/src/yandex.rs:555` let \1 = prefs.update(\1p\1 p.yandex\1folder = Some(chosen));
  - `crates/model/src/notice.rs:840` \1 (Resolver::NextSave, Trigger::Save) =\1 true,

### SEC-109 [low/data-integrity] No parent-directory sync after the atomic rename — CONFIRMED

- Verdict: CONFIRMED. No parent-directory sync after the rename (store.rs:123).

- Confidence: high
- Kind: crash-window
- Trigger: A successful save whose rename (store.rs:123) completes, then power is lost before the directory entry reaches disk (ext4 and similar, Linux or macOS). Windows MoveFileEx is not affected the same way.
- Consequence: After reboot the file can revert to the previous preferences. The reported successful save is lost. The file is not torn or corrupt.
- Suggested fix: On Unix, open the parent directory and call sync\1all after the rename.
- Locations:
  - `crates/app/src/store.rs:123` std::fs::rename(&temp, path)

### SEC-110 [low/data-integrity] Failed write leaves a partial gallery.json.tmp behind — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-031 with a different write point (store.rs:119-121).

- Confidence: high
- Kind: leak
- Trigger: write\1all or sync\1all fails (disk full, device removed) at store.rs:120-121. The ? returns early, and the partially written gallery.json.tmp is not removed.
- Consequence: A stray partial temp file stays beside gallery.json. Nothing reads it, and the next save truncates it, so this is clutter rather than corruption. The platform's TempFile guard exists for exactly this case and is unused here.
- Suggested fix: Wrap the temp in pg\1platform::file::TempFile before File::create and call persist() after the rename succeeds.
- Locations:
  - `crates/app/src/store.rs:119` let mut file = File::create(&temp)?;
  - `crates/app/src/store.rs:120` file.write\1all(bytes)?;

### SEC-111 [low/security] Prefs file read whole into memory with no size cap — CONFIRMED

- Verdict: CONFIRMED. Prefs file read whole with no cap (store.rs:48).

- Confidence: high
- Kind: unbounded-alloc
- Trigger: gallery.json is replaced with a very large file (gigabytes, by hand or by a bug writing a runaway value). std::fs::read loads it all before parsing.
- Consequence: Startup memory use grows with the file size and can fail with out-of-memory. The writer is the local user, so the exposure is limited to self-inflicted or buggy files.
- Suggested fix: Check metadata().len() against a small cap (for example 1 MiB) before reading, and treat an oversize file as unreadable.
- Locations:
  - `crates/app/src/store.rs:48` let (value, unreadable) = match std::fs::read(&path) {

### SEC-112 [low/data-integrity] One wrongly typed field discards the whole preferences file — CONFIRMED

- Verdict: CONFIRMED. One mistyped field fails from\1slice for the whole struct, and the file is quarantined (store.rs:49-51).

- Confidence: high
- Kind: missing-check
- Trigger: A hand-edited or downgraded gallery.json has one field of the wrong type (for example "last\1file": null, "row\1height": "220", or a UTF-8 BOM before the brace). The entire from\1slice fails.
- Consequence: Every valid preference is reset to defaults, and the file is moved to gallery.json.bad. Recoverable from the .bad file, but the other settings are lost.
- Suggested fix: Deserialize field by field (for example into serde\1json::Value, then convert each field with its own default on failure) so one bad value falls back alone and the rest is kept. Strip a leading BOM before parsing.
- Locations:
  - `crates/app/src/store.rs:49` Ok(bytes) =\1 match serde\1json::from\1slice::\1T\1(&bytes) {
  - `crates/app/src/prefs.rs:21` #\1serde(default)\1

### SEC-113 [low/data-integrity] Quarantine failure lets the next save overwrite the unreadable file — CONFIRMED

- Verdict: CONFIRMED. If keep\1aside fails, the unreadable file stays and the next save replaces it (store.rs:102, 111).

- Confidence: medium
- Kind: missing-check
- Trigger: A parse failure at store.rs:51 where keep\1aside fails (Linux file system without RENAME\1NOREPLACE and without hard links, such as FAT or exFAT; the Windows case is less likely since the same lock would also block the save's replace). kept is Err, and the original stays at gallery.json.
- Consequence: The next successful save replaces gallery.json through write\1atomically, destroying the unreadable content. The notice reports keep\1error, so the user is told, but the Unreadable doc promises the file is not overwritten.
- Suggested fix: When kept is Err, do not write over the file on the next save until the user has been told, or copy the content to a separate name with create\1new before replacing it.
- Locations:
  - `crates/app/src/store.rs:111` let kept = pg\1platform::file::keep\1aside(path, ".bad").map\1err(\1e\1 e.to\1string());
  - `crates/app/src/store.rs:102` write\1atomically(&self.path, &json)?;

### SEC-114 [low/data-integrity] Hex name decoder accepts a leading '+' in each digit pair — CONFIRMED

- Verdict: CONFIRMED. u8::from\1str\1radix accepts a leading '+' (model name.rs:285).

- Confidence: high
- Kind: missing-check
- Trigger: A yandex\1folder entry of the form {"b": "+f"} (hand-edited, or corrupted). u8::from\1str\1radix accepts the optional '+' sign, so "+f" decodes to 0x0f instead of being rejected.
- Consequence: The folder name decodes to a wrong byte sequence, and the saved folder is a different path. Only hand-edited or corrupt files reach this, and the next save rewrites canonical hex.
- Suggested fix: Accept only the characters 0-9, a-f and A-F in each pair (for example by checking each byte with is\1ascii\1hexdigit) before calling from\1str\1radix.
- Locations:
  - `crates/model/src/name.rs:285` text.as\1bytes().chunks(2).map(\1pair\1 u8::from\1str\1radix(std::str::from\1utf8(pair).ok()?, 16).ok()).collect()

## F25: Recording and screenshot writes to caller-named paths

- Kind: persistent-write
- Entry: crates/desktop/src/controller.rs:61 PG\1RECORD env path, and crates/desktop/src/screenshot.rs:35 --screenshot path
- Endpoint: crates/desktop/src/controller.rs:61 std::fs::File::create(path) for the event recording; crates/desktop/src/screenshot.rs:50 write settle log and :55 write PNG bytes
- Summary: Two sinks write to paths chosen by environment or command line: the controller recording and the screenshot PNG plus .settle.txt log. Both use std::fs::write or create without temp files or no-replace checks.

### SEC-115 [low/data-integrity] Settle log written before a non-atomic PNG write — CONFIRMED

- Verdict: CONFIRMED. The settle log is written before the PNG, and the PNG write is a plain truncate-and-write (screenshot.rs:50, 55).

- Confidence: high
- Kind: partial-write
- Trigger: Process killed, or the disk fails, between the log write (screenshot.rs:50) and the end of the PNG write (screenshot.rs:55), or during it. Each write is truncate-create with no temp file, no rename, and no fsync.
- Consequence: The log can say 'settled' with no PNG (or an older PNG from an earlier run). A kill mid-write leaves a truncated PNG at the final path, which compare\1shots.py treats as a valid screenshot because it only checks that the file exists.
- Suggested fix: Write the PNG to a temp file beside the target and rename it into place, then write the log last (or write the log to a temp name and rename it too), so the log only describes an image that exists.
- Locations:
  - `crates/desktop/src/screenshot.rs:50` std::fs::write(log, format!("{report}\n"))?;
  - `crates/desktop/src/screenshot.rs:55` std::fs::write(path, png)

### SEC-116 [low/data-integrity] Earlier PNG survives a run that produced no image — CONFIRMED

- Verdict: CONFIRMED. write\1taken returns before the image write (screenshot.rs:51-53), so an earlier PNG at that path stays.

- Confidence: medium
- Kind: stale-artifact
- Trigger: A run at --screenshot=\1png\1 where the window is missing or the window cannot be encoded: png is empty, write\1taken returns at line 51-53 before the image write, and the log is already overwritten with this run's report. A PNG from an earlier successful run at the same path is left in place.
- Consequence: A caller that checks only for the PNG's existence (or opens it by name) reads the earlier run's screenshot as this run's output, while the fresh log says no window. The stale image is not removed or marked. The harness uses a per-run folder, so the impact is limited to reused paths.
- Suggested fix: On the no-image branch, remove any existing file at path (or write a marker that replaces it) before returning the error, so a failed run cannot leave the previous image in place.
- Locations:
  - `crates/desktop/src/screenshot.rs:51` if png.is\1empty() {

### SEC-117 [low/data-integrity] Failed screenshot write still exits with status 0 — CONFIRMED

- Verdict: CONFIRMED. screenshot\1taken reports the error only on stderr (screenshot.rs:39-40), and the process then quits (cpp screenshot.cpp:211).

- Confidence: high
- Kind: missing-check
- Trigger: --screenshot=\1path\1 where \1path\1 cannot be written (missing parent folder, read-only folder, or a directory at the path), or a window present but not encodable. write\1taken fails, screenshot\1taken prints to stderr only, and the caller then calls QCoreApplication::quit().
- Consequence: The process exits 0 with no PNG, so a shell or CI caller that trusts the exit status sees success. lib.rs documents status 1 only for the no-window case, so the failure is reported as success.
- Suggested fix: Have screenshot\1taken return whether the image was written, pass that back to C++ (or call QCoreApplication::exit(1) on failure), and document the exit status for write failures in lib.rs.
- Locations:
  - `crates/desktop/src/screenshot.rs:39` if let Err(e) = write\1taken(path, png, has\1window, report) {
  - `crates/desktop/cpp/screenshot.cpp:211` QCoreApplication::quit();

### SEC-118 [low/data-integrity] Empty --screenshot= writes a stray .settle.txt into the working directory — CONFIRMED

- Verdict: CONFIRMED. An empty value makes the log path ".settle.txt" in the working directory (screenshot.rs:35, 49).

- Confidence: high
- Kind: wrong-target
- Trigger: The argument --screenshot= (empty value). command\1line::path returns Some(""), so the run is scheduled. The log path becomes the bare name ".settle.txt", relative to the current directory.
- Consequence: A .settle.txt file is created in whatever directory the process started from, and the image write to "" then fails. The run exits 0 (see the exit-status finding). The empty value is not rejected as a bad option.
- Suggested fix: Reject an empty --screenshot value in the command-line parser (as a bad option, which refuses the run) so no log is written beside an empty path.
- Locations:
  - `crates/desktop/src/screenshot.rs:35` let Some(path) = command\1line::path("screenshot") else { return };
  - `crates/desktop/src/screenshot.rs:49` log.push(".settle.txt");
  - `crates/desktop/src/command_line.rs:418` pub(crate) fn path(name: &str) -\1 Option\1&'static Path\1 {

### SEC-119 [low/data-integrity] Failed recorder write leaves a truncated line that later records join — CONFIRMED

- Verdict: CONFIRMED. Unbuffered writeln! can leave a fragment, and the next record joins it (controller.rs:80).

- Confidence: high
- Kind: partial-write
- Trigger: A write error (disk full, pipe or drive error) partway through writeln!(file, "{line}") at controller.rs:80. The File is unbuffered, so the fragment that reached the file stays there, and the next record is appended after it with no newline.
- Consequence: The replay file has a corrupt JSON line, joined to the next record, so the replay test fails to parse that session. No data outside the diagnostic recording is affected.
- Suggested fix: After a failed write, stop recording (set the recorder to None and warn once), so a partial line is never followed by more records.
- Locations:
  - `crates/desktop/src/controller.rs:80` if let Err(e) = writeln!(file, "{line}") {

## F26: Qt and cxx-qt bridge boundary (QML, images, file watcher)

- Kind: boundary
- Entry: crates/desktop/src/\1\1bridge.rs (cxx-qt bridge declarations, 60+ bridge files) and crates/desktop/cpp/live\1reload.cpp:64-80 QFileSystemWatcher
- Endpoint: crates/desktop/src/lib.rs:188 add\1image\1provider (bytes from catalog served to QML); crates/desktop/src/live\1reload.rs:30 reload\1on\1change; crates/desktop/src/gallery\1bridge.rs and shell\1bridge.rs (unsafe paths into Rust state)
- Summary: Rust state crosses into Qt through cxx-qt generated unsafe calls, and QML receives image bytes and paths from the catalog. The live reload watcher watches the QML folder in dev mode and reruns QML on change. Unsafe pointer parameters (animation.rs:45, video\1source.rs:25) are passed from C++.

### SEC-120 [high/data-integrity] Cut-borders confirm rewrites originals the user never confirmed — CONFIRMED

- Verdict: CONFIRMED. The confirm acts on the entries at confirm time (edits.rs:185, 195), not the count shown at open (Main.qml:199). Background reloads can change the set (passes.rs:240, controller.rs:326).

- Confidence: high
- Kind: wrong-target
- Trigger: The user opens the filter bar's Cut borders. The dialog reads the count (say N) once in askCutBorders and is then modal. While it is open, the library's Borders pass stores borders for more files in the listed folder (passes.rs:240, s.stored \1 0). That publishes Imported, and Main.qml:130 onImported calls refreshGrid, which calls shell.reloadList. The controller's Event::Reload always opens the list (controller.rs:326), and apply() replaces the entries when no viewer is open (gallery/model.rs:127). Confirm then calls cutBordersConfirmed, which takes borders\1found() from the current entries, so N+M files are cut. A second trigger: a border mark set to not-a-border whose write is still queued stays Borders::Found in grid.entries() and is cut too (gallery/edits.rs:194-196 ignores mark\1writes).
- Consequence: Originals are rewritten in place with their borders cut, with no backup, for files the user did not select in the confirmed count. The cut is irreversible, and the count the dialog shows is wrong.
- Suggested fix: Snapshot the ids when the dialog opens (or pass them back from QML) and have cutBordersConfirmed cut only that set, excluding any id with a queued mark. Alternatively, refuse the confirm when the current set differs from the snapshot.
- Locations:
  - `crates/desktop/qml/Main.qml:199` cutDialog.count = galleryModel.cutBordersCount()
  - `crates/desktop/qml/Main.qml:130` onImported: win.refreshGrid()
  - `crates/library/src/passes.rs:240` if s.stored \1 0 {
  - `crates/client/src/controller.rs:326` Event::Reload =\1 vec!\1self.open\1list(true)\1,
  - `crates/desktop/src/gallery/model.rs:127` let viewing = self.viewing \1= 0;
  - `crates/desktop/qml/Main.qml:207` onConfirmed: galleryModel.cutBordersConfirmed()
  - `crates/desktop/src/gallery/edits.rs:185` let ids = self.borders\1found();
  - `crates/desktop/src/gallery/edits.rs:195` self.grid.entries().iter().filter(\1e\1 e.borders == Borders::Found).map(\1e\1 e.id).collect()
  - `crates/desktop/src/gallery/state.rs:170` self.mark\1writes.get(&file).and\1then(\1queue\1 queue.back().copied()).unwrap\1or(borders == Borders::Marked)

### SEC-121 [medium/security] Yandex originals of every result are kept in memory for the whole search — CONFIRMED

- Verdict: CONFIRMED. Each search keeps all its fetched results in memory (images.rs:100, 158). Medium confidence.

- Confidence: medium
- Kind: unbounded-alloc
- Trigger: A Yandex search returns many results (bounded only by the similar-pages cap, DEFAULT\1MAX\1SIMILAR\1PAGES = 5 in client.rs:25) whose files are large and hosted by remote sites. prefetch queues every result (images.rs:96-112), and fetch\1checked keeps the full Download, up to 100 MB each, in Fetched for as long as the search is shown (images.rs:152-159). A hostile host can serve a large decodable image, which passes the format and grid checks.
- Consequence: Memory use grows with the sum of all kept originals, up to results times 100 MB, and can exhaust memory and crash the window during an ordinary search.
- Suggested fix: Keep only the grid JPEG in memory and re-fetch or spill the original to disk on save, or cap the total bytes of kept originals and evict the oldest.
- Locations:
  - `crates/app/src/yandex/images.rs:100` t.slots.insert(id, Slot { url, state: State::Queued });
  - `crates/app/src/yandex/images.rs:158` Ok(Arc::new(Fetched { file, format, grid }))
  - `crates/yandex/src/fetch.rs:12` pub const MAX\1DOWNLOAD\1BYTES: u64 = 100 \1 1024 \1 1024;
  - `crates/desktop/src/providers.rs:100` let bytes = if original { &picture.file.bytes } else { &picture.grid };
  - `crates/yandex/src/client.rs:25` pub const DEFAULT\1MAX\1SIMILAR\1PAGES: u32 = 5;

### SEC-122 [low/data-integrity] Shutdown waits on Yandex downloads that stop() does not cancel — CONFIRMED

- Verdict: CONFIRMED. stop() sets a flag (images.rs:126) that get does not read. Provider threads can then hold shutdown (providers.cpp:134-135).

- Confidence: high
- Kind: race
- Trigger: A provider thread is inside a Yandex picture fetch (get or get\1known, via image://yandex) when the window closes. app::stop sets the stopped flag, but Images::get never reads it, and pictures() still returns the search's images. yandex.stop only waits for busy and fetchers, not provider-thread calls. drop(engine) then waits for the provider thread pool, and the download can run up to the 15-minute body timeout on a stalled link.
- Consequence: Exit is delayed by an in-flight download. A force-kill during that wait skips app::shutdown, so the thumbnail cache's final write-out and the inbox's and catalog's clean close do not run. The thumbnail cache is regenerable, so the user-data risk is small.
- Suggested fix: Have Images::get return an error when stopped is set, and make yandex.stop wait for provider-thread fetches (or hand them a cancellation flag the transfer checks) within STOP\1WAIT before the engine is dropped.
- Locations:
  - `crates/desktop/src/lib.rs:221` worker::stop(); app::stop(); drop(engine); app::shutdown();
  - `crates/app/src/yandex/images.rs:126` pub fn stop(&self) { lock(&self.table).stopped = true; }
  - `crates/app/src/yandex/images.rs:194` pub fn get(&self, id: u64, url: &str) -\1 Result\1Arc\1Fetched\1, SourceError\1 {
  - `crates/app/src/yandex.rs:387` fn pictures(&self) -\1 Option\1Arc\1Images\1\1 {
  - `crates/desktop/cpp/providers.cpp:134` m\1pool.setExpiryTimeout(0);
  - `crates/desktop/cpp/providers.cpp:135` m\1pool.waitForDone();
  - `crates/yandex/src/http.rs:24` const BODY\1TIMEOUT: Duration = Duration::from\1secs(15 \1 60);

## F27: Unsafe Windows API calls (file, volume, directory, console, dialog, message box)

- Kind: boundary
- Entry: crates/platform/src/file.rs:33 CreateFileW, crates/platform/src/dir.rs:163 CreateFileW, crates/platform/src/drive.rs:289 CreateFileW, crates/platform/src/console.rs:25 CreateFileW
- Endpoint: crates/platform/src/file.rs:226 MoveFileExW (rename no replace), crates/platform/src/sqlite.rs:182 CreateDirectoryW and :249 RemoveDirectoryW, crates/platform/src/message.rs:11 MessageBoxW, crates/platform/src/picker.rs:177-255 COM dialog
- Summary: Windows calls take paths from the scanner and the catalog as UTF-16 NUL-terminated buffers (native.rs). Errors and handles are wrapped (OwnedHandle, TempFile). These are the points where user-tree paths reach the OS with reparse and verbatim-prefix semantics.

### SEC-123 [medium/data-integrity] Crash after CreateDirectoryW leaves a plain folder that blocks every later alias — CONFIRMED

- Verdict: CONFIRMED. CreateDirectoryW runs before the reparse point is set (sqlite.rs:182-185). The None arm then refuses the empty folder (sqlite.rs:91-92), and sweep skips it (:104).

- Confidence: high
- Kind: crash-window
- Trigger: The process is killed or power is lost between CreateDirectoryW (crates/platform/src/sqlite.rs:182) and the FSCTL\1SET\1REPARSE\1POINT call inside set\1reparse (sqlite.rs:185), on the first open of a catalog folder whose name has no UTF-8 form. An empty plain folder then sits at the alias name inside sqlite-aliases.
- Consequence: On the next open, read\1mount\1point returns None because the folder is not a reparse point. alias\1under then takes the None arm with symlink\1metadata OK and returns the error 'is not an alias' (sqlite.rs:91-92). sweep skips it for the same reason (sqlite.rs:104), so it never self-heals. openable\1folder fails for that folder, so the catalog cannot be opened there until the user deletes the empty folder by hand. No data is deleted, but the library is unusable until then.
- Suggested fix: Create the junction under a temporary name with its reparse data set, then rename it into place, so the alias name never holds a plain folder. Also make alias\1under and sweep remove an empty plain folder at the link name.
- Locations:
  - `crates/platform/src/sqlite.rs:182` if unsafe { CreateDirectoryW(name.as\1ptr(), std::ptr::null()) } == 0 {
  - `crates/platform/src/sqlite.rs:185` let result = set\1reparse(&name, substitute, print);
  - `crates/platform/src/sqlite.rs:91` None if link.symlink\1metadata().is\1ok() =\1 {
  - `crates/platform/src/sqlite.rs:104` let Some(substitute) = read\1mount\1point(&link)? else { continue };

### SEC-124 [medium/data-integrity] Rebuild leaves no catalog file between moving the old one aside and renaming the replacement in — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-017: no catalog file between move\1aside and the rename (rebuild.rs:94-96).

- Confidence: medium
- Kind: crash-window
- Trigger: Crash or power loss between rebuild.rs:94 (the old catalog and its sidecars move to .replaced) and rebuild.rs:96 (catalog.sqlite.building is renamed to catalog.sqlite). The path is then empty, and the complete replacement sits at catalog.sqlite.building.
- Consequence: On the next start nothing looks for .building. A missing catalog file is treated as no catalog (core.rs:261), so the library opens empty or a fresh catalog is made. The in-memory Rebuilt notice is lost, so the user is not told that the old rows are in .replaced and the replacement is in .building. Nothing is deleted, but recovery is manual. The final std::fs::rename at rebuild.rs:96 also replaces any file another process put at that path in the gap, which the crate's rename\1no\1replace policy exists to prevent.
- Suggested fix: Record the rebuild in progress (a marker or journal) and, on start, promote an existing .building when catalog.sqlite is missing. Make the final step use rename\1no\1replace so a file that appears in the gap is not overwritten.
- Locations:
  - `crates/catalog/src/rebuild.rs:94` let kept = pg\1dbfile::move\1aside(path, ".replaced")
  - `crates/catalog/src/rebuild.rs:96` std::fs::rename(&replacement, path).with\1context(\1\1 {
  - `crates/catalog/src/core.rs:261` let live = if path == Path::new(":memory:") \1\1 !path.exists() { None } else { Some(take\1shared(path)?) };

### SEC-125 [low/data-integrity] move\1aside renames sidecars first, so a failure mid-sequence breaks its documented guarantee — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-024: sidecars move before the main file (remove.rs:11, 30-37).

- Confidence: high
- Kind: partial-write
- Trigger: A failure or crash after remove.rs:34 has moved -wal aside but before the main file has moved. A sharing violation on -shm or -journal after -wal has moved gives the same state.
- Consequence: The main file stays at path with its uncheckpointed WAL frames moved to \1base\1-wal. The catalog then opens at its last checkpoint, and the newest changes exist only in the aside file. The doc comment says the main file is never left at path without the changes its sidecars held, and this window contradicts that. rebuild checkpoints first (rebuild.rs:63), so the practical loss is small, and the thumbnail cache use is disposable.
- Suggested fix: Checkpoint with TRUNCATE before any move so the sidecars are empty and the order no longer matters, or move the main file last and state in the doc what is kept on a partial failure.
- Locations:
  - `crates/dbfile/src/remove.rs:11` const PARTS: \1&str; 4\1 = \1"-wal", "-shm", "-journal", ""\1;
  - `crates/dbfile/src/remove.rs:16` the moved files when they are no longer wanted. The sidecars move first, so the main file is never left at 'path'
  - `crates/dbfile/src/remove.rs:34` rename\1no\1replace(&from, &to)

### SEC-126 [low/data-integrity] finish\1copy records a part-written original as done when its recorded temp is missing — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-072: NotFound on the recorded temp is treated as copied (writeback lib.rs:742).

- Confidence: medium
- Kind: crash-window
- Trigger: A crash during write\1into leaves an original part-written with its temp recorded. Before the next attempt, the temp is removed by another program or tool, so finish\1copy reads NotFound.
- Consequence: lib.rs:742 takes the original as already holding the new content. The write is recorded done (746-747), analysis is redone on the part-written file, and the temp removal (748) does nothing. The photo stays corrupt with no notice, and the whole new content is gone. The comment at 739-741 acknowledges the case, but nothing checks it.
- Suggested fix: Store a hash of the new content with the recorded temp, and before marking the write done verify the original against it. If the temp is gone and the original does not match, fail the write with a notice.
- Locations:
  - `crates/writeback/src/lib.rs:742` Err(e) if e.kind() == std::io::ErrorKind::NotFound =\1 {}
  - `crates/writeback/src/lib.rs:746` let rewrite = Rewrite { before: None, after, metadata\1only: None };

### SEC-127 [medium/data-integrity] Crash during crop's in-place copy leaves a corrupt original and no recovery for the whole cut — CONFIRMED

- Verdict: CONFIRMED. A crash during the in-place cut leaves a part-written original. The whole cut sits in a temp that is not checked on restart (crop lib.rs:300-315, inplace.rs:46-57).

- Confidence: high
- Kind: crash-window
- Trigger: Power loss or a kill during write\1into (inplace.rs:46), while crop's replace (crop/src/lib.rs:310) copies the cut into the original. The original is then part-written, and the whole cut is in the hidden temp .\1name\1.cut.pg-write (inplace.rs:57).
- Consequence: The Incomplete error (crop/src/lib.rs:315) reaches the user only in-process, so after a crash nothing is shown. Nothing on the next start looks at the .cut.pg-write temp, and crop records nothing in the catalog. The catalog still holds the old size and mtime, so the pre-check at crop/src/lib.rs:300 skips the file and no retry happens. The photo is left as a mix of cut and original bytes, and the only whole copy of the cut is a hidden file. A later successful cut of the same file overwrites that temp at inplace.rs:57. Writeback has this recovery (lib.rs:716-753); crop does not.
- Suggested fix: Record the pending cut's temp in the catalog before the copy and finish it at the next start, as writeback's finish\1copy does. At minimum, have startup detect a leftover .cut.pg-write temp and report it.
- Locations:
  - `crates/crop/src/lib.rs:300` if pg\1platform::file::stat(&path).with\1context(\1\1 format!("reading {}", shown(&path)))? != (s.size, s.mtime)
  - `crates/crop/src/lib.rs:310` match replace(&path, &native, &temp, &new, (s.size, s.mtime)) {
  - `crates/crop/src/lib.rs:315` Err(ReplaceError::Incomplete { temp, error }) =\1 bail!(
  - `crates/writeback/src/inplace.rs:46` write\1into(file, bytes).map\1err(incomplete)?;
  - `crates/writeback/src/inplace.rs:57` std::fs::write(temp, bytes).with\1context(\1\1 format!("writing {}", pg\1platform::names::shown(temp)))?;

### SEC-128 [low/security] Reparse check and the later open are separate steps, so a link swapped in between is followed — CONFIRMED

- Verdict: CONFIRMED. The reparse check and the open are separate steps (refusal.rs:48-49, dir.rs:369). Medium confidence.

- Confidence: medium
- Kind: race
- Trigger: A process running as the same user replaces a library folder with a junction to another folder after check\1folder (refusal.rs:48-49) or after the scanner's listing (scan/src/lib.rs:1017), and before the scanner opens the root (dir.rs:369) or writeback opens a file (file.rs:88 through inplace.rs:59).
- Consequence: The scanner walks a tree outside the library and records its files. Writeback can write a rating or a cut into a same-named file in that tree. inplace.rs:56-60 compares only size and mtime, not file identity, so a target with matching size and mtime is written. The attacker needs the user's own write access to the library tree, so the gain is limited to files the user can already write.
- Suggested fix: Open the write and listing handles with FILE\1FLAG\1OPEN\1REPARSE\1POINT, compare the handle's file ID and final path with the catalog's identity, and refuse on a mismatch before writing.
- Locations:
  - `crates/scan/src/refusal.rs:49` if facts.is\1reparse\1point(&handle).map\1err(\1e\1 unreadable(Fact::Folder, &e))? {
  - `crates/platform/src/dir.rs:369` FILE\1FLAG\1BACKUP\1SEMANTICS,
  - `crates/platform/src/file.rs:88` GENERIC\1WRITE \1 FILE\1READ\1ATTRIBUTES,
  - `crates/writeback/src/inplace.rs:60` unchanged(&shown, before, pg\1platform::file::stat\1of(&file)?)?;

### SEC-129 [low/data-integrity] A failed AssignProcessToJobObject is ignored, so ExifTool can outlive the app — CONFIRMED

- Verdict: CONFIRMED. AssignProcessToJobObject's result is ignored (process.rs:131), and so is SetInformationJobObject (:123).

- Confidence: medium
- Kind: leak
- Trigger: AssignProcessToJobObject fails after the child is spawned (process.rs:131), for example because the app's own job does not allow nesting. CreateJobObjectW or SetInformationJobObject can also fail (process.rs:121-124); their results are ignored too.
- Consequence: spawn\1background still returns Ok, so the stay-open ExifTool is not killed when the app ends. The doc comment's promise of ending with the process 'however it ends' is not kept. After a crash or a kill of the app, ExifTool keeps running.
- Suggested fix: Check the BOOL results. On failure, either return an error from spawn\1background or kill the child at once, so that the guarantee holds or the failure is visible.
- Locations:
  - `crates/platform/src/process.rs:123` SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const info).cast(), size);
  - `crates/platform/src/process.rs:131` unsafe { AssignProcessToJobObject(job, child.as\1raw\1handle()) };

## F28: Linux and POSIX system file reads (sysfs, udev, /proc, statfs)

- Kind: untrusted-input
- Entry: crates/platform/src/drive.rs:389-455 (fs::canonicalize, read\1dir and read\1to\1string under /sys/block), crates/platform/src/dir.rs:592 read of /run/udev/data
- Endpoint: crates/platform/src/drive.rs:437 rotational flag and dir.rs:580 ioctl FS\1IOC\1GETFSUUID; memory.rs:27 /proc/meminfo; process.rs:149 /proc/\1pid\1/stat (tests)
- Summary: Device and volume facts (rotational, device number, FS uuid) are read from kernel-provided text files and used to pick scan parallelism and identity. Symlinks under /sys are canonicalized.

### SEC-130 [medium/data-integrity] Volume identity claims are per scan, so two devices sharing a UUID collide across scans — CONFIRMED

- Verdict: CONFIRMED. Each scan starts a fresh Lister (scan lib.rs:381, 362). A shared filesystem UUID across two scans collides, and record\1file returns Foreign (writers.rs:278-279). Medium confidence.

- Confidence: medium
- Kind: wrong-target
- Trigger: Two distinct devices carry the same filesystem UUID: a block-level clone of the photo disk, or two btrfs subvolumes of one filesystem on kernel 6.13+ (where FS\1IOC\1GETFSUUID reports the filesystem UUID for each subvolume device). Root A is scanned, then root B is scanned in a separate scan (or a separate lane). Each scan starts a fresh Lister, so B's first device claims the shared UUID as its volume id. Inode numbers that match across the two devices then produce the same FileIdentity (uuid, ino).
- Consequence: record\1file (writers.rs:278-279) returns Recorded::Foreign when the matching row is present and not under the scanned root, so the root's file is counted as a duplicate and never recorded (silently absent from the catalog). When the other row is missing and the creation times are equal or unknown (writers.rs:281-286), the file is moved onto that row, carrying the other photo's ratings and analysis: a wrong-target rebinding. The same-scan defence (the 'clone keeps its device number' fallback in dir.rs identify) does not cover cross-scan or cross-lane cases, which the doc comment says it does.
- Suggested fix: Keep the device-to-identity claims in one process-wide registry (or in the catalog) rather than in each Lister, so no two live devices get the same volume id. Alternatively key the identity on the subvolume or device as well as the UUID, so that distinct devices never share one.
- Locations:
  - `crates/platform/src/dir.rs:485` fn identify(&mut self, dev: u64, probe: &Path) -\1 u64 {
  - `crates/platform/src/dir.rs:490` Some(id) if !self.claimed.contains\1key(&id) =\1 {
  - `crates/platform/src/dir.rs:456` pub fn new() -\1 Lister {
  - `crates/scan/src/lib.rs:381` walk(catalog, root, ctl, &mut Lister::new())
  - `crates/scan/src/lib.rs:362` let mut lister = pg\1platform::dir::Lister::new();
  - `crates/library/src/lanes.rs:177` let volume = match pg\1scan::root\1volume(rules, root) {
  - `crates/catalog/src/writers.rs:278` if !hit.missing && !hit.in\1root {
  - `crates/catalog/src/writers.rs:279` return Ok(Recorded::Foreign);

### SEC-131 [low/data-integrity] A mounted device missing from sysfs reads as unplugged, not as unknown — CONFIRMED

- Verdict: CONFIRMED. A missing sysfs entry gives NotFound (drive.rs:401, 415). unplugged treats NotFound as unplugged (drive.rs:50), so the root gets no lane (lanes.rs:183).

- Confidence: medium
- Kind: missing-check
- Trigger: A folder whose st\1dev has no /sys/dev/block/\1maj\1:\1min\1 entry (a sandbox or emulated sysfs that omits block devices). facts\1in canonicalizes that entry (drive.rs:415), which fails with NotFound. drive\1of\1dev propagates it (drive.rs:401), unplugged() (drive.rs:50) returns true, and probe\1place (lanes.rs:183) yields Place::Unplugged. The coordinator re-probes every PLACE\1TTL with the same answer.
- Consequence: The root gets no lane (lanes.rs:37-38 and 43-49), so its files are never scanned while the drive is in fact mounted and readable. The fail-closed Unknown answer would have scanned it in one lane. The NotFound conflates 'the path is gone' (fs::metadata at drive.rs:389, the intended case) with 'sysfs has no entry', so the root is silently excluded from the job.
- Suggested fix: Convert a sysfs lookup failure (canonicalize, read\1dev, read of the device's files) to an io::Error that is not NotFound, for example io::Error::other with a message, so that only the missing path from fs::metadata reads as unplugged and the rest fails closed to Unknown.
- Locations:
  - `crates/platform/src/drive.rs:50` error.kind() == std::io::ErrorKind::NotFound \1\1 unplugged\1code(error.raw\1os\1error())
  - `crates/platform/src/drive.rs:415` let real = fs::canonicalize(dir)?;
  - `crates/platform/src/drive.rs:401` facts\1in(&top, 0, &mut disks)?;
  - `crates/library/src/lanes.rs:183` Err(e) =\1 return if unplugged(&e) { Place::Unplugged } else { Place::Unknown },

### SEC-132 [low/data-integrity] One ext volume gets two different identities depending on which source answers — CONFIRMED

- Verdict: CONFIRMED. Volume identity has three sources in order (dir.rs:567). Changes in environment can change which one answers. Medium confidence.

- Confidence: medium
- Kind: lost-update
- Trigger: An ext2/3/4 volume on a kernel before 6.13 (no FS\1IOC\1GETFSUUID) where /run/udev/data has no ID\1FS\1UUID record for the device at scan time (a container without /run/udev, or a boot before udev has written its database). The first scan uses udev's FNV-1a of the UUID (dir.rs:589-602, 627-635). A scan in the other environment, or before the database exists, uses ext\1fsid, the kernel's fold of the same UUID (dir.rs:607-623), which is a different number.
- Consequence: The same files are keyed under a second volume id. record\1file finds no row (writers.rs:270-273) and inserts new unpaired rows. Pairing must rebind them, and if it can't (ambiguous or changed files), duplicate rows remain. The doc comment says the identity is stable and that the id is what the catalog keys on.
- Suggested fix: Derive one identity for a filesystem UUID regardless of the source (hash the UUID bytes the same way in both paths, or use ext\1fsid only when no UUID is available at all), and store the derivation with the volume id so that a mismatch is detected rather than read as a new volume.
- Locations:
  - `crates/platform/src/dir.rs:567` uuid\1from\1kernel(probe).or\1else(\1\1 uuid\1from\1udev(dev)).or\1else(\1\1 ext\1fsid(probe))
  - `crates/platform/src/dir.rs:630` bytes.iter().fold(0xcbf2\19ce4\18422\12325, \1h, b\1 (h ^ u64::from(\1b)).wrapping\1mul(0x0100\10000\101b3)),
  - `crates/platform/src/dir.rs:622` Some(u64::from(halves\10\1.cast\1unsigned()) \1 (u64::from(halves\11\1.cast\1unsigned()) \1\1 32))

## F29: Audio backend library load (PipeWire)

- Kind: boundary
- Entry: crates/platform/src/audio.rs:24 dll::load("libpipewire-0.3.so.0") under unsafe
- Endpoint: crates/platform/src/audio.rs:36-97 unsafe extern C symbol table calls; crates/desktop/src/lib.rs:117-119 std::env::set\1var("QT\1AUDIO\1BACKEND") under unsafe
- Summary: A shared library is loaded by name from the system search path and its symbols called through transmuted function pointers. The desktop app also writes a process environment variable (QT\1AUDIO\1BACKEND) before Qt starts.

### SEC-133 [low/data-integrity] PipeWire init state never released before dlclose — CONFIRMED

- Verdict: CONFIRMED. pw\1init is called with no pw\1deinit before the library is unloaded (audio.rs:53-67).

- Confidence: medium
- Kind: leak
- Trigger: Every Linux startup where QT\1AUDIO\1BACKEND is unset and libpipewire-0.3.so.0 is installed: pipewire\1context\1fails calls pw\1init (audio.rs:67), runs the probe, and returns, dropping the Library (audio.rs:24) so the library is dlclosed. pw\1deinit is never called; it is not even in the symbol table (audio.rs:52-62).
- Consequence: Heap memory that PipeWire's pw\1init allocates is not freed before the library is unmapped, so it stays leaked for the rest of the process. This happens once per process start, and Qt's later load re-runs pw\1init, so the amount is bounded and small. No crash, no data impact.
- Suggested fix: Resolve pw\1deinit alongside the other symbols (optional, so a missing one is not an error) and call it after pw\1thread\1loop\1destroy and before the Library drops, so the probe leaves no PipeWire global state behind.
- Locations:
  - `crates/platform/src/audio.rs:67` init(ptr::null\1mut(), ptr::null\1mut());
  - `crates/platform/src/audio.rs:53` let init: Init = symbol(lib, b"pw\1init\0")?;

## F30: FFmpeg and ExifTool discovery by file path

- Kind: boundary
- Entry: crates/video/src/ffi/load.rs:167-171 candidate list from std::env::current\1exe and option\1env!(PG\1FFMPEG\1BIN); crates/exiftool/src/lib.rs:31 PG\1EXIFTOOL
- Endpoint: crates/video/src/ffi/load.rs:176 dll::load of avformat/avcodec DLLs; crates/exiftool/src/lib.rs:61 Command::new
- Summary: Libraries and helper executables are found from the executable's directory, a build-time bin path, and an environment override. Their resolved location decides what code runs in-process (FFmpeg) or as a child (ExifTool).

### SEC-134 [low/security] Bare exiftool on Windows is searched in the app folder and System32 before PATH — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-080 for Windows: bare exiftool is searched in the app folder first.

- Confidence: high
- Kind: wrong-target
- Trigger: PG\1EXIFTOOL unset, a file named exiftool.exe in the folder of gallery.exe (or in System32), and a genuine ExifTool on PATH. A rating or orientation write runs the app-folder copy, because std's Windows resolve\1exe searches the app directory and the system directories before the parent's PATH.
- Consequence: Whatever exiftool.exe sits beside the app runs as a child with the app's rights, instead of the ExifTool the user put on PATH. The code and its doc comment say the program is found on PATH. Exploiting it needs write access to the app folder, which already allows replacing gallery.exe, so the impact is limited.
- Suggested fix: Resolve a bare name against the PATH entries here (walk env PATH and take the first existing exiftool.exe), then spawn that absolute path. Alternatively document that PG\1EXIFTOOL or an absolute path is required when the app folder is writable.
- Locations:
  - `crates/exiftool/src/lib.rs:31` std::env::var\1os("PG\1EXIFTOOL").unwrap\1or\1else(\1\1 "exiftool".into())
  - `crates/exiftool/src/lib.rs:61` let mut cmd = Command::new(program);

### SEC-135 [low/security] FFmpeg DLL chosen by first existing file per library, with no fallback when it fails to load — CONFIRMED

- Verdict: CONFIRMED. resolve returns the first existing copy (dll.rs:27) and load then fails with no fallback (load.rs:173-176).

- Confidence: medium
- Kind: missing-check
- Trigger: A truncated or wrong avcodec-61.dll (for example from an interrupted copy) sits beside gallery.exe, while the pinned bin folder holds a good copy. Lib::open for avcodec picks the exe-folder file because is\1file() is true, and load() fails.
- Consequence: The whole FFmpeg subsystem fails with a load error, although a usable copy exists in the bin folder. Because each library is resolved on its own, a set could also mix libraries from different folders. The fallback that the candidate list was meant to provide does not happen.
- Suggested fix: Try each candidate folder in order and keep the first whose load succeeds, and choose one folder for the whole FFmpeg set so the libraries come from one build.
- Locations:
  - `crates/platform/src/dll.rs:27` if let Some(path) = candidates.iter().map(\1dir\1 dir.join(file)).find(\1path\1 path.is\1file()) {
  - `crates/video/src/ffi/load.rs:173` let path = pg\1platform::dll::resolve(&file, &candidates)?;
  - `crates/video/src/ffi/load.rs:176` let lib = unsafe { pg\1platform::dll::load(&path) }.map\1err(\1e\1 format!("loading {}: {e}", path.display()))?;

### SEC-136 [low/security] Relative FFMPEG\1DIR bakes a relative PG\1FFMPEG\1BIN that the Windows loader rejects — PARTIAL

- Verdict: PARTIAL. The relative FFMPEG\1DIR baking at build.rs:100 and :89 is confirmed. The loader rejecting a relative path is taken from library docs and was not checked.

- Confidence: medium
- Kind: wrong-target
- Trigger: FFMPEG\1DIR set to a relative path (for example ..\ffmpeg) at build time, with no DLL copies beside the running binary (the test executables). build.rs bakes a relative bin path, and at run time dll::load calls LoadLibraryExW with LOAD\1LIBRARY\1SEARCH\1DLL\1LOAD\1DIR, which needs a fully qualified path (libloading docs, os/windows/mod.rs:542). The existence check in resolve() is also relative to the current directory at run time.
- Consequence: Each DLL from the bin candidate fails to load, so FFmpeg is unavailable and the tests that need it fail. A relative is\1file() check can also match a file in whatever the working directory is, so the candidate depends on the caller's directory. Only reached when FFMPEG\1DIR is relative; the .env.example value is absolute.
- Suggested fix: Absolutize the prefix in build.rs (std::path::absolute, or canonicalize) before it is used for the include, copy and PG\1FFMPEG\1BIN values.
- Locations:
  - `crates/video/build.rs:100` let prefix = match std::env::var\1os("FFMPEG\1DIR") {
  - `crates/video/build.rs:89` println!("cargo:rustc-env=PG\1FFMPEG\1BIN={}", bin.display());
  - `crates/video/src/ffi/load.rs:171` .chain(option\1env!("PG\1FFMPEG\1BIN").map(PathBuf::from))
  - `crates/platform/src/dll.rs:47` WinLibrary::load\1with\1flags(path, LOAD\1LIBRARY\1SEARCH\1DLL\1LOAD\1DIR \1 LOAD\1LIBRARY\1SEARCH\1DEFAULT\1DIRS)

### SEC-137 [low/data-integrity] The argv route has no deadline, so a stalled ExifTool blocks the write worker forever — CONFIRMED

- Verdict: CONFIRMED. oneshot calls output() with no deadline (exiftool lib.rs:200-204), while the kept process has one (:25).

- Confidence: medium
- Kind: crash-window
- Trigger: A file name with a line feed (Unix only, so the argv route is taken) or with trailing whitespace, and ExifTool stalls on that file, for example on a network mount that stops responding. oneshot() calls Command::output(), which has no timeout.
- Consequence: The write job never finishes and the queue worker holds the ExifTool slot indefinitely. That contradicts the documented COMMAND\1TIMEOUT contract (a command past 120 s fails and its process is killed). Nothing is corrupted, because the original is untouched and the temp is not published.
- Suggested fix: Run the argv-route process with spawn, then wait with the same COMMAND\1TIMEOUT deadline (try\1wait polling), killing it on expiry, as exchange() does for the kept process.
- Locations:
  - `crates/exiftool/src/lib.rs:200` fn oneshot(program: &OsStr, args: &\1&OsStr\1) -\1 Result\1(String, String)\1 {
  - `crates/exiftool/src/lib.rs:204` let out = cmd.output().with\1context(\1\1 format!("starting {} (ExifTool)", program.to\1string\1lossy()))?;
  - `crates/exiftool/src/lib.rs:25` const COMMAND\1TIMEOUT: Duration = Duration::from\1secs(120);

### SEC-138 [low/security] Build machine's absolute FFmpeg bin path is compiled into the executable — CONFIRMED

- Verdict: CONFIRMED. rustc-env embeds the absolute bin path (video build.rs:89), read back by option\1env (load.rs:171).

- Confidence: medium
- Kind: leak
- Trigger: A Windows build that sets FFMPEG\1DIR, or uses the default .deps/ffmpeg under the repo. build.rs embeds the absolute bin path with rustc-env, and option\1env! puts it in the binary's string data, so a shipped gallery.exe carries the developer's folder path, which can include the Windows user name.
- Consequence: The path of the developer's machine is revealed to anyone who opens the shipped binary. It is a path only, not a secret, and it also sends the loader to that folder when the executable's own folder lacks the DLLs.
- Suggested fix: Keep the baked path only for test and development builds (for example behind a cfg or env check), or bake a path relative to the executable so no absolute developer path is stored in shipped binaries.
- Locations:
  - `crates/video/build.rs:89` println!("cargo:rustc-env=PG\1FFMPEG\1BIN={}", bin.display());
  - `crates/video/src/ffi/load.rs:171` .chain(option\1env!("PG\1FFMPEG\1BIN").map(PathBuf::from))

## F31: Build scripts reading environment and resource folders

- Kind: boundary
- Entry: crates/video/build.rs:100 FFMPEG\1DIR, :121 CARGO\1MANIFEST\1DIR; crates/desktop/build.rs:11 read\1dir("resources/mascots"), :128 PG\1DESKTOP\1CONTENT\1KEY
- Endpoint: crates/video/build.rs:133 Command::new("pkg-config"), :156 OUT\1DIR writes, generated bindings; crates/desktop/build.rs cargo rustc-env output
- Summary: Build time reads env vars and a folder, runs pkg-config, and writes generated code into OUT\1DIR. Not run at user runtime, but the output (linked FFmpeg location) feeds F30.

### SEC-139 [low/data-integrity] FFmpeg major not checked against the version Qt expects — PARTIAL

- Verdict: PARTIAL. build.rs:109 checks only that the header exists, so no major-version check is made. The Qt Multimedia consequences were not checked.

- Confidence: medium
- Kind: missing-check
- Trigger: A developer sets FFMPEG\1DIR (or .env) to an FFmpeg build that is not 9.0, e.g. a 7.x or 8.x shared build. build.rs:109 only checks that include/libavcodec/avcodec.h exists, then generates bindings and copies that build's DLLs beside gallery.exe (build.rs:197-232).
- Consequence: The build succeeds and the copied DLLs carry the other major's names. Qt Multimedia, which build.rs:6-8 says loads the same DLLs and must match major, may load a different major or fall back to a copy elsewhere. Failure appears only at video playback time, not at build time. The doc and error text say 9.0, but nothing enforces it.
- Suggested fix: Read LIBAVCODEC\1VERSION\1MAJOR (and the other four) from the generated constants and fail the build unless they match the major 9.0 pin Qt expects, or record the expected major in one place shared with the Justfile pin.
- Locations:
  - `crates/video/build.rs:109` if !include.join("libavcodec").join("avcodec.h").is\1file() {
  - `crates/video/build.rs:207` let entries = std::fs::read\1dir(bin).unwrap\1or\1else(\1e\1 panic!("{}: {e}", bin.display()));

### SEC-140 [low/data-integrity] DLL beside gallery.exe overwritten in place, can be left truncated — CONFIRMED

- Verdict: CONFIRMED. DLLs are copied in place (build.rs:228-230). A killed copy can leave a truncated file.

- Confidence: medium
- Kind: crash-window
- Trigger: The build is killed (Ctrl+C, power loss) during std::fs::copy at build.rs:229 after the destination DLL was truncated, while a previously built gallery.exe is still in the profile folder.
- Consequence: gallery.exe fails to load FFmpeg or Qt Multimedia until the build runs again. The next build self-heals because same() compares length and mtime (build.rs:224-227), and cargo reruns the script after a killed run, but there is a window where the shipped DLL is corrupt.
- Suggested fix: Copy to a temporary name in the profile folder, then std::fs::rename over the destination, so the DLL beside the app is either the old or the new file.
- Locations:
  - `crates/video/build.rs:228` if !same(dll, &dest) {
  - `crates/video/build.rs:229` std::fs::copy(dll, &dest)

### SEC-141 [low/data-integrity] setup deletes .deps/ffmpeg before the new build is moved in — CONFIRMED

- Verdict: CONFIRMED. rmtree of dest runs before move (setup\1ffmpeg.py:133-136). A crash between them leaves no FFmpeg.

- Confidence: medium
- Kind: crash-window
- Trigger: Either a crash between the rmtree and the move, or a DLL under .deps/ffmpeg/bin held open by a running test executable (test binaries load from PG\1FFMPEG\1BIN, load.rs:171, and Windows cannot delete a loaded DLL), so rmtree fails partway through.
- Consequence: A crash leaves no FFmpeg at the destination; the next run re-extracts from the cached, hash-checked zip, so nothing is lost. A partial rmtree leaves a header with a stale or missing pin, and build.rs:86-91 then fails on the DLL count assertion (build.rs:215) until setup is rerun. The cache is recoverable; the build is blocked meanwhile.
- Suggested fix: Move the existing folder aside to a sibling name, move the new build in, then delete the old copy, so a failure never leaves the destination half-removed.
- Locations:
  - `tools/setup_ffmpeg.py:136` if dest.exists():
  - `tools/setup_ffmpeg.py:137` shutil.rmtree(dest)

### SEC-142 [low/data-integrity] Mascot folder read with no filter; any entry is embedded — CONFIRMED

- Verdict: CONFIRMED. Every entry in resources/mascots is added (desktop build.rs:11-15), with no filter. Latent.

- Confidence: high
- Kind: missing-check
- Trigger: A non-PNG file or a subfolder appears in crates/desktop/resources/mascots, for example a Windows Thumbs.db created when Explorer shows the folder as thumbnails. The current folder holds only 01.png to 40.png, so this is latent.
- Consequence: The stray file is added to the qrc and embedded in gallery.exe, or a subfolder makes rcc fail. Build output then depends on the local working tree, not on what is committed.
- Suggested fix: Keep only regular files whose names end in .png (with an explicit 01.png to 40.png check if the set is fixed), and skip the rest.
- Locations:
  - `crates/desktop/build.rs:11` let entries = std::fs::read\1dir("resources/mascots").map\1err(\1e\1 format!("reading resources/mascots: {e}"))?;
  - `crates/desktop/build.rs:15` .map(\1entry\1 format!("resources/mascots/{}", entry.file\1name().to\1string\1lossy()))

### SEC-143 [low/security] QT\1DIR interpolated unquoted into a cmd.exe recipe line — PARTIAL

- Verdict: PARTIAL. The unquoted {{qt}} interpolation at Justfile:59 is confirmed. The claim that the recipe runs under cmd.exe (Justfile:11) was not checked.

- Confidence: high
- Kind: injection
- Trigger: QT\1DIR in .env or the environment contains a space or a cmd metacharacter (such as &), for example C:/Program Files/Qt/6.12.0/msvc2022\164, and the deploy recipe runs under cmd.exe (Justfile:11).
- Consequence: With spaces, cmd splits the path and windeployqt fails loudly. With &, cmd runs the text after it as a separate command. The value comes from the developer's own .env, so exposure is limited to that developer's machine.
- Suggested fix: Quote the windeployqt path in the recipe ("{{ qt }}/bin/windeployqt.exe"), which the Justfile's own comment at line 10 asks for, or call it through the Python tools that already receive the path as a list.
- Locations:
  - `Justfile:59` {{ qt }}/bin/windeployqt.exe --release {{ if pdb == "pdb" { "--pdb" } else { "" } }} --qmldir crates/desktop/qml --no-translations --no-ffmpeg --include-plugins qoffscreen target/{{ profile }}/gallery

## F32: Python tool: ExifTool download, verify and extract

- Kind: network
- Entry: tools/install\1exiftool.py:24 urllib.request.urlopen(url) (URL and SHA-256 from the Justfile pin)
- Endpoint: tools/install\1exiftool.py:30-31 tarfile extractall(filter=data) into a work folder, :36 shutil.rmtree(prefix/lib), :39 subprocess.run(prefix/exiftool -ver)
- Summary: Downloads an archive, checks its digest before use, extracts with the data filter, and deletes a prefix subtree. Runs a freshly installed binary to confirm the version.

### SEC-144 [low/security] Archive body read whole into memory with no size cap or total deadline — CONFIRMED

- Verdict: CONFIRMED. The archive is read whole with only a per-read timeout (install\1exiftool.py:24-25).

- Confidence: high
- Kind: unbounded-alloc
- Trigger: The pinned SourceForge URL (or a redirect target urllib follows) returns an unbounded or very large body, or trickles bytes just under the 120 s per-read timeout, so the hash check at install\1exiftool.py:27 never runs.
- Consequence: The process allocates memory without limit or hangs for an unbounded time inside the Docker RUN or the CI step. Integrity is not affected, because nothing is extracted before the digest matches.
- Suggested fix: Stream the body in chunks into a capped buffer (the archive is about 30 MB, so a cap of a few hundred MB is enough), and enforce a total wall-clock deadline. Keep the SHA-256 check before extraction.
- Locations:
  - `tools/install_exiftool.py:24` with urllib.request.urlopen(url, timeout=120) as response: # noqa: S310 (the URL is the Justfile's pin)
  - `tools/install_exiftool.py:25` archive = response.read()

### SEC-145 [low/data-integrity] Install replaces the live program and lib in place before verifying, so a kill leaves a mixed install — CONFIRMED

- Verdict: CONFIRMED. copy2 (install\1exiftool.py:34) and copytree (:37) replace the live install before the -ver check (:39).

- Confidence: high
- Kind: crash-window
- Trigger: The process is killed, loses power, or hits disk-full between install\1exiftool.py:34 (copy2 over prefix/exiftool) and the end of copytree at :37, or the -ver check at :39 fails after the replacement.
- Consequence: The prefix holds the new exiftool with the old or missing lib, and any previously working install is gone. The run exits non-zero, so the Docker RUN or CI step fails and no artifact is produced. The next run repairs it. The gap is in the failure path, not a silent success.
- Suggested fix: Extract and copy into a staging folder beside the prefix, run -ver on the staged copy, then swap it into place with os.replace or rename, so the live install is only ever replaced by a verified one.
- Locations:
  - `tools/install_exiftool.py:34` shutil.copy2(source / "exiftool", prefix / "exiftool")
  - `tools/install_exiftool.py:36` shutil.rmtree(prefix / "lib", ignore\1errors=True)
  - `tools/install_exiftool.py:37` shutil.copytree(source / "lib", prefix / "lib")
  - `tools/install_exiftool.py:39` reported = subprocess.run(\1str(prefix / "exiftool"), "-ver"\1, capture\1output=True, text=True, check=True).stdout.strip()

### SEC-146 [low/data-integrity] rmtree of prefix/lib deletes any lib folder under --prefix, not just ExifTool's — CONFIRMED

- Verdict: CONFIRMED. rmtree of prefix/lib runs with ignore\1errors (install\1exiftool.py:36) on whatever --prefix names (:50).

- Confidence: high
- Kind: wrong-target
- Trigger: --prefix is a directory that already holds an unrelated lib tree, for example --prefix /usr/local. Today's callers pass /opt/exiftool and $RUNNER\1TEMP/exiftool, so this needs a new caller or a misconfigured one.
- Consequence: The unrelated lib tree is deleted before the copy. ignore\1errors hides the delete failure, and copytree then fails on the leftover directory, so the damage is partial.
- Suggested fix: Install the lib into a dedicated subfolder of the prefix, or refuse to delete a lib that lacks a marker this script wrote. Drop ignore\1errors so a failed delete stops the run.
- Locations:
  - `tools/install_exiftool.py:36` shutil.rmtree(prefix / "lib", ignore\1errors=True)
  - `tools/install_exiftool.py:50` parser.add\1argument("--prefix", type=Path, default=Path("/opt/exiftool"))

## F33: Python tool: FFmpeg archive download and zip extraction

- Kind: network
- Entry: tools/setup\1ffmpeg.py:46 urllib.request.urlopen(url) into a .partial file
- Endpoint: tools/setup\1ffmpeg.py:107-116 SHA-256 check then unlink on mismatch; :121-131 zipfile extractall(extract); :134 shutil.rmtree(dest)
- Summary: Fetches the pinned FFmpeg zip, verifies SHA-256, then extracts it with zipfile.extractall (no member-name filter in this code) into the dependency folder, replacing the previous install with rmtree.

### SEC-147 [medium/data-integrity] Unchecked rmtree of the old install can leave .deps/ffmpeg half-deleted on Windows — CONFIRMED

- Verdict: CONFIRMED. rmtree of dest (setup\1ffmpeg.py:133-134) is unchecked, so a PermissionError escapes.

- Confidence: medium
- Kind: missing-check
- Trigger: 'just setup' runs with a changed pin while a process (the app, or a test run) has a DLL from .deps/ffmpeg/bin loaded. pg-video loads its DLLs from this folder by full path, so Windows refuses to delete that file. At line 134, shutil.rmtree(dest) raises PermissionError.
- Consequence: The old build is partly deleted, and the exception is an uncaught traceback, not a clean failure. The .pin may be removed or left stale, and the header may be gone. The new build is staged in .deps/extract and is not moved in, so the dependency is unusable until the lock is released and setup is rerun. Recovery is automatic on rerun, so the damage is limited to a failed setup and a broken folder.
- Suggested fix: Catch OSError around the rmtree, report that the build is in use and name the folder, and leave the old install intact. Alternatively, move the old folder aside with a rename and delete it only after the new one is in place.
- Locations:
  - `tools/setup_ffmpeg.py:133` if dest.exists():
  - `tools/setup_ffmpeg.py:134` shutil.rmtree(dest)

### SEC-148 [low/data-integrity] Crash between deleting the old install and moving the new one leaves no install — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-141: no install between rmtree (line 134) and move (line 136).

- Confidence: high
- Kind: crash-window
- Trigger: The process is killed or loses power after shutil.rmtree(dest) at line 134 and before shutil.move at line 136.
- Consequence: The previous build is gone and the new one is not in dest. Rerunning recovers, because is\1installed is False and the verified zip is reused, but the window is a gap with no install. The extracted build in .deps/extract is also discarded on rerun.
- Suggested fix: Rename dest to a sibling backup first, then move the staged tree in, then delete the backup. Restore the backup if the move fails.
- Locations:
  - `tools/setup_ffmpeg.py:134` shutil.rmtree(dest)
  - `tools/setup_ffmpeg.py:136` shutil.move(str(extract / top), str(dest))

### SEC-149 [low/data-integrity] Ignored rmtree errors let stale files from an earlier build reach dest with a fresh pin — CONFIRMED

- Verdict: CONFIRMED. rmtree of extract ignores errors (setup\1ffmpeg.py:121), so stale files can be moved in (:136).

- Confidence: medium
- Kind: missing-check
- Trigger: Removing .deps/extract fails silently: an antivirus or indexer holds a file, or the earlier cleanup at line 137 failed. The next run's extractall overwrites only the names the new archive has. Files from the old archive survive in .deps/extract.
- Consequence: shutil.move moves the whole folder, stale files included, into dest. Line 141 then writes .pin, which says the tree is exactly the pinned archive. The pin claim is false for that tree. The effect is limited, since the header check passes, but dest can contain versioned DLLs that the pinned build does not ship.
- Suggested fix: Extract into a fresh unique directory (tempfile.mkdtemp under .deps) and raise if its removal fails, so stale files can never be moved in.
- Locations:
  - `tools/setup_ffmpeg.py:121` shutil.rmtree(extract, ignore\1errors=True)
  - `tools/setup_ffmpeg.py:136` shutil.move(str(extract / top), str(dest))
  - `tools/setup_ffmpeg.py:141` (dest / PIN\1FILE).write\1text(wanted + "\n", encoding="utf-8")

### SEC-150 [low/data-integrity] Extraction on POSIX fails after dest is deleted when an archive uses backslash names — CONFIRMED

- Verdict: CONFIRMED. Latent: entry\1parts normalises backslashes (setup\1ffmpeg.py:71), but extractall on POSIX keeps raw names (:131). The pin prevents it.

- Confidence: medium
- Kind: wrong-target
- Trigger: setup runs on Linux with an archive whose entries use backslashes, such as 'ffmpeg\\include\\a.h'. Only a non-pinned archive could do this, so the pin prevents it. unsafe\1name and top\1folder normalise '\\' to '/', so top is 'ffmpeg'. zipfile on POSIX keeps the raw name and writes one file named 'ffmpeg\\include\\a.h', so extract/'ffmpeg' does not exist.
- Consequence: shutil.move at line 136 raises FileNotFoundError uncaught, after line 134 has already deleted dest. The old install is lost and no pin is written. The flow is documented as Windows-only and the archive is hash-pinned, so this is latent.
- Suggested fix: Refuse any entry name containing a backslash before extraction, so the normalised check and what zipfile writes always agree.
- Locations:
  - `tools/setup_ffmpeg.py:71` return PurePosixPath(name.replace("\\", "/")).parts
  - `tools/setup_ffmpeg.py:131` archive.extractall(extract)
  - `tools/setup_ffmpeg.py:136` shutil.move(str(extract / top), str(dest))

### SEC-151 [low/data-integrity] Two concurrent setup runs in one checkout share the .partial file and the extract folder — CONFIRMED

- Verdict: CONFIRMED. Both runs share the .partial name (setup\1ffmpeg.py:40) and the extract folder (:121).

- Confidence: medium
- Kind: race
- Trigger: Two 'just setup' processes run at once in the same checkout, for example two terminals or an IDE plus a shell. Both write the same \1zip\1.partial at line 40 and both clear and extract into .deps/extract at line 121.
- Consequence: One run's rmtree of .deps/extract can delete the other's staged files mid-extraction, which fails the move with an uncaught error. Interleaved writes to .partial can produce a wrong file that the hash check then rejects and deletes. The final state is retried on the next run. Not a silent bad install, since the hash and header checks still gate it.
- Suggested fix: Take a lock file under .deps before setup runs, or use a per-process unique name for the .partial and the extract folders.
- Locations:
  - `tools/setup_ffmpeg.py:40` partial = target.with\1name(target.name + ".partial")
  - `tools/setup_ffmpeg.py:46` with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as file: # noqa: S310
  - `tools/setup_ffmpeg.py:121` shutil.rmtree(extract, ignore\1errors=True)

### SEC-152 [low/data-integrity] An operator-given --destination is deleted with rmtree and not limited to .deps — CONFIRMED

- Verdict: CONFIRMED. --destination is resolved (:95) and rmtree'd (:134). Latent through manual misuse.

- Confidence: high
- Kind: wrong-target
- Trigger: setup\1ffmpeg.py is run by hand with --destination '.' or an empty string, so dest resolves to the repository root. The Justfile always passes .deps/ffmpeg, so this is latent and not reachable in normal use.
- Consequence: With a fresh download and the header missing at the root, line 134 runs shutil.rmtree on the repository root and deletes the working tree.
- Suggested fix: Require dest to be a strict subfolder of REPO / '.deps' (is\1relative\1to check) before any deletion, and exit otherwise.
- Locations:
  - `tools/setup_ffmpeg.py:95` dest = (REPO / args.destination).resolve()
  - `tools/setup_ffmpeg.py:134` shutil.rmtree(dest)

## F34: Python tools: source download and build outputs (Qt, FFmpeg, libjpeg-turbo)

- Kind: boundary
- Entry: tools/build\1static\1qt.py:82 urlopen(SOURCE\1URL) and :102-103 zipfile extractall(work); tools/build\1static\1ffmpeg.py:63 and :147 subprocess.run(command); tools/build\1jpeg\1turbo.py:39 subprocess.run
- Endpoint: tools/build\1static\1qt.py:201-204 rmtree build and prefix; tools/build\1static\1ffmpeg.py:244-246 rmtree prefix and staged; tools/build\1jpeg\1turbo.py:112 rmtree folder; manifest json writes
- Summary: Build tools fetch pinned sources, apply patches, run compilers and shell commands by argument list, and delete build or prefix trees. Paths are derived from repository layout and environment.

### SEC-153 [medium/data-integrity] FFmpeg promotion nests the new install inside a stale prefix — CONFIRMED

- Verdict: CONFIRMED. rmtree of prefix is ignore\1errors (build\1static\1ffmpeg.py:244), and move (:245) then nests inside the stale prefix.

- Confidence: high
- Kind: wrong-target
- Trigger: A file under work\prefix is locked (antivirus or indexer) when build\1static\1ffmpeg.py:244 runs rmtree(prefix, ignore\1errors=True). rmtree leaves the prefix directory in place, and the run continues.
- Consequence: shutil.move at line 245 moves staged\ffbuild\prefix into the existing prefix as prefix\prefix. The CRT and link checks (lines 249-271) then test the stale prefix\lib from the previous build. The run exits successfully, and the manifest (line 298) records the stale library hashes. The new build sits unused at prefix\prefix, and build\1static\1qt.py check\1prefixes passes. Changed configure flags could ship silently under an old build.
- Suggested fix: Drop ignore\1errors so a failed delete stops the run, or rename the old prefix aside and rename the staged one into place, failing if the target exists. Run the CRT and link checks on the staged folder before promotion.
- Locations:
  - `tools/build_static_ffmpeg.py:244` shutil.rmtree(prefix, ignore\1errors=True)
  - `tools/build_static_ffmpeg.py:245` shutil.move(str(staged\1prefix), str(prefix))
  - `tools/build_static_ffmpeg.py:249` libs = sorted((prefix / "lib").glob("\1.lib"), key=lambda path: path.name)

### SEC-154 [medium/data-integrity] Failed verification leaves unverified prefixes at the consumer path — CONFIRMED

- Verdict: CONFIRMED. The move (build\1static\1ffmpeg.py:245) happens before the CRT and link checks (:254, :271). build\1static\1qt.py:118 checks only two files.

- Confidence: high
- Kind: missing-check
- Trigger: The FFmpeg CRT check finds LIBCMT (build\1static\1ffmpeg.py:254), the link check fails (line 268 or 271), or the Qt check\1install finds DLLs or a wrong qmake version (build\1static\1qt.py:150-153) after the install has already written into prefix.
- Consequence: The script exits with an error, but the failed outputs stay in work\prefix. No manifest is written. build\1static\1qt.py check\1prefixes (lines 118-119) tests only that avcodec.h and avcodec.lib exist, so a later Qt build accepts the bad FFmpeg prefix. check\1qt.py checks only the version, so QT\1DIR can point at a failed Qt prefix.
- Suggested fix: Build and verify into a staging folder, and promote the prefix only after all checks pass. Have check\1prefixes and check\1qt.py require a pass marker or the manifest, not only file presence.
- Locations:
  - `tools/build_static_ffmpeg.py:245` shutil.move(str(staged\1prefix), str(prefix))
  - `tools/build_static_ffmpeg.py:255` sys.exit("a library references LIBCMT (static CRT): see the manifest")
  - `tools/build_static_ffmpeg.py:271` sys.exit(f"static-check output lacks: {expected}")
  - `tools/build_static_qt.py:118` if not (ffmpeg / "include" / "libavcodec" / "avcodec.h").is\1file() or not (ffmpeg / "lib" / "avcodec.lib").is\1file():
  - `tools/build_static_qt.py:150` sys.exit(f"the install has DLLs, so it is not a static build: {', '.join(dlls)}")

### SEC-155 [medium/data-integrity] Working Qt and jpeg prefixes are deleted before the rebuild — CONFIRMED

- Verdict: CONFIRMED. build\1static\1qt.py:204 deletes the prefix before configure. jpeg rmtree (build\1jpeg\1turbo.py:112) does the same.

- Confidence: high
- Kind: crash-window
- Trigger: A Qt re-run is killed or its configure fails after build\1static\1qt.py:204 deletes prefix (QT\1DIR). Or a locked file survives the ignore\1errors rmtree at line 204 or jpeg line 112.
- Consequence: A failed or interrupted run leaves no working Qt kit until a roughly one-hour rebuild completes, and the jpeg prefix is gone until its rebuild. With a locked file, stale files survive: cmake install (line 211) overwrites only matching names, and check\1install plus the manifest's library list (lines 213-216) record the mix of old and new libraries as the new build.
- Suggested fix: Install into a fresh staging prefix and swap it in after the checks, raising errors on delete instead of ignoring them. Keep the last working prefix until the new one passes.
- Locations:
  - `tools/build_static_qt.py:204` shutil.rmtree(prefix, ignore\1errors=True)
  - `tools/build_static_qt.py:214` \1\1(prefix / "lib").glob("\1.lib"), \1(prefix / "plugins").rglob("\1.lib")\1
  - `tools/build_jpeg_turbo.py:112` shutil.rmtree(folder, ignore\1errors=True)

### SEC-156 [low/data-integrity] Manifests are written in place, not atomically — CONFIRMED

- Verdict: CONFIRMED. Manifests are written with write\1text in place (build\1static\1qt.py:229, ffmpeg :298, jpeg :145).

- Confidence: high
- Kind: partial-write
- Trigger: A crash, kill or power loss during manifest write\1text, after the build or checks succeed.
- Consequence: A truncated or empty manifest.json is left, and the record of the pinned inputs and output hashes is lost for that run. Nothing in the repository reads these manifests, so the effect is limited to the record.
- Suggested fix: Write to manifest.json.part and os.replace it into place, so a reader sees the old or the new file, never a partial one.
- Locations:
  - `tools/build_static_qt.py:229` (work / "manifest.json").write\1text(json.dumps(manifest, indent=2), encoding="utf-8")
  - `tools/build_static_ffmpeg.py:298` (work / "manifest.json").write\1text(json.dumps(manifest, indent=2), encoding="utf-8")
  - `tools/build_jpeg_turbo.py:145` (work / "manifest.json").write\1text(json.dumps(manifest, indent=2), encoding="utf-8")

### SEC-157 [low/data-integrity] Completed-extraction marker trusts zip member order — CONFIRMED

- Verdict: CONFIRMED. Extraction is skipped when configure.bat exists (build\1static\1qt.py:99), and extractall order is not guaranteed (:103).

- Confidence: medium
- Kind: crash-window
- Trigger: The process is killed during zipped.extractall at build\1static\1qt.py:103 after configure.bat has been written. The next run finds configure.bat and skips extraction, because the comment at line 97 assumes configure.bat is written last.
- Consequence: configure runs on an incomplete Qt source tree. In most cases the configure or build fails loudly, but the tree is never re-checked against the archive.
- Suggested fix: Extract into a temporary folder and rename it to the source name on success, or write a completion marker after extractall returns.
- Locations:
  - `tools/build_static_qt.py:99` if (source / "configure.bat").is\1file():
  - `tools/build_static_qt.py:103` zipped.extractall(work)

### SEC-158 [low/security] MSYS2 package pins are advisory and installed with partial sync — CONFIRMED

- Verdict: CONFIRMED. pacman -Sy then -S (build\1static\1ffmpeg.py:127-128). A version mismatch is only a warning (:130-133).

- Confidence: high
- Kind: missing-check
- Trigger: make or diffutils is missing from the MSYS2 install. pacman -Sy then pacman -S installs the repository's current versions, and check\1msys\1packages reports a mismatch only on stderr.
- Consequence: Builds run with package versions other than the pins, which are the stated reproducibility inputs. The sync-without-upgrade sequence can leave MSYS2 partially upgraded. The manifest records whichever versions were installed.
- Suggested fix: Install the pinned versions exactly, or fail when any pinned package differs. Run a full pacman -Syu before installing packages.
- Locations:
  - `tools/build_static_ffmpeg.py:127` checked("pacman -Sy", \1str(pacman), "-Sy", "--noconfirm"\1, logs / "pacman-sync.log")
  - `tools/build_static_ffmpeg.py:130` different = \1name for name in MSYS\1PACKAGES if installed.get(name) != MSYS\1PACKAGES\1name\1\1

### SEC-159 [low/data-integrity] dumpbin failure makes the DLL-dependency check pass vacuously — CONFIRMED

- Verdict: CONFIRMED. dumpbin's return code is not checked (build\1static\1ffmpeg.py:272-275), so an empty result passes.

- Confidence: medium
- Kind: missing-check
- Trigger: dumpbin.exe exits nonzero, or its output does not match the regex at line 275. dependents and non\1system then come back empty.
- Consequence: The run prints no warning, and the manifest records that static-check.exe has no non-system DLL dependencies, although the check never ran.
- Suggested fix: Check dumped.returncode and fail if no dependency lines were parsed.
- Locations:
  - `tools/build_static_ffmpeg.py:272` dumped = subprocess.run(
  - `tools/build_static_ffmpeg.py:275` dependents = \1line.strip() for line in dumped.stdout.splitlines() if re.match(r"^\s+\S+\.dll$", line, re.IGNORECASE)\1

### SEC-160 [low/security] Operator-supplied paths reach cmd.exe and sh unquoted — PARTIAL

- Verdict: PARTIAL. Only spaces are rejected (build\1static\1qt.py:185). Operator-supplied paths with & or quotes reach cmd and sh unquoted. The cmd-splitting consequence was not traced.

- Confidence: medium
- Kind: injection
- Trigger: A work, jpeg or ffmpeg folder with & ^ % ! or parentheses (for example D:\build&tools\qt), or with a single quote for the FFmpeg shell scripts. Only spaces are rejected (build\1static\1qt.py:184-186).
- Consequence: cmd.exe reads & as a command separator in configure.bat's argument list (build\1static\1qt.py:143), so the configure line is cut short or the rest runs as a separate command. The FFmpeg configure.sh and build.sh single-quoted paths (build\1static\1ffmpeg.py:217, 234) break on a quote. The paths come from the operator, so this is not remotely reachable.
- Suggested fix: Reject cmd.exe and sh metacharacters in the same check that rejects spaces, or pass the paths through environment variables so they are not parsed.
- Locations:
  - `tools/build_static_qt.py:143` checked("configure", \1"cmd.exe", "/d", "/c", str(source / "configure.bat"), \1flags\1, log, cwd=build)
  - `tools/build_static_qt.py:185` if " " in str(folder):
  - `tools/build_static_ffmpeg.py:217` f"cd '{posix(src)}'\n"

### SEC-161 [low/data-integrity] No lock on the work folder for concurrent runs — CONFIRMED

- Verdict: CONFIRMED. No lock on the work folder. The rmtree of build (build\1static\1qt.py:202) and the shared .part (:79) collide.

- Confidence: medium
- Kind: race
- Trigger: Two runs of build\1static\1qt.py, build\1static\1ffmpeg.py or build\1jpeg\1turbo.py start with the same --work folder, for example from two terminals.
- Consequence: One run's rmtree of build or prefix (build\1static\1qt.py:202-204, build\1jpeg\1turbo.py:111-112) deletes the other run's tree during its build. Both runs write the same .part download (line 79), and both write configure.sh or build.sh to the same path.
- Suggested fix: Create a lock file in the work folder with exclusive create (O\1EXCL) at the start of each run, and remove it on exit, failing if it exists.
- Locations:
  - `tools/build_static_qt.py:202` shutil.rmtree(build, ignore\1errors=True)
  - `tools/build_static_qt.py:79` partial = archive.with\1name(archive.name + ".part")

## F35: PowerShell scripts: deletes, moves and writes under repo or chosen folders

- Kind: persistent-write
- Entry: tools/build-jpeg-turbo.ps1:75 Remove-Item -Recurse -Force on $dir; tools/build-static-ffmpeg.ps1:170 Move-Item to $Prefix; tools/qmltest.ps1:24 Remove-Item -Recurse -Force $staged; tools/profiling/pairing/keep.ps1:36 and :128 Remove-Item -Recurse -Force $base
- Endpoint: tools/build-jpeg-turbo.ps1:111 and tools/build-static-ffmpeg.ps1:215 Set-Content manifest.json; tools/profiling/pairing/keep.ps1:42 Set-Content; keep.ps1:55-56 python -I subprocess
- Summary: Recursive deletes and moves are aimed at folders passed as parameters or derived from the repo root. docs/probes/win\1probe.ps1:1 takes -Dir (default target\pg-test\probe) and deletes its contents with Remove-Item -Force (lines 109, 141, 167). The probe scripts use CreateFileW through Add-Type P/Invoke.

### SEC-162 [medium/data-integrity] Failed rmdir before Move-Item nests the new FFmpeg prefix in the stale one — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-153 in PowerShell: the rmdir result is ignored (build-static-ffmpeg.ps1:169) before Move-Item (:170).

- Confidence: high
- Kind: partial-write
- Trigger: A file under Work\prefix is locked (another process or an antivirus scan holds a .lib or .exe) when build-static-ffmpeg.ps1 runs line 169. rmdir /s /q deletes what it can, exits nonzero, and the exit code is ignored. $Prefix still exists, so Move-Item at line 170 moves the staged prefix inside it.
- Consequence: The new install lands at Work\prefix\prefix and the stale libraries remain at Work\prefix\lib. The CRT check (line 174), the link check (lines 187-188, /LIBPATH:$Prefix\lib) and the manifest hashes (line 215) all use the stale libraries. The script still reports 'done' with a manifest that does not describe the build.
- Suggested fix: Check the rmdir exit code and throw if it fails, or use Remove-Item -Recurse -Force -ErrorAction Stop, and assert that $Prefix no longer exists before the move. The python twin (tools/build\1static\1ffmpeg.py:244-245) has the same nesting shape and needs the same guard.
- Locations:
  - `tools/build-static-ffmpeg.ps1:169` if (Test-Path $Prefix) { & cmd.exe /d /c "rmdir /s /q '"$Prefix'"" }
  - `tools/build-static-ffmpeg.ps1:170` Move-Item -LiteralPath $stagedPrefix -Destination $Prefix

### SEC-163 [medium/data-integrity] Probe scripts resolve their test folder one level above the repo — CONFIRMED

- Verdict: CONFIRMED. Join-Path from docs/probes with ..\..\.. resolves to D:\GitHub\target, outside the checkout (win\1probe.ps1:1, v1\1symlink.ps1:3, v1b\1junction.ps1:4).

- Confidence: high
- Kind: wrong-target
- Trigger: Running docs/probes/win\1probe.ps1 with its default -Dir. Join-Path $PSScriptRoot '..\..\..\target\pg-test\probe' from docs/probes resolves to D:\GitHub\target\pg-test\probe, outside the checkout. v1\1symlink.ps1 and v1b\1junction.ps1 create D:\GitHub\target\pg-test there.
- Consequence: Each run writes test files and deletes the top-level files of that folder, which lies in the parent of the repo (a sibling of the checkout's target). Any unrelated content in D:\GitHub\target\pg-test\probe is deleted with no warning. The path also escapes the intended checkout directory.
- Suggested fix: Use '..\..\target\pg-test' from docs/probes (or resolve from the repo root), and remove the same contents-delete in win\1probe.ps1 unless the folder is provably the probe's own.
- Locations:
  - `docs/probes/win_probe.ps1:1` param(\1string\1$Dir = (\1IO.Path\1::GetFullPath((Join-Path $PSScriptRoot '..\..\..\target\pg-test\probe'))))
  - `docs/probes/win_probe.ps1:109` Get-ChildItem -LiteralPath $Dir -Force \1 Remove-Item -Force
  - `docs/probes/v1_symlink.ps1:3` $testRoot = if ($env:PG\1TEST\1DIR) { $env:PG\1TEST\1DIR } else { Join-Path $PSScriptRoot '..\..\..\target\pg-test' }
  - `docs/probes/v1b_junction.ps1:4` $testRoot = if ($env:PG\1TEST\1DIR) { $env:PG\1TEST\1DIR } else { Join-Path $PSScriptRoot '..\..\..\target\pg-test' }

### SEC-164 [medium/data-integrity] Recursive delete of a user-supplied work folder with no guard — CONFIRMED

- Verdict: CONFIRMED. oracle.ps1:57 and the other pairing scripts Remove-Item a user-given work folder recursively, with no check for the source catalog.

- Confidence: high
- Kind: missing-check
- Trigger: A pairing script is run with -Work set to a folder that holds the -Catalog, for example -Catalog D:\scratch\pairing\catalog.sqlite -Work D:\scratch\pairing (the layout the docs use). oracle.ps1 deletes $Work recursively before copying the catalog into it.
- Consequence: The source catalog (the user's data) is deleted. The same pattern in peak\1mem, sweep and few\1sweep, and the -Dir delete in win\1probe, can remove whatever a mistyped path points at. Nothing checks that the folder is a scratch folder or that it does not contain the source.
- Suggested fix: Refuse to run when $Work contains or equals the -Catalog path or the repo, or delete only a subfolder the script itself created (a fixed name such as pg\1pairing\1oracle\run) with a marker file check.
- Locations:
  - `tools/profiling/pairing/oracle.ps1:57` if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
  - `tools/profiling/pairing/oracle.ps1:81` Remove-Item -LiteralPath $Work -Recurse -Force
  - `tools/profiling/pairing/peak_mem.ps1:29` if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
  - `tools/profiling/pairing/sweep.ps1:40` if (Test-Path $Work) { Remove-Item -LiteralPath $Work -Recurse -Force }
  - `tools/profiling/pairing/few_sweep.ps1:58` Remove-Item -LiteralPath $Work -Recurse -Force

### SEC-165 [medium/data-integrity] Unchecked python exit codes let the oracle pass without owing any pairs — CONFIRMED

- Verdict: CONFIRMED. python exit codes are discarded with Out-Null (oracle.ps1:68, keep.ps1:55) and drop.py's result is not checked.

- Confidence: medium
- Kind: missing-check
- Trigger: set\1owed.py raises on the copied catalog (for example a schema mismatch in the analysis table). Its exit code and output are discarded by Out-Null, and drop.py's result is not checked either.
- Consequence: Nothing is owed. The probe runs with PG\1PROBE\1KEEP\1OWED=1 and does nothing. pdq\1pairs is unchanged, so it still equals the oracle when the catalog was already correct, and the case prints 'identical True still owed 0' and counts as a pass without exercising the path. keep.ps1 likewise registers a template that owes nothing and measures it under a 'few300'-style label.
- Suggested fix: Check $LASTEXITCODE after every python call and throw on failure. Also compare set\1owed's printed count with the requested count before the case runs.
- Locations:
  - `tools/profiling/pairing/oracle.ps1:68` python -I $setOwed $db $case.owed \1 Out-Null
  - `tools/profiling/pairing/oracle.ps1:69` $dropped = python -I $dropPy $db
  - `tools/profiling/pairing/keep.ps1:55` python -I (Join-Path $here 'set\1owed.py') $db $k \1 Out-Null
  - `tools/profiling/pairing/keep.ps1:56` python -I $dropPy $db
  - `tools/profiling/pairing/keep.ps1:57` $templates\1"$build$k"\1 = $db

### SEC-166 [low/data-integrity] Owed count from set\1owed.py is discarded, so workload labels are not checked — CONFIRMED

- Verdict: CONFIRMED. set\1owed.py's LIMIT can owe fewer pairs than requested (set\1owed.py:17), and the printed count is discarded (keep.ps1:55).

- Confidence: medium
- Kind: wrong-result
- Trigger: A workload labeled few3000 is run on a catalog with fewer than 3000 eligible pictures (pdq\1quality \1= 50 and a hash). set\1owed.py's LIMIT ? (set\1owed.py:17) owes fewer than requested, and the caller discards the printed count.
- Consequence: The few-owed and sorted-path measurements and the keep verdict are attributed to the wrong owed count, with no warning.
- Suggested fix: Capture set\1owed's stdout, parse the owed count, and throw when it differs from the requested N.
- Locations:
  - `tools/profiling/pairing/set_owed.py:17` "ORDER BY (file\1id \1 2654435761) % 1000003 LIMIT ?"
  - `tools/profiling/pairing/keep.ps1:55` python -I (Join-Path $here 'set\1owed.py') $db $k \1 Out-Null
  - `tools/profiling/pairing/peak_mem.ps1:34` python -I $setOwed (Join-Path $Work 'catalog.sqlite') $n \1 Out-Null

### SEC-167 [low/data-integrity] Deleting the old prefix before the new one is in place leaves no prefix on a crash — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-153 and SEC-162: the old prefix is removed before the move (build-static-ffmpeg.ps1:169-170).

- Confidence: high
- Kind: crash-window
- Trigger: The process is killed or loses power between the rmdir at line 169 and the Move-Item at line 170 of build-static-ffmpeg.ps1.
- Consequence: The old prefix is gone and the new install sits in staged; the next run must rebuild from source. Consumers that link Work\prefix fail until then. Low because the build is regenerable from the pin.
- Suggested fix: Rename the old prefix aside (Move-Item to a .old folder), move the new one in, then delete the .old folder, so a crash always leaves one complete prefix.
- Locations:
  - `tools/build-static-ffmpeg.ps1:169` if (Test-Path $Prefix) { & cmd.exe /d /c "rmdir /s /q '"$Prefix'"" }
  - `tools/build-static-ffmpeg.ps1:170` Move-Item -LiteralPath $stagedPrefix -Destination $Prefix

### SEC-168 [low/data-integrity] Clean rebuild deletes the previous static libraries before the new build succeeds — CONFIRMED

- Verdict: CONFIRMED. build-jpeg-turbo.ps1:75 removes the prefix and link before the build. A failed build loses the last good output.

- Confidence: high
- Kind: crash-window
- Trigger: build-jpeg-turbo.ps1 is run and cmake configure or build fails (or the process is killed) after line 75 has removed $Prefix and $Link.
- Consequence: The last good libjpeg-turbo prefix and link archive are gone, and the app's and Qt's static link inputs are missing until the next successful build. Regenerable from the pin, so low.
- Suggested fix: Build into a fresh folder and rename it into place only after the install and checks pass, keeping the old prefix until then.
- Locations:
  - `tools/build-jpeg-turbo.ps1:75` if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
  - `tools/build-jpeg-turbo.ps1:89` Invoke-Checked 'cmake configure' { & cmake @cmakeArgs \1\1 "$Work\logs\configure.log" }

### SEC-169 [low/data-integrity] Leftover staged files from a crashed run are carried into the next prefix — CONFIRMED

- Verdict: CONFIRMED. The staged folder is reused and not cleaned after a crash (build-static-ffmpeg.ps1:137). Latent with the same pin.

- Confidence: medium
- Kind: stale-output
- Trigger: A run is killed after make install (line 164) but before the rmdir/move at lines 169-171, and the next run uses a different pin or configuration so its install does not overwrite every old file.
- Consequence: Stale libraries from the earlier build are moved into the new prefix and hashed into the manifest. The CRT check and link check run against them. Latent: with the same pin the same files are overwritten.
- Suggested fix: Remove $staged at the start of the run (before configure), as the script does for the source tree.
- Locations:
  - `tools/build-static-ffmpeg.ps1:137` $staged = Join-Path $Work 'staged'
  - `tools/build-static-ffmpeg.ps1:99` New-Item -ItemType Directory -Force -Path $Work, "$Work\logs", "$Work\test" \1 Out-Null

### SEC-170 [low/data-integrity] Killed run leaves the system power plan capped at 99% with no record of the original — CONFIRMED

- Verdict: CONFIRMED. PROCTHROTTLEMAX is set to 99 (keep.ps1:73-75). The restore is only in finally (:120), so a kill leaves it capped.

- Confidence: high
- Kind: crash-window
- Trigger: keep.ps1 is killed, the console window is closed, or power is lost after line 73-75 set PROCTHROTTLEMAX to 99. The restore is only in the finally block (lines 119-124).
- Consequence: The active power scheme keeps a 99% maximum processor state until someone resets it by hand. The original AC and DC values are printed to the console only, not stored, so a later reset relies on a remembered number.
- Suggested fix: Write the original values to a state file under the scratch folder before changing them, and restore from that file at the start of the next run if it exists.
- Locations:
  - `tools/profiling/pairing/keep.ps1:73` powercfg /setacvalueindex scheme\1current sub\1processor PROCTHROTTLEMAX 99
  - `tools/profiling/pairing/keep.ps1:120` if ($throttle) {

### SEC-171 [low/data-integrity] Copying a WAL catalog without its -wal sidecar gives a stale snapshot — CONFIRMED

- Verdict: CONFIRMED. Only catalog.sqlite is copied (keep.ps1:90, oracle.ps1:64), not the -wal file, and the catalog uses WAL (schema.rs:116).

- Confidence: medium
- Kind: partial-write
- Trigger: -Catalog points at a catalog the app has open or did not checkpoint on close. The catalog uses journal\1mode = WAL (crates/catalog/src/schema.rs:116), and the scripts copy only catalog.sqlite.
- Consequence: The probe runs on a copy missing committed transactions, so the timings and the oracle comparison describe the wrong catalog. The live catalog is only read, so no live data is lost.
- Suggested fix: Refuse a catalog with a non-empty -wal or -shm sidecar, or copy the sidecars too (few\1sweep.ps1 already removes its own sidecars).
- Locations:
  - `tools/profiling/pairing/keep.ps1:90` Copy-Item -LiteralPath $source (Join-Path $run 'catalog.sqlite')
  - `tools/profiling/pairing/oracle.ps1:64` Copy-Item -LiteralPath $Catalog $db

### SEC-172 [low/data-integrity] Fixed scratch folder names let two concurrent runs delete each other's copies — CONFIRMED

- Verdict: CONFIRMED. Fixed scratch names (keep.ps1:35-36) collide between concurrent runs.

- Confidence: medium
- Kind: race
- Trigger: Two pairing scripts run at the same time (keep.ps1, oracle.ps1, peak\1mem.ps1, sweep.ps1 or few\1sweep.ps1, each with a fixed %TEMP% name such as pg\1keep). The second run's Remove-Item -Recurse deletes the first run's catalog copy or drop.py mid-run.
- Consequence: The first run's probe fails or its timings are measured on a different file. The measurements are invalid, but the live catalog is untouched.
- Suggested fix: Give each run a unique folder (a GUID or timestamp suffix) and remove only that folder.
- Locations:
  - `tools/profiling/pairing/keep.ps1:35` $base = Join-Path (\1IO.Path\1::GetTempPath()) 'pg\1keep'
  - `tools/profiling/pairing/keep.ps1:36` if (Test-Path $base) { Remove-Item -LiteralPath $base -Recurse -Force }

### SEC-173 [low/data-integrity] A probe field missing from the output becomes 0 instead of failing the run — CONFIRMED

- Verdict: CONFIRMED. A missing part becomes an empty string and then 0 (keep.ps1:104), and keep.py then reports it as below resolution (keep.py:126).

- Confidence: medium
- Kind: missing-check
- Trigger: The probe's output lacks one of the timed parts (for example 'store' or 'cycles') because its format changed or the run failed partway.
- Consequence: The missing part is recorded as 0. keep.py then reports that part as 'below the probe's resolution: not judged' instead of flagging a failed run, so the result looks like a measurement limit.
- Suggested fix: Throw when a required part is not matched, so a missing field fails the run rather than becoming 0.
- Locations:
  - `tools/profiling/pairing/keep.ps1:104` foreach ($n in $parts) { $one\1$n\1 = \1int\1\1regex\1::Match($line, "\b$n (\d+)").Groups\11\1.Value }
  - `tools/profiling/pairing/keep.py:126` print(f"{workload}: {part} is 0 in some run, below the probe's resolution: not judged")

### SEC-174 [low/data-integrity] Manifest written with a single non-atomic Set-Content — CONFIRMED

- Verdict: CONFIRMED. Duplicate of SEC-156 for PowerShell: Set-Content writes manifest.json in place (build-jpeg-turbo.ps1:111, build-static-ffmpeg.ps1:215).

- Confidence: high
- Kind: partial-write
- Trigger: The process is killed during the Set-Content of manifest.json after the outputs have been built.
- Consequence: A truncated or empty manifest.json is left, so the record of inputs and output hashes is lost or unparseable, even though the libraries were built correctly.
- Suggested fix: Write the manifest to a temporary name and rename it into place after the write completes.
- Locations:
  - `tools/build-jpeg-turbo.ps1:111` $manifest \1 ConvertTo-Json -Depth 6 \1 Set-Content -Encoding utf8 "$Work\manifest.json"
  - `tools/build-static-ffmpeg.ps1:215` $manifest \1 ConvertTo-Json -Depth 6 \1 Set-Content -Encoding utf8 "$Work\manifest.json"

## F36: Eval harnesses: decoded output and benchmark file writes (C++ and Python)

- Kind: persistent-write
- Entry: eval/png-decoders/harness/harness.cpp:483 std::fopen(path, "wb") and tools/profiling/callgraph/callgraph.cpp:357 and :399 fopen(path, "w")
- Endpoint: eval/png-decoders/harness/harness.cpp:485 fwrite of decoded pixels; tools/profiling/callgraph/callgraph.cpp call-graph output; eval/\1.py bench writers (eval/bench\1thumbs.py, eval/bench\1decode.py, eval/verify.py)
- Summary: Eval and profiling harnesses read a corpus of PNG and JPEG inputs (via directory walks and file lists) and write decoded output or reports to caller-named paths. They are developer tooling and not shipped with the app, but they decode untrusted corpus files with third-party C decoders.

### SEC-175 [medium/data-integrity] Checkpoint resume appends onto a torn last line and glues two pairs together — CONFIRMED

- Verdict: CONFIRMED. verify.py:865 appends after the last line with no newline check, and the next row glues onto a torn one.

- Confidence: high
- Kind: partial-write
- Trigger: A checkpoint.add() write fails mid-row (ENOSPC on the drive holding WORK\1DIR, or power loss) so the .partial.csv ends with a row fragment and no newline. The next run resumes, opens the file in append mode, and writes its first new row directly after the fragment.
- Consequence: The glued physical line is parsed on the next resume as one row keyed by the torn pair's id. Its later fields are shifted: decision receives another pair's label (for example 'duplicate'), similarity gets a concatenated number, and so on. That pair then drops out of the confusion counts silently, because write\1outputs only counts decisions match, unrelated and error. The pair whose row was glued is absent from the checkpoint and is recomputed, so the bad row is also appended a second time.
- Suggested fix: On resume, truncate the file to its last complete newline before opening it for append, and drop rows that lack their full column count. Alternatively, write each row to a temp file and append with os.fsync.
- Locations:
  - `eval/verify.py:865` self.\1file = self.path.open("a", newline="", encoding="utf-8") # kept open for the run
  - `eval/verify.py:863` self.rows = {r\1"id"\1: {k: v for k, v in r.items() if v != ""} for r in csv.DictReader(f)}
  - `eval/verify.py:874` self.\1writer.writerow(csv\1row(row))
  - `eval/verify.py:875` self.\1file.flush()

### SEC-176 [medium/data-integrity] Stale checkpoint rows are reused for pair indexes after options change — PARTIAL

- Verdict: PARTIAL. Checkpoint rows are reused by pair index (verify.py:797-798). The checkpoint file name is built from prefix and model (:859), and whether prefix encodes the options was not checked.

- Confidence: high
- Kind: wrong-target
- Trigger: A run is interrupted, then rerun with a different --floor, --limit, --k, --shortcut or decision threshold without deleting WORK\1DIR/\1prefix\1\1\1model\1.partial.csv. --floor and --limit change the pairs list, so pair index N is a different pair. The file name depends only on the pairs set and the model.
- Consequence: results\1N\1 takes a row from the old run, with its old label, path\1a, path\1b, similarity and decision (computed under the old thresholds), and writes it as the result for the new pair N. The CSV and JSON (which records the new params) then misreport which pair was judged and under which settings. Nothing warns the user beyond the 'resuming' line.
- Suggested fix: Store a fingerprint (pair key, the options and the params) in each checkpoint row or in a header, and reuse a row only when its pair key and fingerprint match. Otherwise refuse to resume.
- Locations:
  - `eval/verify.py:859` self.path = WORK\1DIR / f"{prefix}\1{model}.partial.csv"
  - `eval/verify.py:797` if (row := checkpoint.done(pid)) is not None:
  - `eval/verify.py:798` results\1pid\1 = row
  - `eval/verify.py:778` checkpoint = Checkpoint(prefix, args.model)

### SEC-177 [medium/data-integrity] symbolize.py truncates the raw profile before writing its replacement — CONFIRMED

- Verdict: CONFIRMED. open("w") truncates before the write (symbolize.py:336-337).

- Confidence: high
- Kind: crash-window
- Trigger: A crash, Ctrl-C, power loss or ENOSPC during the final write of a profile (the raw profile is the only copy of a Pin run that takes about 50 times longer than normal).
- Consequence: The raw callgrind profile is left truncated or empty, and the symbolized version is never produced. The expensive run must be repeated. Since open('w') truncates first, a failure in the middle leaves a partial file that symbolize.py will skip on rerun ('positions: instr' is gone).
- Suggested fix: Write the symbolized text to a temp file beside the profile, flush and fsync it, then os.replace it over the original. Keep the raw file until the replace succeeds.
- Locations:
  - `tools/profiling/callgraph/symbolize.py:336` with Path(path).open("w", encoding="utf-8", newline="\n") as f:
  - `tools/profiling/callgraph/symbolize.py:337` f.write("\n".join(out))

### SEC-178 [medium/security] C harness sscanf reads past an unterminated .txt buffer — PARTIAL

- Verdict: PARTIAL. The sscanf-past-end mechanism holds if the buffer has no terminator. Whether read\1file adds one was not found in harness.cpp.

- Confidence: high
- Kind: oob-read
- Trigger: NAME.txt ends with its fourth number at end of file, with no trailing newline or NUL. This happens when a crash interrupts the Rust dump's txt write (main.rs:121), or the file is hand-edited, for example to '1920 1080 3 1'.
- Consequence: load\1reference passes the vector's data() to sscanf, which scans until a NUL or a non-matching character. With no terminator, the fourth %u reads past the vector's allocation. This is undefined behaviour: it may read heap garbage into the dimensions (leading to a wrong size-mismatch verdict) or crash the harness.
- Suggested fix: Copy the file into a std::string (which is NUL-terminated) and scan that, or use std::from\1chars on the bounded buffer.
- Locations:
  - `eval/png-decoders/harness/harness.cpp:140` std::vector\1uint8\1t\1 txt;
  - `eval/png-decoders/harness/harness.cpp:143` if (std::sscanf(reinterpret\1cast\1const char\1\1(txt.data()), "%u %u %u %u", &w, &h, &c, &b) != 4) return false;

### SEC-179 [medium/data-integrity] bench\1thumbs keeps all results in memory and writes them only at the very end — CONFIRMED

- Verdict: CONFIRMED. Results are collected in memory and written only at line 376 (bench\1thumbs.py:344, 376).

- Confidence: medium
- Kind: lost-update
- Trigger: Any exception after the quality pool starts: an encoder or ssimulacra2 subprocess failure for one image (the subprocess uses check=True and quality() only catches the read), or torch/CUDA failure in sscd\1reuse after roughly an hour of the quality and timing passes.
- Consequence: The exception propagates through pool.imap and the run dies before line 376. No JSON or partial results are written, so the whole measurement is lost and must be rerun. --no-sscd exists as a workaround, but the default path is exposed.
- Suggested fix: Catch per-image exceptions in quality() and record them as skipped, as the read already does. Write the JSON after the quality and timing passes, then run SSCD and rewrite it.
- Locations:
  - `eval/bench_thumbs.py:344` results = \1r for r in pool.imap(quality, \1(f, heights) for f in files\1, chunksize=2) if r\1
  - `eval/bench_thumbs.py:375` report\1"sscd\1cosine"\1 = sscd\1reuse(files, heights)
  - `eval/bench_thumbs.py:376` (WORK\1DIR / "bench\1thumbs.json").write\1text(json.dumps(report, indent=1), encoding="utf-8")

### SEC-180 [low/data-integrity] Same-stem inputs overwrite each other's dump outputs — CONFIRMED

- Verdict: CONFIRMED. Stem-based names collide (png-decoders main.rs:102, 120; harness.cpp:482).

- Confidence: high
- Kind: wrong-target
- Trigger: Two inputs with the same file stem in different folders (for example inputs/a/x.png and inputs/b/x.png) are dumped to the same DIR in one run.
- Consequence: The second input's NAME.raw and NAME.txt replace the first's. A later compare or verify\1dump then checks the other file's pixels against a reference that belongs to a different image. The C side repeats this, because stem() strips the directory.
- Suggested fix: Include the input's relative path (or a hash) in the output name, or refuse to write when a name is already used in this run.
- Locations:
  - `eval/png-decoders/src/main.rs:102` let name = Path::new(p).file\1stem().unwrap\1or\1default().to\1string\1lossy().into\1owned();
  - `eval/png-decoders/src/main.rs:120` std::fs::write(Path::new(dir).join(format!("{name}.raw")), &out)?;
  - `eval/png-decoders/harness/harness.cpp:482` const std::string path = dir + "/" + stem(args\1i\1) + "." + names()\1d\1 + ".raw";

### SEC-181 [low/data-integrity] Result files are overwritten in place and not atomically — CONFIRMED

- Verdict: CONFIRMED. Reports and CSVs are rewritten in place (verify.py:883, bench\1decode.py:207, bench\1thumbs.py:376).

- Confidence: high
- Kind: partial-write
- Trigger: A crash, power loss or ENOSPC during write\1text or open('w') of an existing report. In the verify --rescore path, the results CSV is the only copy of an expensive verification run, and no checkpoint exists in that path.
- Consequence: The previous report or results CSV is truncated or lost. The rescore path cannot recover the verified results without rerunning the image work.
- Suggested fix: Write each output to a temp file in the same folder, then os.replace it into place.
- Locations:
  - `eval/verify.py:883` with (WORK\1DIR / f"{prefix}\1{args.model}.csv").open("w", newline="", encoding="utf-8") as f:
  - `eval/verify.py:710` write\1outputs(prefix, args, params, labels, results, previous\1"seconds"\1)
  - `eval/bench_decode.py:207` (WORK\1DIR / "bench\1decode.json").write\1text(json.dumps(summary, indent=2), encoding="utf-8")
  - `eval/bench_thumbs.py:376` (WORK\1DIR / "bench\1thumbs.json").write\1text(json.dumps(report, indent=1), encoding="utf-8")

### SEC-182 [low/data-integrity] bench\1thumbs cleanup deletes another run's live scratch folders — CONFIRMED

- Verdict: CONFIRMED. The cleanup glob removes every thumbs\1\1 folder in CACHE\1DIR/tmp (bench\1thumbs.py:377-378).

- Confidence: high
- Kind: race
- Trigger: Two bench\1thumbs processes (or any other process using CACHE\1DIR/tmp) run at the same time. The first to finish runs the cleanup at line 377, which removes every thumbs\1\1 folder, including the other run's live workers' folders.
- Consequence: The other run's next ssimulacra2 or cjpegli call fails because its distorted.ppm or jpegli.ppm was removed (lines 104, 121, 171). The exception propagates through the pool and that run aborts with its results lost (see the lost-update finding above).
- Suggested fix: Remove only the folders this process created (for example, track the mkdtemp paths in a list or use one parent folder per run).
- Locations:
  - `eval/bench_thumbs.py:377` for d in (CACHE\1DIR / "tmp").glob("thumbs\1\1"): # this process's and the workers'
  - `eval/bench_thumbs.py:378` shutil.rmtree(d, ignore\1errors=True)

### SEC-183 [low/data-integrity] C dump's fwrite and fclose results are not checked, so a short write is reported as success — CONFIRMED

- Verdict: CONFIRMED. fwrite and fclose results are not checked (png-decoders harness.cpp:485-487).

- Confidence: high
- Kind: missing-check
- Trigger: A write error during the dump (disk full, or the volume disappears), so fwrite returns fewer bytes or fclose's final flush fails.
- Consequence: A truncated NAME.DECODER.raw is kept, and the harness prints its path as a successful dump. The truncated output is then used to diagnose decoder differences, which can mislead the analysis.
- Suggested fix: Check the fwrite count against out().size() and check fclose's return. On failure, remove the file and return 1.
- Locations:
  - `eval/png-decoders/harness/harness.cpp:485` std::fwrite(decoders\1d\1-\1out().data(), 1, decoders\1d\1-\1out().size(), f);
  - `eval/png-decoders/harness/harness.cpp:486` std::fclose(f);
  - `eval/png-decoders/harness/harness.cpp:487` std::printf("%s\n", path.c\1str());

### SEC-184 [low/data-integrity] JPEG harness dump reports success without checking the PPM stream — CONFIRMED

- Verdict: CONFIRMED. ofstream writes are not checked, and the function returns 0 (jpeg-decoders harness.cpp:206-210).

- Confidence: high
- Kind: missing-check
- Trigger: The output path cannot be opened (missing folder, no permission), or a write fails while the PPM is written.
- Consequence: The mode returns 0 and the caller treats the decode as dumped. No file or a truncated PPM is left, and the failure is silent.
- Suggested fix: Check f after open and after the loop (f.good() or an explicit close), and return 1 on failure.
- Locations:
  - `eval/jpeg-decoders/harness/harness.cpp:206` std::ofstream f(a\12\1, std::ios::binary);
  - `eval/jpeg-decoders/harness/harness.cpp:209` for (size\1t i = 0; i \1 size\1t(out.width) \1 out.height; i++) f.write(reinterpret\1cast\1const char\1\1(out.data + i \1 step), 3);
  - `eval/jpeg-decoders/harness/harness.cpp:210` return 0;

### SEC-185 [low/data-integrity] Ignored PPM write failures in the JPEG encoder quality test — CONFIRMED

- Verdict: CONFIRMED. write\1ppm's return value is discarded (test\1encoder.cpp:1019-1021).

- Confidence: medium
- Kind: missing-check
- Trigger: A PPM file cannot be written (disk full, or the work folder is unavailable), so write\1ppm returns false.
- Consequence: The return value is discarded, so ssimulacra2 is run on a missing or truncated file. Those samples drop out of the medians through the ssim2 returns-false path, and the quality report silently has smaller n.
- Suggested fix: Check the write\1ppm return at each call and record a failed sample as a failure of this test.
- Locations:
  - `eval/jpeg-encoder/test_encoder.cpp:1019` write\1ppm(base + "\1src.ppm", src\1i\1);
  - `eval/jpeg-encoder/test_encoder.cpp:943` return (bool)f;

### SEC-186 [low/data-integrity] Callgraph profile writes ignore fclose and fprintf failures, and a truncated file is reported as written — CONFIRMED

- Verdict: CONFIRMED. fclose and fprintf results are not checked (callgraph.cpp:390, 399-415).

- Confidence: high
- Kind: missing-check
- Trigger: A disk-full error or a crash while callgraph.cpp writes a per-thread profile, or a failed open of PREFIX.\1pid\1.threads.
- Consequence: A truncated per-thread profile (no 'totals:' line) is accepted by symbolize.py, which requires only 'positions: instr', and is processed as a complete profile. If the .threads listing failed, the run still prints 'written to'.
- Suggested fix: Check each fprintf and fclose return value, and write 'totals:' last only after the body succeeds. Report a failed .threads open as an error.
- Locations:
  - `tools/profiling/callgraph/callgraph.cpp:390` fclose(f);
  - `tools/profiling/callgraph/callgraph.cpp:399` FILE\1 f = fopen(path, "w");
  - `tools/profiling/callgraph/callgraph.cpp:415` fprintf(stderr, "callgraph: %llu instructions in %zu threads, written to %s.%d-\1\n", (unsigned long long)total,

### SEC-187 [low/security] Inflate-only modes size their buffers from unchecked IHDR dimensions — CONFIRMED

- Verdict: CONFIRMED. Buffers are sized from IHDR before inflate (png-decoders harness.cpp:358, 454, main.rs:375). The Rust-side bytes\116..26\1 claim was not checked.

- Confidence: high
- Kind: unbounded-alloc
- Trigger: A corpus PNG whose IHDR declares a very large width and height (for example 65536 x 65536 RGBA8, about 16 GiB, or larger), with a valid-looking IDAT or none. The inflate modes read it without validating it against the stream.
- Consequence: The harness allocates the declared size before inflating (std::bad\1alloc and abort, or heavy memory commit). The C stored\1scanline\1bytes product can also wrap for extreme dimensions. A wrapped size gives a wrong buffer, but the zlib and Wuffs calls honour the buffer length, so this does not cause an out-of-bounds write. The Rust side has the same pattern and indexes bytes\116..26\1 without a length check, so a short file panics.
- Suggested fix: Check the declared size against the bytes present (or a cap, such as the decoder limits used elsewhere) before allocating. Reject files shorter than 26 bytes before indexing.
- Locations:
  - `eval/png-decoders/harness/harness.cpp:358` std::vector\1uint8\1t\1 out(stored\1scanline\1bytes(info));
  - `eval/png-decoders/harness/harness.cpp:454` std::vector\1uint8\1t\1 whole(raw);
  - `eval/png-decoders/src/main.rs:375` let mut buf = vec!\10u8; raw\1;

## F37: Developer tool: catalog-copy probe examples (copy and delete trees)

- Kind: persistent-write
- Entry: crates/catalog/examples/pairs\1load\1probe.rs:84 std::fs::copy(source, &db) and crates/catalog/examples/pairs\1write\1probe.rs:80 copy of the catalog named on the command line
- Endpoint: crates/catalog/examples/pairs\1load\1probe.rs:108 and pairs\1write\1probe.rs:114 remove\1dir\1all(&dir)
- Summary: Probe programs copy a user-named catalog into a work folder and remove that folder afterwards. Dev-only examples; the paths come from argv.

### SEC-188 [low/data-integrity] Probe copies the catalog file without its -wal sidecar, or from a live catalog — CONFIRMED

- Verdict: CONFIRMED. The probes copy only the main file (pairs\1load\1probe.rs:84, pairs\1write\1probe.rs:80), with no -wal.

- Confidence: high
- Kind: race
- Trigger: A user-named catalog that is WAL mode (schema.rs:116) and has committed changes not yet checkpointed (Catalog drop does not checkpoint, core.rs:55-60; the app writes with autocheckpoint), or one a running app is writing while the probe copies it. Then pairs\1load\1probe.rs:84 or pairs\1write\1probe.rs:80 runs std::fs::copy(source, &db) on the main file only.
- Consequence: The probe measures a stale snapshot that lacks the newest committed changes, so its row counts and timings do not match the live catalog. If the copy races a checkpoint, the copy can be torn and fail with a malformed-database error. The source is only read and never written, so no user data is damaged. The defect is a wrong probe result, with no effect on the app.
- Suggested fix: Copy the -wal sidecar with the main file (or run wal\1checkpoint(TRUNCATE) through a separate connection to the source before copying, or use SQLite's backup API). Refuse or warn when the source has a non-empty -wal.
- Locations:
  - `crates/catalog/examples/pairs_load_probe.rs:84` std::fs::copy(source, &db).context("copy the catalog")?;
  - `crates/catalog/examples/pairs_write_probe.rs:80` std::fs::copy(source, &db).context("copy the catalog")?;
  - `crates/catalog/src/core.rs:55` /// Merge the WAL into the file, then close the connection (on drop, too, without the merge). The last connection

### SEC-189 [low/data-integrity] Copy folder is removed only on the success path — CONFIRMED

- Verdict: CONFIRMED. The scratch folder is removed only on success (pairs\1load\1probe.rs:108). The error path skips it.

- Confidence: high
- Kind: leak
- Trigger: Any error after run\1folder returns in either probe (for example Catalog::open at pairs\1load\1probe.rs:87, timed(...)? at :94, or the check ensure! at pairs\1write\1probe.rs:97), or a remove\1dir\1all failure at the end (for example on Windows if a handle is still open), or a panic.
- Consequence: The full catalog copy stays in the scratch folder until the next run of the same probe. Its lock is released at process exit, so the next run's sweep removes it. The copy takes disk space, and the folder keeps a copy of a real catalog (with its paths) in the temp folder in the meantime. No original is lost.
- Suggested fix: Remove the copy folder from a guard's Drop (or wrap the body in a closure and remove the folder on both outcomes) so that every exit path removes the copy.
- Locations:
  - `crates/catalog/examples/pairs_load_probe.rs:94` times.entry(variant).or\1default().push(timed(&catalog, &db, variant)?);
  - `crates/catalog/examples/pairs_write_probe.rs:97` check(&conn, variant, &pictures)?;
  - `crates/catalog/examples/pairs_load_probe.rs:108` std::fs::remove\1dir\1all(&dir)?;
