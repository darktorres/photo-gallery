# The crates: library, import and desktop UI (Rust)

See `docs/plans/PLAN.md` §4–7 and §8 (M1, M2); measurements are in `docs/plans/PLAN.md` (§8, M1). The layering is in
`docs/reference/REFACTOR_DESIGN.md`, and the open refactor work is in `docs/plans/REFACTOR_LEDGER.md`.

Every directory under `crates/` is one package, named `pg-<directory>`. From the bottom up:

- `model` (`pg-model`): the domain data: the analysis record, border detection, EXIF orientation, PDQ, formats, media
  kinds, dates, path rules, and the types the catalog's reads and the front ends exchange (grid, tree, query, ratings,
  turns, writes, image keys and views, saved views and the session's listing), which formats can hold a rating
  (`format::can_hold_rating`, which the catalog's writes and the write-back share), and the outcomes the front ends
  word: a pass's phase and progress (`progress`), why a folder is refused (`refusal`), a Yandex search's and save's
  outcomes (`yandex`), and what the app tells the user
  (`notice`: each kind of notice with its code, level, source, how the inbox keeps it and what can be done from it).
  Pure and WASM-safe: no file, catalog, decoder, clock or platform calls.
- `platform` (`pg-platform`): the operating system behind portable functions: file identity, directory handles,
  priorities, processes, the console, the shared libraries loaded at run time, the folder dialog, verbatim paths, the
  system's units of names and arguments, the path rules this machine's paths follow (`native::PATH_RULES`; `model`
  holds both rule sets as data), the facts about the volume a folder is on (`dir::RootFacts`, `dir::fs_type`, and
  which of them this system's folders are checked by, `dir::ROOT_CHECK`; the scanner decides from them), a path given
  as the catalog keys it (`names::normalize_path`, with a typed error the client words), a file's size and last write
  time (`file::stat`), and message boxes. No other crate's production code names a platform
  (`cfg(windows)`, `cfg(unix)`, `target_os`, `std::os::{windows,unix}`, `windows_sys`, `libloading::os`): the folder
  dialog is `picker::pick` on every system (`Unsupported` where the app's own picker over `dir::folders_in` is used),
  and the video player is handed an open file (`file::player_handle`), the same call on every platform.
  `tools/boundaries.py` checks it. A stored path reaches the disk through `names::disk_path` alone (the verbatim form on Windows, which keeps a
  folder ending in a dot or a space); `names::path_buf` is private to the crate.
- `trace` (`pg-trace`): the startup stages, as plain C-named functions, for measuring them (`docs/reference/PROFILING.md`).
- `perf` (`pg-perf`): the perf log, `PG_PERF_LOG`: timed spans of the background job's rounds, its passes and their sub-steps (time, self time, items, bytes, the pipeline's stage shares, the catalog's write-lock waits), written as JSON lines and read back by `pg-driver perf` (`docs/reference/PROFILING.md`, "Reading the perf log"). Unset, a span costs one flag test.
- `log` (`pg-log`): the app's log, `gallery.log`: JSON lines, capped and rotated, with this run's newest records in
  memory for the window's log view (`crates/log/src/lib.rs`). Only the window writes it; the driver prints to stdout and stderr.
- `container` (`pg-container`): a file's containers walked without allocating: JPEG segments, PNG, RIFF (WebP) and
  GIF chunks, extended XMP packets, as byte ranges; and what the edit and the metadata read of them, written once (a
  PNG's or WebP's animation and frames, a lossless WebP, a GIF's image descriptors, a PNG chunk's CRC put right).
- `dbfile` (`pg-dbfile`): a SQLite database file's lifecycle, which the catalog and the thumbnail cache share, each
  keeping its own policy: the schema check (a file's `sqlite_schema` compared with the one this build creates, comments
  and whitespace ignored, and the seed rows), the judgement of what a file is before it is written (`judge`: new,
  usable, another layout, another program's, damaged; the expected schema built once, `Expected`), the lock that
  serializes a file's changes between processes and the holds on an open file, and its move aside and deletion with
  its sidecars.
- `metadata` (`pg-metadata`): ratings, dates, orientation and frame counts as the files hold them (XMP parsed as XML;
  the only XML parser).
- `codec` (`pg-codec`): every format's decoder and encoder (libjpeg-turbo, PNG, GIF, WebP, TIFF, BMP), the resizer, the
  animation frames, and the lossless rewrites of a file's own bytes (`rewrite`: the turns and crops `pg-edit` makes). The
  one place the image codecs are linked (`deny.toml`); its `fixture` feature holds the image files the tests write.
- `video` (`pg-video`): videos through FFmpeg: the probe, the poster frame, the file hash. Its build script finds the
  FFmpeg build and copies its DLLs.
- `exiftool` (`pg-exiftool`): ExifTool kept running (`-stay_open`), for writing ratings and orientation and copying
  metadata, and which program it is (`program`: `PG_EXIFTOOL`, else `exiftool` on `PATH`).
- `edit` (`pg-edit`): the lossless edits of a file in its own bytes: the turns and the crops (`rotate.rs`, `crop.rs`).
  The pixel rewrites they make are `pg-codec`'s (`rewrite`).
- `analysis` (`pg-analysis`): the import's analysis of a file, from its header and container alone (format, dimensions,
  orientation, frames, dates, rating; a video's probe), so no pixel is decoded. It also makes the duplicate hashes (BLAKE3,
  XXH3, PDQ) when the Hashes pass asks, and holds the border detector (`BORDERS_VERSION`).
- `dedup` (`pg-dedup`): the tier-1 duplicates: the all-pairs join (`join`, which the pairing pass runs), and the
  union-find that groups pairs.
- `render` (`pg-render`): the display images made from the files, upright (the one reader of the EXIF orientation for
  a decode): the grid's thumbnails (`maker`, whose `MakeError` says
  whether a file couldn't be read or doesn't decode, and `THUMBS_VERSION`), the viewer's images (`display`: the decode a
  view asks for, a kept decode reused while it covers the view, the fit and the zoomed region, a video's poster, the
  ThumbHash placeholder, in the format the window uploads), and a TIFF re-encoded as a JPEG for the Yandex search
  (`transcode`). Both encoders are the codec's `JpegEncoder`.
- `query` (`pg-query`): the query compiler: a `Query` (`pg-model`'s) becomes the SQL of its conditions and their
  parameters, and the SQL functions those conditions call (the catalog registers them on its connection).
- `catalog` (`pg-catalog`): the SQLite catalog (WAL), the source of truth for files, analyses, pairs and ratings: the
  schema, the file-write queue (`queue`), the reads (`browse`), the filters and sort (their SQL is `query`'s), the
  duplicate reports, the inbox (`inbox`) and the files the passes failed on (`ledger`; `crates/catalog/src/ledger.rs`). Its API
  is `Catalog`'s methods, each concern's in an `impl Catalog` of its own module (`roots`, `lookup`, `work`, `borders`,
  `browse`, `queue`, `dups`, `pair`, `inbox`, `ledger`); `core` is the lifecycle.
- `jobs` (`pg-jobs`): the parallel read/process/write pipeline the passes run on, the control a pass shares (progress,
  pause, stop), the job thread and schedule, the panics it catches and the event bus.
- `thumbcache` (`pg-thumbcache`): the thumbnail cache, a SQLite file of its own (`thumbs.sqlite`, at the path the
  layout names), capped in bytes (`Cap`, which the front ends read from their own input).
- `yandex` (`pg-yandex`): Yandex Images reverse search: the upload, the two result lists and the downloads (the only
  HTTP client). Its error holds the client's cases only; what the app can decode is the app's to say.
- `scan` (`pg-scan`): the folder walk: the directory listing (`pg-platform`), the skipped folders, and the catalog rows
  for what it finds; and which folders it takes (`refusal`: the policy, from the platform's facts, with a typed
  reason).
- `import` (`pg-import`): a file's analysis (`pg-analysis`), written to the catalog; and the pairing pass after it
  (`pairs`, which runs `pg-dedup`'s join on the pictures that owe their pairs).
- `thumbs` (`pg-thumbs`): the thumbnail pass: makes the thumbnails the catalog says are missing (`pg-render`, into the
  thumbnail cache).
- `writeback` (`pg-writeback`): writes the queued ratings and turns into the files (`pg-exiftool`, `pg-edit`).
- `borders` (`pg-borders`): the borders pass: looks for the borders due (`pg-analysis`) and stores them.
- `crop` (`pg-crop`): the cropped copies: cuts the borders the catalog has off into copies (`pg-edit`), with the
  original's metadata (`pg-exiftool`), never writing the originals.
- `library` (`pg-library`): the job (`runner`) that runs the passes over the originals in order, one lane per physical
  drive (`lanes` decides the lanes from the roots' drives and `run_each` runs a command over them, `coordinator` routes
  requests and turned files to them, `lane` is one drive's thread, and `passes` is the sequence each lane runs), the data folder's
  layout (`layout::Layout`, which names every file the app keeps: the catalog, the cache, the preferences, the log and
  SQLite's aliases), how a catalog folder is opened for the passes (`open_catalog`, `open_thumbs`, `versions`), and
  what both front ends do around them, once:
  adding a folder (`folders::add_folder`: its real path, the scanner's check, the catalog's row) and what follows a scan
  (`scans`: the replaced files' thumbnails forgotten, the notices its result carries). Each pass is its own crate, so its manifest
  shows what it may reach: `scan` (`pg-scan`: the folder walk), `import` (`pg-import`: the analyses), `thumbs`
  (`pg-thumbs`: the thumbnails), `borders` (`pg-borders`), `crop` (`pg-crop`: the cropped copies) and `writeback`
  (`pg-writeback`: the ratings and turns into the files). Callers name these crates directly; `pg-library` does not
  re-export them. The integration tests of `borders` and `crop` are `pg-library`'s, since they prepare a catalog with
  its `versions`.
- `client` (`pg-client`): the front end's logic, Qt-free and WASM-safe: the justified layout, the grid's viewport,
  picks, input (the filter bar's typed values), keys and the folder tree; the filter bar's facets and counts
  (`facets`), the grid's row height (`controller::size`) and the Yandex panel's result list (`results`); the
  `image://` scheme (`image`: every provider's URL, built and read). It is `no_std`; the words it shows are `wording`'s.
- `wording` (`pg-wording`): the words everything the user reads, the notices and the search's outcomes included: each
  line is a `Message`, one table of variants and their entries, worded by `render` from the Fluent entries of
  `wording/locales/en/`. It is std-based (`fluent-bundle` needs std), so it keeps the workspace's one crate-level
  `clippy.toml`; it depends on `client` and `model`.
- `app` (`pg-app`): the application layer the window uses (the developer's driver, `pg-driver`, runs the same passes through
  it). `App` opens from a `Config` and holds the services, each built on the ones below it and reached as
  `app.inbox()` (what the user is told, kept in the catalog), `catalogs()`, `prefs()`, `images()`, `reading()` (what
  the window shows, read off the UI thread), `views()` (the place the grid shows, saved views), `edit()` (ratings and
  marks), `library()` (the folders, the scan, the write-back) and `yandex()` (the reverse image search and its saves;
  `Config::yandex` says whether a recorded session that `--yandex-fixture` names answers in Yandex's place); `App`
  orders their shutdown. Beside them: the preferences file (`gallery.json`). What the user should hear is a notice posted
  to its inbox, which logs it.
- `driver` (`pg-driver`, a library and the binary `pg-driver`; not shipped): the developer's driver of the library job without a
  window. `Headless` starts the app's service over a home folder, adds folders, runs its passes until it is quiet, and reads the
  catalog back; it holds no pass logic. The binary prints JSON for scripts and measurements, `pg-driver perf` prints a perf
  log's table, and `pg-driver borders` runs the border detector on images piped in (the eval's `embed.py --trim`). The
  command line of the program is gone: its developer uses are this.
- `desktop` (`pg-desktop`, a library): the desktop app, Qt 6 Quick over CXX-Qt: the window, the QML (`qml/`, one
  flat folder), the image providers and the GPU device. The only crate that links Qt. Each cxx-qt bridge is a file in
  `src/bridges/` (cxx-qt needs one directory for a QML module's Rust sources), declared by `src/bridges.rs`, so the two
  allows its generated code needs (`unsafe_code`, `unreachable_pub`) cover that one module and nothing written by hand.
- `gallery` (`pg-gallery`, binary `gallery`): the program. It opens the desktop app (`pg-desktop`), a GUI program with no
  console window from Explorer. With the `dhat-heap` feature it is the heap profiler's build.
- `testkit` (`pg-testkit`): helpers for the crates' tests: the scratch folders (`run_folder`, under `PG_TEST_DIR`, else
  the temp folder, each owned by a lock from the moment it has its name; the protocol is in `pg-platform`'s `testing`
  module), and the raw RGB pictures the image files are written from. A test names `odd_name`, `held`, `skip` and
  `require_tool` in `pg_platform::testing` (its dev-dependency has the feature), and the generator in `pg-testrng`. Its one
  internal dependency is `pg-platform`, with the `testing` feature, so the test folders' platform code stays in
  `pg-platform`; the image files themselves are `pg-codec`'s `fixtures`.
- `jobs-testkit` (`pg-jobs-testkit`): the job layer's test helper, `Quiet` (a bus that fails its test when anything
  published on it was not taken). A dev-dependency only.
- `bench` (`pg-bench`): the callgrind benches (`benches/ir.rs`), built on Linux only.

## Rules for the crates

These are the rules the crate layout is kept to. `tools/boundaries.py` and `just lint` check the mechanical ones; the rest are checked by review.

### Ground rules

1. **Symbols, not line numbers.** Name the function or type when you write about code; line numbers drift.
2. **One public path per item.** A module is public API (`pub mod`, with a `//!` doc) or private. An item is reached by exactly one public path: a crate root's `pub use` names a private module (`mod x;` with `pub use x::Item;`), never a `pub mod`, because the `pub mod` is public under its own path and the root would give the item a second one. `tools/boundaries.py` checks it. A `pub use` at an old path is removed in the same change as its callers, and a documented layer boundary (`pg_app::reexport`, which the desktop uses because it does not depend on `catalog`, `library`, `jobs`, `thumbcache`, `yandex` or `codec`) keeps its re-exports.
2a. **Field visibility.** A field is as visible as the type that holds it: a plain data record's fields are `pub` on a public type and `pub(crate)` on a crate-private one (the lint `unreachable_pub` says a `pub` field of a private type is unreachable). A type that holds an invariant keeps its fields private, or `pub(crate)` for the crate's own code, and exposes accessors for anything else.
3. **Test support.** A test-support feature in a production crate is acceptable only when a dev-dependency crate would be a cycle or need private access. Today: `pg-codec` `fixture`, `pg-catalog` `fixture`, `pg-writeback` `fault`. Everything else (for example `Quiet` in `pg-jobs`) goes in a dev-only crate.
4. **Large files.** Split when a file holds more than one layer of a dependency graph, not by line count.
5. **Layer claims are by reading or by script, and neither is proof.** The committed split check is `tools/modgraph/split_check.py`, with its skip set in its docstring. The check that counts is the compiler: the `modgraph` run (`just modgraph`).
6. **Platform rule.** No `cfg(windows)`, `cfg(unix)` or `std::os::{windows,unix}` in production code outside `pg-platform`, except cxx-qt and FFI glue that must declare per-platform signatures. Each such exception is stated where it is declared. No `extern "system"` or `extern "C"` block outside `pg-platform`, `trace`, `video` and `desktop` (the crates that bind a native library; `FOREIGN_CRATES` in `tools/boundaries.py`).
7. **Ordering.** A split is done in one sitting on the current HEAD, its measurement re-run first, and committed before anyone else's change to that file lands. The gate runs on a clean tree. No branches.
8. **Invariants.** A condition the code relies on is guarded in one of three forms, and only these: an `assert!` with a message that names the invariant (in production too, when a wrong value would corrupt data); `unreachable!` in a match arm the types leave impossible, with the reason in its message; and `expect` only under `#[expect(clippy::expect_used, reason = "…")]` on the function. A bare `assert!` or `expect` with no message or reason is not used.

### Gates

- **Linux:** `docker run --rm --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/root/.cargo/registry pg-dev`. It runs `docker/check.py` in CI's order. It covers every crate except the desktop app.
- **Windows:** `just lint` and `just test`. The desktop app builds only here, with `just build`.
- **Desktop moves:** also `just shots`. A pure move must show no pixel change against `tools/shots/reference`.
- **Cargo doc:** `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked` (`tools/lint.py`, which `just lint` runs). Run it on each change that moves or re-exports items.
- **Perf:** `just perf` runs callgrind in `pg-dev` and gates against `docs/perf/baseline.json`.
- **Module cycles** (`just modgraph`, the CI step): 115 to 149 seconds on the successful runs of 2026-11-10 and 2026-11-12, on both targets, with `rust-cache` warm.

### Decisions

- **`pg-model` stays one crate.** Its modules are consumed by one to fifteen crates each; its fan-in is expected.
- **`cargo-modules` is a tool, not a dependency.** It is pinned in `docker/Dockerfile` and the Justfile (`cargo_modules_version`, which `just modgraph` checks and installs at the pin). The Linux graph runs in `pg-dev`; the Windows graph runs on the host through `just modgraph`.
- **The driver steers the app's service.** `pg-driver` depends on `pg-app` and holds no pass logic: it starts the service, adds folders, waits until it is quiet, and reads the catalog back.
- **Errors are typed.** A library crate returns its own `thiserror` enum. `anyhow` belongs to `pg-driver` and `pg-desktop` only: `tools/anyhow-allow.txt` lists the library crates still on it (the file is empty), and `AnyhowAllowlist` in `tools/test_ci_files.py` fails when another crate depends on it. A crate's main error is named `Error`; a second enum in the same crate takes a `<What>Error` name (`ReadError`, `MakeError`). A pipeline's failure converts into `pg_jobs::failure::Failed` once, at the crate's boundary, and keeps its class.

### Naming

- **Keys, names and codes.** `key()` and `from_key()` give a value's stored spelling: a catalog column, a queue key, a JSON tag. `name()` gives display text. `code()` gives a number only (`Level::code`, `Purpose::code`). A type with a stored spelling has `key()` and never `code()`, even when a front end also shows the text. `Resolver::token()` gives the composite text a resolver's notice carries, since it is neither a stored key nor a display name. The `Notice` macro's `code()` is the one exception: `Notice::key()` already returns the `Key` type, so the macro keeps the old name.
- **Turns and rotation.** `turns` names a count of quarter turns and the `Turns` type. `rotation` is the write queue's key (`write_rotation!`, the `rotation` columns, the `.rotation.pg-write` temp name) and only that: a Rust field that holds quarter turns is named `turns`.
- **Test names are sentences** that say what holds (`a_write_reads_back_from_its_columns_and_a_value_out_of_range_does_not`). A bare noun names a helper, not a test. A platform-specific test keeps its `_windows` or `_posix` suffix.

### Environment

- **Variables are read through `pg_platform::env`.** `read`, `value`, `flag` and `process` are the readers: `process` is the process's own lookup, which a crate takes as a parameter (`Layout::resolve`), and the others take a variable from [`Var`](pg_platform::env::Var). A `PG_*` variable the app reads has a `Var` and a row in `VARS` there, which says what it holds and who reads it; `.env.example` documents the ones an operator sets. An empty value is unset.
- **Flags have one set of spellings.** `pg_platform::env::parse` says which texts are on and which are off. `flag` gives `Parsed::Invalid` for any other text, and the caller says so once (`PG_DEV` is a bad option, `PG_TRACE` and `PG_REQUIRE_TOOLS` say it at start). The C++ desktop reads its flags with the same set (a test holds the two to one set).
- **The exceptions are named.** A build script (`crates/<crate>/build.rs`) reads the variables of the build, and is exempt by its path (`tools/boundaries.py`, `is_build_script`; the `PG_*` names it uses are checked against the registry). The one write of the environment, which sets `QT_AUDIO_BACKEND` before the app's threads start, is `pg-platform`'s `set_process`, called through `audio::prefer_pulseaudio_if_pipewire_fails`; the registry's two functions are the only other reads, listed in `ENV_READERS` with their headers.

### Review rules

These two rules came out of the fourth modularity review (its full text is in git at `e428ab2`, `docs/temp/MODULARITY_REVIEW_4.md`).

- **R-1: an outcome a front end words is pg-model data.** `pg-client` depends on `pg-model` alone and cannot name a lower crate's type. So every outcome that is put into words is a plain data type in `pg-model`, built by the crate that has the outcome. `pg_client::text` words it, and both front ends render it.
- **R-2: a stored path reaches the file system only through `disk_path`.** A catalog path (`RulePath`) becomes a plain `PathBuf` only through `pg_platform::names::disk_path(&PathRules, &RulePath)`, the verbatim form (on Windows, a plain path drops a trailing dot or space from a folder's name). `names::path_buf` is private to `pg-platform`. A shown path is `PathRules::display`, or `names::shown` for a path the system gave.

## Dependencies

The internal dependencies (normal ones, from `cargo metadata`) only point up the list: no crate depends on one below
it. `tools/crate-layers.txt` lists each crate's internal dependencies exactly, bottom layer first, and
`tools/boundaries.py` (in `just lint` and CI) fails on an edge the file does not list. It also checks that the
dependencies including the dev and build ones have no cycle (a crate's tests never depend on a crate that depends on
it: `pg-platform`'s own tests use its `testing` module directly, so `testkit` sits above `platform`), that a `pub use` names a child module, that a crate root's `pub use` never names a `pub mod` (one public path per item), and the platform
and environment rules above. Module cycles inside a crate are `just modgraph`'s (the compiler's
module graph and a text scan, `tools/modgraph`), with an empty allowlist; CI runs it on both targets.

- `model` and `container` depend on no crate here. `platform`, `client` and `yandex` depend on `model` alone, `dbfile`
  on `platform` alone, and `log` on `model` and `platform`. `wording` depends on `client` and `model`.
- `desktop` is the only crate that links Qt. It depends on `app`, the application layer the window uses, and on
  `client`, `log`, `model`, `platform`, `render`, `trace` and `wording`; it does not depend on `catalog`, `library`, `jobs`,
  `thumbcache`, `yandex` or `codec`. It reaches what it needs of them through `app`'s typed methods, and through
  `pg-model` for the plain data they hand out (the moved types live there), and `app` does not depend on it. `dev` is the developer's driver beside it, and `gallery` depends on
  `desktop` only.
- External crates have a fixed set of users (`deny.toml`, `[bans]` wrappers, checked by `cargo deny`): SQLite in
  `catalog`, `query`, `dbfile`, `thumbcache` and `bench`; `ureq` in `yandex`; FFmpeg in `video`; the image codecs in
  `codec` (the test fixtures included, and the few crates named there); `cxx-qt` in `desktop`; `quick-xml` in `metadata`.

## Where the old paths went

The crates were `core` (`pg-core`), `ingest` (`pg-ingest`), `desktop`, `gallery`, `testkit` and `bench` before the
refactor. The dated logs in `docs/` keep the old paths; the table is for reading them.

| Before (under `crates/`)                                                                         | Now                                                                                                                                                                                               |
|--------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `core/src/{date,format,media,orientation,path,detection,analysis,state,view,pdq}.rs` (the types) | `model/src/` (the same names; `pdq.rs` is the PDQ record, the hasher is in `analysis`)                                                                                                            |
| `core/src/query.rs`                                                                              | the query's types in `model/src/query.rs`, its SQL in `query/src/lib.rs`                                                                                                                          |
| `core/src/catalog.rs`                                                                            | `catalog/src/lib.rs` (the readers and writers are `catalog/src/browse/`, `queue.rs`, `dups.rs`)                                                                                                   |
| `core/src/browse.rs`, `browse/`                                                                  | `catalog/src/browse.rs`, `catalog/src/browse/`                                                                                                                                                    |
| `core/src/analyze.rs`, `borders.rs`, `pdq.rs`                                                    | `analysis/src/` (the same names)                                                                                                                                                                  |
| `core/src/pdq_index.rs`                                                                          | gone: the all-pairs join is `dedup/src/join.rs`, run by the pairing pass (`import/src/pairs.rs`)                                                                                                  |
| `core/src/decode.rs`, `codecs.rs`, `pixels.rs`, `resize.rs`, `animation.rs`                      | `codec/src/` (the same names); the animation player is `render/src/player.rs`                                                                                                                     |
| `core/src/maker.rs`, `display.rs`                                                                | `render/src/` (the same names)                                                                                                                                                                    |
| `core/src/rotate.rs`, `crop.rs`                                                                  | `edit/src/` (the same names)                                                                                                                                                                      |
| `core/src/rewrite.rs`                                                                            | `codec/src/rewrite.rs` (with its modules in `codec/src/rewrite/`)                                                                                                                                 |
| `core/src/container.rs`                                                                          | `container/src/lib.rs`                                                                                                                                                                            |
| `core/src/metadata.rs`                                                                           | `metadata/src/lib.rs`                                                                                                                                                                             |
| `core/src/video.rs`                                                                              | `video/src/lib.rs`                                                                                                                                                                                |
| `core/src/layout.rs`                                                                             | `client/src/layout.rs`                                                                                                                                                                            |
| `core/src/thumbs.rs`                                                                             | `thumbs/` (the pass); the cache is `thumbcache`                                                                                                                                                   |
| `core/src/verbatim.rs`                                                                           | `platform/src/native.rs` (the verbatim paths)                                                                                                                                                     |
| `core/src/yandex.rs`                                                                             | `app/src/yandex.rs` (the search); the HTTP client is `yandex`                                                                                                                                     |
| `core/examples/stage_times.rs`                                                                   | `analysis/examples/stage_times.rs`                                                                                                                                                                |
| `ingest/src/{import,scan,writeback,thumbs}.rs`                                                   | `import/`, `scan/`, `writeback/` and `thumbs/` (the crates of those names)                                                                                                                        |
| `ingest/src/{runner,borders,crop,dups}.rs`                                                       | `library/src/runner.rs`; the passes `borders/src/lib.rs` and `crop/src/lib.rs` (crates of their own); `dups` with its report in `cli/src/dups.rs`, and the borders report in `cli/src/borders.rs` |
| `ingest/src/pipeline.rs`, `control.rs`                                                           | `jobs/src/` (the same names)                                                                                                                                                                      |
| `ingest/src/cli.rs`                                                                              | `cli/src/lib.rs`                                                                                                                                                                                  |
| `ingest/tests/*`                                                                                 | `library/tests/*`                                                                                                                                                                                 |
| `desktop/src/loader.rs`                                                                          | `app/src/reader.rs` (the catalog reads off the UI thread)                                                                                                                                         |
| `desktop/src/state.rs`                                                                           | `app/src/images.rs` (the image work behind the providers)                                                                                                                                         |
| `desktop/src/yandex.rs`, `library.rs`, `animation.rs`                                            | `desktop/src/bridges/` (the Qt bridges), with the logic in `app/src/yandex.rs`, `app/src/library.rs` and `render/src/player.rs`                                                                   |
| `desktop/src/*` (the rest), `gallery/src/main.rs`, `testkit`, `bench`                            | the same paths                                                                                                                                                                                    |

## Build

Needs CMake and NASM (libjpeg-turbo is built from source by the `turbojpeg` crate), and LLVM for clang-cl on Windows:
parajpeg's C++ (the viewer's full-size JPEG decoder, `crates/codec`) is built with clang-cl, because its AVX2 kernels use
GCC/Clang attributes that MSVC lacks. `CLANG_CL` names another `clang-cl.exe`; the build fails with the install's name
if none is found at `C:\Program Files\LLVM\bin`. The clang-cl the build uses is the one LLVM installs, the same as
libclang's. Qt, at the Justfile's `qt_version`, for MSVC
(the `msvc2022_64` kit, whose folder `PG_QT_DIR` in `.env` names; `just doctor` checks it), with the **Qt Image Formats** add-on: the Yandex panel
shows remote previews through Qt's decoders, and without its `qwebp` plugin WebP results stay blank. Writing ratings into files needs
[ExifTool](https://exiftool.org) on `PATH` (or `PG_EXIFTOOL`); without it they stay queued and the bottom bar
says why. Its test (`crates/library/tests/writeback.rs`) works on generated files in
the temp folder's `scratch_rate` (`PG_TEST_DIR` moves it), and skips itself without ExifTool.

Videos go through FFmpeg 9, our own copy rather than the one Qt bundles: BtbN's prebuilt `gpl-shared` package
(on Windows `just setup` fetches the pinned build into `.deps/ffmpeg`, and `crates/video/build.rs` reads its headers there; on
Linux the headers come from the FFmpeg development packages, through pkg-config. `FFMPEG_DIR` in `.env` names another folder on
either. The pin and its SHA-256 are in the `Justfile`). It has dav1d for AV1, which Qt's copy lacks, and the x264, libvpx, libaom and libwebp encoders the
video test needs. The app is linked against this GPL build, so a distributed build would be under the GPL; the app
is not distributed. The bindings to its types are generated from its headers with libclang (LLVM; `LIBCLANG_PATH` in `.env` names its `bin` folder when libclang isn't in the usual place).
Nothing links FFmpeg: `pg-video` loads its DLLs at run time by full path (`crates/video/src/ffi/load.rs`), from the folder
of the executable, then from the `bin` the build script found, so PATH plays no part. Its build script copies the DLLs
into the profile folder, so `gallery.exe` finds them beside it. Qt Multimedia plays videos with
the same DLLs: the pinned Qt is built against FFmpeg 9, the same major version. So `just deploy` runs `windeployqt
--no-ffmpeg`. The video test (`crates/library/tests/video.rs`) makes its clips and animations with the same `ffmpeg` command line,
found at `PG_FFMPEG` if that is set, else in the pinned build's `bin` (`.deps/ffmpeg/bin`), else on `PATH`. The clips go in
the temp folder's `scratch_video` (`PG_TEST_DIR` moves it), and the test skips itself without an ffmpeg.

On a new machine, copy `.env.example` to `.env` and set `PG_QT_DIR` and any path that differs from the defaults. Then `just doctor`
says what is still missing, and `just setup` fetches the pinned FFmpeg into `.deps/ffmpeg`. `just` reads `.env` for every
recipe, and the Python and cmd tools read it too; a plain `cargo` command sees only the real environment. The commands are in the `Justfile` ([just](https://just.systems); `just` lists them):

```powershell
just doctor  # what is missing on this machine, and how to fix it
just setup   # fetch the pinned FFmpeg into .deps/ffmpeg
just deploy  # release build, then the Qt DLLs and QML modules next to gallery.exe (once, and after Qt updates)
just build   # rebuild
just run     # the app; the developer's driver: `cargo run -p pg-driver -- status --home <folder>`
just test    # a debug build, with the dependencies optimized (Cargo.toml), so decoding runs at full speed
just lint    # rustfmt, clippy, cargo-machete, ruff, markdownlint, then the QML's (just lint-qml); cargo-deny is `just deny`
```

`just deploy debug`, then `just dev`, give the debug build (it links the release Qt too). It reads the QML from
`crates\desktop\qml` instead of the copy compiled into it and reloads the window whenever a file there changes, where
it was: QML edits show without a rebuild. QML errors go to the debugger output unless
`QT_FORCE_STDERR_LOGGING=1` is set.

`just lint` also checks that `pg-model` and `pg-client` build for WebAssembly (`wasm32-unknown-unknown`) and that the
rustdoc has no warnings. `just check-linux` runs the Linux job's checks, clippy included, in the container (the Linux-only code). `just lint` needs `cargo binstall cargo-machete` once, and `just deny` needs `cargo binstall cargo-deny` once. `cargo deny check` covers advisories, licenses
(the allowed list is in `deny.toml`), duplicate versions, and each dependency staying in its crates (Qt in `pg-desktop` alone, the image codecs in
`pg-codec`, SQLite in `pg-catalog`, `pg-query`, `pg-dbfile` and `pg-thumbcache`);
`cargo machete` finds unused dependencies. `just lint-qml` fails on any qmllint warning, and on any binding or
function the QML compiler (qmlcachegen, as the build runs it) leaves to the interpreter instead of compiling to C++.
It reads the Rust types from the last build, so build first (`tools/lint_qml.py` says why it lints a copy of the
module in `target/qmllint`).

The QML compiles to C++ only where it is typed throughout. So the Rust objects' properties are `FINAL`. A Rust
object's methods are called only in the file that makes it, by its id: through a property, a method can't be
compiled, because a subclass could replace it. FilterBar owns the filters and Main the gallery model, and the
viewer gets the shown item's values as properties. Text and numbers come typed from Rust (`CommandLine`,
`Library.writeFailed`), not from JavaScript's string functions, which aren't compiled. qt-build-utils is a pinned fork
(`vendor/README.md`), so that the compiler sees the module's types.

## Run

The data folder (its home) is `PG_HOME` when set. Otherwise it is the parent of the catalog folder, when one is given:
`PG_CATALOG_DIR` (which `.env` sets for `just run` and `just dev` on Windows). The developer's driver takes a `--home` instead and reads no environment.
Otherwise it is the platform's default: `%LOCALAPPDATA%\PhotoGallery` on Windows, and `$XDG_DATA_HOME/photo-gallery`
(`~/.local/share/photo-gallery`) on Linux. Under the home are `catalog` (the catalog folder: `catalog.sqlite` and
`thumbs.sqlite` (the thumbnail cache), `gallery.json`, `gallery.log` and, on Windows, `sqlite-aliases` (where a folder whose name SQLite
can't open is reached by an alias): nothing of the app's is written outside it. `PG_PREFS` moves the preferences file alone. Each start's first record in `gallery.log`
is the session's, with the build and the data folder (the log's format is in `crates/log/src/lib.rs`). The app's preferences (write-back, Real Size, borders, row height, the last
file selected, dev mode) are in `gallery.json`, so they survive rebuilding the catalog. Saved views and
where you were (folder, subfolders, filters, sort) are in the catalog. Each of these is resolved in one place,
`crates/library/src/layout.rs`; for now they share the home folder.

The catalog's schema is `crates/catalog/src/baseline.sql`: every table, index, view and trigger, and the seeds of
`meta`, at version 1. Before the first release that file is edited in place and the version stays 1. From the release
on, every change is a migration (`MIGRATIONS` in `crates/catalog/src/schema.rs`) and the baseline is frozen; the
migration machinery is kept for that, and has no migration yet. A catalog at a version after this build's is refused,
as made by another build. Each start checks the catalog's schema against this build's for its version (`pg-dbfile`),
and a difference (a changed column, index or trigger, an extra object, another application id) is rebuilt before the
release: a new catalog is made beside it, the rows are copied into it with their ids, and the old file is moved aside.
Nothing is deleted. A catalog made under the other platform's path rules is refused, not moved: open it on the platform
that made it.

The ids of `files`, `folders` and `roots` are `AUTOINCREMENT`, so a deleted row's id is never given to a later one. The
catalog is created with incremental auto-vacuum, so a removal gives its free pages back; a file made without it is
compacted by one `VACUUM` at its next start (`meta.vacuum_owed`).

Development catalogs from before this reset (any version above 1) are refused. Delete them by hand (`catalog.sqlite`
and its sidecars, `.pre-v*.bak`, `.replaced*`) and let the next start make a new one, then scan again.

Run `.\target\release\gallery.exe` and add folders with **Library…**. The app scans them, imports them and
makes thumbnails in the background, with progress and a Pause button in the bottom bar. Background work runs
below normal priority (thumbnails at idle), so the window and the thumbnails on screen come first. Quitting at
any point is safe: the next start finishes what's pending. An internal error (a Rust panic) shows a message and
appends its details and backtrace to `gallery.log` in the data folder (see Run above).

The same job without the window, for scripts, profiling and checking results: the developer's driver, `pg-driver`, over a home
folder of its own (a separate catalog, so it never touches the app's library):

```powershell
$d = '.\target\release\pg-driver.exe'
$devHome = 'D:\PhotoGallery\dev-home'
& $d run --home $devHome --add 'D:\PhotoGallery\datasets\pinterest' | Out-Host   # add a folder, run the passes until quiet
& $d run --home $devHome --height 256 --passes thumbs,placeholders | Out-Host     # thumbnails at one grid height
& $d run --home $devHome --passes borders | Out-Host                              # the borders of what was imported
& $d status --home $devHome | Out-Host                                            # folders, counts, pending and failing writes
& $d remove --home $devHome 'D:\PhotoGallery\datasets\pinterest' | Out-Host       # take a folder out (its files on disk are not touched)
```

`pg-driver` prints one JSON object per result on stdout, and its progress on stderr. `gallery.exe` is a GUI program, so no
console window opens when it starts from Explorer. PowerShell doesn't wait for a program unless its output is piped, hence
`| Out-Host`.

- **Scan** lists folders by directory handle, which yields each file's NTFS file ID: a moved or renamed file
  keeps its row and its analysis. It records the files whose extension is a supported format (`crates/model/src/media.rs`), with
  their extension and kind.
- **Import** reads each file's header and container only, in MFT-record order, with the reads in flight tuned per
  drive (`pg_jobs::tuner`; `docs/plans/ADAPTIVE_READERS_PLAN.md`): the format, the
  dimensions (from the codec's header, with the EXIF orientation applied), the frame count (from the GIF, APNG or WebP
  container), and the date taken (EXIF DateTimeOriginal, else XMP). No pixel is decoded and no hash is made. A JPEG is
  read up to the end of its header (`pg_container::jpeg_import_bytes`: 16 KiB, then on as its segments say), and a PNG
  over 256 KiB at its two ends, without its image data (`pg_container::png_import_bytes`); each is read whole when its
  header is not as those reads expect. Both open the file for random access (no read-ahead). The import owes the duplicate hashes: a row of `hashes_owed` for each picture. The Hashes pass makes them,
  from a DCT-scaled decode (libjpeg-turbo, to ≥ 512 × 320), with each PDQ hash stored in 4 orientations (the pairing
  join probes all 8, mirrors included); a
  picture of reliable quality then owes its tier-1 pairs (a row of `pairs_owed`), and the pairing pass finds them
  (d ≤ 31). With duplicates detection off (the Library panel) the hashes stay owed until a
  later Hashes pass makes them. A video (`crates/video/src/lib.rs`) is probed by its container at import, and hashed by
  streaming in the Hashes pass, never read whole. Its upright size, duration, codec and creation date come from the
  container: the rotation matrix, and Apple's local creation date before the UTC `creation_time`. Videos get no pixel
  hash or PDQ, so tiers 0b and 1 don't cover them. Every 500 files is one transaction; a rerun continues where the last
  one stopped, and redoes files whose size or mtime changed. A file whose header is valid but whose pixels don't decode
  fails in the first pass that decodes it (the Thumbs or Placeholders pass, or the Hashes pass).
- **Thumbs** makes no thumbnails during the import: they are made afterwards, for any height. The cache
  (`thumbs.sqlite`) is keyed by (file, height) and capped. The cap is kept
  in the cache, so the app and the driver keep to the same one: a twentieth of its drive (1 to 50 GB) for a new cache, changed by
  the app's `PG_THUMB_CAP_GIB` (the Library panel sets it too), and shown in the Library panel. The background job (or `pg-driver run --passes thumbs`) fills it at BelowNormal CPU
  priority for the grid's current height, evicting only other heights, and starts over when the size changes.
  The ones on screen are made on demand, newest request first, and evict the least recently used of any height.
  A video's thumbnail is its poster frame. HDR videos (PQ, HLG) are converted to RGB without tone mapping, so
  their posters look washed out.
  - Layout (`crates/thumbcache/src/lib.rs`): the entries (a few dozen bytes each) and the JPEGs are separate tables, and the totals
    are one row that triggers keep, so opening the cache and listing what's missing never read the JPEGs
    (docs/reference/START_PROFILE.md "Startup"). Reads take no lock; writes go through one writer thread, in IMMEDIATE transactions, so
    the app and `pg-driver` can share the file. "Least recently used" is to the hour: a hit rewrites its
    entry only when it is older, and those writes are batched. A cache far over its cap (the cap lowered) comes
    down in slices of 256 MB, one transaction each, so the other process never waits long for the file.
  - Its layout version is 1. Before the first release a layout change edits the schema in place, and the cache is
    never migrated: it holds derived data only. A file of another version, layout or catalog is started over (its
    cap kept); one that is not a database, or that SQLite finds damaged (at the open, or at the day's check of the
    totals), is moved aside to `thumbs.sqlite.bad` (`pg_dbfile::move_aside`) and made anew; a database that is not
    a cache is refused and left alone. A cache that can't be opened only means thumbnails aren't kept.
  - A stored JPEG that doesn't decode (bad bytes, which SQLite can't see, so no check finds it; the day's check of the
    totals reads only the entries): the read path finds it. The desktop decodes a cache hit as it is (`providers.rs`,
    `thumbnail`). When that fails, the hit is dropped (`ThumbCache::forget_height`, which also forgets that height's
    checked watermark), so the next pass at that height makes the thumbnail anew from the original; until that pass,
    the cell is blank and a line goes to the panic log. A hit that decodes is kept: it is remade only when the file
    changes (a new mtime or rotation makes the entry stale), the cache is started over, or the entry is evicted. A
    shown entry is touched hourly, so it stays. Damage SQLite finds in the entries' pages is found only by the day's
    check, which runs at an open (up to a day late), and then the file is moved aside.
- **Borders** (`crates/analysis/src/borders.rs`, a port of the Python prototype `tools/borders.py`, which is not in this repo; BORDERS.md
  "Border cleaning"): solid borders (black, white or any colour, JPEG noise included) and blurred padding bars, on
  the stored pixels at full size.
  A pass of its own after the import (`pg-driver run --passes borders`, or the background job after the thumbnails, the hashes and the
  pairs; with border detection off in the Library panel the pass does not run and the files stay due) reads and
  decodes each still image the import analyzed at full size: the import's decode is reduced, and the detector
  is calibrated on every pixel. The import marks the file due (`analysis.borders_due`); a re-analysis drops its
  old borders. Every side found is kept (`border_sides`), the rejected candidates too with why; the cut is in
  `analysis.cut_*`. The detector has its own version (`BORDERS_VERSION`): a new one makes every file due again,
  without importing the files again.- **Library**: the folders added (`roots` table). None is inside another: adding a parent replaces
  its children, and adding a subfolder only rescans it. Removing one deletes it for good: its files, their analysis,
  ratings and thumbnails, and its folders (`Catalog::begin_removal`, then `removals::finish` in `pg-library`). The files
  on disk are not touched. A removal can't be added back while it is pending. A change still waiting to be written into
  one of its files keeps that row (and the temp file beside the file) until the app finishes the write or
  `pg-driver remove --discard-pending` gives it up. A folder that is only missing (an offline drive) is not removed: its
  files stay missing, as before. Folders removed before this version (their files hidden, with no marker and no library
  folder covering them) are adopted when the removals thread first runs after the start: each gets a marker, and its
  removal deletes its rows and thumbnails like any other. A folder that a library folder still covers is not adopted.
- **Write** is what the app's writer does in the background (see Ratings below); the command drains the queue
  and reports what still fails.

## The app

a rating app's triage, without Everything: open "Unrated", largest first, rate each picture and move on.

The window comes up before anything is read from the catalog: the folder tree, the grid's files and the filter
counts are read by a thread of their own (`crates/app/src/reader.rs`), oldest request first, each kind keeping only its newest
(which cancels one of its kind running), and shown when they arrive ("Loading…" meanwhile). The selection stays
with its file when a list is refreshed; while the viewer is open a new list waits for it to close. A rating or a turn
made while a list was on its way is kept. Startup times are in docs/reference/START_PROFILE.md "Startup".

- **Folder tree** (left): folders with the same path on different drives are one node (their drives are shown
  in green). "All folders" lists everything (it turns subfolders on); "Drive roots", shown when files lie directly
  in a drive root (`D:\`), lists only those (subfolders off). "Include subfolders" toggles recursion; for the empty
  folder that moves between the two rows.
- **Top bar**: "N of M files", **Filters** (or **F**) to open the filter bar, the order (name, file type, file size, date
  modified, date taken, date created, rating, megapixels, width, height, aspect ratio, duration, frames, border depth)
  and its direction (↑ / ↓). **Views ▾** saves the current folder,
  subfolders, filters and sort under a name (a rating app's bookmarks), and opens or deletes saved ones.
- **Filter bar** (`FilterBar.qml`, `crates/query/src/lib.rs`): type (image, animated, video), extension, rating (unrated,
  1–5), shape (landscape, portrait, square), file size, megapixels, duration, date taken, date modified,
  duplicates (has / has none, from tiers 0a, 0b and tier-1 pairs within PDQ 22; "has none" is listed only once no
  picture owes its pairs, so it is empty until the pairing pass has run), import status (imported, not
  yet, unreadable), borders (none, black, white, coloured, blurred bars), their review (not marked, marked not a
  border) and part of the name. Facets combine with AND, the choices inside one with OR. Each type, extension
  and borders choice shows how many files choosing it would show, given the other facets. Filtering and sorting are
  SQL (values bound, not spliced); a change applies 0.3 s after the last edit. The list re-filters when it is
  reopened or the filters change, not under you: a picture rated out of "Unrated" stays until then.
- **Placeholders** makes each file's ThumbHash after the thumbnails, for every file that owes one. It reads the cached
  thumbnail when the cache holds one at least 200 px high (about a millisecond), and otherwise decodes the original
  DCT-scaled (at least 200 px on its long side). The hash is encoded from at most 100 px and stored in the
  `placeholders` table, with the file's identity (size, mtime, rotation) and `PLACEHOLDER_VERSION`: it is made again
  when the file changes or the definition does. It doesn't depend on the grid's height. It follows the thumbnail cache's
  setting: with the cache disabled it makes none (the stored ones still show), and it resumes when the cache is back on.
  A file whose original doesn't decode is recorded in the Thumbs ledger (`retry = false`), and tried again when
  it changes or the user retries it. `pg-driver run` runs this pass after the thumbnails.
- **Grid**: justified rows. The **size slider**, Ctrl+wheel or Ctrl +/− change the row height at runtime; the
  layout follows at once, thumbnails at the new size follow a moment later (the old ones are scaled meanwhile,
  and the ThumbHash placeholder, where one has been made, shows until the first one arrives). Arrows move, 0–5 rates, **R** / **Shift+R**
  turn left / right, Enter or double-click opens the viewer. A badge shows each rating's symbol, another a
  video's length (▶ 1:23) or the format of an animated image (GIF, APNG, WEBP), and ▭ borders found (struck out
  when marked not a border). **B** shows on every thumbnail what cropping the borders would cut (remembered,
  shared with the viewer), **N** marks the selected file's borders as not a border, or takes the mark back.
- **Viewer**:
  - ← → (or Backspace / Space) step; 4 pictures each way are preloaded.
  - **0–5**, **↑** (4) and **↓** (1) rate and move to the next picture, showing the rating's symbol for half a
    second. The rating bar at the bottom does the same by click, with the current rating ringed; its ⟲ ⟳ (and
    **R** / **Shift+R**) turn the picture.
  - **Zoom**: + / − around the centre, pinch on a touch screen or touchpad, from fit to 32×; drag pans. The
    wheel steps to the previous or next picture (a touchpad's small scrolls add up to a step). Double-click
    toggles fit and 1:1. Fit is a Lanczos3 resample of the original to the window, down for a large picture and
    up for a small one; between fit and 1:1 the visible part is rendered with Lanczos3 as the view moves, and the last
    one shown stays until the next is ready, so the picture stays sharp; from 1:1 up it is the original with nearest neighbour, to judge pixels. A picture
    smaller than the window opens enlarged, and 1:1 is then the furthest zoom out.
  - **Z** Real Size: pictures open at 1:1 instead of fitted, remembered across pictures and runs.
  - **L** a 10× nearest-neighbour loupe under the cursor; **B** a magenta frame around the image to reveal black
    bars, and what cropping its borders would cut, dimmed (grey when marked not a border), with rejected
    candidates as yellow lines; hovering over the cut shows the picture under it. **N** (or ▭ in the rating bar)
    marks the borders as not a border. **H** or a click hides everything over the picture (zen). Touch: swipe left / right to step, up / down
    to rate 4 / 1. Esc, Enter or ✕ close.
  - **Videos** play with Qt Multimedia, fitted to the window, up or down. They start with sound and loop, and the poster
    shows until the first frame. The transport bar has play / pause (**K**), the time, a seek bar, mute (**M**)
    and volume. Rating, turning and stepping work as on pictures; zoom, the loupe and Real Size are for stills.
    The player decodes on the GPU, except on a GPU without AV1 decoding (this machine's RTX 2060). There every
    video decodes on the CPU (AV1 through dav1d) (`crates/desktop/src/gpu_device.rs` explains why).
  - **Animated** GIF, APNG and WebP play the same way: our FFmpeg decodes all three. Qt's `AnimatedImage` has no
    APNG, and has WebP only with Qt's Image Formats add-on. They loop silently, without the transport bar; **K**
    pauses.
  - **≈** (the rating bar) finds similar images on Yandex (`crates/yandex`, `crates/app/src/yandex.rs`, `YandexPanel.qml`): the photo is uploaded
    (a TIFF as a JPEG; a video is refused), and a panel shows its other sizes and the similar images. A click
    picks a result, Shift-click picks or unpicks a range (to the state of the result clicked last), and each group
    has Select all. A double-click shows a
    result at full size, fitted or 1:1 (a click or the button); ←/→ step through the results, Space picks,
    Escape goes back. **Save N images…** saves the picks into a folder; the dialog opens at the folder saved into
    last (in `gallery.json`), or else the photo's. Files are named after the photo, `IMG_1234 (yandex
    1920x1080).jpg` and `IMG_1234 (yandex similar 3).jpg`, with " (2)" on a clash.
- **Ratings**: the catalog (`ratings` table) is the source of truth for every format. With "Write ratings and
  turns into the files too" (Library…, on by default) they also go into the file as `XMP:Rating` wherever the
  format holds XMP (JPEG, PNG, WebP, TIFF, GIF), by a background writer (ExifTool), 2 s after the last change so
  quick changes coalesce. BMP, videos and other formats keep it in the catalog only (the status line says so).
  ExifTool could write XMP into an MP4 or MOV, but it rewrites the whole file, gigabytes for a video. Writes
  are queued in the catalog (`file_writes`), so they survive quitting, wait for offline drives, and retry; the
  bottom bar shows what is pending or failing. A file is rewritten in place, keeping its NTFS identity, and a
  rating write doesn't make it look changed to the import. Ratings already in files (a rating app, Explorer,
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
  modification time, in the background (`pg_crop`). The originals are never written: a folder whose copies
  would land in the library is refused, and a file changed since its borders were found is skipped. JPEGs are
  cropped losslessly; their left and top cuts move inward to the 8 or 16 px block grid. PNG, BMP, GIF and WebP
  are cropped exactly and re-encoded losslessly with every other chunk kept (a lossy WebP comes out lossless,
  larger), TIFF re-encoded with its metadata copied by ExifTool.
- Test hooks (for checking the UI without a screen; see `TestHooks.qml`): `--size=<px> --select=<n> --view=<n>
  --viewer-keys=LBNHK+1<>r<n> --zoom=<x> --menu=views --apply-view=<name> --query=<json> --sort=<json> --filters=1
  --borders=1 --library=1 --save-view=1 --find-similar=<path> --yandex-preview=<n> --add-folder=<path> --save-crops=<folder>
  --screenshot=<png> --wait=<s>`. `--query`, `--sort` and `--apply-view` shape the first list; the others act on it
  once it is shown. `--wait` (10 s by default) counts from the call (the screenshot is scheduled as the window is
  shown, before the first list); a window that hasn't settled or isn't ready by then is saved as it is, and a run
  with no window exits with status 1. `--borders=1` shows the borders (B) for the run. `--find-similar` uploads the
  file to Yandex: a path that isn't a still image (a `.txt`) opens the panel on its error without sending anything; `--yandex-preview` then shows result n at full size. With
  `QT_QPA_PLATFORM=offscreen` needs `platforms\qoffscreen.dll` next to the exe (from Qt's `plugins\platforms`). The app
  points `QT_QPA_FONTDIR` at the Windows fonts folder itself when the platform is offscreen (`crates/desktop/cpp/gui_app.cpp`), since
  offscreen's font lookup otherwise finds no fonts and text draws as boxes. Set `QT_FORCE_STDERR_LOGGING=1` to see QML
  messages.
  Offscreen draws with Qt's software renderer, which shows no video frames: check playback through the player's
  state and position instead. The screenshot is the window as shown, menus included. Rating and turning hooks
  write to the catalog and the files: use them on a scratch library (`PG_CATALOG_DIR`, `PG_PREFS`).
