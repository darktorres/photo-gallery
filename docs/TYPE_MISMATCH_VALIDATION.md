# Type mismatch findings: open items

Seven findings from the type-mismatch review are still open at HEAD `362888c`. All are rated low. Each entry gives the current location, the defect, its impact, and the planned fix. Fixed and stale findings are not listed.

## 1. Exif IFD pointer narrowed to 32 bits (`crates/metadata/src/tiff.rs:155-163`)

`uint` narrows the value to `u32`, and returns `None` for anything above `u32::MAX`. For a BigTIFF, the Exif IFD pointer is an 8-byte offset, and `exif.rs:60` reads it through `uint`. Other offsets in the same reader are `usize` (`offset_at`).

**Impact:** a BigTIFF whose Exif IFD sits above 4 GiB loses `DateTimeOriginal` and `DateTimeDigitized`. The date is lost only when the XMP fallback (`crates/metadata/src/lib.rs:95`) also has none. `crates/analysis/src/analyze.rs:118` stores no date in that case.

**Fix:** read IFD pointers as `u64` (or `usize`) through `offset_at`, not through `uint`.

## 2. User rating type admits 0 (`crates/model/src/rating.rs:24-29`)

`Rating` is shared by user ratings (1 to 5) and file ratings (0 to 5), so `Rating::new` accepts 0. `Edit::rate` takes `Option<Rating>` (`crates/app/src/edit.rs:27`), so the API accepts a 0 user rating.

**Impact:** no data defect today. The desktop maps 0 stars to `None` before calling `rate` (`crates/desktop/src/gallery/edits.rs:77`). The `ratings` table's `CHECK (rating BETWEEN 1 AND 5)` (`crates/catalog/src/schema.rs:647-649`) rejects a stored 0, so a 0 errors rather than writing. The gap is that the type enforces none of this.

**Fix (planned):** split `Rating` into two types.
- `UserRating` (1 to 5) for the `ratings` table, `Write::Rating`, and `Command::Rate`. Its constructor rejects 0. `Grid::Rate(stars)` (`crates/client/src/controller.rs:337`) then fails for 0 at the boundary, and "0 stars clears the rating" becomes an explicit `None` branch in the desktop instead of a `filter`. Call sites: `crates/desktop/src/gallery/edits.rs:77`, `crates/catalog/src/writers.rs:1071`, `crates/catalog/src/browse/fixtures.rs:82`, and the user-rating read at `crates/catalog/src/browse/read/grid.rs:70`.
- `FileRating` (0 to 5) for `file_ratings` and metadata, where 0 means "explicitly unrated" (`crates/metadata/src/lib.rs:41`). The import read-back at `crates/catalog/src/writers.rs:373` and the metadata constructor at `crates/metadata/src/lib.rs:94` use it. Rejecting 0 here would make every import rewrite the `file_ratings` row of an explicit-0 file.

Done when `Rating::new` no longer exists, `Edit::rate` takes `Option<UserRating>`, the desktop filter is gone, and the tests in `writers.rs` and `fixtures.rs` build with the new types. The wire format and the `CHECK` constraints stay unchanged.

## 3. Last-file preference uses a row id and a `-1` sentinel (`crates/app/src/prefs.rs:34-35`)

`Prefs.last_file` is an `Option<FileId>`, but on disk it is `-1` for none (`prefs.rs:65-79`). The Qt bridge also passes an `i64` with `-1` for none (`crates/desktop/src/settings.rs:40-41, 73-77`).

**Impact:** a catalog rebuild reuses row ids, so the startup selection can pick a different file. A stale id that names no file selects nothing. The sentinel itself cannot collide with a generated id.

**Fix (planned):** persist the file's identity, not its row id. Row ids are local to a catalog, and `FileIdentity` (volume plus NTFS file ID) already keeps a file's row across moves and renames.
- `Prefs.last_file` becomes `Option<FileIdentity>`. At startup, resolve it to a `FileId` with one lookup on `files(volume, file_ref)`, which has a UNIQUE index. No match means nothing is selected.
- Disk format: write `null` for none. Read a legacy negative number as none. Read a legacy non-negative number as a row id, resolve it once, and re-save it as an identity.
- Qt bridge: `settings.rs` stops passing an `i64` with `-1`. The property carries an optional value, empty for none. Call sites: `crates/desktop/qml/Main.qml:113` (assigns the sentinel) and `Main.qml:224` (compares it through `galleryModel.indexOf`).
- A displaced row (`files.volume` NULL) has no identity to remember, so it selects nothing.
- Tests: identity round-trip; a legacy `-1` reads as none; a legacy row id resolves once; a rebuild that reuses a row id selects nothing for the other file.

## 4. CLI accepts thumbnail height 0 (`crates/cli/src/args.rs:85-88`)

`Thumbs { height: u32 }` takes any value. The dispatch is at `crates/cli/src/lib.rs:228-234`, and neither the parser nor `pg_thumbs::run` (`crates/thumbs/src/lib.rs:172`) checks the range. The valid range, 16 to 2048, is enforced only in the client (`crates/client/src/image.rs:42, 58-60`).

**Impact:** a deliberate height of 0 is accepted. The effect has not been traced: the expected result is a per-file encode failure with no top-level error.

**Fix (proposed):** validate the height against the client's `THUMB_HEIGHT` range in the CLI parser, or reject it in `pg_thumbs::run`.

## 5. Megabyte buffer size shift can wrap (`crates/cli/src/args.rs:53`)

`opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1) << 20)` shifts a `usize` without a check. Any value of 2^44 MB or more wraps to a small budget, which serialises reads through `Pool::new` (`crates/jobs/src/pipeline.rs:118`).

**Impact:** serialised reads for an absurd argument. No data or hang impact.

**Fix (proposed):** use `checked_mul(1 << 20)` and report an error on overflow.

## 6. XMP packets with a UTF-16 or UTF-32 BOM are decoded as UTF-8 (`crates/metadata/src/xmp.rs:62`)

`String::from_utf8_lossy(packet)` decodes the XMP packet from `crates/metadata/src/lib.rs:75` as lossy UTF-8. There is no BOM or UTF-16/32 detection anywhere in `crates/metadata/src`.

**Impact:** for a JPEG whose XMP packet is UTF-16 or UTF-32, the rating and dates from XMP are dropped. The EXIF fallback still applies.

**Fix (proposed):** detect the BOM (UTF-8, UTF-16 LE/BE, UTF-32 LE/BE) and transcode before parsing. `docs/EXIFTOOL_PORT_PLAN.md:204` already plans this.

## 7. Version counter cast to `i32` for QML (`crates/desktop/src/gallery/model.rs:38`)

`ROLE_VERSION` casts the grid's `u32` version (`crates/client/src/grid.rs:169-170, 203-205`) with `cast_signed()`. Versions at or above 2^31 become negative, and `GridCell.qml` (`crates/desktop/qml/GridCell.qml:25-26`) gates on `>= 0`, so those cells read as unversioned.

**Impact:** practically unreachable. It needs about 2.1 billion version bumps in one process.

**Fix (proposed):** pass the version as a type that holds the full `u32` range, such as `i64`, so no value turns negative.
