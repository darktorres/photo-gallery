# Plan: fix the modularity findings (M-series)

Status (2026-11-08): investigated and scoped; not started. Churn and cost are not a constraint; the goal is the proper fix for each finding, not the cheapest one.

## Investigation results (2026-11-08)

Everything below was measured on the current working tree. The measurement scripts are in `tools/modgraph/` (module graph) and in the session scratchpad (split and DAG checks, not yet in the repo).

- **Toolchain and container.** `rust-toolchain.toml` pins `nightly-2026-11-06`. The `pg-dev` image has nightly rustc and cargo, `just`, `cargo-deny`, `cargo-machete`, `ruff` and `markdownlint`. It has no valgrind, no `cargo-modules`, and no gungraun runner. The cargo target volume is warm (13 GB).
- **Compiler-backed module graph.** `cargo-modules` 0.28.0 resolves paths with rust-analyzer's semantic model, not a text scan. It was run on all 28 library crates on two targets: Linux in `pg-cm-install` (the container, `target/cm2/`), and Windows on the host (`target/cmwin/`, Qt 6.12 MSVC kit on PATH). Command: `cargo modules dependencies --package <crate> --lib --no-externs --no-sysroot`, run by `tools/modgraph/compiler_graph.py`. Only the `uses` edges count; the `owns` (containment) edges stay in the output, because `--no-owns` drops every module that has no `uses` edge, and its items are then counted as the crate root's (this produced two false groups, codec and dedup, on 2026-11-08). `--no-fns` and `--no-types` must not be used, because they remove the function and type nodes that carry most cross-module edges. The first Linux run timed out at 400 seconds and produced nothing, because `cargo-modules` runs a full `cargo check` of the workspace (it rebuilt the Qt C++ bridge); a run with a longer budget completed. Run times on a warm target were not measured.
- **What the compiler graph cannot see.** Two blind spots, found by comparing the two graphs. (1) cxx-qt `#[cxx_qt::bridge]` modules are macro output, and `cargo-modules` does not list them (`shell::qobject` and `Shell` are missing), so it cannot see edges into or out of them: the `pg-desktop` groups A–C and J appear only in the text scan. (2) The text scan used to treat an inline `mod name { … }` block as part of its file, so it missed cycles between a file and an inline child (`dir` and `dir::unix`, `picker` and `picker::native`). The scan now makes each inline module a node (`tools/modgraph/module_graph.py`, 2026-11-08), and it finds those cycles too. The two tools still differ on cxx-qt bridges, so the plan uses the union, with each group checked in the source.
- **Platform-specific modules.** The Linux and Windows graphs differ by `cfg`. `pg-platform` has `dir::unix` on Linux and `dir::windows` on Windows, and `picker::native` exists only on Windows. A cycle in a `cfg` branch counts. Both targets are the gate for `pg-platform` and `pg-desktop`.
- **Rustdoc JSON** was ruled out: it covers public API only, not the references inside function bodies.
- **`unreachable_pub` (M-18), counted with `cargo rustc -- -W unreachable_pub` per crate.** Zero for 22 crates. `pg-video`: 1,165, all in bindgen output under `target/` (generated, not code to narrow). `pg-desktop`: 178, mostly cxx-qt bridge declarations (`type QAbstractListModel;`, `pub struct …Rust`), which the macro needs, plus about 40 ordinary `pub` items (for example `app::open`, `app::get`, `command_line::value`, `panic_log::install`, `picker::take`). `pg-cli`: 6. `pg-app`: 2. `pg-edit`: 1.
- **`pg-model` (open question from the previous draft).** Its 23 modules are consumed by 1 to 15 crates each (`num` by 15, `turns` by 14, `orientation` by 13, `rules` by 12). Only `min_side` and `group_distance` have one consumer, and both are macros that keep a Rust constant and a SQL string in sync, so they stay where they are. **Decision: `pg-model` stays as one crate.** It is the shared vocabulary; its fan-in is expected.
- **`catalog`'s `analysis` dependency.** Used only by `fixture::versions()` (`fixture.rs:43`, `fixture/raw.rs:11-12`) and by catalog's own tests. Nothing else in `catalog` uses it.

### Corrections to earlier drafts

- **M-11 (`borders`) is a layered split, not "no split".** The earlier "no split" reasoning counted cross-references between four files that shared `View` and `Params`. With those two in leaf modules, and the `Detector` scratch state (with `SolidLines`) in a third, the remaining edges form a DAG. Layers: `params`, `view`, `scratch`, `seam`, `side`, `detector`.
- **M-9 (`metadata`): the per-format `impl Meta` idea was wrong.** `impl Meta` has one method, `read`, which is the orchestrator. Its calls into the format modules run one way, so they are the intended design. The split is by format, and it forms a DAG.
- **M-4 `library_cell`** stays with the library service (from the previous draft).

## What was checked

- **Crate graph** (`cargo metadata`): no cycles. The layering in `crates/README.md` holds, except the two edges M-17 removes.
- **Module graph:** **ten** cycle groups (A–J below), from the union of the compiler graph (both targets) and the text scan, each checked in the source. Group J alone is eleven parent-bridge pairs in the desktop crate. The earlier counts were too low. (1) `super::` inside a cxx-qt `pub mod qobject` block refers to its own file's module, not the crate root, so two earlier edges were false. (2) The text scan missed the brace and glob re-export forms (`pub use pair::{…}`, `pub use grid::*`), which hid groups E and G; `tools/modgraph/module_graph.py` now handles both. (3) The text scan misses inline modules and `cargo-modules` misses cxx-qt bridge modules (see Investigation results), which is why the union is needed.
- **Layering:** `catalog` depends on `metadata` for one function (`can_hold_rating`) and on `analysis` only through the `fixture` feature.
- **Public surface:** `pub` items per crate: `client` 105, `model` 94, `catalog` 89, `platform` 53, `desktop` 37, `app` 35. The compiler check above replaces the estimate.
- **The working tree moves.** Since the first inventory in this session, uncommitted changes landed in files this plan measures: `codec/src/encode.rs` grew from 21 to 189 lines, `platform/src/file.rs` from 140 to 195, and `catalog/src/writes.rs` from 551 to 581. `codec/src/codecs.rs`, `metadata/src/lib.rs`, `edit/src/crop.rs`, `edit/src/rotate.rs`, `exiftool/src/lib.rs` and `model/src/orientation.rs` also show as modified in `git status`. None of these changes were made by this session. Every measurement here must be re-run at the start of M-0.

## Ground rules

- Work on the checked-out branch. Do not create branches.
- Implement a group of steps in full, then build and test once at the end of the group. Do not run cargo between steps.
- Moves keep the public paths callers use (`pub use` at the old location) until the last step. Callers change only where a step says so.
- Tests that touch a moved private item move with it, or get `pub(crate)` for the test.

## Gates

- **Linux:** `docker run --rm --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/root/.cargo/registry pg-dev` (`docker/README.md`). It runs `docker/check.sh` in CI's order. It covers every crate except the desktop app.
- **Windows:** `just lint` and `just test`. The desktop app builds only here, with `just build`.
- **Desktop steps** (M-1, M-4 for the `app` services, M-5, M-6, M-12): also `just shots`. These are pure moves, so the screenshot matrix must show no pixel change against `tools/shots/reference`.
- **Cargo doc:** the CI step is `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked` (`.github/workflows/ci.yml:283`). It failed on 2026-11-08 on links to Windows-only items (S-15 in `docs/REFACTOR_PLAN.md`). Run it on each step that moves or re-exports items.
- **Perf** (M-4, M-7, M-8): `just perf` runs callgrind (`Justfile:123-126`). Once the WSL-to-Docker migration is in place it runs in `pg-dev`. The baseline is `docs/perf/baseline.json` (S-16). The first perf run at M-0 is a check of that baseline against the current tree, before any change in this plan.

## Decisions (resolved)

1. **Perf gate.** WSL is being migrated to Docker, and the `pg-dev` container will have the bench tools installed when this plan starts. This plan makes no Dockerfile change for perf.
2. **M-15, `pg-library`.** Split `crop` and `borders` into `pg-crop` and `pg-borders`.
3. **M-4, `library_cell`.** It moves into the library service.
4. **`pg-model`.** Stays one crate (see the investigation results).

## Decision needed

- **`cargo-modules` (done).** It is a tool, not a dependency, and is pinned in two places: `docker/Dockerfile` (the image has 0.28.0, built 2026-11-08) and the Justfile (`cargo_modules_version`, which `just modgraph` checks and installs at the pin). The Linux graph runs in `pg-dev`; the Windows graph runs on the host through `just modgraph`.

## M-0: make the check repeatable, and make it the compiler's

- **Done:** the module-graph scripts are in `tools/modgraph/` (`module_graph.py`, `make_html.py`; standard library only, lint-clean under the repo's ruff). Output goes to `target/modgraph/`.
- **Cycle gate:** the module-cycle check is the union of (a) the compiler graph on Linux (`pg-dev`) and on Windows (the host, `just modgraph`), with only the `uses` edges, grouped by module (items map to their enclosing module, and `(root)` is the crate root), and (b) the text scan, which covers the cxx-qt bridge modules that the compiler graph cannot see. A group in either list fails the check unless it is in the allowlist.
- **Known false positives:** items that a crate re-exports at its root (`pub use app::App;`) appear as root items, so a group can show `(root)` without a real edge to the root (seen in `pg-app`). Check each group in the source before it goes on the allowlist.
- Add the `just modgraph` recipe (both scripts, in order).
- Commit an allowlist of the current cycle groups (`tools/modgraph-allow.txt`). The check fails on any cycle not on it. M-1, M-3, M-4, M-5 and M-6 remove entries, and M-16 empties the file.
- Commit the crate-layer file (see M-16). A draft from the current graph lists 30 crates.
- Re-run every measurement in this file, since the tree has changed.

## Cycles

Ten groups (2026-11-08). "Compiler" means `cargo-modules` on the target shown; "scan" means the text scan, with inline modules as nodes.

| Group | Members | Found by | Back-edges (checked in the source) |
|---|---|---|---|
| A (desktop) | root, `app`, `command_line`, `panic_log`, `providers`, `screenshot` | scan only (cxx-qt bridges) | `panic_log.rs:33` → `crate::message_box` (one edge) |
| B (desktop) | `controller`, `deadline`, `folders`, `shell` | scan only (cxx-qt bridges) | `deadline.rs:27` → `shell::qobject::Shell` (one edge) |
| C (desktop) | `facet_choices`, `filters` | scan only (cxx-qt bridges) | `facet_choices.rs:154` → `filters::count` (one edge) |
| D (app) | `app`, `images`, `library`, `reader` | compiler (both targets), scan | `impl App` blocks in six modules; see M-4 |
| E (catalog) | root, `dups`, `pair`, `schema`, `writes` | compiler (both targets), scan (partly) | `pair.rs:21` `use super::*`; `pair.rs:313` → `crate::Error::Other`; `dups.rs:4`, `writes.rs:10` → `crate::Catalog`; `writes.rs:24` → `super::{…FILE_RATING}`; `schema.rs:13` → `crate::{Error, Result, write_transaction}`; `schema.rs:105` → `crate::rules_name`; `lib.rs:1074` (`Catalog::file_state`) → `browse::read` |
| F (app) | `yandex`, `yandex::replay` | compiler (both targets), scan | `replay.rs:26` → `super::Source` (defined in `yandex.rs`); `yandex.rs:23` → `pub use replay::{Recording, Replay}` |
| G (catalog) | `browse::read`, `browse::read::facets`, `browse::read::grid` | compiler (both targets), scan | `facets.rs:13`, `grid.rs:9` → `super::scope` (defined in `read.rs`); `read.rs` re-exports them with `pub use …::*` |
| H (platform) | `dir`, `dir::unix` (Linux), `dir`, `dir::windows` (Windows) | compiler (per target), scan missed (inline modules) | `dir.rs` `unix` and `windows` (lines 456 and 345) → `use super::{Entry, Lister}`; `dir.rs` → `pub use unix::Dir` / `windows::Dir` |
| I (platform, Windows) | `picker`, `picker::native` | compiler (Windows), scan missed (inline module) | `picker.rs:68` (in `native`, `cfg(windows)`) → `use super::{FolderDialog, ShellItem}` |
| J (desktop, cxx-qt) | `X` and `X::qobject` for X in `animation`, `command_line`, `facet_choices`, `filters`, `folders`, `gallery`, `keymap`, `library`, `picker`, `settings`, `shell`, `trace`, `video_source`, `viewport`, `words`, `yandex` | scan only (cxx-qt bridges) | every bridge's first line: `type X = super::XRust;` (the backing struct lives in `X`), and `impl qobject::X` in `X`; see M-20 |

Groups A, B and C also contain their `qobject` modules, so their back-edges into `qobject` are part of J. M-1, M-5 and M-6 remove the non-bridge back-edges only; M-20 removes J.

`trace` is not in group A. The earlier list had it because of a false edge (see above). Groups H and I are in M-19.

### M-1: group A

The only back-edge into the root is `panic_log.rs:33`, which calls `crate::message_box`. The `command_line` and `trace` edges were false, as explained above.

Fix: delete `message_box` from `crates/desktop/src/lib.rs:131-134`, and have `panic_log` call `pg_platform::message::error_box("Photo Gallery", …)` directly (the root's `fatal` does the same). `pg-platform` already owns message boxes, so the dialog stays with the platform code and `panic_log` stops reaching into the root. Expected result: group A is gone. Confirm with the corrected scan.

Gate: desktop gates.

### M-2 (removed)

Once M-1 is done the root has no back-edges, so nothing more is needed. The root keeps `run`, `fatal`, and the module declarations.

### M-3: group E (catalog), with M-7

Group E is not one edge. Every catalog module that names `crate::Catalog`, `crate::Result` or a root helper closes a cycle back to the root, which is where those items live. The compiler-backed graph (`cargo-modules`, `uses` edges) has group E as `(root), pair, dups, writes, schema`. The back-edges, all checked in the source:

- `pair.rs:21` `use super::*;` in production code. A glob import of the whole root makes every root item a dependency of `pair`. Replace it with explicit imports.
- `pair.rs:313` `crate::Error::Other`. Becomes `crate::error::Error::Other`.
- `dups.rs:4`, `writes.rs:10`, `browse/read/*.rs`: `use crate::Catalog`. `Catalog` moves to `core.rs` in M-7, so these become `crate::core::Catalog`.
- `dups.rs:5`, `writes.rs:11`, `browse/read/*.rs`: `use crate::Result`. Becomes `crate::error::Result`.
- `writes.rs:24` `use super::{DELETE_FILE_RATING, SET_FILE_RATING};`. The two constants move to `types.rs` (M-7).
- `schema.rs:13` `use crate::{Error, Result, write_transaction};`. `Error` and `Result` come from `crate::error`. `write_transaction` (`lib.rs:398`) moves into `schema.rs`; the root and `writes.rs` import it from there.
- `schema.rs:105` `crate::rules_name` (`lib.rs:205`) moves to `types.rs`.
- `schema.rs:278` `crate::query::to_json`. `query` is an alias for the external `pg_query` (`lib.rs:18`), so it becomes `pg_query::to_json`.
- `lib.rs:1074` `Catalog::file_state`, a wrapper whose body is `browse::read::file_state(self, file_id)`. This is a root-to-`browse::read` edge that the children's `Catalog` import turns into a cycle. Delete the wrapper. Its only caller is `pg-thumbs`, `thumbs/src/lib.rs:129`, which calls `pg_catalog::browse::read::file_state(&catalog, id)` (`app/src/images.rs:136` already does this).

The root's `pub use pair::{Coverage, Pairing};` (`lib.rs:139`) stays.

Gate: Linux checks, Windows `just test`.

### M-3a: group G (catalog, `browse::read`)

The compiler-backed graph has `browse::read`, `browse::read::facets` and `browse::read::grid` as a cycle. Two causes, both removed by M-7 and by the `scope` move below:

- The children import `crate::Catalog` and `crate::Result` (the root's items), as in group E.
- `facets.rs:13` and `grid.rs:9` import `super::scope`, a private helper defined in `read.rs:28`. `read.rs` re-exports its children with `pub use facets::*;` and `pub use grid::*;`, so the helper creates the child-to-parent edge. Move `scope` to `browse/read/scope.rs`. `facets.rs:12` (`super::grid::FILE_JOINS`) is a sibling edge and stays.

The scanner's wider `browse::read` group (`changes`, `details`, `state`, `tree`, `views` joined through the same two causes) is not in the compiler-backed group; check it again after these steps.

### M-4: group D (the `app` cycle), as services

**What the code does.** `App` (`app/src/app.rs`) holds the catalog keeper and pool, the maker pool, the thumbnail cache, the last full-size decode, the preferences, the reader and session, the library cell, and the notes bus. Six modules add `impl App` blocks: `app.rs`, `edit.rs`, `images.rs`, `library.rs`, `reader.rs`, `views.rs`.

**Measured** (grep of `crates/app/src`, call sites only):

- `with_catalog` (uses `catalogs`, `config`) has 18 call sites, in `app` (2), `edit` (2), `images` (2), `library` (5), `reader` (2) and `views` (5). It is the main coupling.
- `with_maker` (uses `makers`) has 5 call sites, all in `images`.
- `library` methods use `library_cell`; `stop` calls `stop_library`; `edit::rate` calls `wake_writer`.
- `images` uses `thumbs` and `last_decode`; `edit` uses `prefs`; `reader` and `views` use no fields directly.
- Callers outside `crates/app` were not counted in this pass. Count them at M-4, by name, before choosing the delegates.

**Why not a shared bag.** A `Resources` struct holding all shared state would break the cycle, but every domain module would still depend on one object that holds everything. That is the cycle moved, not removed.

**Services** (each a module; each depends only on services above it in this list, so cycles cannot form):

| Service | Owns | Uses | Methods (from the measurement) |
|---|---|---|---|
| `config` (existing) | `Config` | — | — |
| `notes` | the `Note` type and the notes bus | — | `notes` |
| `makers` | the maker pool | — | `with_maker` |
| `catalogs` | the catalog keeper, the pool, and the catalog path (from `config`) | `config` | `with_catalog`, `file_path` |
| `prefs` | the preferences store | `config`, `notes` | `prefs`, `update_prefs` |
| `images` | `thumbs` (the `&'static ThumbCache`), `last_decode`, and the `Decoded` type | `catalogs`, `makers` | `thumbnail`, `keep`, `turn_jpeg`, `placeholder`, `full_image`, `consistent`, `source`, `decode` |
| `library` | `library_cell` | `catalogs`, `makers`, `images`, `prefs`, `notes`, `config` | `library`, `wake_writer`, `save_crops`, `roots`, `add_folder`, `remove_folder`, `rescan`, `turn`, `stop_library` |
| `reader` (existing `Reader`, `Session`) | the reader thread and the session | `catalogs` | `details`, `entry` |
| `views` | — | `catalogs` | `session_value`, `listing`, `save_listing`, `views`, `save_view`, `delete_view`, `view` |
| `edit` | — | `catalogs`, `prefs`, `library` | `rate`, `set_border_mark` |

`App` (app.rs) owns the services, and keeps every public method as a one-line delegate (`pub fn rate(&self, …) { self.edit().rate(…) }`), so `pg_app`'s public API and the desktop are unchanged. `stop` calls `reader` and `library`; `shutdown` calls `catalogs`, `images` and `notes`. `path_of` (app.rs:47) has no `App` dependency and moves to a leaf.

`yandex` stays a domain module. Its `start_folder` and `save` take the services they need, not `&App`.

The layer check (M-16) enforces the DAG. The exact split of `images` and `library` is read during the step.

Gate: Linux checks, Windows `just test`, `just shots` (the desktop's images come from here), perf (decision 1).

### M-5: group C

`filters::count` (`filters.rs:161`) has one caller: `facet_choices.rs:154`. Fix: move `count` into `facet_choices.rs`, which owns the `Counts` type it returns. Then `facet_choices` no longer depends on `filters`, and the reverse edge (`filters` uses `Counts` and `set_counts`) stays.

Gate: desktop gates.

### M-6: group B

Only `deadline.rs:27` (`use crate::shell::qobject::Shell`) points back into `shell`. Removing it breaks the group: no other edge from `controller`, `deadline` or `folders` returns to `shell`.

Fix, in `deadline.rs`:

- The thread-local `SHELL: RefCell<Option<CxxQtThread<Shell>>>` (line 31) becomes `RefCell<Option<Box<dyn Fn()>>>`. The thread-local needs no `Send`.
- `remember(thread: CxxQtThread<Shell>)` (line 35) becomes `remember(wake: impl Fn() + 'static)`.
- `deadline_fired` (line 58) calls the stored function, instead of queueing `shell.timeout()` itself.
- In `shell.rs:639` (`rearm`), the caller builds the closure: `let thread = self.qt_thread(); deadline::remember(move || { let _ = thread.queue(|shell| shell.timeout()); });`. `CxxQtThread` is `Clone` (cxx-qt 0.10, `threading.rs:68`), so this needs no new type.

The thread-local stays a global because the timer is a C++ `QTimer` that calls back into Rust with no object to carry the state. This matches the refactor design, where deadlines are sans-IO (`docs/REFACTOR_DESIGN.md`, "Time is sans-IO").

Gate: desktop gates.

### M-7: group E, and the `catalog` split

`crates/catalog/src/lib.rs` has 3,090 lines; 1,534 are production code.

Layers (checked as a DAG, method by method):

- `types.rs`: `Rebuilt` (with `carried_note`), `under_params`, and the shared types and constants (`Root`, `Folder`, `ScannedFile`, `WorkItem`, `Status`, `Hit`, `Versions`, `Carried`, `CropSource`, `Recorded`, `folder_path`, the SQL constants). Leaf.
- `lifecycle.rs`: rebuild, move-aside, locks, rules check, version stamps, `max_folder_id`, `pending_import_sql`, `work_list`. Uses `types`.
- `writers.rs`: `ScanWriter`, `AnalysisWriter`, `BordersWriter`, `write_file`. Uses `types`.
- `core.rs`: `Catalog`, `Tx`, and their methods. Uses `lifecycle`, `writers`, `types`, and `error`. It must not use `browse`, `writes`, `dups` or `pair`: the one production call into `browse` (`Catalog::file_state`, `lib.rs:1074`) is removed in M-3.
- `writes.rs` stays the file-write queue. `lib.rs` keeps the module declarations and the re-exports (`pub use core::{Catalog, Tx}`), so callers' paths do not change.
- Items the children and the root share move out of the root: `Catalog`/`Tx` to `core.rs`, `Result` and `Error` stay in `error.rs`, `write_transaction` to `schema.rs`, `rules_name` and the two `*_FILE_RATING` constants to `types.rs`.
- `browse/read/scope.rs` (M-3a): the `scope` helper moves out of `read.rs`.
- Check the result with `cargo-modules` (M-0): group E and group G must be gone from its output.

`Rebuilt` and `under_params` must go to `types.rs`, not `lifecycle.rs`. Otherwise `lifecycle` and `core` form a cycle (`rebuild` uses `Rebuilt`, and `Catalog` uses `lifecycle`), and `writers` uses `under_params`.

Gate: Linux checks, Windows `just test`, perf (decision 1).

## Layering

### M-17: edges that point the wrong way

- **`catalog` → `metadata`:** `can_hold_rating(format: Format) -> bool` (`metadata/src/lib.rs:43`) is one line, `!matches!(format, Format::Bmp)`, and depends only on `pg-model`'s `Format`. Callers: `catalog/src/writes.rs:71`, `writeback/src/lib.rs:377,386`. Move it to `pg-model`, next to `Format` (`model/src/format.rs`).
- **`catalog` → `analysis`:** optional, behind `fixture`. The fixture takes the two version constants from its callers (`fixture.rs:43`, `fixture/raw.rs:11-12`). Catalog's own tests get `analysis` as a **dev-dependency** (no cycle: `analysis` does not depend on `catalog`). The `fixture` feature then needs no `analysis`, and `catalog`'s normal dependencies are only storage crates.

Gate: Linux checks, Windows `just test`.

### M-18: public surface, checked by the compiler

Turn on rustc's `unreachable_pub` lint for the workspace (`[workspace.lints.rust] unreachable_pub = "warn"` in the root `Cargo.toml`; every crate already opts in). Measured scope (above):

- Generated code (`pg-video`'s bindgen output): exclude it at its include site, not by editing the output.
- cxx-qt bridge declarations in `pg-desktop`: these need a bridge-level allow, or a check that the macro compiles with them narrowed. Verify on Windows, since the desktop builds only there.
- Ordinary items: about 40 in `pg-desktop`, 6 in `pg-cli`, 2 in `pg-app`, 1 in `pg-edit`. Narrow each to `pub(crate)` unless another crate uses it. Items used only through a `pub use` that is itself unused get deleted.

Gate: Linux checks, Windows `just build` and `just test` (the desktop is checked on Windows).

### M-20: the cxx-qt backing types (group J, desktop)

Each cxx-qt bridge is a `pub mod qobject` inside a desktop module X, and its first line is `type X = super::XRust;`: the Rust struct that backs the QObject lives in X, and X implements the bridge type (`impl qobject::X`). Each such pair is a cycle. The scan finds 16 modules (the list in group J), and each is one `super::` path to the backing type, plus any other `super::` paths in the bridge (`command_line`, `keymap`, `picker` and `video_source` have a second one; check each).

Fix, per module X:

- Move `XRust` (its fields and the methods the bridge calls) into a leaf module, `X/state.rs` or a sibling file. The bridge names it by its crate path (`type X = crate::<leaf>::XRust;`).
- X keeps `impl qobject::X` and the public functions. That gives X → qobject → leaf, with no edge back.
- If `XRust` uses an item still in X, move that item to the leaf too. Check each one; do not move an item that the leaf's callers reach through X's public API without changing those callers.

Gate: the desktop crate builds only on Windows. Windows `just build` and `just test`, and `just shots` with no pixel change against `tools/shots/reference` (the moves are pure). Run `cargo-modules` (M-0) on the Windows target afterwards; the scan must show no J group.

### M-19: the parent-child cycles (groups F, H, I)

Three groups have the same shape: a parent module defines a trait or types, its child implements them, and the parent re-exports the child. Each is cut the same way: move the shared items into a new leaf module that neither side depends on upward.

- **F (app, `yandex`):** move `pub trait Source` (`yandex.rs:27`, with its four methods) into `crates/app/src/yandex/source.rs`. Its signatures use only `pg_yandex` types, so the leaf has no upward dependency (checked). `yandex.rs` keeps `pub use source::Source;` and `impl Source for Client`; `yandex/replay.rs:26` changes to `use super::source::Source;`. The re-export `pub use replay::{Recording, Replay}` stays, because the parent does not depend on the child any more through the trait.
- **H (platform, `dir`):** move `Entry` and `Lister` out of `dir.rs` into `crates/platform/src/dir/listing.rs`. `dir.rs` re-exports them, and both `mod unix` and `mod windows` (inline, in `dir.rs`) import from `super::listing`. First check that `Entry` and `Lister` use nothing defined in `dir.rs`; if they do, that item moves too.
- **I (platform, `picker`, Windows):** move `FolderDialog` and `ShellItem` out of `picker.rs` into `crates/platform/src/picker/types.rs`, and have `picker.rs` and `native` import them from there. Check the same way: the leaf may use only what sits below `picker` in the layering.

Gate: Linux checks; Windows `just build` and `just test`; both `cargo-modules` runs (M-0), which must show no group F, H or I on either target.

## Large single-file crates

Each split is a move. Items keep their visibility, and public paths stay the same through `pub use`. Every split below was checked as a DAG at group level, method by method. Re-run the check at M-0 and before each step.

### M-8: `thumbcache` (1,175 production lines in `lib.rs`)

Layers:

- `types.rs`: `FORMAT_JPEG`, `Source`, `Thumb`, `ThumbWork`, `Pending`, `Snapshot`, `Event`, `Usage`, `Drift`. Leaf. (`writer` and `reader` both use `Event`, `Source` and `ThumbWork`, which is why these must not stay in `lib.rs`.)
- `config.rs`: the cap functions and constants (`default_cap_bytes`, `parse_cap_gb`, `MAX_CAP_BYTES`, `MIN_DEFAULT_CAP_BYTES`, `MAX_DEFAULT_CAP_BYTES`, `whole_bytes`, `cache_path`), and the timing constants `TOUCH_AGE_S`, `TOUCH_FLUSH`, `HEAL_INTERVAL`, and `CACHE_VERSION`. The schema and the writer both read these, so they can't live with either.
- `schema.rs`: `SCHEMA`, `APPLICATION_ID`, `SEEDS`, `CONNECTION_PRAGMAS`, `BUSY_TIMEOUT`, `Found`, `expected`, `make_layout`, `judge`, `open_writer`, `hold_shared`, `open_file`.
- `reader.rs`: `pending_in`, `pending_sql`, `with_reader`.
- `writer.rs`: `write_loop`, `apply`, `touches_failure`, `put`, `total_and_cap`, `evict`, `heal_if_due`, `heal`, `shrink`, `Touches`, `Op`, `Inbox`, `Queue`, `Closing`, `Store`, the eviction and shrink constants, `unix_now`.
- `lib.rs`: `ThumbCache` and its methods (the orchestrator, which calls down into the other layers).

The check's cross-references that look like cycles are false: `reply.send(…)` is a channel, and `checked`, `cap` and `totals` in `SCHEMA` and `total_and_cap` are SQL names.

Coupling figures from the earlier tree, before this layering: 51 cross-references, most from `lib.rs` down into `writer` and `reader`. Those edges are one-way after the layering.

The 657 lines of tests (from line 1175) stay in `lib.rs` with `use super::…`.

Gate: Linux checks, Windows `just test`, perf (decision 1).

### M-9: `metadata` (655 production lines in `lib.rs`), by format

Layers (checked as a DAG):

- `xmp.rs`: the XMP namespaces, `XMP_WANTED`, `xmp_properties`, `xmp_value`, `percent`, `round_u8`, `Packet`, `MetaScratch`.
- `exif.rs`: `Tiff`, `Entry`, `Exif`, the TIFF and EXIF tag constants, `exif_block`, `tiff_orientation_patches`, and `orientation` (moved here from `lib.rs`, since it is EXIF reading).
- `container.rs`: `Container`, `extended_packets`, `MAX_INFLATED`, `inflate_zlib`, `ItxtXmp`, `png_itxt_xmp`, `gif_xmp`.
- `lib.rs`: `Meta` and `Meta::read` (the orchestrator), `patch_orientation`. `can_hold_rating` moves to `pg-model` under M-17.

Edges: `lib` → `{xmp, exif, container}`, and `container` → `{xmp, exif}`. Nothing calls back up. Gate: Linux checks, Windows `just test`.

### M-10: `codec` (`codecs.rs`, 814 production lines), by format

Layers (checked as a DAG):

- `convert.rs`: `to_rgb`, `flatten`, `convert`, the CMYK and float conversions, `luma8`, `luma16`, `rgb16`, `rgb_f32`, `target`, `zeroed`, `reserve`, `finish`, `check_len`, `over_white`, and `max_decode_bytes` (moved here: the PNG decoder and the dispatcher both use it).
- `errors.rs`: `png_error`, `gif_error`, `webp_error`, `jpeg_error`, `tiff_error`, `tiff_unknown_color`, `png_color`, `check_sample_format`.
- `png.rs`, `gif.rs`, `webp.rs`, `jpeg.rs`: one format each, with its `impl Codecs` block.
- `tiff.rs`: the TIFF decoder, with `TiffPlan`, `Stored` (moved from `codecs.rs`: the TIFF code is their only user) and `TiffDecoder`.
- `codecs.rs`: `Codecs`, `decode`, `trim`, `read_with` (the dispatcher).

Edges: `codecs` → each format; each format → `convert` and `errors`. Nothing calls back. Gate: Linux checks, Windows `just test`.

### M-11: `analysis` border detector (`borders.rs`, 693 production lines), layered

Layers (checked as a DAG):

- `params.rs`: `Params` and its `Default`, `BORDERS_VERSION`.
- `view.rs`: `View` and its methods, `to_count`. Leaf after `params`.
- `scratch.rs`: `Detector` (its scratch buffers and `capacity_bytes`, `trim`) and `SolidLines`, the buffers the detector holds.
- `seam.rs`: `SEAM_SPAN`, `refine_seam`, `seam_straightness`, `DetailSums` and its methods, `median2`, `hist_median2`, `median_f64`, `run_length`.
- `side.rs`: `starts`, `blurred`, `detect_side`, `solid_side`, `blur_side`, `judge`.
- `detector.rs`: `Detector::detect`, `detect`, `detect_with`, `detection`, `blur_threshold`, `PASSES`. The top layer.
- `borders.rs` keeps the public re-exports (`detect`, `Params`, `BORDERS_VERSION`).

Edges: `detector` → all lower layers; `side` → `seam`, `view`, `scratch`, `params`; `seam` → `view`, `params`; `scratch` → `seam`, `view`, `params`. Nothing calls back up. The one `const _` assertion (`PASSES`) stays with `detector`. The border tests run the detector on fixtures, so a changed result fails them. Gate: Linux checks, Windows `just test`.

### M-12: `desktop` `gallery.rs` (1,020 lines)

Targets: `roles.rs` (the `ROLE_*` constants, `CELL_ROLES`, `cell_roles`, `qt_int`). The `qobject` bridge stays in `gallery.rs`.

Risk: every desktop file has its own cxx-qt object, so no file in this repo splits one object's `impl` across files yet. The step's first action is a Windows `just build` with the split, before any other change. If it fails, keep the constants split and leave the `impl` where it is.

Gate: desktop gates.

## Platform boundary

### M-13: Win32 calls only in `pg-platform`

Two sites, both checked:

- `crates/video/src/ffi.rs:189-205`: the Windows branch of `Lib::open` loads the DLL with `libloading::os::windows::Library::load_with_flags` (`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`). This moves to `pg_platform::dll::load_with_search(path)`. `pg-video` keeps the FFmpeg symbol lookup. `pg-platform` already depends on `windows-sys`, so the new module adds no crate.
- `crates/gallery/src/main.rs:50-80`: `attach_console` calls `windows-sys` directly (`GetStdHandle`, `AttachConsole`, `CreateFileW`, `SetStdHandle`). This moves to `pg_platform::console::attach_to_parent()`, and `gallery` calls it.

After both moves, the README's "the one exception" (`crates/README.md:12`) is removed, and the rule holds without exceptions.

Gate: Linux checks (the platform crate's Linux code must still build), Windows `just test`, and the FFmpeg video tests on Windows.

## Dev-dependency cycles

### M-14: `testkit` depends on nothing internal

`testkit` has two internal dependencies, and both can go:

- `pub use pg_catalog::fixture;` (`testkit/src/lib.rs:11`). **Nothing uses it** (`pg_testkit::fixture` appears nowhere). Delete it. This removes the dependency on `pg-catalog`, which is the `catalog` ↔ `testkit` dev cycle.
- `events.rs` (the `Quiet` bus helper, which uses `pg_jobs::events`). Four crates use it: `cli`, `library`, `thumbcache`, `thumbs`. Move it into `pg-jobs` behind a `testing` feature, and give those four crates `features = ["testing"]` in their dev-dependencies. This removes the dependency on `pg-jobs`, which is the `jobs` ↔ `testkit` dev cycle, since `jobs` dev-depends on `testkit`.

`testkit` keeps the scratch folders, `held.rs` and the file generators, with no internal dependencies.

Gate: Linux checks, Windows `just test`. `cargo tree -d` must show no duplicate workspace crates in the test builds.

## Crate granularity

### M-15: `pg-library`'s reach

`pg-library` has 13 internal dependencies. Checked: `edit`, `exiftool`, `writeback`, `codec`, `metadata`, `render`, `video` and `testkit` (dev) are referenced only from `crop.rs` and `borders.rs`. The runner, `dups.rs` and `lib.rs` use the rest. So the split removes **three** dependencies from `pg-library` (`edit`, `exiftool`, `writeback`) and moves the code that needs the others with it.

Decided: a `pg-crop` crate (`crop.rs`, 204 lines) and a `pg-borders` crate (`borders.rs`, 77 lines). The runner (`runner.rs:357` for `crop::save`, `:470` for `borders::find`) depends on both. The callers of `pg_library::borders` (`cli/src/lib.rs:423` and `library/tests/borders.rs`) name `pg-borders` directly, as S-20.3 did for the other passes. The tests move with the code (`library/tests/crop.rs`, `crop_roots.rs`, `borders.rs`).

Gate: Linux and Windows checks.

## Closing

### M-16: enforce, at two levels

- **Module cycles:** empty `tools/modgraph-allow.txt`, so the check fails on any module cycle, and run the `modgraph` check in CI next to the `cargo doc` step.
- **Crate layers:** commit `tools/crate-layers.txt`, one line per crate listing its allowed internal dependencies (normal dependencies). A draft from the current graph (30 crates, dependencies first) is in the session scratchpad as `crate-layers.draft.txt`. CI checks the file against `cargo metadata`: a new internal edge fails until the file changes in the same commit. The file is written from the graph after M-17, so it includes the `app` services, `pg-crop` and `pg-borders`.
- Update `crates/README.md`: the dependency section, the Win32 rule (M-13), `testkit` (M-14), the new crates (M-15), the `app` services (M-4), and the moved `can_hold_rating` (M-17).
- Add a ledger row for each M step to `docs/REFACTOR_PLAN.md`, using the same status words.
- Record the final check results in this file.

## Order

1. M-0: tooling (decision: `cargo-modules` in the container), re-measurement, and the crate-layer file.
2. M-1, M-3, M-3a, M-5, M-6, M-19, M-20 (cycle fixes) and M-17 (layering), then one Windows build and `just shots`, then one Linux check run.
3. M-4 (`app` services), with the perf gate.
4. M-7 (`catalog`). Groups E and G close only here: M-3 and M-3a remove the edges that do not depend on the move (`pair`'s glob import, `Error`, the `file_state` wrapper, `scope`), and M-7 moves `Catalog` out of the root, which the remaining edges need.
5. M-8 to M-11 (file splits, each re-checked first), M-12 (`gallery.rs`, its build check first).
6. M-13 and M-14, independent of everything above.
7. M-15.
8. M-18 (`unreachable_pub`), after the splits, so the public surface reflects the final layout.
9. M-16, last.

## Open decisions

1. None open on `cargo-modules`: it is in the Dockerfile and the Justfile (see Decision needed).
2. Any new decision found while re-measuring at M-0 goes here.
