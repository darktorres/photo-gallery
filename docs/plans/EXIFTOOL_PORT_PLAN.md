# Port ExifTool 13.64's read path for our metadata fields

## Context

The import reads orientation, rating and date taken from each file's EXIF and XMP. Commit 29c4d5e replaced
kamadak-exif with a homemade IFD reader. A homemade reader can't match ExifTool's 20+ years of handling odd files.
ExifTool itself can't be on the import's path: it would read each file a second time, out of disk order, and it
parses in Perl. So the parts of ExifTool that produce our fields are ported to Rust.

The reference is ExifTool 13.64, installed at `C:\Program Files\ExifTool` (the Windows package
`exiftool-13.64_64.zip`, SHA-256 `72c29c44d1e860eebcaefa71692d979af9858fe9a6d7d791a5e7f1dac77804af`, which matches
exiftool.org's `checksums.txt`). Its source is the clone at `D:\GitHub\exiftool`, tag `13.64`:
`lib\Image\ExifTool.pm` plus `lib\Image\ExifTool\*.pm`. The clone's `lib` is the same as the package's
`exiftool_files\lib`, file for file, apart from line endings (checked 2026-11-25, 247 files). Every ExifTool line
cited here is at tag `13.64`.

**The goal is checkable.** For every file we read, the port finds the same values ExifTool does, in the same places,
and picks the same winner. A harness runs both on real files and on mutated ones, and zero mismatches is the bar.

**Scope.** The port covers all of ExifTool 13.64, read and write, as set out in `docs/plans/EXIFTOOL_PORT_DESIGN.md`.
This plan is the first slice of that design: the read side of the five containers (JPEG, PNG, GIF, WebP and TIFF) for
the six fields below. Its steps are the acceptance gate for that slice. The rest of the port is gated by the design's
phases and their done tests.

**Revision.** Written against e428ab2 (2026-11-12) and reviewed adversarially on 2026-11-14. Re-checked against
b33cf639 (2026-11-25): every repo path and line below is at b33cf639. Two commits since the last check changed the
plan: 41ca393c (the reader takes a rating in Windows' order, decision 2) and 13a41bdc (the rating write sets every
rating field and no longer rebuilds the metadata, step 7). The reference moved from 13.60 to 13.64 on 2026-11-25;
see "13.60 to 13.64" below. Every line must be re-cited when its step lands.

### 13.60 to 13.64

The ExifTool line numbers were moved from 13.60 to 13.64 through `git diff 13.60 13.64` in the clone. No cited range
falls inside a changed hunk. What changed in the modules this slice ports:

- `ExifTool.pm`: two file types (MIF3, MSF3), `SetByteOrder` returns the previous order, a JUMBF sequence warning in
  `ProcessJPEG` (8392–8395), and `HandleTag`'s `MakeTagInfo` (9342–9376). None of them touches our six fields.
- `Exif.pm`: two tags' `Format` is `undef` instead of `binary` (963, 1999). Not our tags.
- `GIF.pm`: an extension can be deleted (100, 435–441, 459–462). Write side only: each branch needs `$outfile`.
- `XMP.pm`: `FixUTF8` builds its result in a new string (3026–3054); the replacement of bad bytes is the same.
- `RIFF.pm`: `ProcessChunks` stops at a null tag of length 0 (1797–1798). That is the AVI/WAV walk, not WebP's.
- `Photoshop.pm`: one blank line removed (712).
- New: `Mini.pm` (560 lines), and `t/images/Mini.avif`. Not in this slice; the design lists it (section 10, O21).

## Starting point (b33cf639)

The metadata code is in `crates/metadata/` (package `pg-metadata`, 1,664 lines in 7 files; `lib.rs` is 752 lines).

- **The IFD reader:** `struct Tiff` (`tiff.rs:10`), `Entry` (`tiff.rs:18`) and `Exif` (`exif.rs:49`, reader at
  `exif.rs:62–86`).
  - It wants 42 in the header (43 for BigTIFF, in TIFF files only; `tiff.rs:46–50`).
  - Value sizes and bounds: `value_range` (`tiff.rs:121–138`) applies the TIFF type-size table, the count and the
    bounds. The entries are read as far as they are in the block (`tiff.rs:96–105`).
  - IFD0 gives Orientation (`orientation_kind`, `tiff.rs:142–144`: BYTE, SHORT or LONG, plus LONG8 in BigTIFF),
    Rating and RatingPercent (`unsigned`, `tiff.rs:160–169`: BYTE, SHORT, LONG and IFD, plus LONG8 and IFD8, kept in
    full, since 16895197), and the XMP tag 700.
  - Every tag keeps its last readable entry: an entry that doesn't read is skipped, so an earlier readable one stands
    (`Exif::keep`, `exif.rs:104–109`). The values a later entry replaced are kept in `superseded`, in entry order
    (`exif.rs:57–58`); nothing reads them yet.
  - The Exif IFD (tag 0x8769) gives DateTimeOriginal and DateTimeDigitized (`exif.rs:76–84`).
  - There is no IFD1, no SubIFD and no visited set. Only IFD0 and the Exif IFD are walked.
- **Dates:** `exif_date` (`crates/metadata/src/exif.rs:11`, in pg-metadata, `pub(crate)`) applies kamadak-exif's date
  rules.
  - `parse_iso_date` (`crates/model/src/date.rs:12`, in `pg-model`) is also used outside metadata: by
    `crates/video/src/lib.rs:154` and `:156` (video creation time). `Day::parse` is called from the client input
    (`crates/client/src/input.rs:111`), the desktop filters (`crates/desktop/src/filters.rs:136`),
    `crates/model/src/query.rs:240`, the `Day` deserialiser (`crates/model/src/date.rs:101`), and tests in
    `pg-catalog`, `pg-query`, `pg-model` and `pg-wording`.
  - `wall_clock_seconds` (`crates/model/src/date.rs:38`) is also used by `Day`, the video tests and `exif_date`.
- **The container walk:** `Container::walk` (`crates/metadata/src/container.rs:31`; `struct Container` at `:24`).
  - JPEG: APP1 only. EXIF is the first segment that starts `Exif\0\0` (`jpeg_exif`, `crates/container/src/lib.rs:117–118`;
    taken at `container.rs:40–42`). XMP packets are read last standard packet first (`container.rs:50`), then the
    extended packets (`extended_packets`, `:51`), with no HasExtendedXMP check.
  - PNG: `eXIf` and iTXt only (`container.rs:60–61`). A compressed iTXt packet is inflated within a per-file budget
    (`max_inflated`, `container.rs:142`).
  - WebP: `EXIF` (with or without the `Exif\0\0` prefix, `webp_exif`, `crates/container/src/lib.rs:839–840`) and
    `XMP` with its trailing space (`container.rs:81–82`).
  - GIF: the XMP packet counts only if the walk reaches the trailer (`container.rs:100–104`).
  - TIFF: the whole file is the EXIF block (`container.rs:106`).
- **The orientation-only read:** `pg_metadata::orientation` (`crates/metadata/src/orientation.rs:14`, re-exported from
  `lib.rs:22`) does its own walk through `exif_block` (`orientation.rs:22`).
  - Callers: `crates/edit/src/lib.rs:54` (test code only), `crates/edit/src/rotate.rs:39` and `:48` (the production
    read-back that Step 0 is about), `crates/render/src/display.rs:39` and `:53` (`crates/render/src/maker.rs`
    reaches them through `display::decode` and `display::decode_into`), and `crates/writeback/src/lib.rs:605`.
- **`Meta::read`** (`crates/metadata/src/lib.rs:57`; it takes a `&mut MetaScratch` that keeps the buffers from file to
  file). Callers: `crates/analysis/src/analyze.rs:125`, `:141` and `:158`, `crates/writeback/src/lib.rs:581`, the
  rating-write tests in `crates/exiftool/src/lib.rs` (`:575`, `:623`, `:680`), the tests in
  `crates/library/tests/writeback.rs` (lines 180, 212, 297, 363, 382, 491, 612 and 760), and
  `crates/analysis/examples/stage_times.rs:41`.
- **Rating:** read in Windows' order since 41ca393c (`crates/metadata/src/lib.rs:85–89`): XMP `MicrosoftPhoto:Rating`
  (a percent), EXIF `RatingPercent`, XMP `xmp:Rating` (stars), EXIF `Rating` (stars). The first field the file holds
  decides, and a 0 there is unrated and hides the fields below it. Stars above 5 read 5, a percent above 99 reads 99,
  and a percent maps to stars by Explorer's bands 1–12, 13–37, 38–62, 63–87 and 88–99 (`stars_of_percent`,
  `crates/metadata/src/xmp.rs:134`). XMP's -1, and any value below 0, reads as no rating (`xmp_stars`,
  `xmp.rs:120–121`). The commit checked 952 cases against Windows (JPEG and TIFF); PNG and WebP showed no rating in
  Windows, so they are not covered.
- **XMP:** parsed by quick-xml (`xmp_properties`, `crates/metadata/src/xmp.rs:55`). Outside the library code, quick-xml
  is also declared in `fuzz/Cargo.toml` (its quick-xml entry) on the same fork rev, and `deny.toml` names it in the
  `[bans]` quick-xml entry and the `[sources]` `allow-git` list. The workspace patches it to the `darktorres/quick-xml`
  fork (the quick-xml entry in `[patch.crates-io]`).
- **Shared chunk walkers:** `png_chunks`, `webp_chunks`, `gif_blocks`, `gif_header_len`, `GifBlock` and `Chunk` live
  in `pg-container` (`crates/container/src/lib.rs`). They are used by pg-edit's rewrite code
  (`crates/edit/src/rewrite/{chunk,gif,png,webp}.rs`), and by pg-metadata's `orientation.rs` and `container.rs`.
  `rotate.rs` and `crop.rs` use them only in test code. The rewrite users rewrite files, so their semantics must not
  change.
- **The in-place patch:** `patch_orientation` (`crates/metadata/src/orientation.rs:60`) sets every IFD0 orientation
  entry the reader takes, each at its own width (`tiff_orientation_patches`, `orientation.rs:35`). It is no longer
  limited to the first SHORT×1 entry; that changed in cb123b9 (2026-11-08), see Step 0.
- **Tests:** a test builder, `tiff_block` (`crates/metadata/src/lib.rs:196`, in the tests module; it delegates to
  `tiff_block_as`, defined at `lib.rs:201`).
- **`IMPORT_VERSION` is 1** (`crates/model/src/versions.rs:13`, re-exported at `crates/analysis/src/analyze.rs:26`).
  Commit 7deed676 ("Versions: key version, mark policy, writer guards, reset to 1 (phase 3)") reset it from 2; its
  message says the next open of a catalog marks every analysis pending and re-analyses the library in place. The 6
  files from 29c4d5e's run, which gained a Rating of 4, aren't settled yet.

## Decisions

The defaults below stand unless a later decision changes them.

1. **Faithful everywhere, ExifTool's data-losing quirks included.** For example, ExifTool reads one JPEG segment ahead
   (`ExifTool.pm` 7361–7428), so a complete EXIF segment followed by a truncated one yields no EXIF. Any deviation
   would make "zero mismatches" unmeasurable, so each one is a later decision, made per case. This binds the parser
   (the store and its occurrences). The app's selection rules on top of it are the only layer that is not ExifTool's
   (decision 4).
2. **Selection: ExifTool's own rules for each tag, with a group filter where the app needs a specific source. The
   rating is the app's own rule: Windows' order (41ca393c).**
   - **Orientation is `EXIF:Orientation`.** ExifTool's plain `Orientation` also merges XMP `tiff:Orientation`, in
     file order. Other viewers turn images by EXIF only, so taking XMP's would turn images they show upright.
   - **Date created is `XMP-photoshop:DateCreated`** (IPTC is out of scope).
   - The dates go by tag name: `DateTimeOriginal`, `CreateDate`.
   - **The rating fields go by group**, since the app tells them apart and ExifTool's names merge them:
     `XMP-microsoft:RatingPercent` (MicrosoftPhoto:Rating, `Microsoft.pm` 248–249), `IFD0:RatingPercent`,
     `XMP-xmp:Rating` and `IFD0:Rating`. ExifTool's plain `RatingPercent` merges the MicrosoftPhoto and EXIF copies
     (MicrosoftPhoto is priority 1, EXIF priority 0), and its plain `Rating` merges XMP and EXIF (XMP wins), so the
     plain names would hide the field Windows takes.
   - With a group filter, ExifTool keeps that group's copies and re-applies its priority and file-order rule
     (`ExifTool.pm` 5390–5420). The oracle asks for the same names.
   - Within a name and group, `FoundTag` decides (`ExifTool.pm` 9507–9687):
     - EXIF Orientation, Rating and RatingPercent are priority 0: the first one found wins, unless a later one is in
       the priority directory.
     - The EXIF dates are priority 1: the last one found wins, except JPEG IFD1, which is low priority. The XMP copies
       in the exif and tiff namespaces are priority 0 (`XMP.pm:1992` and `:2462`; `CreateDate` at `:1053`).
     - EXIF DateTimeOriginal and CreateDate beat their XMP copies.
   - Across names, the app picks:
     - Rating, in Windows' order: `XMP-microsoft:RatingPercent`, then `IFD0:RatingPercent`, then `XMP-xmp:Rating`,
       then `IFD0:Rating`. The first one the file holds decides (step 3 has the scales).
     - Date taken: `DateTimeOriginal`, then `CreateDate`, then `XMP-photoshop:DateCreated`.
   - Differences from HEAD, counted in step 6. The parser is written to match ExifTool; step 1's harness has not run,
     so that match is the target, not a result. These are the places where the app's result, or HEAD's, differs:
     - Within XMP, the parser follows ExifTool: a priority-1 property found later wins (the extended packet over the
       standard one, and later packets over earlier ones). HEAD takes the first packet's value of each
       (`crates/metadata/src/lib.rs:68–84`). Its JPEG walk reads the last standard packet first
       (`container.rs:50`), so for JPEG the two agree on standard packets; for WebP and PNG they differ.
     - The app's date rule is policy: it selects XMP only through `XMP-photoshop:DateCreated`. HEAD also reads XMP
       `exif:DateTimeOriginal` and `xmp:CreateDate` (`crates/metadata/src/lib.rs:46–48` and `:82–83`). The parser
       still reads those tags exactly as ExifTool does; the app does not select them.
     - Orientation is read from more places: IFD1, SubIFDs, several EXIF blocks, and the new carriers. The parser
       reads them as ExifTool does; the app's use of them is the HEAD difference.
     - The rating fields are read from more places too (the same carriers, and IFD1 and SubIFDs where ExifTool finds
       them), each under its group. HEAD reads EXIF Rating and RatingPercent from IFD0 only.
     - Extended XMP: HEAD assembles the extended packets with no HasExtendedXMP check (`container.rs:51`); the port
       assembles only the GUID that HasExtendedXMP names (`ExifTool.pm` 7614–7632).
     - GIF: HEAD counts an XMP packet only if the walk reaches the trailer (`container.rs:100–104`). The port counts
       it before a truncation, as ExifTool does: the installed ExifTool reads XMP Rating 3 from a synthetic truncated
       GIF (checked on 13.60; re-check on 13.64 when step 2's GIF module lands).
     - Duplicate IFD0 tags: ExifTool keeps the first Orientation, Rating or RatingPercent that reads (Orientation is
       `Priority => 0` at `Exif.pm:691`; Rating and RatingPercent are `Avoid => 1` at `Exif.pm:1521–1545`; the
       duplicate rule is at `ExifTool.pm` 9573–9623). HEAD keeps the last readable copy (`Exif::keep`,
       `crates/metadata/src/exif.rs:104–109`). A crafted JPEG with two Orientation entries (6, then 8) reads 6 in
       ExifTool and 8 at HEAD; two Rating entries (2, then 5) read 2 and 5. The HEAD test
       `duplicate_orientation_keeps_the_last_readable_entry` (`crates/metadata/src/lib.rs:480`) asserts last-wins and
       will change.
     - Date alone or short EXIF date: HEAD returns none for an EXIF date shorter than 19 bytes or without the
       separators at 4, 7, 10, 13 and 16 (`exif.rs:12`, `:15`), so `2020:01:02` falls through to the next date
       source. Step 3's "a date alone means midnight" is a change from HEAD.
     - TIFF magic number: HEAD requires 42 (43 for BigTIFF; `tiff.rs:46–50`). ExifTool only warns, for APP1
       (`ExifTool.pm:8677`), so a crafted APP1 with magic 0x2b reads Orientation 6 in ExifTool.
     - JPEG EXIF match and joined segments: HEAD requires `Exif\0\0` at offset 0 (`crates/container/src/lib.rs:118`)
       and takes only the first APP1 that matches (`crates/metadata/src/container.rs:40–42`). ExifTool matches
       `/^(.{0,4})Exif\0./is` (`ExifTool.pm:7777`) and joins multi-segment EXIF.
     - PNG `eXIf` prefix: HEAD reads the chunk with no `Exif\0\0` strip (`container.rs:60`). A crafted PNG with that
       prefix before the TIFF reads Orientation 6 in ExifTool and nothing at HEAD.
     - New carriers (PNG `zxIf` and `tXMP`, raw profiles, WebP `XMP\0`) also feed the rating fields and the dates,
       not only Orientation.
   - ExifTool writes our ratings (writeback, into every rating field the file holds since 13a41bdc), and the reader
     checks each write by reading it back (`crates/writeback/src/lib.rs:581`). The port reads each field as ExifTool
     does; which field decides stays Windows' order.
3. **XMP is ported too, and quick-xml goes.** ExifTool's XMP reader is a tolerant scanner, not an XML parser:
   - An undeclared `xmp:` prefix still matches.
   - A wrong trailing slash on a URI is fixed. HEAD reads the MicrosoftPhoto namespace with and without the slash,
     since Windows reads both (41ca393c).
   - A broken element loses only its own nesting level.
   - Dates are normalized as they are read.
4. **Scope:** the whole of ExifTool 13.64, read and write, per the design. This plan's slice is the read side for five
   containers and six fields (see Context). This reverses the earlier out-of-scope list (IPTC, makernotes, GPS,
   writing). Frame counting stays ours: it is app logic, not ExifTool's. The app's selection rules (decision 2: the
   group filters, the date order and Windows' rating order) are the only layer that is not ExifTool's.

## Steps

### 0. Orientation patch that always matches the read (landed in cb123b9, 2026-11-08; the rest is open)

The bug this step was written for: at base 4e1ad4d the patch changed only the first SHORT×1, so a LONG orientation or
a second copy left the old tag in place after the pixels were turned, and the image showed turned twice. HEAD fixes
it:

- `patch_orientation` (`crates/metadata/src/orientation.rs:60`) sets every IFD0 orientation entry the reader takes,
  each at its own width.
- The `rewritten` closure in `rotate.rs` (`crates/edit/src/rotate.rs:43–56`) reads the result back (`:48`) and fails
  with an error when the tag still reads wrong (`:49–54`), so a mismatch is no longer a silent double turn.
- Tests for the LONG and two-copy cases exist.

What is still open in this step, as one commit (each item re-checked at b33cf639):

- `patch_orientation` still returns `Some` without a read-back (`orientation.rs:60–77`). The contract to add: it
  returns `Some` only if `metadata::orientation` of the result is `o`, and `None` otherwise.
- The writeback patch path (`crates/writeback/src/lib.rs:594–595`) has no read-back check; only the ExifTool path
  after it checks (`:605`). The patcher and the reader share `orientation_kind` (`tiff.rs:142`; the reader calls it at
  `exif.rs:68`), so this is defence in depth, not a double turn that occurs today.
- The `rewritten` closure should turn a `None`, when the original had an orientation other than 1, into
  `Outcome::Orientation(total)`: the pixels are left as they are and the tag is set (patch, else ExifTool). At HEAD it
  falls back to the unpatched output (`:47`) and then fails at the read-back.
- Tests for the `None` path and for the `Outcome::Orientation` fallback.
- A test for the read-back error path in `rotate.rs` (`:49–54`).
- The `meta_read` bench (a camera JPEG with EXIF, XMP and a thumbnail IFD), so the baseline exists before the port
  (see Verification). It does not exist yet.

The port (step 2) keeps the same contract: it patches every in-place `EXIF:Orientation` occurrence, and the read-back
moves into the patcher, so the patcher returns `None` rather than an unchecked `Some`.

### 1. Oracle harness (new: `crates/library/examples/metadata_oracle.rs`; not written yet)

- **ExifTool driver:** reuses `pg_exiftool::ExifTool` (`crates/exiftool/src/lib.rs:96`, stay-open: started at
  `:121`, closed in `Drop` at `:378`). The program is a `Program` (`:76`): `Program::from_env` (`:80`) reads
  `PG_EXIFTOOL` through `pg_platform::env`, else `exiftool` on PATH. The writer calls it at
  `crates/writeback/src/lib.rs:577–579` and `:599–600`.
  - It gains `pub fn read_tags(&mut self, files, args) -> Result<String>` beside the private `exec` (`:186`), which
    takes the argument route for names the argument file can't carry. `rating_fields` (`:196`) already runs a
    `-j -G1` read and can share it.
- **Command, no `-fast`:** `-n -a -G1 -args -EXIF:Orientation -XMP-microsoft:RatingPercent -EXIF:RatingPercent
  -XMP-xmp:Rating -EXIF:Rating -DateTimeOriginal -CreateDate -XMP-photoshop:DateCreated -HasExtendedXMP`.
  - The first task: confirm how a batch separates files and keeps duplicates in order. `-args` keeps duplicates;
    JSON may merge them. If needed, run one file per `-execute`.
  - A second run without `-a` gives the winners and their groups.
- **Inputs:** N ExifTool processes run in parallel over a list: a folder or a list file
  (a directory walk that writes a fixed list file; `es.exe` is not used).
- **Cache:** JSONL in `<data root>\scratch_exif_oracle\`, keyed by path, size and mtime, and by the ExifTool version
  (`-ver`), so a reference change empties it.
- **Our side:** dumps every occurrence (name, group1, the value as `-n` prints it, file order) and the winners,
  through step 2's recording sink.
- **Report:** mismatches by kind, with paths and ExifTool's warnings for each file. `--min` writes the smallest
  reproducing input.
- **Safety:** read-only on the library. It runs on E: only after the owner approves it, since E: is the library HDD.

### 2. The port (in the `pg-et-*` crates, per design §1.2: `found.rs`, `jpeg.rs`, `png.rs`, `webp.rs`, `gif.rs`, `irb.rs`, `tiff.rs` and `xmp.rs` are the modules these crates hold)

None of these crates exists at b33cf639.

This section is the slice of design phases P1 to P5, plus the PNG and GIF parts of P6a and the WebP part of P7a
(`docs/plans/EXIFTOOL_PORT_DESIGN.md`). The modules listed here are that slice, not the whole port.

Crate placement: the port is built in the `pg-et-*` crates from the start, as design §1.2 places it. `pg-metadata` is
not reused as the reader (design §1.3). `pg-metadata` calls the new crates, and its old reader is removed at step 6.

Every ported function cites its ExifTool file and line, at tag `13.64`.

- **`found.rs`: the `FoundTag` store.**
  - Fixed slots for our names, each with priority, group0/group1, document and file order.
  - The priority directory: the first IFD with SubfileType 0 or OldSubfileType 1. So 0xFE and 0xFF are read too.
  - Low-priority directories: PreviewIFD; IFD1 for JPEG (`ExifTool.pm:7328`) and for ARW files, which are TIFF-based
    (`ExifTool.pm:8740`); and all tags (`*`) for JPEG Photoshop data outside APP13 (`Photoshop.pm:1042`).
  - Promotion of the existing value to priority 1 for the comparison. The new value wins when new ≥ old.
  - Group-filtered selection.
  - A `Sink` trait: the import's sink keeps winners and allocates nothing; the oracle's sink records everything.
- **`tiff.rs`: `DoProcessTIFF` + `ProcessExif`, read path** (`ExifTool.pm` 8597–8832 for the read path, which ends at
  its return after ProcessDirectory and the trailers; 8833–8975 is the write path and is not cited as read;
  `Exif.pm` 6279–7241).
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
      directory (`Exif.pm` 6471–6478), except for Sony ILCE, where the first entry is skipped instead.
    - The size cap. "Suspicious offset" (below 8, or inside the entry block).
    - The excessive-count skip.
    - The warning count, which aborts after 11 (`Exif.pm` 6456). Only the counted warnings are modelled.
  - Values (`ReadValue`, `ExifTool.pm` 6290), typed and printable as `-n` strings:
    - Lists are joined with spaces. Rationals are computed as ExifTool does.
    - Strings are cut at the first NUL, with trailing spaces kept. Count 0 is an empty value.
    - 0x02bc is forced to `undef`.
  - Traversal:
    - ExifIFD (0x8769), with the "Wrong format" and "Bad value" pointer rules.
    - SubIFD (0x14A, at most 10) and the IFD1… chain.
    - A visited-address set, as ExifTool's PROCESSED check (`ExifTool.pm` 9120–9127): a second `ProcessDirectory` on
      an address already registered is skipped with a warning and contributes no tags, so it can change values.
      Implement the same first-registration order and warning. ExifTool applies the skip inconsistently (an IFD chain
      revisit stops a cycle, but a directory first read as ExifIFD is read again as IFD1), and the port matches that.
  - Our tags are read in any IFD, with group1 set to that IFD's name.
- **Container walks: separate from the shared walkers.**
  - `png_chunks`, `webp_chunks` and `gif_blocks` (in `pg-container`) keep their current semantics for pg-edit's
    rewrite code.
  - The ported walks are their own functions in `jpeg.rs`, `png.rs`, `webp.rs` and `gif.rs`. They may share
    low-level helpers.
- **`jpeg.rs`: `ProcessJPEG`** (`ExifTool.pm` 7275–8541: the scan and SOS stop at 7349–7693; the APP1 block at
  7774–7955, with the EXIF match at 7777, the XMP match near 7939 and the ExtendedXMP store at 7860–7883; the APP13
  dispatch at 8426–8455).
  - The scan: junk and fill bytes skipped, one segment read ahead, stop at SOS.
  - APP1 EXIF:
    - The match is `/^(.{0,4})Exif\0./is`.
    - Multi-segment EXIF is joined into one block. That join is one of the owned buffers here.
  - APP1 XMP:
    - The standard header, or the loose match.
    - ExtendedXMP chunks are kept by GUID and offset and assembled at SOS. Only the GUID named by the winning
      `HasExtendedXMP` is assembled.
  - APP13 Photoshop IRB.
- **`png.rs`** (`PNG.pm` 1412–1651, 1157–1283, 1341–1360; unchanged from 13.60).
  - EXIF:
    - `eXIf` in any case, and `zxIf`.
    - The `Exif\0\0` prefix rule.
  - XMP:
    - `XML:com.adobe.xmp` in tEXt, zTXt or iTXt. An iTXt with a language tag doesn't count.
    - `tXMP`.
  - The hex profiles `Raw profile type exif/APP1/xmp`.
  - Chunks after IDAT and after IEND are read. CRCs are not checked.
  - Inflation uses miniz_oxide.
- **`webp.rs`** (`RIFF.pm` 557–587, 2028–2220).
  - The `EXIF` chunk, with or without the `Exif\0\0` prefix.
  - `XMP` followed by a space, and `XMP` followed by a NUL byte: four-byte chunk ids with a trailing space and a
    trailing NUL. ExifTool warns "Incorrect XMP tag ID" on the NUL variant and still reads it.
  - The scan runs to EOF, not to the RIFF size. A truncated chunk stops it.
- **`gif.rs`** (`GIF.pm` 414–470; the 13.64 lines added in that range are write-side, see "13.60 to 13.64").
  - XMP: the application extension, length bytes kept, trimmed at the xpacket end. XMP found before a truncation
    counts.
  - The frame count stays all-or-nothing.
- **`irb.rs`: Photoshop IRB** (`Photoshop.pm` 254–268). 0x0422 EXIF and 0x0424 XMP, in JPEG APP13 (`Photoshop 3.0\0`,
  multi-segment) and in TIFF 0x8649.
- **`xmp.rs`: `ProcessXMP`, `ParseXMPElement` and `FoundXMP`, only what our properties need** (`XMP.pm` 3099–3152,
  3464–3475, 3736–3768, 3882–4291, 4384–4717).
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
- **Rating, in Windows' order (decision 2), as HEAD reads it since 41ca393c:**
  - The first of `XMP-microsoft:RatingPercent`, `IFD0:RatingPercent`, `XMP-xmp:Rating` and `IFD0:Rating` that the
    file holds decides. A 0 there is unrated, and the fields below it are not read.
  - Stars: a real number, rounded; above 5 reads 5; -1 and anything below 0 read as no rating.
  - Percents: above 99 reads 99, then Explorer's bands 1–12, 13–37, 38–62, 63–87 and 88–99 give 1 to 5 stars.
  - The parser reports ExifTool's raw value for each field; these rules apply on top of it.
- **Orientation:** the winning `EXIF:Orientation` as an integer from 1 to 8, else 1.
- **Date taken:** one parser for ExifTool's form, `YYYY:MM:DD[ HH:MM[:SS[.f]]][zone]`.
  - EXIF strings arrive raw; XMP strings are already converted.
  - Blanks and impossible dates fail through `wall_clock_seconds`. A date alone means midnight.
  - `exif_date` goes. `parse_iso_date` and `wall_clock_seconds` (all in `pg-model`'s `date.rs`) stay: `Day`
    (the desktop's date filters) and videos use them.
- **`patch_orientation`:** step 0's contract on top of the store's occurrences. It covers the EXIF block, IRB EXIF
  and multi-segment EXIF where the bytes are in place. A hex or compressed carrier can't be patched, so it returns
  `None`, and ExifTool writes.
- **Callers and signatures** are unchanged.

### 4. Tests (committed)

- One unit test per ported rule, built with `tiff_block` grown into a builder (TIFF/IFD, plus JPEG, PNG, WebP, GIF and
  IRB wrappers). Each test names the ExifTool line it covers.
- One test runs every builder case through ExifTool and compares. It is skipped without ExifTool, as the tests in
  `crates/library/tests/writeback.rs` are (`:128`).
- The Windows rating cases of 41ca393c stay as tests of step 3's rule.
- Fuzzing: a `cargo-fuzz` target on `Meta::read`. `metadata_read` (`fuzz/fuzz_targets/metadata_read.rs`) is the only
  fuzz target that calls `Meta::read`; the other targets cover other entry points, and per-format targets are new
  work. Fuzzing runs in the Linux container, not on MSVC. Each target must never panic, never hang, and stay within a
  fixed amount of work per input.

### 5. Differential runs, until zero raw mismatches

1. ExifTool's test images: `t/images` in the clone at `D:\GitHub\exiftool`, at tag `13.64` (195 files; `Mini.avif` is
   new since 13.60). The clone's working tree is at 13.64 on `master`; check `git describe --tags` before a run, since
   a pull moves it past the reference. The 13.60 tarball that was at `D:\exiftool-13.60\` is gone.
2. `<data root>\datasets\pinterest` (189k files, mostly stripped re-encodes).
3. About 1M JPEG/PNG/WebP/TIFF files from E:, spread across folders, chosen with Everything. Only after owner approval.
4. A mutation corpus: EXIF and XMP blocks from 1–3, mutated and wrapped in minimal files.
   - Mutations: offsets, counts, formats, byte order, truncation, cycles, junk between segments, duplicate blocks,
     broken XML, namespace variants.

Each mismatch is fixed in the port, and its minimized input becomes a builder test.

### 6. Switch-over, without losing ratings

- **App-level diff:** the old `Meta` against the new one on the default catalog's files. Today's reader is kept as
  `metadata::legacy` until this step ends.
- **Ratings:** `AnalysisWriter::sync_file_rating` (`crates/catalog/src/writers.rs:558`, called from `store_analysis`
  at `:499`) adopts any file rating that differs from `file_ratings` as an external change, unless one of our rating
  writes is queued (`:561–562`). A reader that reads an unchanged file differently would overwrite the catalog rating.
  - So for each file where the old read equals `file_ratings` and the new read differs, `file_ratings` is set to the
    new value. A read of none deletes the row (`set_file_rating` with `None`, `crates/catalog/src/queue.rs:361`; the
    sync path's own delete is `delete_file_rating`, `writers.rs:571`). Then `sync_file_rating` finds no difference and
    changes nothing, so `ratings` is untouched. This order is what keeps `ratings` fixed: this pass must run before any
    re-analysis.
  - This is a one-off pass in the switch-over commit, after a backup of the catalog. It runs before any re-analysis,
    so ratings can be compared before and after it.
  - Since the rating rule is Windows' order at HEAD and in the port, the pass only meets files whose fields the port
    finds in new places (decision 2's carriers, IFD1, SubIFDs, duplicates).
  - 29c4d5e's 6 files are the exception. Their stored read came from kamadak-exif, so `file_ratings` has no row that
    holds 4, and the pass above skips them. The re-analysis sync then adopts the new reader's value if it is 4
    (`set_rating`, `writers.rs:566`). That writes `ratings`, and it is the one intended change to `ratings`.
- **Orientation and date: the re-analysis.** The switch-over changes what is read, but `IMPORT_VERSION` stays 1: no
  version is bumped before the first release (`crates/model/src/versions.rs`). So the re-analysis is asked for, not
  detected: after the switch-over, every file of the development catalog is marked pending
  (`Catalog::mark_pending`, `crates/catalog/src/work.rs`, the call made for a fix to what the import reads), or the
  catalog is made again, and the next import pass re-analyses them all. (`set_import_version` still re-marks a catalog
  whose stored mark is another build's, as it does after the release.)
  - The `file_ratings` pass runs before the files are marked pending.
  - 7deed676's reset to 1 already marked the library pending once. If a build at 1 has opened the catalog since, the
    pass must be checked against the ratings that the sync stored then (`sync_file_rating` has run on those files).
  - The 6 files of 29c4d5e are covered by the exception above.
- **Then remove** `legacy` and quick-xml from the workspace `Cargo.toml` (including its quick-xml fork patch),
  `crates/metadata/Cargo.toml`, `fuzz/Cargo.toml` (its quick-xml entry) and the quick-xml entries in `deny.toml`
  (`[bans]` and `[sources]`). The lock files (`Cargo.lock` and `fuzz/Cargo.lock`, each with a quick-xml package entry
  and pg-metadata's dependency on it) change in the same commit, since `just test` runs `--locked`. Then `cargo deny`
  and `cargo machete` check the result.

### 7. Rating-write retry (dropped)

This step planned a shared `write_tag` helper that retried a failed write with ImageRater's rebuild
(`-all= -tagsfromfile @ -all:all -unsafe -icc_profile`). It is dropped. 13a41bdc (2026-11-23) removed the rebuild
fallback from `write_rating` on purpose: the rebuild can drop any metadata ExifTool doesn't copy back, and the
read-back checks only the rating, so a lossy rebuilt file would pass. At b33cf639:

- `write_rating` (`crates/exiftool/src/lib.rs:216`) reads which rating fields the file holds (`rating_fields`,
  `:196`), sets `XMP:Rating` and each held field on its own scale, and fails with no retry.
- `write_orientation` (`:258`) doesn't retry either. When ExifTool won't update a file, `process` keeps the turn in the
  catalog only (`keep_turn_in_catalog`, `crates/writeback/src/lib.rs:755`).
- `copy_tags` (`:241`, crops) writes its destination in place (`-overwrite_original`), with no retry.

A file ExifTool won't update stays a failed write (rating) or a turn kept in the catalog (orientation). A later
fallback that keeps every tag would be its own plan.

### 8. Docs

- A measurement entry for the port: the rules ported, mismatch counts per corpus and run, and every behaviour change,
  including step 0's bug. It goes in a new file under `docs/eval/`, where ff8b152 moved the other measurements
  (`docs/FINDINGS.md` no longer exists).
- Module docs name ExifTool 13.64 as the source.
- **License:** settled in the design's D14 (Artistic 1.0 or GPL-1+, confirmed 2026-11-13). The repo has no licence file
  today. Step 8 adds a note in `crates/README.md` that cites D14 and says a legal review comes before any public
  release; no such note exists at b33cf639.

## Order and commits

1. Step 0's remaining items first, on today's reader (the rest landed in cb123b9).
2. Step 1, with stubs on our side.
3. Step 2 one module at a time: store, TIFF, JPEG, PNG, WebP, GIF, IRB, XMP. Each with its tests and a harness run on
   t/images and pinterest.
4. Steps 3, 4, 5, 6 and 8.

One commit per verified step. Before building, check that no other build is running.

## Verification

- **Every commit:** `just lint` and `just test` (`just test` runs cargo with `--locked`). `just lint` already runs fmt,
  deny and machete; the explicit `cargo fmt --all --check`, `cargo deny check` and `cargo machete crates` are for a run
  without `just`.
- **Acceptance:**
  - Zero raw mismatches against ExifTool 13.64 on corpora 1, 2 and 4, and on 3 if approved.
  - Whole port, later (not this slice): every design phase's done test passes (design section 7).
  - Fuzzing clean for a fixed time per target.
  - The app-level diff explained file by file.
  - Step 0: `patch_orientation` returns `None` unless the result reads back as `o`; the `None` path, the read-back error
    path in `rotate.rs` and the `Outcome::Orientation` fallback each have a test that fails before the change.
  - Step 3: the Windows rating cases of 41ca393c still pass on the new reader.
  - Step 8: the measurement entry exists and names each behaviour change.
  - `ratings` identical before and after the file_ratings pass in step 6. The 6 files of 29c4d5e are the one case
    decided by the open step 6 choice; their adoption is the one intended change to `ratings`.
- **Measured once at the end:**
  - Instruction counts with gungraun (`just perf`, which benches the committed HEAD), before and
    after.
    - The `SAMPLE` constant (`crates/bench/benches/ir/linux.rs:23`) reads `PG_BENCH_JPEG` at build time
      (`option_env!`), and `analyze_jpeg` (`:52`) benches it.
    - A new `meta_read` bench uses a camera JPEG with EXIF, XMP and a thumbnail IFD. It is added in step 0's commit
      so the baseline exists.
  - A dhat import profile: no allocation at the metadata sites for files without inflated, joined or extended
    metadata.
