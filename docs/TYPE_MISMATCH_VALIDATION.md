# Type mismatch findings: open items

Eight findings from the type-mismatch review are still open at HEAD `362888c`. Items 1 to 7 are rated low. Item 8 is rated medium, because it fails silently at scale. Items 3, 7 and 8 overlap steps of Phase 7 in `docs/BOUNDED_MEMORY_PLAN.md`. Each overlap is noted under its item. The design is not finalized, so no item's status has changed. Each entry gives the current location, the defect, its impact, and the planned fix. Fixed and stale findings are not listed.

## 1. Exif IFD pointer narrowed to 32 bits (`crates/metadata/src/tiff.rs:155-163`)

`uint` narrows the value to `u32`, and returns `None` for anything above `u32::MAX`. For a BigTIFF, the Exif IFD pointer is an 8-byte value, and `exif.rs:60` reads it through `uint`. The other offsets in the reader are `usize`.

**Impact:** a BigTIFF whose Exif IFD sits above 4 GiB loses `DateTimeOriginal` and `DateTimeDigitized`. The date is lost only when the XMP fallback (`crates/metadata/src/lib.rs:95`) also has none. `crates/analysis/src/analyze.rs:118` stores no date in that case.

`uint` also has no arm for IFD (type 13), which TIFF Technical Note 1 defines for pointers, so an Exif IFD pointer written as type 13 is dropped as well. This is from reading the match arms; no file with that type has been checked.

**Fix (planned):** one lossless reader, `unsigned(&Entry) -> Option<u64>`, for every unsigned kind: BYTE, SHORT, LONG, LONG8, IFD (13, added) and IFD8. Nothing narrows in the reader. Each use narrows to its own type with a checked conversion:

- a file position, the Exif IFD pointer (`exif.rs:60`), becomes `usize` through `usize::try_from`, so a value that cannot be a position is `None`;
- an attribute becomes its domain type, not `u32`: orientation an enum of the eight EXIF values (a value outside 1 to 8 is `None`), and the rating and rating percent their own types (item 2 gives ratings `UserRating` 1 to 5 and `FileRating` 0 to 5). Each conversion is a checked `try_from`, so the wide value is never silently truncated.

`uint` is removed rather than kept beside the new reader. A narrowing reader next to a lossless one lets a future pointer caller pick the wrong one, and the two can disagree. (`offset_at` is not the fix: it reads an offset stored at a file position, not an entry's value.)

## 2. Rating admits 0, but the APIs spell "unrated" as `None` (`crates/model/src/rating.rs:24-29`)

`Rating` is one 0 to 5 star value, and 0 means unrated. `Rating::new` accepts 0, but the APIs that take a rating take `Option<Rating>`, where `None` also means unrated. So unrated has two spellings, `Some(Rating(0))` and `None`. `Edit::rate` (`crates/app/src/edit.rs:27`) takes `Option<Rating>`, so a caller can pass `Some(0)`.

**Impact:** no data defect today. The desktop maps 0 stars to `None` before calling `rate` (`crates/desktop/src/gallery/edits.rs:77`), and the catalog stores unrated as no row. But a `Some(0)` that reaches the `ratings` table fails its `CHECK (rating BETWEEN 1 AND 5)` (`crates/catalog/src/schema.rs:647-649`), so the type admits a value the store rejects. The 0 handling is also in two places, the desktop and the catalog.

**Meaning, as the code already uses it.** One rating, 0 to 5 stars. 0 means unrated, and unrated is no row. The file side has the same scale: a file that holds 0 stars is unrated, as `three_way_rating_sync` (`crates/catalog/src/writers.rs`) already treats it. The workflow meanings of 1 to 5 belong to the user's convention (1 rejected, 2 maybe, 3 fix rotation or something else, 4 accepted, 5 favorite). The type holds the number, and the app gives it those meanings.

**Current state of the file side, which differs.** `sync_file_rating` writes a file's 0 as a row in `file_ratings` (`writers.rs:383-385`), while the user side deletes its row for 0 (`writers.rs:380-381`). So 0 and "no row" are two spellings there too.

**Fix (planned):** one `Rating` type, 0 to 5, with 0 as unrated, and no `Option` in the rating APIs.

- The rating APIs take `Rating` instead of `Option<Rating>`: `set_rating` and `Edit::rate` (`crates/app/src/edit.rs:27`), `Grid::Rate` and `Command::Rate` (`crates/client/src/controller.rs:337`), and `Write::Rating` and `WriteEffect::FileRating` (`crates/catalog/src/queue/job.rs:46`, `queue.rs:231`).
- The catalog is the one place that turns 0 into storage. Both `ratings` and `file_ratings` keep no row for 0. `set_rating` deletes the row for 0, as it already does for `None` (`crates/catalog/src/queue.rs:41-44`), and `set_file_rating` should do the same, which it does not yet (`queue.rs:318-323` stores a 0). The `CHECK` constraints stay as they are, and never see a 0.
- The desktop's `filter(|r| r.get() > 0)` (`crates/desktop/src/gallery/edits.rs:77`) and the test helper's filter (`writers.rs:1071`) go, because the catalog now handles 0.
- `Rating::new` stays. It returns `None` only above 5.
- The write of a cleared rating is `Write::Rating(Rating(0))`, so the file gets 0 stars, which is what the file already holds for unrated.
- Tests: `Rating(0)` in, no row out, in both tables. A file's 0 and a file with no rating give the same catalog state.

The wire format stays unchanged.

## 3. Last-file preference uses a row id and a `-1` sentinel (`crates/app/src/prefs.rs:34-35`)

*Related, not reconciled:* `docs/BOUNDED_MEMORY_PLAN.md` step 7.9 names `save_last_file(Option)` and `loadedAnchor`, which overlap this item's proposed Rust-side ownership.

`Prefs.last_file` is an `Option<FileId>`, but on disk it is `-1` for none (`prefs.rs:65-79`). The Qt bridge also passes an `i64` with `-1` for none (`crates/desktop/src/settings.rs:40-41, 73-77`).

**Impact:** a catalog rebuild reuses row ids, so the startup selection can pick a different file. A stale id that names no file selects nothing. The sentinel itself cannot collide with a generated id.

**Fix (planned):** persist the file's identity, not its row id. Row ids are local to a catalog. `FileIdentity` (volume plus NTFS file ID) keeps a file's row across moves and renames.

- `Prefs.last_file` becomes `Option<FileIdentity>`. At startup, resolve it to a `FileId` with one lookup on `files(volume, file_ref)`, which has a UNIQUE index. No match means nothing is selected. When the desktop saves a `FileId`, it looks up that row's identity first.
- Disk format: write `null` for none. Read a legacy negative number as none. Read a legacy non-negative number as a row id, resolve it once, and re-save it as an identity.
- Qt bridge, repo-wide: no bridge carries "none" as a number. *Review:* this site list was found incomplete, so it is not a verified inventory. The `-1` sites listed are `gallery_bridge.rs:51, 57, 98, 166` (current item, viewer item, list shown, item showing a file), `folders_bridge.rs:98` (`-1` collapsed, `-2` not in the tree), `failed_files_bridge.rs:61`, and the action codes in `keymap_bridge.rs:21, 44`. Each is replaced by the type that fits:
  - *Withdrawn in review:* the `QPersistentModelIndex` plan for item selections and positions (current item, viewer item, list shown) did not survive adversarial review, so it is not the plan. A model-roles probe found that a Rust-owned selected role driven by intents works, and that no `-1` crosses the boundary. The replacement design is open (Phase 7 step 7.9 in `docs/BOUNDED_MEMORY_PLAN.md`).
  - Id values, such as `last_file`'s `FileId`: `QVariant`, where null means none, carried by explicit getter and setter functions with a notify signal. Verified by a probe build against the repo's cxx-qt fork (rev `ff48a9b`, Qt 6.12.0 msvc2022_64): a `QVariant` `qproperty` with `NOTIFY` compiles and round-trips. A JS `5` reaches Rust as `Some(5)`, and a JS `null` reaches Rust as a null variant. A `QVariant` property's default is `undefined` in QML, not `null`, so "none" must be written explicitly. Two `bool`/`i64` properties sharing one custom `NOTIFY` also compile. Their custom writer writes both and emits once, and QML's single handler sees the new pair. The probe is in the session scratchpad, outside the repo.
  - Action codes (`ViewerAction`, `PanelAction`): a `None` variant in the Rust enum, so no code is a magic number.
  - The folder tree's collapsed and not-in-tree states: a Rust enum with named variants.

  The identity never crosses into QML, because a 64-bit file ID cannot be held exactly in a JS number. `settings.rs` keeps the conversion to `FileId` at its one boundary function. This is the largest item in the list; it touches the bridges and their QML callers.
- *Probe P1, absence at typed receivers:* a typed `int` QML receiver or parameter turns an absent value into 0, and `select(int)` given `undefined` or `null` selected row 0. Only a `var` or a Rust-checked `QVariant` keeps absence. Rust therefore checks absence before a value reaches a typed side. This constrains any design for this item, whichever type it picks.
- A displaced row (`files.volume` NULL) has no identity to remember, so it selects nothing.
- Tests: identity round-trip; a legacy `-1` reads as none; a legacy row id resolves once; a rebuild that reuses a row id selects nothing for the other file.

## 4. CLI accepts thumbnail height 0 (`crates/cli/src/args.rs:85-88`)

`Thumbs { height: u32 }` takes any value. The dispatch is at `crates/cli/src/lib.rs:228-234`, and neither the parser nor `pg_thumbs::run` (`crates/thumbs/src/lib.rs:172`) checks the range. The valid range, 16 to 2048, is enforced only in the client (`crates/client/src/image.rs:42, 58-60`).

**Impact:** a deliberate height of 0 is accepted. The effect has not been traced: the expected result is a per-file encode failure with no top-level error.

**Fix (planned):** add a `ThumbHeight` newtype that holds the 16 to 2048 range, in `pg-model`. The client, `pg-thumbs`, `pg-cli` and the desktop all depend on it.

- `ThumbHeight::new(u32) -> Option<ThumbHeight>`, used as the CLI's clap `value_parser`, so an out-of-range height is a parse error.
- `ThumbHeight::clamp(u32)` for the UI, replacing the clamp at `image.rs:58-60`. Its meaning stays the same.
- `ImageRequest::Thumb` and `pg_thumbs::run` take `ThumbHeight`, so the range holds by construction.

## 5. Megabyte buffer size shift can wrap (`crates/cli/src/args.rs:53`)

`opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1) << 20)` shifts a `usize` without a check. Any value of 2^44 MB or more wraps to a small budget, which serialises reads through `Pool::new` (`crates/jobs/src/pipeline.rs:118`).

**Impact:** serialised reads for an absurd argument. No data or hang impact.

**Fix (planned):** convert in the clap `value_parser`, using `checked_mul(1 << 20)` and rejecting a value whose byte count does not fit. The parsed field holds bytes, so `options()` does no shift at all.

## 6. XMP packets with a UTF-16 or UTF-32 encoding are decoded as UTF-8 (`crates/metadata/src/xmp.rs:62`)

`String::from_utf8_lossy(packet)` decodes the XMP packet from `crates/metadata/src/lib.rs:75` as lossy UTF-8. There is no BOM or UTF-16/32 detection anywhere in `crates/metadata/src`.

**Impact:** for a JPEG whose XMP packet is UTF-16 or UTF-32, the rating and dates from XMP are dropped. The EXIF fallback still applies.

**Fix (planned):** detect the packet's encoding from its byte order mark, or, without one, from the null-byte pattern of its leading `<?xpacket` bytes. ExifTool and Exempi both do this. I could not confirm the normative wording in the XMP specification, so the rule follows those implementations. Decode it strictly into a UTF-8 `String` (`String::from_utf16` for UTF-16, `char::from_u32` per 4 bytes for UTF-32). A packet that does not decode is skipped, so its values are absent, not partly read. quick-xml then reads UTF-8 as it does now. `docs/EXIFTOOL_PORT_PLAN.md:204` already plans the detection.

## 7. Version counter cast to `i32` for QML (`crates/desktop/src/gallery/model.rs:38`)

*Reconciled (recommended):* `docs/BOUNDED_MEMORY_PLAN.md` step 7.7 lists `u64` versions, and this item now uses the same type.

`ROLE_VERSION` casts the grid's `u32` version (`crates/client/src/grid.rs:169-170`) with `cast_signed()`. Versions at or above 2^31 become negative. `GridCell.qml` (`crates/desktop/qml/GridCell.qml:25-26`) then gates on `>= 0`, so those cells read as unversioned.

**Impact:** practically unreachable. It needs about 2.1 billion version bumps in one process.

**Fix (planned):** versions are `u64` in Rust, as plan 7.7 states. The role carries them as an `i64`, and the QML property is a `real`. A `real` holds integers exactly only up to 2^53, so the conversion is checked, and a version above 2^53 fails loudly. Two things go with it:

- The `>= 0` gates in `GridCell.qml` (lines 39-49) are removed. `Grid::version` returns 0 for a row past the rows (`crates/client/src/grid.rs:166-169`), and the Rust side never sends a negative version. The gates only ever matter after the wrap.
- `GridRow.qml`'s `shownVersion: -1` becomes `null`, so no version value doubles as a sentinel.

## 8. Counts and positions are 32-bit at the Rust↔QML boundary (`crates/desktop/src/gallery/roles.rs:42-44`)

*Related, consistent:* `docs/BOUNDED_MEMORY_PLAN.md` decision D29 (exact `f64` ranks and counts, decimal file ids) and step 7.9 (`qt_int` and `to_i32` deleted) match this item's fix. The plan's windowed model also covers its window-slide and stale-reference concerns, which are not listed here.

`qt_int` clamps any count or row to `i32::MAX`. The bridge properties are `i32`: `gallery_bridge.rs:41` (`count`), `:53` (`current`), `:58` (`viewing`), `:60` (`anchor`), `filters_bridge.rs:25-26` (`total`, `matching`), and `shell.rs:117` (`list_shown(last_file: i32, count: i32)`). Qt's model API is `int` as well: `rowCount` returns `i32` (`facet_choices.rs:88`, `failed_files.rs:122`).

**Impact:** a catalog of 2^31 − 1 items or more (about 2.1 billion) shows a clamped count, silently, and items past that index cannot be addressed. A single flat Qt model cannot hold more than 2^31 − 1 rows.

**Fix (planned):**
- Counts and positions cross as `i64` in Rust and as `real` in QML. A JS number is exact to 2^53, about 9 × 10^15. The `int` declarations in the QML files become `real`. `qt_int`'s saturation is replaced by a checked conversion that fails loudly.
- The Qt model exposes a window of rows around the viewport, kept under 2^31. Rust keeps absolute positions as `i64`. A flat model cannot reach billions of items, so the gallery needs the window.
- Item 3's selection must address a window row with an absolute `i64` position, or the file's identity. Any Qt-side row handle inherits the 32-bit row limit. (The `QPersistentModelIndex` plan in item 3 is withdrawn.)
- *Probe P5 (property level):* QML doubles round-trip exactly up to 2^53 for integers.
- *Probe P10 (rendered):* a marker inside a `Flickable`'s content item, moved by `-contentY` in the scene graph, lands on the wrong row from `contentY` 2^24 and collapses by 2^28, because of float32 spacing. A sibling marker whose y is computed in double is exact up to 2e11. So any `real` or `i64` position that reaches the scene graph must stay viewport-relative and sit outside the moving content item. The earlier P5 claim that a `Flickable` at 2e11 "positions its children correctly" was a property check, and is withdrawn.
- *Window size (probe `window`, and `real` on the user's aspects):* the window (viewport ±1500 px) holds at most 4,140 items at 3840×2160 and 11,462 at 7680×4320 on the user's aspects (the synthetic aspects reach 4,333 and 11,977). Qt's 2^31 row limit is far above these. The 32-bit problem is in the total count, not in the window.
