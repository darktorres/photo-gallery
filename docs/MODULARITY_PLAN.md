# Plan: fix the modularity findings (M-series)

Status (2026-11-10): investigated, re-measured at `905561d`, and re-reviewed in twelve passes. The consolidated rewrite is applied. Every step, M-0 to M-20, is done and committed (each step says where). All of them pass the Windows checks and the Linux checks in `pg-dev` (see the "Validation" sections at the end). The perf gate shows no effect of the steps, and the baseline is re-measured at `3b46058`. Then rebased onto `origin/main`, whose parallel work included its own M-13 moves; the merged tree passes the same checks ("Validation 3"). A fourth review, of responsibilities rather than cycles, read every crate and found what the enforced checks cannot see: work done twice, code in the wrong crate, untyped values, and globals in place of references. Its steps, M-21 to M-49, are open, in [MODULARITY_REVIEW_4.md](MODULARITY_REVIEW_4.md). Churn and cost are not a constraint; the goal is the proper fix for each finding, not the cheapest one.

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

- **Linux:** `docker run --rm --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/root/.cargo/registry pg-dev` (`docker/README.md`). It runs `docker/check.py` in CI's order. It covers every crate except the desktop app.
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
- **Done:** the split-check script (`tools/modgraph/split_check.py`, the portable form of the scratchpad scripts, with the skip set in its docstring) and the `unreachable_pub` counter (`tools/modgraph/unreachable_pub.py`). Both were committed in `c1e0253`.
- **Cycle gate:** the union of (a) the compiler graph on Linux (`pg-dev`) and on Windows (the host, `just modgraph`), with only the `uses` edges, grouped by module (items map to their enclosing module; `(root)` is the crate root), and (b) the text scan, which covers the cxx-qt bridge modules. A group in either list fails the check unless it is on the allowlist.
- **Allowlist:** `tools/modgraph-allow.txt`, read by `tools/modgraph/allowlist.py`. Each line names `source` (scan, linux or windows), the crate and its members. `module_graph.py` and `compiler_graph.py` both fail on a group that is not listed for their source, and on a listed entry that matches no group. Groups H and I exist on one target each, so a fix on one target must not leave a stale entry for the other. M-1, M-3, M-4, M-5, M-6, M-19 and M-20 remove entries; M-16 empties the file. Both checks were run at `7ba93e9`: the text scan and the compiler graph pass on Windows, and the compiler graph passes on Linux.
- **Perf check: runs in the container.** WSL is retired for this repository (`62b4540`). `just perf` runs `tools/perf_check.py` in `pg-dev` through `tools/linux.py perf`, with the callgrind tools in the image's tools stage, and it gates against `docs/perf/baseline.json`. On this image the gate reports `list.all_taken` at -12.6% against the baseline; the other ten benches are within 0.4%. Explained in "Validation 2": the C library's time-zone work no longer runs on that path; the baseline is re-measured there.
- **Private imports resolve to their source** (added at M-20): a path that reaches a name its module binds with a private `use` (not `pub use`, which is the module's API) is an edge to the module the name comes from, as the compiler resolves it. A cxx-qt bridge must name its backing struct as `super::T` (M-20), so the parent binds it with `use state::XRust;`, and the bridge's edge goes to `state`, not back to the parent.
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
- **Gates:** `just build`, `just lint`, `just lint-qml` and `just test` pass. `just shots` against the reference recorded at `c1e0253` (`tools/shots/reference/c1e025319d`): 21 of 21 scenarios same, on two consecutive full runs.
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
- **Gates:** `just lint` and `just test` pass. The Linux container's `cargo fmt --check`, `clippy -D warnings`, `cargo doc` and `cargo test` all pass as root, but the permission-based tests skip as root and `docker/check.sh` fails on any skip. The local `pg-dev` image has no `pg` user (it predates the Dockerfile's `useradd` and `USER pg`), so the Linux pass has to be rerun after the image is rebuilt from `docker/Dockerfile`.

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

- **Result (committed as `158659b`; validated on Windows in "Validation" and on Linux in "Validation 2"):** `App` is the composition root: it opens the services in dependency order and owns them, with no delegates. The services are `Notes` (`notes.rs`), `Catalogs` (`catalogs.rs`: the keeper and the pool), `PrefStore` (`prefs.rs`), `Images`, `Reading` (the reader, plus `details` and `entry`), `Views`, `Edit`, and `LibraryService` (`library.rs`, whose `OnceLock` holds the started `Library`). `Makers` (`makers.rs`) is private. Each holds `Arc` handles to the services below it. Two deviations from the table: `Views` names its methods `all`, `save` and `delete`, so that `app.views()` does not stutter; and `LibraryService` holds only the catalogs and the thumbnail cache, because the job uses neither images nor preferences. `Note` moved to `notes.rs`, and `path_of` to `catalogs.rs`. `yandex`'s `start_folder` and `save` take a `&PrefStore`. The desktop's call sites changed in the same commit (`app::get().library().start(h)`, `app::get().prefs().get()`, `app::get().catalogs().file_path(id)`). The build and `just shots` passed in both validations, and the perf gate in "Validation 2" shows no effect.

### M-4b: one place that opens a catalog folder

The command line does not use `pg-app`, and should not. `run_command` opens one mutable `Catalog`. `scan`, `thumbs` and `write` each open their own `ThumbCache` with the command's cap and a printer thread for its events; `write` leaks its cache so events print to the end. `App` keeps a connection pool, a keeper, a leaked cache, a reader and preferences. Bending both to one shape is worse. What the two share is how a catalog folder is opened.

- **Add to `pg-library`:** `open_catalog(dir)` (`catalog.sqlite` in the folder, `versions()`, `PathRules::NATIVE`) and `open_thumbs(dir, catalog_id, cap, events)`. `versions()` already lives there, and the data folders are `pg_library::layout::Layout` (which replaced `default_catalog_dir()`).
- **Callers:** `App::open` and the four `cli` sites (`Catalog::prepare` in `run_command`; `ThumbCache::open` in the scan, thumbs and write commands) call them. `cli` keeps its control flow and does not depend on `pg-app`.
- **README.** `crates/README.md` calls `app` "the UI-free application layer the front ends share", which is false for the command line. Change it to: "the application layer the window uses; the command line drives the passes directly through `pg-library`".

Gate: Linux checks, Windows `just test`.

- **Result (committed as `9348daa`; validated on Windows in "Validation" and on Linux in "Validation 2"):** `pg_library` has `catalog_path`, `open_catalog` and `open_thumbs`. `App::open`, `Config::catalog_path` and the command line's four production sites call them. `cli` keeps its `Catalog` import for its helpers, and it still does not depend on `pg-app`. Its tests keep their direct calls. The README now describes `app` as the layer the window uses.

### M-5: group C

`filters::count` (the saturating `i64` to `i32`) has two callers: `facet_choices::rows`, and `filters`'s `apply_facets`. Move `count` into `facet_choices`, which owns the `Counts` type it serves. Then `facet_choices` no longer depends on `filters`. `filters` imports `count` from `facet_choices`, which is the edge it already has (`Counts`, `set_counts`). In `rows`, the closure's binding `count` is renamed `shown`, so it does not shadow the function.

- **Result (committed in the M-5 commit):** the text scan's group C is gone. Its two bridge pairs remain, as separate J entries in `tools/modgraph-allow.txt`: `facet_choices, facet_choices::qobject` and `filters, filters::qobject`.

Gate: desktop gates.

### M-6: group B, the deadline timer

Remove both globals, not one:

- `deadline.cpp` holds a process-wide static `QTimer` whose `timeout` runs a context-free lambda that calls `deadline_fired()`.
- `deadline.rs` holds a thread-local `SHELL: RefCell<Option<CxxQtThread<Shell>>>`, which is the only reason `deadline` imports `crate::shell::qobject::Shell`.

**Design:** `deadline_arm` takes the shell's `QObject*`. The timer is parented to it, and its `timeout` queues a call to the shell's `timeout` invokable (`QMetaObject::invokeMethod`, queued connection). Then `deadline_fired`, the thread-local and the `Shell` import all go.

**Fallback:** a boxed closure only if cxx-qt cannot hand C++ the object pointer. If that happens, record why in this section.

- **Result (committed as `39996b0`; validated on Windows in "Validation" and on Linux in "Validation 2"):** the hand-off works through cxx-qt 0.10's `Upcast` (`shell.upcast_pin()`, as `lib.rs` already does for the QML engine). The bridge takes the base as `type QObject = cxx_qt::QObject` and passes `*mut QObject` to C++, so the fallback is not used. The timer is a child of the shell, made on the first arm, and its timeout queues the shell's `timeout`, which is now a `#[qinvokable]`. Removed: the static `QTimer` and the `deadline_fired` callback in `deadline.cpp`, the thread-local `SHELL` and `remember` in `deadline.rs`, and the `Shell` import there. The text scan's group B is gone: `controller` and `deadline` leave the cycle, and what remains is the J pairs `folders` and `shell`, listed separately.
- **Verified** by the Windows build: cxx-qt 0.10 generates `Upcast<QObject>` for the shell, and the `QObject` alias compiles in the bridge. The pointer is made with `core::ptr::from_mut`, since the workspace denies `as` conversions.

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

- **Result (the split committed as `31e6b1f`; the rest in the validation commit):** `lib.rs` holds only the module declarations and the public re-exports; `types.rs`, `lifecycle.rs`, `writers.rs` and `core.rs` hold the layers above. The split left three things, finished at validation:
  - The tests stayed in `lib.rs` (about 1,700 lines), with 18 test-only imports so that `use super::*` still reached the moved items, and one test called the now-private `AnalysisWriter::sync_file_rating`. Each test now sits in the module it tests: 5 in `core.rs` (roots, cancelling, the work list, marking), 31 in `lifecycle.rs` (opening, preparing, layouts, rules, migration, the taken zone, the import version) and 13 in `writers.rs` (the scan, analysis, borders, taken-time and rating-sync writers). The helpers that more than one module's tests use are in `test_support.rs` (`#[cfg(test)]`). The test-only imports are gone, and `sync_file_rating` stays private.
  - The root's crate-private re-exports (`DELETE_FILE_RATING`, `SET_FILE_RATING`, `folders_under`, `rules_name`, `skipped_folders`, `under_params`, and the `fixture`-gated `set_borders_version` and `set_import_version`) are gone: their users import from `types` and `lifecycle` (ground rule 2).
  - `pair`'s `impl Catalog`, group E's last edge (the compiler graph shows it as `core <-> pair`), is a free function, `pg_catalog::pair::after_scan(&mut Catalog, …)`, in the now-public `pair` module, as `writes` and `browse` are. `Coverage` and `Pairing` are reached as `pg_catalog::pair::…`; `pg-scan` is the one caller.

  Group E is gone from the text scan and from the Windows compiler graph.

## Layering

### M-17: edges that point the wrong way

- **`catalog` → `metadata`.** The only production use is `writes.rs`, which calls `can_hold_rating(format: Format) -> bool` (`metadata/src/lib.rs`). That function is one line, `!matches!(format, Format::Bmp)`, and depends only on `pg-model`'s `Format`. Move it to `pg-model`, next to `Format` (`model/src/format.rs`). `writeback` callers change too. Then remove `pg-metadata` from `catalog/Cargo.toml` entirely. `schema.rs` mentions `pg_metadata::rotation_method` in an SQL comment; the comment names that function's real home after the move.
- **`catalog` → `analysis`.** Optional, behind `fixture`. The fixture takes the two version constants from its callers (`fixture.rs`, `fixture/raw.rs`). Catalog's own tests already take `analysis` as a **dev-dependency** (no cycle: `analysis` does not depend on `catalog`). So the optional normal entry goes: remove the optional `pg-analysis` entry. The `fixture` feature stays for its private connection, and it no longer needs `analysis`.

- **Result (committed as `48dc8dc`; validated on Windows in "Validation" and on Linux in "Validation 2"):** `can_hold_rating` is in `pg-model` (`model/src/format.rs`, next to `Format`), and `catalog`'s `writes.rs` and `writeback` call it there. `catalog` no longer depends on `pg-metadata` or on `pg-analysis` (its `fixture` feature is `[]`; `pg-analysis` stays a dev-dependency). `fixture::versions()` is gone: the callers name `pg_analysis::analyze::IMPORT_VERSION` and `pg_analysis::borders::BORDERS_VERSION` themselves (five test call sites, in `catalog`, `thumbcache` and `thumbs`; the last two gain `pg-analysis` as a dev-dependency). The SQL comment names `pg_edit::rotate`, which decides which formats turn in the file: `pg_metadata::rotation_method` does not exist.
- **Validation:** the M-17 commit did not update `Cargo.lock` for its dependency changes (`pg-catalog` loses `pg-metadata`; `pg-thumbcache` and `pg-thumbs` gain `pg-analysis`), so a `--locked` build failed. The lock file is updated in the validation commit.

Gate: Linux checks, Windows `just test`.

### M-18: public surface, checked by the compiler

Turn on rustc's `unreachable_pub` lint for the workspace (`[workspace.lints.rust] unreachable_pub = "warn"` in the root `Cargo.toml`; every crate already opts in). Measured scope (see Investigation results):

- Generated code (`pg-video`'s bindgen output): exclude it at its include site, not by editing the output.
- cxx-qt bridge declarations in `pg-desktop`: these need a bridge-level allow, or a check that the macro compiles with them narrowed. Verify on Windows, since the desktop builds only there.
- Ordinary items: about 40 in `pg-desktop`, 6 in `pg-cli`, 2 in `pg-app`, 1 in `pg-edit`. Narrow each to `pub(crate)` unless another crate uses it. Items used only through a `pub use` that is itself unused are deleted.

Gate: Linux checks, Windows `just build` and `just test`.

- **Result, part 1 (committed as `e1477ee`; validated in "Validation 2"):** the root `Cargo.toml` has `[workspace.lints.rust] unreachable_pub = "warn"`, which `just lint`'s `-D warnings` makes an error. `pg-video`'s bindgen output is exempt at its include site: `unreachable_pub` joins the `#[expect]` on `mod generated` in `ffi.rs`, and the output is not edited. No item was narrowed in this commit. The list is the compiler's, and the counts above predate M-10 to M-15, which moved items and made new ones (most of the new ones `pub(super)` or `pub(crate)` already). Two questions about the cxx-qt bridges are open, and only the Windows build answers them: which generated items carry the bridge's spans (rustc does not report this lint in code spanned to an external macro), and whether a backing `XRust` narrowed to `pub(crate)` passes `private_interfaces` when the bridge's items are effectively crate-visible. M-20 found that the struct must stay `pub`, but for an alias-path reason. Part 2 is the narrowing itself, done in the validation run from the lint's list. It keeps one allow per bridge only where the macro's output forces `pub`.
- **Result, part 2 (the second validation commit):** the narrowing, from the compiler's list.
  - Ordinary items, each narrowed to `pub(crate)` (or `pub(super)` inside an inline module), with their fields where the item was a struct. In `pg-edit`: `rewrite::gif::NothingLeft` and `rewrite::jpeg::Imperfect`. In `pg-app`: `Makers`, and `prefs`'s path serializer and deserializer. In `pg-cli`: `ReportStats`, `report`, `serve` and `import_review` in `borders.rs`, and `DEFAULT_GROUP_DISTANCE` and `report` in `dups.rs`. In `pg-platform`: `sqlite`'s `alias_under`. In `pg-desktop`, about 45 of them (`app::open` and `get`, `log::note`, `command_line::value`, `picker::take`, `trace::install`, ...).
  - The cxx-qt bridges. cxx-qt accepts no attribute on a bridge module but `doc`, outer or inner (`error[cxxqt]: Unsupported attribute!`). An allow on the enclosing module would cover all of it, and lint levels pass to child modules, so an allow on `mod gallery;` would also cover `gallery/model.rs`, `items.rs` and the rest. So each bridge is now a module of its own, `src/<x>_bridge.rs` (cxx-qt-build takes bridge files from `src/` only, M-20). It holds the declarations and the private `use crate::<x>::state::XRust;` that the bridge's `super::XRust` resolves to. Its declaration in `lib.rs` carries the one `#[expect(unreachable_pub, reason = ...)]`. `<x>.rs` keeps the hand-written code and reaches the bridge through `use crate::<x>_bridge::qobject;`, under the lint like every other module. The bridges' shims now live outside `<x>`, so the fields and methods they reach are `pub(crate)` (they were `pub(super)` or private). `providers` and `screenshot` declare free `extern "Rust"` functions, which cxx resolves in the bridge's parent, so their bridge files import `load_image` and `screenshot_taken`. `cpp/providers.cpp` and `cpp/screenshot.cpp` include the renamed generated headers. `app_icon` and `live_reload` were bridges alone, so each is one module with the allow on it. The `settings` and `trace` bridges generate nothing the lint reports, so their declarations have no allow (an `#[expect]` there was unfulfilled). The platform exception now names both halves of the player glue: `video_player.rs` (`play_path`) and `video_player_bridge.rs` (the declarations).
  - The backing structs stay `pub`. Narrowed to `pub(crate)`, each fails to build: `error[E0446]: crate-private type ... in public interface` (the bridge's generated code names it in a public signature). Each `XRust` in `<x>/state.rs` carries `#[expect(unreachable_pub, reason = ...)]` saying so. That settles part 1's second question. The first is settled too: the lint reports the bridges' generated items (the `type QAbstractListModel;` lines, the C++ functions, the shared enums, the `#[inherit]` signals).
  - With it all, `cargo clippy --workspace --all-targets -- -D warnings` passes on Windows.

### M-20: the cxx-qt backing types (group J, desktop)

Each cxx-qt bridge is a `pub mod qobject` inside a desktop module X, and its first line is `type X = super::XRust;`: the Rust struct that backs the QObject lives in X, and X implements the bridge type (`impl qobject::X`). Each pair is a cycle. The scan finds 16 modules (the group J list; `viewport` included).

The `super::` paths in each bridge are that one line. The other `super::` hits in these modules (`command_line`, `keymap`, `picker`, `video_source`, `shell`, `yandex`) are `use super::*;` in `#[cfg(test)] mod tests`, which the scan excludes. Nothing more to check per module than whether `XRust` uses an item still in X.

**Fix, per module X:**

- Move `XRust` (its fields and the methods the bridge calls) into a leaf module, `X/state.rs`. The bridge still names it `super::XRust`: cxx-qt accepts no other alias path (`cxx-qt-gen/src/syntax/foreignmod.rs`: "Type alias path must have at exactly two segments, super::T", because cxx takes an `extern "Rust"` type only by name), so X binds it with a private `use state::XRust;`. The struct is `pub` (the bridge's generated `pub` items name it; the private `state` module keeps it in the crate), and its fields are `pub(super)`.
- X keeps `impl qobject::X` and the public functions. That gives X → qobject → leaf, with no edge back.
- If `XRust` uses an item still in X, move that item to the leaf too. Check each one. Move the item and change its callers.

Gate: the desktop builds only on Windows. Windows `just build` and `just test`, and `just shots` with no pixel change against `tools/shots/reference`. Run `cargo-modules` (M-0) on the Windows target afterwards; the scan must show no J group.

- **Result, part 1 (committed as `bf2ad19`; corrected in the validation commit):** thirteen modules: `trace`, `command_line`, `keymap`, `settings`, `words`, `filters`, `folders`, `library`, `viewport`, `yandex`, `picker`, `shell` and `facet_choices`. Each `XRust` (and, in `facet_choices`, `Row`) is in `X/state.rs`. As committed, the bridge aliases named `crate::X::state::XRust` and the structs were `pub(super)`; neither builds (the fix rule above says why). Each alias is now `super::XRust`, with `use state::XRust;` in X, and each struct is `pub`.
- **Result, part 2 (the validation commit):** the three modules whose backing struct held a type declared in its own bridge.
  - `animation`: the frame sink's C++ declarations (`QVideoSink`, `SinkRef`, `FrameSink` and their functions) are a bridge with no QObject, `src/frame_sink.rs`, with `Frames` (its `Send`, `Sync` and `pg_render` `Sink`) beside them. `animation/state.rs` holds `AnimationPlayerRust`; the `AnimationPlayer` bridge aliases `type QVideoSink = crate::frame_sink::ffi::QVideoSink;` for `setSink` (the precedent is `live_reload`'s `GpuDevice`).
  - `video_source`: the player's declarations (`QMediaPlayer`, `VideoPlayer`, `new_video_player`, `play_units` on Windows, `play_fd` on Unix, `clear`) are a bridge with no QObject, `src/video_player.rs`, with `play_path`, the one place the per-platform call is chosen (FFI glue, ground rule 6). The conversion is `pg_platform`'s: `file::play_target` (origin's `dbf2b86`: the name's UTF-16 units on Windows, an open descriptor the player takes over on Unix), which the rebase kept over this step's `names::units_of_path` and `file::open_descriptor`. `video_source.rs` has no `cfg`; its bridge aliases `QMediaPlayer` from the leaf.
  - `gallery`: `gallery/state.rs` holds `GalleryRust`, whose `viewed_kind` is a `pg_model::query::MediaType`. The property is `READ = read_viewed_kind, NOTIFY` with no `WRITE` (QML only reads it); the getter maps `MediaType` to the bridge's `MediaKind`, whose QML names (`GalleryModel.Video`) are unchanged. `refresh_viewed` compares, stores and signals, as the generated setter did, so the viewer still hears of a change only when there is one.
  - The bridge files must all be in `src/` (cxx-qt-build: one directory per QML module, QTBUG-93443), so the two leaves are top-level modules, as `gpu_device` is.

  The text scan shows no J group (with private imports resolved, M-0), and the allowlist's 16 J entries are gone.

### M-19: the parent-child cycles (groups F, H, I)

Three groups have the same shape: a parent module defines a trait or types, its child implements them, and the parent re-exports the child. Each is cut the same way: move the shared items into a new leaf module that neither side depends on upward.

- **F (app, `yandex`).** Move `pub trait Source` (`app/src/yandex.rs`, with its three methods; the old plan said four) into `crates/app/src/yandex/source.rs`. Its signatures use only `pg_yandex` types, so the leaf has no upward dependency. `yandex.rs` keeps `pub use source::Source;` and `impl Source for Client`; `yandex/replay.rs` changes its import to `use super::source::Source;`. The re-export `pub use replay::{Recording, Replay}` stays, because the parent no longer depends on the child through the trait.
- **H (platform, `dir`).** Move `Entry` and `Lister` out of `dir.rs` into `crates/platform/src/dir/listing.rs`. `dir.rs` re-exports them, and both inline `unix` and `windows` modules import from `super::listing`. First read what `Entry` and `Lister` use from `dir.rs`; any item they use moves too.
- **I (platform, `picker`, Windows).** The Windows `native` module imports `FolderDialog`, `ShellItem` and `shell_path` from `picker`. Moving only the two traits would leave `picker` ↔ `native` as a cycle. Instead, the leaf `crates/platform/src/picker/shell.rs` holds `FolderDialog`, `ShellItem`, `shell_path` and `path_of_units` (pure UTF-16 functions, whose tests move with them). `native` imports only from the leaf. `picker` → `native` stays legal: `failed_call` downcasts `native::CallError`.

Gate: Linux checks; Windows `just build` and `just test`; both `cargo-modules` runs (M-0), which must show no group F, H or I on either target.

- **Result (committed as `ba5a4d3`; validated on Windows in "Validation" and on Linux in "Validation 2"):** F: `Source` is in `app/src/yandex/source.rs` (a leaf); `yandex.rs` re-exports it and implements it for `Client`; `replay.rs` imports `Fetch` from `images` and `Source` from `source`, so it no longer reaches its parent. H: `Entry` is in `platform/src/dir/listing.rs`, which `dir.rs` re-exports. As committed, `Lister` was there too, with its methods (`open`, `each`) left in the backends, which the compiler graph showed as `dir::listing <-> dir::windows`. At validation each backend (`dir::windows`, `dir::unix`) got its own `Lister`, with its fields, `new`, `open` and `each`, and `dir.rs` re-exports it with the backend's `Dir`; `listing.rs` has no `cfg` left. I: `FolderDialog`, `ShellItem`, `shell_path` and `path_of_units` are in `platform/src/picker/shell.rs` with their pure tests; `native` imports from it. The text scan shows none of the three groups. The allowlist's F, H and I entries (scan, Linux and Windows) are removed; the compiler side is checked at the end.

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

- **Result (committed as `a784789`):** the layers above, with `lib.rs` holding `ThumbCache` and its tests. Validation found three doc links the move broke, fixed in the validation commit (see "Validation").

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

- **Result (committed as `cade968`; validated in "Validation 2"):** `tiff.rs` (`Tiff`, `Entry`, the TIFF types, and `EXIF_TAG_ORIENTATION`, which the reader and the patcher both look for in IFD0, so `tiff` stays a leaf), `exif.rs`, `xmp.rs` (with `MetaScratch` and `Packet`), `container.rs` (with `JPEG_XMP`), `orientation.rs`, and `lib.rs` with `Meta` and its read. The edges are `lib` → `{container, exif, xmp, orientation}`, `container` → `xmp`, `orientation` → `{exif, tiff}`, `exif` → `tiff`: `container` uses nothing of `exif` (its EXIF block comes from `pg_container`). The public paths stay (`pub use orientation::{orientation, patch_orientation}` and `xmp::MetaScratch`). A test moves into a module only when it needs that module's private items: `extended_packets`, `inflate_zlib` and `MAX_INFLATED`'s four tests, with their random-data helpers, are in `container.rs`, and `percent_scale` is in `xmp.rs`. The end-to-end tests through `Meta::read` stay in `lib.rs`. `png_chunk`, which both use, is in `test_support.rs`.

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

- **Result (committed as `ae87154`; validated in "Validation 2"):** the layers changed from the list above, for the reason M-7 and M-19 found: the compiler graph puts an inherent method in its type's module, so an `impl Codecs` block in `png.rs` that calls `convert` would make the struct's module depend on `convert` and the format modules, and the format modules depend back on it. No format module has an `impl Codecs`. Each is a set of free functions over only the buffers it uses (`png::decode(&mut ::png::Scratch, &mut raw, bytes, out)`, and so on), and `codecs.rs` keeps `Codecs`, `decode` (which lends each format its fields) and `trim`, so `scratch.rs` is not needed. There is no `errors.rs` either: each error mapper and colour check lives with its format (`png_error` and `png_color` in `png.rs`, `tiff_error`, `check_sample_format` and `tiff_unknown_color` in `tiff.rs`), because no format uses another's. `limits.rs` holds `max_decode_bytes` and `reserve` (every format reserves, so it is a leaf below them); `lib.rs` re-exports `codecs::limits::max_decode_bytes`. `convert.rs` holds the conversions, `target`, `zeroed`, `finish`, `check_len`, `over_white` and `read_with` (BMP's decode). `tiff.rs` is `pub(crate)` with `Stored`, `TiffPlan`, `TiffDecoder`, `decoded_color` and `tiff_limits`, which `encode.rs` now imports from `crate::codecs::tiff`. Edges: `codecs` → each format and `convert`; each format → `convert`, `limits`; `convert` and `limits` → nothing. Inside `codecs.rs` the format crates are written `::png`, `::gif` and `::tiff`, because the child modules have the same names. Tests: `conversions_are_images` and `planes_convert_as_interleaved` are in `convert.rs`; the decode tests through `Codecs::decode` stay in `codecs.rs`; `flattened`, `ne_bytes` and `noise`, which both use, are in `codecs/test_support.rs`.

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

- **Result (committed as `f4ba3c1`; validated in "Validation 2"):** two changes from the list above. First, `Detector` and `Detector::detect` stay in one module, because the compiler graph puts an inherent method in its type's module (the M-7 and M-10 finding): `Detector` in `scratch.rs` with its `detect` in `detector.rs` would be `scratch` ↔ `detector`. So there is no `scratch.rs` or `detector.rs`. The top layer (`detect`, `Detector` with `detect`, `capacity_bytes` and `trim`, `PASSES`, `detect_with`, `detection`, `blur_threshold`) stays in `borders.rs`, which needs no re-export of its own items. It re-exports only `params::{BORDERS_VERSION, Params}`, the public API. Second, the side detection no longer takes the whole `Detector`: `side.rs` has a `Scratch` (`SolidLines`, `DetailSums`, `act_cum`) that `Detector` holds, and each buffer type reports its own `capacity_bytes`, so no module reads another's fields to sum them. `SolidLines` is in `lines.rs`. `to_count` is in `side.rs`, its one user. The modules are `params`, `view`, `stats`, `lines`, `seam` and `side`. Edges: `stats` → `view`; `lines` → `params`, `stats`, `view`; `seam` → `params`, `view`; `side` → `lines`, `params`, `seam`, `stats`, `view`; `borders` → `params`, `side`, `stats`, `view`. `median_is_the_sorted_one`, `run_length_bridges_short_gaps` and `medians_match_numpy` are in `stats.rs`; the detector tests stay in `borders.rs`.

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

- **Result (committed as `4856061`; validated in "Validation 2"):** the split is `gallery/{model,items,edits,nav,roles}.rs` as listed. Per the user's direction (validation at the end of the plan), the one-method build came after the whole split, not before it. Every method moved into a child is `pub(super)` (or keeps its `pub(crate)`): the bridge's shims in `gallery::qobject` and the sibling modules call them. The desktop is invisible to the compiler graph (cxx-qt), so the text scan is the check, and a child that names an item of `gallery.rs` would close a cycle with its parent. So nothing a child uses stays in the root: `qt_int` and `cell_roles` are in `roles.rs` with the constants, and `fresh_entry` moved to `state.rs`. `impl GalleryRust` (`with_details`, `forget_details`) moved to `state.rs` too, so the type and its inherent impl are in one module. `gallery.rs` keeps the bridge, `MODEL`, `post` (`shell.rs`'s callers are unchanged) and `register_model`, which uses `MODEL`. Edges: `gallery` → its children; `model` → `qobject`, `roles`, `state`; `items` and `nav` → `qobject`; `edits` → `qobject`, `roles`, `state`; `qobject` → `state` (the alias). `model` is 312 lines, `items` 192, `edits` 124, `roles` 49, `nav` 28; `gallery.rs` is 230, most of it the bridge.

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

- **Result (committed as `f6992d4`; validated in "Validation 2"):** measured again after M-12. `platform/src/dir.rs` fell under 500 (its `Lister` moved to the backends in M-19); `catalog/src/core.rs` (556, from M-7) and `library/src/runner.rs` (514) joined the list. `catalog/src/browse/read/tests.rs` (949) is a test file. Each file over 500 production lines was read; split or left:

  | File | Lines | Decision |
  |---|---|---|
  | `client/src/text.rs` | 1,473 | Split. Three layers: `text/words.rs` (the Fluent files, the per-thread bundle, the pseudo-locale, `ui`, `shown`), `text/message.rs` (`Message`, `CacheStartOver`, their ids and values, `render`), and `text.rs` (the line builders). The public paths stay (`pub use message::{CacheStartOver, Message, render}`, `pub use words::{set_pseudo, ui}`). The test that the entries and the variants match is in `message.rs`, the file tests in `words.rs`, and `entries_in_file`, which both use, in `text/test_support.rs`. |
  | `video/src/ffi.rs` | 812 | Leave: the FFmpeg declarations, one layer. |
  | `writeback/src/lib.rs` | 794 | Split. The file operations (`stat`, `stat_of`, `unchanged`, `temp_of`, `sync`, `write_into`) are a leaf below the writer: `file.rs`, with their two tests; `lib.rs` re-exports `stat` and `temp_of`, the crate's API. The inline `pub mod fault` is `fault.rs`. The writer (`Writer`, `Inner`) stays in `lib.rs`: one type pair and its loop. |
  | `desktop/src/shell.rs` | 667 | Leave: the bridge (about 250 lines) and one QObject's inherent methods, which hand QML's calls to the controller (`dispatch`) and carry out its commands (`execute`). One layer. |
  | `catalog/src/schema.rs` | 630 | Leave: 330 lines are the baseline SQL; the rest is the version and migration path over it, one layer. |
  | `desktop/src/yandex.rs` | 602 | Leave: the bridge, one model's methods, and the panel's words for the search's states, which only this model uses. |
  | `app/src/yandex.rs` | 599 | Split what is a leaf: the saved file's naming and writing (`saved_stem`, `numbered`, `claim`, `temp_path`, `write_new`, `write_new_with`, `stem_of`, `MAX_CLAIMS`, with their five tests and `odd_stem`) is `yandex/save.rs`. The photo's reading stays: it returns the root's `SearchError`, so a child holding it would depend on its parent. |
  | `edit/src/rewrite.rs` | 578 | Split by format, as M-10 split the codec: `rewrite/op.rs` (`Op`), `raster.rs` (`decoded_size`, `zeroed`, `unpack`, `pack`, `apply_rows`), `chunk.rs` (`png_chunk`, `chunk_bytes`), and `jpeg.rs`, `png.rs`, `webp.rs`, `gif.rs`, `bmp.rs`, each with a `rewrite` (and `jpeg::mcu`). `Imperfect` is in `jpeg.rs`, `NothingLeft` in `gif.rs`. `crop.rs` and `rotate.rs` call `rewrite::png::rewrite` and so on; no re-export. The GIF tests are in `gif.rs`; the test across formats (a huge declared size) stays in `rewrite.rs`. |
  | `cli/src/lib.rs` | 566 | Split. `args.rs` (`Cli`, `PipelineArgs`, `Command`), `report.rs` (the progress lines, the thumbnail cache's events, a rebuilt catalog, a pass's failures), and `lib.rs` (`wants_cli`, `main`, `run_command` and its helpers). |
  | `catalog/src/core.rs` | 556 | Leave: `Catalog` and its inherent impl. Its methods cannot move to other modules without the cycle M-7 found. |
  | `yandex/src/lib.rs` | 520 | Split. `types.rs` (the results and `YandexError`), `parse.rs` (the answers read: pure functions of the bodies), `http.rs` (the agent, a transfer's bounds, the User-Agent, `read_response`), `client.rs` (the paced search), `fetch.rs` (the result files). `lib.rs` re-exports the public items, so `pg_yandex::Client` and the rest keep their paths. |
  | `library/src/runner.rs` | 514 | Leave: M-15 found it coherent (the runner, its passes and its messages). |

## Platform boundary

### M-13: platform calls only in `pg-platform`

Production platform code outside `pg-platform`:

- `crates/video/src/ffi.rs` (line 192): the Windows branch of `Lib::open` loads the DLL with `libloading::os::windows`. This moves to `pg_platform::dll::load_with_search(path)`. `pg-video` keeps the FFmpeg symbol lookup. `pg-platform` already depends on `windows-sys`.
- `crates/gallery/src/main.rs`: `attach_console` calls `windows_sys` directly. This moves to `pg_platform::console::attach_to_parent()`.
- `crates/desktop/src/command_line.rs`: `value_exact`, an `OsString` split by platform. Moves to `pg_platform::names` with the path-to-native-units conversion.
- `crates/desktop/src/gui_app.rs`: `arg_bytes` moves to `pg_platform`.
- `crates/desktop/src/picker.rs`: `has_drive_or_share`, `native`, `run_dialog`, the UTF-16 path conversion, and the `USERPROFILE`/`HOME` lookups. The start-folder logic moves to `pg_platform::picker`.
- `crates/desktop/src/video_source.rs`: done at M-20. The per-platform call is in `src/video_player.rs` (a stated exception: its bridge declares `play_units` on Windows and `play_fd` on Unix), and the file's form is `pg_platform::file::play_target` (origin's `dbf2b86`, which the rebase kept over this step's `units_of_path` and `open_descriptor`).

After the moves, the README's "the one exception" (`crates/README.md`) is removed. Any remaining exception is a cxx-qt or FFI declaration that needs per-platform signatures, and it is stated where it is. M-16 enforces the rule by grep.

Gate: Linux checks (the platform crate's Linux code must still build), Windows `just test`, and the FFmpeg video tests on Windows.

- **Result (`a68b87d`, after the rebase onto `origin/main`):** the moves are origin's, made in parallel with this plan: `397bd85` (`pg_platform::console::attach_to_parent`; `pg_platform::dll`, the library's file names, the Windows folder search and the loader with the restricted search; `pg_platform::names::arg_bytes` and `after_first_equals`) and `dbf2b86` (`pg_platform::picker`'s start folder, drive-or-share test and native-dialog flag; `pg_platform::file::play_target`, the video player's file target). This step's own implementation of the same moves (`pg_platform::library::Libraries`, `names::units_of_os`, `argv_bytes`, `units_of_path`, `file::open_descriptor`, and `picker`'s `HAS_DIALOG`, `system_dialog`, `pick_in` and `start_folder`) was validated in "Validation 2" and then dropped on the rebase in favour of origin's, so the tree has one of each. What the step keeps: the README's platform entry states the rule and its two exceptions, the video player's declarations and the folder dialog's glue (`pg_platform::picker::Dialog` exists on Windows only). The test-only `cfg`s that are left are tests of one platform's behaviour.

## Dev-dependency cycles

### M-14: `testkit`, with no internal dependencies

The dev-dependency component of eight crates is connected through `testkit`. Both edges that tie `testkit` to `catalog` and `jobs` go:

- `pub use pg_catalog::fixture;` (`testkit/src/lib.rs`) has no users. Delete it. This removes the `catalog` edge.
- `events.rs` (the `Quiet` bus helper, which uses `pg_jobs::events`) moves to a new dev-only crate, `pg-jobs-testkit`, which depends on `pg-jobs` and `pg-testkit`. Its users are `cli`, `library`, `thumbcache` and `thumbs`, all as dev-dependencies. `pg-jobs` does not use `Quiet`, so the old plan's `testing` feature in `pg-jobs` is not needed.
- One `odd_name` helper in `pg-testkit` (standard library only, so `testkit` stays a leaf). It replaces the six copies written separately, each under its own `cfg`, in `app/src/yandex.rs`, `app/src/prefs.rs`, `desktop/src/command_line.rs`, `desktop/src/screenshot.rs`, `desktop/src/picker.rs`, and `exiftool/src/lib.rs`. This also removes the test `cfg`s that the M-13 grep would otherwise have to allow.

`testkit` keeps the scratch folders, `held.rs` and the file generators, with no internal dependencies. M-16 checks that the graph including dev edges is acyclic.

Gate: Linux checks, Windows `just test`. `cargo tree -d` must show no duplicate workspace crates in the test builds.

- **Result (committed as `de5cfb0`; validated in "Validation 2"):** `pg-testkit` depends on no crate of the workspace. Its `pub use pg_catalog::fixture` and its `pg-catalog` and `pg-jobs` dependencies are gone. One thing the plan missed: through that dependency, testkit turned on the catalog's `fixture` feature for every test build. So `cli`, `library` and `thumbs`, whose tests call `pg_catalog::fixture` without asking for it, now dev-depend on `pg-catalog` with `features = ["fixture"]` themselves (`app` and `thumbcache` already did). `Quiet` is the new crate `crates/jobs-testkit` (`pg-jobs-testkit`). It depends on `pg-jobs` alone, because `Quiet` uses nothing of `pg-testkit`. Its users (`cli`, `library`'s `video` and `writeback` tests, `thumbcache`, `thumbs`) dev-depend on it. `pg_testkit::odd_name(text)` (`text` and then a lone surrogate on Windows or a raw `0xFF` on Unix) replaces four of the six copies: `app`'s `yandex/save.rs` (`odd_stem`), and `desktop`'s `command_line.rs`, `screenshot.rs` and `picker.rs`. `gui_app.rs`'s Unix test uses it too, and the desktop gains `pg-testkit` as a dev-dependency. Two of the six were not copies of it, and their `cfg`s stay. `app/src/prefs.rs` has the native rules' absolute folders, which differ by platform (`C:\photos\…` and `/photos/…`), and the hex the catalog writes for each. `exiftool/src/lib.rs` has names only Unix can hold and a test that runs only there. Both are tests of one platform's behaviour, not a name helper. The README lists `jobs-testkit` and says that `testkit` is a leaf.

## Crate granularity

### M-15: `pg-library`'s reach

- **`dups` leaves `pg-library`.** `library/src/dups.rs` (60 lines) joins the catalog's duplicate reads with `pg_dedup::UnionFind`. Its only user is `cli/src/dups.rs`, and it has nothing to do with the job runner. Moving it into `pg-dedup` would give that pure crate a `pg-catalog` dependency. It goes to `pg-cli`'s `dups` module, next to the report that prints it. If a later multicut grows it, it gets its own crate then.
- **Dependencies removed.** The split removes four dependencies from `pg-library`: `edit`, `exiftool`, `writeback` (with `crop`, which moves to `pg-crop`), and `pg-dedup` (with `dups`).
- **New crates.** `pg-crop` (`crop.rs`, 323 lines) and `pg-borders` (`borders.rs`, 83 lines). The runner (`crop::save`, `borders::find`) depends on both. `cli` and the tests name `pg-borders` and `pg-crop` directly.
- **Tests stay in `pg-library`.** Five integration test files exercise the two passes: `tests/borders.rs`, `tests/crop.rs`, `tests/crop_roots.rs`, `tests/crop_folders.rs` and `tests/crop_tags.rs` (the last is new and untracked). All of them call `pg_library::versions()` to prepare a catalog. Moved into the new crates, they would dev-depend on `pg-library`, which depends on the new crates: the dev-dependency cycle M-14 and M-16 forbid. So they stay in `pg-library/tests` and import `pg_crop::` and `pg_borders::`. The new crates keep only unit tests that need no catalog.
- **What remains.** After the split `pg-library` is one coherent layer: `runner.rs` (519 lines) and `lib.rs`, plus `versions()`, `layout` (origin's `Layout`, which replaced `default_catalog_dir()`) and M-4b's open functions. It keeps the `pg-writeback` `fault` dev-dependency and the codec, metadata, render and video dev-dependencies for the tests that remain.
- **Stale comment.** `pg-library`'s `Cargo.toml` says the passes are re-exported there under their old names. `lib.rs` re-exports nothing, and no caller uses those paths. Fix the comment in this step.

Gate: Linux and Windows checks.

- **Result (committed as `fea1845`; validated in "Validation 2"):** `crates/crop` (`pg-crop`, the former `library/src/crop.rs`, with its three `publish` unit tests, which need only `pg-testkit`) and `crates/borders` (`pg-borders`, the former `library/src/borders.rs`). One dependency the plan missed: `pg-crop` also depends on `pg-scan`, because it compares a copy's folder with the library in real form through `pg_scan::normalize_path`. `pg-library` now depends on `pg-borders` and `pg-crop`, and has lost `pg-dedup`, `pg-edit`, `pg-exiftool` and `pg-writeback`. It keeps `pg-writeback` only as the `fault` dev-dependency. The runner names `pg_crop::` and `pg_borders::` itself, with no alias. `dups` is in `cli/src/dups.rs` with its report, and `Found`, `find`, `IDENTITIES` and `PDQ_BANDS` are private to that module (their only caller is `report`). `pg-cli` gains `pg-borders` and `pg-dedup`. The five test files stay in `pg-library/tests` and name `pg_borders::` and `pg_crop::`, which they reach as normal dependencies of the package. The manifest comment, the `lib.rs` header, the doc references in `analysis` and `cli`, and the README (two new entries, the `library` and `cli` lines, and the crop feature's module path) say where things now are.

## Closing

### M-16: enforce, at four levels

- **Module cycles:** empty `tools/modgraph-allow.txt`, so the check fails on any module cycle. Run the `modgraph` check in CI. CI cannot run it yet, so this step also: (a) installs `cargo-modules@0.28.0` in both jobs (through `taiki-e/install-action` if it carries the tool; otherwise `cargo install --locked` with a cache); (b) adds a `modgraph` step to the Linux job and to `docker/check.sh` (the Linux compiler graph plus the scan), and `just modgraph` to the Windows job (the Windows-only groups H and I show there only); (c) keeps the three pins equal (Justfile, Dockerfile, `ci.yml`). Measure the step's time first: the first container run exceeded 400 seconds, and the warm-target time was never measured.
- **Crate layers:** commit `tools/crate-layers.txt`, one line per crate listing its allowed normal internal dependencies. A draft from the current graph (30 crates, dependencies first) is in the session scratchpad as `crate-layers.draft.txt`. It is written after M-17 and includes the `app` services, `pg-crop`, `pg-borders`, `pg-jobs-testkit`, and `pg-app` → `pg-query`. CI checks the file against `cargo metadata`: a new internal edge fails until the file changes in the same commit.
- **Dev edges and re-exports:** the graph including dev edges is acyclic; no `pub use` outside the exempt list (ground rule 2); the platform grep (ground rule 6).
- **Module cycles, status:** after the validation commit the allowlist is empty, and the text scan and the Windows compiler graph pass. The Linux compiler graph passes too ("Validation 2").
- **Documents:** update `crates/README.md` (the dependency section, the platform rule, `testkit`, the new crates, the `app` services, the moved `can_hold_rating`, and the `app` description from M-4b). Add a ledger row for each M step to `docs/REFACTOR_PLAN.md`, using the same status words. Record the final check results in this file.
- **Result (committed as `188a772`; built and validated in "Validation 2"):**
  - **`tools/boundaries.py`** (standard library, `cargo metadata --no-deps`) makes four checks. (1) The crate layers: each crate's normal internal dependencies are exactly its line in `tools/crate-layers.txt`, and each dependency is on an earlier line. The file is 34 crates, bottom layer first, so the normal graph is acyclic by construction. (2) The graph of every internal edge, dev and build included, has no cycle. (3) A production `pub use` names a child module of its own module. One that names another crate is allowed only in `app/src/reexport.rs`, and one through `crate::` or `super::` is not allowed anywhere. (4) No platform `cfg` (`windows`, `unix`, `target_os`, `target_family`) and no `std::os::{windows,unix}` in production code outside `pg-platform`, except `desktop/src/video_player.rs` and its bridge file `video_player_bridge.rs` (ground rule 6; the bridge file is M-18's). Production code means `src/` and `build.rs`, with `#[cfg(test)]` and `#[cfg(all(test, …))]` items blanked and their `mod name;` files left out. The test crates (`testkit`, `jobs-testkit`, `bench`) are not checked by (3) and (4). Fed the tree as it was before this step, the script reported the three violations below. With `testkit → jobs` added back, it reports the M-14 cycle. On the tree now, all four pass.
  - **Violations the first run found, fixed in this step.** `PathRules::NATIVE` was a `cfg!(windows)` in `pg-model`. It is now `pg_platform::native::PATH_RULES`, and all of its users name that. (This step also gave `pg-bench` a `pg-platform` dependency, which "Validation 2" removed: the bench names the constant only in a comment.) Because of that move, `FolderTree` and the client's `Tree` lose `Default`, which picked the native rules: `FolderTree::empty(rules)` and `Tree::new(rules)` replace them, the desktop passes `PATH_RULES`, and the tests pass their own (Windows) rules. `pg_scan::normalize_path` stripped the verbatim prefix under `cfg!(windows)`. That code was `pg_platform::names::plain_rule_path_of` until the rebase: origin's `ec872f5` moved the same strip into `names::real_path`, so `normalize_path` is now `names::rule_path_of` of the real path. `pg_app::yandex` re-exported `pg_yandex::YandexError` for the desktop, which is `reexport.rs`'s job, so the re-export moved there.
  - **Wiring.** `tools/lint.py` runs the script, so `just lint`, both CI jobs and `docker/check.py` do. After the rebase onto origin's Python tooling it also replaces origin's `tools/check_platform.py`: that script's extra markers (`windows_sys`, `libloading::os`) and its folder-dialog exception are check 4's now, and its `PathRules::NATIVE` exception went with the constant. Both CI jobs run `just modgraph` after the lint, and `docker/check.py` runs the module-cycle check (the scan plus the Linux compiler graph) after its lint step. Both jobs install `cargo-modules@0.28.0` through `taiki-e/install-action`, which falls back to `cargo-binstall` for a tool it does not carry. The Justfile's `cargo_modules_version` is the one other pin (the container reads it from there), and `ci.yml` says so. The allowlist was already empty. The CI step's time is still unmeasured (the first CI run measures it).
  - **Documents.** `crates/README.md`: the platform rule (with `PATH_RULES` and the check), `model`'s `can_hold_rating`, the `app` services, the dependency section (the layer file and the checks), and the old-path table's `library` row. `docs/REFACTOR_PLAN.md` has a row for each M step, with what each one was validated by.

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

## Validation (2026-11-09, Windows)

The steps done so far, built and checked together on Windows for the first time. Everything the run found is fixed in one commit, each at its cause; each step above says what changed in it.

- `just lint` (fmt, clippy on the whole workspace with `-D warnings`, the wasm32 check, `cargo doc` with `-D warnings`, `cargo deny`, `cargo machete`, ruff, markdownlint, the QML lint): passes.
- `just test`: 79 suites, 719 tests pass, 2 ignored (in `pg-catalog`).
- `just qmltest`: 48 tests pass.
- `just modgraph`: no cycle group in the text scan or the Windows compiler graph; the allowlist is empty.
- `just shots` against the M-0 reference (`c1e025319d`): all 23 scenarios match. An earlier run in the same session showed `query`, `library` and `apply-view` differing in their thumbnails; the cause was not found, and the rerun matched.
- Fixed besides the steps' own results:
  - M-4: `listing_laws` called `App::listing` and `App::save_listing`, which are `app.views()`'s now.
  - M-8: three broken doc links (`judge` from the `cache_version!` docs now links `StartOverReason::OtherBuild`, which states the case; two `Event::StartedOver` links name `crate::Event`).
  - The workspace had `cargo fmt` changes left in M-4's and M-7's files.
  - The QML test stub of `KeyMap` lacked `viewerHint`, which `Hud.qml` calls (since the history squash).
- Not run: the Linux checks (`just check-linux`) and the Linux compiler graph, so the Unix code paths of M-19 (`dir::unix`'s `Lister`) and M-20 (`play_fd`, `open_descriptor`) are not yet built; and the perf gate (M-4, M-7, M-8), which also has to explain the `list.all_taken` result (M-0). All of these ran in "Validation 2".

## Validation 2 (2026-11-10, Windows and Linux)

M-9 to M-16 and M-18, built and checked together for the first time, on Windows and in `pg-dev`. Everything the run found is fixed in one commit, each at its cause. M-18's part 2 is its narrowing.

- Windows: `just lint` passes. It now runs `tools/boundaries.py` too, and all four of its checks pass. `just test`: 85 suites, 720 tests pass, 2 ignored. `just qmltest`: 48 pass. `just modgraph`: no cycle group in the text scan or the Windows compiler graph, which now sees `pg-desktop` (79 module edges) because the bridges are ordinary modules. `just shots -Base c1e025319d`: 23 scenarios. `save-crops` differed once, in 298,609 pixels of its thumbnails, each by at most 1 per channel. It matched in two reruns of that scenario alone, so it is the thumbnail flake of the first validation, and its cause is still not found. The rest match.
- Linux (`just check-linux`, `pg-dev`): every step passes. That is formatting, clippy, tests (85 suites, 692 pass, 2 ignored), docs, wasm32, the lockfiles, the supply chain, the crate boundaries, the module cycles (the text scan and the Linux compiler graph, its first run since M-0) and Python and Markdown. The Unix paths of M-19 and M-20 (`dir::unix`, `play_fd`, `open_descriptor`) are built and tested here for the first time.
- Fixed besides M-18's narrowing:
  - M-9: `metadata`'s `lib.rs` and `container.rs` lacked the imports of the constants the split moved (`exif`'s and `tiff`'s tags, `JPEG_XMP_EXTENSION`). Only the tests use them, so the imports are in the test modules.
  - M-10: `edit`'s `rewrite/op.rs` lacked `Orientation`.
  - M-13: `pg-video` names `pg_platform::library` in production code, so `pg-platform` is a normal dependency of it, not a dev one. The layer file says so.
  - M-12a: `pg-client`'s public `text` docs linked its private `words` and `message` modules.
  - M-16: `TreeBuilder` derived `Default` through the `FolderTree` that lost it; nothing used it, so the derive is gone. `pg-bench` names `PATH_RULES` only in a comment, so the `pg-platform` dependency M-16 gave it is removed. Some test modules kept `PathRules` imports that only the constant had used (`app`'s `app.rs` and `prefs.rs`, `desktop`'s `shell.rs`).
  - Linux only: `catalog`'s `lifecycle.rs` tests imported `sqlite_path` for a Windows-only test (unused on Linux, so named at the call). `platform`'s `picker.rs` docs linked `Dialog` and `pick_with`, which exist on Windows only.
  - `tools/linux.py`: the HEAD export (`check --head`, `perf`, `bench`, `fuzz`) was a `tempfile` folder. Since Python 3.12.4 that folder has an ACL that admits only the current user on Windows, so Docker Desktop refused the mount ("Access denied"). The export is now a plain `mkdir` under `target/linux-head/`, removed after the run.
- Perf (`just perf`, `pg-dev`, at `3b46058`; the gate benches the committed HEAD, so it ran after that commit). Nothing regressed. Two benches are below the baseline, and the other nine are within 0.32%:

  | Bench | Baseline | `3b46058` | Change |
  | --- | --- | --- | --- |
  | `list.all_taken` | 44,674,976 | 40,080,890 | -10.28% |
  | `pending_thumbs.at_height` | 3,767,148 | 3,412,495 | -9.41% |

  - The modularity steps are not the cause. The same benches at `14b6d0c`, the last commit before M-5 to M-20, are within 0.16% of `3b46058` on all eleven (`list.all_taken` 40,064,734, `pending_thumbs.at_height` 3,412,473). That covers the three steps with a perf gate (M-4, M-7, M-8). M-1 and M-3 are before `14b6d0c`; neither touches a benched path.
  - `pending_thumbs.at_height`: per function, against the `88e780b` profile, the instructions saved are all in SQLite's parser and allocator. That is the bundled SQLite 3.54.1 (`1da008e`); the bench's own code is unchanged since M-0.
  - `list.all_taken`: the C library's per-row time-zone work (`tzset`, `offtime`, `vfscanf`) is gone. That was already the -12.6% of M-0, on the first container run. Since M-0 it is about 2.6 points slower, which matches `14b6d0c`, whose sort key compares SQLite values instead of `Option<i64>`.
  - The baseline is re-measured at `3b46058` (`just perf --update`), with the profiles in `docs/perf/callgrind/3b46058.tar.zst`. Rerunning it reproduced every count exactly.
  - `tools/linux.py`: `perf` and `bench` declared their pass-through options as an `argparse.REMAINDER` positional, which rejects a first token that starts with `--`, so the documented `just perf --update` failed. The two commands now take what `parse_known_args` leaves, and the others still reject unknown options.

## Validation 3 (2026-11-10): the rebase onto `origin/main`

`main` was rebased onto `origin/main` (`3bd0829`), which had 45 commits made in parallel with this plan since `14b6d0c`: the Python ports of the tooling (`tools/lint.py`, `docker/check.py`, `perf_check.py`, ...), the Linux scanner work, the data folders (`Layout`), and its own M-13 moves (`397bd85`, `dbf2b86`) and platform check (`tools/check_platform.py`). Where both sides did the same thing, the tree keeps one: origin's platform moves and `play_target`, this plan's `tools/boundaries.py` (which took over `check_platform.py`'s markers and exceptions), and M-4b's open functions over `Layout`'s own `catalog.sqlite` name. M-13 and M-16 say what they kept (their results above). The merged tree had never been built; this is its first run.

- Fixed (`5decc97`): Linux only, so the Windows build did not see them. Origin's `linux_ids::volume_id` was `pub` in a private module, which M-18's `unreachable_pub` refuses. Origin's `/proc` listing test compared each entry's volume with its raw device number, though origin's `94a2530` had made the volume a stable identity; it now checks that the identities follow the devices.
- Fixed besides, while finishing origin's open list: `a1da774` (origin's case-insensitive spelling test skipped itself on every test machine, and `docker/check.py` fails on a skip; its choice is now tested over a listing), `8b8f799` (`.env` values kept their quotes when a script ran outside `just`; the callgraph scripts could not import `pgtools`, and `build.py` read the SDK's registry entry from the wrong view), `3807ae5` (the JPEG eval's Windows build named a vendored libjpeg-turbo the repository no longer has, and `jpeggpu`'s test program does not build with this MSVC), `d2cbc13` (`tools/linux.py`'s HEAD export deadlocked when the tar's end padding did not fit the pipe: `just perf` hung after the last file). `6e20cad` is origin's open A3 decision, taken: Linux refuses FAT and exFAT roots, as Windows does.
- Windows: `just lint` passes (every step, the four boundary checks included); `just test`: 85 suites, 737 pass, 3 ignored; `just qmltest`: 48 pass; `just modgraph`: no cycle group. The release build scans, imports and thumbnails a scratch library into a scratch `PG_HOME`, and starts offscreen on it with nothing written to the default folder (origin's A5).
- Linux (`just check-linux`, `pg-dev`): every step passes, with no skipped test: the lint, the module cycles (the text scan and the Linux compiler graph), and the tests (85 suites, 723 pass, 3 ignored).
- Perf (`just perf` at `d2cbc13`): all eleven benches within 0.002% of the baseline (re-measured at `3b46058`, before the rebase); ten are equal to the instruction. Origin's commits and the rebase move no bench.
- Screenshots: `c1e0253`'s reference no longer matches the fixture (origin changed it), so the base is `origin/main` itself, recorded as `tools/shots/reference/3bd0829c34`: the rebased tree against what it was rebased onto. 20 of 21 scenarios match. The 21st differs by at most 1 per channel, in thumbnails only, and is the thumbnail flake of the earlier validations, now located: two recordings of the same commit (`3bd0829`) differ the same way, by the same pixel counts, in `default`, `sort` and `save-view`, and a scenario's thumbnails come out in one of two fixed variants from run to run. The cause is not found; the exact, repeating counts point at two code paths making a thumbnail with different rounding, chosen by timing. Open.
- The synthetic library is not the cause: it is generated byte for byte the same with 4 workers or 1 (each file's seed is drawn before any is written), and both sides of a comparison photograph the same copy.

## Review 4: responsibilities

The fourth review (2026-11-10) is in [MODULARITY_REVIEW_4.md](MODULARITY_REVIEW_4.md): how every crate was checked, the steps M-21 to M-49, their order and their open decisions.

## Validation 4 (2026-11-11, Windows and Linux): review 4, steps 5 to 9

Steps 5 to 9 of [MODULARITY_REVIEW_4.md](MODULARITY_REVIEW_4.md) (`a0996ca` to `225d2fb`) were written without building, and built and checked together here for the first time. Everything the run found is fixed at its cause in `37b8e9b`; the perf gate's data is `e7576f0`.

- Compile and clippy (five rounds): imports the moves left or lacked; the typed ids of M-42 (`ScanId`, `FileHash`, `Rating`, `Turns`) in the tests; `ThumbCache::open`'s `Option<Cap>` (M-41); replay's errors as M-39's `SourceError`; `Turns::of` without an `as`; a macro used before its definition; an `#[allow]` that is an `#[expect]`; `Results`' groups without a slice that can panic.
- Tests: since M-36 the catalog and the cache open the path they are given, so the tests of a folder SQLite can't name alias it first (`pg_platform::sqlite::openable_folder`), and the cache's tests pass the cache file, not its folder. The thumbnail pass since a change number reads `files_seq` again: M-37 moved the filters into the catalog's view, after which the test catalog planned the pass by the files' `(volume, file_ref)` index for its order; the volume sort term of that query is now an expression (`+f.volume`).
- Module cycles (`just modgraph`), three, each broken by moving the item down:
  - `pg-catalog`, text scan: `write_transaction` is a leaf (`transaction.rs`), so are the current-row check (`current.rs`) and a listing's columns (`listing.rs`); the modules name `core::Catalog`, not the root's re-export; `schema.rs`'s comments no longer link `core` and `rebuild`.
  - `pg-catalog`, compiler graph: M-44 put each concern's methods in an `impl Catalog` of its module, and the compiler graph puts an inherent method in its type's module (the M-7 and M-10 finding), so a method that takes its module's own type made `core` depend on that module. The types those methods take are in leaf children, re-exported where they were: `inbox::id` (`EntryId`), `queue::job` (`Job`, `Rewrite`, `WriteEffect`), `dups::identity`, `pair::coverage`, `ledger::failure`.
  - `pg-app`: `ReadError` is in `error.rs`, beside its `From` impl, which the graph files under `Error`.
- Lint: rustdoc links (the write macros, which are defined after the module docs; `PdqIndex`, now `pub(crate)`; `judge`, a function and a module); ruff on M-48's `tools/modgraph` scripts (and `api_surface.py` now fails when `cargo-modules` does, instead of reading nothing); `deny.toml`'s `turbojpeg` wrappers (`pg-render` is no longer one, M-27); `fuzz/Cargo.lock` (`blake3`); formatting. Linux only: the `ir` bench's unused import.
- Windows: `just lint` passes (every step, the four boundary checks included); `just test`: 88 suites, 840 pass, 3 ignored; `just qmltest`: 71 pass; `just modgraph`: no cycle group in the text scan or the Windows compiler graph.
- Linux (`just check-linux`, `pg-dev`): every step passes: the lint, the module cycles (the text scan and the Linux compiler graph), and the tests (88 suites, 824 pass, 3 ignored).
- Screenshots (`just shots --base cae2f7a28c`): the fixture had changed since that reference was recorded, so it is recorded again. 18 of 21 scenarios match. `query`, `apply-view` and `borders` differ by at most 1 per channel, in thumbnails only, and are the thumbnail flake of the earlier validations: across three reruns each matched at least once, by the same repeating pixel counts (`apply-view` in two variants), and `query` differs from the working tree's own earlier run. Still open. The two recording attempts that ran beside the tests or the lint timed out (a scenario not settled in 10 s, then 30 s); alone, the recording passed.
- Perf (`just perf`): the pending-thumbnails bench stopped (`no such table: cat.thumb_folders`). It read its copy of the data without opening it through `Catalog::prepare`, as the other catalog benches do, so it met `v5-windows-rules`' schema, `user_version` 1, without migration 6's views. The data is `v6-windows-rules` (`e7576f0`): v5's catalog opened once by this build (migrations 2 to 7), the same 4,000 files and rows, made by `tools/profiling/bench_data.py`; the cache's schema is unchanged. Against the baseline (`fafa7d8`), twelve benches are within 0.21%, and two moved:

  | Bench | Baseline | `e7576f0` | Change |
  | --- | --- | --- | --- |
  | `pending_thumbs.at_height` | 3,412,526 | 3,998,089 | +17.16% |
  | `analyze_jpeg.sample` | 69,779,013 | 67,719,462 | -2.95% |

  - `pending_thumbs.at_height`: per function, the 585,563 instructions are SQLite's parse of the attached catalog's schema (`sqlite3RunParser`, `yy_reduce`, the tokenizer, `triggerStepAllocate`, `sqlite3AddColumn`, their allocations). The query's own execution (`sqlite3VdbeExec`) is up 13,616. The old data's schema was version 1, which the app's catalogs left behind with the notifications' migrations; the bench now opens the schema the app opens. `e7576f0`'s message says a change between v5 and v6 is the code's; the schema is part of the data, so this one is the data's.
  - `analyze_jpeg.sample`: M-48 made `Hasher::hash_rgb` test-only, so `dct16` has one caller left and is inlined into `hash_rgb_dihedral` (19,350,092 instructions in `dct16` before, 2,059,551 fewer in all).
  - The baseline is re-measured at `e7576f0` (`just perf --update`), with the profiles in `docs/perf/callgrind/e7576f0.tar.zst`; it reproduced the gated run's counts exactly.
- Not committed by this validation: another session's two edits in the working tree, which `pg-client` and `pg-desktop` need to build: `crates/client/src/text.rs` names the Yandex `Status` as `YandexStatus` (M-47 left two `Status` imports), and `crates/desktop/src/gallery/state.rs`'s `entry` reads `self.grid.item(i)` (M-45 left it calling itself). Every check above ran on the working tree with them.

## Open decisions

None. Unverified until their step, and confirmed there:

- M-12's per-concern split and the visibility rule (the first build in the step).
- The warm-target time of `cargo-modules` in CI (measured at M-0).
