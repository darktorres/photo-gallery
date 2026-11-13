# Port ExifTool 13.60's read path for our metadata fields

## Context

The import reads orientation, rating and date taken from each file's EXIF and XMP. Commit 29c4d5e replaced
kamadak-exif with a homemade IFD reader. A homemade reader can't match ExifTool's 20+ years of handling odd files.
ExifTool itself can't be on the import's path: it would read each file a second time, out of disk order, and it
parses in Perl. So the parts of ExifTool that produce our fields are ported to Rust.

The reference is ExifTool 13.60, installed at `C:\Program Files\ExifTool`. Its source is
`exiftool_files\lib\Image\ExifTool.pm` plus `exiftool_files\lib\Image\ExifTool\*.pm`.

**The goal is checkable.** For every file we read, the port finds the same values ExifTool does, in the same places,
and picks the same winner. A harness runs both on real files and on mutated ones, and zero mismatches is the bar.

Validated against HEAD e428ab2 (2026-11-12) by a claim-by-claim check of every path, line, ExifTool range and
behaviour in this plan. The base 4e1ad4d (2026-11-02) is not an ancestor of HEAD: HEAD's history begins at the
squash commit 88e780b, so the paths and line numbers from the base no longer exist. Every path and line below is at
HEAD.

## Starting point (HEAD e428ab2)

The metadata code is in `crates/metadata/` (package `pg-metadata`, 1,459 lines in 7 files; `lib.rs` is 569 lines).

- **The IFD reader:** `struct Tiff` (`tiff.rs:10`), `Entry` (`tiff.rs:18`) and `Exif` (`exif.rs:33`, reader at
  `exif.rs:46–72`).
  - It wants 42 in the header (43 for BigTIFF, in TIFF files only).
  - Value sizes and bounds: `value_range` (`tiff.rs:118–135`) applies the TIFF type-size table, the count and the
    bounds. The entry count and per-entry bounds are checked at `tiff.rs:95–100`.
  - IFD0 gives Orientation (`tiff.rs:139–141`: BYTE, SHORT or LONG, plus LONG8 in BigTIFF), Rating and RatingPercent
    (`tiff.rs:155–163`: BYTE, SHORT or LONG, plus LONG8 or IFD8 in BigTIFF, each fitting a u32), and the XMP tag 700.
    A repeated tag reads as the last copy that reads; an unreadable later copy leaves the earlier value.
  - The Exif IFD (tag 0x8769) gives DateTimeOriginal and DateTimeDigitized.
  - There is no IFD1, no SubIFD and no visited set. Only IFD0 and the Exif IFD are walked.
- **Dates:** `exif_date` (`crates/metadata/src/exif.rs:11`, in pg-metadata, `pub(crate)`) applies kamadak-exif's date
  rules.
  - `parse_iso_date` (`crates/model/src/date.rs:9`, in `pg-model`) is also used outside metadata: by
    `crates/video/src/lib.rs:146` and `:148` (video creation time). The catalog's date filters now go through
    `Day::parse` instead.
  - `wall_clock_seconds` (`crates/model/src/date.rs:34`) is also used by `Day`, the video tests and `exif_date`.
- **The container walk:** `Container::walk` (`crates/metadata/src/container.rs:29`; `struct Container` at `:22`).
  - JPEG: APP1 only. EXIF is the first `Exif\0\0` segment. XMP is the standard packet first, then `ExtendedXmps`
    with no HasExtendedXMP check.
  - PNG: `eXIf` and iTXt only.
  - WebP: `EXIF` and `XMP`.
  - GIF: the XMP packet counts only if the walk reaches the trailer.
  - TIFF: the whole file is the EXIF block.
- **The orientation-only read:** `pg_metadata::orientation` (`crates/metadata/src/orientation.rs:14`, re-exported from
  `lib.rs:22`) does its own walk through `exif_block` (`orientation.rs:22`).
  - Callers: `crates/edit/src/lib.rs:33` (test code only), `crates/edit/src/rotate.rs:39`,
    `crates/render/src/display.rs:38` (in `decode_into`; `crates/render/src/maker.rs` reaches it through
    `display::decode` and `display::decode_into`), and `crates/writeback/src/lib.rs:545`.
- **`Meta::read`** (`crates/metadata/src/lib.rs:55`; it takes a `&mut MetaScratch` that keeps the buffers from file to
  file). Callers: `crates/analysis/src/analyze.rs:101` and `:143`, `crates/writeback/src/lib.rs:524`, the tests in
  `crates/library/tests/writeback.rs` (lines 170, 202, 295, 313, 416, 535 and 671), and
  `crates/analysis/examples/stage_times.rs:42`.
- **XMP:** parsed by quick-xml (`xmp_properties`, `crates/metadata/src/xmp.rs:55`). quick-xml has no other user (only
  `pg-metadata` depends on it; the workspace also patches it to the `darktorres/quick-xml` fork, `Cargo.toml:132`).
- **Shared chunk walkers:** `png_chunks`, `webp_chunks`, `gif_blocks`, `gif_header_len`, `GifBlock` and `Chunk` live
  in `pg-container` (`crates/container/src/lib.rs`). They are used by pg-edit's rewrite code
  (`crates/edit/src/rewrite/{chunk,gif,png,webp}.rs`), and by pg-metadata's `orientation.rs` and `container.rs`.
  `rotate.rs` and `crop.rs` use them only in test code. The rewrite users rewrite files, so their semantics must not
  change.
- **The in-place patch:** `patch_orientation` (`crates/metadata/src/orientation.rs:60`) sets every IFD0 orientation
  entry the reader takes, each at its own width (`tiff_orientation_patches`, `orientation.rs:35–55`). It is no longer
  limited to the first SHORT×1 entry; that changed in cb123b9 (2026-11-08), see Step 0.
- **Tests:** a test builder, `tiff_block` (`crates/metadata/src/lib.rs:192`, in the tests module; it delegates to
  `tiff_block_as` at `lib.rs:197`).
- **`IMPORT_VERSION` is still 1** (`crates/analysis/src/analyze.rs:27`). The 6 files from 29c4d5e's run, which gained
  a Rating of 4, aren't settled yet.

## Decisions

The defaults below stand unless a later decision changes them.

1. **Faithful everywhere, ExifTool's data-losing quirks included.** For example, ExifTool reads one JPEG segment ahead
   (`ExifTool.pm` 7358–7425), so a complete EXIF segment followed by a truncated one yields no EXIF. Any deviation
   would make "zero mismatches" unmeasurable, so each one is a later decision, made per case.
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
     - Rating: `Rating`, then `RatingPercent`, scaled by `percent`. ExifTool's `RatingPercent` name also carries the XMP
       `MicrosoftPhoto:Rating` copy (`Microsoft.pm` 248–249), which is priority 1 and so beats EXIF `RatingPercent`
       (priority 0). The port reads it under that name.
     - Date taken: `DateTimeOriginal`, then `CreateDate`, then `XMP-photoshop:DateCreated`.
   - Behaviour changes, counted in step 6:
     - Within XMP, a priority-1 property found later wins: the extended packet over the standard one, and later
       packets over earlier ones. Today the first wins.
     - The date chain reads XMP only through `XMP-photoshop:DateCreated`. HEAD also reads XMP `exif:DateTimeOriginal`
       and `xmp:CreateDate` (`crates/metadata/src/lib.rs:46–47`), and the new chain drops both.
     - Orientation is read from more places: IFD1, SubIFDs, several EXIF blocks, and the new carriers.
     - EXIF Rating above 5, and -1, read as none (Step 3). Decision 1 says faithful, so this deviation is listed here
       and counted in step 6.
   - ExifTool writes our ratings (writeback), so reading them back with its rules keeps one source of truth.
3. **XMP is ported too, and quick-xml goes.** ExifTool's XMP reader is a tolerant scanner, not an XML parser:
   - An undeclared `xmp:` prefix still matches.
   - A wrong trailing slash on a URI is fixed (ours has one on MicrosoftPhoto).
   - A broken element loses only its own nesting level.
   - Dates are normalized as they are read.
4. **Out of scope:** IPTC, makernotes, GPS, ExifTool's writing (rating and orientation writes stay with ExifTool), and
   every other tag. Frame counting stays ours. Formats stay the ones `Format::sniff` knows.

## Steps

### 0. Orientation patch that always matches the read (landed at HEAD in cb123b9, 2026-11-08)

The bug this step was written for: at base 4e1ad4d the patch changed only the first SHORT×1, so a LONG orientation or
a second copy left the old tag in place after the pixels were turned, and the image showed turned twice. HEAD fixes
it:

- `patch_orientation` (`crates/metadata/src/orientation.rs:60`) sets every IFD0 orientation entry the reader takes,
  each at its own width.
- `rotate.rs` (`crates/edit/src/rotate.rs:47`) reads the result back (`:48–53`) and fails with an error when the tag
  still reads wrong, so a mismatch is no longer a silent double turn.
- Tests for the LONG, two-copy and read-back cases exist.

What is still open in this step, as one commit:

- `patch_orientation` still returns `Some` without a read-back. The contract to add: it returns `Some` only if
  `metadata::orientation` of the result is `o`, and `None` otherwise.
- The writeback patch path (`crates/writeback/src/lib.rs:537–538`) has no read-back check. The patcher and the reader
  share `orientation_entries`, so this is defence in depth, not a double turn that occurs today.
- The rewritten closure in `rotate.rs` should turn a `None`, when the original had an orientation other than 1, into
  `Outcome::Orientation(total)`: the pixels are left as they are and the tag is set (patch, else ExifTool). The HEAD
  closure has no such fallback.
- Tests for the `None` path and for the `Outcome::Orientation` fallback.

The port (step 2) keeps the same contract: it patches every in-place `EXIF:Orientation` occurrence, and the read-back
moves into the patcher, so the patcher returns `None` rather than an unchecked `Some`.

### 1. Oracle harness (kept: `crates/library/examples/metadata_oracle.rs`; not written yet)

- **ExifTool driver:** reuses `pg_exiftool::ExifTool` (`crates/exiftool/src/lib.rs:35`, stay-open: started at `:62`,
  closed in `Drop` at `:270`). The program is `PG_EXIFTOOL`, read at `crates/exiftool/src/lib.rs:31` (`program()`)
  and called by the writes at `crates/writeback/src/lib.rs:521` and `:542`.
  - It gains `pub fn read_tags(&mut self, files, args) -> Result<String>` beside the private `run`
    (`crates/exiftool/src/lib.rs:92`).
- **Command, no `-fast`:** `-n -a -G1 -args -EXIF:Orientation -Rating -RatingPercent -DateTimeOriginal -CreateDate
  -XMP-photoshop:DateCreated -HasExtendedXMP`.
  - The first task: confirm how a batch separates files and keeps duplicates in order. `-args` keeps duplicates;
    JSON may merge them. If needed, run one file per `-execute`.
  - A second run without `-a` gives the winners and their groups.
- **Inputs:** N ExifTool processes run in parallel over a list: a folder, a list file, or an Everything query
  (`es.exe` 1.1.0.38, fetched into a scratch folder; not checked).
- **Cache:** JSONL in `<data root>\scratch_exif_oracle\`, keyed by path, size and mtime.
- **Our side:** dumps every occurrence (name, group1, the value as `-n` prints it, file order) and the winners,
  through step 2's recording sink.
- **Report:** mismatches by kind, with paths and ExifTool's warnings for each file. `--min` writes the smallest
  reproducing input.
- **Safety:** read-only on the library. It runs on E: only after the owner approves it, since E: is the library HDD.

### 2. The port (pg-metadata: the existing modules are extended, and `found.rs`, `jpeg.rs`, `png.rs`, `webp.rs`, `gif.rs` and `irb.rs` are new)

Every ported function cites its ExifTool file and line.

- **`found.rs`: the `FoundTag` store.**
  - Fixed slots for our names, each with priority, group0/group1, document and file order.
  - The priority directory: the first IFD with SubfileType 0 or OldSubfileType 1. So 0xFE and 0xFF are read too.
  - Low-priority directories: PreviewIFD; IFD1 for JPEG (`ExifTool.pm:7325`) and for ARW files, which are TIFF-based
    (`ExifTool.pm:8735`); and all tags (`*`) for JPEG Photoshop data outside APP13 (`Photoshop.pm:1043`).
  - Promotion of the existing value to priority 1 for the comparison. The new value wins when new ≥ old.
  - Group-filtered selection.
  - A `Sink` trait: the import's sink keeps winners and allocates nothing; the oracle's sink records everything.
- **`tiff.rs`: `DoProcessTIFF` + `ProcessExif`, read path** (`ExifTool.pm` 8592–8827 for the read path, which ends at
  its return after ProcessDirectory and the trailers; 8828–8970 is the write path and is not cited as read;
  `Exif.pm` 6278–7240).
  - Header:
    - II or MM.
    - The magic number is not required (only warned for JPEG APP1). Today's 42 check goes.
    - IFD0 offset ≥ 8.
  - Two source modes:
    - In memory (JPEG, PNG, WebP, IRB): an out-of-buffer value is a "Bad offset", and the entry is skipped.
    - File (a TIFF file): a short read aborts the directory and the chain.
  - Directory placement: `bytesFromEnd` of 0 or 2 is accepted; 1 or 3 is an "Illegal directory size".
  - Entries:
    - Formats 1–13 and 129. Format 0 is silent padding after the first entry. As the first entry it aborts the
      directory (`Exif.pm` 6470–6477), except for Sony ILCE, where the first entry is skipped instead.
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
    - A visited-address set, as ExifTool's PROCESSED check (`ExifTool.pm` 9115–9122): a second `ProcessDirectory` on an
      address already registered is skipped with a warning and contributes no tags, so it can change values. Implement
      the same first-registration order and warning. ExifTool applies the skip inconsistently (an IFD chain revisit
      stops a cycle, but a directory first read as ExifIFD is read again as IFD1), and the port matches that.
  - Our tags are read in any IFD, with group1 set to that IFD's name.
- **Container walks: separate from the shared walkers.**
  - `png_chunks`, `webp_chunks` and `gif_blocks` (in `pg-container`) keep their current semantics for pg-edit's
    rewrite code.
  - The ported walks are their own functions in `jpeg.rs`, `png.rs`, `webp.rs` and `gif.rs`. They may share
    low-level helpers.
- **`jpeg.rs`: `ProcessJPEG`** (`ExifTool.pm` 7272–8536: the scan and SOS stop at 7346–7690; the APP1 block at
  7771–7952, with the EXIF match at 7774, the XMP match near 7936 and the ExtendedXMP store at 7857–7880; the APP13
  dispatch at 8421–8450).
  - The scan: junk and fill bytes skipped, one segment read ahead, stop at SOS.
  - APP1 EXIF:
    - The match is `/^(.{0,4})Exif\0./is`.
    - Multi-segment EXIF is joined into one block. That join is one of the owned buffers here.
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
- **`webp.rs`** (`RIFF.pm` 557–587, 2026–2218).
  - The `EXIF` chunk, with or without the `Exif\0\0` prefix.
  - `XMP ` and `XMP` followed by a NUL byte: four-byte chunk ids with a trailing space and a trailing NUL. ExifTool
    warns "Incorrect XMP tag ID" on the NUL variant and still reads it.
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
  transcode produces an owned buffer. The multi-segment EXIF join and the assembled extended XMP are the owned buffers
  on the JPEG path.

### 3. App policy (the `Meta` code in `crates/metadata/src/lib.rs`)

- **One walk:** `Meta::read` walks the container once, feeds the store, and applies decision 2.
- **`metadata::orientation`** uses the same walk with an orientation-only sink.
- **Rating:**
  - `Rating` as a real number in 0..=5, rounded. -1 and out-of-range values read as none (listed as a deviation in
    decision 2).
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
  `crates/library/tests/writeback.rs:129`.
- Fuzzing: a `cargo-fuzz` target on `Meta::read`. Only `metadata_read` exists today; per-format targets are new work.
  Fuzzing runs in the Linux container, not on MSVC. Each target must never panic, never hang, and stay within a fixed
  amount of work per input.

### 5. Differential runs, until zero raw mismatches

1. ExifTool's test images: `t/images` in `Image-ExifTool-13.60.tar.gz`, checksum-verified, kept on D:. The tarball is
   not on D: today; fetch it to D: and record its SHA-256 when it is fetched.
2. `<data root>\datasets\pinterest` (189k files, mostly stripped re-encodes).
3. About 1M JPEG/PNG/WebP/TIFF files from E:, spread across folders, chosen with Everything. Only after owner approval.
4. A mutation corpus: EXIF and XMP blocks from 1–3, mutated and wrapped in minimal files.
   - Mutations: offsets, counts, formats, byte order, truncation, cycles, junk between segments, duplicate blocks,
     broken XML, namespace variants.

Each mismatch is fixed in the port, and its minimized input becomes a builder test.

### 6. Switch-over, without losing ratings

- **App-level diff:** the old `Meta` against the new one on the default catalog's files. Today's reader is kept as
  `metadata::legacy` until this step ends.
- **Ratings:** `AnalysisWriter::sync_file_rating` (`crates/catalog/src/writers.rs:371`, in `impl AnalysisWriter` at
  `:249`, called from `store_analysis` at `:338`) adopts any file rating that differs from `file_ratings` as an
  external change, unless one of our rating writes is queued (`:374–375`). A reader that reads an unchanged file
  differently would overwrite the catalog rating.
  - So for each file where the old read equals `file_ratings` and the new read differs, `file_ratings` is set to the
    new value. `ratings` is not touched.
  - This is a one-off pass in the switch-over commit, after a backup of the catalog. It runs before any re-analysis,
    so ratings can be compared before and after it.
  - 29c4d5e's 6 files are the exception: their old read was kamadak's, so today's `file_ratings` doesn't hold 4 for
    them. If they still read 4, the sync adopts it as intended. Their `file_ratings` are set from a read-only copy of
    the catalog, not the live library, and the reader that produced them is named separately.
- **Orientation and date:** count the files whose stored values change. The choice is between:
  - bump `IMPORT_VERSION`, which re-analyses everything (hours on the HDD); or
  - re-analyse only those files: deleting their `analysis` rows sets `files.pending` through the `analysis_removed`
    trigger (`crates/catalog/src/schema.rs:771`, in BASELINE at 771–773; `pending` is documented at `schema.rs:522`).
  - The 6 files are settled in the same decision.
- **Then remove** `legacy` and quick-xml from the workspace `Cargo.toml` (including its quick-xml fork patch) and
  `crates/metadata/Cargo.toml`, checked with
  `cargo deny` and `cargo machete`.

### 7. ImageRater's metadata fixer for orientation writes (pg-exiftool `lib.rs`, used by pg-writeback `crates/writeback/src/lib.rs`)

Independent of the port; its own commit. Not done yet.

- **Today:**
  - `ExifTool::write_rating` (`crates/exiftool/src/lib.rs:131`) retries once after ImageRater's rebuild
    (`fix_metadata_and_retry`, `ImageRater\rust\src\services\file_operations.rs:256`):
    `-all= -tagsfromfile @ -all:all -unsafe -icc_profile`.
  - `write_orientation` (`:174`) doesn't retry. When ExifTool won't update a file, `process` keeps the turn in the
    catalog only (`keep_turn_in_catalog`, `crates/writeback/src/lib.rs:693`).
- **Change:** one helper, `write_tag(tag, extra_args, path, out)`, holds the try, then the removal of the half-written
  output, then the rebuild and one retry. Both methods use it; orientation passes `-n`.
  - The read-back check in `process` stays.
  - `copy_tags` (crops) reads its source and writes its destination file in place (`-overwrite_original`), with no
    retry of its own. The helper does not change it.
- **The rebuild is slightly lossy:** segments ExifTool can't copy are dropped. That is the trade ratings already
  make, and it only happens after the plain write has failed.
- **Test**, in `crates/library/tests/writeback.rs`, skipped without ExifTool:
  - A JPEG whose metadata 13.60 won't update in place, but will after the rebuild. Finding such damage is the first
    task, e.g. an IFD entry pointing outside the segment.
  - The turn must reach the file, not the catalog.

### 8. Docs

- A measurement entry for the port: the rules ported, mismatch counts per corpus and run, and every behaviour change,
  including step 0's bug. `docs/FINDINGS.md` no longer exists (it was deleted at 8486787), so the entry goes in a new
  topic file under `docs/temp/`, named when step 8 lands.
- Module docs name ExifTool 13.60 as the source.
- **License:** the port derives from ExifTool (Artistic 1.0 or GPL). The repo has no license today. Step 8 adds a note
  in `crates/README.md` that says this must be settled before any public release; no such note exists at HEAD.

## Order and commits

1. Step 0's remaining items first, on today's reader (the rest landed in cb123b9).
2. Step 1, with stubs on our side.
3. Step 2 one module at a time: store, TIFF, JPEG, PNG, WebP, GIF, IRB, XMP. Each with its tests and a harness run on
   t/images and pinterest.
4. Steps 3, 4, 5, 6 and 8.
5. Step 7 at any point.

One commit per verified step. Before building, check that no other build is running.

## Verification

- **Every commit:** `just lint` and `just test` (both run cargo with `--locked`), plus `cargo fmt --all --check`,
  `cargo deny check` and `cargo machete crates`.
- **Acceptance:**
  - Zero raw mismatches against ExifTool 13.60 on corpora 1, 2 and 4, and on 3 if approved.
  - Fuzzing clean for a fixed time per target.
  - The app-level diff explained file by file.
  - `ratings` identical before and after the file_ratings pass in step 6. The 6 files of 29c4d5e are the one case
    decided separately there.
- **Measured once at the end:**
  - Instruction counts with gungraun (`just perf`, which benches the committed HEAD), before and
    after.
    - `analyze_jpeg` (`crates/bench/benches/ir/linux.rs:50`, attributes at 48–49) reads `PG_BENCH_JPEG` at build time
      (`option_env!`).
    - A new `meta_read` bench uses a camera JPEG with EXIF, XMP and a thumbnail IFD. It is added in step 0's commit
      so the baseline exists.
  - A dhat import profile: no allocation at the metadata sites for files without inflated, joined or extended
    metadata.
