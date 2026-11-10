# Modularity plan, Review 4a (2026-11-10, at `0ed0fbe`): the full read

This file holds the second half of Review 4 of [MODULARITY_PLAN.md](MODULARITY_PLAN.md): the full read, its steps M-33 to M-48, its additions to the steps there (M-12a, M-13, M-21, M-22, M-25, M-26, M-30 to M-32), and the order of the whole Review 4 series. The ground rules and gates are the plan's.

Review 4 read the crate roots, the desktop's Qt-free functions and what its scripts flagged. This pass read every production file of every crate (the code before the first line-start `#[cfg(test)]`), bottom-up, from the leaves (`container`, `model`, `schema`, `testkit`) to the front ends (`cli`, `gallery`, `desktop`). Tests were skimmed. A few files were outlined (signatures and doc comments read, bodies only where a finding needed them): `client/src/text/message.rs` (one enum and its three matches), the codec's format modules, `dedup/src/pdq_index.rs`, the metadata modules below `lib.rs`, the platform backends (`dir/{windows,unix}.rs`, `file`, `names`, `native`, `picker`, `process`), the catalog's `fixture`, `video/src/ffi.rs`, `analysis/src/pdq.rs`, the desktop's 26 bridge files and 18 state files (checked: no function bodies except `Default` and the gallery state's helpers), and `bench/benches/ir/catalog.rs`.

**Read in full:** `container`, `model`, `schema`, `testkit`, `client`, `platform`, `codec`, `dedup`, `metadata`, `query`, `catalog`, `edit`, `exiftool`, `jobs`, `jobs-testkit`, `trace`, `video`, `analysis`, `render`, `scan`, `thumbcache`, `borders`, `import`, `thumbs`, `writeback`, `crop`, `yandex`, `library`, `app`, `cli`, `gallery`, `desktop`, `bench`: all 33 crates under `crates/`.

**Checked and settled during the read:**

- `edit/src/rotate.rs` writes EXIF tag 0x0112 only in its tests; the production code stops at line 85.
- `app/src/yandex/replay.rs` follows the client's flow. It answers through `pg_yandex`'s own `parse_upload`, `parse_other_sizes`, `parse_similar_page` and `read_similar_pages`, and checks `is_captcha_page` on every page.
- Parts of M-1, M-6, M-13 and M-14 are already in the tree:
  - `alert::error_box` and `log.rs`.
  - `deadline::owner`/`arm` and the shell's `timeout` invokable.
  - `pg_platform::console::attach_to_parent`, `names::after_first_equals`, `names::arg_bytes` and `file::play_target`.
  - `pg_testkit::odd_name`.
- Cohesive, with no finding: `container` (except one copy, in M-41), `exiftool`, `trace`, `jobs-testkit` and `bench`.

## M-33: the bugs the read found

Each fix gets a test that fails before it.

- **A temp file reached by its plain path.** Both copies of the scan follow-up delete a replaced file's queued-write temp with `std::fs::remove_file(path_buf(..))`: `library/src/runner.rs` (`Scans::run`) and `cli/src/lib.rs` (`run_command`, `Scan`). The writer reaches files only by `verbatim(..)`. On Windows a plain path drops a trailing dot or space from a folder name, so a temp in such a folder is either missed or another folder's file is deleted. Fix: `verbatim`, in M-21's one `after_scan`. Test: a temp under a folder named `a.` is deleted, and its neighbour in `a` is kept.
- **An unreadable preferences file overwrites the last one kept.** `app/src/store.rs` (`quarantine`) renames the file to `<name>.bad`. A rename replaces the target on both systems, so the `.bad` an earlier failure kept is lost, while the doc says it "is kept". Fix: `pg_schema::move_aside`, which numbers them (`.bad.1`, ...). `store::with_suffix` (a fifth `beside`) goes with M-42.
- **Two notes of the wrong kind.** `App::open` reports a thumbnail cache that cannot be opened as `Note::Thumbs(Event::NotStored { error: "cache disabled: .." })`; that event means one on-demand thumbnail was not stored. `App::shutdown` reports a catalog that fails to close as `Note::Thumbs(Event::WalNotMerged { error: "catalog: .." })`, the cache's WAL event. Both front ends word a note by its variant, so the user reads the wrong sentence. Fix: `Note::CacheDisabled { error }` and `Note::CatalogNotClosed { error }`, each with its own words.
- **A filter's shapes shown untranslated.** `client/src/text.rs` (`describe`) writes a chosen shape by its key (`o.key()`, English) where the bar uses `shape_label`. Fix: `shape_label`. Test: the pseudo-locale's description of a query with a shape has no English word.
- Gate: Windows `just test`, Linux checks.

## M-34: the folder-refusal policy belongs to the scanner, and its words to the client

This extends M-27.

- **Found.**
  - `pg-platform` decides which folders the app accepts:
    - `dir::check_ntfs` (decision 6's steps) and `check_drive`.
    - The Linux `REFUSED` table of file systems.
  - It also words the reasons, in English ("it is on an NFS mount", "its drive is removable", ...).
  - The words reach the window inside a Fluent message (`text::root_refused(root, reason)`), so they are never translated or pseudo-localized.
  - `pg_scan::Listing::check_mount` returns the refusal as `Result<(), String>`.
  - The crop and the write-back declare the same refusal twice. `pg_crop::CropError::FolderRefused { file, folder }` and `pg_writeback::WriteError::FolderRefused { file, folder }` have the same English `Display` and each repeat the `check_folder_names` mapping.
- **Fix.**
  - `pg-platform` reports facts, as a typed value:
    - the drive type;
    - the file system's name or `f_type`;
    - a reparse point;
    - the real volume.
  - `pg-scan` holds the policy table, and returns `Refused { reason: RefusedReason }`.
  - The client words each `RefusedReason`.
  - `check_mount` returns `Result<(), RefusedReason>`.
  - One `FolderRefused` type, built from `pg_platform::native::AlteredFolder`, is carried by both `CropError` and `WriteError`.
  - `RootFacts`, `check_ntfs`, `check_fs_type` and `NativePath::set_plain` are public today only so tests can reach them. They become `pub(crate)`, with the tests beside them.
- Gate: Windows `just test`, Linux checks (the refused-mount tests), `just shots` (the pseudo-locale).

## M-35: one name per concept

- **The list facets are three enums.**
  - `pg_model::query::ListFacet` is in the model's order: Types, Exts, Ratings, Shapes, Status, Borders. It is used only inside the model (`Query::chosen`, `Choice::facet`).
  - `pg_client::facets::Facet` is in the bar's order and spells shapes as `Orientations`.
  - `pg_query::Facet { Types, Exts, Borders }` lists the facets that are counted without their own values.
  - Facet and choice are mapped three times: `pg_client::input::choice`/`key_of` and the model's `Choice::facet`.
  - Fix:
    - Keep one enum, the model's.
    - The client keeps only the bar's order over it (`ALL`) and the choices' order per facet. That order is `facet_choices::choices` today, which M-26 moves.
    - `pg_query`'s enum becomes `Counted`.
    - One mapping is left: `Choice::facet`.
- **The cache's start-over reason is three types.**
  - The types: `pg_thumbcache::StartOverReason`, `pg_client::text::CacheStartOver`, and the re-export `ThumbStartOverReason`.
  - Each front end maps one to another (`cli::report::started_over_reason` and the desktop's copy).
  - Fix: one enum in `pg-model`, which both the cache and the client reach. The mappings go.
- **`MediaType` is the grid's too.** `pg_model::grid::Entry::media_type` uses `query::MediaType`. Fix: it moves beside `media::Kind` (`model/src/media.rs`), and `query` uses it from there.
- **One method, two names.** `pg_model::orientation::turns_swap(turns)` is `turns.swaps_axes()`. Fix: keep `Turns::swaps_axes` and change its 5 callers (`desktop/src/gallery/edits.rs` and `viewport.rs` among them).
- **Two `Prefs`.**
  - `pg_client::controller::prefs::Prefs` is the grid's size.
  - `pg_app::Prefs` is the preferences file.
  - Fix: rename the client's to `size::GridSize`.
- **Two modules named `writes` in the catalog.**
  - `crate::writes` is the file-write queue.
  - `browse::writes` holds the views and the session.
  - Fix: rename them `queue` and `browse::views` (with M-28).
- **A failed file is recorded two ways in `pg-jobs`.**
  - `failure::Failure { path: String, error: String }` is a pass's stats, with 6 users.
  - `FileError { path: String, cause: Cause }` is the app's notices.
  - Fix: one `Failure { path, cause: Cause }`.
- **The jobs header says the opposite of the code.** `jobs/src/lib.rs` says "Nothing here knows what a pass does to a file". But `control::Phase` names the library's passes (`Crop`, `Scan { root: String }`, `Import`, `Borders`, `Thumbs { height }`), and `events::Change` names the runner's and the writer's getters. They are the vocabulary that scan, thumbcache, writeback and library share, and `pg-jobs` is the one crate all four reach. Fix: keep them there and correct the header. `Scan { root }` becomes a `RulePath`, not a display string.
- **`ScanNote::kind() -> &'static str`.** It is a second key for a typed enum (`pg_library`). Fix: the app words the enum directly, and `kind` goes.
- Gate: `just test`, `just qmltest`, `just shots`.

## M-36: typed values across the catalog's API

- **Found.** Values cross the catalog's API as text or raw integers:
  - **The write target.** `write_target` returns `(RulePath, Option<String>, String)`: the format and the kind are text. `writes::rating_in_file_possible(kind: &str, format: Option<&str>)` takes the same text; `set_rating` and `browse::read::details` call it.
  - **The fresh file and the folder's extensions.** `Fresh { ext: String, kind: String }`; `file_source` compares `kind == Kind::Video.key()`; `Facets::exts` is a `Vec<(String, i64)>`.
  - **The write queue's value.** `Job { kind: WriteKind, value: i64 }` is "the rating, or the quarter turns". `temp` is an `Option<(RulePath, i64)>`, and `done` checks with `debug_assert!`s that kind and value agree. `pg-writeback` converts the value at every use (`u8::try_from(job.value)`, `job_turns`' `rem_euclid`, `rewrote`'s `clamp(0, 5)`), and `WriteEffect::FileRating { rating: u8 }` carries it out.
  - **An unknown rewrite** is `Rewrite::before` set to `(-1, -1)`.
  - **Scan ids are raw `i64`s**: `begin_scan -> i64`, `finish_scan(i64)`, `mark_missing(.., scan: i64, ..)`, `ScanWriter`, `after_scan`.
  - **An unknown kind is silently an image.** `lifecycle::work_list` maps an unknown `files.kind` to `Kind::Image`, while `entry_from_row` fails on an unknown extension.
  - **rusqlite in the public API**:
    - `Catalog::release_memory -> rusqlite::Result`;
    - `ScanWriter::new`, `AnalysisWriter::new` and `BordersWriter::new`, which are `pub` and take a `rusqlite::Transaction` (made only through `Tx`, which is `pub(crate)`);
    - `max_folder_id(&Connection, schema: &str)` (M-29).
  - **Stored keys are spelled as literals.** `files.kind` and `file_writes.kind` values (`'video'`, `'image'`, `'rating'`, `'rotation'`) are string literals in:
    - the catalog: `writes`, `writers`, `pair`, `browse::read::{grid,state}`;
    - `query/src/lib.rs`;
    - `thumbcache/src/reader.rs` (M-29).
  - **`pg_metadata::Meta::rating`** is an `Option<u8>`, though `pg_model::rating::Rating` exists and the analysis holds an `Option<Rating>`.
- **Fix.**
  - Each value is parsed once, at the row:
    - `write_target` returns `WriteTarget { path, format: Option<Format>, kind: Kind }`.
    - `rating_in_file_possible(Kind, Option<Format>)`, beside M-17's `can_hold_rating`.
    - `Fresh { ext: Ext, kind: Kind }`.
    - The queue's value is `enum Write { Rating(Option<Rating>), Turns(Turns) }`, in the job and in the temp. The `debug_assert!`s and the conversions in `pg-writeback` go.
    - `Rewrite::before` is an `Option`.
    - `ScanId` goes in `pg_model::ids`, beside `FileId`, `FolderId` and `RootId`.
  - An unknown kind is an error, as an unknown extension is.
  - The writers' constructors become `pub(crate)`, made by `Tx`'s methods.
  - `release_memory` returns the catalog's `Result`.
  - Every stored key is a `pg-model` macro, as `min_side!` and `group_distance!` already are. The catalog's SQL, `query` and the cache all reach `pg-model`; `query` and the cache do not depend on the catalog's `sql.rs`.
  - `Meta::rating` is an `Option<Rating>`.
- Gate: `just test`, Linux checks, perf (the import and the catalog are benched).

## M-37: the Qt boundary's conventions stay in the Qt crate

- **Found.** Qt's int conventions, where -1 means none, appear in the crates without Qt.
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
    - `Error::UnknownFile(i64)` and `Error::Rewriting(i64)`.
  - **`pg_app::images::Upload` is Qt's scene-graph texture format** (`QSGPlainTexture`, RGBX8888, Qt's `qPremultiply` rounding), and its doc names Qt.
  - **The desktop passes text and ints where it has types:**
    - `gallery::items::border_rejected(item, "top")` takes the edge as text, not `detection::Edge`.
    - `keymap::media_of` reads `GalleryModel.MediaKind` as an int, by its position, though the bridge declares the enum.
    - `viewport::swaps_axes` turns an `i32` into `Turns` by hand (`rem_euclid`).
- **Fix.**
  - `pg-client` and `pg-app` use `Option<usize>`, `Option<FileId>`, `FileId`, `Rating` and the model's dimension type.
  - The desktop converts at the bridge, in one helper per direction (`row_of(i32) -> Option<usize>`, `to_row(Option<usize>) -> i32`).
  - The texture conversion moves to the desktop. `pg-render` keeps a premultiplied RGBA image named for what it is, without Qt (with M-23).
  - The desktop's own calls take `Edge` and the bridge's `MediaKind`. One `Turns` constructor from any quarter-turn count, in `pg-model`, replaces the `rem_euclid` at each bridge.
- Gate: `just test`, `just qmltest`, `just shots`.

## M-38: the session is a typed `Listing`, stored by the catalog

- **Found.**
  - The session is a store of text keys and values (`session_get`, `session_set`, `session_set_many(key: &str, value: &str)`). Its keys and encodings are in `app/src/views.rs`: `folder` as JSON, `recursive` as `"true"`, `query`, `sort`.
  - The catalog's rebuild knows the `folder` key and its JSON shape too (`schema::convert_text_folders`). So the session's format is written in two crates.
  - Saved views are already stored typed, a column per field.
  - `pg_model::view::SavedView` declares `Listing`'s four fields again, plus `name`. A `Listing` is rebuilt from one field by field in three places: `client::controller::listing` (`ViewApplied`), `desktop/src/controller.rs` (`start`) and `desktop/src/shell.rs` (`save_view`).
- **Fix.**
  - `SavedView { name, #[serde(flatten)] listing: Listing }`, with no field copies.
  - The catalog stores the session's `Listing` as it stores a view: `session_listing()` and `save_session_listing(&Listing)`, by column.
  - `app::views` and `convert_text_folders` lose their key knowledge. The migration converts the key-value rows once.
  - `session_set` goes (M-31 listed it as used by tests only).
- Gate: `just test`, `just shots` (the start restores the listing), Linux checks.

## M-39: the `image://` scheme in one place

- **Found.**
  - `pg_model::image` builds the `thumb`, `thumbhash` and `full` URLs. That is front-end protocol inside the domain model.
  - `desktop/src/yandex.rs` builds `image://yandex/{id}` itself (`picture_url`, `original_url`).
  - `desktop/src/providers.rs` writes the four provider names a third time (`provider`), and parses the Yandex request (`<id>` or `<id>/original`) itself.
- **Fix.**
  - The key and the view (`ImageKey`, `View`) are the app's interface and stay in `pg-model`.
  - The URL half moves to `pg_client::image`:
    - `ImageRequest` gains `Yandex { id, original }`;
    - `provider` and `parse` are the one name map.
  - The desktop builds and parses every URL through it.
- Gate: `just test`, `just shots` (every provider).

## M-40: `Layout` names every file of the data folder

- **Found.**
  - `library/src/layout.rs` says that "the layout lives in one place". It does not:
    - `thumbs.sqlite` is named in `pg_thumbcache::config::cache_path`.
    - `catalog.sqlite` is named in `pg_library::catalog_path`.
    - The data folder's name is in `pg_platform::dirs::default_home`.
    - The SQLite alias folder is in `pg_platform::sqlite::alias_of_folder`: `%LOCALAPPDATA%\PhotoGallery\sqlite-aliases`, then `ProgramData`.
  - The alias folder ignores `PG_HOME` and the layout. So a run with a scratch home still writes under the user's profile whenever a catalog folder's name is not UTF-8. That breaks the A5 rule (nothing of ours outside the data folder).
  - `pg_app::Config` copies `Layout`'s four paths field by field.
- **Fix.**
  - `Layout` names every file: `catalog_path`, `thumbs_path` and `sqlite_aliases` (`home/sqlite-aliases`). Each is passed down. `pg_platform::sqlite` takes the alias folder as an argument.
  - `Config` holds the `Layout`.
  - Test: with `PG_HOME` set, opening a catalog in a folder with a non-UTF-8 name writes nothing outside it.
- Gate: Windows `just test` (the alias path is Windows'), Linux checks.

## M-41: the container formats are written where they are read

- **Found.**
  - **Whether a file is animated is decided twice.**
    - `edit`'s `rotate` and `crop` look for any `acTL` chunk (PNG) or `ANIM` chunk (WebP); `rotate` also looks for `VP8L`.
    - `pg-metadata`'s `container.rs` counts frames: from an `acTL` before the image data, and from `ANMF` chunks.
    - So a PNG with an `acTL` after its IDAT is a still to the metadata and an animation to the edit.
  - **The container writers are outside `pg-container`:**
    - the PNG chunk and its CRC: `edit::rewrite::chunk::png_chunk`, `metadata::orientation`'s eXIf CRC fix, and test helpers in `codec` and `metadata`;
    - the RIFF header and the VP8X canvas fields (`rewrite::webp`);
    - the GIF sub-blocks (`rewrite::gif::sub_blocks`), plus a second sub-block reader for the LZW data.
  - **The GIF colour-table size** (`3 << ((packed & 7) + 1)`) is written three times: `pg_container::gif_header_len`, `GifBlocks::block`'s 0x2C arm, and `rewrite::gif`.
- **Fix.**
  - One rule, `pg_container::animated(bytes, format)` (and `webp_lossless`), used by the edit and the metadata.
  - `pg-container` gains the writing half: `png_chunk`, `fix_png_crc`, `riff` and `gif_sub_blocks`.
  - `GifBlock::Image` carries the parsed descriptor: place, size, packed byte, table length, code size.
  - The SQL rule (a still is an image of one frame: `pg_query::media_sql`, `MEDIA_CASE`, `pg_catalog::sql`'s analysable stills) reads `analysis.frames`, which is the metadata's count. It agrees once the edit uses the same rule.
  - Test: a PNG with an `acTL` after its IDAT is the same kind to both.
- Gate: `just test`, the fuzzer's container target (Linux).

## M-42: a database file's lifecycle is `pg-schema`'s, and the catalog's `schema.rs` is split

- **Found.**
  - **The cache repeats the catalog's file lifecycle:**
    - the lock;
    - the judgement against an expected schema held in memory (a `OnceLock<Schema>` in each);
    - moving a file aside (`move_damaged` against `rebuild`);
    - the shared and exclusive holds (`hold_shared` against `take_shared`), worded alike;
    - deleting a database and its sidecars: `thumbcache`'s `schema::open_once` loops over `["-wal", "-shm", "-journal", ""]`, the catalog has `lifecycle::remove_database`.
  - **`beside` is written five times:** `pg_schema` (where `Lock::take` inlines it again), `catalog/src/lifecycle.rs`, `thumbcache/src/schema.rs`, and `app/src/store.rs` (`with_suffix`).
  - **`pg-schema`'s name and header say "schema" only**, though it holds `Lock`, `Live` and `move_aside`.
  - **The catalog's `schema.rs` holds three concerns:**
    - the schema: `BASELINE`, `MIGRATIONS`, the versions, `expected`, `mismatch`;
    - the rebuild's row copy: `carry_rows`, `copy_tables`, `convert_text_folders`, `table_columns`, `column_type`;
    - `write_transaction`.
  - **`lifecycle.rs` holds the rest of the rebuild, and more:**
    - the rebuild's file side: `rebuild`, `make_replacement`, `remove_database`, `stored_under`;
    - the locks and the version marks;
    - the work lists: `pending_import_sql`, `work_list`, `max_folder_id`.
- **Fix.**
  - `pg-schema` gains `remove_database` (with the sidecars) and a public `beside`. The catalog, the cache and the app's store use them. (M-30's `beside` item is this.)
  - Its header says it holds a SQLite file's lifecycle.
  - The catalog is split into `rebuild.rs` (both halves of the rebuild), `lifecycle.rs` (locks, holds, marks), `work.rs` (the work lists) and `schema.rs` (the schema). This is M-12a's answer for `catalog/src/schema.rs`.
- Gate: `just test`, Linux checks, perf.

## M-43: one pass skeleton for the per-file passes

- **Found.**
  - `pg-borders`, `pg-import` and `pg-thumbs` each write the same skeleton:
    - the work list, then `ctl.begin`;
    - a `batch` of `(index, result)`, flushed every `BATCH` items in a catalog transaction with the pass's writer;
    - `pipeline::run`, whose read closure is `let [folder, name] = work.parts(item.path); set_file(wide, folder, name)` and whose sink is `ctl.add`, `batch.push`, then the flush.
  - The path FFmpeg opens a video by (`verbatim(&path_buf(&work.path(item.path, sep)))`, with its comment) is written in both `import` and `thumbs`.
  - The thumbnail and crop passes show failure paths with `String::from_utf8_lossy` of the stored bytes, where `PathRules::display` is the rule.
  - The rule "the file is still as listed, and the app is not writing to it" is written three times: in Rust in `thumbs` (`file_state`: mtime, size, rotation, writing), and in SQL in `AnalysisWriter::current` and `BordersWriter::current`.
  - `pipeline::run` reports a worker whose setup failed by sending the index `usize::MAX` on the results channel.
- **Fix.**
  - `pg_jobs::pipeline::batched` takes the read, the work and the flush. The three passes become their work and their writer.
  - `pg_platform::names::verbatim_of(&WorkList, FilePath)`.
  - `PathRules::display` for every shown path.
  - The still-as-listed rule is one SQL fragment in `catalog/src/sql.rs`. `thumbs` reads the result instead of comparing again.
  - The results channel carries an enum: `Done(index, result)` or `SetupFailed(error)`.
- Gate: `just test`, Linux checks, perf (the per-file benches).

## M-44: `pg-writeback`'s dependencies

- **Found.**
  - `pg-writeback` depends on `pg-analysis` for one function, `analyze::file_hash` (BLAKE3 of a file's bytes). That edge brings in the analysis layer of the codec, video (FFmpeg) and metadata crates.
  - Its `Cargo.toml` comment lists its dependencies as "the edit, the metadata and ExifTool". It also has analysis, catalog, jobs and thumbcache.
  - `WriteStatus::thumbnail_error: Option<String>` is English made here (`"file {id}: {e:#}"`).
- **Fix.**
  - `file_hash` moves to a place both reach without the analysis: `pg_model::analysis::FileHash`, with the `blake3` call in `pg-metadata`. `pg-writeback` loses `pg-analysis`, and the layer file changes with it.
  - The comment states the dependencies as they are.
  - `thumbnail_error` is a typed `(FileId, Error)`, which the client words.
- Gate: `just lint` (the layer check), `just test`.

## M-45: `pg-yandex`'s error holds only the client's cases

- **Found.**
  - `YandexError::Undecodable` and `YandexError::Recorded` are made only by `pg-app` (`yandex/images.rs`, `yandex.rs`, `yandex/replay.rs`). The variant's own doc says "Never made by the client".
  - `pg_yandex::sniffed` repeats which formats the app decodes (JPEG, PNG, WebP, GIF, BMP). The codec's list is the rule.
- **Fix.**
  - The app's `Source` returns its own error, `pg_app::yandex::SourceError { Yandex(YandexError), Undecodable, Recorded }`, and `pg-yandex` drops the two variants.
  - `sniffed` takes the codec's list. `pg-yandex` depends on `pg-model` alone, so the list is either passed in or moved to `pg_model::format`, with the codec reading it from there.
- Gate: `just test`, `just shots` (the `yandex` scenarios).

## M-46: one numeric conversion, one fit, one decode cap

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
  - **Video timing conversions are written twice each in `pg-video`:** the display width from the sample aspect ratio (`probe`, `to_rgb`), and a stream timestamp to milliseconds (`probe`'s `in_ms`, `frame_into`'s `ms`).
  - **`cli/src/borders.rs` declares `MAX_PIXELS`** as `(512 << 20) / 3`, a copy of the decoders' cap. `pg_codec::max_decode_bytes()` is the rule.
  - **`DECODE_MARGIN`** is declared in both `pg_app::images` and `pg_render::maker`, with the same value and the same comment.
  - **The FILETIME epoch is declared twice:** `pg_query`'s `filetime_unix_offset!` and `pg_platform::file::UNIX_EPOCH_TICKS`.
  - **BigTIFF is detected twice:** `pg_codec::encode::tiff_samples_cropped` and `pg_model::format::sniff`.
- **Fix.**
  - `pg_model::num` gets the clamping conversions (`clamp_to_i32`, `clamp_to_u32`, `clamp_to_i64`, `clamp_to_usize`), and all eight sites use them.
  - `MinSize::fit_scale` in the codec serves all three fits (with M-23).
  - The two video conversions become methods: `Stream::display_width` and `Stream::ms`.
  - `MAX_PIXELS` is computed from `max_decode_bytes()`.
  - `DECODE_MARGIN` is declared once, in `pg-render` (M-23 moves the app's copy).
  - The FILETIME epoch is `pg_model`'s macro, used by both crates.
  - BigTIFF detection is `pg_model::format`'s.
- Gate: `just test`, perf.

## M-47: one JPEG encoder, and the maker decodes through `display`

- **Found.**
  - turbojpeg's `Compressor` is set up twice: in `Maker::new`/`encode_image` at q85 with 4:2:0 subsampling, and in `transcode::to_jpeg` at q95 with 4:4:4. Each also writes its own `Image { pixels, width, pitch: width * 3, height, RGB }`.
  - The codec owns decoding (and `encode::cropped`), but not this encoder.
  - `render/src/display.rs` says it is "the one place that reads a file's EXIF orientation for decoding". `Maker::make_image` calls `pg_metadata::orientation` and `Decoder::decode_into` itself.
- **Fix.**
  - `pg_codec::encode::jpeg(&Rgb, quality, subsampling)`, used by both.
  - The maker decodes through `display::decode_into`, into its own buffers, so the claim in `display.rs` becomes true (with M-23).
- Gate: `just test`, perf (`thumb_jpeg`), `just shots`.

## M-48: an item lives with its only user

- `pg_model::query::parse_number` reads typed input (a comma or a point). `query.rs`'s header says the front end reads text typed into a filter. Its users are `client::input` and `desktop::filters`. It moves to `pg_client::input`.
- `pg_model::date::exif_date` is EXIF's text format (kamadak-exif's rules), and only `pg-metadata` uses it. It moves to `metadata/src/exif.rs`; the model keeps `wall_clock_seconds`.
- `pg_thumbcache::config::parse_cap_gb` reads the command line's `--cap-gb` and the app's `PG_THUMB_CAP_GB`, and makes English errors in the cache crate. The parse moves to `pg_client::input`, which the command line can use, and the cache takes bytes.
- `pg_dedup`'s `pdq_index` module and `PdqIndex` are public, but used only through `Pairer` (and `IndexFull`). The module becomes private.
- `pg_analysis::analyze::video(path, turns, &mut Analyzer)` becomes `Analyzer::video`, beside `Analyzer::analyze`.
- `pg_scan::child_path`, which extends a `RulePath` by a name, becomes a `RulePath` method.
- `pg_crop::place_of` reads the anchor's bytes and drops `:` and `\` (the Windows anchor). It becomes a `PathRules` method.
- `cli::rule_path_of` (with the rules) sits beside `pg_platform::names::rule_path_of` (native). It becomes a `PathRules` method, used by both.
- Gate: `just test`.

## Additions to M-12a, M-13, M-21, M-22, M-25, M-26 and M-30 to M-32

- **M-12a.**
  - `video/src/ffi.rs` (779 production lines) is one layer, the safe wrapper of FFmpeg's C API, in three parts:
    - the loader: `Api`, `Lib`, `load`, the version checks;
    - the I/O source: `Source`, the callbacks, `free_io`;
    - the wrappers: `Input`, `Stream`, `Packet`, `Decoder`, `Frame`, `Scaler`.
  - Decision: split it into `ffi/load.rs`, `ffi/io.rs` and the wrappers.
  - `catalog/src/schema.rs` is split as M-42 says.
  - `client/src/text.rs` has about sixty one-line wrappers, each of which builds one `Message`, so every line has two APIs. `message.rs` keeps three parallel matches per variant (id, arguments, samples). Decision: callers build the `Message` and call `render`, and the wrappers go. A macro generates the three matches from one table per variant.
- **M-13.**
  - Done: `attach_console`, `value_exact`, `arg_bytes` and `video_source`.
  - Left: `desktop/src/picker.rs` still has `cfg(windows)` on `native` and `run_dialog`, and a `not(windows)` stub. Fix: `pg_platform::picker::pick(start, owner) -> io::Result<Option<PathBuf>>` on every system, which returns `Unsupported` off Windows; the desktop's three items go. The in-app dialog's listing (`folders_in`) moves to `pg_platform::dir`.
  - `pg_platform::picker::start_folder` reads `USERPROFILE`/`HOME` directly. It takes the same lookup `dirs` takes.
  - `pg_platform::names` also holds the command line's argument bytes (`arg_bytes`, `after_first_equals`). They move to their own `args` module.
- **M-21.** Confirmed by the read, with these additions:
  - The CLI's copy also lacks the `VolumeFallback` line.
  - The plain-path deletion is M-33's first bug: `after_scan` uses `verbatim`.
  - `LibraryService::remove_folder` stops a running scan by queuing an empty one (`library.scan(Vec::new())`). It becomes a `Runner::stop_scan` that says what it does.
- **M-22.**
  - The desktop's other process-wide state is per-QObject state that a sibling QObject reads. It is kept, because cxx-qt default-constructs the QObjects and nothing passes them a reference. Each gets a line in its module saying so:
    - `controller::CORE`, `gallery::MODEL` and `folders::TREE`, which are thread-locals;
    - `picker`'s `CHOSEN`, `LISTING` and `START`.
  - `facet_choices::COUNTS` goes in M-26.
  - `app/src/yandex/images.rs` (`grid_picture`) keeps a thread-local `Maker`, a second pool of makers beside `makers::Makers`. It uses the app's pool.
  - The Yandex grid picture's height (220) is written three times: `yandex::images::GRID_HEIGHT`, the panel's `ROW_HEIGHT`, and the default of `Prefs::row_height`. The panel's row height is the desktop's, so the desktop passes the height to the search.
  - `reader::Session::submit` carries a panic and an open failure as `pg_catalog::Error::Io(io::Error::other(..))`, the catalog's error used for the app's cases. They become variants of the reader's own error.
  - The change pump is written twice (`desktop/src/library.rs` and `desktop/src/yandex.rs`, `ensure_pump`). There is to be one helper.
- **M-25.**
  - Confirmed: `desktop/src/app.rs` (`note_text`) and `cli/src/report.rs` (`thumb_event_text`) are word for word the same.
  - Two more groups of English outside Fluent go the same way:
    - the key hints in `client/src/keys.rs` (`SELECT`, `VIEW`, ...), which then show in the pseudo-locale's clipping check;
    - the selection line in `desktop/src/gallery/items.rs` (`refresh_derived`, `format!("{}   {symbol}")`).
- **M-26.** More to move:
  - The combo-box index of a yes/no/either choice is mapped four times in `desktop/src/filters.rs`: `duplicates_index`, `review_index`, and the matches in `set_*_index` that reverse them. It moves to `pg_client::input` (`tri_index`, `tri_of_index`), where `Facet::from_index` and `Field::from_index` already are.
  - `edits::set_rating` clamps an `i32` that `Rating::try_from` should read.
  - `Filters::sync` writes three compare-and-set blocks by hand, where the bridge's generated setters do the same.
- **M-30.** More duplicates:
  - In the catalog:
    - `stored_under` and `check_rules` both read `meta.path_rules` and word the same two reasons: one function.
    - `set_import_version`, `set_borders_version` and `set_taken_zone` have one shape: read the mark, start an immediate transaction, read again, set, recompute. They become one `set_mark(key, value, recompute_sql)`.
    - The change-stamp trigger body is spelled 13 times in `BASELINE` and `TO_2`. A `stamp!(new|old)` macro replaces it, only if its expansion is byte for byte the same: the baseline is frozen and compared as text.
    - The ratings upsert is written in `AnalysisWriter::new` and in `writes::set_rating`. It becomes a constant, like `SET_FILE_RATING`.
    - `pair::rebind` and `pair::replace` write the same 11-column UPDATE.
    - The border colour's 0xRRGGBB packing is in `writers::store_borders`, its unpacking in `details::border_sides_on`. `detection::Side` owns both.
    - `fixture::raw::{add_scan, add_folder}` repeat `fixture::state::{add_scan, add_folder}`.
    - `facets` sizes its sums with a literal 5. It uses `BorderFacet::COUNT` (strum).
    - `pg_query`'s `MARKED` repeats the cut sum in `bordered!`.
  - In the thumbnail cache:
    - Seven waiting calls each repeat `channel(); send(Op::X { reply }); recv()`. One `call` helper replaces them.
    - The upsert that forgets a height's watermark is written twice: `evict` and `forget_height_entry`.
    - `maintenance::Drift` is a tuple of tuples. It becomes a struct.
  - In the desktop, `screenshot::schedule` parses `--wait` the way `command_line::number_of` does. It calls `command_line::number_of`.
- **M-31.** More public items used only by tests:
  - `pg_platform`'s `RootFacts`, `check_ntfs`, `check_fs_type` and `NativePath::set_plain` (in M-34);
  - `pg_dedup::pdq_index` (in M-48).
  - `analysis/src/pdq.rs` still has a crate-level `#![cfg_attr(not(test), deny(clippy::print_stdout, ...))]` as a module attribute, left from when it was a crate. It goes; `lib.rs` already has it.
- **M-32.** More stale text to fix:
  - **Linux volume identity:**
    - `pg_model::ids::FileIdentity` still calls the Linux volume an "open item".
    - `docs/STORAGE.md` (the identity is "`(st_dev, st_ino)`") is stale too.
    - Since `94a2530`, a Linux volume is its file system's UUID (`pg_platform::dir`).
  - **Code headers:**
    - `scan/src/lib.rs` describes the Windows listing (`GetFileInformationByHandleEx`, "PLAN §8 named an Everything-based scanner") in a crate that does not list files itself.
    - `model/src/lib.rs` names only some of its modules.
    - The `pg-schema` header (M-42), the jobs header (M-35) and `render/src/display.rs` (M-47) are corrected in their steps.
  - **Smaller doc errors:**
    - The `Cargo.toml` comment of `pg-writeback` (M-44).
    - `picker::start_for`'s doc describes the shell's refusals, which are now `pg-platform`'s.
    - `command_line::args` has a doc line left over from another function ("The part of `arg` after its first `=`").

## Order of M-21 to M-48

1. **M-33** (the bugs), with their tests.
2. **M-27** and **M-34** (the typed refusal), then **M-24** (the helpers move down), then **M-21** (one follow-up and one add-folder, on both).
3. **M-22**, **M-23** with **M-47**, then **M-37** (the app's services, `pg-render`, the Qt conventions), each with `just shots`.
4. **M-25**, **M-26**, **M-38** and **M-39** (the words, the client's logic, the session, the image scheme).
5. **M-29**, **M-36**, **M-42**, then **M-28** (the catalog), then **M-35** (the names, after the catalog's modules are settled), with the perf gate.
6. **M-40**, **M-41**, **M-43**, **M-44**, **M-45** and **M-46**, which are independent of one another.
7. **M-48**, **M-30**, **M-31**, then **M-32**, and the Windows and Linux validation of the whole series.

## Open decisions

Unverified until their step, and confirmed there:

- M-36 and M-38: the migration that converts the queue's values and the session's key-value rows (a new `TO_n`; `BASELINE` stays frozen).
- M-41: which kind a PNG with an `acTL` after its IDAT should be. The container's rule decides it, and the test pins it.
- M-45: whether the decodable formats are passed into `pg-yandex` or moved to `pg_model::format`. The codec's dependency on the model decides it.
