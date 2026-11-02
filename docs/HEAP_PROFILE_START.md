# Heap profile: the app's start on the default catalog (dhat, 2026-11-01, after three rounds of fixes 2026-11-02)

Every Rust heap allocation from process start until the window has loaded and settled, charged to the code of ours
that decides to make it, each site read at its source lines. How the profiles are made and read: PROFILING.md
"Tool: dhat". The import and the scan: [HEAP_PROFILE.md](HEAP_PROFILE.md).

**Run:** `target/dhat/gallery.exe --screenshot=…\ready.png` (commit d2205ba, `--profile dhat --features dhat-heap`,
Qt deployed beside it with windeployqt), untrimmed (`PG_DHAT_UNTRIMMED=1`), `PG_TRACE=1`, no `PG_CATALOG_DIR`: the
default catalog `D:\PhotoGallery\catalog` and the default preferences. `--screenshot` quits 4 s after the window is
`ready`. Profile: `D:\PhotoGallery\scratch_dhat\start\dhat-heap.json`.

**What it opened:** the pinterest library, 189,518 files in 690 folders, all analyzed, none pending, no writes queued,
all 189,518 thumbnails cached (`gallery status`). The session view is "All folders", by name, 220 px rows; the
window (1600 × 1000) showed 151 thumbnails. Stages under dhat: `qml_loaded` 853 ms, `tree_shown` 1010 ms,
`list_shown` 1284 ms, `ready` 1317 ms, and the background job's first round done at 1380 ms with nothing to do (no
import, no borders, no thumbnails). The process ended at 6.6 s.

**Not in it:** Qt's allocations (C++, the C runtime's heap) and SQLite's page caches (C). This is the Rust side only.

## After the third fixes (2026-11-02)

The same run at commit 5951f06, offscreen, on the same library (189,518 files, all analyzed, the cache complete) with
the default preferences. `row_height` is 220, the code's default: an earlier test run with `--size=150` had saved 150
in `gallery.json`, and the profile made with it is discarded. The earlier test runs (view, zoom and size hooks) used
the default catalog too, so the window's placeholders are not an exact repeat: 101 this run, 118 before. Profile:
`D:\PhotoGallery\scratch_dhat\third\start\dhat-heap.json`. `ready` came at 852 ms under dhat (977 ms in the second
round's run).

| | Before (d2205ba) | After the first fixes (4e1ad4d) | After the second (1d3685b) | After the third (5951f06) |
|---|---:|---:|---:|---:|
| Allocated | 38.9 MB in 16,523 blocks | 13.1 MB in 686 blocks | 12.9 MB in 521 blocks | 11.9 MB in 354 blocks |
| Peak | 17.2 MB in 2,125 blocks | 11.4 MB in 131 blocks | 11.4 MB in 131 blocks | 11.5 MB in 118 blocks |
| At exit | 1.9 KB in 26 blocks | 2.1 KB in 26 blocks | 2.1 KB in 26 blocks | 1.2 KB in 15 blocks |

- **The thumbnails' JPEGs are no longer copied per thumbnail.** A cache hit is read into the provider thread's kept
  buffer (`thumbs.rs:775`): 13 blocks, 208 KB, grown to the largest thumbnail that thread read. The second round's 118
  blocks (1.18 MB) are gone. dhat names clap's `StyledStr::push_str` for those 13 blocks, but the stack goes through
  rusqlite's `query_row` under `with_reader`: the name is most likely a folded copy of the generic growth code, which
  the linker merged with clap's function.
- **The placeholders:** 101 blocks, 290 KB, one RGBA image per visible placeholder, handed to Qt without a copy (by
  design).
- **The list is the start's cost:** 10.6 MB in one block (`browse.rs:635`, 89% of all the bytes). That is 56 bytes for
  each of the 189,518 files, made at its exact count. It is the peak as well.
- **The folder tree:** 4 blocks, 137 KB (`browse.rs:211`, `:212`, `:214`, `:261`), each made once at its size.
- **The arguments** are read once: `main` (`main.rs:50`), parsed once (`command_line.rs:68`), 19 blocks, 1.3 KB.

### What is left after the third fixes

| Blocks | Bytes | Site | What |
|---:|---:|---|---|
| 1 | 10.6 MB | `browse.rs:635` `list_grid` | The list: 56 bytes for each file in the library, at its exact count |
| 1 | 537 KB | `layout.rs:34` | The rows at their exact count |
| 1 | 90 KB | `gallery.rs:595` `lay_out` | The rows' versions |
| 101 | 290 KB | `state.rs:296` → `thumbhash` `thumb_hash_to_rgba` | Each visible placeholder's RGBA, handed to Qt: by design |
| 13 | 208 KB | `thumbs.rs:775` `with_reader` → `query_row` | Each provider thread's kept JPEG buffer for cache hits (the clap name above) |
| 4 | 137 KB | `browse.rs:211`, `:212`, `:214`, `:261` | The folder tree, each made once at its size |
| 10 | 4.7 KB | `thumbs.rs:771` | Each provider thread's connection to the cache |
| 223 | 24 KB | everything else | Threads and channels, the connections and their statement caches, the arguments (19 blocks), the drive letters (12 blocks, 96 bytes), the catalog's opens and the background job |

What is left is the list and the placeholders' images, both by design. The thumbnails' buffers are kept per provider
thread, so each thumbnail shown adds no allocation of its own.

## After the second fixes (2026-11-02)

The same run at commit 1d3685b, offscreen, on the same catalog and session (`session.file` 8838). Profile:
`D:\PhotoGallery\scratch_dhat\final\start\dhat-heap.json`. A release build was started once on the catalog
beforehand, so that the background job had already looked at the six files `import --again` had just re-analyzed.
The run then found nothing to do, as before. Under dhat, `ready` came at 977 ms. The release builds' stage times are
in PROFILING.md.

| | Before (d2205ba) | After the first fixes (4e1ad4d) | After the second (1d3685b) |
|---|---:|---:|---:|
| Allocated | 38.9 MB in 16,523 blocks | 13.1 MB in 686 blocks | 12.9 MB in 521 blocks |
| Peak | 17.2 MB in 2,125 blocks | 11.4 MB in 131 blocks | 11.4 MB in 131 blocks |
| At exit | 1.9 KB in 26 blocks | 2.1 KB in 26 blocks | 2.1 KB in 26 blocks |

| Area | After the first fixes | After the second |
|---|---:|---:|
| The thumbnails and placeholders (118) | 457 blocks, 1.52 MB | 332 blocks, 1.51 MB |
| The folder tree | 68 blocks, 378 KB | 28 blocks, 139 KB |
| The list and the grid | 37 blocks, 11.2 MB | 37 blocks, 11.2 MB |
| The command line | 51 blocks, 3.5 KB | 51 blocks, 3.5 KB |
| Catalog opens, the background job, the writer, the rest | 73 blocks, 3.6 KB | 73 blocks, 3.6 KB |

- **Placeholders:** each hash is read in place into an inline value (`browse.rs:953`–`967`), so the 118 blocks of
  `browse.rs:903` are gone.
- **The folder tree:** its paths, keys and rows are made once at their sizes, read with the rows
  (`browse.rs:210`–`213`). The nodes are reserved at their exact count (`browse.rs:260`). That is 4 blocks, from 44
  for the four arenas growing.
- **The peak** is still the list (10.6 MB, `browse.rs:634`), with the rows at their exact count (`layout.rs:34`) and
  their versions (`gallery.rs:595`).

### What is left after the second fixes

| Blocks | Bytes | Site | What |
|---:|---:|---|---|
| 118 | 1.18 MB | `thumbs.rs:427` | Each thumbnail's JPEG read from the cache: the bytes Qt decodes. By design |
| 118 | 312 KB | `state.rs:276` | Each placeholder's image: handed to Qt without a copy. By design |
| 24 + 8 + 16 + 16 | 12 KB | `thumbs.rs:430`, `:422`, `:718`, `:715` | Each provider thread's first use of the cache: its connection and statements |
| 6 + 2 | 1.2 KB | `browse.rs:967`, `:953` | The placeholder statement on each provider thread |
| 1 + 1 + 1 + 1 | 137 KB | `browse.rs:210`, `:211`, `:213`, `:260` | The folder tree's paths, keys, rows and nodes, each made once at its size |
| 12 | 96 B | `browse.rs:389` | A visible folder's drives (`D:`) as text for its QML role, per call. Qt needs a `QString` made from it anyway |
| 4 + 1 | 1.2 KB | `folders.rs:152`, `:164` | The folder model's visible and expanded lists |
| 1 | 10.6 MB | `browse.rs:634` | The list |
| 1 + 1 | 627 KB | `layout.rs:34`, `gallery.rs:595` | The rows at their exact count, and their versions |
| 14 + 14 + 15 + 3 | 3.2 KB | `command_line.rs:53`, `:57`, `:59`, `main.rs:85`, `:49` | The arguments, read once each by the test hooks, by Qt's application and by `main` |
| 10 + 2 | 426 B | `catalog.rs:704`, `:389` | Catalog opens: paths only |
| ~60 | ~4 KB | `runner.rs`, `writeback.rs`, `library.rs`, `loader.rs`, `gpu_device.rs`, `settings.rs`, `thumbs.rs:394`–`:542`, `:787`–`:838` | Threads, channels, paths, the cache's writer and its first replies |

What is left is what the start shows (the list, the thumbnails' bytes, the placeholders' images), each thread's
setup, and the arguments, read three times (by `main`, by Qt's application and by the test hooks: about 45
blocks, 3 KB, once).

## After the first fixes (2026-11-02)

The same run at commit 4e1ad4d, offscreen (`QT_QPA_PLATFORM=offscreen`, which changes Qt's allocations but not
ours), on the same catalog and session. Profile: `D:\PhotoGallery\scratch_dhat\after\start\dhat-heap.json`.

- **Session restored before the run.** Name searches run on this catalog the day before had left it with no
  selected file (`session.file` = -1). It was set back to file 8838, the one selected in the first run. The window
  then showed the same rows, with the same file selected.
- **Stage times aren't comparable.** Under dhat and offscreen, `ready` came at 695 ms, against 1,317 ms on screen
  before. The start's times are measured on release builds instead (PROFILING.md).

| | Before | After |
|---|---:|---:|
| Allocated | 38.9 MB in 16,523 blocks | 13.1 MB in 686 blocks |
| Peak | 17.2 MB in 2,125 blocks | 11.4 MB in 131 blocks |
| At exit | 1.9 KB in 26 blocks | 2.1 KB in 26 blocks |

| Area | Before | After |
|---|---:|---:|
| The folder tree | 11,979 blocks, 864 KB | 68 blocks, 378 KB |
| The thumbnails and placeholders | 3,106 blocks for 151 thumbnails | 457 blocks for 118 |
| The list and the grid | 762 blocks, 32.5 MB | 37 blocks, 11.2 MB |
| The command line | 427 blocks, 63 KB | 51 blocks, 3.5 KB |
| Catalog opens | 114 blocks, 1.5 MB | 12 blocks, 426 B |
| The background job, the writer, the rest | 135 blocks, 23 KB | 61 blocks, 3.1 KB |

- **118 thumbnails, not 151.** The view is now placed on the selected file before any cell is made, so the rows at
  the top are no longer made and dropped.
- **The peak is the list:** 189,518 entries of 56 B, 10,613,008 B, made at its exact size (`browse.rs:603`). Beside
  it are the layout's rows at their exact count (`layout.rs:34`, 537 KB) and the rows' versions as the model
  inserted them (`gallery.rs:595`, 90 KB). Before, the peak was the list at 14.7 MB with 4.1 MB unused, plus the
  aspects' `Vec` (1.5 MB) and the doubled rows.
- **At exit:** threads still running and statics, as before (`runner.rs:77`, `:90`, `writeback.rs:82`,
  `library.rs:154`, `:160`, the selected file's details and border sides, `browse.rs:646`, `:662`, `:671`, `:698`),
  plus the command line's arguments, parsed once into a static (`command_line.rs:57`, `:59`). Nothing leaks.

### What was left after the first fixes

| Blocks | Bytes | Site | What |
|---:|---:|---|---|
| 118 | 1.18 MB | `thumbs.rs:392` | Each thumbnail's JPEG read from the cache: the bytes Qt decodes. By design |
| 118 | 312 KB | `state.rs:247` → `thumb_hash_to_rgba` | Each placeholder's image: handed to Qt without a copy. By design |
| 118 | 2.4 KB | `browse.rs:903` | Each placeholder's hash (20 B) read into a `Vec`. **Lead:** read it in place (`get_ref`) into the decoder |
| 18 + 6 + 12 + 12 | 9 KB | `thumbs.rs:387`, `:395`, `:678`, `:681` | Each provider thread's first use of the cache: its connection and statements |
| 16 + 4 + 3 + 3 | 12 KB | `thumbs.rs:758`, `:377`, `:362`, `:708` | The cache's writer: its batches, channel and touches |
| 13 + 13 + 9 + 9 | 376 KB | `browse.rs:211`, `:47`, `:256`, `:220` | The folder tree's four arenas (paths, keys, rows, nodes) growing by doubling. **Lead:** reserve them from the folder count the query returns |
| 1 | 10.6 MB | `browse.rs:603` | The list |
| 1 + 1 | 627 KB | `layout.rs:34`, `gallery.rs:595` | The rows at their exact count, and their versions |
| ~50 | ~3.5 KB | `command_line.rs`, `main.rs:49`, `:85` | The arguments, read once |
| 12 | 426 B | `catalog.rs:705`, `:386` | Catalog opens: paths only |
| ~60 | ~3 KB | `runner.rs`, `writeback.rs`, `library.rs`, `loader.rs`, `gpu_device.rs`, `settings.rs` | Threads, channels and paths |

All six leads at the end were fixed (plan steps 6, 8, 9, 10). The old lead 6, the page caches, was measured in step 10:
the loader releases its cache after each job.

## Before the fixes (2026-11-01, d2205ba)

### Totals

38.9 MB in 16,523 blocks. Peak 17.2 MB in 2,125 blocks at 1.16 s (while the list was laid out). 1.9 KB in 26 blocks
at exit.

| Area | Blocks | Bytes | At the peak |
|---|---:|---:|---:|
| The folder tree (`browse::folder_tree`, `folders.rs`) | 11,979 (72.5%) | 864 KB (2.2%) | 168 KB |
| The 151 thumbnails and their placeholders | 3,106 (18.8%) | 4.0 MB (10.2%) | 0 |
| The list and the grid (`list_grid`, `relayout`, `justify`, `image_key`, the selected file) | 762 (4.6%) | 32.5 MB (83.4%) | 17.0 MB |
| The command line (our test hooks, clap) | 427 (2.6%) | 63 KB | 259 B |
| Catalog opens (14) | 114 (0.7%) | 1.5 MB (3.9%) | 72 B |
| The thumbnail cache's writer, the background job, the rest | 135 (0.8%) | 23 KB | 2.1 KB |

### The peak: 17.2 MB

| Live | Bytes | Why |
|---|---:|---|
| The list (`browse.rs:373`) | 14,680,064 (85.6%) | 189,518 `Entry`s of 56 B in a `Vec` collected without a size hint: its capacity doubled to 262,144, so 4.1 MB of it is unused, and it is kept so for as long as the list is shown |
| The aspects (`gallery.rs:490`) | 1,516,144 (8.8%) | `relayout` collects every entry's aspect into a `Vec<f64>` to pass to `justify` |
| The rows (`layout.rs:30`) | 786,432 (4.6%) | `justify`'s `Vec<Row>`, doubled to 32,768 rows of 24 B |
| The folder tree | 168,170 (1.0%) | 693 nodes, their names, paths and drives, and the tree's `Vec`s |
| The rest | 2,628 | QObjects' Rust halves, channels, the thumbnail cache |

### At exit: 1.9 KB

Held by threads that are still running or by statics, by design: the background job's and the writer's thread
state (`runner.rs:75`, `:88`, `writeback.rs:84`, `library.rs:154`, `:160`; they are left to end with the process,
`library::stop`), the selected file's details and border sides (`browse.rs:399`, `:415`, `:424`, `:451`, held by
the open view), the catalog reader (`loader.rs:52`, `:66`), connections' paths (`catalog.rs:333`), the thumbnail
cache's paths (`state.rs:35`, `thumbs.rs:130`, `thumbs.rs:403`), the panic log's path (`panic_log.rs:17`), the
catalog folder (`main.rs:68`, `:69`), clap's default-value strings (`cli.rs:33`, `:108`, `:111`). Nothing leaks.

### Every site

Sorted by blocks within each area.

#### The folder tree: 11,979 blocks for 693 nodes

`folder_tree` (`browse.rs:112`–`175`) reads the 690 folders and builds the merged tree. For each folder it rebuilds
the merged path one component at a time from empty, and lowercases each prefix whole as the key for `by_key`: about
five components per folder, each with its own `String`s.

| Blocks | Bytes | Peak | Site | What, and what it would take |
|---:|---:|---:|---|---|
| 3,441 | 97.7 KB | 0 | `browse.rs:136` `merged.to_lowercase()` | The key of every prefix of every folder: 690 folders × ~5. A key per node of (parent id, lowercased component) would lowercase each component once, and only for the nodes it creates. |
| 2,072 | 58.2 KB | 0 | `browse.rs:135` `merged.push_str(part)` | `merged` rebuilt from empty for each folder, growing 3 times. Reusing one `String` across folders (`clear()`) keeps its capacity. |
| 1,371 | 131 KB | 0 | `browse.rs:130` `components(…).collect()` | A `Vec<&str>` of the components per folder, 2 blocks as it grows; iterating `components` directly needs none. |
| 764 | 142 KB | 0 | `browse.rs:121` `SELECT id, path FROM folders` into a `BTreeMap<i64, String>` | Every folder's path and the map's nodes. |
| 694 + 693 | 7.2 KB | 0 | `browse.rs:37`, `:39` `folder_key` (from `:169`) | The sort key of every node's name: a `Vec` and a lowercased copy each. |
| 693 | 28.4 KB | 0 | `folders.rs:162` `merged_path.to_lowercase()` in `replace` | Every node's path lowercased to look it up in `keep`, the folders expanded before, which at the start is empty. Skipping the filter when `keep` is empty removes them all. |
| 693 | 28.4 KB | 28.4 KB | `browse.rs:140` `merged.clone()` | The node's merged path. Kept. |
| 693 | 5.5 KB | 5.5 KB | `browse.rs:160` `node.drives.push_str(drive)` | `"D:"` as a `String` on every node. A drive set as bits (26 letters) needs no block. |
| 693 | 2.2 KB | 2.2 KB | `browse.rs:139` `part.to_string()` | The node's name. Kept (it is also the end of `merged_path`). |
| 116 | 44.3 KB | 0 | `browse.rs:137` `by_key.entry(…)` | The `BTreeMap`'s nodes. |
| 15 + 1 | 16.7 KB | 8.4 KB | `browse.rs:149`, `:150` children and roots `push` | |
| 9 | 245 KB | 123 KB | `browse.rs:138` `tree.nodes.push` | The node `Vec` doubling. |
| 9 | 32.7 KB | 0 | `browse.rs:117` the counts' `Vec` | Doubling. |
| 9 + 3 + 1 + 1 + 1 | 1.6 KB | 312 B | `folders.rs:181`, `:170`, `:173`, `:174`, `:167`, `:169` | `replace` cloning roots and children to walk them, and the visible list. |
| 1 | 16.6 KB | 0 | `browse.rs:169` the names' keys `Vec` | |
| 1 | 5.4 KB | 0 | `browse.rs:172` `sort_by` | The stable sort's scratch buffer. A parent's children have distinct keys, so `sort_unstable_by` gives the same order without one. |

#### The thumbnails and placeholders: 3,106 blocks for 151 thumbnails

Each visible image asks for its ThumbHash placeholder and its thumbnail, on Qt's image-provider threads (10 of them
opened a catalog connection).

| Blocks | Bytes | Peak | Site | What, and what it would take |
|---:|---:|---:|---|---|
| 906 | 448 KB | 0 | `state.rs:136` `thumbhash::thumb_hash_to_rgba` | The crate decoding each placeholder: 6 blocks a call. Its API. |
| 302 | 72.5 KB | 0 | `browse.rs:588` `file_source`'s `prepare_cached` | rusqlite's cache key, a copy of the SQL (240 B) on every call. |
| 302 | 27.1 KB | 0 | `browse.rs:596` the path | Read for both requests; the placeholder doesn't use it. |
| 302 | 19.3 KB | 0 | `providers.rs:75` `id.split('/').collect()` | A `Vec<&str>` to parse the image id; `split_once` twice needs none. |
| 302 | 6.1 KB | 0 | `browse.rs:599` the ThumbHash | Read for both; the thumbnail doesn't use it. Two queries, each with the columns its request uses, would drop this and the path's half. |
| 302 | 1.5 KB | 0 | `browse.rs:594` `r.get::<_, String>(4)` | The kind as a `String` to compare; `get_ref(4)?.as_str()?` reads it in place (as `details` does). |
| 151 | 1.48 MB | 0 | `thumbs.rs:309` the cached JPEG | The thumbnail's bytes, read from the cache. |
| 151 | 1.48 MB | 0 | `state.rs:108` `data.clone()` | The same bytes copied, because `cached` is matched by reference; matching it by value returns them without a copy. |
| 151 | 409 KB | 0 | `decode.rs:243` `transform` from `state.rs:137` | The placeholder copied pixel by pixel at orientation 1 (no turn): `transform` has no identity shortcut (`orient` has). |
| 151 | 39.9 KB | 0 | `thumbs.rs:304` `get_nearest`'s `prepare_cached` | rusqlite's cache key. |
| 30 + 24 + 16 + 16 | 12.3 KB | 0 | `browse.rs:603`, `thumbs.rs:312`, `thumbs.rs:592`, `:589` | Each provider thread's first use: its catalog and cache connections and their statement caches. |

#### The list and the grid: 762 blocks, 32.5 MB

| Blocks | Bytes | Peak | Site | What, and what it would take |
|---:|---:|---:|---|---|
| 687 | 17.0 KB | 0 | `gallery.rs:514` `image_key`'s `format!` | The image id (`id@mtime.rotation.turns`): 687 blocks for 151 delegates, the QML asking for each key several times. |
| 17 | 29.4 MB | 14.7 MB | `browse.rs:373` `entries.collect()` | The list: 17 doublings to 262,144 slots for 189,518 entries, 29.4 MB allocated and copied for 10.6 MB kept, 4.1 MB of it unused while the list is shown. The folder tree knows the folder's file count, an upper bound for any filter: `Vec::with_capacity` with it, then `shrink_to_fit` when a filter left much less. |
| 14 | 1.57 MB | 786 KB | `layout.rs:30` `rows.push` | Doubling to 32,768 rows; a starting capacity estimated from the width and the row height would skip most of it. |
| 1 | 1.52 MB | 1.52 MB | `gallery.rs:490` the aspects `Vec<f64>` | Built on every `relayout` (every resize, every row-size change), only to be read once. `justify` taking `impl Iterator<Item = f64>` (or the entries and a closure) needs none. |
| 5 | 128 B | 0 | `gallery.rs:397` `open_folder` | The query and path for the reader. |
| 3 | 1.4 KB | 0 | `browse.rs:354` `grid_sql`'s `format!` | The list's SQL. |
| 3 + 2 + 1 | 489 B | 144 B | `loader.rs:70`, `:68`, `:85`, `:66` | The catalog reader thread and its jobs. |
| 2 + 2 + 1 + 1 + 1 + 1 | 650 B | 0 | `query.rs:279`, `:633`, `:626`, `:344`, `:375`, `browse.rs:85`, `:86` | The session's query and sort as JSON and SQL, the scope's bounds. |
| 2 + 2 + 1 | 284 B | 0 | `gallery.rs:744`, `:740`, `:742`, `:331` | The status line of the selected file (`info`). |
| 3 + 1 + 1 + 1 | 760 B | 0 | `browse.rs:399`, `:415`, `:424`, `:451` | The selected file's details and border sides. |

#### Catalog opens: 14, 114 blocks

`Catalog::open` (`catalog.rs:580`) runs on each new connection: the GUI thread's (`state.rs:36`), the catalog
reader's (`loader.rs:144`), the background job's (`runner.rs:175`), the writer's (`writeback.rs:154`), and one per
image-provider thread that asked for something (`state.rs:70`): 8 for thumbnails, 2 for placeholders. Each one builds
the schema text (`baseline()`, `catalog.rs:46`, 3 `replace`s, 108 KB) though only a new catalog uses it: 84 blocks,
1.5 MB. A `LazyLock<String>`, or building it inside `migrate` when `user_version` is 0, makes that once or never.
The rest: `Connection::open`'s path (`catalog.rs:333`, 2 a connection), `catalog_id` (`catalog.rs:598`),
`cancel_when` (`catalog.rs:607`).

Not in this profile: `connect` sets `PRAGMA cache_size = -262144` (256 MB) on every connection, so 14 page caches
could each grow to 256 MB in SQLite's heap. The provider threads only read a few rows each; they are where a smaller
cache would cost nothing.

#### The command line: 427 blocks

| Blocks | Bytes | Site | What, and what it would take |
|---:|---:|---|---|
| 252 + 43 + 20 + 2 + 1 + 1 | 18.9 KB | `command_line.rs:52`, `:51`, `:71`, `:87`, `:75`, `:52` | Every `CommandLine.has`/`text` call from the QML (43 at `started`) formats `--name=` and parses the whole command line again with `std::env::args()`, about 6 blocks a call. Parsing the arguments once into a static map needs none per call. |
| 93 | 43.2 KB | `cli.rs:20`–`112` (29 sites) | `cli::wants_cli` (`cli.rs:123`) builds clap's whole `Command` to compare the arguments with the subcommands' names, also when there are no arguments (the normal start). Checking `args.is_empty()` first, or a list of the names, skips it. |
| 15 | 1.1 KB | `main.rs:49` | `args_os().collect()`. |

#### The rest: 135 blocks

The thumbnail cache's writer and its channel (`thumbs.rs:276`–`279`, `:294`, `:661`, `:669` the touched entries'
`BTreeSet`, `:706`, `:712`), the background job's first round checking for work (`catalog.rs:502`, `:510` the
import's query, `thumbs.rs:531`, `:543`, `:567` the thumbnails' query, `thumbs.rs:401`, `:403`, `:422`, `:424`, the
`checked` and `shrink` replies, `catalog.rs:672` the roots, `thumbs.rs:614`, `:619` touches written), the threads
(`runner.rs:75`, `:88`, `writeback.rs:84`, `:95`, `gpu_device.rs:60`, `:62`), the QObjects' Rust halves
(`create_rs_*`), the settings and session (`settings.rs:71`–`85`, `browse.rs:650`), `state::init` and `run_app`'s
paths. All once per start.

### Leads, by what they would save

1. **The list's capacity** (`browse.rs:373`): 29.4 MB allocated and copied to build a 10.6 MB list, and 4.1 MB
   unused for as long as it is shown. Every folder change and filter builds a list.
2. **`relayout` without the aspects `Vec`** (`gallery.rs:490`, `layout.rs:20`): 1.5 MB at every resize and row-size
   change, plus the rows `Vec`'s doublings.
3. **The folder tree's construction** (`browse.rs:126`–`166`): 72.5% of the start's blocks, about 17 per folder.
   Keyed by (parent, lowercased component), reusing one `String`, no per-folder `Vec`, drives as bits, and
   `folders.rs:162` skipped when nothing was expanded: a few blocks per new node instead of 17 per folder.
4. **The thumbnail request path:** the cached JPEG returned without a copy (`state.rs:108`), the identity
   `transform` skipped (`state.rs:137`), each request reading only its columns (`browse.rs:588`–`600`), the image id
   split without a `Vec` (`providers.rs:75`).
5. **Once per open or per start:** `baseline()` built 14 times (`catalog.rs:46`), clap's `Command` built at a
   normal start (`cli.rs:123`), the test hooks re-reading the arguments at every call (`command_line.rs:50`–`53`).
6. **Not measured here:** a 256 MB SQLite page cache allowed on each of 14 connections (`catalog.rs:336`).
