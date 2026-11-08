# Workspace refactor: from 3 crates to a layered, acyclic set of crates

## Context

The workspace has three crates: `pg-core` (now about 14k lines, with the new `codecs.rs`), `pg-ingest` (2.7k) and
`pg-ui` (4.4k Rust + 2.9k QML). A web client is planned (docs/WEB_CLIENT.md). It needs a UI-free application layer,
and the goal is a full refactor beyond that split: the most proper design, with churn accepted and no arbitrary size
limits: files are split by cohesion.

**Revalidated 2026-11-02 at HEAD f3d2e60**, 36 commits after 4e1ad4d (the previous check). What the intervening commits added,
and what this plan now accounts for:

- decoders for PNG, GIF, WebP, BMP, TIFF and the JPEG fallback, driven directly into per-worker buffers
  (`core/src/codecs.rs`, 1198 lines, new), owned by `decode::Decoder`; eight more vendored crates (png, fdeflate, gif,
  image-webp, zune-core, zune-jpeg, tiff, quick-xml);
- metadata read into kept buffers (`metadata::MetaScratch`), with the container walk as `Container`;
- `paths.rs` gained arena work lists: `PathArena`, `PathRef`, `FilePath`, `WorkList<T>`; the catalog's pending lists are
  `WorkList<WorkItem>`; `CropSource.format` is a typed `Format`;
- inline results: `analyze::ThumbHash` (25 bytes), `borders::Sides` (`ArrayVec<Side, 12>`);
- catalog: `close()` (WAL merge), `files_under`, `mark_pending` (for `import --again`), `crop_source(file_id, &mut PathArena)`;
- thumbnails: `thumbs::pending_in` reads the catalog's tables directly; `with_reader` keeps reader connections in a
  thread-local;
- ingest: `pipeline.rs` reader pool (`Pool`, `Lease`, `read_whole`, `open` through `CreateFileW`); runner counters
  `scans_done`/`imports_done`; `import --again`; the writer keeps one connection;
- UI: `gui_app.rs` (the QGuiApplication built from the argv bytes), `command_line::init` (argv read once), `main.rs`
  drops the engine before `state::shutdown`, which closes the thumbnail cache, then the catalog. `filters.rs`,
  `FilterBar.qml`, `settings.rs` (gallery.json) and `view.rs` are not in the earlier plan.

Coverage of this revision:

- **Read in full:** `codecs.rs`, `decode.rs`, `paths.rs`, `state.rs`, `providers.rs` (56–154), `runner.rs` (186–316),
  `metadata.rs` (1–150, 480–970), the complete `catalog.rs` diff, the `analyze.rs` and `borders.rs` diffs, and the
  `main.rs`, `library.rs` and `gui_app` changes.
- **Checked by grep or by range, with the line content read:** every file:line reference in this plan, the crate
  usage per dependency, the deny/patch/Justfile/doc references, the QML cited ranges, `pipeline.rs`, `scan.rs`,
  `cli.rs`, `import.rs`, `control.rs`, `dups.rs`, `writeback.rs`, `browse.rs`, `thumbs.rs`, and `pdq_index.rs`.
- **Not re-read (unchanged since 2026-11-01 or since 4e1ad4d):** the bodies of `query.rs`, `rewrite.rs`, `rotate.rs`,
  `crop.rs`, `pdq.rs`, `layout.rs`, `resize.rs`, `media.rs`, `video.rs`, and the QML bodies. Their claims rest on the
  earlier full read and on the greps.

The findings below are as of f3d2e60.

- **Cycles and inverted edges in core:**
  - `catalog`↔`browse`: `catalog.rs:455` (`order_key_of`), `:1089` (`folder_key`, `merged_path`), `:1104`
    (`push_name_key`) call browse's key functions; `browse.rs:737-739` is a wrapper over `Catalog::set_border_mark`.
  - `decode`↔`metadata`: `metadata.rs:21` imports `decode::Format`; `decode.rs:141,150,186` read the EXIF orientation
    (`decode()`, `decode_stored()`). `decode_into` already takes the orientation from its caller.
  - `catalog`→`analyze`: `catalog.rs:21-22` (`Analysis`, `IMPORT_VERSION`, `BORDERS_VERSION`, `Detection`).
  - `catalog`→`pdq_index`: `catalog.rs:27`, `:850` (`PdqIndex::build`).
  - `video`→`analyze`: `video.rs:17` imports `Analyzer`; `video.rs:261-267` (`analyze`, `analyze_video`) is analysis.
  - `thumbs` mixes three things: the `Maker` pipeline (`thumbs.rs:55-197`), the `ThumbCache` store (`:355-620`), and
    `pending_in` (`:694-737`), which ATTACHes the catalog (`:698`) and reads `cat.files`/`cat.analysis` directly.
  - `ingest/src/thumbs.rs:109` and `ui/src/state.rs:114,130,294` call `browse::file_state`/`file_source`/`placeholder`.
- **God files:**
  - `catalog.rs` (2088 lines: schema, rebuild, versions, work lists, four writers, write statics, status);
  - `browse.rs` (1817: path keys, the tree builder, read models, per-file state, commands, views, session);
  - `query.rs` (997: the query model and its editing, the SQL, `pg_contains`);
  - `metadata.rs` (1538: container walkers, EXIF, XMP, dates, orientation algebra, patching);
  - `thumbs.rs` (1568: see above);
  - `Main.qml` (1081 lines; about 190 of them controller JS, counted on 2026-11-01);
  - `gallery.rs` (956: list session, row diffing and versions, layout geometry, English text, edits inside the Qt
    model).
  - `codecs.rs` (1198) is one concern (the non-JPEG decoders) and stays whole.
- **The catalog leaks:**
  - `pub conn` (`catalog.rs:354`) lets ingest, the CLI, the benches and the tests run raw SQL.
  - The `file_writes` queue is split across `browse`, `catalog` and `writeback`. `MAX_ATTEMPTS` is a constant once
    (`writeback.rs:36`) and a literal `attempts < 5` twice more (`cli.rs:286`, `tests/writeback.rs:74`); write kinds
    are strings.
  - The writer records each write through `Catalog` statics that take a `&Transaction` (`set_file_rating` `:960`,
    `file_rewritten` `:974`, `analysis_turned_back` `:996`, `add_catalog_rotation` `:1007`), passed to `done` as
    closures (`writeback.rs:313,338`; `done` at `:347`).
  - The thumbnail cache knows the catalog schema (`thumbs.rs:694-737`).
  - The `dups` report is SQL in ingest (`dups.rs:34-62`, `current!`, `groups!`, `grouped!`).
  - The format is no longer a name in `CropSource` (fixed: `catalog.rs:669-683`), but the read still parses it
    (`Format::from_name`, `catalog.rs:933`).
- **Duplication:**
  - Orientation tables in 5 places: `borders::upright` (`borders.rs:320`), `decode::transform_into` (`decode.rs:280`),
    `metadata::orientation_parts`/`orientation_from` (`metadata.rs:901,914`), `rewrite::map_point` (`rewrite.rs:77`),
    and `video::matrix_orientation` (`video.rs:72-88`, which maps the display matrix through `orientation_from`).
  - The swap rule (quarter turns swap the sides) in 11 places in Rust and SQL: `decode.rs:84,187,288`,
    `rewrite.rs:26`, `browse.rs:481,527`, `catalog.rs:930`, `video.rs:104,272`, the borders sort SQL
    (`query.rs:752-753`), and the writeback SQL (`writeback.rs:365`). Two more in the front end: `gallery.rs:707` and
    `Viewer.qml:404-405`. `Orientation::swaps_axes` and `Turns::swaps_axes` replace them all; the two SQL ones become
    one `swapped!` macro in `query_sql`.
  - The `key`/`from_key`/`ALL` boilerplate on 10 enums: borders (`Edge`, `Kind`, `Class`, `Reason`; `borders.rs:89-214`),
    `media::Kind` (`media.rs:17-24`), and query (`MediaType`, `Orientation`, `Status`, `BorderFacet`, `SortKey`;
    `query.rs:100-217,689-712`).
  - Rust enums repeated as QML string lists: `sortKeys`, `types`, `orientations`, `statuses`, `borderValues`.
  - The batch-and-flush sink 3 times, and the latest-wins request boilerplate 3 times.
  - Three URL-to-path conversions: `gallery.rs:861`, `library.rs:176`, `yandex.rs:426` (`to_local_file`).
  - Two file hashers (BLAKE3 in `analyze.rs` and `video.rs`), and two JPEG marker walks (`rewrite.rs:127`
    `jpeg_is_progressive`, `metadata.rs:582` `jpeg_segments`).
  - The image URL protocol is built in QML (`Viewer.qml:217-220,369`) and parsed in `providers.rs:101-141`.
  - Scratch-dir helpers in every test file: `catalog.rs:1431` (`test_dir`), `thumbs.rs:1042` (`scratch`),
    `ingest/tests/{crop,video,writeback}.rs` (`scratch`).
- **Service patterns** (`Runner`, `Writer`, `ui::loader`, the ThumbCache writer):
  - Progress reaches the UI by polling a message list with a single consumer (`runner.rs:183` `take_messages`,
    `library.rs:226` `poll`), so a second front end would steal its messages.
  - `Control::checkpoint` polls every 100 ms while paused (`control.rs:152-163`).
  - `ThumbCache` is passed as a leaked `&'static` (`cli.rs:278-279`, `tests/writeback.rs:101`).
  - Per-thread state: the UI's catalog connection and `Maker` (`state.rs:44-47`), the provider's JPEG buffer
    (`providers.rs:63-66`), and the thumbnail cache's reader connections (`thumbs.rs:757-779`).
  - Per-worker scratch (new): `codecs::Codecs` in `decode::Decoder`, `metadata::MetaScratch` in `analyze::Analyzer`,
    `Maker` for thumbnails. Their buffers are kept and reused; the plan keeps that (see the allocation rule).
- **App logic lives only in Qt objects and QML JS:**
  - the list session (stale guard, held list, edited/turned overlays), and its row diffing: the `Changed` kinds,
    per-row versions, and which roles each change sends (`gallery.rs:288-307,492-639`);
  - the image consistency check: the state read before and after the file, retried while a write is in progress
    (`state.rs:124-139`), and a thumbnail served from its key alone (`state.rs:192-247`);
  - layout geometry and the viewer's zoom and pan math (`Viewer.qml:106-221,355-375`);
  - tree expansion;
  - the image-key protocol;
  - the write-back rule;
  - session restore, with its folder-gone fallback and spelling normalization (`Main.qml:205-237`);
  - library change detection;
  - the Yandex workflow and its picks (`ui/yandex.rs:384-413`);
  - every English string.

  None of it can be unit-tested (`pg-ui` has `test = false`), and the edited overlay only knows this client's own
  edits.
- **Error handling and platform code:**
  - `anyhow` in every library; the one typed error is `core/src/yandex.rs:38` (`YandexError`);
  - Windows code everywhere, with no `cfg(windows)` anywhere (0 hits);
  - ExifTool's stderr is read only after stdout (`writeback.rs:477-478`), so a full stderr pipe would block it.
- **Tests and benches in the wrong place:**
  - `ingest/tests/video.rs` (213 lines) mostly tests core;
  - the UI's three tests (`ui/src/yandex.rs` ×2, `command_line.rs` ×1) never run: `pg-ui` has `test = false`;
  - the Linux-only catalog bench doesn't compile at f3d2e60: `benches/ir/catalog.rs:113` calls
    `thumbs::pending_in`, and `thumbs` is not imported (`:12-16`).

The outcome:

- every crate has one concern, and the graph is acyclic;
- heavy dependencies are confined to one crate each, enforced by cargo-deny;
- `pg-app` serves both front ends: the Qt desktop now, the axum server later;
- `pg-client` holds the front end's logic, Qt-free and WASM-safe, so the desktop is glue and the web client can
  reuse it;
- behaviour stays the same, and the allocation work of the last 36 commits survives the moves.

## Target crates (all in `crates/<dir>`, package `pg-<dir>`; bottom to top)

Status: all of these crates exist, and `pg-core`, `pg-ingest` and `pg-ui` are gone (`pg-ingest` became `pg-library` and
`pg-cli`; `pg-ui` became `pg-desktop` and `pg-gallery`). The tables and lists below are the plan as written, with the
file and line references of f3d2e60. Where the built crate differs, the ledger row under "Order of work" says so (for
example `pg-metadata` stayed one module, S-05.1).

| Crate | Concern | Deps |
|---|---|---|
| `pg-model` | domain data, pure and WASM-safe (list below) | serde, strum, arrayvec |
| `pg-client` | front-end logic, Qt-free and WASM-safe (list below) | model |
| `pg-platform` | the OS layer behind portable APIs (list below) | windows-sys |
| `pg-trace` | the stage markers (`ui/trace.rs` minus Qt), every stage in one list. Their `#[no_mangle]` names don't change, so `tools/profiling/callgraph` and `profile.ps1` still find them | platform |
| `pg-container` | walking file containers without allocating: JPEG segments and their SOF (replaces `rewrite::jpeg_is_progressive`), extended XMP chunks (`ExtendedChunk`, `extended_packets`), PNG chunks, RIFF chunks, GIF blocks (`GifBlock(s)`, `gif_blocks`, `gif_header_len`); later ISOBMFF for HEIC. The TIFF IFD reader isn't here: only EXIF reads it, so it stays in `metadata::exif` | — |
| `pg-metadata` | `exif` (the TIFF reader, `exif_date`), `xmp` (`xmp_properties`, `png_itxt_xmp`, `inflate_zlib`, `MetaScratch`), `read` (`Meta::read` and `Container::walk`, now with the scratch), `patch` (`patch_orientation`, PNG CRCs), `orientation()` (the file's EXIF orientation alone) | model, container, quick-xml, miniz_oxide, crc32fast |
| `pg-codec` | pixels in and out (list below) | model, turbojpeg, image, fast_image_resize, png, gif, image-webp, tiff, zune-jpeg, zune-core |
| `pg-video` | FFmpeg probe (`Probe.taken`), frames, `to_rgb` (the one `unsafe`), `build.rs` (FFmpeg DLL copy); applies `verbatim` itself | model, codec, platform, ffmpeg-next |
| `pg-edit` | lossless edits: `rewrite`, `rotate`, `crop` (its `reencoded` moves to codec's `encode`) | model, container, metadata, codec, png, image-webp, weezl, crc32fast, turbojpeg |
| `pg-analysis` | (list below) | model, metadata, codec, video, blake3, xxhash-rust, thumbhash, turbojpeg |
| `pg-dedup` | `PdqIndex` built from ids and hashes (`pdq_index.rs`); `Pairer`, the query-then-insert with the best distance per neighbour over the dihedral variants (out of `import.rs`, the loop at `:49-75`); union-find grouping (out of `dups.rs`); later tier 2, the verifier, multicut, keeper | model |
| `pg-catalog` | schema, migrations, rebuild; typed repositories; read models; query→SQL. Private `conn` (list below) | model, rusqlite, serde_json |
| `pg-jobs` | the job framework and the event primitives (list below) | platform, crossbeam-channel |
| `pg-thumbcache` | `ThumbCache` storage only: LRU, cap, heal, shrink. Its writer is a `jobs::Service`, its errors go on a `Bus` (today a `Report` callback), and it is shared as `Arc`. Its read connections come from a small pool it owns, not a thread-local (`thumbs.rs:757-779`). Its pending query reads only the catalog's `thumb_sources` view (`pending_in`, `:694-737`). Its compile-time SQL macros (`cache_version!`, `shrink_step_pages!`) move with it | model, jobs, rusqlite; dev: catalog, testkit |
| `pg-render` | display images (list below) | model, codec, video, metadata, thumbhash, turbojpeg |
| `pg-yandex` | the client (`core/src/yandex.rs`); `Download` returns a model `Format` confirmed by `Format::sniff` on the bytes, instead of an extension string; `thiserror` | model, ureq, serde_json |
| `pg-exiftool` | the `-stay_open` driver (`writeback.rs:422-560`): write rating, write orientation, copy tags. Stderr is drained on its own thread, each command has a timeout, and the process is killed on close (platform). The program path is given by the caller, not read from `PG_EXIFTOOL` inside | platform |
| `pg-library` | the passes, each a `Pass` with no printing (list below) | catalog, thumbcache, analysis, dedup, edit, render, codec, metadata, exiftool, jobs, platform, trace |
| `pg-app` | the UI-free application layer, sync and runtime-free (list below) | library, catalog, thumbcache, render, yandex, jobs, model |
| `pg-cli` | clap commands, every `println!`, `Progress::line`, the borders tools (`--csv` report, `--stdin` serve, `--import-review`), the `dups` printout, `import --again`, `wants_cli` (kept on `Subcommand::has_subcommand`, `cli.rs:143`, so a GUI start never builds clap's `Command`; its test moves along) | app, library, analysis, clap, anyhow |
| `pg-desktop` (lib) | Qt glue only: QObject facades over `pg-client` and `pg-app`, list models, image providers, the frame sink, the GPU device, screenshot, live reload, `gui_app` (the QGuiApplication built from the argv bytes), `command_line` (argv read once), the flat `qml/` folder, the desktop's client prefs. `[lib] test = false, doctest = false` | app, client, trace, cxx-qt, cxx-qt-lib, cxx, serde_json |
| `pg-gallery` (bin `gallery`) | `main` (CLI or GUI dispatch), `windows_subsystem`, console attach, message box, panic hook, the `dhat-heap` feature, winres icon (its build.rs). `desktop` is an optional default feature, so a headless build is possible | cli, desktop, platform, dhat (feature), winres (build) |
| `pg-testkit` (dev) | scratch dirs (`PG_TEST_DIR`), pattern, letterbox and animated test images (the only `image` user besides codec), ExifTool and ffmpeg discovery that prints SKIPPED, `wait_for_writes`. It re-exports the catalog fixture (`catalog.rs:1348`, `pub(crate) mod fixture` today), which lives in `pg-catalog` behind a `fixture` feature, since only catalog sees `conn` | model, catalog (`fixture`), image |
| `pg-bench` (dev) | the gungraun `ir` harness (`benches/ir`), so `wsl-bench.sh` has one stable target. It goes through the catalog API, not `conn` | analysis, render, catalog, thumbcache, gungraun |

**`pg-model` contents:**

- ids as newtypes (`FileId`, `FolderId`, `RootId`), `Rating`, `Turns` (new: none of these exist today);
- the EXIF `Orientation`: compose (`metadata.rs:928`), parts (`:901`), `from` (`:914`), one coordinate map, `swaps_axes`.
  It replaces the 5 tables and `borders::upright`;
- `media::{Kind, Ext}` (`media.rs`, with `Ext::of_name_utf16`), `Format` (`decode.rs:14-57`: `sniff` with BigTIFF,
  `name`, `from_name`, and `can_hold_rating` from `metadata.rs:36`);
- `pdq::{Hash, Pdq}` and the thresholds (`GROUP_DISTANCE`, `MATCH_DISTANCE`, `MIN_QUALITY`). `Hash::to_hex` is already
  gone, and `Detection::depth()` (`borders.rs:311`) has no caller: delete it;
- **`analysis::Analysis`** (`analyze.rs:27`), the record analysis produces and catalog stores:
  - shared fields, plus `ImageFacts` (`Format`, file rating, `PixelHashes`) or `VideoFacts` (codec, duration, poster);
  - `Taken::{Wall, Utc}` instead of `taken` + `taken_utc` (`analyze.rs:44-45`);
  - a `version` field stamped by its producer (new).

  The dihedral PDQ variants (`PixelHashes::pdq_dihedral`, `analyze.rs:81`) are returned beside the record, not stored
  in it;
- `ThumbHash` (`analyze.rs`, inline 25 bytes with `len`; `THUMBHASH_MAX` as a literal equal to `thumbhash::MAX_LEN`,
  with a test);
- the borders result types: `Detection`, `Side`, `Edge`, `Kind`, `Class`, `Reason`, `Sides` and `MAX_SIDES` (12 = 3
  rounds of 4 edges, `borders.rs`), and `MIN_SIDE`, which replaces the `64` hardcoded in catalog SQL (`has_borders!`,
  `catalog.rs:65-68`);
- **literal macros for compile-time SQL.** The catalog builds every statement with `concat!` and takes no run-time
  constants. So the values its SQL shares with model are exported twice: as a `macro_rules!` literal, and as the
  constant made from it (`group_distance!()` → `GROUP_DISTANCE`, `pdq.rs:18-27`; `min_side!()` → `MIN_SIDE`). A test
  checks each pair;
- `FileIdentity {volume, file_index}` (new);
- civil dates: `wall_clock_seconds` (`metadata.rs:458`), `parse_iso_date` (`:433`), and a typed `Day` (new, used by
  `query::DateRange`, `query.rs:269`);
- `path`: the app's Windows path semantics in one module. It holds:
  - `merged_path`, `components` (`browse.rs:19,27`);
  - the order keys: `folder_key`/`push_folder_key` (`browse.rs:35,42`; escapes 0x01/0x02, Σ special-cased),
    `name_key`/`push_name_key` (`:70,77`), `order_key`, `order_key_of` (`:94,102`);
  - `scope` (`browse.rs:108`, a folder as a key range);
  - `is_under` (today `catalog.rs:348`), `path_key`;
  - the work lists (today `paths.rs`): `PathArena` (`push`, `push_parts`, `get`), `PathRef`, `FilePath`, and
    `WorkList<T>` (`file`, `parts`, `path`, `items`). `WorkList::wide` calls `verbatim_wide` (platform), so the
    method moves to a platform helper and model keeps the pure part.

  The `push_*` forms write into a caller's buffer: the scanner's `ScanWriter` keeps one per transaction;
- the query data model with typed editing (`Query::toggle(Facet, value)`, `bound(RangeFacet, End)`). The facet
  `Orientation` is renamed `Shape`. There is no SQL, no serde_json and no text parsing in it. `Query` and `DateRange`
  are `query.rs:283` and `:269`;
- `Sort`/`SortKey` (`query.rs:689`), the saved `View` (`browse.rs:953`), and the `Session` record (folder, recursive,
  query, sort, last file; `browse.rs:994-1011` today as key/value pairs);
- `FolderTree`, compact as `browse.rs:129-165,327-420` has it:
  - nodes in pre-order, their names and keys as ranges of one `paths` string and one `keys` buffer;
  - the queries over that data: `len`, `node`, `merged_path`, `name`, `key`, `parent`, `subtree_end`,
    `has_children`, `children(Option<usize>)`, `merged`, `total`, `find(merged, &mut key)`, `find_key`;
  - `drives()` (`browse.rs:383`) returns the node's drive letters as data. Their text ("D: E:") is `pg-client::text`'s.

  Its builder (`TreeBuilder`, `browse.rs:192-326`: one sort, a stack of open nodes) stays in `catalog::read::tree`,
  beside the `FOLDER_COUNTS` query it consumes (`browse.rs:165`);
- the grid `Entry` DTO (56 B, asserted at `browse.rs:447`, with `content`, `shown_cut`, `side_line`, `stored_size`,
  `shown`) and `Details` (`browse.rs:667`);
- the per-file states the readers check against:
  - `FileState {mtime, rotation, turns, writing}` (`browse.rs:858`);
  - `FileSource {path, state, poster_ms}` (`:898`);
  - `Placeholder {hash, turns}` (`:926`);
- `ImageKey {file, source: Source {mtime, rotation}, turns}` (`<id>@<mtime>.<rotation>.<turns>`, today `providers.rs`'s
  `parse_key`, `:124-134`, and `state::Named`, `state.rs:143`).
  - `Display` writes without allocating (`gallery.rs:652-661` formats it into a 64-byte stack buffer, and the desktop
    keeps doing so through `write!`).
  - `FromStr` takes the key apart.
- `ImageRequest` (`Thumb(h)`, `Placeholder`, `Fit(w,h)`, `Full`, `Region(rect, w, h)`), with `Display`/`FromStr`, and
  the same clamps as `providers.rs`: the height `clamp(16, 2048)` (`:141`), the region's `min(1.0 - x)` and
  `min(16384)` (`:120`).

  `ImageKey` and `ImageRequest` are the one definition of the image URL protocol;
- `WriteKind::{Rating, Turns}`;
- every persisted enum key through `strum` derives (`snake_case`, the strings unchanged). A test lists all keys,
  since they're stored in the catalog and in the views' JSON;
- an optional `specta` feature for TypeScript types later.

Banned from model: serde_json, rusqlite, `SystemTime`, `Instant`. The `contains_ignoring_case` helper (`query.rs:783`,
used only by the test at `:856`) moves under `#[cfg(test)]`; it is not model.

**`pg-client` contents** (all unit-tested; the desktop wraps each in a facade):

- `layout`: a `Layout` value (rows, clamped aspects, spacing) with:
  - `justify_into` (into a kept buffer, as now; `layout.rs`);
  - `row_of`, `span`, `neighbor` (`gallery.rs:903-935`);
  - the row diff `RowChange::between`/`updates` → `RowUpdate {span, items, height}` (`layout.rs` and `gallery.rs:404-416`);
- `grid`: the list session.
  - The snapshot carries the catalog `seq` it was read at.
  - A list that arrives while the viewer is open is held.
  - `current` and `anchor` follow their files to a new list.
  - At start, the file the user was on is selected if the list has it (`Main.qml` `gridLoaded`, `:199`).
  - The changes since the snapshot's `seq` are patched in. This replaces the `edited`/`turned` overlay, which only
    knew this client's edits.
  - The row bookkeeping from `gallery.rs:288-307,492-639`:
    - the `Changed` kinds (`Nothing`, `Items`, `List`, `All`; `gallery.rs:297`);
    - per-row versions that are never reused (`ROLE_VERSION`, `:294`);
    - the kept `spare_rows`.

    `lay_out` (`gallery.rs:558`) returns a list of `GridUpdate`s (`Reset`, `Insert(range)`, `Remove(range)`,
    `Changed(range, Roles{span, version, height})`). The desktop model only turns them into
    `begin/endInsertRows`, `dataChanged` and so on. So the rule "a row gets a new span or a new version, never both" is
    tested in Rust;
- `tree`: the expansion view-model, from `folders.rs` as it is now:
  - `expanded: Vec<bool>` per node;
  - visible rows walked with `subtree_end`;
  - expansion carried to a new tree through `find_key`;
  - `toggle` returning the row range inserted or removed;
  - `reveal` up the `parent` chain;
  - `row_of_path` by binary search over the pre-order rows;
  - the single-child chains opened at first;
  - one kept key buffer for lookups;
- `viewport`: the viewer's fit, zoom about a point, clamp and region rectangle (`Viewer.qml:106-221,355-375`);
- `picks`: picks with an anchor and Shift ranges (`ui/yandex.rs:384-413`: `toggle_pick`, `pick_to`, `picked_in`);
- `input`: typed filter text into bounds, and its validity (out of `filters.rs` and `query.rs`'s string API);
- `keys`: the key map (grid, viewer, panels) as actions over an abstract `Key`, and the help lines made from it;
- `text`: every English string the front end shows:
  - facet labels and `describe`;
  - badge, info and `borders_info` (`gallery.rs:937`);
  - the progress status;
  - write status, crop note (`runner.rs:312`), Yandex `explain` and status lines (`ui/yandex.rs:169`);
  - the rating symbols and tooltips;
  - `group_digits`, `duration`, the `m:ss` time (`control.rs:91,103`);
  - `Side::describe`/`Reason::text` stay in the CLI, which prints them.

**`pg-platform` contents:**

- `verbatim` and `verbatim_wide` (`paths.rs:16-47`), and `WorkList::wide` built on them;
- NTFS directory enumeration and `normalize_path` (renamed from `normalize_root` at f3d2e60, `scan.rs:158`):
  - `Dir`, `FileIdBothDirectoryInfo`, the volume serial (`scan.rs:52-92`);
  - a `Lister` that keeps its UTF-16 path, its aligned record buffer and its entries from one folder to the next
    (`scan.rs:42-51`), with `verbatim_wide`;
  - each record is handed out as its raw UTF-16 name, attributes, file id, size and mtime. Which files to keep
    (`Ext::of_name_utf16`, before any conversion) is the library scan's call, so the scan still allocates nothing per
    skipped entry;
  - the reparse tag from `EaSize`, so the skip-by-tag fix in STORAGE.md needs no new API;
- the reader pool's platform calls: `open` through `CreateFileW` with the sharing flags (`pipeline.rs:192-212`) and
  `set_priority` (`pipeline.rs:76-87`);
- later, from STORAGE.md: the MFT reader and the USN journal reader (volume handle, record fixups, `$FILE_NAME`
  choice), streamed as the same raw records. The elevated `gallery` subcommand that runs them is in `pg-cli`, the
  pipe and its DACL in platform, and the catalog writes stay in the unelevated app through
  `catalog::files::ScanWriter`;
- `fs`: FILETIME stat; write-in-place that keeps the NTFS identity (`copy_into`, `temp_path`, `writeback.rs:396-412`);
  `set_modified` (`keep_time`, `ingest/crop.rs:151`); an atomic replace for small files (prefs);
- `Priority {Normal, BelowNormal, Idle}` (today `THREAD_PRIORITY_*` in `runner.rs`, `thumbs.rs`);
- a kill-on-close job object, a no-window process spawn (`CREATE_NO_WINDOW`, `writeback.rs:39`);
- the process start time (trace), local time (panic log).

The real implementations are behind `cfg(windows)`, with a portable fallback elsewhere (`std::fs` walk, no-op
priority).

**`pg-codec` contents** (`crates/core/src/codecs.rs`, `decode.rs`, `resize.rs`, `animation.rs`, and `crop`'s encoder):

- `Rgb`, `Info`, `MinSize`, `Decoded`, `Stored` (`decode.rs`);
- `Decoder { jpeg: Decompressor, codecs: Codecs }` (`decode.rs:121-124`):
  - `decode_into(bytes, format, exif, min, turns, out, spare)` and `decode_stored_into`: the caller gives the EXIF
    orientation, so the codec reads no metadata;
  - `decode()` and `decode_stored()` (`decode.rs:139-154`) read the metadata themselves. They are deleted; the example
    `examples/decodediff.rs` and the test at `crop.rs:95` call `decode_stored_into` with the orientation they read;
  - `trim` (`decode.rs:132`), `release_over` (`:313`);
- `Codecs` (`codecs.rs:26-42`, private to the crate): PNG (the png crate with its `Scratch`), GIF (`gif` with
  `skip_metadata`), WebP (image-webp), the JPEG fallback (zune-jpeg), TIFF (the tiff crate's `Scratch`, with `image`'s
  limits split), BMP (`image`'s decoder). Every format's pixels go straight into `out` when they are 8-bit RGB(A), and
  otherwise through `raw`; `over_white` flattens alpha. The test `files_decode_as_image_did` checks 72 kinds of file
  against `image`'s own decode, forwards and backwards;
- transforms over model `Orientation`: `orient`, `orient_into`, `orient_pixels`, `transform`, `transform_into`
  (`decode.rs:249-309`);
- `Resizer` (`resize.rs`);
- `encode`: the TIFF/BMP re-encode for crop (`crop.rs:57-73` `reencoded` moves here), and the JPEG encoder
  (`thumbs.rs` `encode_image`);
- animation frames (`animation.rs`, `image`'s GIF, PNG and WebP decoders).

turbojpeg, png, gif, image-webp, tiff, zune-jpeg, zune-core and `image` are confined here (the bans below). Later
features `heif`, `raw`.

**`pg-analysis` contents:**

- `Analyzer` for images and videos (`analyze.rs`, and `video.rs:261-290`'s `analyze`, moved in);
- hashing over any `Read` (one BLAKE3 path for loaded and streamed files);
- the pixel hash, ThumbHash encoding (`analyze.rs` `thumbhash`), the PDQ hasher (`pdqhash` reference test, `pdq.rs:312`);
- the borders detector (`borders.rs`, 1206 lines: one job, kept whole);
- `IMPORT_VERSION`, `BORDERS_VERSION`;
- `examples/stage_times`.

**`pg-catalog` contents:**

- **Internals:**
  - Private `conn` (`catalog.rs:354`), a `Tx` wrapper.
  - **Two opens, as today's two.** Today's `Catalog::open` (`catalog.rs:714`) creates or migrates, rebuilds an earlier
    layout, and syncs the version marks; `Catalog::open_current` (`:735`) only sets settings and SQL functions and
    refuses a missing catalog or a foreign schema version. The plan:
    - `Catalog::prepare(path, Versions{import, borders})` is today's `open`. The library calls it once at startup. It
      writes.
    - `Catalog::open(path)` is today's `open_current`. Every other connection uses it (the reader, the writer, the
      thumbnail pass, the UI).

    The version constants stay with their producers. Catalog stops importing `analyze` and `borders` for them
    (`catalog.rs:21-22`).
  - **Close.** `Catalog::close()` (`catalog.rs:709`) merges the WAL and is kept: the app's shutdown order depends on it
    (see the rules).
  - **Writers as they are now.** Each pass's writer borrows its transaction and prepares its statements once:
    `ScanWriter` (`catalog.rs:1053`), `AnalysisWriter` (`:1131`, with `store_analysis` `:1199`, `store_failure` `:1235`,
    `store_pdq_pair` `:1253`, and the three-way rating sync `sync_file_rating` `:1263`, called from `store_analysis`),
    `BordersWriter` (`:1282`). That's the repository shape:
    - a `Tx` hands them out;
    - the `not_being_written!` guard (`:1037`) stays in their `current` checks.
  - **SQL functions:** `pg_contains` (`query.rs:764-777`, with the `Needle` of `query.rs:788`) is registered on every
    connection; `pg_order_key` only on `rebuild`'s copy (`catalog.rs:450-455`).
  - **Work lists:** `pending_import` and `pending_borders` return `WorkList<WorkItem>` (`catalog.rs:821,855`), built by
    `work_list` (`:595`) with `folder_path` (`:629`) and `max_folder_id` (`:623`). Thumbnails' `pending_in` needs the
    same reads, so these become one `catalog::read::work` API that thumbcache calls instead of its own SQL.
  - Typed errors where callers branch (`Interrupted`, `Busy`, `Layout`; today `interrupted` on `anyhow::Error`,
    `catalog.rs:684`); one error type with source and path context elsewhere.
  - Reads take a `Cancel` token, which generalizes `cancel_when` (`catalog.rs:757`).
  - A second change stamp, `files.changed`, from the same `meta.seq` counter. It is set on every user-visible change
    (file, analysis, rating, border mark, queued turn) by triggers on those tables, and it backs `changes_since(seq)`.
    - Today `files.seq` is the thumbnail source's change number. Its triggers (`catalog.rs:252-315`: `files_added`,
      `files_changed`, `files_source_changed`, `analysis_added`, `analysis_changed`, `analysis_removed`) fire on files
      and analysis only, so ratings, marks and queued turns don't stamp it. The index is `files_seq` (`:117`).
    - Reusing `files.seq` would make every rating send that file back through the thumbnail pass's work list.
    - This is a schema edit in place, version 1 (no schema bumps before release).
- **Modules:**
  - `schema`/`migrate`/`rebuild` (`catalog.rs:78-515` in the baseline, `migrate`, `build_copy`);
  - `files` (`ScanWriter`: `folder`, `upsert_file` returning whether anything changed; `begin_scan`, `mark_missing`,
    `finish_scan`), `roots`, `files_under` (`catalog.rs:887`), `mark_pending` (`:900`, for `import --again`);
  - `analysis` (`AnalysisWriter`, above, with `store_pdq_pair` and the `ThumbHash` it stores);
  - `borders` (`BordersWriter`, marks: `set_border_mark` `catalog.rs:949`, which `browse::set_border_mark` wraps);
  - `writes`: the whole `file_writes` queue:
    - producers (`browse::set_rating` `browse.rs:795`, `queue_rotation` `:838`);
    - the consumer API: `next_ready(settle)`, `record_temp`, `done(job, written, effect)`, `failed(offline)`.
      - `done` takes a typed `WriteEffect`: `Rewritten {before, after, metadata_only}`, `FileRating(u8)`,
        `TurnedBack(turns)`, `KeptInCatalog(turns)`. It applies the effect in the transaction that takes the write off
        the queue.
      - That replaces the closures `writeback.rs:313,338` pass to `done` (`:347`), and the four `Catalog` statics
        (`set_file_rating`, `file_rewritten`, `analysis_turned_back`, `add_catalog_rotation`) become private to
        `writes`.
      - The width/height swap (`writeback.rs:363-366`) is included;
    - `remaining`, `recent_errors` (`cli.rs:266-290`);
    - `MAX_ATTEMPTS`, once: `writeback.rs:36`, with the `attempts < 5` literals (`cli.rs:286`,
      `tests/writeback.rs:74`) and the `?1` binding (`writeback.rs:170-182`) replaced by it;
  - `views`, `session` (typed records; the views' query as JSON inside);
  - `read`, one module per read model:
    - `tree` (the builder and `FOLDER_COUNTS`, `browse.rs:165`);
    - `grid`: `entry_columns!` (`browse.rs:542`), `file_joins!` (`:553`), `list_grid` (`:622`) sized by a count in the
      same transaction, the single-file `entry` (`:594`);
    - `facets` (`BORDER_SUMS`, `query.rs:234`);
    - `details` (`:676`), `border_sides` (`:704`);
    - `changes_since(seq)`;
    - `crop_source` (`catalog.rs:916`, typed `Format`), `status` (`:1013`);
    - **the per-file states readers check against:** `file_state` (`file_state_columns!`, `browse.rs:871`), `file_source`
      (`:905`), `placeholder` (`:932`). They come out of `browse.rs`.

    These two shapes come from DATABASE.md, which asks for paged grids and counts without scans; neither is built in
    this refactor:
    - `grid` takes a range and returns the total beside the entries. The first version reads the whole range, as today,
      so the keyset paging can replace it without changing callers;
    - `facets` and the tree's counts sit behind one function each, so trigger-kept counters can replace their
      `GROUP BY` the same way;
  - `dups`: tier counts, pair histogram, group edges, paths, with the compile-time SQL macros now in
    `ingest/dups.rs:34-62` (`current!`, `groups!`, `grouped!`);
  - `query_sql`:
    - `Condition` with rusqlite `Value`s;
    - the per-enum `sql()` methods and the macros in `query.rs:17-60` (`filetime_unix_offset!`, `bordered!`,
      `border_none!`, `border_class!`, `present_analysis!`, `pair!`) and `duplicates!` (`:645`);
    - `key_sql` (`query.rs:735`), `MediaType::CASE` (`:96`), `BORDER_SUMS` (`:234`);
  - `functions`: `pg_order_key` and `pg_contains` (`query.rs:764`);
  - `pending` (import, borders);
  - `pdq_hashes()` (ids and hash words, for `dedup`). Today `pdq_index()` (`catalog.rs:836-852`) builds the index
    itself (`PdqIndex::build`, `:850`); that moves to dedup;
  - the `thumb_sources` view (new: not in the schema at f3d2e60).
- **The `thumb_sources` view** (new):
  - Created in the baseline schema, over catalog tables only. It is published as the catalog's contract.
  - No ORDER BY, aggregate or DISTINCT in it, so SQLite flattens it and keeps the `files_seq` index (`catalog.rs:117`).
  - It exposes `volume`, `record`, `seq` and the other columns the query needs. The thumbcache's query keeps the
    `entries` join and its ORDER BY (`pending_sql`, `thumbs.rs:739-755`).
  - Plan tests on both sides: `thumbs.rs:1485-1502` (`a_pass_after_a_complete_one_reads_the_changed_files`) moves to
    thumbcache, and catalog gets one against a TEMP stub `entries`.

**`pg-jobs` contents:**

- `pipeline` (`ingest/pipeline.rs`: `Options` `:30`, `run` `:88`, the `Pool`/`Lease` byte budget `:226-352`, `read_whole`
  `:178`): the ordered reader pool within a byte budget, workers and sink, priorities from `Priority`, and a batch hook
  for GPU stages later;
- a batching sink helper (replaces the three copies);
- `Control` (`ingest/control.rs:112-207`) with a typed `Phase` and no formatting (today `begin(phase: &str, detail: String,
  total)`, `:164`); pause waits on a condvar (today `checkpoint` polls, `:152-163`);
- a `Service` skeleton (thread, wake, stop, join), as `Writer` (`writeback.rs:69-157`) and `Runner` (`runner.rs:68-134`)
  have it;
- a `Scheduler` that runs registered passes in order, with preemption and requeue (`runner.rs:203-308`: `run`,
  `requeue`, `round`; `round`'s crop-then-scan-then-import-then-borders-then-thumbs is its first schedule);
- `Reader`: latest-wins slots keyed by (session, kind), with cancellation, over a bounded connection pool (from
  `ui/loader.rs`: `Kind`, `KINDS = 3`, `submit` `:80`, `run` `:122`). Each connection is opened with `Catalog::open` and
  gets `release_memory` after each job. That keeps today's measured trade (`loader.rs:150-158`): 120 MB of page cache
  not held, at ~100 ms per full-library list;
- **two event primitives:**
  - `Watch<T>`, latest value wins: progress, write status, library status;
  - `Bus<E>` with a bounded queue per subscriber. Invalidations carry the catalog's `meta.seq`
    (`FilesChanged{seq}`, `TreeChanged`). A subscriber that falls behind gets `Lagged` and resyncs through
    `changes_since`.

**`pg-render` contents:**

- `Maker` (`thumbs.rs:55-197`: `new`, `decoder`, `read_file` (into the thread's buffer), `make_from_file`, `decode_file`,
  `make`, `make_into`, `load_frame`, `encode_frame`), for thumbnails and video posters;
- `upright` decode, the one place that reads the orientation from metadata and hands it to the codec;
- fit, region and full images with the last-decode cache (`state.rs:327` `LAST_DECODE`), `DECODE_MARGIN` (`state.rs:302`)
  and `big_enough` (`state.rs:392-407`);
- `turn_jpeg` (`state.rs:250`); ThumbHash to RGBA (`state.rs:293-299`); JPEG for HTTP (`encode_image`);
- `pixels`: the output layouts a caller asks for.
  - `Rgb` for encoders, `Rgbx` and `RgbaPremultiplied` (rounded as `qPremultiply`, `state.rs:278-289`) for GPU upload.
  - These are `state::Upload::opaque` (`:268`) and `straight` (`:278`). The desktop asks for the two upload layouts,
    and `providers.cpp` `to_qimage` (`:40-65`) adopts the `Vec` without a copy, as it does now.
  - A layout is a parameter, not a Qt type, so the web server's encoder asks for `Rgb` from the same code;
- the animation `Player` (`ui/animation.rs:133-226`: `Worker`, `play`, `wait_until`): the frame clock, pause and loop,
  behind a `FrameSink` trait that the desktop implements.

**`pg-library` contents:**

- `scan` (`ingest/scan.rs`: the traversal policy, `SKIP_DIRS` `:26`, `ScanStats` `:169` with `changed`, `scan` `:182`),
  over the platform `Lister`. `ScanStats.changed` decides whether the runner counts the scan as done
  (`runner.rs:265-267`, `scans_done`): a scan that found everything as it was sends nothing;
- `import` (`ingest/import.rs`): the pairing through `dedup::Pairer` (the loop at `:49-75`). Files changed while read
  count as `changed` (`ImportStats.changed`, `import.rs:25`), not as failed. `import --again` marks files first
  (`cli.rs:199-202`, `Catalog::mark_pending`);
- the `borders` pass (`ingest/borders.rs`: `find` `:29`, `report` `:127`, `serve` `:198`, `import_review` `:226`);
- the `thumbs` pass (`ingest/thumbs.rs`, `run` `:32`). It checks each made thumbnail against
  `catalog::read::file_state` before keeping it (`:103-114`). A mismatch lowers `first_failed`, so the next pass starts
  from that file's seq;
- the `crop` export (`ingest/crop.rs`: `save` `:36`, `write_copy` `:131`, `keep_time` `:151`; the copies' path policy and
  the refusal inside the library);
- `dups` (`ingest/dups.rs`: `report` `:77`, the union-find `:12-33`);
- `writeback`:
  - the `Writer` as a `Service` (`writeback.rs:69-157`). It keeps one connection across drains and opens a new one after a
    failed drain (`run`, `:130-157`);
  - the queue through `catalog::writes`, with each outcome as a `WriteEffect`;
  - `Offline` as a typed error;
  - `wait_idle()` for the CLI and tests;
- `Catalog::prepare(path, Versions)` once at startup, before anything else opens the catalog;
- events on a `Bus` instead of `on_round_done` (`runner.rs:228`), `on_pixels_changed`, `turned` and `take_messages`;
- later: embeddings (a future `pg-embed` crate holds the GPU/ONNX runtime, banned elsewhere), the dedup batch, quarantine.

The runner's hard-coded round (`runner.rs:240-308`) becomes a schedule of passes.

**`pg-app` contents:**

- **`App`:** `App::open(Config)` replaces the `STATE`, `RUNNER`, `WRITER`, `PREFS` and `SERVICE` globals (`state.rs:42`,
  `library.rs:91-92`, `loader.rs:61`), with `shutdown()`.
  - `Config` resolves every `PG_*` variable, the ExifTool path and the default catalog dir in one place (today
    `cli.rs:18` `DEFAULT_CATALOG_DIR` and `settings.rs`'s `PG_PREFS`).
  - Bounded pools of catalog connections and `Maker`s replace the thread-locals (`state.rs:44-47`). The pool keeps one
    connection open, so the WAL isn't checkpointed away (`state.rs:56-58`).
  - **Shutdown order is kept:** the engine (with the image providers and their threads) ends first, then
    `thumbs.close()`, then the catalog's `close()` (`state.rs:79-88`, and `main.rs`'s `drop(engine)`). The last
    connection to each file deletes its WAL, so the order is what keeps the files whole.
- **`Session`:** keyed by id, one per client; the desktop has one. It holds the client's read slots on the `Reader` and
  its event subscription. List state is the client's (`pg-client::grid`).
- **Images:** `images` resolves an `ImageKey` + `ImageRequest` through the catalog, thumbcache and render (out of
  `ui/state.rs` and `providers.rs`). It keeps today's consistency rules:
  - A thumbnail the cache keeps for the key's `Source` is served from the key alone, turned by the key's turns
    (`state.rs:192-205`).
  - Anything else is made from the catalog's `FileSource`. Its `FileState` is read before and after the file, and the
    read is retried every 20 ms while a write is in progress, up to `WRITE_WAIT` (10 s): today's `consistent`
    (`state.rs:118-139`).
  - The last-decode cache is kept only once the read proved consistent (`state.rs:341-345`).
  - The offline fallback (the best smaller thumbnail) stays as `Found::Offline` has it (`state.rs:150-158`).
- **Edits:** rate and rotate with the write-back rule and the writer wake, border marks, crops.
- **Reads:** grid, tree, facets and details through the session's `Reader` slots.
- **Views and session:**
  - save, apply and delete views (`ui/view.rs`, `browse.rs:953-992`);
  - `restore`, which resolves the stored folder to the tree's spelling or reports it gone (out of `Main.qml:205-237`).
- **Library:** control (`library.rs`: `start` `:157`, `add_folder` `:175`, `remove_folder` `:196`, `rescan` `:208`,
  `toggle_pause` `:220`), plus status as `Watch` and invalidations as `Bus`, replacing `poll()` (`library.rs:226`).
- **Prefs:** today one file, `gallery.json` (`ui/settings.rs`), holds the write-back setting, Real Size, borders shown,
  row height, the Yandex folder, and the session's last file. The split: the server's (write-back, the Yandex folder)
  in `gallery.json`, with one atomic update; the client's (real size, borders shown, row height) belong to each front
  end, and the desktop keeps them in its own file.
- **Yandex:** the search and download jobs, `save_all` (`ui/yandex.rs:272`), `unique_path` (`:253`), `sanitize`
  (`:230`), and the still-image check by `Format`. Its tests (`ui/yandex.rs`, two) run once it's here.
- **No English:** APIs return typed values. Each front end formats them (`pg-client::text` for the desktop).

## Rules across crates

- **Modules split by cohesion, not by length:** a file holds one concern, however long that makes it. The splits listed
  above (catalog by aggregate, metadata by format concern, browse's five jobs, `gallery.rs`, `Main.qml`) are there
  because those files mix unrelated things. A long file with one job (the borders detector, the thumbcache store,
  `codecs.rs`) stays whole.
- **Tests:**
  - Unit tests stay inline.
  - Cross-crate behaviour goes in the owning crate's `tests/`. `ingest/tests/video.rs` splits into `video/tests` (probe,
    frames), `codec/tests` (animations) and `library/tests` (passes). `ingest/tests/crop.rs` and `writeback.rs` move to
    `library/tests`.
  - Tests that cross the new boundaries split with them. The analyze test that builds thumbnails with `thumbs::Maker`
    (`analyze.rs:232-290`) becomes an analysis test and a render test.
  - Tests reach the catalog through its API and testkit, never `conn`. Today the catalog tests (`catalog.rs:1400+`), the
    browse tests (`browse.rs:1013+`, which use `c.conn` for fixtures) and `ingest/tests/writeback.rs` (`c.conn`
    counts at `:74`, `browse::placeholder` at `:169`) read tables directly; each gets the read it needs from the API,
    or a `fixture` feature helper.
  - The scratch-dir helpers (`catalog.rs:1431`, `thumbs.rs:1042`, `ingest/tests/{crop,video,writeback}.rs`) become
    testkit's.
- **Errors:** `thiserror` in libraries, typed variants where a caller branches:
  - `catalog::Interrupted`;
  - `writeback::Offline`;
  - `yandex` (already typed, `core/src/yandex.rs:38`), `exiftool`;
  - the app API, for HTTP statuses.

  `anyhow` only in `cli`, `desktop`, `gallery`.
- **Output:** `deny(clippy::print_stdout, print_stderr)` in every library crate (`core/src/lib.rs:4` already does this for
  core). Diagnostics become events or returned values. Today 34 print calls sit in core and ingest's non-test code
  (`eprintln!` for the import's `tier-1 index holds` and the like).
- **Dependency bans (deny.toml):**
  - Qt → desktop (`deny.toml:45-48` names `pg-ui` today, which becomes `pg-desktop`);
  - ffmpeg-next → video;
  - rusqlite → catalog, thumbcache (today also in `ingest/writeback.rs` and `core/src/{browse,query,thumbs}.rs`, all of
    which move);
  - turbojpeg → codec, render, analysis, edit (today also `analyze.rs`, `rewrite.rs`, `rotate.rs`, `thumbs.rs`; the
    rule holds once they move);
  - the format crates png, gif, image-webp, tiff, zune-jpeg, zune-core → codec, plus png and image-webp → edit (its
    container edits; the decoder stays in codec);
  - `image` → codec, testkit. The tests outside codec that build fixtures with it today take them from testkit:
    - `analyze.rs:255-257` (the RGBA test image),
    - `metadata.rs:1299-1305` (a test's GIF encoder),
    - `rotate.rs:193-194`,
    - `crop.rs:107-142` (the test images and the GIF one),
    - `ingest/tests/crop.rs:38-59`,
    - `ingest/tests/writeback.rs:34-50`.

    Inside codec, `decode.rs:352-354` (a test's PNG), `animation.rs:58-64` and `codecs.rs`'s tests keep it. The
    production `crop.rs:57-73` `reencoded` moves to codec's `encode` first;
  - `pdqhash` (its own image 0.23) → analysis, test-only (`pdq.rs:312`; `deny.toml:10` notes it, and the RUSTSEC ignore
    is its);
  - quick-xml, miniz_oxide → metadata; crc32fast → metadata and edit (the PNG patches);
  - ureq → yandex;
  - windows-sys → platform, desktop, gallery (the ingest and UI crates use it today: `pipeline.rs`, `scan.rs`,
    `runner.rs`, `writeback.rs`, `cli.rs`, `ui/main.rs`, `panic_log.rs`, `trace.rs`);
  - cxx-qt, cxx-qt-lib, cxx → desktop (`deny.toml:45-48`).
- **WASM:** `cargo check -p pg-model -p pg-client --target wasm32-unknown-unknown` joins `just lint` as soon as both exist.
- **Vendored crates:** the root `[patch.crates-io]` (`Cargo.toml`) stays as it is: fast_image_resize, fdeflate, gif,
  image-webp, png, qt-build-utils, quick-xml, rusqlite, thumbhash, tiff, turbojpeg-sys, zune-core, zune-jpeg pinned by git rev to
  the `darktorres` forks (`vendor/README.md`). dhat is a git dependency of the desktop's heap-profile feature (`crates/gallery/Cargo.toml`; `crates/ui` when this was written), not a patch. The
  patches are by crate name, so moving their users between crates changes nothing there. The bans name those crates, so
  the forks' copies are confined the same way.
- **Allocation profile kept:** the earlier allocation work made hot paths allocation-free, and moves keep them so:
  - kept buffers in `ScanWriter`, the `Lister`, `justify_into`, `spare_rows`, the tree's key buffer, `Codecs` (per
    worker, in `Decoder`), `MetaScratch` (in `Analyzer`), the reader `Pool`, the `PathArena`/`WorkList` string arenas;
  - inline results: `ThumbHash` (25 bytes), `Sides` (12);
  - statements prepared once per transaction;
  - `Needle`'s auxdata;
  - the stack-formatted image key;
  - the provider thread's JPEG buffer (`providers.rs:63-66`).

  A new API takes the buffer from its caller where today's code does.
- **Shutdown order** (see `pg-app`): engine, then the thumbnail cache, then the catalog; each connection closes before
  the process ends (commit 9759681). The desktop's `main.rs` keeps `drop(engine)` before `state::shutdown`.

## QML (pg-desktop)

Status: the component split and the typed bridge are done (S-12.2, S-12.3); the viewer's item is the gallery's `viewed*`
properties (S-12.1); the per-component facades are not. `ViewsMenu` is a separate file since S-12.5 (`TopBar.qml` opens it). The line
references below are those of the unsplit `Main.qml` and `Viewer.qml`.

- **Main.qml** splits into `TopBar` (+ `ViewsMenu`), `StatusBar`, `FolderTree`, `GalleryGrid` (+ `GridCell`),
  `LibraryPanel`, `SaveViewDialog`, `ModalOverlay` (shared with Yandex), and `TestHooks`. The start choreography
  (`treeLoaded` `:225`, `gridLoaded` `:199`, `started` `:1045`, and `settling` `:81,185,193,217`) becomes a startup gate
  in the grid and tree facades.
- **FilterBar.qml** (274 lines, new to this plan) stays a component. It makes the `filters::Filters` query and is the only
  QML that calls into it (`ui/src/filters.rs:3`); it moves its calls to the facade.
- **Viewer.qml** (752 lines, 23 `required property` declarations) splits into `ImageLayers`, `MediaView` +
  `VideoTransport`, `Loupe`, `RatingBar`, `Hud`, `RatingPopup`. Zoom math comes from the viewport facade.
- **YandexPanel.qml** splits off `YandexPreview` and `ResultTile`.
- **Kept as they are:** `WheelListView`, `BorderOverlay`, `Ratings.qml` (11 lines): QML-only behaviour.
- **Shared:**
  - a `Theme` singleton for colours;
  - the rating symbols and the help lines read from `pg-client::text` and `keys`;
  - facet values and sort keys as enums from Rust instead of string lists.
- **One flat folder.** Live reload watches a single folder (`live_reload.cpp:68-72`), and `lint-qml.ps1` copies
  `qml/*.qml` (`tools/lint-qml.ps1:28`).
- **Facades, to keep the QML compiled to C++** (crates/README.md: a Rust object's methods only in the file that makes
  it, by id):
  - each component gets its own small QObject facade, made inside it, over the shared `pg_app::App` and a `pg-client`
    view-model;
  - components talk through properties and signals;
  - the controller JS moves into those facades;
  - image URLs come from facade invokables that format an `ImageRequest`.
- **Spike first,** checked against `lint-qml`, for two things:
  - a `CurrentItem` of FINAL properties read through a property, plus a `#[qml_singleton]`. cxx-qt-gen 0.10 generates
    singletons; what the spike checks is that qmlcachegen compiles calls through them;
  - grid cells reading their values as roles of a per-row item model, instead of the
    `rowItem.cellsVersion >= 0 ? galleryModel.f(item)` calls (`Main.qml:675-689`).

  Whatever passes replaces the Viewer's 23 required properties and the per-cell calls. Whatever doesn't stays as it is.
- **The grid row's delegate logic stays in QML,** as the `GridRow` component (`Main.qml:618-665`). That is span/version,
  `cellsFirst`/`cellsCount`/`cellsVersion`, `pooled`/`live`, `grid.settling` and `update()`. It is about ListView's
  delegate pooling, not app state. What it consumes (span, version, the settling start and end around a load or
  relayout) comes from the grid facade.
- **Typed invokables** replace JSON strings and magic integers across the bridge, including the providers' `kind`
  (`providers.rs:59-61`, `KIND_*`) and `Pixels.format` (`providers.cpp`: 1 or 2). `Watch`/`Bus` events are queued onto the
  Qt thread at most once per frame, which replaces the 500 ms and 250 ms polling timers.

## Order of work (each line one commit, or several where marked; every commit passes lint and tests)

Each row is one commit, or one sub-item of a step. A step's own row has the step's name and its status, and its sub-rows
hold the commits. The ID is the step number (`S-nn`, with `.m` for a sub-item). The numbers are the ones the review and
the code comments cite ("step 8"), so they stay. IDs never change and are never reused. The status follows the review
ledger: `open`, `open (part: <sha>)`, `fixed <sha>` (the commit that did it), `verified`.

Now: steps 0 to 3 are fixed, and their verify items are checked (S-02.3, FINDINGS 2026-11-03). Step 4's crates and the wasm32 check are in
(S-04.1 to S-04.8; `specta` came last, in cef96b0). Step 5's eight crates are in (S-05.1 to
S-05.8); S-05.9 is fixed (2295b5a): its deny wrappers followed steps 9 and 11. Step 6's three crates are in: `pg-catalog` (S-06.1,
828dbf0), `pg-thumbcache` (S-06.2, 66f757f) and `pg-render` (S-06.3, 81141cb). The desktop's image functions moved
with S-11, and the animation player with S-06.3. Step 7 is in: S-07.1 to S-07.6 are fixed, with the open items of its first pass closed. Step 8 is in, R-10 included (the catalog's own tests no longer touch `conn`, S-08.6, 713cd7e). Step 9 is fixed: the job layer and its service, schedule and stats (S-09.1 to S-09.3), the crate split (S-09.4, 6c1f007: `pg-library` and `pg-cli`), the job's notices as typed values (S-09.5, b2c7707), the thumbnail cache's writer as a service with its failures as events (S-09.6, 1a8d851), and the cross-crate tests (S-09.7, e5043dd). Step 10 is fixed (`pg-client`, S-10.1 to S-10.8) and step 11 is fixed (`pg-app`, S-11.1 to S-11.7, one commit). Step 12 is partly fixed: the QML split, the typed bridge and the `Watch`/`Bus` queueing are in (S-11.5, S-12.2, S-12.3); the facades and the `CurrentItem` spike are open. Step 13 is open. Steps 14 to 20 are under way: S-19 and S-20 are done, S-14, S-16 and S-18 are partly done, S-15 is written but has not run on a runner, and S-17 is partly done (S-17.0, S-17.1, S-17.2, S-17.4, S-17.6, S-17.7 and S-17.8 are done; S-17.3 and S-17.5 are partly done).

Rules that hold for every row:

- Every commit passes lint and tests (`just lint`, `just test`).
- Before each step: no other change is in progress in the tree.
- From S-02 on, every commit that moves code greps for the old names and paths (`pg-core`, `pg-ingest`, `pg-ui`,
  `crates/core`, `crates/ingest`, `crates/ui`, and the file it moved) outside the dated logs (FINDINGS, HEAP_PROFILE),
  and updates them in the same commit.

| ID | Status | Work | Notes |
|---|---|---|---|
| S-00 | fixed | Baselines. | Rows below. |
| S-00.1 | fixed 71daf88 | Fix `benches/ir/catalog.rs:113`: `thumbs::pending_in`, with `thumbs` not imported at `:12-16`. | The bench moved to `crates/bench` in 77d0210 (S-01.1). |
| S-00.2 | fixed 49d8f3b | Record the gungraun counts, the import throughput on the Pinterest sample, the startup stages, and the test-hook screenshots. | `docs/FINDINGS.md`, "Phase 0 baseline". |
| S-01 | fixed | Scaffolding. | Rows below. |
| S-01.1 | fixed 77d0210 | `members = ["crates/*"]`; `pg-bench` (benches moved, `wsl-bench.sh:38-39` pointed at it). | |
| S-01.2 | fixed 2122a8f | `pg-testkit` with the fixture and scratch dirs. | |
| S-01.3 | fixed 82df5ee | `just wsl-check`: a new recipe, clippy for every crate but desktop in the WSL clone. | Leaves out `pg-ingest` until S-04 (21a5fb1): its Windows-only code is not behind `cfg(windows)` yet. The gap closes in S-04 (the platform crate), so it is deferred to phase 4. Closed by S-04.4 (feddc07): `wsl-check` covers `pg-ingest` again. |
| S-02 | fixed | Spike. | Rows below. |
| S-02.1 | fixed d4c484b | `pg-ui` becomes `pg-desktop` (lib, calls `cxx_qt::init_crate!(pg_desktop)` in `run()`), plus `pg-gallery` (bin `gallery`, winres moved to its build.rs). | The QML module URI stays `pg.gallery`, so `app_icon.cpp`'s `:/qt/qml/pg/gallery/resources/` path holds. |
| S-02.2 | fixed d4c484b | Update every reference. Find them with a grep for `pg-core`, `pg-ingest`, `pg-ui`, `crates/core`, `crates/ingest` and `crates/ui`, excluding `target` and `vendor`, and keep the dated logs (FINDINGS, HEAP_PROFILE). The hits at f3d2e60: the manifests and `Cargo.lock` (regenerated), `.cargo/config.toml:4`, `deny.toml:10,45-48`; `Justfile:12,21`, `tools/lint-qml.ps1:13,19,28,36`, `tools/make_icon.py:4,9`, and under `tools/profiling/`: `profile.ps1:2`, `callgraph/callgraph.cpp:7`, `cgtab.py:195`, `wsl-bench.sh:2,38-39`, `bench_data.py:1`; `docs/PROFILING.md` (19, 58, 117, 124, 137, 155, 168, 180, 359, 373), `docs/PLAN.md:283,307`, `docs/VIDEO_PLAYER.md:55,73`, `docs/STORAGE.md:10,12`, `docs/DATABASE.md:21`, `docs/JPEG_DECODERS.md:3,91`; `crates/README.md`, `crates/core/build.rs:7,20`, and the `include!("pg-ui/cpp/…")` paths (`providers.cpp:22`, `animation.rs:15`, `app_icon.rs:7`, `gpu_device.rs:28`, `gui_app.rs:11`, `live_reload.rs:14,18`, `providers.rs:44`, `screenshot.rs:13`); `eval/imgload.py:41`, and `eval/jpeg-decoders/` (comments, `win-build.cmd:29`). | Checked: no `pg-ui` or `crates/ui` outside the dated logs. |
| S-02.3 | fixed | Verify: `just deploy`, the icon, live reload, `lint-qml`, the screenshot smoke test. | Checked 2026-11-03 and written up in FINDINGS, "Refactor S-02.3: deploy, icon and live reload": `just deploy` exits 0 (two warnings about DirectX and Visual Studio, neither an error), the exe shows the app icon, a live reload changed the text on screen, and `lint-qml` runs in `just lint`. The grid, viewer and filters screenshots match the pre-split build (R-14, 0ac379e), recorded then and not run again. |
| S-03 | fixed | Cut the cycles inside pg-core, one commit each. | Rows below. |
| S-03.1 | fixed e5575a0 | `Format` into `format.rs`. | `metadata` no longer imports it from the decoder. |
| S-03.2 | fixed b5235c8 | `Orientation` into `orientation.rs`: its algebra and both coordinate maps replace the five tables. | |
| S-03.3 | fixed 959236e | `path` (the order keys out of `browse`, `is_under` out of `catalog`) into `path.rs`. | |
| S-03.4 | fixed 6e4c8a8 | The work lists (`paths.rs`) into `path.rs`, with `verbatim` staying put for S-04. | |
| S-03.5 | fixed f96ab21 | Dates into `date.rs`. | |
| S-03.6 | fixed abf8a6b | The literal macros: the minimum side (`min_side!()`), like the group distance. | |
| S-03.7 | fixed a8d4b75 | The typed `Analysis` record (with `ThumbHash` and `Sides`) into `analysis.rs`, apart from the analyzer. | |
| S-03.8 | fixed 74dfde3 | `browse` split by job: the read models with `FileState`/`FileSource`/`Placeholder` and the tree builder into `browse::read`, the commands into `browse::writes`. | |
| S-03.9 | fixed 959236e | The catalog stops calling `browse` (`catalog.rs:455,1089,1104`). | The catalog's test `borders_are_looked_for_after_the_import_and_again_after_a_new_detector` (`catalog.rs:1896`) still calls `browse::read`. Its move belongs to S-08.6 (tests reach the catalog through its API), so it is not moved here. |
| S-03.10 | fixed 220c379 | The container walkers in one module, `container.rs`: `jpeg_is_progressive` is replaced; `Container` stays in metadata. | |
| S-03.11 | fixed bd8c86e | The decoder takes the orientation: `decode()` and `decode_stored()` (`decode.rs:139-154`) deleted. | `upright` into `display.rs`: c531b7c. |
| S-03.12 | fixed a99eebb | Video analysis into analyze (`video.rs:261-290`). | |
| S-03.13 | fixed 6e98cb9 | The catalog hands `pdq_hashes()` to the index (`catalog.rs:836-852`). | |
| S-03.14 | fixed 6ce2a5b | `Maker` apart from `ThumbCache`: `maker.rs`. | |
| S-03.15 | fixed 6898f97 | The `thumb_sources` view, with plan tests. | The thumbnail side of the test moves with S-06.2. `thumb_clock` came in f2c29d1. |
| S-03.16 | fixed 0bc5d6b | `browse::set_border_mark` deleted (`browse.rs:737-739`), and `Detection::depth` (`borders.rs:311`). | |
| S-04 | fixed cef96b0 | Extract `pg-model`, `pg-platform`, `pg-trace` and `pg-container`, then add the wasm32 check to lint. | Phase 4. Rows below. `specta` (S-04.8) is done, so the phase is done too. |
| S-04.1 | fixed a94fdc9 | `pg-container`: the file containers' walkers (JPEG segments, extended XMP, PNG and RIFF chunks, GIF blocks), with no dependencies. | |
| S-04.2 | fixed a18388e | `pg-platform`, part 1: `verbatim` and `verbatim_wide`, and a work list's file path as a platform helper. | |
| S-04.3 | fixed 0831072 | `pg-platform`, part 2: the Win32 calls of ingest and desktop (the directory listing, the reader pool's opens, priorities, the job object, the no-window spawn, file times, the message box, the local and process times). | |
| S-04.4 | fixed feddc07 | `just wsl-check` includes `pg-ingest` again (the S-01.3 exclusion is gone): the platform's fallbacks build it on Linux, and `tests/borders.rs` is `cfg(windows)`. | |
| S-04.5 | fixed e13819d | `pg-trace`: the startup stages, with the app's printing as a sink (`set_sink`, `PG_TRACE=1` in the desktop). | |
| S-04.6 | fixed 3f37fec | `pg-model`, part 1: formats, media kinds, civil dates and the path rules. | |
| S-04.7 | fixed d21f4b0 | `pg-model`, part 2: the EXIF orientation, border detection, the analysis record and the PDQ types; the wasm32 check in `just lint`. The decoder-dependent orientation tests are core's (`decode` tests). | |
| S-04.8 | fixed cef96b0 | `pg-model`, the items the plan lists that are not in the crate yet, each with the step that first needs it: the id newtypes, `Rating`, `Turns`, `FileIdentity` and a typed `Orientation` (S-07); `strum` keys for the enums, which are hand-written today (S-07); `Side::describe` and `Reason::text` out to the CLI (S-09); `pdq_dihedral` returned beside the record instead of stored in it (S-08.3); the query data model with typed editing, `Day`, `Sort`, `FolderTree` and the `Entry` and `Details` DTOs, and `contains_ignoring_case` under `cfg(test)` (S-08.7); `FileState`, `FileSource` and `Placeholder` (S-08.4); `WriteKind` (S-08.1); `ImageKey` and `ImageRequest` (S-11.1); `View` and `Session` (S-11.4); the optional `specta` feature (after S-13). | Done: `FileIdentity` (fcb46e6); `pdq_dihedral` beside the record (e1ce1d2); `FileState`, `FileSource`, `Placeholder` and `WriteKind` (3320363); the query data with typed editing, `Sort`, `Day` and `RatingFilter` (22e7a05); `Entry`, `Borders` and `Details` (4405a3c); `FolderTree` with its builder (bac7bab). The strum keys (S-07.1) and the newtypes (S-07.2) were done before. `contains_ignoring_case` stays a catalog test helper, not model. Done (2295b5a): `ImageKey` (with `Version`), `ImageRequest` and the image `View` (S-11.1), in `pg_model::image`; the saved view as `pg_model::view::SavedView` and the session record as `pg_model::view::Place` (S-11.4). Deviations: the plan's `Session` name is the reader handle's (`pg_app::reader::Session`, which holds the catalog's reader, so it stays in `pg-app`), so the record is `Place`; the record has no last file, which stays its own `session` key (`desktop/src/settings.rs`), as before. `Side::describe` and `Reason::text` went to `pg-cli` in S-09.4 (`describe_side`, `reason_text`). `specta` is done: an optional `specta` feature in `pg-model` (`specta = "=2.0.0-rc.27"`, with its derive). The wire types of `query` (`MediaType`, `Shape`, `Status`, `BorderFacet`, `RatingFilter`, `SortKey`, `Sort`, `Range`, `DateRange`, `Query`) derive `specta::Type` behind the feature, and `Day` (which serializes as text) is `specta(type = String)`. `RatingFilter`'s `try_from`/`into` u8 are read by the derive. Nothing else in the model derives it: the web client needs nothing more yet. Checked: `cargo check` and `cargo clippy -p pg-model --features specta --all-targets -- -D warnings` pass; the feature is not run (no consumer yet). |
| S-05 | fixed 2295b5a | Extract the crates below, one commit each, each adding its deny ban. | Rows below. The eight crates are in; S-05.9's deny wrappers are updated (2295b5a). |
| S-05.1 | fixed 2e62472 | `pg-metadata`: the file metadata (EXIF, XMP, the dates, the orientation and rating patches), with its GIF and APNG fixtures in `pg-testkit`. It stays one module, since its one job is the file metadata; it isn't split by format. The deny ban is `quick-xml` only: `miniz_oxide` and `crc32fast` are also used by `png`, `tiff` and `flate2`, and a wrapper rule can't name third-party parents. | |
| S-05.2 | fixed bcc3707 | `pg-codec`: the decoders (`decode`, `codecs`), the resizer, the animation frames, the pixel buffers, and the crop's re-encode (`encode::cropped`). The BigTIFF fixture is behind the `fixtures` feature. The `image` ban holds with core and ingest as wrappers, until their fixtures move (S-05.9). The JPEG encoder (`encode_image` in `maker.rs`) stays with `Maker` in core until render (S-06.3). | |
| S-05.3 | fixed d6ef3d9 | `pg-video`: the probe, the frames and the file hash, with the FFmpeg DLL build script. | `verbatim` in video is open (S-05.9). |
| S-05.4 | fixed ec4e810 | `pg-edit`: the rotation and the crop, with the rewrites private to the crate. The crop's re-encode is the codec's, and takes the model's `Format`. The tests take their stills from `pg-testkit` and decode with the codec and the metadata reader, not core's display module. | |
| S-05.5 | fixed f4d51e0 | `pg-analysis`: the analyzer (`analyze`), the border detector (`borders`), the PDQ hasher (`pdq`), and `examples/stage_times`. The kept-buffers test drives core's `Maker`, so it moved to core's `maker` tests; the ThumbHash literal test stays here. | One BLAKE3 hashing path is open (S-05.9). |
| S-05.6 | fixed 0e571fb | `pg-dedup`: the PDQ index, the import's pairing (`Pairer`: query, then insert, the best per neighbour), and the duplicates' union-find (`UnionFind`). | The tier-2 verifier, multicut and keeper are the plan's later tiers. |
| S-05.7 | fixed 30325f1 | `pg-yandex`: the client. A download carries the format its bytes sniff as, not the format the response claims; the desktop saves it under `Format::extension`. `YandexError` derives its messages with thiserror. `ureq` is `pg-yandex`'s alone (deny.toml). | The desktop's two Yandex tests move with S-11.6. |
| S-05.8 | fixed 5b2540b | `pg-exiftool`: the `-stay_open` driver. The program comes from the caller: `pg-library`'s `exiftool_program` (`writeback.rs`; in ingest when this landed) reads `PG_EXIFTOOL`. | The per-command timeout and the kill on close are open (S-05.9). |
| S-05.9 | fixed 2295b5a (part: 90e9b75, 6d5a6f1) | The parts of step 5 the moves did not finish: `verbatim` in `pg-video`, one BLAKE3 path, a timeout on each ExifTool command with a kill on close, and the deny wrappers that name `pg-core` and `pg-ingest`. | Done: `verbatim` is made by `pg-video`'s FFmpeg `open` (90e9b75); the file hash is `pg-analysis`'s, `file_hash` for loaded bytes and `file_hash_of` for a streamed file (90e9b75); each ExifTool command has a 120 s deadline, a process that fails or times out is killed, and a closing ExifTool is killed after 5 s (6d5a6f1, a behaviour change). Done: the deny wrappers that named `pg-core` and `pg-ingest` (for `image`, `turbojpeg`, `png`, `gif`, `image-webp`, `tiff`, `zune-jpeg`, `zune-core` and `quick-xml`). They came out as their code and fixtures moved, in steps 9 and 11: this pass removed the `rusqlite` wrapper for `pg-library` (S-09.7 took its SQLite out), and kept `image`'s, since its tests write image fixtures. `cargo deny --offline check bans` is clean. Note: `pg-core` and `pg-ingest` are gone from `deny.toml` (S-09.4, S-10.1); the `image` wrapper names `pg-library` (its tests), and `rusqlite` names `pg-catalog`, `pg-thumbcache` and `pg-bench`; S-13.4 re-checks them. |
| S-06 | fixed | `pg-catalog`, `pg-thumbcache`, `pg-render`. | Rows below. Its three crates are in; the desktop's image functions moved to `pg-app` with S-11, and the animation player to `pg-render` (S-06.3). |
| S-06.1 | fixed 828dbf0 | `pg-catalog` as a mechanical move: `catalog.rs` is the crate root, with `query.rs` and `browse/` as modules, and `conn` still public. The catalog's test fixture is behind the `fixture` feature, and pg-testkit re-exports it (R-11). `display::upright`, a pure rectangle map that the grid uses, is `pg_model::orientation::upright_rect`. `max_folder_id` is public until the thumbnail cache moves (S-06.2). The rusqlite ban is in deny.toml, with pg-core, pg-ingest and pg-bench as transitional wrappers. | The scratch-dir wrappers of R-11 moved to testkit (2122a8f, 5999ff8). |
| S-06.2 | fixed 66f757f | `pg-thumbcache`: the cache's store, `pending_in` and their tests, moved whole (`thumbs.rs`). The Maker test (files read into the maker) is in `maker.rs`, so the cache's tests need no JPEG encoder. Its one catalog dependency is `max_folder_id`, transitional until the catalog's read API carries the work list. | `pending_in` still reads the catalog's tables (S-08.4 moves it to `thumb_sources`). Its errors still go to the `Report` callback; the `Bus` comes with S-09. |
| S-06.3 | fixed 81141cb | `pg-render`: `display` (the upright decodes) and `maker` (the thumbnail `Maker`), moved whole; the decodediff example too. It is a local eval tool and is not in the repository, and it is the reason render has dev-dependencies on `image`, `png` and `xxhash-rust`, and why the `image` and `png` bans name pg-render. `pg-render` depends on `pg-video` (the posters' frames). | Done: the desktop's image functions (fit, region and full images, `turn_jpeg`, the ThumbHash to RGBA, the pixels) moved to `pg-app` with S-11. The animation player's loop (`Player`, its thread and pacing) is `pg-render::player` now, behind a `Sink` trait with the tests that the loop repeats and closes its sink; the desktop keeps the Qt glue in `animation.rs` (the source, the sink and the frame sink's thread-safety). Checked: `cargo test -p pg-render`, clippy, `just deploy`; the viewer's GIF screenshots are the same as HEAD's on the offscreen harness, which shows no animation for either (the offscreen run can't show what the video sink draws, so the animation itself is checked by the tests only). The core keeps `layout` (S-10.1), so `pg-core` is deleted at S-10.1, not at S-09 as the S-09 row says. |
| S-07 | fixed | Newtypes (`FileId`…) and `strum` keys, with no file moves mixed in. | Rows below. |
| S-07.1 | fixed 011f832 | Every persisted enum key is a `strum` derive (`snake_case`; the stored strings are unchanged): the detection, media and format keys in `pg-model`, and the query keys in `pg-catalog`. A test in each crate pins the key list. | |
| S-07.2 | fixed 393a254 (the Linux bench's turns: 2933b88) | `FileId`, `FolderId`, `Turns` and `Rating` in `pg-model` (`ids.rs`, `turns.rs`, `rating.rs`). Every crate's id and quarter-turn parameters use them: SQL binds `.get()`, rows read back into them, and the desktop keeps i64 at the QML boundary. `Rating::new` refuses above 5, and the catalog's `None` is unrated, so 0 stars is no row. | |
| S-07.3 | fixed 6c1a65e | The grid entry's rating is `Option<Rating>` (`None` is unrated), read as `NULL` from the ratings table. | |
| S-07.4 | fixed 9c622cb | The test fixture's file and folder ids are `FileId` and `FolderId`. | |
| S-07.5 | fixed 54660b1 | `RootId`: `roots` has a row id, `Catalog::roots` returns `Root { id, path }`, `remove_root` takes the id, and `root_id` finds it by path for the command line and the desktop. The rebuild fills the id from the old row's rowid, and the layout check counts the key. | Schema edited in place (no version bump, as before the first release). |
| S-07.6 | fixed d961a23, 52b9fa1 | The query's `Orientation` facet is `Shape` (its list is `shapes`); the facet's key `orientations` stays, since the QML uses it. The EXIF orientation is `pg_model`'s `Orientation` (1–8): `new` for a value that must be one, `from_tag` for a file's tag (out of range reads as upright), `get` for the value. Its arithmetic is methods, and the metadata, codec, video, edit, analysis and catalog carry the newtype. | |
| S-08 | fixed | Catalog API, one commit per consumer area. | Rows below. Every row is fixed; the catalog's own tests are ported (R-10, S-08.6). |
| S-08.1 | fixed 3320363 | The writeback queue (`writes`, with `WriteEffect`), and `WriteKind` in pg-model. | The queue is `catalog::writes`: the rows, the producers (`set_rating`, `queue_rotation`) and the consumer steps (`next_ready`, `record_temp`, `done`, `failed`, `drop_gone`, `abandon_temp`). `done` takes a `WriteEffect` and applies it in the transaction that takes the job off the queue: `FileRating`, `TurnedBack`, `KeptInCatalog` and `Nothing` (641a74f). `MAX_ATTEMPTS` is defined once, and `WriteKind` keeps the stored keys (`rating`, `rotation`), so no schema change. |
| S-08.2 | fixed fefda65 | Dups: `catalog::dups` (the compile-time SQL macros, `Identity`, the group and pair queries). The report takes `Catalog::file_path`. | |
| S-08.3 | fixed e1ce1d2, 2069f23 | Scan, import and borders: the writers are handed out by the `Tx`; `import --again` goes through `mark_pending` and `files_under`; the dihedral PDQ hashes are returned beside the record. | Scan, import and borders write through `Tx::{scan_writer, analysis_writer, borders_writer, commit}`, and the border mark is `set_border_mark`. The dihedral hashes are returned as `Analyzed` (e1ce1d2). The Linux bench follows the return type (2069f23). |
| S-08.4 | fixed 3320363 | The thumbs pass's `file_state` check; `pending_in` reading `thumb_sources` (f0b6925); and `FileState`, `FileSource` and `Placeholder` in pg-model. | The readers (`file_state`, `file_source`, `placeholder`) stay in the catalog. |
| S-08.5 | fixed fc8f74e | The writeback queue, the duplicates report and the CLI's status and errors take the catalog, not its connection. | |
| S-08.6 | fixed 1917e9c, 1f54aea, 713cd7e | Benches and tests. | Deviations, recorded. (1) The read API's signatures change in one commit (1917e9c). (2) The cross-crate tests (`crates/ingest/tests`) still reach the connection through `fixture::connection` (fixture feature only); they move with S-09 and are not part of R-10. R-10 is fixed in 713cd7e: the catalog's own tests no longer touch `conn`. Each statement they need is a named helper in the test-only `fixture::raw`, so the SQL is in one place, and the behaviour tests use the API where it reaches the state. Two judgment calls: the layout tests (the rebuild and the migrations) check the file on disk, so they open it with `raw::on_disk`; and the state the API does not expose (`files.seq`, `thumb_clock`, the meta seq, the cut sides, the border marks, the rating tables, the queued value, the analysis width) is read through `raw`. |
| S-08.7 | fixed 22e7a05, 4405a3c, bac7bab | UI: the query data model (typed editing, `RatingFilter`, `Sort`, `Day`), `Entry`, `Borders`, `Details` and `FolderTree` are pg-model's. The catalog keeps the SQL, the JSON and the readers. | The `FolderTree` builder is pure, so it moved with the tree (the plan kept it in the catalog). `contains_ignoring_case` stays a catalog test helper (`cfg(test)`, used only by the catalog's name test), so it isn't in model. Screenshots (bac7bab build, 5789247 release): the grid and viewer match the step-0 baseline exactly; the filters match the previous build exactly, and differ from the baseline in one cell's selection outline (8,665 pixels). That difference isn't attributed. The Nov 2 baseline's `filters.png` differs from HEAD's filter screenshot across the whole frame (about 1.29 million pixels), so it is not a pixel reference, and the 8,665 pixels can't be reproduced from the saved files. The HEAD and new filter-bar captures differ only in the bottom status bar (4,466 pixels: the thumbnail progress). Attributing the outline needs a build at the commit before S-08.7 and its screenshot, which this pass did not run. |
| S-08.8 | fixed 2911b2f | `files.changed` (set by the triggers on files, analyses, ratings, border marks and queued writes) and `changes_since`. | |
| S-08.9 | fixed f2a24cd, 0ad1c56 | `conn` is private. `Catalog::prepare(path, Versions)` makes and migrates the catalog, and the library passes its versions (`pg_library::versions()`; `pg_ingest::versions()` when this landed); `Catalog::open` is the settings-only connection (the old `open_current`). The writeback's reads go through `file_gone`, `file_folder` and `write_target`. Errors: `catalog::Error` with `Interrupted` (the desktop reader matches it), `Value`, `Io` and `Other` (anyhow, with context). `close()` is kept. | Not typed: `Busy` and `Layout`, because no caller branches on them (a layout failure names its path in the message). |
| S-09 | fixed 70ae280, 113baaa, 712cb50, 6c1f007, b2c7707, 1a8d851, e5043dd | `pg-jobs`, then `pg-library` and `pg-cli` together: pipeline, `Control`, the scheduler, the writer service, events, text out of the job layer. | Rows below. `pg-ingest` is gone: it is `pg-library` and `pg-cli` (S-09.4). `pg-core` is deleted at S-10.1, when its `layout` module moves (see S-06.3). R-59 (the job-thread panic) is fixed in 113baaa. |
| S-09.1 | fixed 70ae280 | `pg-jobs`: the pipeline and the control move out of ingest unchanged. | The commit is titled "(S-09)". |
| S-09.2 | fixed 113baaa | The job thread is a `Service`, a round a `Schedule` of the five passes, progress a typed `Phase` (no text) with pause on a condvar and a `Bus` that publishes at most every 5 s. The writer is a `Service` too. A panic in a round is caught and logged, and the thread keeps going (R-59). | |
| S-09.3 | fixed 712cb50 | The passes return their diagnostics as stats (the pending and indexed counts, the cache size, the first failed folders and thumbnails, `thumbnail_error` in the writer's status). The CLI prints them; the passes print nothing. | |
| S-09.4 | fixed 6c1f007 | The crate split: `crates/ingest` becomes `pg-library` (`crates/library`), with `scan`, `import`, `borders`, `thumbs`, `crop`, `dups` and `writeback` as its passes. `pg-cli` (`crates/cli`) takes `cli.rs`, every `println!`/`eprintln!` of the passes' results, the dups printout, the borders tools (`report`, `serve`, `import_review`), `Side::describe` and `Reason::text`, and `wants_cli`, with its test. `pg-ingest` is deleted. The references are rewired: `crates/desktop`, `crates/gallery`, `deny.toml`, `crates/README.md`, `docs/`. | The `image` and `rusqlite` bans still name `pg-ingest` until this row lands, so the wrappers change in the same commit. Checked: clippy, fmt, deny, machete, the wasm32 check and the docs (warnings denied), `cargo test --workspace`, `just wsl-check`, and an offscreen release run. The grid screenshot matches the one from before the split (the same catalog) outside the scrollbar lane (x 1582–1594), which 102b671 added. The viewer and filters start with no QML errors; their pixels were not compared. |
| S-09.5 | fixed b2c7707 | Text out of the pass layer. The runner's English (the warnings at `runner.rs:222-314`, and the crop note at `:369-374`) becomes typed values: a `Notice` enum (unreadable folder, failed scan, failed or panicked round, full thumbnail cache, crop outcome, crop failure), with `is_warning` on the notice itself. The app formats them in `notice_text` (`crates/desktop/src/library.rs`); the wording moved to `pg-client::text` in S-10.8. | Checked: clippy, the runner tests (the newest notices kept, and a crop is a warning only with failures), `just lint` and `just test`. Offscreen release run: the grid screenshot is pixel-identical to the S-09.4 one at a row height of 220 px. The saved prefs say 180 px, so that run used a copy of them with 220 px. |
| S-09.6 | fixed 1a8d851 | `pg-thumbcache` on the job layer: its errors go on a `Bus` instead of the `Report` callback, and its writer is a `Service` over a queue that closes. `Event` (`TotalsFixed`, `TouchesNotWritten`, `NotStored`, `WalNotMerged`) is published on the bus `ThumbCache::open` takes; the command line and the app print it on a thread (the command line joins it after the cache closes). The writer ends by closing the queue, so no caller waits on it. `pg-testkit`'s `Quiet` fails a test on an event it didn't expect. | Deferred from S-06.2 ("the `Bus` comes with S-09"). Deviation: a writer thread that can't be spawned now panics in `Service::start`, where `open` used to return the error. Checked: clippy, the thumbcache tests (25, two new: a closed cache takes no writes and still reads, and a fix at open is published before `open` returns), `just lint`, `just test`, `just wsl-check` on the committed HEAD. Offscreen release run: the grid screenshot is pixel-identical to the S-09.4 one at 220 px, with no QML errors and no cache events. |
| S-09.7 | fixed e5043dd | The cross-crate tests (`crates/ingest/tests` → `library/tests`, and the thumbcache tests) stop reaching `fixture::connection`, which is deleted: each statement they run is a named function in a public, test-only `fixture::state` module. Deviation: the module is `state`, not `fixture::raw`, because `raw` is the catalog's own `cfg(test)` module. `writes_still_tried` counts the attempts below the limit for every write row, where `writes::left_to_write` leaves out a failed write with no temp file; the library tests' wait loop needs the first. `rusqlite` is no longer a `pg-library` dependency (cargo-machete). | Deferred from S-08.6. Checked: `just lint`, `just test` (the ExifTool and ffmpeg tests ran), `just wsl-check` on e5043dd. |
| S-10 | fixed | `pg-client`, one commit per module, each rewiring the desktop to it. | Rows below. S-10.4 to S-10.8 are one commit. |
| S-10.1 | fixed fb4218b | Layout. | `pg-client` is the crate, and `pg-core` is deleted. `layout` is the justified layout, moved unchanged. `Placement` (the row, the span and the neighbour a step away) replaces the three methods `gallery.rs` had. Deviation: the row diff (`RowChange`) and the `Layout` value wait for S-10.3, when the grid's list session takes them. Stale names left outside the dated logs, to S-13.1: the prose in `docs/PROFILING.md` (including its `pg_core::` filter), `docs/JPEG_DECODERS.md`, `docs/STORAGE.md`, the crate map in `crates/README.md`, and the example symbols in `tools/profiling/callgraph/symbolize.py`. Checked: `just lint`, `just test`, and the release build run offscreen: the grid screenshot is pixel-identical to S-09.4. |
| S-10.2 | fixed dc32a23 | Tree. | `Tree::toggle` is the apply step the plan called `apply_toggle`; `plan_toggle` gives the rows first. |
| S-10.3 | fixed db54068 | Grid list session. | The smoke catalog has no files, so the pixel check covers an empty library; a populated run (4000 synthetic files) was checked by eye, not diffed. |
| S-10.4 | fixed (with S-10.5 to S-10.8) | Viewport. | R-06 and R-22: the QML copies of the swap rule (`Viewer.qml`) now go through `viewport.swapsAxes`; their Rust and SQL copies are checked too: they go through `turns_swap` and `swapping_orientations!` (the review rows are closed). The smoke run used the empty catalog, so the viewer's zoom and pan were not exercised by pixels. |
| S-10.5 | fixed (with S-10.4, S-10.6 to S-10.8) | Picks. | |
| S-10.6 | fixed (with S-10.4, S-10.5, S-10.7, S-10.8) | Input. | |
| S-10.7 | fixed (with S-10.4 to S-10.6, S-10.8) | Keys. | The key handlers switch on action codes from `keymap.rs`, with a QML `enum` per file that lists them in the same order. The codes replaced the string actions, which the QML compiler couldn't compile. |
| S-10.8 | fixed (with S-10.4 to S-10.7) | Text. | The QML's own literals (tooltips and the like) were not moved to `text`. |
| S-11 | fixed (one commit, with S-11.1 to S-11.7) | `pg-app`, each sub-step rewiring the desktop in the same commit. | Deviation: the sub-steps went in one commit, not one each (built and checked once, at the end). Checked: `just lint`, `cargo test --workspace`, and the release build run offscreen on a scratch copy of the synthetic catalog: the grid, viewer (`--view=0`), filters, library and views menu screenshots are pixel-identical to the step-0 baseline. Run since, on the disposable fixture, against HEAD's screenshots: `--viewer-keys` (turn, borders, mark and rating: pixel-identical to HEAD where the baseline is current), `--find-similar` on a `.txt` (the error shows), and `--save-crops` (one cropped copy, `00_bordered.png`, 638×438: the accepted cut of 81 px a side). |
| S-11.1 | fixed (with S-11.2 to S-11.7) | (a) `Config`, `App`, pools, `images`, the shutdown order. | `pg-app` depends on pg-codec and pg-video, which the plan's list leaves out (the images need them). `App` leaks the thumbnail cache to a `'static` reference, because `pg-library`'s job and writer take `&'static ThumbCache`. Shutdown: `app::stop`, then the engine, then `app::shutdown` (cache, then the pooled connections, the keeper last). |
| S-11.2 | fixed (with S-11.1, S-11.3 to S-11.7) | (b) `Session` and the `Reader`. | One reader thread serves every session; a session's requests are keyed by kind (`Grid`, `Tree`, `Facets`), and a newer request replaces its kind's waiting one. |
| S-11.3 | fixed (with S-11.1, S-11.2, S-11.4 to S-11.7) | (c) Edits. | `rate` wakes the writer even when write-back is off (the old code didn't); the writer does nothing then. |
| S-11.4 | fixed (with S-11.1 to S-11.3, S-11.5 to S-11.7) | (d) Views and restore. | `FolderModel.restore()` returns the saved folder (merged path) when the tree still has it, else `""`; QML calls it on first load. `save_session` now re-serializes the query and sort through `Place`, so a stored query is canonical JSON, not verbatim. Superseded by S-12.7b: there is no `restore()` now; the start gate in `pg_client::controller::start` picks the folder. |
| S-11.5 | fixed (with S-11.1 to S-11.4, S-11.6, S-11.7) | (e) `Watch`/`Bus` replace the polling. | The job's `Control` gets an `on_change` hook; `pg-library` publishes a `Change` bus (Progress, Notice, Scanned, Imported, Round, Writes, Turned). The desktop pump queues each change to the Qt thread; `poll()` and the QML timer are gone. |
| S-11.6 | fixed (with S-11.1 to S-11.5, S-11.7) | (f) Yandex. | The search and save logic moved to `pg-app::yandex`, print-free. A failed preferences write now reaches the notes bus (`Note::PrefsNotSaved`), not stderr only. |
| S-11.7 | fixed (with S-11.1 to S-11.6) | (g) The prefs split. | Server prefs (`gallery.json`: write-back, Yandex folder) are `pg-app::Prefs`; the desktop's (real size, borders, row height) are `desktop.json` beside it, migrated from `gallery.json` when absent. The gap is closed: the server's load renames a `gallery.json` it can't parse to `gallery.json.bad` before the desktop migration runs, so the migration reads that copy when `gallery.json` is gone (`desktop/src/settings.rs`). A file that is unreadable JSON still gives the defaults, as before. Compiled, and `just lint` and `just test` pass; the fallback itself is not run offscreen (the test hooks don't reach the prefs file). |
| S-12 | partly fixed | QML: the facade spike, then one component per commit, then the typed bridge. | Rows below. Verified: `lint-qml` clean (1050 bindings compiled to C++); fmt, clippy `-D warnings`, `cargo test --workspace` pass. Runtime, offscreen, against a HEAD build on a disposable fixture (12 files copied from the synthetic library, one generated image with an accepted border; the catalog and library restored from pristine copies before every run, so the real library is never written): `--view=0`, turn (`>`), border mark (`B`, `N`), turn with borders (`>BN`) and rating (`r3`) give pixel-identical screenshots to HEAD. Each key changes the picture against `view0`, and the mark is written to `border_marks`. The Yandex panel shows the error for a `.txt` through the change pump, with no timer. Not done: the facades. The Yandex results are a model (S-12.1), checked on the error path only. The offscreen font folder is set by the app (`cpp/gui_app.cpp`). The Nov 2 baseline differs from HEAD by the scrollbar lane (`102b671`) and by background thumbnail progress, so it is not a reference for pixels. |
| S-12.1 | partly fixed | The facade spike: grid cells read their values as roles of the row, not as Rust method calls. | Deviation: the roles are ten per-row lists (`keys`, `badges`, `ratings`, `bordersMarked`, `imageAspects`, `aspects`, `contentX`/`Y`/`Width`/`Height`), not a per-row model of cell records: a list of maps read through `var` fell back to the interpreter (20 `lint-qml` warnings). `GridCell` builds its URLs from the key in QML. The viewer's item: `GalleryModel` has 17 FINAL `viewed*` properties (`viewedKey`, `viewedBorderBox`, …), set by `refresh_viewed()` when the item changes and before each change of `revision`; `Viewer.qml` reads them through `required property GalleryModel gallery` and `readonly property` aliases, so it has 6 required properties (`index`, `count`, `gallery`, `showBorders`, `settings`, `fullUrl`), not 23. This is the design `crates/README.md` states. Rejected: a `#[qml_singleton]` (the item lives in `GalleryRust.grid`, which a singleton can't reach, so Main would still push every value); an object-typed property (no precedent in the repo, and the grid would move behind `Rc<RefCell<_>>`). Per-item invokables used only by the viewer are gone; `rating`, `info`, `borderMarked`, `setBorderMarked` and `fullUrl` stay for the selection and status bar. The filter bar's list facets are models now: one `FacetChoices` per facet (`facet_choices.rs`), whose rows (`key`, `label`, `count`, `chosen`) follow the query and the counts, so the chips read roles and the filter bar's guards are gone. The typed enum tables in `filters.rs` are removed with them (R-82). The Yandex results are a model too: `YandexSearch` is a `QAbstractListModel` whose rows are the results (roles `thumb`, `imageWidth`, `imageHeight`, `picked`), and the panel's two groups are two `Repeater`s over it, each showing its own rows. The group counts (`otherPicked`, `similarPicked`) and the preview's values (`previewUrl`, `previewWidth`, `previewHeight`, `previewPicked`, set from `previewItem`) are properties, so no binding calls a function. The results are not run offscreen: a live search sends the photo to Yandex, so only the error path is checked (pixel-identical to HEAD). `Main.qml`'s two `galleryModel.revision >= 0` bindings are `GalleryModel` properties now: `selectionInfo` (the selected item's info and rating symbol, built in Rust) and `croppable` (the bordered count), read again with each revision by `bump_revision`. `current` is set through `set_current_item`, which reads the selection's text again. No revision or count guard is left in the QML, so R-63 is closed (`REVIEW.md`). |
| S-12.2 | fixed c552170 | One component per commit (the split in the QML section). | Done: `Main.qml` split into `TopBar`, `StatusBar`, `FolderTree`, `GalleryGrid` (with `GridRow`, `GridCell`), `LibraryPanel`, `SaveViewDialog`, `ModalOverlay`, `TestHooks`, `Hud`, `RatingBar`, `RatingPopup`; `Viewer.qml` into `ImageLayers`, `MediaView`, `VideoTransport`, `Loupe`, `BorderOverlay`; `YandexPanel.qml` into `YandexPreview` and `ResultTile`; a `Theme` singleton for colours. Deviations: no per-component QObject facades (components take properties, `Viewer` has 6 of them and the rest come from `gallery`); `ViewsMenu` is not a separate file (S-12.5 extracted it later). The timers all stay, as debounces: `Main.qml`'s `thumbTimer` (300 ms), `sessionTimer` (1000 ms, the save), `layoutTimer` (50 ms, restarted by `onGridResized`), `applyTimer` (300 ms); `ImageLayers.qml`'s `regionTimer` (150 ms). The Yandex panel's 250 ms poll timer is gone (the pump below). |
| S-12.3 | fixed | The typed bridge: typed invokables replace JSON strings and magic integers, including the providers' `kind` and `Pixels.format`; `Watch`/`Bus` events queued onto the Qt thread at most once per frame. | Done: the filter and sort invokables take the facet and sort values as an `i32` index in declaration order, because QML doesn't resolve a qenum as a parameter type (`filters.rs`). Correction (2026-11-06): the code has no `#[qenum]` filter or sort types; `Shape`, `MediaType`, `Status` and `SortKey` are plain Rust enums in `pg_model::query`, and the only `#[qenum]` is `MediaKind` (`gallery.rs:39`, for `viewedKind`). Providers: `ImageKind` and `PixelFormat` enums replace `KIND_*` and the number (`providers.rs`, `providers.cpp`). Done: both desktop pumps (the library's in `library.rs`, the Yandex search's in `yandex.rs`) hand their changes to the Qt thread in passes. At most one pass is queued at a time, so the Qt thread runs one per event-loop turn at most; a pass handles what came in meanwhile, each kind once (the library's rounds are all counted). The Yandex search announces its changes on a `Bus` (`Search::subscribe`). The polling timers are gone (S-11.5). Checked: `--add-folder` on a scratch catalog (the new build and HEAD) gives pixel-identical screenshots, including the status bar. |
| S-12.4 | fixed (spikes) | Spike results before the commits that need them. | S1 to S7, S8, S8b, S9, S9b, S11 and S13 are recorded in the plan. S12 and S13 pass; S14 (Qt-WASM screen) is optional and was not run. |
| S-12.5 | verified (shots: `menu-views`) | `ViewsMenu` extracted from `TopBar`. | `crates/desktop/qml/ViewsMenu.qml`. |
| S-12.6 | verified (tests; shots: `default`, `size120`) | URL grammar in `pg-model` (`ImageRequest`, `ImageKey`), with round trips. | `pg_model::image`. The thumbnail height is clamped there. |
| S-12.7 | verified (tests; `just test`, `just lint`, `just qmltest`, `just shots -Expect apply-view`) | Characterisation tests for the session and the views; the controller's core tests (the plan's "Core tests"). | `crates/app/tests/listing_laws.rs`: the saved listing (folder, recursion, sort, query) reads back through the catalog's session, 12 cases; its scratch folders come from `PG_TEST_DIR`, and the per-case catalogs stay locked until the process exits, so they are not removed. Start order: `crates/client/tests/controller/start.rs`. Replay: `crates/client/tests/replay.rs` replays the recorded session `replay/session.jsonl` (its commands must match the recording), and `crates/client/tests/controller_laws.rs` holds six laws (size clamps and settling, one list per debounce, the start once in any order, a lost folder lists all folders, item keys need a current item, JSON round trip). Deviation: `serde_json` is built with `float_roundtrip` (workspace `Cargo.toml`), since a recorded `Chosen` float otherwise replays to a different value. |
| S-12.7b | verified (start traces; shots: `apply-view`, declared) | The start gate as a sub-machine (`pg_client::controller::start`), and the `--apply-view` fix. | `Shell` (`crates/desktop/src/shell.rs`) runs it; the start traces are in `crates/client/tests/controller/start.rs`. The `--apply-view` screenshot is taken: it differs from dd09f70 as the fix (the view's `folder-00` is shown, where dd09f70 showed the session's `folder-01`). |
| S-12.8 | verified (tests; shots: `filters`, `filters-view0`) | Typed facets (`pg_client::facets`), index order pinned by `crates/client/tests/facets.rs`. | `FacetChoices` takes a facet index at the boundary. Range facets are typed too: `pg_client::input::{Field, Edge}` replace the string pairs `range_facet`, `end_of` and `day_end_of` (removed), and the bar sends indices. |
| S-12.9 | verified (tests; shots) | `Place` renamed `Listing`; `View` holds one. | `pg_model::view::Listing`. |
| S-12.10 | verified (tests; shots: `sort`) | Sort policy in `pg_client::sort` (`sort_choice`, `choice_index`). | Index 0 is ascending by name, the others descending. Traces in `crates/client/tests/sort.rs`. |
| S-12.11 | verified (shots: `query`, `sort`, `apply-view`) | Test hooks' `--query`, `--sort` and `--apply-view` resolved in Rust (`StartOptions`). | `crates/desktop/src/command_line.rs`. |
| S-12.12 | verified (shots: `query`, `filters`, `library`) | Typed listing; JSON leaves the bridge. | `openList(keep)` takes no arguments, `View` and the filter bar are typed. R-81 is closed: counts no longer re-set the query, and the choice model resets only when its rows change. |
| S-12.14 | verified (tests; shots: viewer keys) | Enums and URLs. | `viewedKind` is a qenum, and the viewer's codes are `i32` with a test pinning each. Done: `viewedFullUrl` is a `GalleryModel` property (`fullUrl` is gone); thumbnail and thumbhash URLs are per-row roles (`thumbUrls`, `thumbhashUrls`) made in Rust at the thumbnail height QML supplies (`GalleryModel.thumbHeight`), with cell roles then version emitted on a change. |
| S-12.15 | verified (tests; shots: `apply-view`, `query`, `sort`) | The listing policy (`pg_client::controller::listing`); `openPath`, `applyView` and the sort and recursive policy leave QML. | `Shell` (`crates/desktop/src/shell.rs`, `listing_event`) calls `listing::dispatch` (the `View` object was folded into `Shell` under S-12.20); traces in `crates/client/tests/controller/listing.rs`. |
| S-12.16 | verified (tests; shots: `viewer-turn`, `viewer-one-to-one`) | Grid input (`pg_client::controller::input`): the current item, first and last by count, the viewer's clamp; the neighbour step is resolved by the grid's layout (QML). | Traces in `crates/client/tests/controller/input.rs`. |
| S-12.17 | verified (shots: `viewer-rate`, `viewer-turn`, `viewer-loupe-borders`) | `gridKey` becomes `Shell.keyPressed(key, modifiers)`; the grid's part of the `KeyMap` facade is gone. | QML keeps the `GridAction` switch for rate, turn and border mark (the codes are unchanged). |
| S-12.18 | verified (tests; every matrix shot settles) | Deadlines in the core (`pg_client::controller::deadlines`); one C++ single-shot `QTimer` (`cpp/deadline.cpp`, spike S13's pattern); the shutdown flush; the screenshot waits for no pending pause (`cpp/screenshot.cpp`). | The QML timers `applyTimer`, `sessionTimer`, `layoutTimer` and `thumbTimer` are gone. `Viewer.zenTimer`, `ImageLayers`' region timer, `YandexPreview.centre` and `TestHooks.menuTimer` stay as presentation. Traces in `crates/client/tests/controller/deadlines.rs`. |
| S-12.19 | verified (tests) | One prefs store: `pg_app::store::JsonStore<T>` (load, `.bad` quarantine, temp-and-rename, `serde(default)`), `gallery.json` is the only prefs file. | `desktop.json` is no longer read. The old real size, borders and row height reset once (no compatibility, as the plan says). `lastFile` moves into the prefs. The prefs' golden is in `crates/app`. |
| S-12.20 | verified (tests; shots; the keyed steps by hash) | Panels (`filtersOpen`, `showBorders`), the size (`Prefs`), and the viewer clamp are in the core; `Shell` mirrors them as qproperties. | Done: the viewer's open flag is `Panels.viewer_open` (`Shell.setViewerOpen` from Main.qml; `keyPressed` is false while it is open). `View` is folded into `Shell` (`folder`, `recursive`, `descending` and `savedViews` are qproperties; the listing invokables and `listingChanged` are on `Shell`; `view.rs` is deleted and `listing::start` is in `listing.rs`). Done since: `Select`, `RestoreSelection` and `Step` set the selection on the grid model's lane (`gallery::post`), so a `Step` finds its neighbour where the model is, and queued steps see each other. The QML routes `selectItem` and `stepRequested` and the functions `restoreSelection` and `stepSelection` are gone; `selectRequested` only scrolls (`Main.qml` `scrollTo`). Checked by hash: `--grid-keys=RRRRR` (a test hook sending five Right presses through `shell.keyPressed`) gives a PNG byte-identical to `--select=5` (SHA-256 `6260386c…`). Kept in QML by design: `relayout()` (its scroll anchor is read from the view before the layout changes, so the model's relayout stays after it), and `select()`, which the test hooks and the viewer's close call. The hook is not in `just shots`: its base reference has no entry for it, so it is checked by hash, not by the matrix. |
| S-12.21 | verified (lint: `cargo deny` and `cargo machete`; tests; one `--yandex-fixture` screenshot, inspected) | The boundary (R-96): typed `App` methods (`Session::tree`, `grid`, `facets`, `details`, `entry`; `pg_app::reexport` for the types the window needs), `with_catalog` `pub(crate)`, the desktop manifest drops catalog, library, jobs, thumbcache, yandex and codec. The Yandex `Source` trait in `pg_app::yandex` (`impl Source for Client`, `find_similar`/`save_all` generic, `Search::start` takes a factory), with a recorded replay behind `--yandex-fixture=<dir>` (`upload.json`, `other_sizes.html`, `similar-N.json`, optional `downloads.tsv`). | Deviations: the fixture replays recorded responses, not a fake network client for every path, and it is selected only by the flag. `serde_json` removed from the desktop manifest (no uses). Checked: one screenshot with `gallery --yandex-fixture=<dir> --find-similar=<photo>` (the fixture's answers are the replay tests' samples: one other size at 714×320, similar items at 640×480 and 300×200, a captcha on page 2). The panel shows both groups with those sizes and the captcha's stop message; the thumbnails are blank, since their URLs are remote and nothing is fetched offline. There is no pixel baseline (HEAD has no fixture flag), so this is an inspection, not a comparison, and the Yandex panel is not in `just shots`. `cargo deny` and `cargo machete` pass in `just lint`. |
| S-12.22 | verified (lint: `clippy -D warnings` and `fmt --check`) | Sweep and docs. | The stale comments in the files this pass changed are fixed (`listing.rs`, `shell.rs`, `Main.qml`). The dead-code check covered the imports of the files this pass changed, by grep; `clippy -D warnings` in `just lint` checks the unused imports. `docs/REVIEW.md` rows R-79, R-81, R-96, R-119 and R-120 are `fixed, not verified`. `crates/README.md` has no text on `Listing` to change. A search found no "web client can reuse `pg-client`" claim left in this file or `crates/README.md`. |
| S-13 | open | Docs and the final measurement. | Rows below. |
| S-13.1 | fixed 85bcc65 | `crates/README.md`: crate map, dependency rules, an old → new path table (the dated logs keep their old paths). | Done: the crate map lists the 24 crates in dependency order (checked by script: no crate depends on one listed after it); a Dependencies section gives the rules (`pg-model`, `pg-platform` and `pg-container` depend on no crate; `pg-desktop` depends only on the Qt crate and `pg-app`, which does not depend on it); an old → new path table (`pg-core`, `pg-ingest` and the rest). The stale names left by S-10.1 are fixed: `docs/PROFILING.md`, `docs/JPEG_DECODERS.md`, `docs/STORAGE.md` (every path it names exists), and the examples in `tools/profiling/callgraph/symbolize.py` and `tools/profiling/cgtab.py`. The dated logs keep their old names. Checked: markdownlint on `docs` and the READMEs, and `ruff check` on `tools`, both clean; the full `just lint` runs at the end of the phase. |
| S-13.2 | fixed, no change needed | PLAN §7. | Checked: §7 has no `pg-core` or `pg-ingest` name (the only crate names in PLAN.md are the current ones, and each is a crate in the tree). Its claims about the image provider (`crates/desktop/src/providers.rs`) and the `pg-app` and `pg-client` boundary hold. §7 describes the target, so the duplicate review's UI (not built yet) stays as the milestones track it. |
| S-13.3 | fixed b54ab42 | WEB_CLIENT.md ("layout in WASM" becomes `pg-client` in WASM). | Done: the verdict names `pg-client`'s `layout::justify_into`; the viewer's touch `DragHandler` reference is `Viewer.qml:301` (it had been `:280, :298`, before the viewer split). The Immich and Ente lines describe those projects and stay. Checked: markdownlint clean. |
| S-13.4 | fixed, no change needed | `deny.toml` cleanup. | The wrappers were brought up to date in S-05.9. Done: `cargo deny check` passes (advisories, bans, licenses, sources; the network is up now); every wrapper named in `deny.toml` is a crate in the tree or an external crate that the rule lists. The duplicate-version warnings (`multiple-versions = "warn"`) are upstream pins, not ours: `unicode-width`, `codespan-reporting` (cxx-qt-build's 0.11 against cxx-gen's 0.13), `hashbrown`, `foldhash`, `miniz_oxide`, `syn` and `thiserror`. None is fixable in our manifests without an upstream release, so they stay as warnings. |
| S-13.5 | partly fixed (the import and the startup measured; the first start of an S-00 catalog is 12 s slower, a one-time migration) | The final measurement, against the S-00 baselines: gungraun counts within ±1%, import throughput and startup stages unchanged, `gallery` built without the `desktop` feature. | Done: `cargo check -p pg-gallery --no-default-features` passes. The gungraun counts are run at HEAD (S-16.1: `docs/perf/baseline.json`), not yet compared with the S-00 files. Measured 2026-11-04 against S-00 built and run the same day (FINDINGS, "Refactor S-13.5"): the Pinterest import rate is within about 2% (447–450 files/s for HEAD, 447–460 for S-00); the startup stages to `ready` are 807 ms median for HEAD (steady state) against 869 ms for S-00. The first start of HEAD on a catalog written by S-00 takes about 12 s more (`init_done`): HEAD migrates the catalog in place, once. Not compared: `round_done` (did not print within 15 s of `ready`). Still open: the choice whether that in-place migration stays. R-20 and R-66 are measured here. Both are deferred for now. |
| S-14 | partly fixed | Verification: the screenshot matrix, the desktop tests, the QML tests. | S-14.2: `just test` passes; the two dead tests run, and a skipped test fails the job. S-14.3: `just qmltest` runs 19 tests (`GridRow`, `WheelListView`, `Viewer`), all passing, with `lint-qml` clean. The wheel tests drive `WheelListView.wheelBy()`: under the test harness `mouseWheel` reaches a `MouseArea` but not a `WheelHandler` (checked, and a plain `ListView` does not scroll on it), so the wheel routing is not covered by a test. The tests import the `pg.gallery` module that `tools/qmltest.ps1` stages from the real QML and the stubs, so the `Ratings` singleton resolves and the warning is gone. S-14.1: `just shots` compares the working tree with a recorded reference (`tools/shots/reference/dd09f70e0a`, 21 scenarios, committed in c0c8fd9; the reference directory is now `tools/shots/reference/8f980e17cf`, per S-19.2). On the current tree 20 scenarios are identical, and `apply-view` differs as declared (the fix: the view's folder shows). |
| S-15 | written, run locally (not on a runner) | CI: `.github/workflows/ci.yml`, with `windows` and `linux` jobs. | Every command is one the Justfile or `tools/` has, so a failure reproduces locally. Not run on a runner: it needs the `FFMPEG_URL` repository variable, and the Qt install at `C:\Qt`. Branch protection is a repository setting, not changed here. Skip policy: the CI workflow sets `PG_REQUIRE_TOOLS=1`, so a test whose tool (ExifTool, ffmpeg) is missing fails instead of skipping (`pg_testkit::missing_tool`). Checked locally on the uncommitted tree: `just lint` and `just test` pass (no failures, case-sensitive; ExifTool and ffmpeg tests ran); `just deploy` builds and runs windeployqt; the release `gallery.exe`, started offscreen, reaches `pg_stage_ready` (6.5 s on the first start of this build, a one-time cost; 816 ms on the second). The `linux` job's commands pass in WSL Ubuntu 26.04: `clippy -D warnings` and `cargo test --workspace --exclude pg-desktop --exclude pg-gallery`, 0 failures. Off Windows, no test is `ignore`d for a Linux gap any more (S-17 Unit F, 2026-11-07): the five library tests (`writeback` ×2, `crop`, `crop_roots`, `video`) and `thumbcache::a_cache_of_another_catalog_starts_over` run on Linux, and `pipeline::files_open_as_std_opens_them` runs everywhere. The two `ignore`s left are real-catalog tests that need `PG_TEST_CATALOG` (`crates/catalog/src/browse/read/tests.rs:351,500`), skipped on every platform by design. The Linux `cargo test` runs the whole workspace (`ci.yml:270-271`, `--workspace --locked`): the `--exclude` of the two Qt crates was WSL's line, and went in Unit F. The `linux` job now installs `ffmpeg` (CI and `tools/profiling/wsl-setup.sh`), which the video tests run. Not run: any push. |
| S-16 | partly fixed | Performance. | S-16.1 counts: `docs/perf/baseline.json` holds 11 callgrind results (the row was written when it held 13; the file in the tree now has 11), measured at HEAD a22fbad by `just perf --update`. `just perf` at the same commit reports every result within ±1%. S-16.1 against S-00: S-00 re-measured today (`49d8f3b`, same data and toolchain) gives five of eleven within ±1%, and six (the `list` benches and `tree.all`) 1.4–2.6% lower at HEAD; the Nov 2 numbers do not reproduce on this setup. See FINDINGS, "Refactor S-16.1". S-16.3 done, not adopted: thin LTO and one codegen unit move the hot benches by at most 0.7% (`tree.all` −4% and +1%); FINDINGS, "Refactor S-16.3". The `#[inline]` audit is closed with no action: the counts do not point at any cross-crate call, so nothing in it is measured as hot. Not done: S-16.4 (checkpoints after S-12, after S-17/S-20, at the end). The benches are HEAD's; the comparer and the baseline are the checkout's (`tools/perf-check.sh`). Phase 3 checkpoint (S-16.4), not run: `just perf` measures the committed HEAD (`tools/perf-check.sh`), and nothing is committed in Phase 3. Run it after the commit, then decide the S-17.2 re-baseline (`--update`). Since then (2026-11-07): the baseline was re-measured at `ae4a09d` on the v3 catalog data; S-17 Unit A moved the bench layout to v4 (`68f8864`), and `just perf` has not been run on HEAD since, so the S-17 changes' instruction counts are not checked against the baseline. Open. 2026-11-07: `just perf` failed before it measured: the bench data (`v4-windows-rules`) was made by an older schema (S-17 Units A and D made `views.folder` and `file_writes.temp` BLOBs). 2026-11-08, fixed and re-baselined: `LAYOUT` is `v5-windows-rules` (`crates/bench/benches/ir/catalog.rs`); its data is `bench_data.py` over a catalog the current `gallery.exe` built from the synthetic library (4,000 files, thumbnails at 256 px, the bench's `THUMB_HEIGHT`; an earlier try at 220 px made `pending_thumbs` 16 times larger, which was the data, not the code). The baseline was re-measured in WSL on the working tree. Nine of eleven benches are within ±1% of the old baseline. Two moved outside it: `list.folder_flat` +1.23% and `tree.all` +2.26%. They are not separable here between S-17's folder-tree code and the rebuilt data, since the old data can't run on this schema. Not yet verified with `just perf` on HEAD: the layout bump and the new baseline are uncommitted, and `just perf` measures the committed HEAD. |
| S-17.0 | done as a review aid | Reference keys for the current `merged_path`, `folder_key`, `name_key` and `order_key`. | `crates/model/tests/reference_keys.tsv`: 2034 rows (the catalog's literals, astral and lone-surrogate names, seeded random names). The rows are for the S-17.2 diff; no test pins them yet. |
| S-18 | partly fixed | Fuzzing and property tests. | S-18.1: `fuzz/` has 7 targets (containers, metadata, edit, model keys and query JSON, codec transform). Under nightly with the sanitizers, in WSL, each target built and ran about 1M inputs in 15 s with no crash. Building needed `libfuzzer-sys = "0.4"` and the workspace's vendored patches and lock, which `fuzz/` now has. S-18.2: `pg-model` has 10 laws (turns, orientation, image key); `pg-client` has 3 for the justified layout (rows cover the items, rows fit and closed rows fill the width, aspects are clamped). Also written: path keys against a reference comparator in `pg-model` (`path_laws`, 4); `pg-container` `find` and `range` (`search_laws`, 2); `pg-client` `Tree` reveal and rows (`tree_laws`, 1) and `Grid` rows, coverage and order (`grid_laws`, 1); the saved listing reads back in `pg-app` (`listing_laws`, 1). The `Place` (now `Listing`) round trip is covered through the saved file by `listing_laws`; a pure round trip without the catalog is not written, and is accepted as not needed: its encoding is inline in `App::listing`/`save_listing`, and extracting it for one test is not worth the change. S-18.3: `tools/fuzz-smoke.sh` runs each target for N seconds; it clones HEAD, so it tests committed code only. |
| S-17.1 | done; used by the catalog (S-17.3), platform (`pg-platform::names`) and scan | `Name` (`pg-model::name`): the exact OS name as canonical bytes (WTF-8 for an unpaired surrogate on Windows; raw bytes on Unix); `from_wide`, `from_raw`, `from_wtf8` (strict), `units`, `to_wide`, `display()` (lossy, UI only), `push_name_key`, serde (a string when valid UTF-8, else `{"b": hex}`). | `crates/model/src/name.rs` (unit tests) and `crates/model/tests/name_laws.rs` (proptests: the wide round trip, equal names have equal bytes and the reverse, injective name keys, the case-insensitive order then exact, any byte sequence decodes). |
| S-17.2 | done; used by the catalog (S-17.3) and the library (S-17.6) | `PathRules { Windows, Posix }` (a closed enum): `parse`, `parse_bytes`, `display`, `push_folder_key`/`folder_key`, `order_key`, `scope_keys`, `is_under` (anchors and parts), `split_file_name`. `Path` is an anchor (`D:`, the UNC root `\`, or empty) and `Name` parts. | `crates/model/src/rules.rs`. Windows keys diffed against `reference_keys.tsv` (`crates/model/tests/rules_reference.rs`): all 1018 name rows match exactly; the folder keys match exactly for every component list without Σ; **83 folder keys differ, and each differs only by the final-Σ unification**, which is the intended change. |
| S-17.3 | partly fixed | Catalog takes `PathRules`: folder and file names are BLOBs; the separator is no longer in SQL; `under_root!` binds the subtree bounds; `file_by_path` looks up by folder key and name key; `meta.path_rules` is checked on open and a catalog with another value, or none, is refused with a rebuild message (`catalog/src/lib.rs`, `check_rules`). | Done in S-17 Units A and B (2026-11-07). `views.folder` is a BLOB (the rules' bytes); a catalog of the TEXT layout has its saved views and session carried over by `carry_rows` (`crates/catalog/src/schema.rs`). `Details.path` and the browse state are `rules::Path`. The catalog's tests run over both rule sets where the rules allow (`crates/catalog/src/lib.rs`, `browse`, `writes.rs`, `dups.rs`, `fixture/raw.rs`); the reference-key and drive tests stay Windows-only, with the reason stated. The final-Σ bug is fixed: a folder `ΑΣ` lists its files, because `scope_keys` uses the rule the writer stores (test `browse/read/tests.rs:451`, Windows). The browse display strings are still `String`, for display only (lossy by design). Catalogs are rebuilt, with their rows carried. |
| S-17.4 | done | `FolderTree` anchors: `anchors: u64` over the anchor names sorted by text; bit 63 is shared past 63 anchors; `anchors()` and `multi_anchor()` replace `drives()` and `merged()`. | `crates/model/src/tree.rs`, test `a_folder_shows_each_anchor_it_is_on_in_text_order`; the catalog browse tests and `desktop/folders.rs` use the new methods (QML role names unchanged). The desktop side is not run (Qt). |
| S-17.5 | partly fixed | `pg-platform::native` has `NativePath`, `set_os_str` and `utf8_text`; `Name` holds each listed name exactly; the scanner keeps a lone surrogate (test `a_listed_name_is_kept_exactly`, which replaces the lossy test at `scan.rs:189-208`); the directory lister and file open take native units. `pg-scan::normalize_path` returns the exact `Path` (test `a_root_whose_real_name_is_not_unicode_keeps_it`, Windows and Linux); the library's scan queue and the app's `add_folder`, `rescan` and `scan` carry typed roots, not text. `crop::save` takes its folder exactly (`names::rule_path_of`): a folder that is not a path is an error, not copies written into a different folder. `ATTACH DATABASE` in the catalog and the thumbnail cache refuses a path with no UTF-8 form instead of attaching a lossy name. | Done in S-17 Units B to E (2026-11-07): FFmpeg reads each file through a Rust `File` (an AVIO over it, `crates/video/src/ffi.rs`); ExifTool takes exact names (the argument file, or one process with the name as its argument, exact on Unix; a name ending in whitespace is refused, since ExifTool takes the type from it); writeback's temp is a `rules::Path`; the animated player and the viewer take file ids; the Yandex folder is `Prefs.yandex_folder: Option<RulePath>`; the library, the folder pickers and the save-crops folder take typed paths. `fill_file` was not added as named: the lister and file open take `NativePath` units. `native::utf8_text` and `names::text_of` are gone. Documented limits, all at a text boundary: `desktop/src/command_line.rs` option values other than paths (lossy UTF-8 on Windows, as read; the paths are exact through `path()`); Qt's own argv on Windows (probed exact; the app reads its paths from `args_os`); display strings (S-17.3). |
| S-17.6 | done | Library scan, crop, writeback, import, thumbs, borders and runner take `PathRules` and `NativePath`; the skipped folders are `skip_dirs(rules)`, which is data (the NTFS system folders on Windows, none on Posix). | The library tests pass on Windows and Linux (S-17 Unit F ran the five that were ignored on Linux), each with its platform's rules (`PathRules::NATIVE`). |
| S-17.7 | done; the wall-clock timing is in `docs/PROFILING.md` (not gated) | A file's identity is `FileIdentity { volume: u64, file_index: u64 }` on both platforms: the 64-bit `FileIdInfo` serial on Windows, the whole device number, per entry, on Unix (`crates/model/src/ids.rs`, `crates/platform/src/dir.rs`). `files.volume` and `files.file_ref` are nullable: a displaced row keeps its id, folder, name, ratings and content, and loses its identity. `files.created` (FILETIME ticks, NULL when unknown) and `files.unpaired` are new. A catalog of an earlier layout is moved aside and a new one is made (first written as `*.pre-s17-7` with `LAYOUT_PARTS` 26; both superseded, as below). Superseded: the move-aside is `move_aside` (`crates/catalog/src/lib.rs:313-326`, to `catalog.sqlite.replaced`), and there is no layout number (the schema check is in `crates/schema`, see docs/PLAN.md, M2). The pairing at the end of each scan (`crates/catalog/src/pair.rs`) rebinds the same file on another volume (same folder, name and identity), a file moved while the app was closed (same identity, size, mtime and extension, on a complete scan only), and a save over a name (a changed or unknown creation time, on a complete scan only). A replacement resets the file's own content and rating, keeps its queued rating writes without their temp files, and deletes its thumbnails. A root on a non-NTFS Windows volume, or on an NFS, CIFS or SMB mount on Linux, is refused when added and when scanned (`pg_platform::dir::check_root`). Reparse points are indexed without being followed; a folder already on the walk's path (a bind-mount cycle) is not descended; the same file reached by a second path is skipped and counted. | Step 8, the wall-clock timing, is done (S-17 Unit G, 2026-11-07, `docs/PROFILING.md`, Measurements): a 10,000-file library in WSL, first scan 1.6 s warm and 4.8 s with the caches dropped, a rescan after renumbering pairs every row (none missing); not gated, and not a callgrind result. Not run: the Windows timing, and the `wsl --shutdown` run (the cold VM). Deviations from the plan, all in `docs/S17_7_PLAN.md`'s terms: a fresh row with no candidate in a tier is not ambiguous (the plan counted it, which would flag every new file); a new reparse file gets no row, since the flag that would block the import is not in this step; a refused folder is not added (the add fails with the reason), while a root refused later stays and is reported on each scan; the Z: drive test is a fake listing, because the real drive is not on CI. The Linux tests run in CI on each commit. |
| S-17.8 | done, on a copy of the working tree | `just wsl-check` runs clippy and then `cargo test --workspace` on HEAD (`tools/wsl-check.sh`; the two Qt crates' `--exclude` went in S-17 Unit F, with Qt 6.12 installed in WSL). The Linux CI job already runs both. Headless build and CLI smoke test, run in WSL (Ubuntu-26.04) on a copy of the working tree (`git ls-files` into `~/pg-wt`): `cargo clippy --workspace --exclude pg-desktop --exclude pg-gallery --all-targets -- -D warnings` and the same `cargo test` pass; `cargo build -p pg-gallery --no-default-features` builds. Fixed on the way: `crates/gallery/src/main.rs` `attach_console` called Windows console APIs on every platform, so the headless build failed to link on Linux; it is now Windows-only. CLI smoke: `gallery scan`, `import` and `status` on a 24-file synthetic tree: 24 imported, 0 failed. Then a file and a folder whose names hold a byte no UTF-8 sequence has (`\xff`) were added: 26 files in 4 folders, 2 more imported, 0 failed. | Done in S-17 Unit F: `just wsl-check-tree` (`tools/wsl-check.sh --working-tree`) checks the working tree, copied into `~/pg-wt`; `just wsl-check` still checks HEAD, on purpose. |
| S-19.1 | done | Text seam: every `pg-client::text` line is a typed `text::Message`, worded by the Fluent entry of the same id in `crates/client/locales/en/text.ftl`; plurals are Fluent selects (`$count` chooses, `$text` shows). The public functions keep their signatures; the Yandex constants became messages. | `crates/client/src/text.rs`. Tests: the variants and the file's entries name the same ids; every message renders; the file parses; the filter line and the borders text are pinned. Verified: `cargo test -p pg-client`, wasm32 check of `pg-client`. The QML literals are S-19.2 and the pseudo-locale is S-19.3, both done. |
| S-19.2 | done | The QML literals S-10.8 left behind move behind the `Words` facade: `words.text(id)` and `words.phrase(id, count, value, second)`. | The user-visible literals of the 28 QML files go through the facade, each with an entry in `crates/client/locales/en/ui.ftl` (103 `ui-` ids). Not words, so not moved: colours, numbers and symbols (`%`, `×`, the `(n)` counts, `✓` in `ui-picked`), comments and property names. A second scan after the first pass found two more (`Back` in the Yandex preview, the grid's `file ≈ N of M`), now moved. The test `the_qml_words_and_the_ui_entries_are_the_same_set` (`crates/desktop/src/words.rs`) fails on a `words.` id missing from `ui.ftl`, or an entry no QML uses, in either direction. Verified: `just lint-qml` (1113 bindings and functions compiled to C++, none interpreted: `String(...)` calls became `"" + x`, which the compiler handles), `just qmltest` 19 of 19, `just shots` 21 of 21 same as `8f980e1`. Not in the matrix: the Yandex results panel (never pixel-checked). |
| S-19.3 | done | Only `en` ships; a pseudo-locale runs in the screenshot matrix. | `text::set_pseudo` (`crates/client/src/text.rs`): letters accented, each line bracketed and padded by a third; `--locale=pseudo` switches it on (`crates/desktop/src/lib.rs`). `tools/compare-shots.ps1` has a `pseudo-locale` scenario: it runs, its image is kept in `target\shots\pseudo\pseudo-locale.png`, and it is not compared (`SHOWN`). Checked by eye: the top bar's labels are not clipped. Test: `the_pseudo_locale_accents_letters_and_keeps_digits`. |
| S-20.1 | done | Measured each library pass's real `use` list (not the grep of names): scan: catalog, jobs, model, platform. import: analysis, catalog, dedup, jobs, model, platform. thumbs: catalog, jobs, model, platform, render, thumbcache. writeback: analysis, catalog, edit, exiftool, jobs, metadata, model, platform, thumbcache. crop: catalog, edit, exiftool, jobs, model, platform. borders: analysis, catalog, jobs. runner: all of them. | `use` lines in `crates/library/src/*.rs` at the time of the split. |
| S-20.2 | done | `pg-query` split out of `pg-catalog`. The audit: the compiler (`condition`, `key_sql`, the JSON) builds SQL and parameters and reads no catalog state; only `register_functions` takes a connection, and it is given one. | `crates/query` (package `pg-query`); `pg_catalog::query` is a re-export, so callers keep their paths. Catalog's `serde` dependencies dropped (no longer used). `deny.toml`: `rusqlite` wrappers name `pg-query`. |
| S-20.3 | done, one deviation | Four pass crates: `pg-scan`, `pg-import`, `pg-thumbs`, `pg-writeback`. `Failure`, `keep_failure` and `Change` moved to `pg-jobs`; the name helpers (`native.rs`) to `pg-platform::names` (which now depends on `pg-model`). `pg-library` keeps the runner, crop, borders and dups, and no longer re-exports the four: callers (`pg-app`, `pg-cli`, the library tests) name the real crates. `Change` and `Failure` are imported from `pg-jobs`. `crates/README.md` updated. | Deviation: by the S-20.1 rule, `crop` and `borders` also have strict-subset dependency sets; they stay in `pg-library` (the plan's expected list). Verified: `cargo check` and `cargo clippy -D warnings` (workspace and Qt crates), 368 workspace tests pass, `cargo deny check`, `cargo machete crates`, markdownlint. The S-16.4 checkpoint is not run. |
| S-21 | open | The remaining REVIEW rows: R-75, R-76, R-77, R-78, R-80, R-83, R-101, R-112, R-113, R-127, R-66, each with a measured before and after. | Phase 4. Includes the final S-16 run and the plan's "Now" paragraph. |

## Verification

- **Every commit:** `just lint` and `just test`.
  - Lint covers rustfmt, clippy `-D warnings`, cargo-deny with the bans so far, cargo-machete, ruff, markdownlint,
    qmllint/compiled QML, and the wasm32 check from step 4 on.
  - The ExifTool and ffmpeg integration tests (`writeback`, `crop`, `video`) must print that they ran, not SKIPPED.
  - `pg-client`'s tests run from step 10. They cover the logic that today only the screenshots check.
  - The codec's `files_decode_as_image_did` test (72 files against `image`'s own decode) runs unchanged in `pg-codec`.
- **Commits that touch `platform`, benches or any crate below `library`:** `just wsl-check`.
- **Commits that touch desktop, client, app or QML:** run the release build offscreen (`QT_QPA_PLATFORM=offscreen`,
  `QT_QPA_FONTDIR`) on a scratch library (`PG_CATALOG_DIR`, `PG_PREFS`) with the test hooks:
  - `--screenshot` of the grid, the viewer (`--view`), `--filters=1`, `--menu=views`, `--library=1`;
  - `--viewer-keys` for rate, turn, borders, zoom;
  - `--save-crops`, and `--find-similar` on a `.txt` (the error path).

  Compare the screenshots with the ones from step 0.
- **Once at the end**, against the step-0 baselines:
  - gungraun counts within ±1% (no LTO in release, so calls that cross the new crate boundaries may inline
    differently; anything over that tolerance gets investigated);
  - import throughput and startup stages unchanged;
  - `gallery` built without the `desktop` feature.
