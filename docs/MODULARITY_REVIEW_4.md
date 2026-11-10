# Modularity review: responsibilities

The fourth review of [MODULARITY_PLAN.md](MODULARITY_PLAN.md) (2026-11-10, at `3b94a5b`, read through `0ed0fbe`). The plan's steps M-0 to M-20 removed the cycles and enforce the graph's shape. This review asks a question those checks cannot answer: where each piece of work lives. Its findings are the steps M-21 to M-49 below, all open. The ground rules and the gates are the plan's.

## What was checked

The enforced checks all pass at `3b94a5b`:

- no module cycle on either target;
- the crate layers as `tools/crate-layers.txt` lists them;
- no cycle with dev edges;
- the re-export and platform rules;
- `unreachable_pub`.

They check the shape of the graph, not whether work is done twice, sits in the wrong crate, or reaches another part through a global.

**By script, over every `.rs` file under `crates/`.** The scripts are in the session scratchpad (`review/`); M-48 moves them to `tools/modgraph/`.

1. **Public surface** (`api_surface.py`). Every `pub` item of every library crate, from `cargo modules structure` (1,157 items in 31 crates), with where it is named: other crates' production code, tests, or nowhere. The match is on the item's name. So an item reached only by type inference (a service returned by an accessor), or only through `pg_app::reexport`, looks unused. Every "no use" was checked by a grep of the whole tree, QML and C++ included.
2. **Dependency edges** (`edges.py`). For each internal normal edge, the names the dependent's production code takes through it, and in how many files. An edge that carries one or two names is where code tends to sit in the wrong crate.
3. **Duplicates** (`dupes.py`). Every window of 6 and of 4 normalized code lines, hashed across all files, tests included. The production groups that span crates were read.
4. **Sizes** (`sizes.py`). Production lines per file, and functions over 80 lines.
5. **Process-wide state.** Every `static` with interior mutability, `OnceLock`, `LazyLock` and `thread_local!` in production code.

**By reading, every production file of every crate.** That is the code before the first line-start `#[cfg(test)]`, read bottom-up: from the leaves (`container`, `model`, `schema`, `testkit`) to the front ends (`cli`, `gallery`, `desktop`). Tests were skimmed.

The following were outlined instead: signatures and doc comments read, bodies only where a finding needed them.

- `client/src/text/message.rs`: one enum and its three matches.
- The codec's format modules and `dedup/src/pdq_index.rs`.
- The metadata modules below `lib.rs`.
- The platform backends: `dir/{windows,unix}.rs`, `file`, `names`, `native`, `picker`, `process`.
- The catalog's `fixture`.
- `video/src/ffi.rs` and `analysis/src/pdq.rs`.
- The desktop's 26 bridge files and 18 state files. Checked: they have no function bodies except `Default` and the gallery state's helpers.
- `bench/benches/ir/catalog.rs`.

All 33 crates under `crates/` were read: `container`, `model`, `schema`, `testkit`, `client`, `platform`, `codec`, `dedup`, `metadata`, `query`, `catalog`, `edit`, `exiftool`, `jobs`, `jobs-testkit`, `trace`, `video`, `analysis`, `render`, `scan`, `thumbcache`, `borders`, `import`, `thumbs`, `writeback`, `crop`, `yandex`, `library`, `app`, `cli`, `gallery`, `desktop` and `bench`.

**Also checked:**

- **Pure crates.** `model`, `client`, `container`, `query` and `dedup` make no file, thread, environment or clock calls.
- **Test-support features** are enabled only from `[dev-dependencies]`.

**Settled during the read:**

- **Rotation tag.** `edit/src/rotate.rs` writes EXIF tag 0x0112 only in its tests; its production code ends at line 85.
- **Yandex replay.** `app/src/yandex/replay.rs` follows the client's flow. It answers through `pg_yandex`'s own `parse_upload`, `parse_other_sizes`, `parse_similar_page` and `read_similar_pages`, and checks `is_captcha_page` on every page.
- **Plan steps already in the tree:**
  - M-1: `alert::error_box`, `log.rs`.
  - M-6: `deadline::owner`/`arm`, the shell's `timeout` invokable.
  - M-13: `pg_platform::console::attach_to_parent`, `names::after_first_equals`, `names::arg_bytes`, `file::play_target`.
  - M-14: `pg_testkit::odd_name`.
  - What is left of M-13 is M-40.
- **Cohesive crates.** These have no finding, except where a step below names them: `container` (one copy, in M-25), `exiftool`, `trace`, `jobs-testkit` and `bench`.

The steps are grouped by kind: the bugs first, then work done twice, then code in the wrong crate, then types and names, then cleanup.

## Bugs

### M-21: the bugs the read found

Each fix gets a test that fails before it.

- **A temp file reached by its plain path.**
  - Both copies of the scan follow-up delete a replaced file's queued-write temp with `std::fs::remove_file(path_buf(..))`: `library/src/runner.rs` (`Scans::run`) and `cli/src/lib.rs` (`run_command`, `Scan`).
  - The writer reaches files only by `verbatim(..)`. On Windows a plain path drops a trailing dot or space from a folder name, so a temp in such a folder is missed, or another folder's file is deleted.
  - Fix: `verbatim`, in M-22's one `after_scan`.
  - Test: a temp under a folder named `a.` is deleted, and its neighbour in `a` is kept.
- **An unreadable preferences file overwrites the last one kept.**
  - `app/src/store.rs` (`quarantine`) renames the file to `<name>.bad`. A rename replaces its target on both systems, so the `.bad` an earlier failure kept is lost. The doc says it "is kept".
  - Fix: `pg_schema::move_aside`, which numbers them (`.bad.1`, ...).
- **Two notes of the wrong kind.**
  - `App::open` reports a thumbnail cache that cannot be opened as `Note::Thumbs(Event::NotStored { error: "cache disabled: .." })`. That event means one on-demand thumbnail was not stored.
  - `App::shutdown` reports a catalog that fails to close as `Note::Thumbs(Event::WalNotMerged { error: "catalog: .." })`, the cache's WAL event.
  - Both front ends word a note by its variant, so the user reads the wrong sentence.
  - Fix: `Note::CacheDisabled { error }` and `Note::CatalogNotClosed { error }`, each with its own words.
- **A filter's shapes shown untranslated.**
  - `client/src/text.rs` (`describe`) writes a chosen shape by its key (`o.key()`, English), where the bar uses `shape_label`.
  - Fix: `shape_label`.
  - Test: the pseudo-locale's description of a query with a shape has no English word.
- Gate: Windows `just test`, Linux checks.

## Work done twice

### M-22: one scan follow-up, one way to add a folder

- **Found.** The command line and the library runner each do what a scan's result asks for, and the copies have drifted.
  - `cli/src/lib.rs` (`run_command`, the `Scan` arm) and `library/src/runner.rs` (`Scans::run`) each forget the replaced files' thumbnails, delete their queued writes' temp files ("decision 7"), and turn the scan's counts into notes.
  - The runner reports `ScanStats::refused_mount_list` (`Notice::MountRefused`): the mounts under a root that the scan refused, such as FAT and exFAT on Linux since `6e20cad`. `gallery scan` never prints it, so a refused mount is skipped without a word. It does not print the `VolumeFallback` line either.
  - The CLI's scan notes are its own English (`"skipped as duplicates (another path to the same file)"`, ...), while the app's come from `scan-note` in `client/locales/en/text.ftl`.
  - Both copies delete temps by the plain path (M-21).
- **Also twice: adding a folder.** `normalize_path`, then `check_folder`, then `Catalog::add_root`, in the CLI's `Scan` arm and in `app/src/library.rs` (`LibraryService::add_folder`).
- **Fix.** `pg-library` gets the one copy of each, used by both front ends.
  - `add_folder(catalog, path) -> Result<Added, AddError>` normalizes, checks and adds. `AddError` is typed: `Refused`, `NotAPath`, the catalog's error.
  - `after_scan(catalog, thumbs, &ScanStats) -> Vec<Notice>` does the forgets and the deletions (by `verbatim`). It returns every note the scan's result carries as the runner's `Notice`s: its failures, `UnreadableFolders`, `MountRefused`, `VolumeFallback` and the `ScanNote` counts. The runner's `Scans::run` and the CLI call it.
  - The CLI prints each `Notice` with the same `pg_client::text` words the window shows (`text::scan_note`, ...), so its hard-coded English goes.
  - The CLI's scan opens the thumbnail cache only when `replaced_ids` is not empty, as now.
  - `LibraryService::remove_folder` stops a running scan by queueing an empty one (`library.scan(Vec::new())`). It becomes `Runner::stop_scan`.
- **Test.** A `pg-cli` test over a `ScanStats` with a refused mount: `gallery scan` prints it. The runner's existing scan tests keep passing.
- Gate: Windows `just test`, Linux checks.

### M-23: one skeleton for the per-file passes

- **Found.**
  - `pg-borders`, `pg-import` and `pg-thumbs` each write the same skeleton:
    - the work list, then `ctl.begin`;
    - a `batch` of `(index, result)`, flushed every `BATCH` items in a catalog transaction with the pass's writer;
    - `pipeline::run`, whose read closure is `let [folder, name] = work.parts(item.path); set_file(wide, folder, name)`, and whose sink is `ctl.add`, `batch.push` and the flush.
  - The path FFmpeg opens a video by (`verbatim(&path_buf(&work.path(item.path, sep)))`, with its comment) is written in both `import` and `thumbs`.
  - The thumbnail and crop passes show failure paths with `String::from_utf8_lossy` of the stored bytes, where `PathRules::display` is the rule.
  - The rule "the file is still as listed, and the app is not writing to it" is written three times: in Rust in `thumbs` (`file_state`: mtime, size, rotation, writing), and in SQL in `AnalysisWriter::current` and `BordersWriter::current`.
  - `pipeline::run` reports a worker whose setup failed by sending index `usize::MAX` on the results channel.
- **Fix.**
  - `pg_jobs::pipeline::batched` takes the read, the work and the flush; the three passes become their work and their writer.
  - `pg_platform::names::verbatim_of(&WorkList, FilePath)`.
  - `PathRules::display` for every shown path.
  - The still-as-listed rule is one SQL fragment, and `thumbs` reads its result instead of comparing again.
  - The results channel carries an enum: `Done(index, result)` or `SetupFailed(error)`.
- Gate: `just test`, Linux checks, perf (the per-file benches).

### M-24: a database file's lifecycle is `pg-schema`'s, and the catalog's `schema.rs` is split

- **Found.**
  - **The thumbnail cache repeats the catalog's file lifecycle:**
    - the lock;
    - the judgement against an expected schema held in memory (a `OnceLock<Schema>` in each);
    - moving a file aside (`move_damaged` against `rebuild`);
    - the shared and exclusive holds (`hold_shared` against `take_shared`), worded alike;
    - deleting a database and its sidecars: `thumbcache`'s `schema::open_once` loops over `["-wal", "-shm", "-journal", ""]`, and the catalog has `lifecycle::remove_database`.
  - **`beside(path, suffix)` is written five times:** in `pg_schema` (and again inline in `Lock::take`), `catalog/src/lifecycle.rs`, `thumbcache/src/schema.rs`, and `app/src/store.rs` (`with_suffix`). `pg-catalog` already depends on `pg-schema`.
  - **`pg-schema`'s name and header say "schema" only**, though it holds `Lock`, `Live` and `move_aside`.
  - **The catalog's `schema.rs` holds three concerns:**
    - the schema: `BASELINE`, `MIGRATIONS`, the versions, `expected`, `mismatch`;
    - the rebuild's row copy: `carry_rows`, `copy_tables`, `convert_text_folders`, `table_columns`, `column_type`;
    - `write_transaction`.
  - **The catalog's `lifecycle.rs` holds the rest of the rebuild, and more:**
    - the rebuild's file side: `rebuild`, `make_replacement`, `remove_database`, `stored_under`;
    - the locks and the version marks;
    - the work lists: `pending_import_sql`, `work_list`, `max_folder_id`.
- **Fix.**
  - `pg-schema` gains `remove_database` (with the sidecars) and a public `beside`; the catalog, the cache and the app's store use them.
  - Its header says it holds a SQLite file's lifecycle.
  - The catalog is split:
    - `rebuild.rs`: both halves of the rebuild;
    - `lifecycle.rs`: the locks, holds and marks;
    - `work.rs`: the work lists;
    - `schema.rs`: the schema.
- Gate: `just test`, Linux checks, perf.

### M-25: the container formats are written where they are read

- **Found.**
  - **Whether a file is animated is decided twice.**
    - `edit`'s `rotate` and `crop` look for any `acTL` chunk (PNG) or `ANIM` chunk (WebP); `rotate` also looks for `VP8L`.
    - `pg-metadata`'s `container.rs` counts frames: from an `acTL` before the image data, and from `ANMF` chunks.
    - So a PNG with an `acTL` after its IDAT is a still to the metadata and an animation to the edit.
  - **The container writers are outside `pg-container`:**
    - the PNG chunk and its CRC: `edit::rewrite::chunk::png_chunk`, `metadata::orientation`'s eXIf CRC fix-up, and test helpers in `codec` and `metadata`;
    - the RIFF header and the VP8X canvas fields (`rewrite::webp`);
    - the GIF sub-blocks (`rewrite::gif::sub_blocks`), and a second sub-block reader for the LZW data.
  - **The GIF colour-table size** (`3 << ((packed & 7) + 1)`) is written three times: `pg_container::gif_header_len`, `GifBlocks::block`'s 0x2C arm, and `rewrite::gif`.
- **Fix.**
  - One rule, `pg_container::animated(bytes, format)` (and `webp_lossless`), used by the edit and the metadata.
  - `pg-container` gains the writing half: `png_chunk`, `fix_png_crc`, `riff` and `gif_sub_blocks`.
  - `GifBlock::Image` carries the parsed descriptor: place, size, packed byte, table length and code size.
  - The SQL rule "a still is an image of one frame" (`pg_query::media_sql`, `MEDIA_CASE`, and `pg_catalog::sql`'s analysable stills) reads `analysis.frames`, which is the metadata's count. It agrees with the edit once both use the container's rule.
  - Test: a PNG with an `acTL` after its IDAT is the same kind to both.
- Gate: `just test`, the fuzzer's container target (Linux).

### M-26: an event's words are written once

- **Found.**
  - Both front ends format the lower layers' events for the log, word for word:
    - the thumbnail cache's `Event`, every variant, with `started_over_reason`: `cli/src/report.rs` (`thumb_event_text`) and `desktop/src/app.rs` (`note_text`, `Note::Thumbs`);
    - "catalog rebuilt: …": `cli/src/report.rs` (`report_rebuilt`) and `desktop/src/app.rs`.
  - Beneath them, `pg-catalog` writes English of its own: `Rebuilt::carried_note` (`catalog/src/types.rs`).
  - Other English outside the Fluent bundle, which the pseudo-locale's clipping check therefore never sees:
    - the key hints in `client/src/keys.rs` (`SELECT`, `VIEW`, ...);
    - the selection line in `desktop/src/gallery/items.rs` (`refresh_derived`, `format!("{}   {symbol}")`).
- **Fix.**
  - The log lines are like an error's `Display`, so each type says itself: `impl Display for pg_thumbcache::Event` and `for pg_catalog::Rebuilt` (which takes in `carried_note`). The front ends print `{event}`.
  - The UI's words for a start-over (`text::thumb_cache_started_over`, `CacheStartOver`) stay the client's, and `started_over_reason` is written once, where the UI shows it. If the log is to be localized later, the log lines move to the Fluent bundle together.
  - The key hints and the selection line move to `ui.ftl`, through `pg_client::text`.
- Gate: `just test`, `just shots` (the pseudo-locale).

### M-27: one JPEG encoder, and the maker decodes through `display`

- **Found.**
  - turbojpeg's `Compressor` is set up twice: `Maker::new`/`encode_image` at q85 with 4:2:0, and `transcode::to_jpeg` at q95 with 4:4:4. Each writes its own `Image { pixels, width, pitch: width * 3, height, RGB }`.
  - The codec owns decoding (and `encode::cropped`), but not this encoder.
  - `render/src/display.rs` says it is "the one place that reads a file's EXIF orientation for decoding". But `Maker::make_image` calls `pg_metadata::orientation` and `Decoder::decode_into` itself.
- **Fix.**
  - `pg_codec::encode::jpeg(&Rgb, quality, subsampling)`, used by both.
  - The maker decodes through `display::decode_into`, into its own buffers, so the claim holds (with M-30).
- Gate: `just test`, perf (`thumb_jpeg`), `just shots`.

### M-28: one numeric conversion, one fit, one decode cap

- **Found.**
  - **The clamped float-to-integer conversion is written eight times:**
    - `pg_codec::decode` and `resize` (`to_pixels`);
    - `pg_app::images`;
    - `pg_video` (`whole_i64`, `whole_u32`);
    - `pg_analysis::borders::side::to_count`;
    - `pg_thumbcache::config::whole_bytes`;
    - `pg_client::controller::prefs`;
    - `desktop/src/screenshot.rs` (`millis`).
  - **Scaling to fit a box is written three times:** `Decoder::decode_jpeg` (`MinSize::Fit`), `Resizer::fit_box`, and `app::images::big_enough`.
  - **`pg-video` writes two conversions twice each:** the display width from the sample aspect ratio (`probe`, `to_rgb`), and a stream timestamp to milliseconds (`probe`'s `in_ms`, `frame_into`'s `ms`).
  - **`cli/src/borders.rs` copies the decoders' cap.** Its `MAX_PIXELS` is `(512 << 20) / 3`, while `pg_codec::max_decode_bytes()` is the rule.
  - **`DECODE_MARGIN`** is in `pg_app::images` and `pg_render::maker`, with the same value and the same comment.
  - **The FILETIME epoch** is `pg_query`'s `filetime_unix_offset!` and `pg_platform::file::UNIX_EPOCH_TICKS`.
  - **BigTIFF is detected twice:** `pg_codec::encode::tiff_samples_cropped` and `pg_model::format::sniff`.
  - **The civil date of a day count** (Hinnant's `civil_from_days`) is in `model/src/date.rs` (`civil`), and inline in `platform/src/clock.rs` (`local_time`, Unix).
- **Fix.**
  - `pg_model::num` gets the clamping conversions (`clamp_to_i32`, `clamp_to_u32`, `clamp_to_i64`, `clamp_to_usize`), used by all eight.
  - `MinSize::fit_scale` in the codec, used by all three fits (with M-30).
  - `Stream::display_width` and `Stream::ms`.
  - `MAX_PIXELS` is computed from `max_decode_bytes()`.
  - `DECODE_MARGIN` once, in `pg-render` (M-30 moves the app's copy).
  - Shared from `pg_model`, used by the other crates:
    - the FILETIME epoch, as a macro;
    - BigTIFF detection, in `pg_model::format`;
    - `pg_model::date::civil`, made public.
- Gate: `just test`, perf.

## Code in the wrong crate

### M-29: the Yandex search is an `App` service

- **Found.**
  - Every other service hangs off `App` (M-4); the search does not:
    - The desktop's QObject owns the `pg_app::yandex::Search` (`desktop/src/yandex/state.rs`, `search: Search`).
    - The search publishes its images through a process-wide `static CURRENT: RwLock<Option<Arc<Images>>>` (`app/src/yandex/images.rs`, `set_current`, `current`).
    - The image provider reaches them through the free function `pg_app::yandex::image(id)` (`desktop/src/providers.rs`).
  - So the panel and the provider are linked by a global, not a reference, and `App::stop` and `shutdown` do not stop a search or a save in progress.
  - `desktop/src/library.rs`'s `static LIBRARY: OnceLock<&'static pg_app::Library>` caches what `app::get().library()` already holds.
  - `app/src/yandex/images.rs` (`grid_picture`) keeps a thread-local `Maker`, a second pool beside `makers::Makers`.
  - The Yandex grid picture's height (220) is written three times: `yandex::images::GRID_HEIGHT`, the panel's `ROW_HEIGHT`, and `Prefs::row_height`'s default.
- **Fix.**
  - `App` holds a `Yandex` service (`app.yandex()`), with the `Search` and the images of the search on screen as fields.
    - It is built with the `Factory` from `Config`: the command line's `--yandex-fixture` goes into `Config`, not `desktop/src/command_line.rs::yandex_source`.
    - `CURRENT`, `set_current`, `current` and the free `image` go. The provider calls `app::get().yandex().image(id)`, and the QObject holds no `Search`.
    - `App::stop` stops the search's threads in its order, before the catalogs.
  - `LIBRARY` goes; the desktop reads `app::get().library()`.
  - `grid_picture` uses the app's maker pool.
  - The panel's row height is the desktop's, so the desktop passes it to the search.
- **Kept, with the reason written beside each.** The desktop's other process-wide state is one QObject's state that a sibling QObject reads: `controller::CORE`, `gallery::MODEL`, `folders::TREE`, and `picker`'s `CHOSEN`, `LISTING` and `START`. cxx-qt default-constructs the QObjects, and nothing hands them a reference. `facet_choices::COUNTS` goes in M-32.
- Gate: `just test`, `just shots` (the `yandex` scenarios), Linux checks.

### M-30: the viewer's images are `pg-render`'s

- **Found.**
  - `pg-render` is "what a file shows as" (`render::display`), but the viewer's image pipeline is in `app/src/images.rs` (`Images::full_image`, `decode`, `big_enough`, `placeholder`):
    - the decode size for each `View` (`DECODE_MARGIN`);
    - reusing the last decode when it is big enough;
    - the fit and region resizes;
    - the ThumbHash placeholder;
    - the video poster.
  - That is why `pg-app` depends on `pg-codec` (`MinSize`, `orient_pixels`, `Rgb`, `Resizer`), on `pg-video` (`frame`), and on `thumbhash` itself.
- **Fix.**
  - `render::display` gets the pipeline over bytes and frames:
    - `min_size(view)` and `big_enough`;
    - `fit(view, &Rgb)`, for the fit and the region;
    - `placeholder(hash, turns)`;
    - the video poster, through `pg_video` (already a dependency of `pg-render`).
  - `Images` keeps what is the app's: the catalog lookup, the consistency check (`consistent`), the shared last decode and the makers.
  - `pg-app` loses `pg-codec`, `pg-video` and `thumbhash` if nothing else of them is left. The upload type moves out (M-33). The layer file changes with it.
- Gate: `just test`, `just shots`, perf (the viewer is not benched; the gate is that nothing moves).

### M-31: a pass does not reach a sibling pass for a helper

- **Found.** `pg-crop` depends on `pg-scan` and `pg-writeback` only for helpers:
  - `pg_scan::normalize_path`: the real path, then `rule_path_of`, both from `pg_platform::names`. `app`, `cli` and the `library` tests use it too.
  - `pg_writeback::exiftool_program`: `PG_EXIFTOOL`, else `exiftool`. This is ExifTool's concern.
  - `pg_writeback::stat`: a file's size and last-write time. This is `pg_platform::file`'s concern (`last_write_time` is there).
- **Fix.**
  - `normalize_path` moves to `pg_platform::names`; its errors become a typed one, or stay `io::Error` with the path.
  - `exiftool_program` moves to `pg-exiftool`, and `stat` to `pg_platform::file`.
  - Callers are changed, with no re-export (ground rule 2).
  - `pg-crop` loses `pg-scan` and `pg-writeback`, which leaves `crop: catalog edit exiftool jobs model platform` in the layer file. `pg-app` keeps `pg-scan` only if `check_folder` is still its; after M-22 it is `pg-library`'s.
  - The layer file changes in the same commit.
- Gate: `just lint` (the layer check), `just test`, Linux checks.

### M-32: the Qt crate holds no logic that needs no Qt

- **Found.**
  - **The filter bar's facet choices.**
    - `desktop/src/facet_choices.rs` (`Counts`, `choices`, `rows`) decides which choices a facet offers, in the bar's order, with which counts.
    - `desktop/src/filters.rs` (`publish_counts`) assembles the counts and adds the chosen extensions the folder lacks.
    - The two Qt modules pass the counts through a thread-local (`facet_choices::COUNTS`, `set_counts`).
  - **The filter bar's yes/no/either choices** are mapped to and from a combo-box index four times in `desktop/src/filters.rs`: `duplicates_index`, `review_index`, and their inverses in `set_*_index`.
  - **`Filters::sync`** writes three compare-and-set blocks by hand, where the bridge's generated setters do it.
  - **`edits::set_rating`** clamps an `i32`, where `Rating::try_from` should read it.
  - **The Yandex panel's result list** is kept in `desktop/src/yandex.rs` (`without_dead`, `drop_dead`, and `show`'s bookkeeping): its two groups, the picks that follow their results, and taking the dead results off.
- **Fix.**
  - `pg_client::facets` gets `Counts`, `choices` (the order of each facet's choices) and the extension merge, tested without Qt.
    - The counts live in the client's controller state with the query (`desktop/src/controller.rs`'s `CORE`), so `COUNTS` goes and `FacetChoices::refresh` reads both from one place.
  - The yes/no/either mapping goes to `pg_client::input` (`tri_index`, `tri_of_index`), beside `Facet::from_index` and `Field::from_index`.
  - `Filters::sync` uses the setters.
  - The result list becomes a `pg_client` type, generic over its item (`Results<T>`: groups, picks, `drop(ids)`), which the panel's QObject holds. `Hit` stays `pg_app`'s.
  - What remains in the desktop is roles, Qt ints and signals.
- **Not moved.** The outcome-to-words functions stay the desktop's: `desktop/src/yandex.rs` (`status_text`, `explain`, ...) and `desktop/src/library.rs` (`notice_text`, `write_error_text`). They match on `pg_app` and `pg_library` types, which `pg-client` may not name (it depends on `pg-model` alone). M-22 makes the command line use the same `text` functions for the scan.
- Gate: `just test`, `just qmltest`, `just shots`.

### M-33: Qt's conventions stay in the Qt crate

- **Found.** Qt's int conventions, -1 for none, are used in the crates without Qt.
  - **In `pg-client`:**
    - `Grid::item(i32)` and `position_of -> i32`;
    - `Selection { current: i32, anchor: i32 }`;
    - `Placement::neighbor(i32, ..) -> i32`;
    - `Tree::row(i32)`;
    - `Facet/Field/Edge::from_index(i32)`, `sort_choice(i32)` and `choice_index -> i32`;
    - `Event::Rate { rating: i32 }`.
  - **In `pg-app`:**
    - `Prefs::last_file: i64`, with -1 for none;
    - `yandex::Hit { width: i32, height: i32 }`;
    - `Error::UnknownFile(i64)` and `Rewriting(i64)`.
  - **`pg_app::images::Upload`** is Qt's scene-graph texture format (`QSGPlainTexture`, RGBX8888, Qt's `qPremultiply` rounding), and its doc names Qt.
  - **The desktop passes text and ints where it has types:**
    - `gallery::items::border_rejected(item, "top")` takes the edge as text, not `detection::Edge`.
    - `keymap::media_of` reads `GalleryModel.MediaKind` as an int, by its position, though the bridge declares the enum.
    - `viewport::swaps_axes` builds `Turns` from an `i32` with `rem_euclid`.
- **Fix.**
  - `pg-client` and `pg-app` use `Option<usize>`, `Option<FileId>`, `FileId`, `Rating` and the model's dimension type.
  - The desktop converts at the bridge, with one helper each way (`row_of(i32) -> Option<usize>`, `to_row(Option<usize>) -> i32`).
  - The texture conversion moves to the desktop. `pg-render` keeps a premultiplied RGBA image named for what it is, without Qt.
  - The desktop's own calls take `Edge` and the bridge's `MediaKind`.
  - One `Turns` constructor, from any quarter-turn count, goes in `pg-model`.
- Gate: `just test`, `just qmltest`, `just shots`.

### M-34: a refused folder is a typed value, decided by the scanner and worded by the client

- **Found.**
  - **Refusals are found by downcast.** `pg_scan::scan` returns `anyhow::Result`, and its callers tell a refused folder from a failure with `downcast_ref::<pg_platform::dir::Refused>()`: `cli/src/lib.rs` (the `Scan` arm) and `library/src/runner.rs` (`Scans::run`). The contract is a doc comment (`scan/src/lib.rs`, "an error that is a `Refused`"), and nothing checks it.
  - **The policy and its words are in the OS layer.** `pg-platform` decides which folders the app accepts (`dir::check_ntfs` for decision 6's steps, `check_drive`, and the Linux `REFUSED` table of file systems). It also writes the reasons in English ("it is on an NFS mount", "its drive is removable", ...). They reach the window inside a Fluent message (`text::root_refused(root, reason)`), so they are never translated or pseudo-localized.
  - **`pg_scan::Listing::check_mount`** returns its refusal as a `Result<(), String>`.
  - **One refusal, two types.** `pg_crop::CropError::FolderRefused { file, folder }` and `pg_writeback::WriteError::FolderRefused { file, folder }` have the same English `Display`, and each repeats the `check_folder_names` mapping.
  - **Public for the tests only:** `RootFacts`, `check_ntfs`, `check_fs_type` and `NativePath::set_plain`.
- **Fix.**
  - `pg-platform` reports facts as a typed value: the drive type, the file system's name or `f_type`, a reparse point, the real volume.
  - `pg-scan` holds the policy table. `scan` returns `Result<ScanStats, ScanError>`, with `ScanError::{Refused(Refused), Failed(anyhow::Error)}` and `Refused { reason: RefusedReason }`, and both callers match on it.
  - `check_mount` returns `Result<(), RefusedReason>`.
  - The client words each `RefusedReason`.
  - One `FolderRefused`, built from `pg_platform::native::AlteredFolder`, is carried by both `CropError` and `WriteError`.
  - The test-only items become `pub(crate)`, with their tests beside them.
  - This was the only typed outcome downcast across crates. The rest are within a crate, or `jobs`' own `Panicked`.
- Gate: Windows `just test`, Linux checks (the refused-mount tests), `just shots` (the pseudo-locale).

### M-35: the `image://` scheme in one place

- **Found.** The scheme is written in three places.
  - `pg_model::image` builds the `thumb`, `thumbhash` and `full` URLs: front-end protocol in the domain model.
  - `desktop/src/yandex.rs` builds `image://yandex/{id}` itself (`picture_url`, `original_url`).
  - `desktop/src/providers.rs` writes the four provider names a third time (`provider`), and parses the Yandex request (`<id>` or `<id>/original`) itself.
- **Fix.**
  - The key and the view (`ImageKey`, `View`) are the app's interface, and stay in `pg-model`.
  - The URL half moves to `pg_client::image`: `ImageRequest` gains `Yandex { id, original }`, and `provider` and `parse` are the one map between requests and names.
  - The desktop builds and parses every URL through it.
- Gate: `just test`, `just shots` (every provider).

### M-36: `Layout` names every file of the data folder

- **Found.**
  - `library/src/layout.rs` says "the layout lives in one place". The names are elsewhere:
    - `thumbs.sqlite`: `pg_thumbcache::config::cache_path`;
    - `catalog.sqlite`: `pg_library::catalog_path`;
    - the data folder's name: `pg_platform::dirs::default_home`;
    - the SQLite alias folder: `pg_platform::sqlite::alias_of_folder`, which is `%LOCALAPPDATA%\PhotoGallery\sqlite-aliases`, then `ProgramData`.
  - The alias folder ignores `PG_HOME` and the layout. A run with a scratch home therefore still writes under the user's profile when a catalog folder's name is not UTF-8. That breaks the A5 rule: nothing of ours outside the data folder.
  - `pg_app::Config` copies `Layout`'s four paths field by field.
- **Fix.**
  - `Layout` names every file: `catalog_path`, `thumbs_path`, and `sqlite_aliases` (`home/sqlite-aliases`). Each is passed down, and `pg_platform::sqlite` takes the alias folder as an argument.
  - `Config` holds the `Layout`.
  - Test: with `PG_HOME` set, opening a catalog in a folder with a non-UTF-8 name writes nothing outside the home.
- Gate: Windows `just test` (the alias path is Windows'), Linux checks.

### M-37: the thumbnail cache reads the catalog through its views only

- **Found.**
  - `pg-thumbcache` uses `pg-catalog` for one function, `pg_catalog::max_folder_id(&tx, "cat.")` (`thumbcache/src/reader.rs`, `pending_in`). It reads the catalog's raw `folders` table.
  - `reader::pending_sql` tests `f.kind = 'video'`, which is the catalog's stored key.
  - The rest of the contract is the catalog's views (`thumb_sources`, `thumb_clock`, `catalog/src/schema.rs`). That is the designed interface.
  - In the other direction, the catalog's test `thumb_sources_is_the_analysed_files_with_their_folders` (`catalog/src/writers.rs`) copies the cache's pending join, written against the cache's private `entries` table.
- **Fix.**
  - The folder count becomes part of the views: `thumb_clock` gains `max_folder_id`, or a new `thumb_folders` view.
  - `thumb_sources` gives `poster_ms` already resolved.
  - `pg-thumbcache` loses its normal `pg-catalog` dependency; it keeps the `fixture` dev-dependency for its tests.
  - The catalog's test checks only its views. The join is tested in `pg-thumbcache`, which owns `entries`.
- Gate: `just test`, Linux checks, perf (`pending_thumbs.at_height`).

### M-38: `pg-writeback`'s dependencies

- **Found.**
  - `pg-writeback` depends on `pg-analysis` for one function, `analyze::file_hash` (BLAKE3 of the bytes). That pulls in the analysis layer of the codec, video (FFmpeg) and metadata crates.
  - Its `Cargo.toml` comment lists "the edit, the metadata and ExifTool". It also depends on analysis, catalog, jobs and thumbcache.
  - `WriteStatus::thumbnail_error: Option<String>` is English made here (`"file {id}: {e:#}"`).
- **Fix.**
  - `file_hash` moves where both reach it without the analysis: `pg_model::analysis::FileHash`, with the `blake3` call in `pg-metadata`. `pg-writeback` loses `pg-analysis`, and the layer file changes with it.
  - The comment states the dependencies as they are.
  - `thumbnail_error` is a typed `(FileId, Error)` that the client words.
- Gate: `just lint` (the layer check), `just test`.

### M-39: `pg-yandex`'s error holds only the client's cases

- **Found.**
  - `YandexError::Undecodable` and `YandexError::Recorded` are made only by `pg-app` (`yandex/images.rs`, `yandex.rs`, `yandex/replay.rs`); the variant's own doc says "Never made by the client".
  - `pg_yandex::sniffed` lists again which formats the app decodes (JPEG, PNG, WebP, GIF, BMP), where the codec's list is the rule.
- **Fix.**
  - The app's `Source` returns its own error, `pg_app::yandex::SourceError { Yandex(YandexError), Undecodable, Recorded }`, and `pg-yandex` drops the two variants.
  - `sniffed` takes the codec's list. `pg-yandex` depends on `pg-model` alone, so the list is either passed in or moved to `pg_model::format` (and the codec reads it there).
- Gate: `just test`, `just shots` (the `yandex` scenarios).

### M-40: the platform code left in the desktop

- **Found.**
  - Most of M-13 is done, but `desktop/src/picker.rs` still has three platform items: `cfg(windows)` on `native` and `run_dialog`, and a `not(windows)` stub.
  - The in-app dialog's listing, `folders_in`, is a directory read outside `pg_platform::dir`.
  - `pg_platform::picker::start_folder` reads `USERPROFILE` and `HOME` directly, where `dirs` takes a lookup.
  - `pg_platform::names` holds the command line's argument bytes (`arg_bytes`, `after_first_equals`) beside the path names.
- **Fix.**
  - `pg_platform::picker::pick(start, owner) -> io::Result<Option<PathBuf>>` exists on every system and returns `Unsupported` off Windows; the desktop's three items go.
  - `folders_in` moves to `pg_platform::dir`.
  - `start_folder` takes `dirs`' lookup.
  - The argument bytes move to an `args` module.
- Gate: Windows `just test`, Linux checks, `just shots` (the pickers).

### M-41: an item lives with its only user

- **`pg_model::query::parse_number`** reads typed input (a comma or a point). `query.rs`'s header says the front end reads text typed into a filter. Its users are `client::input` and `desktop::filters`. It moves to `pg_client::input`.
- **`pg_model::date::exif_date`** is EXIF's text format (kamadak-exif's rules), and only `pg-metadata` uses it. It moves to `metadata/src/exif.rs`; the model keeps `wall_clock_seconds`.
- **`pg_thumbcache::config::parse_cap_gb`** reads the command line's `--cap-gb` and the app's `PG_THUMB_CAP_GB`, and makes English errors in the cache crate. It moves to `pg_client::input`, and the cache takes bytes.
- **`pg_dedup::pdq_index` and `PdqIndex`** are public but used only through `Pairer` and `IndexFull`. The module becomes private.
- **`pg_analysis::analyze::video(path, turns, &mut Analyzer)`** becomes `Analyzer::video`, beside `Analyzer::analyze`.
- **`pg_scan::child_path`**, which extends a `RulePath` by a name, becomes a `RulePath` method.
- **`pg_crop::place_of`** reads the anchor's bytes and drops `:` and `\` (the Windows anchor). It becomes a `PathRules` method.
- **`cli::rule_path_of`** (taking the rules) duplicates `pg_platform::names::rule_path_of` (native). It becomes a `PathRules` method.
- **`ScanStats`** (`scan/src/lib.rs`) re-declares `Pairing`'s fields (`catalog/src/pair.rs`: `rebound`, `replaced`, `ambiguous`, `unpaired_with_data`, `replaced_ids`, `temps`) and copies them one by one. It holds the `Pairing` instead.
- Gate: `just test`.

## Types and names

### M-42: typed values across the catalog's API, and stored keys spelled once

- **Found.**
  - **Values cross the catalog's API as text:**
    - `write_target` returns `(RulePath, Option<String>, String)`, with the format and the kind as text.
    - `writes::rating_in_file_possible(kind: &str, format: Option<&str>)` is called by `set_rating` and `browse::read::details`.
    - `Fresh { ext: String, kind: String }`.
    - `file_source` compares `kind == Kind::Video.key()`.
    - `Facets::exts` is a `Vec<(String, i64)>`.
  - **The write queue's value is untyped.**
    - `Job { kind: WriteKind, value: i64 }` holds "the rating, or the quarter turns", and `temp` is an `Option<(RulePath, i64)>`.
    - `done` uses `debug_assert!`s to check that kind and value agree.
    - `pg-writeback` converts the value at every use (`u8::try_from(job.value)`, `job_turns`' `rem_euclid`, `rewrote`'s `clamp(0, 5)`), and so does `WriteEffect::FileRating { rating: u8 }`.
  - **An unknown rewrite** is `Rewrite::before` set to `(-1, -1)`.
  - **Scan ids are raw `i64`s:** `begin_scan -> i64`, `finish_scan(i64)`, `mark_missing(.., scan: i64, ..)`, `ScanWriter`, `after_scan`.
  - **An unknown kind is silently an image.** `lifecycle::work_list` maps an unknown `files.kind` to `Kind::Image`, while `entry_from_row` fails on an unknown extension.
  - **rusqlite in the public API:**
    - `Catalog::release_memory -> rusqlite::Result`.
    - `ScanWriter::new`, `AnalysisWriter::new` and `BordersWriter::new` are `pub` and take a `rusqlite::Transaction`, which only `Tx` makes, and `Tx` is `pub(crate)`.
    - `max_folder_id(&Connection, schema: &str)` (M-37).
  - **Stored keys are string literals.** The `files.kind` and `file_writes.kind` values (`'video'`, `'image'`, `'rating'`, `'rotation'`) are spelled out in:
    - the catalog: `writes`, `writers`, `pair`, `browse::read::{grid,state}`;
    - `query/src/lib.rs`;
    - `thumbcache/src/reader.rs` (M-37).
  - **`pg_app::reader::Session::submit` borrows the catalog's error.** It carries a panic and an open failure as `pg_catalog::Error::Io(io::Error::other(..))`.
  - **`pg_metadata::Meta::rating` is an `Option<u8>`**, though `pg_model::rating::Rating` exists and the analysis holds an `Option<Rating>`.
- **Fix.**
  - **Each value is parsed once, at the row:**
    - `write_target` returns `WriteTarget { path, format: Option<Format>, kind: Kind }`.
    - `rating_in_file_possible(Kind, Option<Format>)`, beside M-17's `can_hold_rating`.
    - `Fresh { ext: Ext, kind: Kind }`.
    - The queue's value is `enum Write { Rating(Option<Rating>), Turns(Turns) }`, in the job and in the temp. The `debug_assert!`s and `pg-writeback`'s conversions go.
    - `Rewrite::before` is an `Option`.
    - `ScanId` goes in `pg_model::ids`, beside `FileId`, `FolderId` and `RootId`.
  - **An unknown kind is an error**, as an unknown extension is.
  - **rusqlite leaves the public API.** The writers' constructors become `pub(crate)`, made by `Tx`'s methods, and `release_memory` returns the catalog's `Result`.
  - **Every stored key is a `pg-model` macro**, as `min_side!` and `group_distance!` already are. The catalog's SQL, `query` and the cache all reach `pg-model`.
  - **The reader gets an error of its own**, with a variant for each of its two cases.
  - **`Meta::rating` is an `Option<Rating>`.**
- Gate: `just test`, Linux checks, perf (the import and the catalog are benched).

### M-43: the session is a typed `Listing`, stored by the catalog

- **Found.**
  - **The session is a store of text keys and values** (`session_get`, `session_set`, `session_set_many(key: &str, value: &str)`).
    - Its keys and encodings are in `app/src/views.rs`: `folder` as JSON, `recursive` as `"true"`, `query`, `sort`.
    - The catalog's rebuild knows the `folder` key and its JSON shape too (`schema::convert_text_folders`), so the session's format is written in two crates.
    - Saved views, by contrast, are already stored typed, a column per field.
  - **`SavedView` copies `Listing`.** `pg_model::view::SavedView` declares `Listing`'s four fields again, plus `name`. A `Listing` is rebuilt from it field by field in three places:
    - `client::controller::listing` (`ViewApplied`);
    - `desktop/src/controller.rs` (`start`);
    - `desktop/src/shell.rs` (`save_view`).
- **Fix.**
  - `SavedView { name, #[serde(flatten)] listing: Listing }`, with no copies.
  - The catalog stores the session's `Listing` as it stores a view, by column: `session_listing()` and `save_session_listing(&Listing)`.
  - `app::views` and `convert_text_folders` lose their knowledge of the keys. A migration converts the key-value rows once.
  - `session_set` goes; it is used only by tests.
- Gate: `just test`, `just shots` (the start restores the listing), Linux checks.

### M-44: the catalog's API has one shape per concern

- **Found.**
  - `catalog/src/core.rs` has 33 methods on `Catalog`:
    - its lifecycle (`open`, `prepare`, `close`, `tx`, `commit`, the writers);
    - the roots (`roots`, `root_id`, `add_root`, `remove_root`);
    - the import and borders queues (`pending_import`, `mark_pending`, `pending_borders`, `pdq_hashes`);
    - lookups (`file_by_path`, `files_under`, `file_path`, `file_folder`);
    - crops (`crop_source`), marks (`set_border_mark`) and `status`.
  - The other concerns are 58 free functions in their modules (`browse`, `writes`, `dups`, `pair`).
  - Some concerns are split between the two forms. `set_border_mark` is a method, but `border_sides` is a free function. `mark_pending` is a method, beside the `writes` queue's free functions.
  - `browse::read` is seven `pub use x::*` globs, the source of M-3a's scan edges.
- **Fix.**
  - `core.rs` keeps the lifecycle.
  - Each concern's operations go to that concern's module, in one form: methods, in an `impl Catalog` block in that module (`roots.rs`, `queues.rs`, `lookup.rs`, and the existing `browse`, `writes`, `dups`). Callers keep `catalog.x()`, and the module says where `x` lives.
  - The free functions become methods, with their callers changed (ground rule 2: no aliases). The globs go with them.
- Gate: `just test`, Linux checks, perf (the catalog is benched).

### M-45: one name per concept

- **The list facets are three enums.**
  - `pg_model::query::ListFacet` is in the model's order: Types, Exts, Ratings, Shapes, Status, Borders. It is used only inside the model (`Query::chosen`, `Choice::facet`).
  - `pg_client::facets::Facet` is in the bar's order, and calls shapes `Orientations`.
  - `pg_query::Facet { Types, Exts, Borders }` lists the facets counted without their own values.
  - Facet and choice are mapped three times: `pg_client::input::choice` and `key_of`, and the model's `Choice::facet`.
  - Fix:
    - One enum, the model's.
    - The client keeps the bar's order over it (`ALL`), and the choices' order (M-32).
    - `pg_query`'s enum becomes `Counted`.
    - `Choice::facet` is the one mapping.
- **The cache's start-over reason is three types.**
  - `pg_thumbcache::StartOverReason`, `pg_client::text::CacheStartOver`, and the re-export `ThumbStartOverReason`.
  - A mapping is written in each front end: `cli::report::started_over_reason`, and the desktop's.
  - Fix: one enum in `pg-model`, which both the cache and the client reach, and no mappings.
- **`MediaType` is the grid's too.** `pg_model::grid::Entry::media_type` uses `query::MediaType`. Fix: it moves beside `media::Kind` (`model/src/media.rs`), and `query` uses it from there.
- **One method with two names.** `pg_model::orientation::turns_swap(turns)` is `turns.swaps_axes()`. Fix: keep `Turns::swaps_axes` and change its five callers.
- **Two `Prefs`.** `pg_client::controller::prefs::Prefs` is the grid's size; `pg_app::Prefs` is the preferences file. Fix: the client's becomes `size::GridSize`.
- **Two `writes` modules in the catalog.** `crate::writes` is the file-write queue; `browse::writes` is the views and the session. Fix: `queue` and `browse::views` (with M-44).
- **A failed file is recorded two ways.**
  - `pg_jobs::failure::Failure { path: String, error: String }` is a pass's stats, with 6 users.
  - `FileError { path: String, cause: Cause }` is used for the app's notices.
  - Fix: one `Failure { path, cause: Cause }`.
- **The jobs header contradicts its code.**
  - `jobs/src/lib.rs` says "Nothing here knows what a pass does to a file". But `control::Phase` names the library's passes (`Crop`, `Scan { root: String }`, `Import`, `Borders`, `Thumbs { height }`). `events::Change` names the runner's and the writer's getters.
  - They are the vocabulary scan, thumbcache, writeback and library share, and `pg-jobs` is the one crate all four reach.
  - Fix: keep them there and correct the header. `Scan { root }` becomes a `RulePath`, not a display string.
- **`pg_library::ScanNote::kind() -> &'static str`** is a second key for a typed enum. Fix: the app words the enum directly, and `kind` goes.
- Gate: `just test`, `just qmltest`, `just shots`.

## Cleanup

### M-46: the small duplicates

- **In the catalog:**
  - `stored_under` and `check_rules` both read `meta.path_rules` and word the same two reasons: one function.
  - `set_import_version`, `set_borders_version` and `set_taken_zone` share one shape: read the mark, an immediate transaction, read again, set, recompute. They become one `set_mark(key, value, recompute_sql)`.
  - The change-stamp trigger body is spelled 13 times in `BASELINE` and `TO_2`. A `stamp!(new|old)` macro replaces it, but only if its expansion is byte for byte the same: the baseline is frozen and compared as text.
  - The ratings upsert is in `AnalysisWriter::new` and in `writes::set_rating`. It becomes a constant, like `SET_FILE_RATING`.
  - `pair::rebind` and `pair::replace` write the same 11-column UPDATE.
  - The border colour's 0xRRGGBB packing is in `writers::store_borders`, and the unpacking in `details::border_sides_on`. `detection::Side` owns both.
  - `fixture::raw::{add_scan, add_folder}` repeat `fixture::state::{add_scan, add_folder}`.
  - `facets` sizes its sums with a literal 5. It uses `BorderFacet::COUNT` (strum).
  - `pg_query`'s `MARKED` repeats `bordered!`'s cut sum.
- **In the thumbnail cache:**
  - The seven waiting calls (`put_batch`, `retag`, `forget`, `forget_height`, `checked`, `heal`, `shrink`) each repeat `channel(); send(Op::X { reply }); recv()`. One `call` helper replaces them.
  - The upsert that forgets a height's watermark is in `evict` and `forget_height_entry`.
  - `maintenance::Drift` is a tuple of tuples; it becomes a struct.
- **In the desktop:**
  - The change pump is written twice: `desktop/src/library.rs` and `desktop/src/yandex.rs` (`ensure_pump`).
  - `screenshot::schedule` parses `--wait` the way `command_line::number_of` does.
- Gate: `just test`.

### M-47: the large files the read found

These are additions to the plan's M-12a list.

- **`video/src/ffi.rs`** (779 production lines) is one layer, the safe wrapper of FFmpeg's C API, in three parts:
  - the loader: `Api`, `Lib`, `load`, the version checks;
  - the I/O source: `Source`, the callbacks, `free_io`;
  - the wrappers: `Input`, `Stream`, `Packet`, `Decoder`, `Frame`, `Scaler`.

  It is split into `ffi/load.rs`, `ffi/io.rs` and the wrappers.
- **`client/src/text.rs`** has about sixty one-line wrappers, each building one `Message`, so every message has two APIs. `message.rs` keeps three parallel matches per variant: the id, the arguments and the samples.
  - The callers build the `Message` and call `render`, and the wrappers go.
  - A macro makes the three matches from one table per variant.
- **`catalog/src/schema.rs`** is split as M-24 says.
- Gate: `just test`, `just shots`.

### M-48: public items nothing uses, and the survey kept

- **Dead.** `render::display::decode_stored` has no caller anywhere. Its doc says "what a lossless crop cuts", and the crop does not call it. It is deleted.
- **Used by tests only.** Each is made `pub(crate)` or `#[cfg(test)]`, or deleted with its test when the test checks nothing else.
  - `catalog::browse::writes::session_set`: `session_set_many` is the one in use (M-43).
  - `codec::decode::orient`: `pg-edit`'s and `pg-library`'s tests use it, so it goes to `pg-codec`'s `fixtures`.
  - `analysis::pdq::Hasher::hash_rgb`.
  - `model::name::Name::from_wtf8`: `model/tests/name_laws.rs` also uses it. It stays public if that law test needs it, and says so.
  - `ThumbCache::heal`: `heal_if_due` and the writer use the module's `heal`.
  - The platform's and dedup's items, in M-34 and M-41.
- **Kept, with a reason.** `catalog::browse::read::changes::changes_since` is used only by tests today, but `docs/DECODE_ONCE_PLAN.md` phase 4 builds the incremental grid on it.
- **A leftover attribute.** `analysis/src/pdq.rs` keeps a crate-level `#![cfg_attr(not(test), deny(clippy::print_stdout, ...))]`, left from when it was a crate. It goes; `lib.rs` has it.
- **The survey is kept.** `review/api_surface.py`, `edges.py` and `dupes.py` go to `tools/modgraph/`, run by `just modgraph-report`, so the next review starts from them. They are reports, not gates: they match by name.
- Gate: `just lint`, `just test`.

### M-49: the documents

- **`crates/README.md`:** the `platform` entry runs into the next one ("…with the exceptions listed there.- `trace`"), and needs a line break.
- **`app/src/lib.rs`:** its header says the app is "what the front ends share", which M-4b made untrue. It says what the README says: "the application layer the window uses".
- **The Linux volume.** Since `94a2530` it is identified by its file system's UUID (`pg_platform::dir`). Two texts say otherwise:
  - `pg_model::ids::FileIdentity` still calls it an "open item".
  - `docs/STORAGE.md` says the identity is "`(st_dev, st_ino)`".
- **`scan/src/lib.rs`:** its header describes the Windows listing (`GetFileInformationByHandleEx`, "PLAN §8 named an Everything-based scanner"), in a crate that does not list files itself.
- **`model/src/lib.rs`:** its header names only some of its modules.
- **`desktop/src/picker.rs`:** `start_for`'s doc describes the shell's refusals, which are `pg-platform`'s now.
- **`desktop/src/command_line.rs`:** `args` has a doc line that belongs to another function ("The part of `arg` after its first `=`").
- **Fixed in their own steps:** the `pg-schema` header (M-24), `render/src/display.rs` (M-27), the `pg-writeback` `Cargo.toml` comment (M-38) and the jobs header (M-45).
- **After the other steps:**
  - the README's `app`, `library`, `crop`, `thumbcache`, `render` and `client` entries;
  - the layer file;
  - a ledger row per step in `docs/REFACTOR_PLAN.md`;
  - a "Validation 4" section in `MODULARITY_PLAN.md`.
- Gate: `just lint` (markdownlint).

## Order

1. **M-21**, the bugs, with their tests.
2. **M-34** (the typed refusal), then **M-31** (the helpers move down), then **M-22** (one follow-up and one add-folder, for both front ends).
3. **M-29**, **M-30** with **M-27**, then **M-33**: the app's services, `pg-render`, and the Qt conventions. Each gets `just shots`.
4. **M-26**, **M-32**, **M-43** and **M-35**: the words, the client's logic, the session, and the image scheme.
5. The catalog, with the perf gate: **M-37**, **M-42**, **M-24**, then **M-44**. Then **M-45** (the names), once the catalog's modules are settled.
6. **M-23**, **M-25**, **M-28**, **M-36**, **M-38**, **M-39**, **M-40**, **M-41** and **M-47**, independent of one another.
7. **M-46**, **M-48**, then **M-49**, and the Windows and Linux validation of the whole series.

## Open decisions

None block a step. These are confirmed at their step:

- **M-29.** Whether the search's `Factory` fits in `Config` as it is. It is an `Arc<dyn Fn>`, so `Config` may need to stop deriving what a closure cannot.
- **M-30.** Whether `pg-app` drops `pg-codec` entirely: `turn_jpeg` may keep it.
- **M-32.** Whether the facet counts belong in the client's controller state or beside it. What `controller.rs`'s `CORE` already holds decides.
- **M-42 and M-43.** The migration that converts the queue's values and the session's key-value rows: a new `TO_n`, with `BASELINE` frozen.
- **M-25.** Which kind a PNG with an `acTL` after its IDAT is. The container's rule decides it, and the test pins it.
- **M-39.** Whether the decodable formats are passed into `pg-yandex` or moved to `pg_model::format`. The codec's dependency on the model decides it.
- **M-44.** The size of the change, which is every caller of the free functions. It is counted at the step and done in one sitting (ground rule 7).
