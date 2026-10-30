# M1–M2: core, import and desktop UI (Rust)

See `docs/PLAN.md` §4–7 and §8 (M1, M2); measurements are in `docs/FINDINGS.md` ("M1 import").

- `core` (`pg-core`): catalog (SQLite), decoding, the import analysis (tiers 0a/0b/1, dimensions, ThumbHash),
  the PDQ index, the thumbnail cache, folder browsing (merged drives), the justified layout and ratings. No UI
  dependency.
- `ingest` (`pg-ingest`): the passes over the originals (scan, import, thumbnails), the app's background job
  that runs them, and the command line.
- `ui` (`pg-ui`, binary `gallery`): the one program. Without arguments it opens the desktop app (Qt 6 Quick
  over CXX-Qt); with a command it runs that pass on the command line.

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

The catalog lives in `D:\PhotoGallery\catalog` (`PG_CATALOG_DIR`, or `--catalog-dir` on the command line, to
change it).

Run `.\target\release\gallery.exe` and add folders with **Library…**. The app scans them, imports them and
makes thumbnails in the background, with progress and a Pause button in the bottom bar. Background work runs
below normal priority (thumbnails at idle), so the window and the thumbnails on screen come first. Quitting at
any point is safe: the next start finishes what's pending.

The same passes on the command line, without the window (for scripts, profiling and checking results):

```powershell
$g = '.\target\release\gallery.exe'
& $g scan 'D:\PhotoGallery\datasets\pinterest' | Out-Host  # add to the library and record image files
& $g scan | Out-Host                                       # rescan the whole library
& $g import | Out-Host                                     # analyze new/changed files; resumable
& $g thumbs --height 256 | Out-Host                        # thumbnails at one grid height
& $g status | Out-Host                                     # library folders and counts
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
- **Library**: the folders added (`roots` table, schema v3). None is inside another: adding a parent replaces
  its children, and adding a subfolder only rescans it. Removing one marks its files missing; adding it back
  restores them with their analysis and ratings.

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
  --viewer-keys=LBZ --library=1 --add-folder=<path> --screenshot=<png> --wait=<s>`. With `QT_QPA_PLATFORM=offscreen` (needs `platforms\qoffscreen.dll`
  next to the exe, from Qt's `plugins\platforms`) and `QT_QPA_FONTDIR=C:\Windows\Fonts` no window opens.
