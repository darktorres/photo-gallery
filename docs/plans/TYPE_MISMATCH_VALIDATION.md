# Type mismatch findings: adversarial validation

Validation of the 39 confirmed findings in `docs/plans/TYPE_MISMATCH_WORKFLOW_OUTPUT.json`, against the code at HEAD `c988b9c`.

Note (2026-11-06): the verdicts and line numbers are as of `c988b9c`, which is not in the current history (the history was squashed). Every source file the document cites still exists (33 paths), but several cited lines have moved: for example `add_folder` is at `crates/app/src/library.rs:161`, and the `views` table is at `crates/catalog/src/schema.rs:402`. This was not re-validated against the current tree. The S-17.3 to S-17.5 rows of `docs/REFACTOR_PLAN.md` list which of the surrogate-name items are still open.

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

### 0. crates/model/src/path.rs:47

**Verdict:** CONFIRMED. **Severity:** high → medium.

**Claim:** Two definitions of the same folder key disagree for any component with a word-final Σ: path::folder_key gives "ΑΣ" -> ...CF 82 (ς), PathRules::folder_key gives ...CF 83 (σ). Files are stored under the PathRules key and listed under the path key.

**Validation:** The mismatch and the failure hold at HEAD: stored folder keys come from PathRules::folder_key (σ), and the grid's scope from the legacy path::folder_key (ς), with no guard between them. Severity is lowered from high to medium: the trigger is a component with a word-final capital Σ (in practice Greek all-caps names), the failure is silent but loses no data, and the folder's files still appear in every ancestor's recursive listing.

### 1. crates/catalog/src/browse/read.rs:28

**Verdict:** PARTLY. **Severity:** high → medium.

**Claim:** The grid's scope range (list_grid and facets, via scope() at read.rs:27-36) is computed with the legacy text folder key, but file order keys are written with the PathRules key. The two disagree in two cases: a Windows folder whose component ends in capital sigma (final 'ς' in the scope, 'σ' in the stored key), and every Posix catalog (a 0x00 separator in the stored key, 0x01 in the scope). The ledger S-17.2 records the key change as intended but does not record this consumer still using the old key. Tests miss it: the scope tests use order_key_of (the text key) on both sides, and the fixture (fixture.rs) stores PathRules keys.

**Validation:** The Windows half (a folder with a word-final Σ shows an empty grid) is real and the same as entry 0. The Posix half is real code, but no shipped path builds or opens a Posix catalog on Windows, so it is latent. Overall severity is medium, not high.

**Scope:** Affected: folder listings (grid and facets, read.rs:27-36) of a Windows folder with a word-final capital Σ in one of its components (in practice Greek all-caps names). Also every non-root folder of a Posix catalog (rules.rs:131 against path.rs:54), but no shipped path creates or opens one: app.rs uses PathRules::NATIVE, which is Windows on Windows. Not affected: root listings, and Windows folders with no word-final Σ.

### 2. crates/writeback/src/lib.rs:224

**Verdict:** CONFIRMED. **Severity:** medium → medium.

**Claim:** A queued job holds two values: value (the newest request) and temp_value (what the finished temp file contains). The copy writes temp_value into the file, but the effect and the catalog update record job.value.

**Validation:** The mismatch is real: the copy writes the temp's content while done() records the newer job.value, which the code's own comments say should stay queued. It needs a failed copy and then a user change, which a locked file on Windows can cause, and the result is a silently lost turn or a reverted rating. Medium is kept.

### 3. crates/model/src/state.rs:29

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** A name is exact (WTF-8 with the surrogate, kept by the scanner per the S-17.5 test). The provider's file open uses the lossy display text, so it opens a different path. The background thumbs pass (crates/thumbs/src/lib.rs, via work.parts and path_buf) uses the exact bytes, so the two disagree on the same file. Not in the S-17.5 documented list.

**Validation:** Real: the viewer and on-demand thumbnails open the lossy display text, while the background pass uses the exact name, against S-17.5. Lowered from medium to low because it needs a name with an unpaired surrogate (rare on NTFS), and the grid thumbnail from the background pass still works.

### 4. crates/model/src/tree.rs:84

**Verdict:** PARTLY. **Severity:** medium → low.

**Claim:** The tree's merged path is display text (U+FFFD for an unpaired surrogate), but it is used as the listing's key and hashed back into order-key bounds, where the stored keys use the exact bytes.

**Validation:** The listing failure is real: a folder whose name has an unpaired surrogate lists nothing when clicked, because the scope key is built from U+FFFD bytes and the stored key from the WTF-8 bytes. The reveal() part of the scenario is wrong, so the verdict is PARTLY. Severity is low: it needs such a folder name, and the code comment acknowledges the lossy tree.

**Scope:** Affected: clicking a folder whose own name holds an unpaired surrogate. Its grid, non-recursive and recursive, is empty, although recursive listings of its ancestors still show its files. Not affected: reveal() and the tree's own row selection (tree-internal lookups on the same display text), and every folder whose name has no lone surrogate.

### 5. crates/catalog/src/dups.rs:66

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** SQL GROUP BY puts every NULL pixel_hash in one group, so identical_groups(Pixels) counts all videos as one identical-pixels group. identical_edges(Pixels) uses `pixel_hash IN (...)`, which is never true for NULL, so no edges are produced for them. The two readers of the same concept disagree. The count is reported as `Found.identical` (library/src/dups.rs:33 and :14-15, printed by the dups report).

**Validation:** The mismatch and the failure scenario hold exactly as written: identical_groups(Pixels) counts the NULL video group, while identical_edges returns nothing for it. The effect is limited to the pixel-count line of the dups report, which disagrees with its own group listing; no file is grouped or acted on wrongly.

### 6. crates/catalog/src/browse/read/details.rs:40

**Verdict:** CONFIRMED. **Severity:** medium → medium.

**Claim:** The same question, can this image hold its rating in the file, is answered two ways for an image with no usable format (not yet imported, or analysis failed, so the LEFT JOIN gives NULL format). details() says yes (is_none_or), and set_rating() says no (is_some_and), so set_rating never queues a write-back for it.

**Validation:** The scenario holds as written for a JPEG rated before its analysis succeeded, or after it failed, under default preferences: set_rating queues nothing, details() reports rating_in_file = true, and the info line omits the catalog-only note. The rating stays in the catalog, so it is not lost, but it never reaches the file and the info line is wrong about it, so medium is kept.

### 7. crates/catalog/src/browse/read/state.rs:67

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** A file's path is passed back to the platform to open it, but it is carried as lossy display text (unpaired surrogates become U+FFFD). The lossless RulePath / NativePath is the contract. The writeback crate refuses such names (text_path); the image providers do not.

**Validation:** The mismatch is real: the lossy display text is reopened as a path, so a surrogate-named file fails to open, and a sibling literally named with U+FFFD could supply pixels that are cached under this file's id. The trigger needs a lone surrogate in a file name, which most Windows tools will not create, and the damage is one file's thumbnail or viewer, so the severity is lowered from medium to low.

### 8. crates/catalog/src/lib.rs:285

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** A folder reference is stored as TEXT and used as a lookup key (it is the scope of list_grid), not as the lossless RulePath/Name bytes the rest of the catalog uses. The ledger records this as not done.

**Validation:** The mismatch is real: a view saved on a folder with a lone surrogate keeps the lossy text, and on reopen that text matches no stored folder bytes, so the view shows no files (or another folder whose name literally contains U+FFFD). It is tracked as S-17.3, but it needs a rare name, so low fits.

### 9. crates/catalog/src/browse/read/tree.rs:37

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The tree's node path is lossy text, and the desktop uses it as the lookup key for a folder's files. The lossless path is available (rules.join / PathRules) but is not used for the node.

**Validation:** The mismatch and the scenario hold: a surrogate-named folder's grid is empty while the tree shows its count. The affected set is folders with lone surrogates in their path, which is rare, and nothing is written or lost, so the severity is lowered from medium to low.

### 10. crates/app/src/images.rs:189

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** FileSource.path is a lossy display String, and the thumbnail and viewer providers open the original by it. The exact name the catalog holds (WTF-8 with unpaired surrogates on Windows) is replaced by U+FFFD before the file is opened. The ledger (S-17.5) says the file open takes native units, but the app's image path does not.

**Validation:** FileSource.path is a lossy display String, and the providers open the original by that text (maker.rs:69 opens the path as given). A file whose name holds an unpaired surrogate therefore opens a U+FFFD name, goes offline, and cannot be shown full size. The defect is real and not in the documented limits, but it needs an unusual name and fails visibly without data loss.

### 11. crates/app/src/library.rs:151

**Verdict:** PARTLY. **Severity:** medium → low.

**Claim:** Roots leave the app as lossy display text and come back as text that is parsed and used as the catalog lookup key. The round trip is not exact for names with unpaired surrogates, and the lookup miss is treated as success.

**Validation:** For a root whose stored path holds an unpaired surrogate, roots() returns display text and remove_folder silently does nothing (catalog lib.rs:945 matches exact bytes), so that folder cannot be removed by the app or the CLI. No current entry point stores such a root, though, because add_folder takes text and cannot add one, so the defect is effectively unreachable and was lowered from medium.

**Scope:** Affected: only a library root whose stored path contains an unpaired surrogate, which current entry points cannot create (only a typed 8.3 short-name path that resolves to a surrogate long name, or a hand-edited catalog). Not affected: any root whose name is valid text, including one with a literal U+FFFD, since its display text round-trips exactly; and the add path, which fails with an error for a surrogate folder (see index 14).

### 12. crates/cli/src/borders.rs:40

**Verdict:** PARTLY. **Severity:** medium → low.

**Claim:** The file list for the borders report is `Vec<String>` built with to_string_lossy, and the file is re-opened from that text. Non-UTF-8 names are also dropped silently by to_str() in is_image. Both lose the exact path the platform has (the lister keeps native units, S-17.5 is not applied to the borders command).

**Validation:** Two real effects remain: an image whose own name has a lone surrogate is silently left out of the folder report, and an image whose path has one elsewhere becomes U+FFFD text and errors with that lossy path. The finder's specific scenario (the walk pushes a U+FFFD file name) does not happen. This is a diagnostic report on rare NTFS names, so medium is too high.

**Scope:** Affected: (1) a walked image whose own name holds a lone surrogate is dropped with no row (line 46); (2) a walked image or an explicit file argument whose path holds a lone surrogate in any ancestor component, or in the argument itself, is pushed as U+FFFD text (lines 40, 54), so the open fails and the row is an error row with that text (or the open reads a different file if one with that name exists). Everything else is exact.

### 13. crates/app/src/reader.rs:194

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The folder the grid lists is the text merged path. The catalog turns it back into a key by parsing text, and the tree that produces the text is built from display() (lossy). The folder identity is therefore carried as text across the boundary, and the lookup is not exact for unpaired surrogates.

**Validation:** The folder tree and the grid use display text as a folder's identity, so selecting a folder whose name holds an unpaired surrogate lists nothing, though its files are stored under exact keys. The mismatch is real at HEAD and is a recorded open item in the S-17.3 ledger. The impact is limited to rare names, and no data is wrongly written or lost.

### 14. crates/app/src/library.rs:157

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The entry point for adding a library folder takes &str, so the folder is a lossy String before normalize_path sees it. The ledger's claim that add_folder carries typed roots is only true after normalization, not at the input.

**Validation:** add_folder takes text and the desktop passes it a Rust String built from Qt, so a folder whose path holds an unpaired surrogate cannot be added: normalize_path sees a U+FFFD name and fails. The S-17.5 row says add_folder carries typed roots, which holds only after normalization. The defect is real and not in the documented limits, but the result is a visible error for a rare name.

### 15. crates/metadata/src/lib.rs:350

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The Exif IFD pointer in a BigTIFF is an 8-byte offset (LONG8 or IFD8). The rest of the reader keeps offsets as u64/usize (`offset_at`, line 299), but this pointer is narrowed to u32 through `uint`, which returns None for any value above u32::MAX. The offset type disagrees with the file's offset width.

**Validation:** The u32 narrowing at lib.rs:350 is applied to the Exif IFD pointer, which is an offset, so DateTimeOriginal and DateTimeDigitized are silently lost for such files and analyze.rs:120 stores no date. The uint doc shows the u32 limit was chosen for values, not offsets, so the mismatch is real but rare. Severity lowered from medium to low for that reason.

### 16. crates/client/src/controller.rs:76

**Verdict:** PARTLY. **Severity:** medium → low.

**Claim:** A merged folder path is carried as String, but a Windows folder name can hold an unpaired UTF-16 surrogate, which the S-17 work keeps exact everywhere else (Name, PathRules). The QString to String conversion replaces it with U+FFFD before the catalog lookup, so the listing names a different folder than the one the user picked.

**Validation:** For a lone-surrogate folder opened from the tree the lookup misses and the grid comes up empty, so the outcome is real. The finding places the loss at the controller's String boundary, where it does not occur, and retyping Listing.folder alone would not fix it. The impact is a rare edge name with no data loss, and the next start falls back to all folders (start.rs:4), so severity is lowered from medium to low.

**Scope:** Affected: a folder whose name has an unpaired UTF-16 surrogate on Windows, when opened from the folder tree (or any QML string built from the tree's display text), and the session that then records it. Not affected by the controller's QString to String conversion as the cause: it receives text already replaced by U+FFFD at catalog tree.rs:37 and folders.rs:235, so changing only the controller or shell boundary would not fix it. A real fix needs the tree and Details keyed by exact names (S-17.4, S-17.3).

### 17. crates/app/src/reader.rs:191

**Verdict:** CONFIRMED. **Severity:** medium → low.

**Claim:** The empty string is used for two meanings: 'all folders' (in the listing and the reader doc) and 'a drive root' (in merged_path). With recursive=false, scope_keys('') gives the range [0x00, 0x01), which holds only files directly in a drive root, not all folders. The reader doc says '' means all folders with or without recursive. The test comment at tests.rs:494 documents the drive-root reading. The two documents disagree.

**Validation:** The code and its test agree that a non-recursive empty folder means drive-root files only, but reader.rs:191 states that empty means all folders with or without recursive, so the caller contract is wrong. The reachable effect is that All folders with Include subfolders unchecked shows only files sitting directly in a drive root. That is a real doc/behaviour contradiction, but not the defect the claim suggests.

### 18. crates/platform/src/dir.rs:183

**Verdict:** STALE. **Severity:** low → none.

**Claim:** st_dev is a 64-bit device number, but the volume id is truncated to 32 bits, and the device number is not stable across reboots. Two devices can share one identity, and the same device can change identity.

**Validation:** The truncation the finding describes is gone at HEAD: the Unix volume is a whole u64 and the plan and ledger both record it as done. Only a stale doc comment in ids.rs still describes the 32-bit cast, so the finding is stale rather than a live defect. The residual Linux device-number instability is a documented open item, not the truncation this finding claims.

### 19. crates/platform/src/dir.rs:153

**Verdict:** REFUTED. **Severity:** low → none.

**Claim:** The 64-bit listing class cannot carry a 128-bit file ID (ReFS), so the identity loses its upper 64 bits on such volumes. The model's FileIdentity is documented as the file's identity, but the platform only supplies a truncation of it.

**Validation:** The scanner refuses a non-NTFS root before it lists anything, so a ReFS volume never reaches the 64-bit listing and the claimed collision cannot occur. The documented refusal (STORAGE.md:39, ledger row REFACTOR_PLAN.md:823) matches the code, so the finding is refuted rather than a known limit.

### 20. crates/catalog/src/lib.rs:273

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** A rating job's value is an i64 with no range check in the schema, but the code treats it as a 0 to 5 rating. The cast to u8 wraps values of 256 and above, and the min(5) silently clamps values from 6 to 255.

**Validation:** The missing range check on file_writes.value is real, but the claimed failure at value 6 is defeated by the read-back at writeback/src/lib.rs:253. Only values whose low byte is 0..=5 are written, and then the file and catalog agree on the wrapped value, so the damage is a silently changed requested value, reachable only from a catalog the app did not write. Low severity is kept, with the scope narrowed as stated below.

**Scope:** Only file_writes rows of kind 'rating' whose value's low byte is 0..=5 are affected (256 writes and records 0; 261 writes and records 5), and there the file and the catalog agree on the wrapped value. Rows whose low byte is 6..=255 (e.g. value 6) are not written: the read-back fails with 'the rating didn't read back after writing it', the job is retried up to MAX_ATTEMPTS (writes.rs:23) and an error is recorded, so the catalog-says-5-file-holds-6 state does not arise. The line in the finder's evidence (lib.rs:273) is now lib.rs:295-298. Only catalogs the app did not write can reach this.

### 21. crates/catalog/src/lib.rs:800

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** A kind string the Kind enum does not know is silently read as Image instead of being an error. The schema admits any text, and the code treats an unknown value as impossible without checking it.

**Validation:** The silent fallback is real and the scenario holds as written, but only a hand-edited or foreign catalog can produce an unknown kind, so the impact is low. The repeated re-analysis is a cost for such a catalog, not a correctness hazard for catalogs the app writes.

### 22. crates/thumbcache/src/lib.rs:327

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** The cap is an i64 that `open` accepts with the single check `cap < 0`, so Some(0) passes. The crate's own parser treats 0 as a value that is not a cache size, but the typed boundary used by the writer does not enforce that. A cap of 0 is admitted and written into the shared cache file.

**Validation:** The validation gap is real: open() accepts a zero cap, persists it for every later opener, and then stores no thumbnail, while the parser's own documented rule (lib.rs:106-107) refuses it. The scenario holds as written, but only a direct library caller can reach it, so the low severity stands.

### 23. crates/app/src/yandex.rs:423

**Verdict:** REFUTED. **Severity:** low → none.

**Claim:** The folder remembered for the next save is stored as lossy text in Prefs.yandex_folder: String, while the save itself used the exact PathBuf. The remembered value does not equal the folder saved into.

**Validation:** The mismatch the claim describes cannot occur: the save and the stored preference are built from the same lossy String, so they are equal. The real defect is the earlier desktop conversion at yandex.rs:418, which is covered by index 14 and not by this preference.

### 24. crates/app/src/edit.rs:12

**Verdict:** REFUTED. **Severity:** low → none.

**Claim:** The type of a user rating admits 0 stars, which the model documents as impossible for a user rating (1 to 5, with 0 meaning no rating, i.e. None). The only guard is in one caller; app::rate and set_rating store a 0 without a check.

**Validation:** The CHECK constraint makes the scenario's stored 0 impossible: the call errors and nothing is written, so no file shows zero stars or writes Some(0) back. The API does lack a check of its own, which is a weaker contract, but it produces no data defect, so the finding is refuted.

### 25. crates/app/src/prefs.rs:24

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** A file's identity is FileId everywhere else in the app, but the preference holds a raw i64 with -1 as 'none'. The type cannot tell a real id from the sentinel, and nothing checks that the id still names a file in the current catalog.

**Validation:** The raw i64 with a -1 sentinel where FileId is used elsewhere is a real type inconsistency (prefs.rs:24). The one real effect is that a rebuild that reuses an id selects a different file at startup, which the module doc already accepts. It selects rather than opens, and a missing id selects nothing, so the impact is low.

**Scope:** Affected: the startup selection after a catalog rebuild or hand-edit that reuses last_file's id for another file that is in the current list. Not affected: a stale id that names no file in the list (no selection, via indexOf returning -1); the -1 sentinel, which cannot collide with a generated id; and the viewer, which is not reopened at startup (Command::Select only).

### 26. crates/app/src/yandex.rs:292

**Verdict:** REFUTED. **Severity:** low → none.

**Claim:** The name of each saved file is built from the photo's name as lossy text. Each output name is therefore not the photo's name, and the uniqueness check runs on the lossy name.

**Validation:** The failure scenario cannot happen: the Yandex search reads the photo by the same lossy path, so a surrogate-named photo never gets results to save. stem_of is lossy in code, but that conversion is unreachable for such names. Nothing is written with the wrong name.

### 27. crates/cli/src/lib.rs:88

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** The CLI accepts any u32 for the thumbnail height, including 0, which the model's THUMB_HEIGHT range excludes. Nothing at the parser or in pg_thumbs::run refuses 0.

**Validation:** The mismatch is real: the height is an unchecked u32 with no up-front check, and 0 is outside THUMB_HEIGHT. Height 0 makes every file fail at encode and stores nothing, without a top-level error. It needs a deliberate operator value of 0, so low stands.

### 28. crates/cli/src/lib.rs:69

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** Folder and file paths given on the command line are String fields, parsed by clap, so a path with unpaired surrogates cannot reach normalize_path exactly. The paths are typed as text where OsString or PathBuf would be exact.

**Validation:** The String fields are a real type gap for names with an unpaired surrogate, but clap refuses such arguments with an error, so nothing silently targets the wrong folder. Only paths with a lone surrogate are affected, so low stands.

**Scope:** Affected: the CLI's text path arguments (scan roots, remove root, import --again) cannot express a path with an unpaired surrogate; clap rejects it with an invalid_utf8 error before any folder is touched. Not affected: the 'different folder' outcome, which clap prevents, and scan with no arguments, which rescans the catalog's own roots (cli/src/lib.rs:253).

### 29. crates/cli/src/lib.rs:59

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** A megabyte count is multiplied by 2^20 in usize with <<, which discards the high bits silently in release and debug alike. Any value at or above 2^44 MB has no representable byte count.

**Validation:** The silent wrap is real, and the finding's arithmetic checks out (2^44 MB gives 0; larger values wrap to small budgets). The only effect is serialised reads for an absurd argument, with no data or hang impact, so low stands.

### 30. crates/testkit/src/lib.rs:67

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** The brightness of frame i is computed in u8 (`i as u8 * 60`) with frames a usize. The doc promises 'each after it brighter', but the multiplication overflows at i = 5 (`5 * 60 = 300`) and wraps for larger frame indices.

**Validation:** The overflow is real, but the finder's example is off by one: gif(5) is safe, and the defect needs six or more frames. No caller passes more than three, and the helper is test-only, so low stands.

**Scope:** Affected: pg_testkit::gif with frames >= 6 (the loop reaches i = 5, and `5 * 60 = 300` overflows u8): a debug panic 'attempt to multiply with overflow', and in release brightness that wraps (i = 5 gives 44, i = 6 gives 104). Not affected: gif(1) and gif(3) (the only current callers) and gif(5), whose largest i is 4.

### 31. crates/exiftool/src/lib.rs:79

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** ExifTool's `-@ -` protocol uses one argument per line, but a path is carried as an unescaped &str that may contain '\n'. Valid UTF-8 with a newline is a legal POSIX file name, and it passes text_of. The text form of the path cannot represent the argument boundary, so one path becomes several ExifTool arguments.

**Validation:** The mismatch is real: a path carried as text with a newline becomes several ExifTool arguments, so copy_tags with -overwrite_original can also rewrite another named file in place. It is confined to POSIX names with a newline in a library folder, so the low severity is unchanged.

### 32. crates/render/src/player.rs:35

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** The animation's file path is held as a String and read with std::fs::read. A name with no UTF-8 form cannot be carried, and the path is not the lossless Name or RulePath type the rest of the platform uses.

**Validation:** The String-typed path and the silent failure are real, and only animated GIF, APNG and WebP with names that have no UTF-8 form are affected. But the loss happens in the catalog's display text, so the named player boundary is not the cause and changing only the player would not fix it. Severity stays low, as the doc records it as an accepted limit.

**Scope:** Affected: animated GIF, APNG and WebP files whose names have no UTF-8 form (an unpaired surrogate on Windows, or non-UTF-8 bytes on POSIX), since only the animation player takes the path. Not affected: still images, which are keyed by file id. The String in player.rs:35 and animation.rs:112 is not where the name is lost: Details.path (details.rs:34) is already display text when the QML source is set (gallery.rs:963).

### 33. crates/metadata/src/lib.rs:157

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** The XMP packet is bytes whose encoding the file declares (UTF-8, UTF-16 or UTF-32, marked by a BOM, per the XMP specification), but it is decoded as lossy UTF-8 and handed to an XML reader that expects text. The packet's type disagrees with its contents.

**Validation:** The mismatch is real and not yet handled: BOM-declared bytes are decoded as lossy UTF-8, so rating and dates are dropped for a JPEG whose XMP is UTF-16 (the EXIF fallback still applies). The planned port would add BOM and UTF-16/32 detection (EXIFTOOL_PORT_PLAN.md:204), which is not done at HEAD. Such packets are rare, and the spec point rests on ExifTool's behaviour rather than the spec text, so severity stays low.

### 34. crates/desktop/src/gallery.rs:932

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** A URL-to-folder conversion goes through a lossy String and back to PathBuf. Unpaired UTF-16 surrogates become U+FFFD, so the path no longer names the folder the user picked. The fallback branch also keeps the raw URL text as a path.

**Validation:** The lossy conversion is real and breaks the exactness contract written at crop.rs:43-45 and in the S-17.5 ledger row, and unpaired-surrogate names are valid on NTFS and survive Qt. The non-local-URL fallback cannot be reached from any UI or command-line path, so the claim is narrower than written. Severity stays low because the effect is limited to one rare folder-name class.

**Scope:** Affected: a Save cropped copies folder whose name contains an unpaired UTF-16 surrogate; the copies go to a different, stray U+FFFD folder that create_dir_all creates. Not affected: folder names without unpaired surrogates, which round-trip exactly, and the non-local URL fallback, which no UI or command-line path reaches.

### 35. crates/desktop/src/yandex.rs:418

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** The download destination is converted from a QUrl through a lossy QString-to-String step, then used as a filesystem path. Non-local URLs fall back to the raw URL text.

**Validation:** The lossy conversion is real and reachable for surrogate-named folders, but the result is a reported write failure rather than a download in a wrong place, unless a same-named U+FFFD folder exists. The finding's wrong-location outcome and its relative-path fallback do not hold. Severity stays low.

**Scope:** Affected: a download folder whose name contains an unpaired UTF-16 surrogate. Each file then fails with a reported write error, and the lossy path is remembered as the next default folder. Misplacement happens only if a folder literally named with U+FFFD already exists. Not affected: ordinary folder names and the non-local fallback.

### 36. crates/desktop/src/library.rs:299

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** A folder path is passed to add_folder as a lossy String. The non-local fallback passes URL text into the normalize-as-path step.

**Validation:** The String round trip is real and contradicts the S-17.5 ledger claim, but a surrogate-named folder is refused with an error rather than stored as a corrupt root, so the finding's outcome is wrong. Severity stays low because the only effect is that such a folder cannot be added.

**Scope:** Affected: adding a folder whose name contains an unpaired UTF-16 surrogate fails with a can't-open error. A wrong folder is added only if a real sibling is literally named with U+FFFD. Not affected: folders with ordinary names, and the non-local fallback.

### 37. crates/desktop/src/library.rs:313

**Verdict:** PARTLY. **Severity:** low → low.

**Claim:** The removal key is the root's display text, which the QML sends back as a QString. It is converted to a lossy String and re-parsed as a rule path. The lossless RulePath is not used on this round trip.

**Validation:** Only roots whose stored name has an unpaired surrogate fail to remove, and such a root is reachable only through a CLI short-name alias. The finding's UI outcome is also wrong: the root stays listed after the silent Ok(()), so the failure is a silent no-op, not a false removal. Severity stays low.

**Scope:** Affected: removing a root whose stored name contains an unpaired UTF-16 surrogate; the removal is a silent no-op and the root stays listed. Such a root can only be stored by a CLI scan given a short-name alias of the folder. Not affected: every root with an ordinary name, which round-trips exactly.

### 38. crates/desktop/src/gallery.rs:429

**Verdict:** CONFIRMED. **Severity:** low → low.

**Claim:** A u32 counter is cast to i32. Past 2^31 values it becomes negative, which the QML reads as the 'no version' sentinel.

**Validation:** The i32 cast turns versions at or above 2^31 negative, and GridCell.qml then reads those cells as unversioned, so the finding holds as written. It needs about 2.1 billion version bumps in one process, so it stays low and practically unreachable.

## Not validated

The workflow refuted 7 candidates. The JSON records only the count, so their content is not available here. They can't be validated or reported on until the workflow is rerun and its refuted entries are kept.
