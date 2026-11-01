# M1–M2: core, import and desktop UI (Rust)

See `docs/PLAN.md` §4–7 and §8 (M1, M2); measurements are in `docs/FINDINGS.md` ("M1 import").

- `core` (`pg-core`): catalog (SQLite), decoding, the import analysis (tiers 0a/0b/1, dimensions, ThumbHash,
  the rating stored in the file), the PDQ index, the thumbnail cache, folder browsing (merged drives, filters,
  sorts, saved views), the justified layout, ratings, reading XMP/EXIF ratings (`metadata`), lossless
  rotation (`rotate`), the border detector (`borders`) and cropping (`crop`; both rewrite a file's pixels in its
  own format through `rewrite`). No UI dependency.
- `ingest` (`pg-ingest`): the passes over the originals (scan, import, borders, thumbnails, cropped copies), the
  app's background job that runs them, the writer that puts ratings and turns into the files (`writeback`), and
  the command line.
- `ui` (`pg-ui`, binary `gallery`): the one program. Without arguments it opens the desktop app (Qt 6 Quick
  over CXX-Qt); with a command it runs that pass on the command line.

## Build

Needs CMake and NASM (libjpeg-turbo is built from source by the `turbojpeg` crate) and Qt 6.12 for MSVC
(`C:\Qt\6.12.0\msvc2022_64`, set in `.cargo/config.toml`), with the **Qt Image Formats** add-on: the Yandex panel
shows remote previews through Qt's decoders, and without its `qwebp` plugin WebP results stay blank. Writing ratings into files needs
[ExifTool](https://exiftool.org) on `PATH` (or `PG_EXIFTOOL`); without it they stay queued and the bottom bar
says why. Its test (`crates/ingest/tests/writeback.rs`) works on generated files in
`D:\PhotoGallery\scratch_rate` (`PG_TEST_DIR`), and skips itself without ExifTool.

Videos go through FFmpeg 9, our own copy rather than the one Qt bundles: BtbN's prebuilt `lgpl-shared` package
(`D:\PhotoGallery\tools\ffmpeg-9.0`, `FFMPEG_DIR` in `.cargo/config.toml`; the exact build is in its
`SOURCE.txt`). It has dav1d for AV1, which Qt's copy lacks. Its bindings are generated with libclang (LLVM,
`LIBCLANG_PATH`). `pg-core`'s build script copies its DLLs next to the binaries. Qt Multimedia plays videos with
the same DLLs: Qt 6.12 is built against FFmpeg 9, the same major version. So `just deploy` runs `windeployqt
--no-ffmpeg`. The video test (`crates/ingest/tests/video.rs`) makes its clips and animations with an `ffmpeg`
command line that has the GPL encoders (libx264, libvpx, libaom, libwebp), such as gyan.dev's "essentials" build
(`choco install ffmpeg`). It looks on `PATH`, or at `PG_FFMPEG`; our LGPL package lacks libx264. The clips go in
`D:\PhotoGallery\scratch_video`, and the test skips itself without that ffmpeg.

The commands are in the `Justfile` ([just](https://just.systems); `just` lists them):

```powershell
just deploy  # release build, then the Qt DLLs and QML modules next to gallery.exe (once, and after Qt updates)
just build   # rebuild
just run     # the app; `just run status`, `just run scan <folder>`… for the command line
just test    # a debug build, with the dependencies optimized (Cargo.toml), so decoding runs at full speed
just lint    # rustfmt, clippy, cargo-deny, cargo-machete, ruff, markdownlint, then the QML's (just lint-qml)
```

`just deploy debug`, then `just dev`, give the debug build (it links the release Qt too). It reads the QML from
`crates\ui\qml` instead of the copy compiled into it and reloads the window whenever a file there changes, where
it was: QML edits show without a rebuild. QML errors go to the debugger output unless
`QT_FORCE_STDERR_LOGGING=1` is set.

`just lint` needs `cargo binstall cargo-deny cargo-machete` once. `cargo deny check` covers advisories, licenses
(the allowed list is in `deny.toml`), duplicate versions, and Qt staying out of `pg-core` and `pg-ingest`;
`cargo machete` finds unused dependencies. `just lint-qml` fails on any qmllint warning, and on any binding or
function the QML compiler (qmlcachegen, as the build runs it) leaves to the interpreter instead of compiling to C++.
It reads the Rust types from the last build, so build first (`tools/lint-qml.ps1` says why it lints a copy of the
module in `target\qmllint`).

The QML compiles to C++ only where it is typed throughout. So the Rust objects' properties are `FINAL`. A Rust
object's methods are called only in the file that makes it, by its id: through a property, a method can't be
compiled, because a subclass could replace it. FilterBar owns the filters and Main the gallery model, and the
viewer gets the shown item's values as properties. Text and numbers come typed from Rust (`CommandLine`,
`Library.writeFailed`), not from JavaScript's string functions, which aren't compiled. qt-build-utils is vendored so
that the compiler sees the module's types (`vendor/README.md`).

## Run

The catalog lives in `D:\PhotoGallery\catalog` (`PG_CATALOG_DIR`, or `--catalog-dir` on the command line, to
change it). The app's preferences (write-back, Real Size, row height) are in `gallery.json` beside the catalog
folder (`D:\PhotoGallery\gallery.json`; `PG_PREFS` to change it), so they survive rebuilding the catalog.
Saved views and where you were (folder, filters, sort, selected file) are in the catalog.

The catalog schema is one baseline (`catalog.rs`). Until the first catalog worth keeping, schema changes edit it in
place, with no new version, so a catalog made before a change has to be deleted and re-imported. The migration
mechanism is in place for later, unused.

Run `.\target\release\gallery.exe` and add folders with **Library…**. The app scans them, imports them and
makes thumbnails in the background, with progress and a Pause button in the bottom bar. Background work runs
below normal priority (thumbnails at idle), so the window and the thumbnails on screen come first. Quitting at
any point is safe: the next start finishes what's pending. An internal error (a Rust panic) shows a message and
appends its details and backtrace to `gallery.log` beside the catalog folder (`D:\PhotoGallery\gallery.log`).

The same passes on the command line, without the window (for scripts, profiling and checking results):

```powershell
$g = '.\target\release\gallery.exe'
& $g scan 'D:\PhotoGallery\datasets\pinterest' | Out-Host  # add to the library and record its media files
& $g scan | Out-Host                                       # rescan the whole library
& $g import | Out-Host                                     # analyze new/changed files; resumable
& $g borders | Out-Host                                    # find the borders of what was imported; resumable
& $g thumbs --height 256 | Out-Host                        # thumbnails at one grid height
& $g write | Out-Host                                      # write queued ratings and turns into the files
& $g status | Out-Host                                     # library folders, counts, pending and failing writes
& $g dups --show 5 | Out-Host                              # what tiers 0-1 found, and the groups they form
& $g borders --csv 'D:\x\report.csv' 'D:\x\images' | Out-Host # borders of any files, in the prototype's columns
& $g borders --import-review 'D:\x\review.json'            # the eval viewer's "not a border" marks, into the catalog
& $g remove 'D:\PhotoGallery\datasets\pinterest'           # take a folder out (its files are hidden, not deleted)
```

`gallery.exe` is a GUI program, so no console window opens when it starts from Explorer. Its commands print to
the console they were typed in. PowerShell doesn't wait for a GUI program unless its output is piped, hence
`| Out-Host`.

- **Scan** lists folders by directory handle, which yields each file's NTFS file ID: a moved or renamed file
  keeps its row and its analysis. It records the files whose extension is a supported format (`media.rs`), with
  their extension and kind.
- **Import** reads files in MFT-record order with 8 reads in flight, decodes once (libjpeg-turbo, DCT-scaled
  to ≥ 512 × 320), and stores the BLAKE3 file hash, the pixel hash, PDQ, dimensions and a ThumbHash, plus the
  frame count (from the GIF, APNG or WebP container) and the date taken (EXIF DateTimeOriginal, else XMP). Each new
  PDQ hash is queried (all 8 orientations) and then inserted, so tier-1 pairs (d ≤ 31) come out of the import
  itself. A video (`video.rs`) is hashed by streaming, never read whole. Its upright size, duration, codec and
  creation date come from the container: the rotation matrix, and Apple's local creation date before the UTC
  `creation_time`. Its ThumbHash comes from the poster frame, at 10% of the duration (at most 10 s). Videos get
  no pixel hash or PDQ, so tiers 0b and 1 don't cover them. Every 500 files is one transaction; a rerun continues
  where the last one stopped, and redoes files whose size or mtime changed.
- **Thumbs** makes no thumbnails during the import: they are made afterwards, for any height. The cache
  (`thumbs.sqlite`) is keyed by (file, height) and capped. The cap is kept
  in the cache, so the app and `gallery thumbs` keep to the same one: 50 GB for a new cache, changed by
  `gallery thumbs --cap-gb` or the app's `PG_THUMB_CAP_GB`, and shown by `gallery status`. The background job (or `gallery thumbs`) fills it at idle CPU
  priority for the grid's current height, evicting only other heights, and starts over when the size changes.
  The ones on screen are made on demand, newest request first, and evict the least recently used of any height.
  A video's thumbnail is its poster frame. HDR videos (PQ, HLG) are converted to RGB without tone mapping, so
  their posters look washed out.
  - Layout (`thumbs.rs`): the entries (a few dozen bytes each) and the JPEGs are separate tables, and the totals
    are one row that triggers keep, so opening the cache and listing what's missing never read the JPEGs
    (FINDINGS "Startup"). Reads take no lock; writes go through one writer thread, in IMMEDIATE transactions, so
    the app and `gallery thumbs` can share the file. "Least recently used" is to the hour: a hit rewrites its
    entry only when it is older, and those writes are batched. A cache far over its cap (the cap lowered) comes
    down in slices of 256 MB, one transaction each, so the other process never waits long for the file.
  - Its layout version stays 1 before the first release, like the catalog's: a layout change edits it in place,
    and an existing cache file is deleted by hand. A file of another version is refused, and one that can't be
    opened only means thumbnails aren't kept.
- **Borders** (`borders.rs`, a port of the Python prototype `tools/borders.py`, now in git history; FINDINGS
  "Border cleaning"): solid borders (black, white or any colour, JPEG noise included) and blurred padding bars, on
  the stored pixels at full size.
  A pass of its own after the import (`gallery borders`, or the background job before the thumbnails) reads and
  decodes each still image the import analyzed at full size: the import's decode is reduced, and the detector
  is calibrated on every pixel. The import marks the file due (`analysis.borders_due`); a re-analysis drops its
  old borders. Every side found is kept (`border_sides`), the rejected candidates too with why; the cut is in
  `analysis.cut_*`. The detector has its own version (`BORDERS_VERSION`): a new one makes every file due again,
  without importing the files again. `gallery borders --csv` reports on any files, outside the catalog.
- **Library**: the folders added (`roots` table). None is inside another: adding a parent replaces
  its children, and adding a subfolder only rescans it. Removing one marks its files missing; adding it back
  restores them with their analysis and ratings.
- **Write** is what the app's writer does in the background (see Ratings below); the command drains the queue
  and reports what still fails.

## The app

ImageRater's triage, without Everything: open "Unrated", largest first, rate each picture and move on.

The window comes up before anything is read from the catalog: the folder tree, the grid's files and the filter
counts are read by a thread of their own (`loader.rs`), oldest request first, each kind keeping only its newest
(which cancels one of its kind running), and shown when they arrive ("Loading…" meanwhile). The selection stays
with its file when a list is refreshed; while the viewer is open a new list waits for it to close. A rating or a turn
made while a list was on its way is kept. Startup times are in FINDINGS "Startup".

- **Folder tree** (left): folders with the same path on different drives are one node (their drives are shown
  in green). "All folders" lists everything; "Include subfolders" toggles recursion.
- **Top bar**: "N of M files", **Filters** (or **F**) to open the filter bar, the order (name, file size, date
  modified, date taken, megapixels, duration, border depth) and its direction (↑ / ↓). **Views ▾** saves the current folder,
  subfolders, filters and sort under a name (ImageRater's bookmarks), and opens or deletes saved ones.
- **Filter bar** (`FilterBar.qml`, `query.rs`): type (image, animated, video), extension, rating (unrated,
  1–5), shape (landscape, portrait, square), file size, megapixels, duration, date taken, date modified,
  duplicates (has / has none, from tiers 0a, 0b and tier-1 pairs within PDQ 22), import status (imported, not
  yet, unreadable), borders (none, black, white, coloured, blurred bars), their review (not marked, marked not a
  border) and part of the name. Facets combine with AND, the choices inside one with OR. Each type, extension
  and borders choice shows how many files choosing it would show, given the other facets. Filtering and sorting are
  SQL (values bound, not spliced); a change applies 0.3 s after the last edit. The list re-filters when it is
  reopened or the filters change, not under you: a picture rated out of "Unrated" stays until then.
- **Grid**: justified rows. The **size slider**, Ctrl+wheel or Ctrl +/− change the row height at runtime; the
  layout follows at once, thumbnails at the new size follow a moment later (the old ones are scaled meanwhile,
  and the ThumbHash placeholder shows until the first one arrives). Arrows move, 0–5 rates, **R** / **Shift+R**
  turn left / right, Enter or double-click opens the viewer. A badge shows each rating's symbol, another a
  video's length (▶ 1:23) or the format of an animated image (GIF, APNG, WEBP), and ▭ borders found (struck out
  when marked not a border). **B** shows on every thumbnail what cropping the borders would cut (remembered,
  shared with the viewer), **N** marks the selected file's borders as not a border, or takes the mark back.
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
  - **L** a 10× nearest-neighbour loupe under the cursor; **B** a magenta frame around the image to reveal black
    bars, and what cropping its borders would cut, dimmed (grey when marked not a border), with rejected
    candidates as yellow lines; hovering over the cut shows the picture under it. **N** (or ▭ in the rating bar)
    marks the borders as not a border. **H** or a click hides everything over the picture (zen). Touch: swipe left / right to step, up / down
    to rate 4 / 1. Esc, Enter or ✕ close.
  - **Videos** play with Qt Multimedia, fitted and never enlarged. They start with sound and loop, and the poster
    shows until the first frame. The transport bar has play / pause (**K**), the time, a seek bar, mute (**M**)
    and volume. Rating, turning and stepping work as on pictures; zoom, the loupe and Real Size are for stills.
    The player decodes on the GPU, except on a GPU without AV1 decoding (this machine's RTX 2060). There every
    video decodes on the CPU (AV1 through dav1d) (`gpu_device.rs` explains why).
  - **Animated** GIF, APNG and WebP play the same way: our FFmpeg decodes all three. Qt's `AnimatedImage` has no
    APNG, and has WebP only with Qt's Image Formats add-on. They loop silently, without the transport bar; **K**
    pauses.
- **Ratings**: the catalog (`ratings` table) is the source of truth for every format. With "Write ratings and
  turns into the files too" (Library…, on by default) they also go into the file as `XMP:Rating` wherever the
  format holds XMP (JPEG, PNG, WebP, TIFF, GIF), by a background writer (ExifTool), 2 s after the last change so
  quick changes coalesce. BMP, videos and other formats keep it in the catalog only (the status line says so).
  ExifTool could write XMP into an MP4 or MOV, but it rewrites the whole file, gigabytes for a video. Writes
  are queued in the catalog (`file_writes`), so they survive quitting, wait for offline drives, and retry; the
  bottom bar shows what is pending or failing. A file is rewritten in place, keeping its NTFS identity, and a
  rating write doesn't make it look changed to the import. Ratings already in files (ImageRater, Explorer,
  Lightroom: `xmp:Rating`, `MicrosoftPhoto:Rating`, EXIF Rating/RatingPercent) are read at import; a rating
  changed in a file by another program since we last saw it wins over the catalog's.
- **Turns** are lossless and real wherever possible, so every program shows them: JPEG pixels are turned with
  libjpeg-turbo's perfect transform (otherwise the EXIF Orientation tag), PNG, BMP, still GIF and lossless WebP
  are re-encoded losslessly, TIFF gets its Orientation tag. What none of that covers (lossy or animated WebP,
  animated GIF, videos…) is turned in the catalog and at display. A video's own rotation matrix is applied by
  the player and by the import; the catalog's turns go on top. The grid and viewer show a turn at once; the
  file follows in the background, and its analysis and thumbnails are redone.
- **Save cropped copies (N)…** (the filter bar's borders row) writes the listed files with their borders cut
  off into a folder of your choice, as `<folder>\<library folder name>\<path inside it>`, with the original's
  modification time, in the background (`ingest::crop`). The originals are never written: a folder whose copies
  would land in the library is refused, and a file changed since its borders were found is skipped. JPEGs are
  cropped losslessly; their left and top cuts move inward to the 8 or 16 px block grid. PNG, BMP, GIF and WebP
  are cropped exactly and re-encoded losslessly with every other chunk kept (a lossy WebP comes out lossless,
  larger), TIFF re-encoded with its metadata copied by ExifTool.
- Test hooks (for checking the UI without a screen; see `Main.qml`): `--size=<px> --select=<n> --view=<n>
  --viewer-keys=LBNHK+1<>r<n> --zoom=<x> --menu=views --apply-view=<name> --query=<json> --sort=<json> --filters=1
  --borders=1 --library=1 --save-view=1 --find-similar=<path> --add-folder=<path> --save-crops=<folder>
  --screenshot=<png> --wait=<s>`. `--query`, `--sort` and `--apply-view` shape the first list; the others act on it
  once it is shown, and `--wait` (4 s by default) counts from then. `--borders` shows the borders (B) for the run. `--find-similar` uploads the file to Yandex: a path that isn't a still image (a `.txt`)
  opens the panel on its error without sending anything. With
  `QT_QPA_PLATFORM=offscreen` (needs `platforms\qoffscreen.dll` next to the exe, from Qt's `plugins\platforms`)
  and `QT_QPA_FONTDIR=C:\Windows\Fonts` no window opens; set `QT_FORCE_STDERR_LOGGING=1` to see QML messages.
  Offscreen draws with Qt's software renderer, which shows no video frames: check playback through the player's
  state and position instead. The screenshot is the window as shown, menus included. Rating and turning hooks
  write to the catalog and the files: use them on a scratch library (`PG_CATALOG_DIR`, `PG_PREFS`).
