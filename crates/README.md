# M1–M2: core, import and desktop UI (Rust)

See `docs/PLAN.md` §4–7 and §8 (M1, M2); measurements are in `docs/FINDINGS.md` ("M1 import").

- `core` (`pg-core`): catalog (SQLite), decoding, the import analysis (tiers 0a/0b/1, dimensions, ThumbHash,
  the rating stored in the file), the PDQ index, the thumbnail cache, folder browsing (merged drives, filters,
  sorts, saved views), the justified layout, ratings, reading XMP/EXIF ratings (`metadata`) and lossless
  rotation (`rotate`). No UI dependency.
- `ingest` (`pg-ingest`): the passes over the originals (scan, import, thumbnails), the app's background job
  that runs them, the writer that puts ratings and turns into the files (`writeback`), and the command line.
- `ui` (`pg-ui`, binary `gallery`): the one program. Without arguments it opens the desktop app (Qt 6 Quick
  over CXX-Qt); with a command it runs that pass on the command line.

## Build

Needs CMake and NASM (libjpeg-turbo is built from source by the `turbojpeg` crate) and Qt 6.9 for MSVC
(`C:\Qt\6.9.2\msvc2022_64`, set in `.cargo/config.toml`). Writing ratings into files needs
[ExifTool](https://exiftool.org) on `PATH` (or `PG_EXIFTOOL`); without it they stay queued and the bottom bar
says why. Its test (`crates/ingest/tests/writeback.rs`) works on generated files in
`D:\PhotoGallery\scratch_rate` (`PG_TEST_DIR`), and skips itself without ExifTool.

```powershell
cargo build --release
cargo test --release
# Once (and after Qt updates): put the Qt DLLs and QML modules next to gallery.exe.
C:\Qt\6.9.2\msvc2022_64\bin\windeployqt.exe --release --qmldir crates\ui\qml --no-translations target\release\gallery.exe
```

A debug build (`cargo build`, then the same `windeployqt --release` on `target\debug\gallery.exe`: it links the
release Qt too) reads the QML from `crates\ui\qml` instead of the copy compiled into it, and reloads the window
whenever a file there changes, where it was: QML edits show without a rebuild. QML errors go to the debugger
output unless `QT_FORCE_STDERR_LOGGING=1` is set.

Dependency checks (`cargo binstall cargo-deny cargo-machete` once): `cargo deny check` for advisories,
licenses (the allowed list is in `deny.toml`), duplicate versions, and Qt staying out of `pg-core` and
`pg-ingest`; `cargo machete` for unused dependencies.

## Run

The catalog lives in `D:\PhotoGallery\catalog` (`PG_CATALOG_DIR`, or `--catalog-dir` on the command line, to
change it). The app's preferences (write-back, Real Size, row height) are in `gallery.json` beside the catalog
folder (`D:\PhotoGallery\gallery.json`; `PG_PREFS` to change it), so they survive rebuilding the catalog.
Saved views and where you were (folder, filter, sort, selected file) are in the catalog.

The catalog schema is one baseline (`catalog.rs`); until the first release, schema changes edit it and a catalog
made before it is refused with "delete it and re-import". The migration mechanism is in place for later.

Run `.\target\release\gallery.exe` and add folders with **Library…**. The app scans them, imports them and
makes thumbnails in the background, with progress and a Pause button in the bottom bar. Background work runs
below normal priority (thumbnails at idle), so the window and the thumbnails on screen come first. Quitting at
any point is safe: the next start finishes what's pending. An internal error (a Rust panic) shows a message and
appends its details and backtrace to `gallery.log` beside the catalog folder (`D:\PhotoGallery\gallery.log`).

The same passes on the command line, without the window (for scripts, profiling and checking results):

```powershell
$g = '.\target\release\gallery.exe'
& $g scan 'D:\PhotoGallery\datasets\pinterest' | Out-Host  # add to the library and record image files
& $g scan | Out-Host                                       # rescan the whole library
& $g import | Out-Host                                     # analyze new/changed files; resumable
& $g thumbs --height 256 | Out-Host                        # thumbnails at one grid height
& $g write | Out-Host                                      # write queued ratings and turns into the files
& $g status | Out-Host                                     # library folders, counts, pending and failing writes
& $g dups --show 5 | Out-Host                              # what tiers 0-1 found, and the groups they form
& $g remove 'D:\PhotoGallery\datasets\pinterest'           # take a folder out (its files are hidden, not deleted)
```

`gallery.exe` is a GUI program, so no console window opens when it starts from Explorer. Its commands print to
the console they were typed in. PowerShell doesn't wait for a GUI program unless its output is piped, hence
`| Out-Host`.

- **Scan** lists folders by directory handle, which yields each file's NTFS file ID: a moved or renamed file
  keeps its row and its analysis.
- **Import** reads files in MFT-record order with 8 reads in flight, decodes once (libjpeg-turbo, DCT-scaled
  to ≥ 512 × 320), and stores the BLAKE3 file hash, the pixel hash, PDQ, dimensions and a ThumbHash. Each new
  PDQ hash is queried (all 8 orientations) and then inserted, so tier-1 pairs (d ≤ 31) come out of the import
  itself. Every 500 files is one transaction; a rerun continues where the last one stopped, and redoes files
  whose size or mtime changed.
- **Thumbs** makes no thumbnails during the import: they are made afterwards, for any height. The cache
  (`thumbs.sqlite`) is keyed by (file, height) and capped (`--cap-gb`, default 50; `PG_THUMB_CAP_GB` for the
  app). The background job (or `gallery thumbs`) fills it at idle CPU priority for the grid's current height,
  evicting other heights first, and starts over when the size changes. The ones on screen are made on demand,
  newest request first.
- **Library**: the folders added (`roots` table). None is inside another: adding a parent replaces
  its children, and adding a subfolder only rescans it. Removing one marks its files missing; adding it back
  restores them with their analysis and ratings.
- **Write** is what the app's writer does in the background (see Ratings below); the command drains the queue
  and reports what still fails.

## The app

ImageRater's triage, without Everything: open "Unrated", largest first, rate each picture and move on.

- **Folder tree** (left): folders with the same path on different drives are one node (their drives are shown
  in green). "All folders" lists everything; "Include subfolders" toggles recursion.
- **Top bar**: which ratings to show (all, unrated, rated, exactly 1–5) and the order (by name, largest first,
  newest first), filtered and sorted in SQL. **Views ▾** saves the current folder, subfolders, filter and sort
  under a name (ImageRater's bookmarks), and opens or deletes saved ones. The list re-filters when it is
  reopened, not under you: a picture rated out of "Unrated" stays until then.
- **Grid**: justified rows. The **size slider**, Ctrl+wheel or Ctrl +/− change the row height at runtime; the
  layout follows at once, thumbnails at the new size follow a moment later (the old ones are scaled meanwhile,
  and the ThumbHash placeholder shows until the first one arrives). Arrows move, 0–5 rates, **R** / **Shift+R**
  turn left / right, Enter or double-click opens the viewer. A badge shows each rating's symbol.
- **Viewer**:
  - ← → (or Backspace / Space) step; 4 pictures each way are preloaded.
  - **0–5**, **↑** (4) and **↓** (1) rate and move to the next picture, showing the rating's symbol for half a
    second. The rating bar at the bottom does the same by click, with the current rating ringed; its ⟲ ⟳ (and
    **R** / **Shift+R**) turn the picture.
  - **Zoom**: the wheel zooms around the cursor from fit to 32×, + / − around the centre, pinch on a touch
    screen or touchpad; drag pans. Double-click toggles fit and 1:1. Fit is a Lanczos3 downscale from the
    original (never enlarged); between fit and 1:1 the GPU scales it at once and the visible part is rendered
    again with Lanczos3 when zooming stops; from 1:1 up it is the original with nearest neighbour, to judge
    pixels.
  - **Z** Real Size: pictures open at 1:1 instead of fitted, remembered across pictures and runs.
  - **L** a 10× nearest-neighbour loupe under the cursor, **B** a magenta frame around the image to reveal black
    bars, **H** or a click hides everything over the picture (zen). Touch: swipe left / right to step, up / down
    to rate 4 / 1. Esc, Enter or ✕ close.
- **Ratings**: the catalog (`ratings` table) is the source of truth for every format. With "Write ratings and
  turns into the files too" (Library…, on by default) they also go into the file as `XMP:Rating` wherever the
  format holds XMP (JPEG, PNG, WebP, TIFF, GIF), by a background writer (ExifTool), 2 s after the last change so
  quick changes coalesce. BMP and other formats keep it in the catalog only (the status line says so). Writes
  are queued in the catalog (`file_writes`), so they survive quitting, wait for offline drives, and retry; the
  bottom bar shows what is pending or failing. A file is rewritten in place, keeping its NTFS identity, and a
  rating write doesn't make it look changed to the import. Ratings already in files (ImageRater, Explorer,
  Lightroom: `xmp:Rating`, `MicrosoftPhoto:Rating`, EXIF Rating/RatingPercent) are read at import; a rating
  changed in a file by another program since we last saw it wins over the catalog's.
- **Turns** are lossless and real wherever possible, so every program shows them: JPEG pixels are turned with
  libjpeg-turbo's perfect transform (otherwise the EXIF Orientation tag), PNG, BMP, still GIF and lossless WebP
  are re-encoded losslessly, TIFF gets its Orientation tag. What none of that covers (lossy or animated WebP,
  animated GIF, AVIF…) is turned in the catalog and at display. The grid and viewer show a turn at once; the
  file follows in the background, and its analysis and thumbnails are redone.
- Test hooks (for checking the UI without a screen; see `Main.qml`): `--size=<px> --select=<n> --view=<n>
  --viewer-keys=LBH+1<>r<n> --zoom=<x> --menu=views --apply-view=<name> --library=1 --add-folder=<path>
  --screenshot=<png> --wait=<s>`. With `QT_QPA_PLATFORM=offscreen` (needs `platforms\qoffscreen.dll` next to
  the exe, from Qt's `plugins\platforms`) and `QT_QPA_FONTDIR=C:\Windows\Fonts` no window opens; set
  `QT_FORCE_STDERR_LOGGING=1` to see QML messages. Menus are in the window's overlay, which the screenshot
  doesn't include. Rating and turning hooks write to the catalog and the files: use them on a scratch library
  (`PG_CATALOG_DIR`, `PG_PREFS`).
