# Plan: fix the modularity findings (M-series)

Status (2026-11-09): investigated, re-measured at `905561d`, and re-reviewed in twelve passes. The consolidated rewrite is applied. M-0, M-1, M-3 and M-3a are done and committed (each step says where). Churn and cost are not a constraint; the goal is the proper fix for each finding, not the cheapest one.

## Investigation results

Measured on the working tree at `905561d` (2026-11-09). The measurement scripts are in `tools/modgraph/` (module graph); the split-check and `unreachable_pub` scripts are in the session scratchpad and are committed under `tools/modgraph/` in M-0.

- **Toolchain and container.** `rust-toolchain.toml` pins `nightly-2026-11-06`. The `pg-dev` image has nightly rustc and cargo, `just` 1.59.0, `cargo-deny` 0.20.2, `cargo-machete` 0.9.2, `ruff` 0.17.3, `markdownlint` 0.50.0 and `cargo-modules` 0.28.0 (added 2026-11-08). It has no valgrind and no gungraun runner. The cargo target volume is warm.
- **Compiler-backed module graph.** `cargo-modules` 0.28.0 resolves paths with rust-analyzer's semantic model, not a text scan. Command: `cargo modules dependencies --package <crate> --lib --no-externs --no-sysroot`, run by `tools/modgraph/compiler_graph.py`. Only the `uses` edges count; the `owns` (containment) edges stay in the output, because `--no-owns` drops every module that has no `uses` edge, and its items are then counted as the crate root's (this produced two false groups, codec and dedup, on 2026-11-08). `--no-fns` and `--no-types` must not be used: they remove the function and type nodes that carry most cross-module edges. The first Linux run timed out at 400 seconds, because `cargo-modules` runs a full `cargo check` (it rebuilt the Qt C++ bridge). The warm-target time has not been measured.
- **What the compiler graph cannot see.** Two blind spots. (1) cxx-qt `#[cxx_qt::bridge]` modules are macro output, and `cargo-modules` does not list them (`shell::qobject` and `Shell` are missing), so it cannot see edges into or out of them. The `pg-desktop` groups A to C and J appear only in the text scan. (2) The text scan used to treat an inline `mod name { … }` block as part of its file. The scan now makes each inline module a node (`tools/modgraph/module_graph.py`), which finds the cycles between a file and an inline child (`dir` and `dir::unix`, `picker` and `picker::native`).
- **Re-measured results (at `7ba93e9`, 2026-11-09, clean tree).**
  - Text scan: 19 groups (`desktop` 14, `app` 2, `catalog` 1, `platform` 2), identical on both targets. Groups A to J are as in the table below. Group J lists 15 modules but counts 16, so `viewport` is added.
  - Compiler graph, Windows: 6 groups (`catalog` E and G; `app` D and F; `platform` `dir`/`dir::windows` and `picker`/`picker::native`).
  - Compiler graph, Linux (`pg-dev`, repository mounted read-only): 5 groups (`catalog` E and G; `app` D and F; `platform` `dir`/`dir::unix`). There is no `picker` group on Linux.
  - The union of both targets equals the compiler-side groups in the table below. `pg-desktop` shows 0 groups on both targets (the cxx-qt blind spot).
  - `unreachable_pub`, Windows: `pg-video` 1,223 (bindgen, generated), `pg-desktop` 181, `pg-cli` 6, `pg-app` 2, `pg-edit` 2, `pg-platform` 1 (a Windows-only item). Linux: `pg-video` 1,165, `pg-desktop` 181, `pg-cli` 6, `pg-app` 2, `pg-edit` 2, `pg-platform` 0. `pg-edit` is 2 on both, not 1 as before.
  - Catalog scan and compiler disagree for a known reason: the scan counts a parent's `pub use child::*` and the children's `use crate::Catalog` as edges, and misses `dups`; the compiler includes only some glob re-exports. Do not treat the two views as agreeing. The gate is the union.
- **Platform-specific modules.** The Linux and Windows graphs differ by `cfg`. A cycle in a `cfg` branch counts, so both targets are the gate for `pg-platform` and `pg-desktop`.
- **Dev-dependency cycles.** With dev edges, `cargo metadata` shows one strongly connected component of eight crates: `analysis`, `codec`, `video`, `metadata`, `catalog`, `platform`, `jobs` and `testkit`. They are all connected through `testkit`, which dev-depends on crates that dev-depend on it.
- **`unreachable_pub` (M-18), counted with `cargo rustc --lib --profile check -- -W unreachable_pub` per crate.** `pg-video` 1,165 (bindgen output under `target/`, generated); `pg-desktop` 181 (plan 178), mostly cxx-qt bridge declarations (`type QAbstractListModel;`, `pub struct …Rust`) that the macro needs, plus about 40 ordinary `pub` items (for example `app::open`, `app::get`, `command_line::value`, `panic_log::install`, `picker::take`); `pg-cli` 6; `pg-app` 2; `pg-edit` 1; every other library crate 0. `pg-bench` and `pg-gallery` have no library target and are ignored by the script.
- **Call sites (2026-11-09).** `with_catalog` 20 (plan 18), `with_maker` 6 (plan 5). `pg_model::num` has 14 consumers (plan 15).
- **Rustdoc JSON** was ruled out: it covers public API only, not the references inside function bodies.
- **`pg-model` stays one crate.** Its 23 modules are consumed by 1 to 15 crates each (`num` by 15, `turns` by 14, `orientation` by 13, `rules` by 12). It is the shared vocabulary; its fan-in is expected. The two modules with one consumer (`min_side`, `group_distance`) are macros that keep a Rust constant and a SQL string in sync, so they stay.
- **`catalog`'s `analysis` dependency** is used only by `fixture::versions()` and by catalog's own tests (see M-17).
- **Re-measured at `905561d` (2026-11-09).** Text scan: 19 groups, unchanged. Compiler graph: Windows 6 groups and Linux 5, whose union is the compiler-side list above; `pg-desktop` is 0 on both. `unreachable_pub`: `pg-video` 1,165 (generated), `pg-desktop` 181, `pg-cli` 6, `pg-app` 2, `pg-edit` 1 (2 at `7ba93e9`; the M-18 count settles it), `pg-platform` 0 on Linux and 1 on Windows, the rest 0. Call sites: `with_catalog` 20, `with_maker` 6, `pg_model::num` 14 consumers. The catalog's scan reports one group of twelve modules; the compiler reports groups E and G. They differ for the reasons given in M-3, so they are not agreeing views. Dev edges: one strongly connected component of eight crates, as above. Sizes that moved are in the M-12a table and the sizes note.

### The working tree moves

Concurrent feature work lands in the files this plan splits. 111 files changed between `4d0ed46` and `905561d` (+7,456 lines), including `writeback/src/lib.rs` (+650), `thumbcache/src/lib.rs` (+387), `metadata/src/lib.rs` (+210), `edit/src/rewrite.rs` (+228), `jobs/src/pipeline.rs` (+223), `yandex/src/lib.rs` (+230) and `platform/src/native.rs` (+289). Every size, line number and group in this file predates `cb123b9` and `905561d`. Ground rule 7 governs how each split lands.

## What was checked

- **Crate graph** (`cargo metadata`): no cycles. The layering in `crates/README.md` holds, except the two edges M-17 removes.
- **Module graph:** ten cycle groups (A to J below), from the union of the compiler graph (both targets) and the text scan, each checked in the source. Group J alone is eleven parent-bridge pairs in the desktop crate. (1) `super::` inside a cxx-qt `pub mod qobject` block refers to its own file's module, not the crate root. (2) The text scan handles the brace and glob re-export forms (`pub use pair::{…}`, `pub use grid::*`). (3) The scan sees inline modules; the compiler does not see cxx-qt bridges; the union covers both.
- **Public surface:** `pub` items per crate: `client` 105, `model` 94, `catalog` 89, `platform` 53, `desktop` 37, `app` 35.
- **Checks are heuristic where stated.** The scratchpad split-check scripts match a function by bare name (`x(` or `.x(`) and skip common names (`new open close drop push next put get clear flush write read commit default insert len from into run trim clone eq fmt iter map take find detect`). `detect`, `trim`, `find` and `put` are therefore blind spots in M-8, M-10 and M-11. The hand checks in M-10 and M-11 cover them. None of these checks is proof: the compiler-backed `modgraph` run after each step is the check.

## Ground rules

1. **Symbols, not line numbers.** The tree moved since the plan was written (`Catalog::file_state` is at `catalog/src/lib.rs:1053`, not 1074; `trait Source` at `app/src/yandex.rs:34`, not 27). Name the function or type in every step.
2. **Re-exports.** A `pub use` at an old path stays only where the item is part of the crate's documented public API, or a documented layer boundary (`pg_app::reexport`, which the desktop uses because it does not depend on `catalog`, `library`, `jobs`, `thumbcache`, `yandex` or `codec`). Every other transitional re-export (M-7 to M-11, M-19, M-20) is removed in the same step with its callers. M-16 checks it.
3. **Test support.** A test-support feature in a production crate is acceptable only when a dev-dependency crate would be a cycle or need private access. Today: `pg-codec` `fixtures`, `pg-catalog` `fixture`, `pg-writeback` `fault`. Each stays. Everything else (for example `Quiet` in `pg-jobs`) goes in a dev-only crate (M-14).
4. **Large files.** Split when a file holds more than one layer of a dependency graph, not by line count. M-12a assesses the files over 500 production lines.
5. **Layer claims are by reading or by script, and neither is proof.** The scratchpad scripts matched functions by bare name and skipped common ones (`new open close drop push next put get clear flush write read commit default insert len from into run trim clone eq fmt iter map take find detect`); `detect`, `trim`, `find` and `put` are therefore blind spots (M-8, M-10, M-11). The committed form is `tools/modgraph/split_check.py`, with the skip set in its docstring. The check that counts is the compiler: the `modgraph` run after each step.
6. **Platform rule.** No `cfg(windows)`, `cfg(unix)` or `std::os::{windows,unix}` in production code outside `pg-platform`, except cxx-qt and FFI glue that must declare per-platform signatures. Each such exception is stated in M-13.
7. **Ordering.** The tree changes under the plan (111 files, +7,456 lines between `4d0ed46` and `905561d`, mostly in the files M-7 to M-9 split). Each split step is done in one sitting on the current HEAD, its measurement re-run first, and committed before anyone else's change to that file lands. The M-0 gate runs on a clean tree. No branches.

## Gates

- **Linux:** `docker run --rm --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/root/.cargo/registry pg-dev` (`docker/README.md`). It runs `docker/check.sh` in CI's order. It covers every crate except the desktop app.
- **Windows:** `just lint` and `just test`. The desktop app builds only here, with `just build`.
- **Desktop steps** (M-1, M-4, M-6, M-12, and the M-19 and M-20 desktop moves): also `just shots`. These are pure moves, so the screenshot matrix must show no pixel change against `tools/shots/reference`.
- **Cargo doc:** `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked` (`.github/workflows/ci.yml`). It failed on 2026-11-08 on links to Windows-only items (S-15 in `docs/REFACTOR_PLAN.md`). Run it on each step that moves or re-exports items.
- **Perf** (M-4, M-7, M-8): `just perf` runs callgrind (`Justfile`). It runs in `pg-dev`. The baseline is `docs/perf/baseline.json` (S-16). The first perf run at M-0 checks that baseline against the current tree.

## Decisions (resolved)

1. **Perf gate.** WSL is being migrated to Docker, and the `pg-dev` container has the bench tools. This plan makes no Dockerfile change for perf.
2. **M-15, `pg-library`.** Split `crop` and `borders` into `pg-crop` and `pg-borders`; the integration tests stay in `pg-library` (see M-15).
3. **M-4, `library_cell`.** It moves into the library service.
4. **`pg-model`.** Stays one crate.
5. **`cargo-modules`.** It is a tool, not a dependency, pinned in `docker/Dockerfile` and the Justfile (`cargo_modules_version`, which `just modgraph` checks and installs at the pin). The Linux graph runs in `pg-dev`; the Windows graph runs on the host through `just modgraph`.
6. **Command line and `pg-app`.** The command line does not depend on `pg-app`. It opens catalogs and thumbnail caches through `pg-library`'s shared open functions (M-4b).

## M-0: make the check repeatable, and make it the compiler's

- **Done:** the module-graph scripts are in `tools/modgraph/` (`module_graph.py`, `make_html.py`; standard library only, lint-clean under the repo's ruff). Output goes to `target/modgraph/`.
- **Done:** the split-check script (`tools/modgraph/split_check.py`, the portable form of the scratchpad scripts, with the skip set in its docstring) and the `unreachable_pub` counter (`tools/modgraph/unreachable_pub.sh`). Both were committed in `c1e0253`.
- **Cycle gate:** the union of (a) the compiler graph on Linux (`pg-dev`) and on Windows (the host, `just modgraph`), with only the `uses` edges, grouped by module (items map to their enclosing module; `(root)` is the crate root), and (b) the text scan, which covers the cxx-qt bridge modules. A group in either list fails the check unless it is on the allowlist.
- **Allowlist:** `tools/modgraph-allow.txt`, read by `tools/modgraph/allowlist.py`. Each line names `source` (scan, linux or windows), the crate and its members. `module_graph.py` and `compiler_graph.py` both fail on a group that is not listed for their source, and on a listed entry that matches no group. Groups H and I exist on one target each, so a fix on one target must not leave a stale entry for the other. M-1, M-3, M-4, M-5, M-6, M-19 and M-20 remove entries; M-16 empties the file. Both checks were run at `7ba93e9`: the text scan and the compiler graph pass on Windows, and the compiler graph passes on Linux.
- **Blocked: the perf baseline check.** `just perf` runs through `wsl -d`, and this repo does not use WSL. The `pg-dev` image has no valgrind, so `tools/perf-check.sh` cannot run there either. This needs a Docker path before M-4's perf gate, and the baseline check is not run.
- **Known false positives:** items that a crate re-exports at its root (`pub use app::App;`) appear as root items, so a group can show `(root)` without a real edge to the root. Check each group in the source before it goes on the allowlist.
- Add the `just modgraph` recipe (both scripts, in order).
- Re-measure every number in this file on a clean tree, first, before any other step. Done at `7ba93e9` (see Investigation results); the numbers above are from that run.

## Cycles

Ten groups. "Compiler" means `cargo-modules` on the target shown; "scan" means the text scan, with inline modules as nodes.

| Group | Members | Found by | Back-edges (checked in the source) |
|---|---|---|---|
| A (desktop) | root, `app`, `command_line`, `panic_log`, `providers`, `screenshot` | scan only (cxx-qt bridges) | `panic_log`'s hook calls `crate::message_box` (one edge) |
| B (desktop) | `controller`, `deadline`, `folders`, `shell` | scan only (cxx-qt bridges) | `deadline` uses `shell::qobject::Shell` (one edge) |
| C (desktop) | `facet_choices`, `filters` | scan only (cxx-qt bridges) | `facet_choices` calls `filters::count` (one edge) |
| D (app) | `app`, `images`, `library`, `reader` | compiler (both targets), scan | `impl App` blocks in six modules; see M-4 |
| E (catalog) | root, `dups`, `pair`, `schema`, `writes` | compiler (both targets), scan (partly) | see M-3 |
| F (app) | `yandex`, `yandex::replay` | compiler (both targets), scan | `replay` uses `super::Source` (defined in `yandex`); `yandex` re-exports `replay::{Recording, Replay}` |
| G (catalog) | `browse::read`, `browse::read::facets`, `browse::read::grid` | compiler (both targets), scan | `facets` and `grid` use `super::scope` (defined in `read`); `read` re-exports both children with `pub use …::*` |
| H (platform) | `dir`, `dir::unix` (Linux); `dir`, `dir::windows` (Windows) | compiler (per target); scan missed (inline modules) | the inline `unix` and `windows` modules use `super::{Entry, Lister}`; `dir` re-exports `unix::Dir` / `windows::Dir` |
| I (platform, Windows) | `picker`, `picker::native` | compiler (Windows); scan missed (inline module) | `native` uses `super::{FolderDialog, ShellItem, shell_path}` |
| J (desktop, cxx-qt) | `X` and `X::qobject` for X in `animation`, `command_line`, `facet_choices`, `filters`, `folders`, `gallery`, `keymap`, `library`, `picker`, `settings`, `shell`, `trace`, `video_source`, `viewport`, `words`, `yandex` | scan only (cxx-qt bridges) | each bridge's first line `type X = super::XRust;`, and `impl qobject::X` in `X`; see M-20 |

Groups A, B and C also contain their `qobject` modules, so their back-edges into `qobject` are part of J. M-1, M-5 and M-6 remove the non-bridge back-edges only; M-20 removes J. `trace` is not in group A (an earlier list had it because of a false edge).

### M-1: group A, and one error box

- **Root back-edge.** `panic_log` reaches the root through `crate::message_box`. The root's `fatal` also calls `message_box`, which is the same function. The earlier plan said `fatal` already calls `pg_platform::message::error_box`; it does not.
- **Fix:** one `error_box` wrapper in a leaf module of the desktop, with the title `"Photo Gallery"` included. `fatal` and the panic hook both call it. `message_box` is deleted from `crates/desktop/src/lib.rs`. Deleting it and calling `pg_platform` from both places would repeat the title.
- **Logger rename.** `panic_log` is the desktop's logger under the wrong name. `note` is called 33 times from 14 modules (`gallery` 7, `shell` 4, `yandex`, `providers`, `library`, `controller` 3 each, `picker`, `lib`, `app` 2 each, and one each in `video_source`, `folders`, `filters`, `command_line`, `animation`); `install` is called once. Create a leaf `log.rs` (`note`, the `LOG` static, `append`, `local_time`). `panic_log.rs` keeps `install` and `report`, and depends on `log` and the `error_box` wrapper. The 33 call sites change to `log::note`.
- Expected result: group A is gone. Confirm with the corrected scan.
- **Result (committed as `170f8e2`, after the M-0 commit `c1e0253`):** the root edge and the `panic_log` edges are gone, and the scan no longer reports group A. What remains is `command_line <-> command_line::qobject`, the bridge pair of group J (M-20), which the allowlist now lists in its place. The scan had to be fixed first: it read only tracked files, so the new `log.rs` and `alert.rs` were invisible to it (fixed in `module_graph.py`, which now includes untracked files).
- **Gates:** `just build`, `just lint-windows`, `just lint-qml` and `just test` pass. `just shots` against the reference recorded at `c1e0253` (`tools/shots/reference/c1e025319d`): 21 of 21 scenarios same, on two consecutive full runs.
- **The `save-view` difference was a caret, and the earlier fix did not hold.** The 14 pixels are the name field's text caret: the `save-view` shot had it, the base did not. Commit `32bc9b7` bound the field's `cursorVisible` to `caret` (off with `--screenshot`), but Qt's TextInput sets `cursorVisible` itself when the field gains focus, which breaks that binding, so the caret still blinked into some shots. The fix is `cursorDelegate: dialog.caret ? null : noCaret` in `SaveViewDialog.qml`: a photographed window gets an empty caret, and every other window gets the default (`null`). Two full runs match all 21 scenarios. The fix is committed as `3be8edf`.

Gate: desktop gates.

### M-2 (removed)

Once M-1 is done the root has no back-edges. The root keeps `run`, `fatal`, and the module declarations.

### M-3 and M-3a: group E and group G (catalog)

Every catalog module that names `crate::Catalog`, `crate::Result` or a root helper closes a cycle back to the root, where those items live. The back-edges, all checked in the source:

- `pair`: a glob import of the whole root (`use super::*;`) in production code. Replace it with explicit imports. `crate::Error::Other` becomes `crate::error::Error::Other`.
- `dups`, `writes`, `browse/read/*` and `browse/writes.rs`: `use crate::Catalog` becomes `crate::core::Catalog` (M-7), and `use crate::Result` becomes `crate::error::Result`.
- `writes`: `use super::{DELETE_FILE_RATING, SET_FILE_RATING};`. Both constants move to `types.rs` (M-7).
- `schema`: `use crate::{Error, Result, write_transaction};`. `Error` and `Result` come from `crate::error`. `write_transaction` moves into `schema.rs`; the root and `writes` import it from there. `crate::rules_name` moves to `types.rs`.
- `schema`: `crate::query::to_json`. The root has `pub use pg_query as query;`, which makes every `crate::query::…` path (18 uses in `schema`, `browse/read/{facets,grid,views}`, `browse/writes.rs`, `browse/fixtures.rs`) an edge to the root. Delete the alias. Use `pg_query::` inside the catalog. Give `pg-app` a direct `pg-query` dependency; its one outside user is `app/src/views.rs`, which is `pg_catalog::query::{from_json, to_json}`. `pg-query` is a lower layer and goes in the M-16 layer file.
- `Catalog::file_state`, a root wrapper whose body is `browse::read::file_state(self, file_id)`. It is a root-to-`browse::read` edge that the children's `Catalog` import turns into a cycle. Delete the wrapper. Its only production caller is `pg-thumbs` (`thumbs/src/lib.rs`), which calls `pg_catalog::browse::read::file_state(&catalog, id)` directly (`app/src/images.rs` does this already).
- The root's `pub use pair::{Coverage, Pairing};` stays.
- **M-3a, group G:** `scope` moves to `browse/read/scope.rs`. `facets` and `grid` import it from there; `read` stops being their parent for that helper. `facets`'s import of `super::grid::FILE_JOINS` is a sibling edge and stays.
- **Expectation:** after M-3, M-3a and M-7, both the scan and the compiler views of groups E and G are empty. Record that expectation here, and make the gate the union (a group in either tool fails).
- **Result (committed as `ebe42a2`, with M-3a; the M-1 commit `170f8e2` and the caret commit `3be8edf` come before it):** `pg_query` alias deleted (callers use `pg_query::`; `pg-app` depends on `pg-query`); `pair`'s glob import replaced; `write_transaction` moved to `schema`; `Catalog::file_state` wrapper deleted (`thumbs` calls `browse::read::file_state`); `scope` moved to `browse/read/scope.rs`. Group G is gone in both views. Group E is now `(crate root), pair` on both targets: `pair` implements `Catalog` (`impl Catalog`), and `schema` no longer closes a cycle. What is left (`pair`'s `impl Catalog`, and the root's `Catalog`, `rules_name` and the file-rating constants) is M-7's move, so the allowlist entry is narrowed, not removed.
- **Gates:** `just lint-windows` and `just test` pass. The Linux container's `cargo fmt --check`, `clippy -D warnings`, `cargo doc` and `cargo test` all pass as root, but the permission-based tests skip as root and `docker/check.sh` fails on any skip. The local `pg-dev` image has no `pg` user (it predates the Dockerfile's `useradd` and `USER pg`), so the Linux pass has to be rerun after the image is rebuilt from `docker/Dockerfile`.

Gate: Linux checks, Windows `just test`.

### M-4: group D, `app` as services

**What the code does.** `App` (`app/src/app.rs`) holds the catalog keeper and pool, the maker pool, the thumbnail cache, the last full-size decode, the preferences, the reader and session, the library cell, and the notes bus. Six modules add `impl App` blocks: `app`, `edit`, `images`, `library`, `reader`, `views`.

**Measured:**

- `with_catalog` (uses `catalogs`, `config`) has 20 call sites in `app`, `edit`, `images`, `library`, `reader` and `views`. It is the main coupling.
- `with_maker` (uses `makers`) has 6 call sites, all in `images`.
- `library` methods use `library_cell`; `stop` calls `stop_library`; `edit::rate` calls `wake_writer`.
- `images` uses `thumbs` and `last_decode`; `edit` uses `prefs`; `reader` and `views` use no fields directly.
- Callers outside `crates/app`: about 40 sites, all in `crates/desktop` through `app::get()` (11 of them are `.prefs`). `cli` has none.

**Why not a shared bag.** A `Resources` struct holding all shared state would break the cycle, but every domain module would still depend on one object that holds everything. That is the cycle moved, not removed.

**Services.** Each is a module, and each depends only on services above it in this table, so cycles cannot form:

| Service | Owns | Uses | Methods (from the measurement) |
|---|---|---|---|
| `config` (existing) | `Config` | — | — |
| `notes` | the `Note` type and the notes bus | — | `notes` |
| `makers` | the maker pool | — | `with_maker` |
| `catalogs` | the catalog keeper, the pool, and the catalog path | `config` | `with_catalog`, `file_path` |
| `prefs` | the preferences store | `config`, `notes` | `prefs`, `update_prefs` |
| `images` | `thumbs` (the `&'static ThumbCache`), `last_decode`, and the `Decoded` type | `catalogs`, `makers` | `thumbnail`, `keep`, `turn_jpeg`, `placeholder`, `full_image`, `consistent`, `source`, `decode` |
| `library` | `library_cell` | `catalogs`, `makers`, `images`, `prefs`, `notes`, `config` | `library`, `wake_writer`, `save_crops`, `roots`, `add_folder`, `remove_folder`, `rescan`, `turn`, `stop_library` |
| `reader` (existing `Reader`, `Session`) | the reader thread and the session | `catalogs` | `details`, `entry` |
| `views` | — | `catalogs` | `session_value`, `listing`, `save_listing`, `views`, `save_view`, `delete_view`, `view` |
| `edit` | — | `catalogs`, `prefs`, `library` | `rate`, `set_border_mark` |

**How a service reaches the ones it uses.** Each service is built in dependency order and holds `Arc` handles to the services below it (`Library` holds `Arc<Catalogs>`, `Arc<Makers>`, `Arc<Images>`, `Arc<Prefs>`, `Arc<Notes>`), never a handle to `App`. The lazy library becomes a `OnceLock` inside the library service, which takes those handles at first use. `App::stop` and `App::shutdown` keep the cross-service order (reader and library, then the thumbnail cache, then catalogs, with the keeper last), in the composition root only. The module comment in `app.rs` records that order.

**Delegates are deleted.** `App` owns the services and exposes them (`app.library()`, ...). It keeps no one-line delegates for each method. The callers outside `crates/app`, about 40 sites, change in the same step. `path_of` (`app.rs`) has no `App` dependency and moves to a leaf. `Note` names `pg_catalog::Rebuilt` and `pg_thumbcache::Event`, so `notes` is a leaf only with respect to this crate's modules.

**Re-exports.** Lower-crate types in service signatures that the desktop needs (for example `Progress`, `Facets`, `ThumbEvent`) are added to `pg_app::reexport` in the same step. Otherwise the desktop would gain a direct dependency.

**`yandex`** stays a domain module. Its `start_folder` and `save` take the services they need, not `&App`.

**The desktop keeps `app::get()`.** Qt default-constructs the cxx-qt objects, so they cannot take dependencies. A `OnceLock` gives them the app.

The layer check (M-16) enforces the DAG. The exact split of `images` and `library` is read during the step.

Gate: Linux checks, Windows `just test`, `just shots` (the desktop's images come from here), perf (decision 1).

### M-4b: one place that opens a catalog folder

The command line does not use `pg-app`, and should not. `run_command` opens one mutable `Catalog`. `scan`, `thumbs` and `write` each open their own `ThumbCache` with the command's cap and a printer thread for its events; `write` leaks its cache so events print to the end. `App` keeps a connection pool, a keeper, a leaked cache, a reader and preferences. Bending both to one shape is worse. What the two share is how a catalog folder is opened.

- **Add to `pg-library`:** `open_catalog(dir)` (`catalog.sqlite` in the folder, `versions()`, `PathRules::NATIVE`) and `open_thumbs(dir, catalog_id, cap, events)`. `versions()` and `default_catalog_dir()` already live there.
- **Callers:** `App::open` and the four `cli` sites (`Catalog::prepare` in `run_command`; `ThumbCache::open` in the scan, thumbs and write commands) call them. `cli` keeps its control flow and does not depend on `pg-app`.
- **README.** `crates/README.md` calls `app` "the UI-free application layer the front ends share", which is false for the command line. Change it to: "the application layer the window uses; the command line drives the passes directly through `pg-library`".

Gate: Linux checks, Windows `just test`.

### M-5: group C

`filters::count` has one caller: `facet_choices`. Move `count` into `facet_choices`, which owns the `Counts` type it returns. Then `facet_choices` no longer depends on `filters`. The reverse edge (`filters` uses `Counts` and `set_counts`) stays.

Gate: desktop gates.

### M-6: group B, the deadline timer

Remove both globals, not one:

- `deadline.cpp` holds a process-wide static `QTimer` whose `timeout` runs a context-free lambda that calls `deadline_fired()`.
- `deadline.rs` holds a thread-local `SHELL: RefCell<Option<CxxQtThread<Shell>>>`, which is the only reason `deadline` imports `crate::shell::qobject::Shell`.

**Design:** `deadline_arm` takes the shell's `QObject*`. The timer is parented to it, and its `timeout` queues a call to the shell's `timeout` invokable (`QMetaObject::invokeMethod`, queued connection). Then `deadline_fired`, the thread-local and the `Shell` import all go.

**Fallback:** a boxed closure only if cxx-qt cannot hand C++ the object pointer. If that happens, record why in this section.

Gate: desktop gates, including `just shots`.

### M-7: group E, and the `catalog` split

`crates/catalog/src/lib.rs` has 3,090 lines; 1,547 are production code at `905561d` (1,534 at `7ba93e9`; see the sizes note at the end).

Layers, checked as a DAG method by method:

- `types.rs`: `Rebuilt` (with `carried_note`), `under_params`, and the shared types and constants (`Root`, `Folder`, `ScannedFile`, `WorkItem`, `Status`, `Hit`, `Versions`, `Carried`, `CropSource`, `Recorded`, `folder_path`, `rules_name`, `DELETE_FILE_RATING`, `SET_FILE_RATING`, and the SQL constants). A leaf.
- `lifecycle.rs`: rebuild, move-aside, locks, rules check, version stamps, `max_folder_id`, `pending_import_sql`, `work_list`. Uses `types`.
- `writers.rs`: `ScanWriter`, `AnalysisWriter`, `BordersWriter`, `write_file`. Uses `types`.
- `core.rs`: `Catalog`, `Tx`, and their methods. Uses `lifecycle`, `writers`, `types`, and `error`. It must not use `browse`, `writes`, `dups` or `pair`; the one production call into `browse` (`Catalog::file_state`) is removed in M-3.
- `schema.rs`: `write_transaction` (moved here from the root).
- `writes.rs` stays the file-write queue. `lib.rs` keeps the module declarations and the re-exports (`pub use core::{Catalog, Tx}`), so callers' paths do not change.
- `browse/read/scope.rs`: the `scope` helper (M-3a).

`Rebuilt` and `under_params` go to `types.rs`, not `lifecycle.rs`. Otherwise `lifecycle` and `core` form a cycle (`rebuild` uses `Rebuilt`, and `Catalog` uses `lifecycle`), and `writers` uses `under_params`.

Check the result with `cargo-modules` (M-0): groups E and G must be gone from its output.

Gate: Linux checks, Windows `just test`, perf (decision 1).

## Layering

### M-17: edges that point the wrong way

- **`catalog` → `metadata`.** The only production use is `writes.rs`, which calls `can_hold_rating(format: Format) -> bool` (`metadata/src/lib.rs`). That function is one line, `!matches!(format, Format::Bmp)`, and depends only on `pg-model`'s `Format`. Move it to `pg-model`, next to `Format` (`model/src/format.rs`). `writeback` callers change too. Then remove `pg-metadata` from `catalog/Cargo.toml` entirely. `schema.rs` mentions `pg_metadata::rotation_method` in an SQL comment; the comment names that function's real home after the move.
- **`catalog` → `analysis`.** Optional, behind `fixture`. The fixture takes the two version constants from its callers (`fixture.rs`, `fixture/raw.rs`). Catalog's own tests already take `analysis` as a **dev-dependency** (no cycle: `analysis` does not depend on `catalog`). So the optional normal entry goes: remove the optional `pg-analysis` entry. The `fixture` feature stays for its private connection, and it no longer needs `analysis`.

Gate: Linux checks, Windows `just test`.

### M-18: public surface, checked by the compiler

Turn on rustc's `unreachable_pub` lint for the workspace (`[workspace.lints.rust] unreachable_pub = "warn"` in the root `Cargo.toml`; every crate already opts in). Measured scope (see Investigation results):

- Generated code (`pg-video`'s bindgen output): exclude it at its include site, not by editing the output.
- cxx-qt bridge declarations in `pg-desktop`: these need a bridge-level allow, or a check that the macro compiles with them narrowed. Verify on Windows, since the desktop builds only there.
- Ordinary items: about 40 in `pg-desktop`, 6 in `pg-cli`, 2 in `pg-app`, 1 in `pg-edit`. Narrow each to `pub(crate)` unless another crate uses it. Items used only through a `pub use` that is itself unused are deleted.

Gate: Linux checks, Windows `just build` and `just test`.

### M-20: the cxx-qt backing types (group J, desktop)

Each cxx-qt bridge is a `pub mod qobject` inside a desktop module X, and its first line is `type X = super::XRust;`: the Rust struct that backs the QObject lives in X, and X implements the bridge type (`impl qobject::X`). Each pair is a cycle. The scan finds 16 modules (the group J list; `viewport` included).

The `super::` paths in each bridge are that one line. The other `super::` hits in these modules (`command_line`, `keymap`, `picker`, `video_source`, `shell`, `yandex`) are `use super::*;` in `#[cfg(test)] mod tests`, which the scan excludes. Nothing more to check per module than whether `XRust` uses an item still in X.

**Fix, per module X:**

- Move `XRust` (its fields and the methods the bridge calls) into a leaf module, `X/state.rs` or a sibling file. The bridge names it by its crate path (`type X = crate::<leaf>::XRust;`).
- X keeps `impl qobject::X` and the public functions. That gives X → qobject → leaf, with no edge back.
- If `XRust` uses an item still in X, move that item to the leaf too. Check each one. Move the item and change its callers.

Gate: the desktop builds only on Windows. Windows `just build` and `just test`, and `just shots` with no pixel change against `tools/shots/reference`. Run `cargo-modules` (M-0) on the Windows target afterwards; the scan must show no J group.

### M-19: the parent-child cycles (groups F, H, I)

Three groups have the same shape: a parent module defines a trait or types, its child implements them, and the parent re-exports the child. Each is cut the same way: move the shared items into a new leaf module that neither side depends on upward.

- **F (app, `yandex`).** Move `pub trait Source` (`app/src/yandex.rs`, with its three methods; the old plan said four) into `crates/app/src/yandex/source.rs`. Its signatures use only `pg_yandex` types, so the leaf has no upward dependency. `yandex.rs` keeps `pub use source::Source;` and `impl Source for Client`; `yandex/replay.rs` changes its import to `use super::source::Source;`. The re-export `pub use replay::{Recording, Replay}` stays, because the parent no longer depends on the child through the trait.
- **H (platform, `dir`).** Move `Entry` and `Lister` out of `dir.rs` into `crates/platform/src/dir/listing.rs`. `dir.rs` re-exports them, and both inline `unix` and `windows` modules import from `super::listing`. First read what `Entry` and `Lister` use from `dir.rs`; any item they use moves too.
- **I (platform, `picker`, Windows).** The Windows `native` module imports `FolderDialog`, `ShellItem` and `shell_path` from `picker`. Moving only the two traits would leave `picker` ↔ `native` as a cycle. Instead, the leaf `crates/platform/src/picker/shell.rs` holds `FolderDialog`, `ShellItem`, `shell_path` and `path_of_units` (pure UTF-16 functions, whose tests move with them). `native` imports only from the leaf. `picker` → `native` stays legal: `failed_call` downcasts `native::CallError`.

Gate: Linux checks; Windows `just build` and `just test`; both `cargo-modules` runs (M-0), which must show no group F, H or I on either target.

## Large single-file crates

Each split is a move. Items keep their visibility, and public paths stay the same through `pub use` where rule 2 allows it. Every split below was checked as a DAG at group level, method by method, by reading. Re-run the check at M-0 and before each step.

### M-8: `thumbcache`

Layers:

- `types.rs`: `FORMAT_JPEG`, `Source`, `Thumb`, `ThumbWork`, `Pending`, `Snapshot`, `Event`, `Usage`. A leaf. `writer` and `reader` both use `Event`, `Source` and `ThumbWork`, so these must not stay in `lib.rs`.
- `config.rs`: the cap functions and constants (`default_cap_bytes`, `parse_cap_gb`, `MAX_CAP_BYTES`, `MIN_DEFAULT_CAP_BYTES`, `MAX_DEFAULT_CAP_BYTES`, `whole_bytes`, `cache_path`), and the timing constants `TOUCH_AGE_S`, `TOUCH_FLUSH`, `HEAL_INTERVAL`, and `CACHE_VERSION`. The schema and the writer both read these.
- `maintenance.rs`: `heal`, `heal_if_due`, `shrink` and `unix_now`, with their constants (`SHRINK_*`) and `Drift`. These maintain the cache database and use nothing of the write loop. They sit below both `schema` and `writer`. Put here, the open path (`ready` calls `heal_if_due`) no longer depends on the write loop. `unix_now` is also used by `lib.rs`.
- `schema.rs`: `SCHEMA`, `APPLICATION_ID`, `SEEDS`, `CONNECTION_PRAGMAS`, `BUSY_TIMEOUT`, `Found`, `Why`, `damage`, `damage_in`, `Writing`, `Attempt`, `expected`, `make_layout`, `judge`, `open_writer`, `hold_shared`, `open_file`, `open_once`, `ready`, `move_damaged`, `read_found`, and `StartOverReason` (public) with the types.
- `reader.rs`: `pending_in`, `pending_sql`, `with_reader`.
- `writer.rs`: `write_loop`, `apply`, `touches_failure`, `put`, `total_and_cap`, `evict`, `Touches`, `Op`, `Inbox`, `Queue`, `Closing`, `Store`, and the eviction constants.
- `lib.rs`: `ThumbCache` and its methods (the orchestrator, which calls down into the other layers).

The check's cross-references that look like cycles are false: `reply.send(…)` is a channel, and `checked`, `cap` and `totals` in `SCHEMA` and `total_and_cap` are SQL names.

The 657 lines of tests (from `mod tests`) stay in `lib.rs` with `use super::…`.

Gate: Linux checks, Windows `just test`, perf (decision 1).

### M-9: `metadata`, by format

Layers (checked as a DAG):

- `xmp.rs`: the XMP namespaces, `XMP_WANTED`, `xmp_properties`, `xmp_value`, `percent`, `round_u8`, `Packet`, `MetaScratch`. `MetaScratch` and `Packet` stay here: `Container::walk` takes `&mut MetaScratch`, so moving it to `lib.rs` would make `container` depend on `lib`.
- `orientation.rs`: `orientation` (read), `patch_orientation` (write), `exif_block` and `tiff_orientation_patches`. Read and write use the same `exif_block`, so they stay together. It depends on `exif` and on `Tiff`.
- `tiff.rs` (to consider): `Tiff`, `Entry` and the TIFF constants (about 170 lines, EXIF-independent). `exif` and `orientation` depend on it.
- `exif.rs`: `Exif` and the EXIF tag constants.
- `container.rs`: `Container`, `extended_packets`, `MAX_INFLATED`, `inflate_zlib`, `ItxtXmp`, `png_itxt_xmp`, `gif_xmp`.
- `lib.rs`: `Meta` and its read (the orchestrator). `can_hold_rating` moves to `pg-model` under M-17.

Edges: `lib` → `{xmp, exif, container, orientation}`, `container` → `{xmp, exif}`, `orientation` → `{exif, tiff}`. Nothing calls back up. Gate: Linux checks, Windows `just test`.

### M-10: `codec`, by format

`codecs.rs` is 814 production lines. The plan as first written creates a module cycle, the same shape as group D: `Codecs` is the scratch-buffer struct, `decode` dispatches with `self.png(...)` and so on, and each format module adds an `impl Codecs` block, so `codecs` ↔ each format.

Layers (checked as a DAG):

- `scratch.rs`: `Codecs` (the struct, its constructor, and `trim`). A leaf.
- `convert.rs`: `to_rgb`, `flatten`, `convert`, the CMYK and float conversions, `luma8`, `luma16`, `rgb16`, `rgb_f32`, `target`, `zeroed`, `reserve`, `finish`, `check_len`, `over_white`, `read_with`, and `max_decode_bytes` (the PNG decoder and the dispatcher both use it).
- `errors.rs`: `png_error`, `gif_error`, `webp_error`, `jpeg_error`, `tiff_error`, `tiff_unknown_color`, `png_color`, `check_sample_format`.
- `png.rs`, `gif.rs`, `webp.rs`, `jpeg.rs`: one format each, with its `impl Codecs` block, importing from `scratch`.
- `tiff.rs`: the TIFF decoder, with `TiffPlan`, `Stored` (moved from `codecs.rs`: the TIFF code is their only user) and `TiffDecoder`.
- `codecs.rs`: `decode` only, the dispatcher.

Edges: `codecs` → each format; each format → `scratch`, `convert`, `errors`; `convert` and `errors` → nothing. `max_decode_bytes` stays reachable through `lib.rs`'s re-export. Nothing calls back. Gate: Linux checks, Windows `just test`.

### M-11: `analysis` border detector, layered

`borders.rs` is 693 production lines. Layers (checked as a DAG, by hand, because the script skips `detect`, `trim` and `find`; nothing below `detector` calls them):

- `params.rs`: `Params` and its `Default`, `BORDERS_VERSION`.
- `view.rs`: `View` and its methods, `to_count`. A leaf after `params`.
- `stats.rs`: `median2`, `hist_median2`, `median_f64`, `run_length`. Plain statistics, used by `SolidLines::measure_to` (scratch), `solid_side` (side) and `blur_threshold` (detector). `median2` depends on `view`; the other three are pure. They go here, not in `seam.rs`: the seam items use none of them.
- `scratch.rs`: `Detector` (its scratch buffers, `capacity_bytes`, `trim`) and `SolidLines`, the buffers the detector holds.
- `seam.rs`: `SEAM_SPAN`, `refine_seam`, `seam_straightness`, `DetailSums` and its methods. They use only `View` and `Params`.
- `side.rs`: `starts`, `blurred`, `detect_side`, `solid_side`, `blur_side`, `judge`.
- `detector.rs`: `Detector::detect`, `detect`, `detect_with`, `detection`, `blur_threshold`, `PASSES`. The top layer.
- `borders.rs` keeps the public re-exports (`detect`, `Params`, `BORDERS_VERSION`).

Edges: `stats` → `view`; `seam` → `view`, `params`; `scratch` → `stats`, `seam`, `view`, `params`; `side` → `stats`, `seam`, `scratch`, `view`, `params`; `detector` → all lower layers. Nothing calls back up. The one `const _` assertion (`PASSES`) stays with `detector`. The border tests run the detector on fixtures, so a changed result fails them. Gate: Linux checks, Windows `just test`.

### M-12: desktop `gallery.rs`

The invokables are inherent `impl qobject::GalleryModel` blocks, which Rust allows in any module of the crate. cxx-qt's generated shims in `gallery::qobject` call those methods, so a method moved into a child module needs `pub(super)` or wider visibility.

Split `gallery.rs` into `gallery/{model,items,edits,nav,roles}.rs`:

- `model`: model plumbing (`data`, `role_names`, `apply`, `show`, `relayout`, `items_changed`).
- `items`: item queries (urls, keys, size, `info`, aspects).
- `edits`: `rotate`, ratings, border marks, crop copies.
- `nav`: `row_of`, `neighbor`, `is_item`.
- `roles`: the `ROLE_*` constants, `CELL_ROLES`, `cell_roles`, `qt_int`.

The impl is about 600 lines in these four concerns, plus the constants.

The first action is a Windows `just build` after moving one method, to confirm the visibility rule. Whether the per-concern split holds is not yet verified; it is confirmed at that build.

Gate: desktop gates, including `just shots`.

### M-12a: the rest of the large files (new)

Split by dependency layers, not by line count. Read each file over 500 production lines and record "split" or "leave, with reason" in this section. Sizes at `905561d` (production lines, counted before the first line-start `#[cfg(test)]`):

| File | Lines | Note |
|---|---|---|
| `client/src/text.rs` | 1,447 | The second largest in the workspace and not in the old plan. The typed `Message` enum and its Fluent `render`, then about sixty label and line builders. Read first. |
| `video/src/ffi.rs` | 813 | Declarations |
| `writeback/src/lib.rs` | 756 | Grew by 650 lines since the old plan |
| `desktop/src/shell.rs` | 660 | |
| `desktop/src/yandex.rs` | 628 | |
| `catalog/src/schema.rs` | 624 | |
| `app/src/yandex.rs` | 584 | |
| `edit/src/rewrite.rs` | 566 | |
| `cli/src/lib.rs` | 566 | |
| `yandex/src/lib.rs` | 533 | |
| `platform/src/dir.rs` | 511 | |

`jobs/src/pipeline.rs`, `scan/src/lib.rs` and `query/src/lib.rs` fall under 500 and leave the list. Other sizes: `catalog/src/lib.rs` 1,547 production (old plan 1,534), `thumbcache/src/lib.rs` 1,302 (1,175), `metadata/src/lib.rs` 715 (655), `codec/src/codecs.rs` 822 (814), `analysis/src/borders.rs` 693, `library/src/crop.rs` 323 (204), `library/src/borders.rs` 83 (77).

## Platform boundary

### M-13: platform calls only in `pg-platform`

Production platform code outside `pg-platform`:

- `crates/video/src/ffi.rs` (line 192): the Windows branch of `Lib::open` loads the DLL with `libloading::os::windows`. This moves to `pg_platform::dll::load_with_search(path)`. `pg-video` keeps the FFmpeg symbol lookup. `pg-platform` already depends on `windows-sys`.
- `crates/gallery/src/main.rs`: `attach_console` calls `windows_sys` directly. This moves to `pg_platform::console::attach_to_parent()`.
- `crates/desktop/src/command_line.rs`: `value_exact`, an `OsString` split by platform. Moves to `pg_platform::names` with the path-to-native-units conversion.
- `crates/desktop/src/gui_app.rs`: `arg_bytes` moves to `pg_platform`.
- `crates/desktop/src/picker.rs`: `has_drive_or_share`, `native`, `run_dialog`, the UTF-16 path conversion, and the `USERPROFILE`/`HOME` lookups. The start-folder logic moves to `pg_platform::picker`.
- `crates/desktop/src/video_source.rs`: `play_path` and `fd_of`. `video_source` gets its player target (units or an open descriptor) from `pg_platform::file`.

After the moves, the README's "the one exception" (`crates/README.md`) is removed. Any remaining exception is a cxx-qt or FFI declaration that needs per-platform signatures, and it is stated where it is. M-16 enforces the rule by grep.

Gate: Linux checks (the platform crate's Linux code must still build), Windows `just test`, and the FFmpeg video tests on Windows.

## Dev-dependency cycles

### M-14: `testkit`, with no internal dependencies

The dev-dependency component of eight crates is connected through `testkit`. Both edges that tie `testkit` to `catalog` and `jobs` go:

- `pub use pg_catalog::fixture;` (`testkit/src/lib.rs`) has no users. Delete it. This removes the `catalog` edge.
- `events.rs` (the `Quiet` bus helper, which uses `pg_jobs::events`) moves to a new dev-only crate, `pg-jobs-testkit`, which depends on `pg-jobs` and `pg-testkit`. Its users are `cli`, `library`, `thumbcache` and `thumbs`, all as dev-dependencies. `pg-jobs` does not use `Quiet`, so the old plan's `testing` feature in `pg-jobs` is not needed.
- One `odd_name` helper in `pg-testkit` (standard library only, so `testkit` stays a leaf). It replaces the six copies written separately, each under its own `cfg`, in `app/src/yandex.rs`, `app/src/prefs.rs`, `desktop/src/command_line.rs`, `desktop/src/screenshot.rs`, `desktop/src/picker.rs`, and `exiftool/src/lib.rs`. This also removes the test `cfg`s that the M-13 grep would otherwise have to allow.

`testkit` keeps the scratch folders, `held.rs` and the file generators, with no internal dependencies. M-16 checks that the graph including dev edges is acyclic.

Gate: Linux checks, Windows `just test`. `cargo tree -d` must show no duplicate workspace crates in the test builds.

## Crate granularity

### M-15: `pg-library`'s reach

- **`dups` leaves `pg-library`.** `library/src/dups.rs` (60 lines) joins the catalog's duplicate reads with `pg_dedup::UnionFind`. Its only user is `cli/src/dups.rs`, and it has nothing to do with the job runner. Moving it into `pg-dedup` would give that pure crate a `pg-catalog` dependency. It goes to `pg-cli`'s `dups` module, next to the report that prints it. If a later multicut grows it, it gets its own crate then.
- **Dependencies removed.** The split removes four dependencies from `pg-library`: `edit`, `exiftool`, `writeback` (with `crop`, which moves to `pg-crop`), and `pg-dedup` (with `dups`).
- **New crates.** `pg-crop` (`crop.rs`, 323 lines) and `pg-borders` (`borders.rs`, 83 lines). The runner (`crop::save`, `borders::find`) depends on both. `cli` and the tests name `pg-borders` and `pg-crop` directly.
- **Tests stay in `pg-library`.** Five integration test files exercise the two passes: `tests/borders.rs`, `tests/crop.rs`, `tests/crop_roots.rs`, `tests/crop_folders.rs` and `tests/crop_tags.rs` (the last is new and untracked). All of them call `pg_library::versions()` to prepare a catalog. Moved into the new crates, they would dev-depend on `pg-library`, which depends on the new crates: the dev-dependency cycle M-14 and M-16 forbid. So they stay in `pg-library/tests` and import `pg_crop::` and `pg_borders::`. The new crates keep only unit tests that need no catalog.
- **What remains.** After the split `pg-library` is one coherent layer: `runner.rs` (519 lines) and `lib.rs`, plus `versions()`, `default_catalog_dir()` and M-4b's open functions. It keeps the `pg-writeback` `fault` dev-dependency and the codec, metadata, render and video dev-dependencies for the tests that remain.
- **Stale comment.** `pg-library`'s `Cargo.toml` says the passes are re-exported there under their old names. `lib.rs` re-exports nothing, and no caller uses those paths. Fix the comment in this step.

Gate: Linux and Windows checks.

## Closing

### M-16: enforce, at four levels

- **Module cycles:** empty `tools/modgraph-allow.txt`, so the check fails on any module cycle. Run the `modgraph` check in CI. CI cannot run it yet, so this step also: (a) installs `cargo-modules@0.28.0` in both jobs (through `taiki-e/install-action` if it carries the tool; otherwise `cargo install --locked` with a cache); (b) adds a `modgraph` step to the Linux job and to `docker/check.sh` (the Linux compiler graph plus the scan), and `just modgraph` to the Windows job (the Windows-only groups H and I show there only); (c) keeps the three pins equal (Justfile, Dockerfile, `ci.yml`). Measure the step's time first: the first container run exceeded 400 seconds, and the warm-target time was never measured.
- **Crate layers:** commit `tools/crate-layers.txt`, one line per crate listing its allowed normal internal dependencies. A draft from the current graph (30 crates, dependencies first) is in the session scratchpad as `crate-layers.draft.txt`. It is written after M-17 and includes the `app` services, `pg-crop`, `pg-borders`, `pg-jobs-testkit`, and `pg-app` → `pg-query`. CI checks the file against `cargo metadata`: a new internal edge fails until the file changes in the same commit.
- **Dev edges and re-exports:** the graph including dev edges is acyclic; no `pub use` outside the exempt list (ground rule 2); the platform grep (ground rule 6).
- **Documents:** update `crates/README.md` (the dependency section, the platform rule, `testkit`, the new crates, the `app` services, the moved `can_hold_rating`, and the `app` description from M-4b). Add a ledger row for each M step to `docs/REFACTOR_PLAN.md`, using the same status words. Record the final check results in this file.

## Order

1. **M-0:** re-measurement on a clean tree, committed scripts, the allowlist format and the crate-layer draft.
2. **M-1, M-3, M-3a, M-5, M-6, M-19, M-20 and M-17:** the cycle fixes and the layering. Then one Windows build and `just shots`, then one Linux run.
3. **M-4** (`app` services), then **M-4b** (the shared open functions), with the perf gate.
4. **M-7** (`catalog`). Groups E and G close only here, after M-3 and M-3a remove the edges that do not depend on the move.
5. **M-8 to M-11**, then **M-12** and **M-12a**. Each split is re-checked on current HEAD first (ground rule 7).
6. **M-13 and M-14**, independent of the above.
7. **M-15.**
8. **M-18** (`unreachable_pub`), after the splits, so the public surface reflects the final layout.
9. **M-16**, last.

## Open decisions

None. Unverified until their step, and confirmed there:

- M-12's per-concern split and the visibility rule (the first build in the step).
- M-6's hand-off of the `QObject*` to C++ in cxx-qt (fallback recorded if it fails).
- H's `Entry` and `Lister` dependencies on `dir.rs` (read first).
- The warm-target time of `cargo-modules` in CI (measured at M-0).
