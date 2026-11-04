# Port ExifTool 13.60's read path for our metadata fields

## Context

The import reads orientation, rating and date taken from each file's EXIF and XMP. Commit 29c4d5e replaced
kamadak-exif with a homemade IFD reader. A homemade reader can't match ExifTool's 20+ years of handling odd files.
ExifTool itself can't be on the import's path: it would read each file a second time, out of disk order, and it
parses in Perl. So the parts of ExifTool that produce our fields are ported to Rust.

The reference is ExifTool 13.60, installed at `C:\Program Files\ExifTool`. Its source is
`exiftool_files\lib\Image\ExifTool*.pm`.

**The goal is checkable.** For every file we read, the port finds the same values ExifTool does, in the same places,
and picks the same winner. A harness runs both on real files and on mutated ones, and zero mismatches is the bar.

Revalidated against HEAD 4e1ad4d (2026-11-02). The other session's plan is finished.

## Starting point (HEAD)

`crates/metadata/src/lib.rs` (package `pg-metadata`, 1,180 lines; it was `crates/core/src/metadata.rs` before the
crate split):

- **The IFD reader:** `struct Tiff`, `Entry`, `Exif` (`:225–412`).
  - It wants 42 in the header (43 for BigTIFF, in TIFF files only).
  - IFD0 gives Orientation, Rating, RatingPercent (BYTE, SHORT or LONG) and the XMP tag 700. A repeated tag reads
    as the last copy.
  - The Exif IFD gives DateTimeOriginal and DateTimeDigitized.
  - There is no IFD1, no SubIFD, no visited set, and no format or count rules.
- **Dates:** `exif_date` (`crates/model/src/date.rs:11`, in `pg-model` now) applies kamadak's date rules.
  - `parse_iso_date` (`date.rs:25`) is also used outside metadata: by `crates/video/src/lib.rs:134/136` (video
    creation time). The catalog's date filters now go through `Day::parse` instead.
  - `wall_clock_seconds` (`date.rs:50`) is also used by `Day`, video tests and `exif_date`.
- **The container walk:** `Container::walk` (`:426`).
  - JPEG: APP1 only. EXIF is the first `Exif\0\0` segment. XMP is the standard packet first, then `ExtendedXmps`
    with no HasExtendedXMP check.
  - PNG: `eXIf` and iTXt only.
  - WebP: `EXIF` and `XMP`.
  - GIF: the XMP packet counts only if the walk reaches the trailer.
  - TIFF: the whole file is the EXIF block.
- **The orientation-only read:** `pg_metadata::orientation` (`:140`) does its own walk through `exif_block` (`:598`).
  - Callers: `crates/edit/src/lib.rs:26/38`, `crates/edit/src/rotate.rs:39`, `crates/render/src/display.rs:16/25`,
    `crates/render/src/maker.rs:111`, `crates/library/src/writeback.rs:264`.
- **`Meta::read`** (`:101`; it now takes a `&mut MetaScratch` that keeps the buffers from file to file). Callers:
  `crates/analysis/src/analyze.rs:103/145`, `crates/library/src/writeback.rs:247`,
  `crates/library/tests/writeback.rs:111/143`, `crates/analysis/examples/stage_times.rs:36`.
- **XMP:** parsed by quick-xml (`xmp_properties`, `:150`). quick-xml has no other user (only `pg-metadata` depends
  on it; the workspace also patches it to `vendor/quick-xml`).
- **Shared chunk walkers:** `png_chunks`, `webp_chunks`, `gif_blocks`, `gif_header_len`, `GifBlock` and `Chunk` live
  in `pg-container` (`crates/container/src/lib.rs`) and are also used by `pg-edit`'s `rewrite.rs`, `rotate.rs` and
  `crop.rs`. Those use them to rewrite files, so their semantics must not change.
- **The in-place patch:** `patch_orientation` (`:618`) patches the **first** IFD0 Orientation that is a SHORT×1.
- **Tests:** a test builder, `tiff_block` (tests, `:729`).
- **`IMPORT_VERSION` is still 1** (`crates/analysis/src/analyze.rs:29`). The 6 files from 29c4d5e's run, which gained
  a Rating of 4, aren't settled yet.

## Decisions (defaults I chose; tell me to change any)

1. **Faithful everywhere, ExifTool's data-losing quirks included.** For example, ExifTool reads one JPEG segment ahead
   (`ExifTool.pm` 7358–7425), so a complete EXIF segment followed by a truncated one yields no EXIF. Any deviation
   would make "zero mismatches" unmeasurable, so each one is a later decision for you, made per case.
2. **Selection: ExifTool's own rules, with a group filter where the app needs a specific source.**
   - **Orientation is `EXIF:Orientation`.** ExifTool's plain `Orientation` also merges XMP `tiff:Orientation`, in
     file order. Other viewers turn images by EXIF only, so taking XMP's would turn images they show upright.
   - **Date created is `XMP-photoshop:DateCreated`** (IPTC is out of scope).
   - The rest go by tag name: `Rating`, `RatingPercent`, `DateTimeOriginal`, `CreateDate`.
   - With a group filter, ExifTool keeps that group's copies and re-applies its priority and file-order rule
     (`ExifTool.pm` 5388–5418). The oracle asks for the same names.
   - Within a name, `FoundTag` decides (`ExifTool.pm` 9498–9678):
     - EXIF Orientation, Rating and RatingPercent are priority 0: the first one found wins, unless a later one is in
       the priority directory.
     - Dates are priority 1: the last one found wins, except JPEG IFD1, which is low priority.
     - XMP Rating beats EXIF Rating. EXIF DateTimeOriginal and CreateDate beat their XMP copies.
   - Across names, the app picks:
     - Rating: `Rating`, then `RatingPercent`, scaled by `percent`.
     - Date taken: `DateTimeOriginal`, then `CreateDate`, then `XMP-photoshop:DateCreated`.
   - Behaviour changes, counted in step 6:
     - Within XMP, a priority-1 property found later wins: the extended packet over the standard one, and later
       packets over earlier ones. Today the first wins.
     - When EXIF DateTimeOriginal is missing, XMP's now beats EXIF DateTimeDigitized.
     - Orientation is read from more places: IFD1, SubIFDs, several EXIF blocks, and the new carriers.
   - ExifTool writes our ratings (writeback), so reading them back with its rules keeps one source of truth.
3. **XMP is ported too, and quick-xml goes.** ExifTool's XMP reader is a tolerant scanner, not an XML parser:
   - An undeclared `xmp:` prefix still matches.
   - A wrong trailing slash on a URI is fixed (ours has one on MicrosoftPhoto).
   - A broken element loses only its own nesting level.
   - Dates are normalized as they are read.
4. **Out of scope:** IPTC, makernotes, GPS, writing, and every other tag. Frame counting stays ours. Formats stay the
   ones `Format::sniff` knows.

## Steps

### 0. Orientation patch that always matches the read (fix in today's code, its own commit)

A latent bug at HEAD:

- `Exif::read` takes IFD0 Orientation as BYTE, SHORT or LONG, the last copy winning.
- `patch_orientation` changes only the first SHORT×1.
- `crates/edit/src/rotate.rs:45` (`Outcome::Rewritten`) calls
  `pg_metadata::patch_orientation(&out, format, Orientation::UPRIGHT).unwrap_or(out)`. So when the orientation is a
  LONG (or a second copy), the pixels are turned but the old tag stays: the image shows turned twice.
- The writeback's patch path (`crates/library/src/writeback.rs:257`) has no read-back check either.

The fix:

- `patch_orientation` patches, in place, every entry the reader counts, at its own width.
- It returns `Some` only if `metadata::orientation` of the result is `o`. Otherwise it returns `None`.
- `rotate.rs`'s rewritten closure turns a `None`, when the original had an orientation other than 1, into
  `Outcome::Orientation(total)`: the pixels are left as they are and the tag is set (patch, else ExifTool).
- Tests: a LONG orientation, two copies, and a rewritten JPEG that reads back as 1.
- The port (step 2) keeps the same contract: patch every in-place `EXIF:Orientation` occurrence, and check the read.

### 1. Oracle harness (kept: `crates/library/examples/metadata_oracle.rs`; not written yet)

- **ExifTool driver:** reuses `pg_exiftool::ExifTool` (`crates/exiftool/src/lib.rs:23`, stay-open; the program is
  `PG_EXIFTOOL`, read in `crates/library/src/writeback.rs:351`).
  - It gains `pub fn read_tags(&mut self, files, args) -> Result<String>` beside the private `run` (`:61`).
- **Command, no `-fast`:** `-n -a -G1 -args -EXIF:Orientation -Rating -RatingPercent -DateTimeOriginal -CreateDate
  -XMP-photoshop:DateCreated -HasExtendedXMP`.
  - The first task: confirm how a batch separates files and keeps duplicates in order. `-args` keeps duplicates;
    JSON may merge them. If needed, run one file per `-execute`.
  - A second run without `-a` gives the winners and their groups.
- **Inputs:** N ExifTool processes run in parallel over a list: a folder, a list file, or an Everything query
  (`es.exe` 1.1.0.38, re-downloaded to scratch per memory).
- **Cache:** JSONL in `D:\PhotoGallery\scratch_exif_oracle\`, keyed by path, size and mtime.
- **Our side:** dumps every occurrence (name, group1, the value as `-n` prints it, file order) and the winners,
  through step 2's recording sink.
- **Report:** mismatches by kind, with paths and ExifTool's warnings for each file. `--min` writes the smallest
  reproducing input.
- **Safety:** read-only on the library. It runs on E: only with your go-ahead, since E: is the library HDD.

### 2. The port (pg-metadata: `crates/metadata/src/lib.rs` grows modules beside it)

Every ported function cites its ExifTool file and line.

- **`found.rs`: the `FoundTag` store.**
  - Fixed slots for our names, each with priority, group0/group1, document and file order.
  - The priority directory: the first IFD with SubfileType 0 or OldSubfileType 1. So 0xFE and 0xFF are read too.
  - Low-priority directories: PreviewIFD, plus IFD1 for JPEG only.
  - Promotion of the existing value to priority 1 for the comparison. The new value wins when new ≥ old.
  - Group-filtered selection.
  - A `Sink` trait: the import's sink keeps winners and allocates nothing; the oracle's sink records everything.
- **`tiff.rs`: `DoProcessTIFF` + `ProcessExif`, read path** (`ExifTool.pm` 8592–8788, `Exif.pm` 6278–7240).
  - Header:
    - II or MM.
    - The magic number is not required (only warned for JPEG APP1). Today's 42 check goes.
    - IFD0 offset ≥ 8.
  - Two source modes:
    - In memory (JPEG, PNG, WebP, IRB): an out-of-buffer value is a "Bad offset", and the entry is skipped.
    - File (a TIFF file): a short read aborts the directory and the chain.
  - Directory placement: `bytesFromEnd` of 0 or 2 is accepted; 1 or 3 is an "Illegal directory size".
  - Entries:
    - Formats 1–13 and 129. Format 0 is silent padding. An invalid first entry aborts the directory.
    - The size cap. "Suspicious offset" (below 8, or inside the entry block).
    - The excessive-count skip.
    - The warning count, which aborts after 11 (`Exif.pm` 6455). Only the counted warnings are modelled.
  - Values (`ReadValue`, `ExifTool.pm` 6287), typed and printable as `-n` strings:
    - Lists are joined with spaces. Rationals are computed as ExifTool does.
    - Strings are cut at the first NUL, with trailing spaces kept. Count 0 is an empty value.
    - 0x02bc is forced to `undef`.
  - Traversal:
    - ExifIFD (0x8769), with the "Wrong format" and "Bad value" pointer rules.
    - SubIFD (0x14A, at most 10) and the IFD1… chain.
    - A visited-address set. It never changes a value, only where ExifTool would hang.
  - Our tags are read in any IFD, with group1 set to that IFD's name.
- **Container walks: separate from the shared walkers.**
  - `png_chunks`, `webp_chunks` and `gif_blocks` (in `pg-container`) keep their current semantics for pg-edit's
    `rewrite.rs`, `rotate.rs` and `crop.rs`.
  - The ported walks are their own functions in `jpeg.rs`, `png.rs`, `webp.rs` and `gif.rs`. They may share
    low-level helpers.
- **`jpeg.rs`: `ProcessJPEG`** (`ExifTool.pm` 7272–7690).
  - The scan: junk and fill bytes skipped, one segment read ahead, stop at SOS.
  - APP1 EXIF:
    - The match is `/^(.{0,4})Exif\0./is`.
    - Multi-segment EXIF is joined into one block. That join is the one owned buffer here.
  - APP1 XMP:
    - The standard header, or the loose match.
    - ExtendedXMP chunks are kept by GUID and offset and assembled at SOS. Only the GUID named by the winning
      `HasExtendedXMP` is assembled.
  - APP13 Photoshop IRB.
- **`png.rs`** (`PNG.pm` 1412–1651, 1157–1283, 1341–1360).
  - EXIF:
    - `eXIf` in any case, and `zxIf`.
    - The `Exif\0\0` prefix rule.
  - XMP:
    - `XML:com.adobe.xmp` in tEXt, zTXt or iTXt. An iTXt with a language tag doesn't count.
    - `tXMP`.
  - The hex profiles `Raw profile type exif/APP1/xmp`.
  - Chunks after IDAT and after IEND are read. CRCs are not checked.
  - Inflation uses miniz_oxide.
- **`webp.rs`** (`RIFF.pm` 557–587, 2026–2217).
  - The `EXIF` chunk, with or without the `Exif\0\0` prefix.
  - `XMP` and `XMP\0` chunks.
  - The scan runs to EOF, not to the RIFF size. A truncated chunk stops it.
- **`gif.rs`** (`GIF.pm` 413–459).
  - XMP: the application extension, length bytes kept, trimmed at the xpacket end. XMP found before a truncation
    counts.
  - The frame count stays all-or-nothing.
- **`irb.rs`: Photoshop IRB** (`Photoshop.pm` 254–268). 0x0422 EXIF and 0x0424 XMP, in JPEG APP13 (`Photoshop 3.0\0`,
  multi-segment) and in TIFF 0x8649.
- **`xmp.rs`: `ProcessXMP`, `ParseXMPElement` and `FoundXMP`, only what our properties need** (`XMP.pm` 3098–3151,
  3463–3474, 3735–3767, 3881–4290, 4383–4716).
  - Encoding: BOM and UTF-16/32 detection.
  - The element scanner: same-name nesting, CDATA and comments skipped.
  - IDs: attributes and elements give the same ID. Qualifiers and `rdf:li` resolve to it.
  - Unescaping: the five named entities and numeric references only.
  - Namespaces: by URI when declared, with the slash fix and the version wildcard. An undeclared prefix is taken
    literally; a standard prefix bound to a foreign URI is not recognised.
  - Dates go through `ConvertXMPDate`.
  - An error drops only its own nesting level.
  - Values are borrowed unless unescaping or transcoding has to make them owned.
- **Allocation:** none on the common path. Only an inflate, a hex decode, a multi-segment join, an extended XMP or a
  transcode produces an owned buffer.

### 3. App policy (the `Meta` code in `crates/metadata/src/lib.rs`)

- **One walk:** `Meta::read` walks the container once, feeds the store, and applies decision 2.
- **`metadata::orientation`** uses the same walk with an orientation-only sink.
- **Rating:**
  - `Rating` as a real number in 0..=5, rounded. -1 and out-of-range values read as none.
  - Otherwise `RatingPercent`, through `percent`.
- **Orientation:** the winning `EXIF:Orientation` as an integer from 1 to 8, else 1.
- **Date taken:** one parser for ExifTool's form, `YYYY:MM:DD[ HH:MM[:SS[.f]]][zone]`.
  - EXIF strings arrive raw; XMP strings are already converted.
  - Blanks and impossible dates fail through `wall_clock_seconds`. A date alone means midnight.
  - `exif_date` goes. `parse_iso_date` and `wall_clock_seconds` (all in `pg-model`'s `date.rs`) stay: `Day`
    (the catalog's date filters) and videos use them.
- **`patch_orientation`:** step 0's contract on top of the store's occurrences. It covers the EXIF block, IRB EXIF
  and multi-segment EXIF where the bytes are in place. A hex or compressed carrier can't be patched, so it returns
  `None`, and ExifTool writes.
- **Callers and signatures** are unchanged.

### 4. Tests (committed)

- One unit test per ported rule, built with `tiff_block` grown into a builder (TIFF/IFD, plus JPEG, PNG, WebP, GIF and
  IRB wrappers). Each test names the ExifTool line it covers.
- One test runs every builder case through ExifTool and compares. It is skipped without ExifTool, as in
  `crates/library/tests/writeback.rs:79`.
- Fuzzing: a `cargo-fuzz` target on `Meta::read` for each format (nightly, MSVC). It must never panic, never hang, and
  stay within a fixed amount of work per input.

### 5. Differential runs, until zero raw mismatches

1. ExifTool's test images: `t/images` in `Image-ExifTool-13.60.tar.gz`, checksum-verified, kept on D:.
2. `D:\PhotoGallery\datasets\pinterest` (189k files, mostly stripped re-encodes).
3. About 1M JPEG/PNG/WebP/TIFF files from E:, spread across folders, chosen with Everything. Only with your
   go-ahead.
4. A mutation corpus: EXIF and XMP blocks from 1–3, mutated and wrapped in minimal files.
   - Mutations: offsets, counts, formats, byte order, truncation, cycles, junk between segments, duplicate blocks,
     broken XML, namespace variants.

Each mismatch is fixed in the port, and its minimized input becomes a builder test.

### 6. Switch-over, without losing your ratings

- **App-level diff:** the old `Meta` against the new one on the default catalog's files. Today's reader is kept as
  `metadata::legacy` until this step ends.
- **Ratings:** `AnalysisWriter::sync_file_rating` (`crates/catalog/src/lib.rs:1458`) adopts any file rating that differs from
  `file_ratings` as an external change. A reader that reads an unchanged file differently would overwrite your
  catalog rating.
  - So for each file where the old read equals `file_ratings` and the new read differs, `file_ratings` is set to the
    new value. `ratings` is not touched.
  - This is a one-off pass in the switch-over commit, after a backup of the catalog.
  - 29c4d5e's 6 files are the exception: their old read was kamadak's, so today's `file_ratings` doesn't hold 4 for
    them. If they still read 4, the sync adopts it as intended.
- **Orientation and date:** count the files whose stored values change. You choose:
  - bump `IMPORT_VERSION`, which re-analyses everything (hours on the HDD); or
  - re-analyse only those files: deleting their `analysis` rows sets `files.pending` through the `analysis_removed`
    trigger (`crates/catalog/src/lib.rs:350`).
  - The 6 files are settled in the same decision.
- **Then remove** `legacy` and quick-xml from the workspace `Cargo.toml` (including its `vendor/quick-xml` patch) and
  `crates/metadata/Cargo.toml`, checked with
  `cargo deny` and `cargo machete`.

### 7. ImageRater's metadata fixer for orientation writes (pg-exiftool `lib.rs`, used by pg-library `writeback.rs`)

Independent of the port; its own commit. Not done yet.

- **Today:**
  - `ExifTool::write_rating` (`crates/exiftool/src/lib.rs:91`) retries once after ImageRater's rebuild
    (`fix_metadata_and_retry`, `ImageRater\rust\src\services\file_operations.rs:256`):
    `-all= -tagsfromfile @ -all:all -unsafe -icc_profile`.
  - `write_orientation` (`:117`) doesn't retry. When ExifTool won't update a file, `process` keeps the turn in the
    catalog only (`keep_turn_in_catalog`, `crates/library/src/writeback.rs:312`).
- **Change:** one helper, `write_tag(tag, extra_args, path, out)`, holds the try, then the removal of the half-written
  output, then the rebuild and one retry. Both methods use it; orientation passes `-n`.
  - The read-back check in `process` stays.
  - `copy_tags` (crops) only reads its source, so it is unchanged.
- **The rebuild is slightly lossy:** segments ExifTool can't copy are dropped. That is the trade ratings already
  make, and it only happens after the plain write has failed.
- **Test**, in `crates/library/tests/writeback.rs`, skipped without ExifTool:
  - A JPEG whose metadata 13.60 won't update in place, but will after the rebuild. Finding such damage is the first
    task, e.g. an IFD entry pointing outside the segment.
  - The turn must reach the file, not the catalog.

### 8. Docs

- A FINDINGS entry: the rules ported, mismatch counts per corpus and run, and every behaviour change, including step
  0's bug.
- Module docs name ExifTool 13.60 as the source.
- **License:** the port derives from ExifTool (Artistic 1.0 or GPL). The repo has no license today. A note in
  `crates/README.md` says this must be settled before any public release.

## Order and commits

1. Step 0 first, on today's reader.
2. Step 1, with stubs on our side.
3. Step 2 one module at a time: store, TIFF, JPEG, PNG, WebP, GIF, IRB, XMP. Each with its tests and a harness run on
   t/images and pinterest.
4. Steps 3, 4, 5, 6 and 8.
5. Step 7 at any point.

One commit per verified step. Before building, check that no other session is building.

## Verification

- **Every commit:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all --check`, `cargo deny check`, `cargo machete crates`.
- **Acceptance:**
  - Zero raw mismatches against ExifTool 13.60 on corpora 1, 2 and 4, and on 3 if approved.
  - Fuzzing clean for a fixed time per target.
  - The app-level diff explained file by file.
  - `ratings` identical before and after the re-baseline pass.
- **Measured once at the end:**
  - Instruction counts with gungraun (`tools/profiling/wsl-bench.sh`, which benches the committed HEAD), before and
    after.
    - `analyze_jpeg` (`crates/bench/benches/ir/linux.rs:35`) reads `PG_BENCH_JPEG`.
    - A new `meta_read` bench uses a camera JPEG with EXIF, XMP and a thumbnail IFD. It is added in step 0's commit
      so the baseline exists.
  - A dhat import profile: no allocation at the metadata sites for files without inflated, joined or extended
    metadata.
