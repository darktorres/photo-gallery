# M1–M2: core, import and desktop UI (Rust)

See `docs/PLAN.md` §4–7 and §8 (M1, M2); measurements are in `docs/FINDINGS.md` ("M1 import").

- `core` (`pg-core`): catalog (SQLite), decoding, the import analysis (tiers 0a/0b/1, dimensions, ThumbHash),
  the PDQ index, the thumbnail cache, folder browsing (merged drives), the justified layout and ratings. No UI
  dependency.
- `ingest` (`pg-ingest`, binary `pg`): the command-line front end (scan, import, thumbnails, reports).
- `ui` (`pg-ui`, binary `gallery`): the desktop app, Qt 6 Quick over CXX-Qt.

## Build

Needs CMake and NASM (libjpeg-turbo is built from source by the `turbojpeg` crate) and Qt 6.9 for MSVC
(`C:\Qt\6.9.2\msvc2022_64`, set in `.cargo/config.toml`).

```powershell
cargo build --release
cargo test --release
# Once (and after Qt updates): put the Qt DLLs and QML modules next to gallery.exe.
C:\Qt\6.9.2\msvc2022_64\bin\windeployqt.exe --release --qmldir crates\ui\qml --no-translations target\release\gallery.exe
```

## Run

The catalog lives in `D:\PhotoGallery\catalog` (`--catalog-dir` or `PG_CATALOG_DIR` to change it, for both
programs).

```powershell
$pg = '.\target\release\pg.exe'
& $pg scan 'D:\PhotoGallery\datasets\pinterest'   # record image files (identity, size, mtime); unseen ones -> missing
& $pg import                                      # analyze new/changed files; resumable, stop it any time
& $pg thumbs --height 256                         # optional: pre-make thumbnails at one grid height
& $pg status                                      # counts
& $pg dups --show 5                               # what tiers 0-1 found, and the groups they form
.\target\release\gallery.exe                      # the desktop app
```

- **Scan** lists folders by directory handle, which yields each file's NTFS file ID: a moved or renamed file
  keeps its row and its analysis.
- **Import** reads files in MFT-record order with 8 reads in flight, decodes once (libjpeg-turbo, DCT-scaled
  to ≥ 512 × 320), and stores the BLAKE3 file hash, the pixel hash, PDQ, dimensions and a ThumbHash. Each new
  PDQ hash is queried (all 8 orientations) and then inserted, so tier-1 pairs (d ≤ 31) come out of the import
  itself. Every 500 files is one transaction; a rerun continues where the last one stopped, and redoes files
  whose size or mtime changed.
- **Thumbs** makes no thumbnails during the import: they are made afterwards, for any height. The cache
  (`thumbs.sqlite`) is keyed by (file, height) and capped (`--cap-gb`, default 50; `PG_THUMB_CAP_GB` for the
  app). `pg thumbs` fills it at idle CPU priority, evicting other heights first; the app makes the ones on
  screen on demand, newest request first.

## The app

- **Folder tree** (left): folders with the same path on different drives are one node (their drives are shown
  in green). "All folders" lists everything; "Include subfolders" toggles recursion.
- **Grid**: justified rows. The **size slider**, Ctrl+wheel or Ctrl +/− change the row height at runtime; the
  layout follows at once, thumbnails at the new size follow a moment later (the old ones are scaled meanwhile,
  and the ThumbHash placeholder shows until the first one arrives). Arrows move, 0–5 rates, Enter or
  double-click opens the viewer.
- **Viewer**: fit to window (Lanczos3 from the original, never enlarged), ← → step (neighbours are preloaded),
  **Z** 1:1 with panning, **L** a 10× nearest-neighbour loupe under the cursor, **B** a magenta frame around
  the image to reveal black bars, 0–5 rates, Esc closes.
- Ratings are stored in the catalog (`ratings` table), not in the files.
- Test hooks (for checking the UI without a screen; see `Main.qml`): `--size=<px> --select=<n> --view=<n>
  --viewer-keys=LBZ --screenshot=<png>`. With `QT_QPA_PLATFORM=offscreen` (needs `platforms\qoffscreen.dll`
  next to the exe, from Qt's `plugins\platforms`) and `QT_QPA_FONTDIR=C:\Windows\Fonts` no window opens.
