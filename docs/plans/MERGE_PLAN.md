# Merge split images into one file, with stacking

Status: **design, not built.** The decisions below are made. The few choices left to the user are in §12.
Earlier drafts (with the full measurement log) are not in the repo.

## 1. Summary

The user selects two or more files that are pieces of one picture, and chooses **Merge into one image**. The app
works out the arrangement, shows a preview, and on confirmation writes one PNG beside the first piece, in the library.
The merged picture is then **stacked**: the gallery shows only the merged picture, with a badge that opens the
originals. The originals are never changed. Stacking is built for merges first and extended later.

## 2. Scope

In:

- Two or more pieces in one run, sitting **edge to edge** (no overlap, no gap, no scaling), in a row, a column, or a
  grid. A ragged last row or column is allowed.
- Input: whatever the decoder reads (JPEG, PNG, WebP, GIF first frame, TIFF, BMP), decoded upright.
- Output: 8-bit RGB PNG, lossless.
- Gallery multi-select, the merge action with its preview, and the stack badge and open view.

Out:

- Overlapping pieces, panorama blending, exposure matching, rotation or skew, pieces of different scales.
- A JPEG output or a lossless JPEG join.
- Changing originals, their ratings, or their marks.
- Manual ordering of pieces (a later addition, if a real case needs it).

## 3. Decisions

| Decision | Reason |
| --- | --- |
| Edge-to-edge pieces, PNG output, gallery UI in the first delivery | Chosen by the user. |
| Pairwise edge score: the relative seam score in §4 | Correct on all photo neighbours measured. The flat-area score is 0, so acceptance depends on the ratio and the layout checks, not the value. |
| Mutual-best links with a ratio of 1.3 | 1.3 gave no wrong layouts on 80 measured images; 1.6 gave three, from one 2×2 photo. |
| Adjacency rule: every pair of pieces side by side in the result needs an accepted link | Without it, one wrong link plus correct ones can form a valid rectangle. With it, the 1.6 errors are gone. |
| A preview must be confirmed before anything is written | The checks reduce wrong layouts but cannot prove one correct. |
| Output beside the first piece, in the library, then rescan that folder | Stacking must show the merged file, and the file is only shown once it is catalogued (§8). |
| Metadata: copy the tags, then set orientation to 1 | Verified: the copy carries the source's orientation, which would turn the merged picture sideways. |
| Output never overwrites: a numbered name is used | A merge is a new picture; an earlier one must not be lost. |
| Stacking: hidden members everywhere the library lists files | Counts must match what is shown. |

## 4. Algorithm

1. **Decode.** Each piece is decoded upright at full size (`Decoder::decode_into`, `MinSize::Full`).
2. **Edge score.** For pieces A (left) and B (right), with equal heights:
   `score(A, B) = mean|A.last − B.first| / (mean|A.last − A.second_last| + mean|B.first − B.second| + ε)`.
   Vertical pairs use rows, with equal widths. The score is 0 for a flat seam; it is never accepted on value alone.
3. **Links.** For each piece, its best right and best bottom neighbour. A link is accepted only if the best is clearly
   better than the second-best (ratio ≥ 1.3), and the piece is the other piece's best in that direction (mutual).
4. **Positions.** Place the first piece at (0, 0) and walk the accepted links. A conflicting position, or a piece not
   reached, refuses the layout.
5. **Rectangle.** The bounding box must be full, or its empty cells must be only in the last row, or only in the last
   column.
6. **Adjacency.** Every two pieces side by side in the result must have an accepted link in that direction.
7. **Compose.** Copy each piece to its place and stream the PNG row by row (`Writer::stream_writer`, present in the
   pinned `image-png` fork). The canvas is `width × height × 3` bytes.

A refusal names the step that failed: not reached, not a rectangle, missing link, or unclear link. The preview shows the
layout the checks found, numbered, so the user can see a wrong order before writing.

## 5. Failure cases

| Case | Behaviour |
| --- | --- |
| Fewer than two pieces | Action disabled. |
| A piece cannot be decoded | Refused, naming the file. |
| Layout refused by a step in §4 | Refused, with the step named. No output. |
| Canvas over the decoder cap (`max_decode_bytes`, 512 MiB) | Refused before allocation. Four 24-megapixel pieces (288 MB) fit. |
| Output name taken | Numbered name. Never overwritten. |
| An original changed since the scan | Refused, using the crop's check (size and mtime against the catalog). |
| Stop during the job | The partial file is removed (the crop's `TempFile` pattern). |

## 6. The gallery and the dialog

- **Selection.** The gallery has one current item and an anchor today; it has no multi-select. Add Ctrl-click to toggle,
  Shift-click for a range, and a count in the status bar. The client already has `pg_client::picks::Picks` (an index
  set with an anchor and range), the natural base. The grid's list is loaded whole (`list_grid`, no `LIMIT`) and replaced
  on each listing. A refresh keeps the selection, and a new listing resets it.
- **Action.** "Merge into one image" is enabled for two or more picked files. Its key: G or J are free in
  `keys.rs` and the desktop; B, F, H, K, L, M, N, R and Z are taken. A menu entry is always present.
- **Dialog** (`MergeDialog.qml`, modelled on `CutBordersDialog.qml`): thumbnails of the pieces in the found layout,
  numbered; the output name and folder; the state (found, or the refusal reason); **Merge** (writes the file) and
  **Cancel**. Merge is disabled until a layout is found.
- **Notices.** Three new kinds in `pg-model`'s `notice` module: merged (event, with the output path), refused (event,
  with the step), failed (event, with the reason). Each needs a code, a level, a source, a lifecycle, and its actions.
  The words go in `pg-client`'s `text` module.

## 7. Output, metadata, and the catalog

- **Written by:** `pg-library`'s background job, next to `save_crops`, with progress, pause and stop. `pg-merge` only
  computes; it does no file or catalog work.
- **Location.** Beside the first picked piece, named `<first stem>_merged.png`. This is inside a library folder, which
  the crop refuses for its copies. The merge deliberately differs: a merged file that is not catalogued cannot be
  stacked. After the write, the job calls the same `LibraryService::scan` on that root that `add_folder` calls.
- **Metadata.** `pg_exiftool`'s `copy_tags` (`-all:all`), then `write_orientation(path, out, 1)`. Verified against a scratch
  PNG: `Make` survives, and orientation reads back as normal.
- **Crop comparison.** The crop writes its copies without catalogue registration, and scans run only on add-folder and
  rescan (checked in `crates/app/src/library.rs` and the scan queue). So a file outside a scanned folder is never seen.

## 8. Stacking

A stack groups files so the gallery shows one representative and hides the rest.

- **Structure.** One root (shown) and one or more members (hidden). A file is in at most one stack; stacks don't nest.
  Stacking is non-destructive: members stay in the library and on disk, and their rows are unchanged. Unstacking only
  removes the grouping. Deleting a root is a separate action with its own confirmation.
- **Data model.** The catalog schema is frozen at baseline version 1, and `MIGRATIONS` is empty, so this is the first
  migration: `stacks(id, root_file_id, kind, created)` and `stack_members(file_id PRIMARY KEY, stack_id, position)`.
  `kind` is a text key (`merge` now), so later kinds need no schema change. `CATALOG_SCHEMA_VERSION` is raised, and the
  migration tests in `schema.rs` and `lifecycle.rs` are updated.
- **Hiding.** Three readers list files, each with its own filter, so the hiding goes in all three through one shared
  predicate (a `NOT EXISTS` on `stack_members` for a file that is not a root):
  - the grid (`list_grid`) and facets, through `scope` (`browse/read/scope.rs`);
  - the folder tree counts (`FOLDER_COUNTS`, `FOLDER_SIZES` in `browse/read/tree.rs`), which do not use `scope`.
  A test checks that the facet total, the tree count and the grid length agree with stacks present.
- **Entry.** `Entry` is 56 bytes (measured by compiling against `pg-model`). A copy of its field list with one extra
  `u8` also measures 56, so a one-byte badge field fits.
- **Badge and open view.** A root's cell shows a badge with the member count (`GridCell.qml`). Opening the badge (or a
  key) switches the grid to the members, with "back to the library". Unstacking returns the members to the library.
- **Lifecycle.** A stack with no members is removed. A member removed from the library leaves its stack. A root removed
  from the library leaves its members visible.
- **Ratings and marks** belong to each file. A merged root starts with none. Write-back goes to the root only.
- **Later kinds** (bursts, duplicates) may keep one member visible. That is a per-kind rule, designed when needed.
- **Upscale** is the next kind, planned in [UPSCALE_PLAN.md](UPSCALE_PLAN.md). Its upscaled file is the root, and
  upscaling a merged picture re-roots the merge stack instead of nesting one. That re-root is not yet in §8 above.

## 9. Evidence

Measurements were run on 2026-11-22 with throwaway scripts outside the repo. Those scripts are not reproducible from
the repo. The layout rows below are reproduced by `tools/merge_measure.py` (same files, same settings, same totals), so
they can be checked. The neighbour and seam rows are not reproduced by it. These are evidence, not a proof.

| Test | Result |
| --- | --- |
| Pairwise neighbour checks, 6 photos, JPEG q90/q75 and PNG pieces, grids 2×2, 3×2, 2×4 | The correct neighbour won all 378 checks. Smallest ratio of second-best to best: 1.67. |
| Flat white control | Score 0 for every pair; ratio 1.00. |
| One real pair from the user's catalog (local only) | Correct order: score 0.90. Reversed: 24.86. |
| Layouts, 40 photos (360 grids), plan's formula with the adjacency rule, ratio 1.3 | 248 correct, 0 wrong, 112 refused. Reproduced by the tool. |
| Layouts, 40 UI screenshots (360 grids), same rule | 31 correct, 0 wrong, 329 refused. Reproduced by the tool. |
| Same photos, without the adjacency rule, ratio 1.3 | 257 correct, 0 wrong, 103 refused. |
| Same photos, without the adjacency rule, ratio 1.6 | 302 correct, 3 wrong (one 2×2 photo, all three formats). |
| Flat black, rendered text, line art (9 grids each), with the adjacency rule, ratio 1.3 | All 27 refused. (Flat black and line art are reproducible with the tool; the rendered-text image is not generated by it.) |

Also measured: the catalog holds 715,051 present files; the largest folder holds 55,517; a recursive listing can cover
the whole catalog. `Entry` is 56 bytes.

What the numbers mean: the layout rules never produced a wrong layout on the measured photos and screenshots at 1.3, but
they refuse most UI grids and about a quarter of photo grids. Refusal is the honest outcome, and it is expected.

## 10. Tests

- **`pg-merge` (pure).** Generated images cut into tiles, shuffled, then reassembled for a row, a column, a 3×2 grid and
  a ragged grid. PNG tiles must give the original exactly. JPEG tiles (quality 90 and 75) must give the same layout. The
  2×2 wrong-link case is built from a generated image so the adjacency rule is covered. Flat white and flat black must
  refuse. A missing tile must give "not a rectangle".
- **`pg-codec`.** PNG encode round-trips through the decoder, exactly, including 1×1 and odd sizes.
- **`pg-library` and `pg-app`.** The job writes one file, leaves the originals unchanged, reports progress, and leaves no
  partial file on a stop. The action is refused below two files.
- **Catalog.** The migration; the facet total, tree count and grid length agree with stacks present.
- **Desktop.** QML tests for the dialog states and the badge, under `crates/desktop/tests/qml`.
- **Local only.** The user's own pair, run on their machine. Not committed.

## 11. Build order

1. `pg-codec`: PNG encoder, with its round-trip tests.
2. `pg-merge`: the algorithm in §4, with the tests in §10. Its totals on the same files must match
   `tools/merge_measure.py` (§9), which is the reference.
3. `pg-library` job and `pg-app` methods, including the rescan after the write.
4. Catalog migration and the shared hiding predicate (§8), with the agreement tests.
5. Gallery multi-select (§6).
6. `pg-model` notices and `pg-client` text.
7. Desktop: the action, `MergeDialog.qml`, the badge and the open view.
8. Gates: `just lint`, `just test`, the Linux check in `pg-dev` (it covers every crate except the desktop app), and
   `just build` and `just shots` where the desktop changed. Validation is batched until the change list is complete.

## 12. Risks, and what is still open

Risks:

- **Wrong layouts that pass every check.** The adjacency rule and the rectangle check reduce this; they do not remove
  it. The preview is the final guard.
- **Refusal rate.** Expect many refusals, especially on UI and graphic content.
- **Memory.** A four-piece merge of 24-megapixel photos holds about 288 MB in RAM during the write.
- **Duplicates.** The duplicate pass may pair a merged picture with its pieces. Not measured; test on the user's pair.
- **Gallery list.** The whole folder is loaded per listing. About 56 bytes per entry, so 55,517 entries is about 3 MB.
  Not measured in the window.

Not verified:

- Merge speed on real sizes, and the peak memory of the real build.
- The duplicate pass on the merged picture and its pieces.
- The adjacency rule on content beyond the 80 images measured.

## 13. Choices still for you

These are the only decisions left. Each has a default, used if you don't choose.

1. **Metadata.** Copy the tags and reset orientation (default), or write no metadata.
2. **Alpha.** Flatten onto white (default), or refuse pieces with alpha.
3. **Output folder.** Beside the first piece, in the library, with a rescan (default), or a folder chosen each time
   (which then cannot be stacked until added to the library).
4. **Stacking display.** Hide members everywhere the library lists files (default), and open a stack as a grid view with a
   way back (default). The badge sits top-right of the root's cell (default).
