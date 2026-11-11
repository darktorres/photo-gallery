# The start, to `ready`, after the optimizations

Where every instruction of the app's start goes, thread by thread, after the order key, the grid model, tracked
work, the one D3D11 device, the deferred QML and the PDQ column pass (PLAN §7, first stage). Instructions, not time:
the counts are exact, and the few that move between runs are marked.

## How it was measured

- **Binary:** release `gallery.exe` built before the history was squashed, Qt 6.12.0 (msvc2022_64) with its PDBs beside the DLLs.
- **Tool:** the Pin call graph (`tools/profiling/callgraph`, docs/PROFILING.md), from process start until the first
  thread enters `pg_stage_ready`; then `symbolize.py` for Windows' and the driver's functions. One run: 297 s under
  Pin.
- **Data:** the scratch catalog rebuilt for tracked work, `D:\PhotoGallery\scratch_tracked\catalog`: the pinterest
  dataset, 189,518 files in 690 folders, all imported, every 220 px thumbnail made. The session folder is "All
  folders", so the grid lists every file. Prefs: row height 220, write-back off.
- **Files:** `D:\PhotoGallery\profiles\loaders\B-1.38332-*` (one per thread, callgrind format) and `.threads`.
  Read them with `python tools/profiling/cgtab.py table|calls FILE...`.
- **Compared with:** the profile in PROFILING "The start, to `ready`" (2026-10-31,
  `D:\PhotoGallery\scratch_startup\profiles\ready.30980-*`), the same dataset in the catalog of the time, without
  thumbnails.

Three of the old profile's thread files (`-29`, `-30`, `-37`, 4.6 M instructions together) can't be read: they
refer to a file name id (`fi=(14)`) they never define, a bug of the tool's first version. The current tool's files
read fine. They are left out of the old sums below, which therefore come to 15.145 G instead of 15.150 G.

## Totals

**15.15 G instructions in 85 threads → 3.70 G in 60 threads (−75.5%).**

| Thread | Before | Now | What changed |
|---|---:|---:|---|
| catalog reader | 10.90 G | 1.43 G | the grid's list 10.25 G → 1.32 G; the folder tree 656 M → 115 M |
| main | 1.24 G | 0.78 G | QML created when first needed; a cheaper Direct3D 9 probe; one regression (below) |
| QSGRenderThread | 1.18 G | 0.10 G | renders with the app's device instead of making its own |
| gpu device (was "video decoding") | 0.93 G | 0.92 G | now the app's only D3D11 device |
| QQmlThread | 0.30 G | 0.30 G | QML type loading, unchanged |
| unnamed (65 → 42 threads) | 0.37 G | 0.13 G | mostly NVIDIA's driver worker; varies with the frame count |
| Thread (pooled) | 0.21 G | 0.03 G | mostly timing: 42 thumbnails served before `ready` then, 6 now |
| others | 0.01 G | 0.00 G | vsync, pixmap reader; the WASAPI probe threads are gone |

Across all threads, `D3D11CreateDevice` is called once instead of twice, and NVIDIA's signature scans
(`imagehlp!RabinKarpFindPatternInBuffer`) dropped from 11 (2.01 G) to 5 (0.89 G).

## Thread by thread

### catalog reader, 1,435 M

| Item | Instructions |
|---|---:|
| `browse::list_grid` (`crates/catalog/src/browse/read/grid.rs`) | 1,317 M |
| `browse::folder_tree` (`.../read/tree.rs`) | 115 M |
| opening the connection | 2 M |

- **The grid's list** has no sort and no collation left. SQLite reads the 189,518 rows from `files_order`: 667 M in
  the VM's own code (`sqlite3VdbeExec`, mostly decoding records), three B-tree lookups per row for the joins
  (`sqlite3BtreeTableMoveto`, 568,555 calls, 179 M), and the index's rowids (103 M). rusqlite's column reads
  (`entry_from_row`) are 348 M: `value_ref` and `sqlite3_column_type` for each of 10 columns, and the extension read
  as text and matched (`media::Ext::of`, `crates/model/src/media.rs`, `from_utf8`). About 6,900 instructions per file.
- **The tree:** 102 M of the 115 M is the per-folder count over `files_present` (691 groups from 189,518 index
  entries); the rest is building the tree.

### gpu device, 924 M

| Item | Instructions |
|---|---:|
| `D3D11CreateDevice` | 922 M |
| — NVIDIA's loader checking its signature (`OpenAdapter10_2` → `CryptQueryObject`) | 733 M |
| — the rest of the device | 188 M |
| DXGI factory and adapter | 1 M |
| the AV1 decoder query (`GetVideoDecoderProfileCount`) | 0.13 M |

NVIDIA's loader (`nvldumdx.dll`) calls `CryptQueryObject` three times while it opens the adapter, and once more from
`nvwgf2umx.dll`. Each call reads a DLL's embedded signature (`GetEmbeddedPKCS7`), which scans the whole file for
invalid markers: four scans of 183,361,250 instructions each, so the same file four times. The driver does this for
every device a process makes; making one device is the most the app can do.

### main, 778 M

| Item | Instructions |
|---|---:|
| `QQmlApplicationEngine::load` | 385 M |
| — Qt's first font fallback query (`QFontEngineMulti::ensureFallbackFamiliesQueried`) | 215 M |
| — QML objects, bindings and text layout | ~170 M |
| `pg::show_windows` | 181 M |
| — window creation (`WindowCreationData::create`) | 166 M |
| — of which the Direct3D 9 probe (`Direct3DCreate9`), one signature scan 152 M | 165 M |
| `state::init` (now `App::open`, `crates/app/src/app.rs`) | 94 M |
| — `thumbs::heal` (now `heal` in `crates/thumbcache/src/lib.rs`) | 91 M |
| — `Catalog::open` | 2.5 M |
| event loop up to `ready` (`QCoreApplication::exec`) | 75 M |
| — the views' delegates (24 `createItem`, and 42 items their Repeaters make) | 63 M |
| — `layout::justify` (now `justify_into`, `crates/client/src/layout.rs`), two passes over 189,518 aspects | 6 M |
| `QGuiApplication` and `QQmlApplicationEngine` | 21 M |
| process start before `main` (the DLL loader) | 18 M |

- **The font fallback query.** The first text with a character the main font lacks makes Qt ask for the fallback
  families: `QWindowsDirectWriteFontDatabase::fallbacksForFamily` → `populateFamily` for 184 families, creating a
  DirectWrite face for each of their 491 fonts (`DWriteFont::CreateFontFace`, 192 M) to read its writing systems.
  DirectWrite also writes its cache data for them (`FontFaceRegionBuilder::WriteRegion`). By DLL, DWrite is the main
  thread's largest (227 M self), and it makes 79k of the thread's 170k heap allocations. The old profile had it too
  ("Text and fonts on the main thread, about 0.3 G"); it is now the largest item on this thread.
- **The Direct3D 9 probe** (QTBUG-50371's workaround in Qt's window creation, deferred) costs 165 M instead of
  585 M: one signature scan instead of three. Most likely because the app's device is alive when the window is
  made, so NVIDIA's loader is still loaded; before, the AV1 probe's device had been released by then.
- **`heal()` here is a regression of tracked work.** `ThumbCache::open` runs it on the caller's thread, the GUI
  thread, before the window exists. The plan put it at the start of the cache's writer thread. Before tracked work it
  ran on the library job's thread after `ready`, once per round; now it runs once per process.
- **The Loaders:** the load was 500.5 M with every view created (HEAD before them, same data), 384.9 M now.
- **Delegates:** the views were reset four times (`QQuickItemViewPrivate::clear`) while the list and the tree
  arrived, but only 24 delegates were made, so few were thrown away. About 2.6 M per row delegate, most of it text.

### QQmlThread, 303 M

Qt's type loader, on its own thread; the load waits for it.

| Item | Instructions |
|---|---:|
| type registration of the imported modules (2,622 types from 18 modules) | 106 M |
| — Qt Quick Templates | 47 M |
| — Qt Quick | 30 M |
| — Qt Quick Controls impl | 13 M |
| — Qt Multimedia | 5 M |
| — Layouts, Fusion impl, QtQml.Models and the rest | 11 M |
| qrc lookups for the file selector (`QQmlFileSelectorInterceptor` → `QFileSelector::select`, 179 URLs) | 72 M |
| — `QResourceRoot::findNode`, 62,580 calls | 65 M |
| the rest: imports, qmldir lookups, type resolution, the 49 compiled units | ~125 M |

### QSGRenderThread, 104 M, and NVIDIA's driver worker, 93 M

Both scale with the number of frames: 209 frames rendered before `ready` (`syncAndRender`, about 0.5 M each, a
third of it text nodes and the distance-field glyph cache), and the driver's worker thread does work per frame (206
calls into the same entry point). Under Pin the start is about 50 times slower, and the grid's BusyIndicator keeps
Qt rendering until the list arrives, so the count is far higher than at normal speed. The same worker thread counted
40 M in another run of the same start.

### The rest, about 70 M

| Threads | Instructions |
|---|---:|
| image providers (10 pooled threads) | 28.5 M |
| — opening a catalog connection on each (`Catalog::open`, 2.26 M each) | 22.6 M |
| Windows' thread pool at process start (the DLL loader's parallel work) | 21 M |
| small NVIDIA driver threads, COM's RPC thread | ~10 M |
| QDxgiVSyncThread, QQuickPixmapReader, thumbnail cache | ~4 M |

A catalog connection costs 2.26 M to open, against 0.88 M before: `execute_batch`'s first statement makes SQLite
parse the schema, which tracked work's triggers and indexes made larger.

## Leads, by size

1. **`heal()` on the GUI thread, 91 M:** a regression; run it as the cache writer thread's first job, as planned.
2. **Qt's font fallback query, 215 M:** find which character triggers it, and whether Qt can be given the fallback
   fonts so it doesn't populate every family (Qt's source in `C:\Qt\6.12.0\Src`).
3. **The Direct3D 9 probe, 165 M:** deferred (PLAN, user decision 2026-10-31).
4. **The file selector's qrc lookups, 72 M,** on the type loader the load waits for: whether the engine can do
   without `QQmlFileSelector`.
5. **A catalog connection per image thread, 22.6 M:** each parses the schema.
6. **The list, 6,900 instructions per file:** fewer columns, or the extension as a number.

Out of reach: NVIDIA's signature checks (733 M per device), Qt's type registration (106 M). Not costs at normal
speed: the render thread's and the driver's per-frame work.


## Startup (2026-10-31)

Measured on a copy of the real catalog (189,518 files, 690 folders; session "All folders", recursive, by name) and
its thumbnail cache (189,518 thumbnails at 220 px, 2 GB), release build, timestamps logged from process start
(temporary traces). "Cold" is the first run after copying the files; "warm" the next one.

### Before

| Step | Cold | Warm | On the GUI thread? |
|---|---|---|---|
| Process start → `main` (DLLs) | 0.3 s | 0.03 s | — |
| D3D11 AV1 probe (`video_decoding`) | 0.33 s | 0.32 s | yes, before anything |
| `state::init`: `SELECT sum(bytes) FROM thumbs` | **10.7 s** | 0.46 s | yes, **before any window** |
| `engine.load` until `onCompleted` | 3.0 s | 2.1 s | yes |
| — of which Qt Multimedia's hardware probe | 1.36 s | 1.36 s | yes |
| `onCompleted`: the grid's list (`list_files`, 189k rows) and layout | 1.0 s | 0.75 s | yes |
| **First frame** | **15.4 s** | **3.7 s** | |
| The job's `pending()`, holding the cache mutex the thumbnails need | 1.7 s | 1.6 s | no, but it blocks them |

  Note 2026-11-06: the names in this section have since changed. `state::init` is not in the tree, and the cache file is
  `thumbs.sqlite` (`crates/thumbcache/src/lib.rs:130`), not `thumbs-v3.sqlite`. The measurements stand as recorded.

- **The cache's layout made every metadata scan read the JPEGs' pages.** `thumbs` was one rowid table with each
  ~10.5 KB JPEG inline as its last column, so each row's start (with `bytes`, `mtime`, `last_used`) sat on its own
  leaf page among the blobs' overflow pages. `sum(bytes)` read 522 MB in 127k page reads. Measured read-only with
  `GetProcessIoCounters`.
- **`pending()` read 2 GB with nothing pending.** It looks each catalog file up in `thumbs`, in the catalog's
  on-disk order: random order for `thumbs`. With SQLite's default 2 MB page cache every lookup re-read its index
  and table pages: 498k reads (2.6 per file), 6.2 s even with the file in Windows' cache. It ran under the mutex
  the on-screen thumbnails lock, after the import pass at startup and at every row-height change.
- **Qt Multimedia probes every hardware device type for 1.4 s** (CUDA 0.11 s, QSV 0.85 s, Vulkan 0.3 s;
  `qt.multimedia.ffmpeg.hwaccel`, "Device types checked"), on the GUI thread when the hidden viewer's player is
  made. It does so for `QT_FFMPEG_ENCODING_HW_DEVICE_TYPES` too: unset, the list is probed (`qffmpeghwaccel.cpp`,
  `deviceTypes(const char*)`), and the app never encodes through Qt. Our `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` was
  set only on a GPU without AV1 decoding, so on one with it the decoders were probed as well.
- **SQLite details found on the way** (Python 3.12's SQLite 3.49, in memory): after an upsert that updates,
  `last_insert_rowid()` is the id of the last row *inserted*, another file's, so keying the JPEGs by it would have
  put one file's thumbnail under another; `RETURNING id` is right on both paths. A deferred transaction that
  reads, then writes after another connection committed, fails at once with `SQLITE_BUSY_SNAPSHOT` (517), whatever
  the busy timeout: two processes writing the old cache (the app and `gallery thumbs`) could fail at random.
  `INSERT OR REPLACE` fires delete triggers only with `recursive_triggers` on. On Windows a SQLite file open
  elsewhere can't be deleted (WinError 32: SQLite opens without `FILE_SHARE_DELETE`).
- **Two more, found reviewing the change.** `PRAGMA incremental_vacuum(N)` frees one page per step (it returns a
  row per page), and rusqlite's `execute_batch` steps each statement once: through it the old cache's "vacuum"
  freed one page a call. It has to be stepped to the end. `sqlite3_interrupt` only stops the statements running
  at that moment: one arriving between two statements of a read is lost (and `in_scope` dropped the failed rows
  of an interrupted folder scan, so the read went on). A flag checked by a progress handler, cleared only when the
  next request starts, cancels the whole read.

### After

The same catalog copy, with the new cache filled (`gallery thumbs --height 220`, 183,612 thumbnails, 2.00 GB, 492 s
at idle priority). "First" is the first run after a new build, whose pages and the catalog's weren't in
Windows' cache (the fill had just read 2 GB of images); "warm" the next one.

| Step | First | Warm |
|---|---|---|
| Process start → `main` | 0.07 s | 0.02 s |
| `state::init` (the totals are one row) | 0.01 s | 0.01 s |
| Qt starts, the D3D11 probe (on a thread) joined | 0.38 s | 0.33 s |
| `engine.load` (no Qt probe: both lists set) | 1.33 s | 0.70 s |
| **First frame**, "Loading…" | **1.80 s** | **1.07 s** |
| The grid's list, read off the GUI thread, shown | 4.5 s | 2.2 s |
| `gallery thumbs` with nothing pending: open, heal, `pending()`, all of it | 1.0 s | |

- **The window paints 3.5× sooner warm, and no longer waits for the disk.** Nothing before the first frame
  reads the cache or the files; the catalog's only read there is opening it.
- **`pending()` with 189k files to do takes 0.5 s** (it was 1.6 s with nothing to do, and blocked the
  thumbnails); the first run of the new build made the empty `thumbs-v3.sqlite` and deleted the old 2 GB file.
- **What's left in `engine.load` is Qt's.** Creating the viewer's `MediaPlayer`, `AudioOutput` and
  `VideoOutput` (which loads the FFmpeg plugin) made no measurable difference in Qt's `qml` tool: 0.61–0.82 s from
  process start to the first frame with them, 0.64–0.80 s without, over three runs each. So they aren't deferred.
- **The cache's totals held** through a `gallery thumbs --cap-gb 1.9 --height 100` run killed after 20 s: 1.897 GB
  in 187,068 entries, equal to the entries' sum, no JPEG without its entry; the 11,000 new 100 px thumbnails had
  evicted 220 px ones, the least recently used first.
