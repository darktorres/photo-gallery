# Type mismatch findings: open items

Six findings from the type-mismatch review are still open at HEAD `9d167729`. The findings were first written at `489f3d31`, and every line reference and status below was re-checked against `9d167729`. Items 1 to 5 are rated low. Item 6 is rated medium, because it fails silently at scale. Items 3 and 6 overlap steps of Phase 7 in `docs/plans/BOUNDED_MEMORY_PLAN.md`. Each overlap is noted under its item. The design is not finalized. Each entry gives the current location, the defect, its impact, and the planned fix. Fixed and stale findings are not listed. Item 2 is partly done, and its entry says what remains.

## 1. Exif IFD pointer narrowed to 32 bits (`crates/metadata/src/tiff.rs:157`)

`uint` narrows the value to `u32`, and returns `None` for anything above `u32::MAX`. For a BigTIFF, the Exif IFD pointer is an 8-byte value, and `exif.rs:61` reads it through `uint`. The other offsets in the reader are `usize`.

**Impact:** a BigTIFF whose Exif IFD sits above 4 GiB loses `DateTimeOriginal` and `DateTimeDigitized`. The date is lost only when the XMP fallback (`crates/metadata/src/lib.rs:92`) also has none. `crates/analysis/src/analyze.rs:114` stores no date in that case.

`uint` also has no arm for IFD (type 13), which TIFF Technical Note 1 defines for pointers, so an Exif IFD pointer written as type 13 is dropped as well. This is from reading the match arms. Checked since: a hand-written TIFF with a type-13 Exif pointer (a test embeds its bytes) reads the same date that ExifTool reads from it. ExifTool's sample images have none: all 49 Exif pointers there are LONG. `uint` also serves the orientation and rating reads at `exif.rs:55-59`, so the fix changes those callers too.

**Status:** implemented, uncommitted. The metadata crate's 30 tests pass, including the item 1 tests below and a 4 GiB BigTIFF test. The tests of the dependent crates (`pg-analysis`, `pg-writeback`, `pg-exiftool`, `pg-edit`, `pg-library`, `pg-render`) pass too.

**Checked against ExifTool's sample images** (46 files in the ExifTool source's `t/images`): orientation matches in 46 of 46, and the rating matches the Windows precedence model applied to ExifTool's fields in 46 of 46. Dates match in 33 of 35. The two misses are not reader errors: one is a maker-note date (`Casio`), which the reader doesn't read, and the other is an ExifTool Composite date with a time zone, where the file holds only a date-only XMP value.

**Fix:** four parts. Each was checked against the code at `9d167729` and against ExifTool.

1. **One lossless reader.** `Tiff::unsigned(&Entry) -> Option<u64>` for every unsigned kind: BYTE, SHORT, LONG, LONG8, IFD (13, added) and IFD8. Nothing in the reader narrows. `uint` is removed rather than kept beside it, because a narrowing reader next to a lossless one lets a future pointer caller pick the wrong one. (`offset_at` is not the fix: it reads an offset stored at a file position, not an entry's value.)

2. **Presence follows ExifTool.** An entry whose value cannot be read (its offset or size falls outside the block) is skipped, so an earlier entry with the same tag stands. The last readable entry wins, for every field. In ExifTool, a "Bad offset" sets `$bad`, the value is never formatted, and `next unless defined $val` skips the entry before `FoundTag` (`lib/Image/ExifTool/Exif.pm:6661`, `:6916`). Its duplicate rule keeps the last readable entry under the plain tag name and moves the earlier value to `Tag (1)` (`lib/Image/ExifTool.pm:9623`). This plan keeps the losing values too, so the reader follows the port's rule rather than a narrower one. `Exif` keeps each tag's superseded readable values in entry order, as ExifTool keeps `Tag (1)`, `Tag (2)` and so on. The winner stays the last readable entry. The port design's duplicate rule (`docs/plans/EXIFTOOL_PORT_DESIGN.md:85`) is the same: keep last-seen at equal priority, `Name (N)` for the rest. The source is commit `3d7a0538` of `exiftool/exiftool`, read in a local clone at `D:\GitHub\exiftool`. This replaces two rules in the current code: the orientation exception (an unreadable last entry keeps the earlier one), and the rule that makes a rating or date absent when its last entry doesn't read.

3. **`Exif` stores raw values; the domain decides validity.** `Exif` keeps the raw `Option<u64>` of each field's last readable entry. The conversions to domain types happen in one place, in `Meta::read`, and a value outside a domain is not a value of that type. ExifTool does the same: its `Rating` (0x4746) and `RatingPercent` (0x4749) have no range conversion, so a stored 7 is reported as 7 (`lib/Image/ExifTool/Exif.pm:1521`, `:1541`). Its XMP module has no range conversion for Rating either.
   - **Orientation:** `u8::try_from(v).ok().and_then(Orientation::new)`, using the existing validated type (`crates/model/src/orientation.rs`, 1 to 8). Not a new eight-variant enum, which would duplicate it. An invalid value reads as absent, so `orientation()` gives `UPRIGHT`, as now. `[6, 9]` reads as upright, as it does today.
   - **Rating:** stars above 5 show 5 stars, and a 0 is unrated. Windows clamps (measured: an EXIF or XMP 7 shows 5 stars), so 7 reads as 5, as it did before the item 1 commit. The cap is `min(5)` in `Meta::read`.
   - **Rating percent:** a percent above 99 shows 99, which is 5 stars (measured: 100 and 101 show 99). The stored value is then mapped to stars by Explorer's bands, in `stars_of_percent` (`crates/metadata/src/xmp.rs`): 1 to 12 is 1 star, 13 to 37 is 2, 38 to 62 is 3, 63 to 87 is 4, and 88 to 99 is 5. These bands replace the ExifTool canonical points, which `percent()` used before (measured: 47 of the 99 percent values differ from `percent()`).

4. **File position.** The Exif IFD pointer is read as `usize` with `usize::try_from`. A value that doesn't fit a `usize` reads as absent, and the entry is skipped as an unreadable one, so its dates are absent.

**Behaviour changes, all deliberate:**
- Rating 7 reads as 5 stars, as Windows shows it. (The item 1 commit made it no rating; this corrects that.)
- Rating percent above 100 reads as 5 stars, as Windows shows it. (The item 1 commit made it no rating; this corrects that.)
- A percent is mapped to stars by Explorer's bands, not by `percent()`.
- An unreadable last rating, rating percent or date entry leaves the earlier readable value, not an absent one.
- A BigTIFF Exif pointer above 4 GiB, and IFD- or IFD8-typed pointers, now read their dates.
- Orientation: no change for any input.

**Tests (written, passing):**
- a BigTIFF whose Exif IFD is above 4 GiB reads its dates, and a BigTIFF pointer above `u32::MAX` never wraps to a real IFD;
- an IFD (13) pointer and an IFD8 pointer read;
- duplicate orientation entries: `[6, 3]` gives 3; `[6, 9]` gives upright; `[6, <out of block>]` gives 6;
- a tag given three times keeps the two superseded values, in entry order, beside the winner;
- an unreadable last rating or date entry leaves the earlier one;
- rating 7 reads as 5 stars, and rating 7 with percent 50 reads as 3 stars (the percent outranks the stars);
- rating percent 101 reads as 5 stars.

## 2. Rating admits 0, but the APIs spell "unrated" as `None` (`crates/model/src/rating.rs:47`)

*Status at `9d167729`: partly done.* `Command::Rate` (used at `crates/client/src/controller.rs:474`), `Write::Rating` and `WriteEffect::FileRating` (`crates/catalog/src/queue/job.rs:47`) already take a `Rating`, not an `Option`. Still open: `Edit::rate`, `set_rating` and `set_file_rating` take `Option<Rating>`. `set_file_rating` still stores a 0 (`queue.rs:364-370`). The desktop filter (`crates/desktop/src/gallery/edits.rs:105`) and the `Rating::new(n).filter(...)` in the catalog tests (`writers.rs:2283`, `browse/fixtures.rs:73`) still exist. The module doc (`rating.rs:3`) still says an unrated file is `Option<Rating>`.

`Rating` is one 0 to 5 star value, and 0 means unrated. `Rating::new` accepts 0, but the APIs that take a rating still take `Option<Rating>`, where `None` also means unrated. So unrated has two spellings, `Some(Rating(0))` and `None`. `Edit::rate` (`crates/app/src/edit.rs:27`) takes `Option<Rating>`, so a caller can pass `Some(0)`.

**Impact:** no data defect today. The desktop maps 0 stars to `None` before calling `rate` (`crates/desktop/src/gallery/edits.rs:105`), and the catalog stores unrated as no row. But a `Some(0)` that reaches the `ratings` table fails its `CHECK (rating BETWEEN 1 AND 5)` (`crates/catalog/src/baseline.sql:81`; the schema is in `baseline.sql`, not `schema.rs`), so the type admits a value the store rejects. The 0 handling is also in two places, the desktop and the catalog.

**Meaning, as the code already uses it.** One range, 1 to 5 stars, for the user's rating and the file's. Unrated is `None` in both. The file's 0 is EXIF's spelling of unrated, and the catalog never holds it. The file side has the same scale: a file that holds 0 stars is unrated, as `three_way_rating_sync` (`crates/catalog/src/writers.rs`) already treats it. The workflow meanings of 1 to 5 belong to the user's convention (1 rejected, 2 maybe, 3 fix rotation or something else, 4 accepted, 5 favorite). The type holds the number, and the app gives it those meanings.

**Current state of the file side, which differs.** `sync_file_rating` writes a file's 0 as a row in `file_ratings` (`writers.rs:571`), while the user side deletes its row for 0 (`writers.rs:566-568`). So 0 and "no row" are two spellings there too.

**Status:** implemented. The type keeps the name `Rating` (1 to 5), not `Stars` as first written: the name already fit, and renaming about 230 uses added no meaning. Everywhere below, `Rating` is this type. The catalog's and the file's ratings are `Option<Rating>`. Zero is refused by `Rating::new` and `TryFrom<u8>`, and the schema refuses a 0 row in `file_ratings`. Tests: `crates/model/src/rating.rs`, `crates/catalog/src/writers.rs` (`the_store_refuses_a_zero_rating_row`, `a_file_with_no_rating_keeps_the_user_rating_on_its_first_sync`).

**Fix:** one type, and one spelling of unrated. `Stars` (here `Rating`) is a star rating from 1 to 5. A rating is `Option<Stars>`, and `None` is unrated, everywhere in the code. Zero is not a value of any type. It appears only as EXIF's and the job wire's spelling of unrated, at the file edges.

1. **One type: `Stars` (1 to 5).** It serves the user's rating and a file's rating alike. `Stars::new(u8) -> Option` accepts 1 to 5, and `TryFrom<u8>` rejects 0. Nothing converts between the two sides, because both hold the same number. The field and parameter names carry the domain at each use: `user_rating` for the catalog's `ratings`, and `file_rating` for `file_ratings`, `sync_file_rating` and the write decision. Two types were considered and rejected. The two sides share the same valid range and the same edge handling, so a second type would add conversions that do nothing.
2. **The state is `Option<Stars>` on both sides.** `ratings` already has `CHECK (rating BETWEEN 1 AND 5)`, and rule 6 tightens `file_ratings` to the same range.
3. **A zero means different things by field.** Measured on Windows 10, reading the shell's `System.Rating` for ExifTool-written copies of `sample.jpg`:
   - In a percent field, a 0 is no value. XMP `xmp:Rating` 0 with EXIF `RatingPercent` 75 shows 75, and EXIF `Rating` 0 with EXIF `RatingPercent` 75 shows 75.
   - In XMP stars, a 0 hides EXIF stars. XMP 0 with EXIF `Rating` 5 shows unrated, and XMP 2 with EXIF `Rating` 0 shows 25.

   So the reader keeps XMP `xmp:Rating` in three states (absent, 0, stars), because 0 suppresses EXIF stars. The other three fields are `Option<Stars>`. The writer encodes a clear as an explicit XMP 0 (`tool.write_rating(&path, &temp, …)`), and the job wire does the same (`crates/model/src/write.rs:113` writes it, `write.rs:123` reads it back). So the wire format stays unchanged, and `Write::Rating` carries `Option<Stars>`. The write must also remove the stale fields, as described below.
4. **Conversions happen at two edges, each in one place:**
   - *input:* the key value 0 becomes `None`. The controller reads it at `crates/client/src/controller.rs:474` as `Stars::new(stars)`, which is `None` for 0. `keys.rs:147` produces `Rate(0)` for the 0 key.
   - *file read:* `Stars::new` on the raw value. A value above 5 and an explicit 0 both read as absent (rule 3).
   - The write path has no conversion: `Option<Stars>` passes through unchanged.
5. **Signatures keep `Option`.** `set_rating(file, Option<Stars>, write_back)` (`crates/catalog/src/queue.rs:40`), `Edit::rate(file, Option<Stars>)` (`crates/app/src/edit.rs:27`), `Command::Rate { rating: Option<Stars> }`, `Write::Rating(Option<Stars>)` and `WriteEffect::FileRating { rating: Option<Stars> }`. Once no type holds 0, `Some(0)` can't be built, and `None` is the one spelling. The earlier plan's "no `Option`" would have made 0 a sentinel that every reader must filter.
6. **Storage.** `file_ratings` holds 1 to 5 only, and `None` is no row. The catalog is not released yet, so this is an edit to `baseline.sql`, not a migration: `crates/catalog/src/schema.rs:34` says that before the first release the baseline is edited in place, and `MIGRATIONS` is empty (`schema.rs:84`). The `CHECK` tightens to `BETWEEN 1 AND 5`. Any catalog created with the current baseline then has to be rebuilt or recreated, which is your decision to make before a release. `rebuild.rs:160` already lists `file_ratings`, so rebuilds carry it.
7. **The sync and write rules have no special cases.** `sync_file_rating` (`crates/catalog/src/writers.rs:558`) compares `in_file` with `known`, both `Option<Stars>`. The write decision in `crates/catalog/src/queue.rs:61-70` compares `wanted` with `in_file`, both `Option<Stars>`. The special cases `None if in_file > 0 => Some(0)` and `None => in_file` go.
8. **Filters go.** The desktop's `filter(|r| r.get() > 0)` (`crates/desktop/src/gallery/edits.rs:105`) and the test-helper filters (`writers.rs:2283`, `crates/catalog/src/browse/fixtures.rs:73`) are deleted.
9. **Tests (planned):**
   - `Stars::new(0)` is `None`, and `TryFrom<u8>` rejects 0.
   - EXIF 0 reads as unrated, and `None` writes an explicit 0. The wire round trip (`write.rs:113`, `:123`) gives `None` back.
   - Clearing a file that Windows rated (XMP `xmp:Rating` 4, a stale EXIF `RatingPercent` 75, and the other stale fields) reads as unrated, and Windows shows unrated, after the write below.
   - Sync, adoption: a file cleared by another program (known 4, none in the file) clears the user's rating, as now.
   - Sync, no change: a file with no rating, never synced, and the user's rating 3 keeps the rating. This holds today (verified: the catalog keeps 3).
   - Sync, file holds an explicit 0, never synced, and the user's rating 3: **changed.** Today this clears the rating (verified: the catalog reads `None`), because the file's 0 is `Some(0)` against a `None` baseline. The new rule keeps the rating.
   - Clearing writes `None` only into a file that holds a rating.
   - The baseline's `file_ratings` `CHECK` rejects 0 (a catalog built from the edited baseline), and a rebuilt catalog holds no 0 rows.
   - Pairing: a file whose only rating row was 0 is no longer kept by `has_children` (`crates/catalog/src/sql.rs:88`). **Changed.** Verified today: a file with no rating has `has_children` 0, and after an explicit 0 is synced (one `file_ratings` row) it is 1. The predicate is the one pairing uses (`crates/catalog/src/pair.rs:321`, `:449`). The full pass agrees: a displaced row (its identity taken by another file) counts as having data (`unpaired_with_data` 1) when its only rating row is 0, and 0 without one.

**The reader keeps all four rating fields.** `Meta::read` reads four fields: XMP `xmp:Rating`, XMP `MicrosoftPhoto:Rating` (a percent), EXIF `Rating` (stars), and EXIF `RatingPercent` (a percent) (`crates/metadata/src/lib.rs:84-92`, and `XMP_WANTED` in `xmp.rs`). It then collapses them into one value by a fixed chain: XMP stars, then XMP percent, then EXIF stars, then EXIF percent. The first present one wins, and the other three are dropped. So a file whose fields disagree reads as one value, and the disagreement is lost. Our own writer makes this happen: it sets only `XMP:Rating` (`crates/exiftool/src/lib.rs:196-197`), and ExifTool does not sync the other fields. Measured with the installed ExifTool 13.60 on a copy of `crates/bench/benches/ir/sample.jpg`: a file with EXIF `RatingPercent` 75 still reads 75 after `-XMP:Rating=2`.

Under this plan the reader returns all four readings, each as `Option<Stars>` (rule 3). One function derives the effective rating from them, and that function is the only place the precedence lives. The percent readings convert to stars with `percent()`, using the Windows scale of 1, 25, 50, 75 and 99 (`Microsoft.pm:249-252`). Tests cover each pair of fields that disagree, and the measured pairs above, each checked against Windows' result.

**Precedence, measured on Windows 10 in a full matrix.** The matrix covers JPEG singles, triples and quads, and TIFF singles: 952 cases, with stars 0 to 7 and percents 0 to 101 (`scratchpad\rating_matrix_ext.ps1`). The first field the file holds decides: XMP percent, then EXIF percent, then XMP stars, then EXIF stars. A 0 there is unrated, and hides the fields below it but not the fields above. The reader matches all 952 cases. PNG and WebP showed no rating at all, so the matrix does not cover them.

The pairs below are the first measurements, kept for reference. The order is XMP percent, then EXIF percent, then XMP stars, then EXIF stars. A 0 in a percent field is no value, and a 0 in XMP stars hides EXIF stars (rule 3). Each pair tested agrees with this order, and `System.Rating` shows the winner: XMP percent 75 over XMP stars 2 gives 75; EXIF percent 75 over XMP stars 2 gives 75; EXIF percent 75 over EXIF stars 2 gives 75; XMP stars 2 over EXIF stars 5 gives 25; XMP percent 75 over EXIF percent 25 gives 75; XMP percent 25 over EXIF stars 5 gives 25. Today's chain differs in three places: it checks XMP stars before XMP percent, XMP stars before EXIF percent, and EXIF stars before EXIF percent. So the app shows a different rating from Windows for the same file. The plan adopts Windows' order, and the one effective-rating function uses it. These results come from one Windows 10 machine and ExifTool 13.60, so the tests should pin them.

**The write updates the rating fields the file already has (measured).** Our writer sets only `XMP:Rating`, and Windows then shows a stale percent. Measured on Windows 10 with ExifTool 13.60, on copies of `sample.jpg`:
- A file with stale EXIF `Rating` 5, EXIF `RatingPercent` 75, XMP `xmp:Rating` 4 and `XMP-microsoft:RatingPercent` 75. Setting XMP to 2 and updating only the fields present (each to its value on its own scale) makes Windows show 25, and every present field reads 2 stars.
- A file with only XMP stars 4 and EXIF `RatingPercent` 75. The same update makes Windows show 25. No field is added, and fields absent from the file stay absent.
- Clearing the present fields (each set to 0) makes Windows show unrated.
- A plain file with no rating fields, given XMP 2 alone, shows 25.
- ExifTool's conditional `-if '$EXIF:RatingPercent' …` fails on an absent field and writes nothing.

So the write sets each present rating field to its value on its own scale: stars for `Rating` and `xmp:Rating`, and the Windows percent (1, 25, 50, 75, 99 for 1 to 5 stars, 0 for unrated) for the two percent fields. Presence is read first, then one write sets only the present fields. The write adds no field and removes none, so other tools' data stays as the file had it. This does not write all four fields. It is preferred over deleting the stale fields, which also gives Windows the right value but removes data the file had.

**Decided.**
1. **Recording a disagreement: keep only the effective value (option a), as today.** A disagreement between fields is neither stored nor logged.
2. **The write side:** update the rating fields the file already has, as above. Implemented in `write_rating` (`crates/exiftool/src/lib.rs`): it sets `XMP:Rating` and each rating field the file already holds. The rebuild-the-metadata fallback is gone. A test writes over a stale percent and reads the new rating back.

**Behaviour changes, each a decision (reversible):**
- A never-synced file that reads unrated no longer clears the user's rating (the sync test above).
- A file whose only rating row was 0 no longer counts as a child for pairing (`sql.rs:88`).
- An explicit 0 in a percent field no longer blocks the next field. Windows does the same (measured, rule 3). An XMP 0 still hides EXIF stars, as Windows does.

## 3. Last-file preference uses a row id and a `-1` sentinel (`crates/app/src/prefs.rs:43`)

*Related, not reconciled:* `docs/plans/BOUNDED_MEMORY_PLAN.md` step 7.9 names `save_last_file(Option)` and `loadedAnchor`, which overlap this item's proposed Rust-side ownership.

`Prefs.last_file` is an `Option<FileId>`, but on disk it is `-1` for none (the `last_file` serializer at `prefs.rs:112`: `serialize` at 117, `deserialize` at 122). The Qt-side copy is no longer in `settings.rs`, which has no `last_file` now. The value reaches the desktop at `crates/desktop/src/controller.rs:129` and is saved at `crates/desktop/src/shell.rs:95`.

**Impact (corrected by validation):** a row id means something only in the catalog that assigned it. The rebuild that copies a catalog keeps its ids (`carry_rows`, and the id high-water mark carried over, `crates/catalog/src/rebuild.rs`), so it does not change which file a saved id names. A catalog made anew does: one moved aside as unreadable, or rebuilt from a scan, hands out ids again from 1, so a saved id can name a different file, and the restore selects it. A stale id that names no file selects nothing (`RestoreSelection`, `crates/client/src/controller/start.rs`). The sentinel can't collide with an id, since ids start at 1.

**Fix (planned, revised by validation):** persist the file's identity and the size and mtime the catalog keeps for it, not its row id. Row ids are local to a catalog. The identity alone is not enough: `FileIdentity` (`crates/model/src/ids.rs`) documents that an inode can be reused after a file is deleted, and that size and mtime are how a different file is told apart. Restore selects the row only when the identity matches, the row is present (`missing = 0`), and its size and mtime match. That keeps a file selected across a rename or a move on its volume, and refuses a reused identity.

- `Prefs.last_file` becomes `Option<SavedFile>`, where `SavedFile` holds the identity, size and mtime. At startup, resolve it to a `FileId` with one lookup on `files(volume, file_ref)`, which has a UNIQUE index, then apply the restore rule above. No match means nothing is selected. When the desktop saves a `FileId`, it looks up that row's identity, size and mtime first. The save runs at shutdown through `flush` (`crates/desktop/src/shell.rs:250`), and the catalog closes later, in the app's shutdown (`crates/app/src/app.rs:211`). So the lookup should find the catalog open. That order is read from the code, not run.
- Disk format: write `null` for none, and an object with the identity, size and mtime for a file. A legacy number, negative or not, reads as none. The plan used to resolve a legacy row id once, and validation rejects that: a row id from an earlier catalog has no meaning here, and resolving it would select whichever file holds that id now. The saved file is then re-saved on the next save, not converted.
- *Split out by validation: the bridge work is its own item, not part of the saved-file fix. Its QVariant claims cite a probe that is not in the session scratchpad, so they are unverified.* Qt bridge, repo-wide: no bridge carries "none" as a number. *Review:* the inventory below was found by grepping for `-1` and `-2` at `9d167729`. It is not guaranteed complete. The earlier list (at `489f3d31`) had stale line numbers, and it named `gallery/state.rs`, which no longer exists. Its sites are, at `9d167729`: `gallery_bridge.rs:54, 60, 91, 116, 175, 189` (current item, viewer item, rejected-edge fraction, list shown, the anchor's place, item showing a file), `gallery_state.rs:77-80, 94-97`, `folders_bridge.rs:97, 110, 121` (`-1` collapsed, `-1` not in the tree), `failed_files_bridge.rs:61`, the action codes in `keymap_bridge.rs:21, 44`, the selection in `log_model_bridge.rs:28, 31, 84` and `log_model_state.rs:18, 33-34`, the selection and pass codes in `notices_bridge.rs:38, 45, 154, 165, 174` and `notices_state.rs:26, 51`, the lane and ledger codes in `lane_model_state.rs:16, 26`, the mascot and preview in `command_line_bridge.rs:13, 15`, and `gallery/items.rs:27, 152`, `gallery/model.rs:159, 166, 173` (`set_anchor(-1)`), `gallery/nav.rs:8, 10`. Each is replaced by the type that fits:
  - *Withdrawn in review:* the `QPersistentModelIndex` plan for item selections and positions (current item, viewer item, list shown) did not survive adversarial review, so it is not the plan. A model-roles probe found that a Rust-owned selected role driven by intents works, and that no `-1` crosses the boundary. The replacement design is open (Phase 7 step 7.9 in `docs/plans/BOUNDED_MEMORY_PLAN.md`).
  - Id values, such as `last_file`'s `FileId`: `QVariant`, where null means none, carried by explicit getter and setter functions with a notify signal. Verified by a probe build against the repo's cxx-qt fork (rev `ff48a9b`, Qt 6.12.0 msvc2022_64): a `QVariant` `qproperty` with `NOTIFY` compiles and round-trips. A JS `5` reaches Rust as `Some(5)`, and a JS `null` reaches Rust as a null variant. A `QVariant` property's default is `undefined` in QML, not `null`, so "none" must be written explicitly. Two `bool`/`i64` properties sharing one custom `NOTIFY` also compile. Their custom writer writes both and emits once, and QML's single handler sees the new pair. The probe is in the session scratchpad, outside the repo.
  - Action codes (`ViewerAction`, `PanelAction`): a `None` variant in the Rust enum, so no code is a magic number.
  - The folder tree's collapsed and not-in-tree states: a Rust enum with named variants.

  The identity never crosses into QML, because a 64-bit file ID cannot be held exactly in a JS number. `settings.rs` keeps the conversion to `FileId` at its one boundary function. This is the largest item in the list; it touches the bridges and their QML callers.
- *Probe P1, absence at typed receivers:* a typed `int` QML receiver or parameter turns an absent value into 0, and `select(int)` given `undefined` or `null` selected row 0. Only a `var` or a Rust-checked `QVariant` keeps absence. Rust therefore checks absence before a value reaches a typed side. This constrains any design for this item, whichever type it picks.
- A displaced row (`files.volume` NULL) has no identity to remember, so it selects nothing.
- Tests (revised by validation): identity, size and mtime round-trip; `null` reads as none; a legacy number of either sign reads as none, and is not resolved; a restore whose row's size or mtime differs selects nothing (a reused identity); a renamed file keeps its selection; a row marked `missing` selects nothing; a fresh catalog whose ids start again selects nothing for the saved file.

## 4. CLI accepts buffer size megabytes that saturate (`crates/cli/src/args.rs:58`)

*Changed at `9d167729`: the wrap is gone.* `buffer_mb` is now `Option<u64>` (`args.rs:45`), and the line is `opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1).saturating_mul(1 << 20))`. A value too large for a byte count no longer wraps to a small budget. It saturates to `u64::MAX`, which `Budget::new` (`crates/jobs/src/governor.rs:76`) keeps as an effectively unlimited budget. So the old claim that such a value serialises reads through `Pool::new` is no longer true. The value is still accepted silently, and the byte count is not checked.

**Impact:** an absurd argument is accepted and its limit is silently ignored. No data or hang impact.

**Fix (planned):** check the conversion in the clap `value_parser`, using `checked_mul(1 << 20)` and rejecting a value whose byte count does not fit. The parsed field holds bytes, so `options()` does no multiplication at all.

*Closed since the first list:* the thumbnail height entry is fixed. `ThumbHeight` (`crates/model/src/size.rs:82`) holds the 16 to 2048 range (`THUMB_HEIGHT`, `size.rs:12`), and the CLI parses it with a clap `value_parser` (`crates/cli/src/args.rs:131`, parser at `args.rs:190`). `pg_thumbs::run` and `ImageRequest::thumb` take it. The plan's `ThumbHeight::clamp(u32)` was not built. The UI clamp in `client/src/image.rs` is gone, and `ThumbHeight::for_row(RowHeight, Scale)` (`size.rs`) does that job instead.

## 5. XMP packets with a UTF-16 or UTF-32 encoding are decoded as UTF-8 (`crates/metadata/src/xmp.rs:62`)

`String::from_utf8_lossy(packet)` decodes the XMP packet from `crates/metadata/src/lib.rs:76` as lossy UTF-8. There is no BOM or UTF-16/32 detection anywhere in `crates/metadata/src`, as checked at `9d167729`.

**Impact:** for a JPEG whose XMP packet is UTF-16 or UTF-32, the rating and dates from XMP are dropped. The EXIF fallback still applies.

**Fix (planned):** detect the packet's encoding from its byte order mark, or, without one, from the null-byte pattern of its leading `<?xpacket` bytes. ExifTool and Exempi both do this. I could not confirm the normative wording in the XMP specification, so the rule follows those implementations. Decode it strictly into a UTF-8 `String` (`String::from_utf16` for UTF-16, `char::from_u32` per 4 bytes for UTF-32). A packet that does not decode is skipped, so its values are absent, not partly read. quick-xml then reads UTF-8 as it does now. `docs/plans/EXIFTOOL_PORT_PLAN.md:295` already plans the detection.

*Closed since the first list:* the version counter entry (old item 7) is gone. No version role, `cast_signed()` or `>= 0` version gate remains in the desktop, the QML or `client/src/grid.rs`. The last commit that touched `ROLE_VERSION` is `d28e2f72`.

## 6. Counts and positions are 32-bit at the Rust↔QML boundary (`crates/desktop/src/rows.rs:31`)

*Related, consistent:* `docs/plans/BOUNDED_MEMORY_PLAN.md` decision D29 (exact `f64` ranks and counts, decimal file ids, line 131) and step 7.9 (`qt_int` and `to_i32` deleted, line 1510) match this item's fix. The plan's windowed model also covers its window-slide and stale-reference concerns, which are not listed here.

`qt_int` (`crates/desktop/src/rows.rs:31`) clamps any count or row to `i32::MAX`. The copies the plan named are gone: `failed_files.rs` now calls the shared `qt_int`, and `notices.rs` has no copy of it. `facet_choices.rs:29` clamps an `i64` count. The bridge properties are `i32`: `gallery_bridge.rs:42` (`count`), `:56` (`current`), `:61` (`viewing`), `:63` (`anchor`), `filters_bridge.rs:37-38` (`total`, `matching`), and `shell.rs:127` (`list_shown(count: i32)`, declared at `bridges/shell_bridge.rs:61`). `gallery_bridge.rs:187, 192` already take and return file ids as `i64`. Qt's model API is `int` as well: `rowCount` returns `i32` (`facet_choices_bridge.rs:63`, `failed_files_bridge.rs:50`, `folders_bridge.rs:50`, `gallery_bridge.rs:131`). No windowed model exists yet at `9d167729`.

**Impact:** a catalog of 2^31 − 1 items or more (about 2.1 billion) shows a clamped count, silently, and items past that index cannot be addressed. A single flat Qt model cannot hold more than 2^31 − 1 rows.

**Fix (planned):**

- Counts and positions cross as `i64` in Rust and as `real` in QML. A JS number is exact to 2^53, about 9 × 10^15. The `int` declarations in the QML files become `real`. `qt_int`'s saturation is replaced by a checked conversion that fails loudly.
- The Qt model exposes a window of rows around the viewport, kept under 2^31. Rust keeps absolute positions as `i64`. A flat model cannot reach billions of items, so the gallery needs the window.
- Item 3's selection must address a window row with an absolute `i64` position, or the file's identity. Any Qt-side row handle inherits the 32-bit row limit. (The `QPersistentModelIndex` plan in item 3 is withdrawn.)
- *Probe P5 (property level):* QML doubles round-trip exactly up to 2^53 for integers.
- *Probe P10 (rendered):* a marker inside a `Flickable`'s content item, moved by `-contentY` in the scene graph, lands on the wrong row from `contentY` 2^24 and collapses by 2^28, because of float32 spacing. A sibling marker whose y is computed in double is exact up to 2e11. So any `real` or `i64` position that reaches the scene graph must stay viewport-relative and sit outside the moving content item. The earlier P5 claim that a `Flickable` at 2e11 "positions its children correctly" was a property check, and is withdrawn.
- *Window size (probe `window`, and `real` on the user's aspects):* the window (viewport ±1500 px) holds at most 4,140 items at 3840×2160 and 11,462 at 7680×4320 on the user's aspects (the synthetic aspects reach 4,333 and 11,977). Qt's 2^31 row limit is far above these. The 32-bit problem is in the total count, not in the window.
