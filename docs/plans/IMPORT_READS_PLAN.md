# Import reads, round 3: measure what matters first, then act

## Context

d8039b54 (JPEG up to the end of its header) and 1b1544c8 (PNG at its two ends) made the import read 16-64 KiB per
JPEG/PNG. The user asked whether more is possible, for these, the other file types and video. Two rounds of
adversarial review (six independent reviews) reshaped the plan; this version folds in round 2.

Facts the plan now rests on (verified 2026-11-17):

- The tree is clean at HEAD 59d496ec; the perf work is committed (f687d0b7: `crates/perf`, `PG_PERF_LOG`, the
  `import.store` tally, `gallery perf`). HEAD raised `IMPORT_VERSION` 2 -> 3, so baselines and scratch catalogs are
  built at HEAD.
- The library is one root, `D:\PhotoGallery\datasets\pinterest` (690 folders), all on D: (Kingston NV2 NVMe):
  187,616 JPEGs (27.8 GB, avg 148 KB), 875 WebM + 837 MP4 (4.1 GB), 141 PNG, 51 WebP, no GIF/TIFF/BMP. Hard disks:
  G: (Toshiba MQ01ABD100 laptop HDD, holds `wallhere`) and E: (Seagate Exos 24 TB, the HDD the 70M-library notes in
  `IO.md` refer to; its bench copies were deleted by the user).
- The grid shows files before the import ends (`catalog/src/browse/read/grid.rs:26,41`: width/height 0 until imported),
  and visible thumbnails are made on demand (`app/src/images.rs:207-221`). So the import mainly fixes layout; the
  Thumbs pass reads every file whole after it (`runner.rs:497-498`, MFT order, `thumbcache/src/reader.rs:83`).
- At ~14k files/s the NVMe import reads the library's JPEGs in ~13 s; a writer at ~0.1 ms/file (an estimate from
  instruction counts, `PROFILING.md:1010`) would cap it near 10k/s.

**The question that decides everything: did the header-only import make import + thumbnails slower on a hard disk?**
Before d8039b54 the 256 KiB prefix read a whole 148 KB JPEG and left it in the file cache for the Thumbs pass; now the
import reads 16 KiB with read-ahead off, so the Thumbs pass may seek to every file a second time. On NVMe that likely
doesn't matter; on an HDD it could outweigh the import's gain.

## Step 1: two deciding measurements (no product code)

Rules: the app closed (it runs on the live catalog), SysMain noted, Defender state noted (off here, on for most users);
cold = `purge.ps1` (made to fail on a non-zero NTSTATUS; committed under `tools/` so it can be reproduced) plus, on a
hard disk, evicting the drive's own cache (the 4,000 random 128 KiB raw reads that worked on E:) or a fresh block per
round; folders re-listed after the purge (warm directory indexes, as after a scan). 3 rounds per configuration in
rotating order, median and range; a difference counts only when the medians lie outside both ranges. Scratch
catalogs only (`--catalog-dir D:\PhotoGallery\scratch\import_reads\...`), a fresh copy of one scanned catalog per run.
The perf log's printed verdict (60% thresholds, `perf/src/lib.rs:613-620`) is not the decision rule; the fields are.

M1. NVMe, the whole library, HEAD, defaults: `gallery import --no-dups` with `PG_PERF_LOG`. `writer_busy_pct >= 90`
means writer-bound (then deeper queues are dead on NVMe); also `workers_wait_reads_pct`, the `import.store` tally,
readers' CPU time. Then `gallery thumbs` on the same catalog: the end-to-end time (scan + import + thumbs) is the
baseline every later change is judged by.

M2. HDD, HEAD vs 1b1544c8 (the 256 KiB prefix), import + thumbs end-to-end on a fixed block of ~10k library JPEGs
copied once to E: (and the `wallhere` JPEGs on G:). Records each pass's time and the total.

## Step 2: act on M1/M2

- **If M2 shows HEAD slower end-to-end on the HDD**: the fix is the HDD read rule, not a deeper queue. Candidates,
  measured the same way, in this order: (a) on a drive with a seek penalty, read files under a size threshold whole
  (one seek; the Thumbs pass then finds them cached while the cache holds the pass); (b) run Import and Thumbs in
  batches over the same files, so the thumbnails are made while the files are cached (works whatever the cache size,
  e.g. for the 70M library). The drive kind comes from `IOCTL_STORAGE_QUERY_PROPERTY` /
  `StorageDeviceSeekPenaltyProperty` per volume in `pg-platform` (Linux: `/sys/dev/block/M:m/queue/rotational`; other
  platforms: unknown), with "unknown" or a failed query treated as no seek penalty (today's behaviour). A pass whose
  pending files span volumes applies the rule per file by its volume (the read closure sees the item; no per-pass
  `Options` change needed).
- **If M1 shows NVMe read-bound (writer < 90%)**: a single deeper default for the import's readers. A sweep 8/16/32/64
  with `--slots` and `--buffer-mb` pinned to what the change would adopt (today `--readers` also scales both,
  `args.rs:53-55`). Adopted as `Options::header_reads` for the app's import (`runner.rs:661`) and `gallery import`
  (`readers` becomes `Option` in `PipelineArgs`). The memory bound is stated correctly in the doc: budget + readers x
  `GATHER_CAP` + one whole file (gather admissions are checked, `pipeline.rs:583,737`; growth is counted, not checked,
  `:674-685,787`). With 32+ readers, measure the pool's `notify_all` wakeups (`:759,772,783`) and the per-item
  checkpoint mutex (`control.rs:271`) through readers' CPU time; wake only the next item's reader if it shows. No
  per-drive depth unless the sweep shows NVMe and HDD optima conflict. UI check: a probe opening unbuffered, fresh
  files on the same disk every 200 ms, p95 during the import <= 2x idle.
- **If neither**: the reads are done; report that, with the numbers, and update `IO.md`/`STORAGE.md` (they still say
  decoding bounds the import above ~1,600 files/s and "8 readers ... plain std::fs::File reads").

Outside reads, and bigger for a new library on any disk (to raise with the user, not built in this plan): import the
shown folder first, so the first screen is laid out in well under a second however long the whole import takes. It
needs its own plan (how the grid lays out width-0 cells; the import's order from the grid's query).

## Lower value, kept as decided with the user (after Step 2)

The round-2 value review recommends cutting both: they save under a second on NVMe for this library.

- **Video (B)**: a probe-only open in `Input::open` (`video/src/ffi.rs:93`): a 32 KiB I/O buffer (ffprobe n9.0.2 reads
  ~64 KB of a library MP4, ~110-140 KB of a WebM; the crate's 256 KiB buffer is the cost) and a random-access open (a
  `&Path` variant of `open_read_random` in `pg-platform`, matching std's long-path and NUL handling so `Unopened` and
  error texts don't change). `frame_into`/`frame` (thumbnails, viewer) keep 256 KiB and read-ahead. No change to
  `find_stream_info` or `Decoder::open` (H.264 SAR comes from the decode). Both modes reachable from outside the crate
  (a hidden `probe_with(path, Open)`). Check: `probe_check` compares the whole `Result<Probe>` and counts seeks, over
  the library videos and a generated set (ffmpeg CLI as `crates/library/tests/video.rs:79-114`: HEVC MOV with/without
  faststart, rotated, fragmented, MKV, AVI, MPEG-PS, truncated, no moov, junk); tests in `crates/library/tests/video.rs`
  as relative checks (probe-only == full), since Linux CI's apt FFmpeg differs.
- **WebP and BMP gathers (C)**; TIFF deferred (decided). Corrected design:
  - API: a gather needs a third outcome, "read whole now" (a budgeted, unserialized whole read), so a many-frame WebP
    doesn't go through the serialized `read_the_rest`; `Read::Gather`'s fn returns `Gathered | Whole | Fallback`.
  - WebP (fork `image-webp` 1e9023c `decoder.rs`, `extended.rs`; `container/src/lib.rs:675-790`): `None` unless the top
    walk ends exactly at RIFF size + 8 == file length; `None` for a VP8X < 10 bytes, a simple VP8 < 10, a VP8L < 5;
    every VP8X's data kept (`WebpAnimation::see` ORs each one's flag); ANIM, EXIF, XMP whole; image payloads cut (VP8 to
    10, VP8L to 5); the first ANMF keeps its subchunk headers whatever the flag; later ANMFs cut to `min(size, 24)`;
    sizes rewritten. Its own RIFF header reader.
  - BMP: a `min(size, 16 KiB)` prefix when the file starts with `BM` (checked inline: `pg-container` has no internal
    dependencies, `tools/crate-layers.txt:8`).
  - Tests: fixtures in `pg_codec::fixtures` behind its `fixtures` feature (`deny.toml:56,62` allow `webp`/`image`
    only there), including files written by the app's own WebP rewrite (`crates/edit/src/rewrite/webp.rs`);
    analysis-equality tests in `pg-import` (new dev-dependencies: `pg-codec` with `fixtures`, `proptest`), comparing
    error texts too; proptest case count pinned as `container/src/lib.rs:1295`.

## Applies to every change

- `gather_check` counts a gather `Err` as a difference (compared as the stored "read: ..." text) instead of stopping at
  `?` (`gather_check.rs:69-71`).
- A gather error re-stats the file; a changed size is "changed while read", not a failure (`Positioned::read`'s
  `read_exact`, `container/src/lib.rs:404`).
- `IMPORT_VERSION` is not bumped: the facts are exact (failure texts may differ, which only matters to
  `--retry-errors`).
- Docs: `IO.md` (entries say where and when, newest first, cite the base commit and that the change was uncommitted),
  `STORAGE.md:17,33`, `crates/README.md:336-341`, `pipeline.rs:4-6`, `args.rs:32-33`, `import/src/lib.rs:7-9,140`,
  `video/src/ffi/io.rs:13` comment as each change lands.
- Constraints: native PowerShell build with `.env` and QT_DIR, no WSL, no full Qt build; examples and the baseline
  binaries are built up front for Step 1 (measurement needs them), product code per change built once; no commits or
  branches until asked.

## Verification

- Step 1's numbers decide Step 2; each adopted change is re-measured the same way (end-to-end, not only import files/s).
- `cargo test` for the crates touched, the library video tests with ffmpeg present, `just lint`.
- Exactness: `gather_check` / `probe_check` 0 differences, errors included.
