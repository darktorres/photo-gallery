# Type mismatch findings: adversarial validation

Validation of the 39 confirmed findings in `docs/TYPE_MISMATCH_WORKFLOW_OUTPUT.json`, against the code at HEAD `c988b9c`.

Re-checked 2026-11-11 at HEAD `362888c`. Each remaining section has a **Status at HEAD** line giving the current verdict and current file:line. The original c988b9c verdicts and claims are kept as history. Findings that are fixed at HEAD have been removed from the document. The re-check read the current source for all 39 indices, and I spot-checked the key claims myself (the `folder_key` consolidation, the `IS NOT NULL` guard, `Cap::from_bytes`, the `uint` narrowing, the lossy XMP decode, the unchecked `buffer_mb << 20`, the `cast_signed` version cast). The reasoning is read-only and nothing was built or run.

Note (2026-11-06): the verdicts and line numbers are as of `c988b9c`, which is not in the current history (the history was squashed). Every source file the document cites still exists (33 paths), but several cited lines have moved: for example `add_folder` is at `crates/app/src/library.rs:161`, and the `views` table is at `crates/catalog/src/schema.rs:402`. This was not re-validated against the current tree. The S-17.3 to S-17.5 rows of `docs/REFACTOR_PLAN.md` list which of the surrogate-name items are still open.

## Status at HEAD 362888c

The 30 findings that are FIXED at HEAD have been removed from this document: 0–14, 16, 17, 20–23, 26, 28, 30–32, 34–37. Finding 18, stale at HEAD, has also been removed after its stale comment in `crates/model/src/ids.rs` was corrected. Finding 19, refuted at HEAD, has also been removed. The sections below are the 7 that remain.

| Status | Count | Indices |
|---|---|---|
| STILL PRESENT (low) | 5 | 15, 27, 29, 33, 38 |
| PARTLY still present (low) | 1 | 25 |
| Open contract gap, no data defect (low) | 1 | 24 |

Only 6 findings still describe a live defect, all rated low. The surrogate-name items are almost all fixed: exact names flow through `Name`/`RulePath`/`disk_path`, and the desktop picks folders by typed values rather than by display text.

## Method

- The workflow produced 46 results: 39 confirmed and 7 refuted. The JSON keeps only the count of the refuted ones, so those 7 are not validated here.
- Six agents each took a fixed set of the 39 confirmed findings, by index: 0–4; 5–9, 20–21; 10–11, 13–14, 17, 23, 25–26; 12, 24, 27–30; 18–19, 34–38; 15–16, 22, 31–33. Every index was covered once.
- Each agent tried to disprove its findings. It read the cited code and its callers, looked for guards, invariants and documented limits, checked reachability on Windows, and checked whether the defect still exists at HEAD. Each verdict records an attack, the evidence and a reason.
- The evidence was checked mechanically. All 344 quotes were found in their cited files within eight lines of the cited line number, and no cited file is missing.
- Nothing was built or run. A verdict rests on reading the code, not on a reproduction. The agents are LLM-driven, and the mechanical check confirms that the quotes exist, not that each verdict is right.

## Summary

| Verdict | Count |
|---|---|
| CONFIRMED | 20 |
| PARTLY | 14 |
| REFUTED | 4 |
| STALE | 1 |

34 of 39 findings survive as defects. 21 findings changed severity: 2 high to medium, 14 medium to low, and 5 low to none. Originally 2 high, 16 medium and 21 low; after validation 4 medium, 30 low and 5 none, with no high.

Half of the survivors (17 of 34) involve a file or folder name with an unpaired UTF-16 surrogate. NTFS allows such names, and the app's display text turns them into U+FFFD. The S-17 work keeps exact names in the catalog, but these consumers still use the lossy text. The usual results are an empty folder grid, a file that fails to open, or a refused add. The validators describe no overwrite of existing data for them. Two of them can misplace output: cropped copies (34) and downloads (35) can land in a stray folder, but only when a folder literally named with U+FFFD already exists.

The Greek sigma problem (findings 0 and 1) is not in that group. A folder whose name has a word-final capital Σ can list as empty on Windows, because two folder-key functions disagree about the final letter. It is real and rated medium. The folder's files still appear in every ancestor's recursive listing, and nothing is lost.

## Findings

### 15. crates/metadata/src/lib.rs:350

**Status at HEAD 362888c:** STILL PRESENT (low). `lib.rs:350` is now a test helper. The narrowing is in `uint` (`crates/metadata/src/tiff.rs:155-163`), which returns `None` above `u32::MAX` for LONG8/IFD8, and `exif.rs:60` uses it for the Exif IFD pointer. Dates are lost only when the XMP fallback (`crates/metadata/src/lib.rs:95`) also has none.

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The Exif IFD pointer in a BigTIFF is an 8-byte offset (LONG8 or IFD8). The rest of the reader keeps offsets as u64/usize (`offset_at`, line 299), but this pointer is narrowed to u32 through `uint`, which returns None for any value above u32::MAX. The offset type disagrees with the file's offset width.

**Validation:** The u32 narrowing at lib.rs:350 is applied to the Exif IFD pointer, which is an offset, so DateTimeOriginal and DateTimeDigitized are silently lost for such files and analyze.rs:120 stores no date. The uint doc shows the u32 limit was chosen for values, not offsets, so the mismatch is real but rare. Severity lowered from medium to low for that reason.

### 24. crates/app/src/edit.rs:12

**Verdict (HEAD 362888c):** OPEN, a contract gap with no data defect. **Severity:** low.

**Claim:** The user-rating type admits 0 stars. `Rating` is shared with file ratings, where 0 is valid, so `Rating::new` accepts 0 (`crates/model/src/rating.rs:24-29`). `Edit::rate` takes `Option<Rating>` (`crates/app/src/edit.rs:27`), so the API accepts a 0 user rating. The model documents a user rating as 1 to 5, with 0 meaning no rating (None).

**Validation:** No data defect. The desktop maps 0 stars to `None` before calling `rate` (`crates/desktop/src/gallery/edits.rs:77`), and the `ratings` table's `CHECK (rating BETWEEN 1 AND 5)` (`crates/catalog/src/schema.rs:647-649`) rejects a stored 0, so a 0 errors rather than writing. The gap is that the type enforces none of this: the guarantee lives in one caller and in the schema.

**Hardening (planned, not done):** split the one `Rating` type, which currently serves two domains.

- **User ratings (1 to 5):** the `ratings` table, `Write::Rating` and `Command::Rate`. These become `UserRating`, whose constructor rejects 0. `Grid::Rate(stars)` (`crates/client/src/controller.rs:337`) then fails for 0 at the boundary, and "0 stars clears the rating" becomes an explicit `None` branch in the desktop instead of a `filter`. Callers to change: `crates/desktop/src/gallery/edits.rs:77`, `crates/catalog/src/writers.rs:1071`, `crates/catalog/src/browse/fixtures.rs:82`, and the user-rating read at `crates/catalog/src/browse/read/grid.rs:70`.
- **File ratings (0 to 5):** the `file_ratings` table and metadata, where 0 means "explicitly unrated" (`crates/metadata/src/lib.rs:41`). These become `FileRating`, which keeps 0. The import read-back at `crates/catalog/src/writers.rs:373` and the metadata constructor at `crates/metadata/src/lib.rs:94` use it. Rejecting 0 here would make every import rewrite the `file_ratings` row of an explicit-0 file.

Done when `Rating::new` no longer exists, `Edit::rate` takes `Option<UserRating>`, the desktop filter is gone, and the tests in `writers.rs` and `fixtures.rs` build with the new types. The wire format and the `CHECK` constraints stay unchanged.

**History:** the c988b9c verdict was REFUTED, with the same no-data-defect reasoning. That verdict still holds. The section now tracks the contract gap above, not the original claim.

### 25. crates/app/src/prefs.rs:24

**Status at HEAD 362888c:** PARTLY, still present (low). `app::Prefs.last_file` is `Option<FileId>` (`crates/app/src/prefs.rs:34-35`). The -1 sentinel survives only as the on-disk encoding (`prefs.rs:65-79`) and in the Qt bridge, which takes an i64 (`crates/desktop/src/settings.rs:40-41, 73-77`). The ambiguity remains at that bridge.

**Fix (planned, not done):** persist the file's identity, not its catalog row id, and remove the sentinel at every layer. Row ids are local to a catalog and a rebuild reuses them, which is the defect in this finding. The file's identity (`FileIdentity`: volume plus NTFS file ID) already keeps a file's row across moves and renames.

- **Persist identity:** `Prefs.last_file` becomes `Option<FileIdentity>`. At startup, resolve it to a `FileId` with one lookup on `files(volume, file_ref)`, which has a UNIQUE index. No match means nothing is selected.
- **Disk format:** write `null` for none. Read a legacy negative number as none. Read a legacy non-negative number as a row id, resolve it once, and re-save it as an identity.
- **Qt bridge:** `settings.rs` stops passing an `i64` with `-1` for none. The property carries an optional value, empty for none. The call sites change: `crates/desktop/qml/Main.qml:113` (assigns the sentinel) and `Main.qml:224` (compares it through `galleryModel.indexOf`).
- **NULL identity:** a displaced row (`files.volume` NULL) has no identity to remember, so it selects nothing.
- **Tests:** identity round-trip, a legacy `-1` reads as none, a legacy row id resolves once, and a rebuild that reuses a row id selects nothing for the other file.

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** A file's identity is FileId everywhere else in the app, but the preference holds a raw i64 with -1 as 'none'. The type cannot tell a real id from the sentinel, and nothing checks that the id still names a file in the current catalog.

**Validation:** The raw i64 with a -1 sentinel where FileId is used elsewhere is a real type inconsistency (prefs.rs:24). The one real effect is that a rebuild that reuses an id selects a different file at startup, which the module doc already accepts. It selects rather than opens, and a missing id selects nothing, so the impact is low.

**Scope:** Affected: the startup selection after a catalog rebuild or hand-edit that reuses last_file's id for another file that is in the current list. Not affected: a stale id that names no file in the list (no selection, via indexOf returning -1); the -1 sentinel, which cannot collide with a generated id; and the viewer, which is not reopened at startup (Command::Select only).

### 27. crates/cli/src/lib.rs:88

**Status at HEAD 362888c:** STILL PRESENT (low), moved. The field is `Thumbs { height: u32 }` at `crates/cli/src/args.rs:85-88`. Neither the parser nor `pg_thumbs::run` (`crates/thumbs/src/lib.rs:172`) refuses 0. The 16..=2048 range is enforced only in the client (`crates/client/src/image.rs:42, 58-60`). The zero-height failure mode was not traced.

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** The CLI accepts any u32 for the thumbnail height, including 0, which the model's THUMB_HEIGHT range excludes. Nothing at the parser or in pg_thumbs::run refuses 0.

**Validation:** The mismatch is real: the height is an unchecked u32 with no up-front check, and 0 is outside THUMB_HEIGHT. Height 0 makes every file fail at encode and stores nothing, without a top-level error. It needs a deliberate operator value of 0, so low stands.

### 29. crates/cli/src/lib.rs:59

**Status at HEAD 362888c:** STILL PRESENT (low). The cited line was wrong; the code is `crates/cli/src/args.rs:53`, `opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1) << 20)`, and nothing clamps `buffer_mb` before the unchecked shift. The wrapped budget still reaches `Pool::new` (`crates/jobs/src/pipeline.rs:118`).

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** A megabyte count is multiplied by 2^20 in usize with <<, which discards the high bits silently in release and debug alike. Any value at or above 2^44 MB has no representable byte count.

**Validation:** The silent wrap is real, and the finding's arithmetic checks out (2^44 MB gives 0; larger values wrap to small budgets). The only effect is serialised reads for an absurd argument, with no data or hang impact, so low stands.

### 33. crates/metadata/src/lib.rs:157

**Status at HEAD 362888c:** STILL PRESENT (low). The XMP packet is still decoded as lossy UTF-8 with `String::from_utf8_lossy` (`crates/metadata/src/xmp.rs:62`), called from `crates/metadata/src/lib.rs:75`. There is no BOM or UTF-16/32 detection in `crates/metadata/src`. `lib.rs:157` is now inside a test.

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** The XMP packet is bytes whose encoding the file declares (UTF-8, UTF-16 or UTF-32, marked by a BOM, per the XMP specification), but it is decoded as lossy UTF-8 and handed to an XML reader that expects text. The packet's type disagrees with its contents.

**Validation:** The mismatch is real and not yet handled: BOM-declared bytes are decoded as lossy UTF-8, so rating and dates are dropped for a JPEG whose XMP is UTF-16 (the EXIF fallback still applies). The planned port would add BOM and UTF-16/32 detection (EXIFTOOL_PORT_PLAN.md:204), which is not done at HEAD. Such packets are rare, and the spec point rests on ExifTool's behaviour rather than the spec text, so severity stays low.

### 38. crates/desktop/src/gallery.rs:429

**Status at HEAD 362888c:** STILL PRESENT (low). The cited line was wrong; the cast is at `crates/desktop/src/gallery/model.rs:38`, `QVariant::from(&(self.grid.version(at).cast_signed()))`. The `u32` counter (`crates/client/src/grid.rs:169-170`) wraps negative at 2^31, and `GridCell.qml` (`crates/desktop/qml/GridCell.qml:25-26`) gates on `>= 0`, so such cells read as unversioned. Practically unreachable.

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** A u32 counter is cast to i32. Past 2^31 values it becomes negative, which the QML reads as the 'no version' sentinel.

**Validation:** The i32 cast turns versions at or above 2^31 negative, and GridCell.qml then reads those cells as unversioned, so the finding holds as written. It needs about 2.1 billion version bumps in one process, so it stays low and practically unreachable.

## Not validated

The workflow refuted 7 candidates. The JSON records only the count, so their content is not available here. They can't be validated or reported on until the workflow is rerun and its refuted entries are kept.
