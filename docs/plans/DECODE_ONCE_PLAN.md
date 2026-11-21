# Decode each new file once, feature toggles, incremental grid fill: phased plan

## Context

A new file is read and decoded three times by three passes (`Inner::round` in `crates/library/src/runner.rs`: Crops, Scans, **Import**, **Borders**, **Thumbs**): import decodes DCT-scaled to >= 512x320 (`Analyzer::analyze_image`), borders decode full-size in stored orientation (`Analyzer::find_borders`), thumbs decode scaled to >= 4x grid height (`Maker::make_image`). `D:\wallhaven\downloads` (counted read-only on 2026-11-12, recursive): 85,073 files / 328.5 GB decimal (305.9 GiB), 67% JPEG (avg 3.1 MB; wallpapers: 1080p decodes at 1/2, 4K at 1/4 for the import), 33% PNG (avg 5.4 MB). Kingston NVMe, 12 logical cores, 64 GB RAM, so the work is **CPU-bound**. The user paused at borders, so the thumbs pass never ran and tiles were made on demand while scrolling (`crates/desktop/cpp/providers.cpp`: 8 threads, no explicit priority). Open item R-86 (`docs/plans/REVIEW.md:41`).

The plan makes one full decode per new file (Phase 3 moves the import to the full stored decode; Phase 5 merges the stages into one pass).

## Ground rules for every phase

- Cite and navigate by **symbol**; line numbers drift. Homes after Modularity (re-checked 2026-11-10 at `d2cbc13`): `Catalog::prepare` in `catalog/src/core.rs`; `pending_borders`/`pdq_hashes` in `catalog/src/work.rs`; `status` in `catalog/src/lookup.rs`; `set_borders_version`/`set_import_version`/`set_mark` in `lifecycle.rs`; `store_analysis`/`store_failure`/`store_borders` and `store_pdq_pair` in `writers.rs`; `Versions` in `types.rs`; `has_borders!`/`stale!`/`work_columns!`/`pending_files!`/`not_being_written!` in `sql.rs`; `MIGRATIONS`/`SEEDS`/`expected` in `schema.rs` (`carry_rows` in `rebuild.rs`); thumbcache `pending_sql` in `reader.rs`, `CACHE_VERSION` in `config.rs`.
- **Crate layers (M-16):** `tools/crate-layers.txt` lists every crate's exact internal dependencies, and `tools/boundaries.py` (in `just lint`) checks it. A dependency must point to a line *earlier* in the file (`tools/boundaries.py:107`). Any new internal edge edits that file in the same commit. Also enforced: a `pub use` names only a child module of its module; no platform `cfg` outside `pg-platform`; `unreachable_pub` is on (new items are `pub(crate)` unless another crate names them).
- Docs and strings are updated inside the phase that changes behaviour. The callgrind baseline is refreshed in Phase 6 (see step 2 there).
- Tests are written with each phase and run in Phase 6. Golden values are captured in Phase 6, never between phases (3.11).
- A phase that changes a manifest or adds a crate edge commits `Cargo.lock` with it: `justfile:70` runs `cargo test --workspace --locked`, and `Cargo.lock` is tracked. Phase 5 does (the new `import` and `bench` edges). Phase 3 changes no manifest: the Dups pass is a module of `pg-import` (3.7).
- **Stored values have one definition.** Every value written to `analysis` is computed by one function from one decode of the file, independent of the toggles, the grid height and the path that ran first: the duplicate hashes and ThumbHash come from the full stored decode F (`analysis::hashes`, 3.1), and the borders from the stored pixels. The duplicate hashes are computed in every mode (the toggle gates the pairing, 3.6). Borders are the one stored value a toggle can leave out: their columns are NULL while border detection is off. A test checks that every stored analysis value is identical in all eight toggle combinations, comparing the border columns only where both catalogs ran borders (5.12, 6.6). Thumbnails are display artifacts, never inputs to a stored value, and every thumbnail is downscaled from the same full decode F (5.1), so the path that made it does not change its pixels. Superseded 2026-11-16 (commit aeb7d2c): with duplicates detection off the import stores no hashes, and the Hashes pass completes them with the same functions (`analyze::image_hashes`); `the_hashes_pass_makes_what_an_import_with_them_would_have` pins identical values.
  - Superseded 2026-11-17 (commit 5fcff8bb): the import decodes nothing and makes no hashes. Its ThumbHash moved to a Placeholders pass right after Thumbs, made from the cached thumbnail or a DCT-scaled decode of the original (`crates/README.md`, "Placeholders"). The DCT-scaled decoding is unchanged; the duplicate hashes are made by the Hashes pass from their own decode.
- Process: **wait for the Modularity work to land** (the user will say when; git already has review 4's steps 1 to 9 at `04ff156`, `37b8e9b` and `e6cdf16`, though `MODULARITY_REVIEW_4.md:3` still says "all open"). Then implement **one phase at a time: implement, commit on `main`, next phase. No cargo, tests, benches or other validation between phases; validation happens once, at the end (Phase 6).** Fixes found in validation go in a closing commit. No new branches; commit only the phase's files, never the user's other work.
- **Transactions are short.** The sink commits at most 100 files or 250 ms of work per transaction (Phase 5), because the app's writers wait only 100 ms for the write lock (`crates/app/src/inbox.rs:73`).

## Phase 1: remove the "no version bumps before release" rule (docs and comments only)

No CLAUDE.md; nothing enforces it (no lint, CI, test or golden pins `IMPORT_VERSION == 1` or `CACHE_VERSION == 1`); `.claude/worktrees/` is git-excluded stale copies (exclude from greps). In each place below, replace the rule with a pointer to the module doc of `crates/model/src/versions.rs` (lines 1-9), which states the bump rule once; do not restate it:

- the `IMPORT_VERSION` doc in `crates/analysis/src/analyze.rs` (the constant is at `:27`);
- the `CACHE_VERSION` doc (`crates/thumbcache/src/config.rs`);
- `crates/README.md` cache layout paragraph (line 316);
- `docs/plans/PLAN.md` "Schema (user decision, 2026-10-30 ...)" paragraph (line 334; "no migrations yet", line 341);
- `d28e2f72^:docs/temp/REFACTOR_PLAN.md:390` ("no schema bumps before release");
- `docs/plans/BOUNDED_MEMORY_PLAN.md:2004` ("the cache layout freezes with BASELINE", which rests on the rule) and `:1197` ("CACHE_VERSION stays 1").

Not in the list: `docs/temp/S17_REMAINDERS_PLAN.md` (since deleted; it had no rule); `docs/plans/REVIEW.md` R-123 (line 85; the catalog split, not the version rule). History gets "Superseded" notes dated the day of the commit. No existing note exists to copy (the S-17.7 plan had an unrelated bullet there; that plan is removed), so the first one written sets the style: a one-line blockquote under the old rule, `> Superseded YYYY-MM-DD (commit <sha>): <new rule>.`

Other old notes: `docs/plans/NTFS_MFT_SCAN_PLAN.md` (190, 258); `docs/reference/REFACTOR_MEASUREMENTS.md` (the S-13.5 section, line 111; its home `FINDINGS.md:1574` is blank, so the note goes under the section heading instead); `docs/plans/EXIFTOOL_PORT_PLAN.md:54` (its `analyze.rs:29` cite is stale: `IMPORT_VERSION` is at line 27; correct the cite in the same edit). Close ledger items L-24 and L-59 in `docs/plans/REVIEW_LEDGER.md` (both Open; L-24 cites `crates/README.md:249`, which is catalog text; the cache paragraph is at line 316).

**Leave** `crates/catalog/src/schema.rs` and `crates/README.md` "baseline is frozen" (line 253; a migration policy, not a version rule).

Grep afterwards, and every hit must be a Superseded note or a line that stays true: `"first release"`, `"pre-release"`, `"no schema bumps"`, `"stays 1"`, `"still 1"`, `"edited in place"`, `"rebuilt before release"`, `"no migrations"`, `"no bump"`, `"before release"`, `"freeze"`.

## Phase 2: reorder, the features, disabled-feature messages, UI/render priority (no schema change)

### 2.1 The features values

> Superseded 2026-11-16 (commit aeb7d2c): `Features { borders, duplicates }` only (no `thumbnails`, no `Need`); the import takes `Hashing` (`Compute` or `Skip`) from it.

`pg_model::features` (new `crates/model/src/features.rs`, `pub mod features;` in `crates/model/src/lib.rs`) holds `Features` and `Need`. The model is the bottom layer (`tools/crate-layers.txt:9`), and the library, app and client all name these types, so the types live there. One value, one source of truth: the prefs (`crates/app/src/prefs.rs`), read at start-up and written by the Settings setters.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features { pub borders: bool, pub thumbnails: bool }   // Phase 3 adds `pub duplicates: bool`

impl Features {
    pub const ALL: Features = Features { borders: true, thumbnails: true };
    pub const fn wants(self, need: Need) -> bool {
        match need { Need::Borders => self.borders, Need::Thumbnails => self.thumbnails }
    }
}

/// A feature a control or a start option depends on. `code` is the value of QML's `Need` enum (`Need.qml`, 2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need { Borders, Thumbnails }   // Phase 3 adds Duplicates, code 2

impl Need {
    pub const ALL: [Need; 2] = [Need::Borders, Need::Thumbnails];
    pub const fn code(self) -> i32 { match self { Need::Borders => 0, Need::Thumbnails => 1 } }
    pub fn from_code(code: i32) -> Option<Need> { Need::ALL.into_iter().find(|n| n.code() == code) }
    pub const fn bit(self) -> u32 { 1 << self.code() }
}
```

No `Default` derive: a derived default would be all off, and the prefs default is all on.

### 2.2 Storage and the job

> Superseded 2026-11-16 (commit aeb7d2c): `FeatureCell` holds the value and wakes the job; there is no `Cx` snapshot. Each pass takes a `still_wanted` closure (`import_while`, `run_while`, `find_while`, `hashes::hash`) that reads the cell after each batch.

`FeatureCell` lives in `pg_library` (`crates/library/src/runner.rs`), beside `HeightCell` (`:48-80`). The job's `Inner` and `Cx` need it, and `app` depends on `library` (`tools/crate-layers.txt:45-46`), so it cannot live in `app`.

```rust
#[derive(Clone)]
pub struct FeatureCell(Arc<Mutex<FeatureState>>);
struct FeatureState { features: Features, wake: Option<Arc<dyn Fn() + Send + Sync>> }
impl FeatureCell {
    pub fn new(features: Features) -> FeatureCell;
    pub fn get(&self) -> Features;
    /// Stores the value and wakes the job if one listens. It never stops a round.
    pub fn set(&self, features: Features);
    fn listen(&self, wake: impl Fn() + Send + Sync + 'static);
}
```

Wiring: `Library::start` (`runner.rs:151`) takes `features: FeatureCell` after the heights (`:154`); `Inner` holds it (after `:106`). `round()` (`:399-410`) takes one snapshot into `Cx` (`:416`, a new field `features: Features`), so a pass never sees two values in one round. `LibraryService::new` gains a `features: Features` parameter (`crates/app/src/library.rs:228-247`) and builds the cell from it. It is called from `app.rs:81`, after `PrefStore::open` at `app.rs:78`, so the cell is seeded from the prefs before the job starts. `LibraryService::set_features(&self, f: Features)` writes the cell; it works before the job starts, as `set_thumb_height` does (`library.rs:252`).

A change **wakes but does not stop** a round: a feature switched off ends its pass as `Done` (2.9), and a switched-on feature is caught up on the next round.

### 2.3 Prefs, the settings bridge, and the Library panel

> Superseded 2026-11-16 (commit aeb7d2c): the prefs are `detect_borders` and `detect_duplicates`, and the panel has those two checkboxes.

Prefs (`crates/app/src/prefs.rs`; struct `:21-40`, `Default` impl `:98-108`). The struct-level `#[serde(default)]` (`:21`) reads an old file as on:

```rust
/// Detect the borders of the files, in the background (on by default). Off: the pass waits; its results stay and stay shown.
pub detect_borders: bool,
/// Make thumbnails of first-import files ahead of the grid (on by default).
pub pregenerate_thumbnails: bool,
```

`impl Prefs { pub fn features(&self) -> Features { Features { borders: self.detect_borders, thumbnails: self.pregenerate_thumbnails } } }`. Phase 3 adds `detect_duplicates` the same way.

Settings bridge (`crates/desktop/src/settings_bridge.rs:11-14`, `settings.rs:29-41`): two qproperties after `realSize`, each with NOTIFY. Each setter writes the prefs and then tells the library:

```rust
#[qproperty(bool, detect_borders, cxx_name = "detectBorders", READ = read_detect_borders, WRITE = set_detect_borders_saved, NOTIFY, FINAL)]
#[qproperty(bool, pregenerate_thumbnails, cxx_name = "pregenerateThumbnails", READ = read_pregenerate_thumbnails, WRITE = set_pregenerate_thumbnails_saved, NOTIFY, FINAL)]
```

```rust
pub(crate) fn set_detect_borders_saved(self: Pin<&mut Self>, value: bool) {
    if app::get().prefs().get().detect_borders != value {
        change_prefs(|p| p.detect_borders = value);
        app::get().library().set_features(app::get().prefs().get().features());
        self.detect_borders_changed();
    }
}
```

The pregenerate setter has the same shape. Today the setters only write prefs (the `set_write_back_saved` pattern, `settings.rs:29-34`), so this is one new call and no new invokable.

LibraryPanel (`crates/desktop/qml/LibraryPanel.qml`): the fixed pane grows from 440 to 560 px high, and the folder list absorbs the extra height. A ScrollView over the whole pane would hide the Close button, so it is not used. Two CheckBoxes go after the write-back CheckBox (`:67-73`), bound to new required properties `detectBorders: bool`, `pregenerateThumbnails: bool`, `bordersUnchecked: int`, and to signals `detectBordersToggled(bool on)` and `pregenerateThumbnailsToggled(bool on)`:

```qml
CheckBox { text: words.phrase("ui-detect-borders-count", 0, Qt.locale().toString(panel.bordersUnchecked), ""); checked: panel.detectBorders; onToggled: panel.detectBordersToggled(checked) }
CheckBox { text: words.text("ui-pregenerate-thumbnails"); checked: panel.pregenerateThumbnails; onToggled: panel.pregenerateThumbnailsToggled(checked) }
```

`Main.qml` LibraryPanel (`:611-620`) binds `detectBorders: appSettings.detectBorders`, `pregenerateThumbnails: appSettings.pregenerateThumbnails`, `bordersUnchecked: library.bordersUnchecked`, and writes back with `onDetectBordersToggled: (on) => appSettings.detectBorders = on` and the pregenerate counterpart.

### 2.4 One gate for feature-dependent actions

> Superseded 2026-11-16 (commit aeb7d2c): for §2.4 to 2.7 the gate is borders-only. The pure rules are in `pg_client::features` (`uses_borders`, `uses_borders_in`, `without_borders`, `grid_refusal`); the start is in `desktop/src/controller.rs` (`start_state`, `start_refused`); one popup is in `Main.qml`; the strings are `ui-borders-disabled`, `ui-options-borders-disabled` and `ui-nothing-to-crop`.

`pg_model::features::Need` names the features (2.1). The pure functions live in `crates/client/src/features.rs` (new):

```rust
pub fn grid_need(grid: Grid) -> Option<Need>                             // Grid::Borders and Grid::MarkBorder -> Borders
pub fn grid_refusal(grid: Grid, features: Features) -> Option<Need>      // the need, when its feature is off
pub fn needs_mask(needs: &[Need]) -> i32                                 // the OR of Need::bit
pub fn disabled_key(need: Need) -> &'static str                          // "ui-borders-disabled", "ui-thumbnails-disabled"
pub fn view_refusal(listing: &Listing, features: Features) -> Vec<Need>   // 2.7: the disabled needs a view uses; empty when it can apply
```

**Where the gate sits: the desktop boundary.** The controller and `keys::grid` / `keys::viewer` stay pure: they take no features (`client/src/keys.rs:103, :124`). The gate runs where the prefs are read:

- Grid keys: `shell.rs` `key_pressed` (`:157-168`), after `keymap::grid_key` returns the grid key and before dispatch: `if let Some(need) = features::grid_refusal(grid, app::get().prefs().get().features()) { self.as_mut().feature_refused(need.bit() as i32); return true; }`.
- QML asks through a synchronous invokable `featureOn(need: i32) -> bool` (`shell_bridge.rs`, the same kind as `viewerStep`, `:86`): `Need::from_code(need).is_some_and(|n| app::get().prefs().get().features().wants(n))`.
- Two signals. `featureRefused(needs: i32)` is emitted when a control was used while its feature is off (a mask of `Need::bit`). `optionsRefused(needs: i32)` is emitted when a start option or a saved view is not applied because it needs a disabled feature (2.7).
- There is no `Event::FeaturesChanged` and no features in the client controller. The prefs are read at each key press and each start, so there is one source and no copy to keep in step.

The popup text is looked up by need (`disabled_key`). Phase 3 adds `ui-duplicates-disabled`. There is no second copy of the rule.

### 2.5 The popup

`Main.qml` has no Popup today (the one Popup-like item is `RatingPopup`, `Viewer.qml:514`, a Label). Add one as a child of `win` after `libraryLoader` (`Main.qml:606`). Qt parents a Popup's content to the window overlay, so it draws above the grid, the viewer and the panels whatever its place in the tree (the Overlay documentation, `qquickpopup.cpp`). It is non-modal, so a press outside it closes it and reaches the control under it: `blockInput` is false for a non-modal popup and `handlePress` closes it on an outside press (`quicktemplates/qquickpopup.cpp:584-606`). Each control's own handler does the gating.

```qml
Popup {
    id: featurePopup
    property string message: ""
    modal: false
    focus: false
    padding: 12
    closePolicy: Popup.CloseOnPressOutside
    x: Math.round((win.width - width) / 2)
    y: win.height - height - 64
    Label { text: featurePopup.message; wrapMode: Text.Wrap; width: Math.min(360, win.width - 48) }
    Timer { id: featurePopupTimer; interval: 4000; onTriggered: featurePopup.close() }   // Qt's Popup has no timeout
    function show(text) { message = text; open(); featurePopupTimer.restart() }
    // A control was used while its feature is off: the disabled text of the one feature (a bit of the mask).
    function refused(needs) { show(words.text(needs & (1 << Need.Borders) ? "ui-borders-disabled" : "ui-thumbnails-disabled")) }
    // A start option or a saved view was not applied: ui-start-option-disabled, with the feature names joined by a locale-aware separator.
    function optionsRefused(needs) {
        var names = []
        if (needs & (1 << Need.Borders)) names.push(words.text("ui-feature-borders"))
        if (needs & (1 << Need.Thumbnails)) names.push(words.text("ui-feature-thumbnails"))
        show(words.phrase("ui-start-option-disabled", 0, Qt.locale().createSeparatedList(names), ""))   // the locale's list separator (QML Locale, Qt 6.10+; no bridge needed)
    }
}
Connections {
    target: shell
    function onFeatureRefused(needs) { featurePopup.refused(needs) }
    function onOptionsRefused(needs) { featurePopup.optionsRefused(needs) }
}
```

`crates/desktop/qml/Need.qml` (new, like `PanelKind.qml`): `QtObject { enum Kind { Borders, Thumbnails } }`, whose values equal `Need::code`. The popup text names the feature and says to turn it on in the Library panel (2.11).

### 2.6 Intercepts, per control

When the feature is off, each control below pops up and performs no action. When it is on, it behaves as today. A control is never hidden or greyed out (the user's rule). A control that does nothing for another reason also pops up with the reason, so no click goes unexplained (decided 2026-11-13): nothing to crop (`ui-nothing-to-crop`), a viewer N on a file with no border (`ui-no-border-here`), and a grid N on a file that is not checked yet (`ui-borders-not-checked`, the Pending state below the table).

| Control | Input | Need | Intercept point | Note |
|---|---|---|---|---|
| Borders facet chips | click | Borders | `FilterBar.qml:239-242` | no filter change |
| Crop button | click | Borders | `FilterBar.qml:252-260` | `enabled: bar.croppable > 0` is removed. Feature on and `croppable == 0` shows `ui-nothing-to-crop` |
| Review combo, borders entries | choose | Borders | `FilterBar.qml:245-251` | index 0 is "any" (`reviewChoices`, `:49`) |
| Sort combo, "By border depth" | choose | Borders | `TopBar.qml` sort box (`:107-113`); `Main.qml` `onSortActivated` (`:463-466`) | the last sort choice (`client/src/sort.rs:21`) |
| Duplicates combo (Phase 3) | choose | Duplicates | `FilterBar.qml:217-223` | gated in Phase 3, with the Duplicates toggle |
| Grid **B** | key | Borders | `shell.rs` `key_pressed` (`grid_refusal`) | no `Change::ShowBorders` is produced |
| Grid **N** | key | Borders | `shell.rs` `key_pressed` (`grid_refusal`) | feature off: the disabled popup. Feature on: `Borders::Pending` (due, not checked yet) shows `ui-borders-not-checked`; `None` or `Unknown` (never checked: video, animated, tiny) shows `ui-no-border-here`; `Found` or `Marked` marks it (the grid's border state, below the table). |
| Viewer **B** | key | Borders | `Main.qml` `onBordersToggled` (`:596`) and `onBordersRequested` (`:418`) | the QML checks before `shell.toggleBorders()` |
| Viewer **N** | key | Borders | `client/src/keys.rs:138` (guard removed); `Main.qml` `onBorderMarkToggled` (`:597`) | the QML decides: disabled popup, no-border popup, or the mark |
| Viewer N button | visible | Borders | `RatingBar.qml:18` and `:50` | `visible: bar.hasBorders` is removed, so the button is always visible |
| Inbox **Retry** (thumbnails row) | click | Thumbnails | `InboxPanel.qml:125-128` (`retrySelected`), button `:469-474`, R key `:236-237` | no retry and no row removal. Only the Thumbs ledger has a Retry action (`notice.rs:1163`) |
| Saved views | apply | the whole view, if it needs a disabled feature | 2.7 | not applied; one popup |
| `--borders` start option | apply at start | Borders | `controller.rs:113` | gated, like Grid B: no action, one popup |

**The grid's border state.** `Entry.borders` (`crates/model/src/grid.rs`) is read in `entry_from_row` (`crates/catalog/src/browse/read/grid.rs:53-81`). It gains `Borders::Pending`: a row with no cut columns and `a.borders_due = 1`. The query reads `a.borders_due` as column 16, after the marked flag at 15, so `SORT_COLUMN` moves from 16 to 17. The detail query (`grid.rs:103`) uses the same column list. The one exhaustive-looking match, `desktop/src/gallery/items.rs:185`, has a `_` arm, and the other uses compare with `==` or `matches!`, so nothing else changes. `Pending` needs no migration, and the feature-off case is answered first by the disabled popup. The state reads `a.borders_due`, not a second copy of the `has_borders!` predicate in the client, so there is one definition of which files are checked.

**Why a combo reverts explicitly.** Qt sets `currentIndex` before it emits `activated` (`qquickcombobox.cpp:378-381`), so a rejected pick leaves the combo showing the pick. Each combo's `onActivated` puts `currentIndex` back. For the filter combos that is `currentIndex = Qt.binding(() => filters.borderMarkedIndex)` (and the duplicates index in Phase 3): a JavaScript assignment removes a QML binding, so the binding is restored by hand. The C++ `setCurrentItemAtIndex` (`qquickcombobox.cpp:680-693`) does not remove it. The sort combo reverts to `shell.sortIndex()` (the controller's sort, `shell_bridge.rs:274`, as `Main.qml:271` and `:743` already read it). It must not revert to the `sortIndex` alias of `TopBar.qml:43`: that alias is the box's own index, so assigning it changes nothing. The binding restore is settled by reading the source (`hidePopup` sets the index before it emits `activated`, `qquickcombobox.cpp:378-385`); the test runs at build.

### 2.7 Start options and saved views

`view_refusal` returns the disabled needs that a view or a start option uses. A view that needs one is not applied as a whole; a view that needs none is applied as saved:

```rust
/// The disabled needs that a view uses (for the popup). Empty: the view applies as saved.
pub fn view_refusal(listing: &Listing, features: Features) -> Vec<Need> {
    let mut refused = Vec::new();
    if !features.wants(Need::Borders) {
        let q = &listing.query;
        if !q.borders.is_empty() || q.border_marked.is_some() || listing.sort.key == SortKey::Borders { refused.push(Need::Borders); }
    }
    refused   // Phase 3 adds the same for `q.duplicates` (Need::Duplicates)
}
```

`Listing` is `pg_model::view`; the query fields are `query.rs:219-256`.

- **Starts.** The client `Start` struct (`controller.rs:38-50`) gains `pub gated: Vec<Need>` (`Start::new`, `:54`, sets it empty). In the desktop `start_state` (`crates/desktop/src/controller.rs:91-118`), after `--apply-view` (`:104-107`): `let gated = features::view_refusal(&listing, prefs.features());`. A non-empty `gated` means the start shows the default listing instead of the view, and the start carries `gated`. The `started` signal (`shell_bridge.rs:69-70`, `#[qsignal]`) is raised for the `Start` that `start_state` builds (`controller.rs:91-118`), so the handler that raises it reads `gated` from that `Start` and emits `optionsRefused(needs_mask(&gated))` when the list is not empty. That is one popup per start.
- **Menu.** `shell.rs` `apply_view` (`:375-381`): `let gated = features::view_refusal(&v.listing, prefs.features());`. If `gated` is not empty, it emits `optionsRefused(needs_mask(&gated))` and the view is not applied. Otherwise it emits `ListingEvent::ViewApplied` with `v.listing`. Saved views and start options share this one function.

The view is not applied as a whole. A saved view is a unit the user chose, so partial application would show a state nobody saved.

**Switch-off.** When a feature is switched off, the current listing drops that feature's facets and sort through the same `ListingEvent::ViewApplied` path. A filter the user cannot clear behind a blocked control is therefore never left. Stored results stay and stay shown.

### 2.8 Counts

`FeatureCounts` (new, in `pg-catalog` beside `status()`, `lookup.rs:174`). It is not named `Counts`, which is `ledger::Counts` (`catalog/src/ledger/state.rs:64`, re-exported as `InboxCounts` at `app/src/reexport.rs:11`):

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeatureCounts { pub borders_unchecked: i64 }   // Phase 3 adds duplicates_unchecked

impl Catalog {
    pub fn feature_counts(&self) -> Result<FeatureCounts> {
        let snapshot = self.conn.unchecked_transaction()?;
        let borders_unchecked: i64 = snapshot.query_row(
            "SELECT count(*) FROM analysis a CROSS JOIN files f
             WHERE a.borders_due = 1 AND f.id = a.file_id AND f.missing = 0 AND f.pending = 0",
            [], |r| r.get(0))?;
        Ok(FeatureCounts { borders_unchecked })
    }
}
```

The predicate is the one `pending_borders` uses (`work.rs:92`), including `f.pending = 0`. The partial index `analysis_borders_due` (`schema.rs:588`) serves the first filter. No thumbnail count.

Carrier: `CountsCell(Arc<Mutex<FeatureCounts>>)` in `pg_library`, beside `HeightCell`, with `get()` and `set()`. `Inner` holds it; `LibraryService` holds a clone and exposes `pub fn feature_counts(&self) -> FeatureCounts` (a Mutex snapshot).

The job computes the counts at each round end, beside `self.changes.publish(Change::Round)` (`runner.rs:381`), on the round's connection: `match catalog.feature_counts() { Ok(c) => counts.set(c), Err(e) => a warning in the log, and the last values are kept }`, then `self.changes.publish(Change::Counts)`. A wake starts a round, so a `FeatureCell::set` is counted at the next round end. A failed count keeps the last values and posts no notice (it is display only). `Change::Counts` is payload-free, like the others (`jobs/src/events.rs:128-141`: the bus drops the oldest events, so payloads would be lossy).

Desktop: `crates/desktop/src/library.rs` `Changes` (`:41-50`) gains `counts: bool`, with an `add()` arm `Change::Counts => self.counts = true`. `on_pass` calls `refresh_counts()` when `taken.counts` is set. `refresh_counts` reads `app::get().library().feature_counts()` (a Mutex snapshot, so no SQL runs on the Qt thread; `desktop/src/library.rs` already reads job state there) and sets the qproperty. `library_bridge.rs` (`:18-33`): `#[qproperty(i32, borders_unchecked, cxx_name = "bordersUnchecked", READ, WRITE, NOTIFY, FINAL)]`. It is a qproperty pushed by the pump, not an invokable, which is how the pump already pushes state (`refresh_progress`, `refresh_writes`).

### 2.9 Round order, height changes, and Thumbs errors

> Superseded 2026-11-16 (commit aeb7d2c): the round is `Crops, Cuts, Scans, Import, Thumbs, Hashes, Pairing, Borders`. The three changes below are implemented as written, and the duplicate passes are now the Hashes pass and the Pairing pass, with borders last.

Round order: `Crops, Scans, Import, Thumbs, Borders` (Phase 3 adds `Dups` last). Three changes make this order safe:

- **A height change does not stop the round.** `HeightCell::set` (`runner.rs:64-76`) wakes the job and no longer stops it: the listener at `runner.rs:185-190` drops `request_stop` (`:187`) and keeps `service.wake()`. The wake is sticky (`crates/jobs/src/service.rs:65, :98-100`), so the job runs another round after the current one. Without this, every zoom step restarts the round from Import, and each height's full Thumbs pass gates Borders (`reader.rs:37` lists the whole catalog when no `checked` row exists for the height).
- **The Thumbs pass returns `Done` on a height change.** `pg_thumbs::run` takes one `&Control` (`crates/thumbs/src/lib.rs:173-179`), so the height is a second stop source, given as a closure: `pg_thumbs::run(..., still_wanted: &dyn Fn() -> bool, ...)`, with `still_wanted = || heights.get() == Some(height)` (the runner reads the height once at the pass's start, `runner.rs:556-571`). The check runs at each flush, before `put_batch` (`thumbs/src/lib.rs`, the flush before the empty check at `:269`). When it fails, the flush's thumbnails go back to the spare buffers, the pass returns incomplete, and the runner's step is `Done`, so Borders and Dups run in the same round. An incomplete pass writes no `checked` row (`thumbs/src/lib.rs:343-345`), so the next round lists the files at the new height. The pass posts no notice of its own; a notice from an earlier flush is posted as today. Thumbs never returns `Preempted`, and this change does not add it.
- **A Thumbs error does not end the round.** `pg_thumbs::run(...)?` (`runner.rs:562`) and `crates/jobs/src/schedule.rs:25` propagate errors, so a persistent `put_batch` error (`thumbs/src/lib.rs:274`) would end Borders and Dups on every round. The error is recorded in the pass's stats; the runner posts one `Notice::CacheNotStored` for the pass (existing, `notice.rs:422-452`); the pass stops producing thumbnails; and the round goes on:

```rust
let stored = match cache.put_batch(&mut thumbs, Some(thumb)) {
    Ok(stored) => stored,
    Err(e) => { stats.cache_error.get_or_insert_with(|| format!("{e:#}")); return Ok(false); }
};
// Then the existing bookkeeping of the flush (thumbs/src/lib.rs:276-292), unchanged, for `stored`.
```

A skipped Thumbs pass never reaches `ThumbCache::checked` (its callers are `thumbs/src/lib.rs:195` and `:344`), so re-enabling thumbnails relists everything missed.

When a feature is switched off, its pass returns `Done` before `ctl.begin`. The pass's notices are cleared and not run: the `files_read` keys with `Part::Code(pass.code())` (`runner.rs:320`) and the notices at `runner.rs:541-547`. So a notice posted before the switch does not stay up. Due state is left due, and stored results stay and stay shown.

### 2.10 Thread priorities

- `Priority::AboveNormal` is added to `pg_platform::priority::Priority` (`crates/platform/src/priority.rs:6-10`; today `Normal`, `BelowNormal`, `Idle`). Windows: `THREAD_PRIORITY_ABOVE_NORMAL` (value 1 in windows-sys 0.61, which the workspace pins in `[workspace.dependencies]`); the match at `:18-22` gains the arm. Linux: `AboveNormal` is a no-op, like `Normal`. Setting nice 0 would reset a thread that an earlier call had lowered, and a negative nice needs privilege. Other platforms: no-op (`:53-54`).

```rust
let nice = match priority {
    // Not raised: a negative nice needs CAP_SYS_NICE, and setting 0 would undo an earlier lowering.
    Priority::AboveNormal | Priority::Normal => return,
    Priority::BelowNormal => 5,
    Priority::Idle => 19,
};
```

Test, in `mod tests`: `above_normal_keeps_the_nice_value` (the nice value is the same before and after the call).

- The GUI thread: `priority::set_current_thread(Priority::AboveNormal)` in `crates/desktop/src/lib.rs`, immediately before `app.exec()` (`:208`). `pg-platform` is already a desktop dependency (`crate-layers.txt:48`).
- The render thread. The hook is per window, not inside the device branch. The existing `sceneGraphInitialized` connection (`desktop/cpp/gpu_device.cpp:157-160`) lives inside `if (gpu.device)` (`:150-161`) and is the device-loss handler, so it is not the hook. The new `raise_render_threads(QQmlApplicationEngine&)` (in `gpu_device.cpp`, declared in `gpu_device.h`) connects `QQmlApplicationEngine::objectCreated` (`qqmlapplicationengine.h:39`). For each window it connects `sceneGraphInitialized` with `Qt::DirectConnection`. The signal is emitted from `QSGRenderContext::initialized` (`qquickwindow.cpp:840`) on the render thread (`:1015-1016`). The slot calls the Rust `raise_current_thread()` (in `desktop/src/gpu_device.rs`, exposed through `gpu_device_bridge.rs`, the way `screenshot_bridge.rs` exposes `screenshot.rs`). `desktop/src/lib.rs` calls it before `engine.load` (`:189`).

```cpp
void raise_render_threads(QQmlApplicationEngine& engine) {
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreated, &engine, [](QObject* root, const QUrl&) {
        auto* window = qobject_cast<QQuickWindow*>(root);
        if (!window) return;
        // DirectConnection: the slot runs where the scene graph initializes. Not single-shot: the signal comes again
        // after a scene graph is released, on the same thread.
        QObject::connect(window, &QQuickWindow::sceneGraphInitialized, window,
                         [] { raise_current_thread(); }, Qt::DirectConnection);
    });
}
```

- Qualified: Qt's threaded render loop (`qsgrenderloop.cpp:189-190`) is used for every non-OpenGLES2 RHI backend. With `QRhi::Null`, with `QML_NO_THREADED_RENDERER`, or with `QSG_RENDER_LOOP=basic`, the basic loop runs instead (`qsgrenderloop.cpp`, about lines 199-201 and 219-233). The slot then runs on the GUI thread, which is already AboveNormal, so the hook is harmless there. `sceneGraphInitialized` is emitted on the rendering thread (`quick/items/qquickwindow.cpp:1015-1016`) and is connected as `DirectConnection` (line 840), so the raise runs on whichever thread renders. A Phase 6 manual check confirms the render thread in each configuration.

### 2.11 Strings

New keys in `crates/client/locales/en/ui.ftl`:

```text
ui-borders-disabled = Border detection is off, so this does nothing. Turn it on in the Library panel.
ui-thumbnails-disabled = Pre-generated thumbnails are off, so this does nothing. Turn them on in the Library panel.
ui-nothing-to-crop = No borders to crop in this view.
ui-no-border-here = This file has no detected border.
ui-borders-not-checked = Borders are not checked yet for this file.
ui-start-option-disabled = Left out, because { $text } is off. Turn it on in the Library panel.
ui-feature-borders = border detection
ui-feature-thumbnails = pre-generated thumbnails
ui-detect-borders-count = Detect borders ({ $text } not yet checked)
ui-pregenerate-thumbnails = Pre-generate thumbnails
```

Phase 3 adds `ui-duplicates-disabled`, `ui-dupl-not-checked`, `ui-feature-duplicates`, `ui-detect-duplicates-count` and `ui-detect-duplicates`. The parameterized strings are read with `words.phrase(id, 0, text, "")`, as the crop button does (`FilterBar.qml:253`). The id test in `client/src/text/words.rs` covers each id. The English wording is approved, and the joined feature list uses a locale-aware separator. No Notice variant is added: the disabled-feature messages are popups.

### 2.12 Tests

- Popup intercepts: each row of the table in 2.6 pops up and performs no action. The QML tests live in `crates/desktop/tests/qml/`. The stubs need the members the tests use: `stubs/pg/gallery/Shell.qml` (`:8`, which has `viewerStep`) gets `function featureOn(need) { return true }` and `signal featureRefused(int needs)`, and `stubs/pg/gallery/Settings.qml` (`:4-5`, which has `realSize`) gets `writeBack`, `detectBorders` and `pregenerateThumbnails`, all default true. Each stub value can be set by a test.
- The Rust intercepts: `grid_refusal` for `Grid::Borders` and `Grid::MarkBorder`, with the feature off and on, in `client/src/features.rs` tests.
- The combo revert: a rejected pick leaves the combo on its bound value (QML test).
- `view_refusal` returns the disabled needs. A view that needs one is not applied; a view with none is applied as saved. Switching a feature off clears its facets and sort from the current listing.
- Toggle off: the pass is skipped, its due state is kept, and its notices clear. Toggle on: the next round catches up.
- A height change during Thumbs returns `Done`, and the round reaches Borders.
- A `put_batch` error posts `CacheNotStored` once and the round continues. The seam is a real failing store (a read-only cache file, or a `cfg(test)` failing store), because `ThumbCache::disabled()` returns `Ok(false)` (`thumbcache/src/lib.rs:199-201`) and never reaches the `Err` path.
- `checked` is unchanged with thumbnails off. The seam is a real `ThumbCache` (the runner tests' `thumbs(...)` helper, around `runner.rs:629`). `ThumbCache::disabled()` makes `checked` a no-op (`thumbcache/src/lib.rs:280-283`), so a test built on it cannot fail.
- `feature_counts` excludes missing and pending files, and is 0 when `borders_due` is 0.
- The prefs golden: the golden text at `prefs.rs:165-173` gains `"detect_borders": true` and `"pregenerate_thumbnails": true` after `"dev_mode": false` (the field order of the file). The changed-value test (`:195-207`) expects both keys. The old-file test `a_float_row_height_from_an_old_file_reads_rounded_and_clamped` (`:185-193`) and the old-folder test (`:257-261`) still pass.
- `FeatureCell::set` before start and after start both wake the job, and the next round reads the new value. The listener test sits beside the heights test (`runner.rs:596-607`).
- The viewer N guard is removed (`keys.rs:138`, decided 2026-11-13). The `keys.rs` test at `:303-305` changes to expect `MarkBorder` for a still.

## Phase 3: duplicate hashes on the full decode, and the duplicate toggle (a migration)

> Superseded in part 2026-11-17 (commit 5fcff8bb): the duplicate hashes are made by the Hashes pass, not stored at import, and the ThumbHash moved to the Placeholders pass. The definitions in 3.1 are unchanged; the import-time parts of 3.4 to 3.7 are not what the code does now.
>
> Superseded 2026-11-16 (commit aeb7d2c): the duplicate catch-up is the Hashes pass over `hashes_owed` (migration 15, `TO_15_HASHES_OWED`) and the pairing pass over `pairs_owed`; there is no Dups pass.

### 3.1 The hash definition (decided by the user)

- `pixel_hash` = xxh3 of the upright full-size pixels (`analyze.rs:177-183`). The PDQ is fitted from that image (`fit_within(img, 512)`, `resize.rs:25-35`), and the dihedral hashes come from `hash_rgb_dihedral`. ThumbHash is fitted from it to `THUMBHASH_SIDE` = 100 (`analyze.rs:36, :187-192`). ThumbHash no longer fits a 512 px intermediate, so it changes only where the decode changes.
- Every new file gets **one full stored decode F in every mode**: `Analyzer::analyze_image` uses `decode_stored_into(MinSize::Full)` (or `Full`, oriented in place by `orient_into`), not `Sides(512, 320)`. The hash and ThumbHash definitions are then the same whatever the toggles are. This is the reason the decode is F in every mode (Phase 5 relies on it).
- Hashes of PNG, WebP, GIF, BMP and TIFF are unchanged, and so are JPEGs with denom 1 (the full decode is the same pixels). JPEGs with denom > 1 change.
- One function computes the hashes from an upright full-size image, and the import and the Dups pass both call it. It lives in `pg-analysis` (`analyze.rs`), with `Info::upright()` (`codec/src/decode.rs:34`) made `pub`:

```rust
/// The duplicate hashes of an upright full-size image (the import and the Dups pass both call this).
pub struct Hashes { pub pixel: [u8; 16], pub quality: u8, pub dihedral: [pdq::Hash; 8] }   // pdq == dihedral[0]
pub fn hashes(upright: &Rgb) -> Hashes;      // PDQ fitted to 512, the pixel hash, the eight dihedral hashes
pub fn thumbhash(upright: &Rgb) -> Vec<u8>;  // fitted to THUMBHASH_SIDE
```

- **The hashes are computed in every mode.** The duplicates toggle gates the pairing (3.6, 3.9), not the hashing. This is the consequence that makes the stored hash values identical in all toggle combinations (decided 2026-11-13, option (a); the rejected alternative is below).
- **Measured 2026-11-13** (one thread, release, 200 sampled files from `D:\wallhaven\downloads`: 122 JPEG at 9.2 MP mean, 78 PNG at 6.8 MP). For a JPEG, the hashes on the full image cost 41 ms against 4 ms on the scaled image: the pixel hash 3 ms, PDQ 16 ms and ThumbHash 22 ms. The pixel hash and PDQ, the part the duplicates toggle could skip, are about 18 ms of a 135 ms full decode. ThumbHash runs either way, because it is always stored. For a PNG the hashes are the same on both paths, since PNG is not scaled on decode. The resizes dominate the cost, not the pixel hash.
- **Why always compute (decided 2026-11-13, option (a)).** Gating the hashing with the duplicates toggle (the rejected option) saves about 24 minutes of CPU on the 85,073-file library, for users who never enable duplicates. Those users would pay for it later: enabling duplicates would then need one more full decode pass (about 2.6 hours of CPU for the library, from the measured full decodes of about 135 ms per JPEG and 62 to 65 ms per PNG, 5.1), plus a re-read of the library (about 306 GiB). Stored hashes would also be absent in one toggle state, so the 5.12 and 6.6 checks could compare only where both sides hold a value. Always computing keeps the stored values identical in every toggle combination, and enabling duplicates is a pairing job.

### 3.2 The 512 MiB JPEG cap lands here

The cap is in `codecs/limits.rs:8` (`max_decode_bytes()`, 512 MiB, confirmed by the user) and is enforced today only by the PNG and TIFF decoders (`png.rs:20`, `tiff.rs:221`). `decode_jpeg` (`codec/src/decode.rs:141-150`) has no check, so an over-cap JPEG decodes in full today. The check is **new code** in `decode_jpeg`:

- It reads the stored size from the header alone: `Decoder::dimensions(bytes, format) -> Result<(u32, u32)>` (the JPEG header is read at `decode.rs:148`).
- It compares the full allocation with the cap before allocating (every decode is full size, so there is no scaled allocation). The figure is the one the codec reserves: `w * h * 3` for JPEG (the colour-type figure in `codecs/limits.rs:14-17` is for PNG and TIFF).
- It returns an outcome, not an `Err`: `DecodeOutcome::OverCap { width, height }` (5.2). `decode_stored_into` swallows a decode error (`decode.rs:124`, `.ok()`), and the codecs fallback would then fail the whole file.

PNG and TIFF keep their decoder caps as errors, as today. The over-cap rule applies to JPEG only. Phase 3 is where the first full decode lands, so the over-cap rule (5.2) lands here too.

### 3.3 Migration TO_10

`TO_10` was the next free number when this section was written (against schema 9: `TO_2` to `TO_9` existed, and migration N takes the version from N-1 to N, so TO_10 takes 9 to 10). The current build's schema is version 1 with no migrations (`MIGRATIONS` is empty; see `crates/catalog/src/schema.rs`), so the next migration is `TO_2` (1 to 2) and the number here must be renumbered when it is written. The steps below are unchanged. It is one `MIGRATIONS` entry after `&[Step::Sql(TO_9_CHEAPER_WRITES)],` (the line numbers were `schema.rs:44`, `:365` when written):

```sql
ALTER TABLE analysis ADD COLUMN dups_state TEXT NOT NULL DEFAULT 'done'
    CHECK (dups_state IN ('due', 'done', 'skipped'));
CREATE INDEX analysis_dups_due ON analysis(file_id) WHERE dups_state = 'due';
INSERT INTO meta (key, value) VALUES ('dups_version', 1);
-- The io read attempts of a file in a pass (5.6). Cleared by the trigger when the file changes, and by the
-- pass whenever it stores a result for the file, or writes the terminal failure at the third attempt.
CREATE TABLE io_attempts(
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass        TEXT    NOT NULL,           -- 'thumbs' | 'placeholders' | 'borders' | 'import' | 'duplicates'
    attempts    INTEGER NOT NULL,
    next_try_at INTEGER NOT NULL DEFAULT 0,  -- as IMPORT_FAILURES_PLAN 5.4 E: the worklists skip rows with next_try_at > unixepoch()
    PRIMARY KEY(file_id, pass)
);
CREATE INDEX io_attempts_due ON io_attempts(pass, next_try_at);
CREATE TRIGGER io_attempts_stale AFTER UPDATE OF mtime, size, rotation, missing ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation
    OR old.missing IS NOT new.missing
BEGIN
    DELETE FROM io_attempts WHERE file_id = new.id;
END;
```

- `dups_state` has three values, and its meaning is (one column, because a boolean cannot say both the pairs-owed and the not-checked states):
  - `'due'`: the duplicate status is owed. Either the **pairs** are owed (the hashes are stored, but duplicate detection was off when the file was imported), or the **hashes** are owed too (`pixel_hash` is NULL, after a version bump). The Dups pass handles both (3.7).
  - `'done'`: the hashes are stored and the pairs are current; or there is nothing to hash (a video, a non-image, or an analysis error).
  - `'skipped'`: never retried and not checked. Set for an over-cap JPEG (5.2) and for a decode failure in the Dups pass (3.7).
- Existing rows are `'done'` (the default), so the migration re-hashes nothing by itself. The version bump (3.4) does.
- `dups_version` is seeded as `1` (the old definition's version), so `set_dups_version` (3.4) sees a key that exists, as `borders_version` does. The `meta` table takes `value ANY`, so the seed is an integer.
- The partial index is used by a query that repeats `a.dups_state = 'due'` literally, as `live_failures!` does.
- `expected()` (the baseline plus the migrations) must match a fresh schema. `expected_schemas_follow_the_version` (`schema.rs:851-865`) checks that for every migration, so TO_10 needs no new test code.
- `Versions` gains a `dups` field (`types.rs:224-228`). The struct-literal sites are counted at the start of Phase 3 (grep `Versions {`: 19 matches in 12 files, some of them declarations).
- README note: an older build opening a migrated catalog moves it aside as `catalog.sqlite.replaced` and rebuilds, losing the pairing state.

### 3.4 The version bump: `DUPS_VERSION` = 2

`pub const DUPS_VERSION: i64 = 2;` goes beside `IMPORT_VERSION` (`analyze.rs:27`). The catalog already depends on `pg-analysis` (its `pg-analysis` dev-dependency in `crates/catalog/Cargo.toml`). `set_dups_version(conn, current)` goes in `lifecycle.rs` beside `set_borders_version` (`:137`). It runs from `Catalog::prepare` after `set_borders_version` (`core.rs:117`) through `set_mark`, in one transaction:

```sql
UPDATE analysis
   SET pixel_hash = NULL, pdq = NULL, pdq_quality = NULL, pdq_dihedral = NULL, thumbhash = NULL,
       dups_state = 'due'
 WHERE kind = 'image' AND format = 'jpeg' AND error IS NULL AND dups_state <> 'skipped';
DELETE FROM pdq_pairs
 WHERE a IN (SELECT file_id FROM analysis WHERE dups_state = 'due')
    OR b IN (SELECT file_id FROM analysis WHERE dups_state = 'due');
```

- It resets **every** JPEG row, denom-1 ones included, because the row does not record its decode scale. Their hashes come back equal.
- It also resets **ThumbHash**. ThumbHash comes from the same full decode as the hashes, so after the bump the Dups pass rewrites hash and ThumbHash together (3.7). Without the reset, an existing row keeps the old ThumbHash and the single-definition rule fails. The grid draws the ThumbHash as its placeholder (`GridCell.qml`, `GridRow.qml`), so these rows have no placeholder until the pass runs in the following rounds. That is a transient, and it is stated here.
- It deletes the pairs of every reset row, **including pairs to non-JPEG partners**, since a partner's pair with a JPEG is derived from the JPEG's hash. The Dups pass pairs the re-hashed JPEGs against the non-JPEG rows again. Non-JPEG rows keep their hashes.
- It does not touch `file_hash`, so exact-duplicate detection is unaffected.
- `view_epoch` is not touched in Phase 3. Phase 4 adds `bump_view_epoch` to this function (4.5), so an open grid reloads once instead of stamping every JPEG row.
- `format = 'jpeg'` is verified: `Format::Jpeg` serialises as `jpeg` (strum snake_case, `model/src/format.rs:4-9, 33-40`), and `analysis.format` is written from `Analysis::format()` (`analysis.rs:114-119`) through `store_analysis` (`writers.rs:320`).
- Consequence: the first open of an upgraded catalog re-decodes every JPEG once, in the Dups pass, whatever the duplicate toggle says. The hash work always runs; only the pairing waits for the toggle (accepted 2026-11-13). Phase 6 times it. The bump's own writes fire the Phase 4 triggers, and the bump's duration is measured in Phase 6 against `busy_timeout` (5000 ms, `schema.rs:377-378`).
- The bump also resets `thumbhash`. Without that, a row keeps its old ThumbHash after the bump, and the single-definition rule (3.1) fails. The Dups pass recomputes the hashes and ThumbHash from the same F decode.

### 3.5 Writers

`store_analysis` becomes an upsert in Phase 3, because it is the statement that writes `dups_state`. The old `INSERT OR REPLACE` leaves `dups_state` at its default (`'done'`) and would reset `cut_*` only by accident. Phase 4's triggers depend on the upsert (4.4), and in Phase 3 it fires no stamping triggers, so the change is safe there.

```rust
/// The duplicate state of a row, as the sink knows it (writers.rs).
pub enum DupsState<'a> {
    /// Hashes stored and pairs made: 'done'.
    Done(&'a Hashes),
    /// Hashes stored, pairs owed: 'due' (duplicate detection was off).
    Owed(&'a Hashes),
    /// Over the cap, or a decode failure in the Dups pass: 'skipped'.
    Skipped,
    /// Nothing to hash (a video, a non-image, or an analysis error): 'done'.
    Nothing,
}
```

- `store_analysis(item, analyzed, dups: DupsState<'_>)` (`writers.rs:300-340`) is one upsert naming all 29 columns (the 28 of the baseline plus `dups_state`). The old `INSERT OR REPLACE` named 24 (`writers.rs:256-258`), and the four `cut_*` columns were reset only because they were left out. The upsert names them and sets `cut_left..cut_bottom` to NULL explicitly, as the design needs:

```sql
INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
    file_hash, pixel_hash, format, orientation, width, height, pdq, pdq_quality, thumbhash, frames,
    taken, taken_utc, duration_ms, poster_ms, borders_due, pdq_dihedral,
    cut_left, cut_top, cut_right, cut_bottom, dups_state)
VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL,
    ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
    CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END,
    CASE WHEN ?16 THEN ?15 END,
    ?17, ?18, 0, ?21,
    NULL, NULL, NULL, NULL, ?22)
ON CONFLICT(file_id) DO UPDATE SET
    version = excluded.version, size = excluded.size, mtime = excluded.mtime, rotation = excluded.rotation,
    kind = excluded.kind, analyzed = excluded.analyzed, error = excluded.error,
    file_hash = excluded.file_hash, pixel_hash = excluded.pixel_hash, format = excluded.format,
    orientation = excluded.orientation, width = excluded.width, height = excluded.height,
    pdq = excluded.pdq, pdq_quality = excluded.pdq_quality, thumbhash = excluded.thumbhash,
    frames = excluded.frames, taken = excluded.taken, taken_utc = excluded.taken_utc,
    duration_ms = excluded.duration_ms, poster_ms = excluded.poster_ms, borders_due = excluded.borders_due,
    pdq_dihedral = excluded.pdq_dihedral,
    cut_left = NULL, cut_top = NULL, cut_right = NULL, cut_bottom = NULL, dups_state = excluded.dups_state;
```

The bound values follow the existing `store_analysis` parameters (`writers.rs:256-261`). For `Skipped` and `Nothing`, the hash parameters are NULL. The statement still deletes the file's pairs (`writers.rs:254, :306`).

- `store_failure(item, error)` (`writers.rs:343-359`) becomes an upsert too. An `INSERT OR REPLACE` would skip the delete triggers (4.4). It NULLs every analysis column but the key and the error, as REPLACE left it, sets `borders_due = 0`, and sets `dups_state = 'done'` (`Nothing`). The file leaves `'due'` with its failure. This is what 3.7 needs: a failed analysis is not a duplicate-status question.
- `store_hashes(item, hashes, thumbhash, owed: bool) -> Result<bool>` (new), used by the Dups pass for a row with no hash. It is guarded like `store_analysis` (`current` first, then `not_being_written!()`) and writes the hashes and the ThumbHash only into a row that is still due and has no hash:

```sql
UPDATE analysis
   SET pixel_hash = ?2, pdq = ?3, pdq_quality = ?4, pdq_dihedral = ?5, thumbhash = ?6,
       dups_state = CASE WHEN ?7 THEN 'due' ELSE 'done' END
 WHERE file_id = ?1 AND dups_state = 'due' AND pixel_hash IS NULL
   AND NOT EXISTS (SELECT 1 FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL);
```

It returns `Ok(rows == 1)`. `owed` is `true` when duplicate detection is off.

- `store_pairs_done(item) -> Result<bool>` (new): sets `dups_state = 'done'` on a row that is `'due'` and has a hash, after its pairs are inserted (the same guard). It is used by the pairs-only phase (3.7) and by the hash phase when detection is on.
- `store_skipped(item) -> Result<bool>` (new): `UPDATE analysis SET dups_state = 'skipped' WHERE file_id = ?1 AND dups_state = 'due' AND NOT EXISTS (SELECT 1 FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL)` (the same guard).
- `store_pdq_pair` is unchanged (`writers.rs:361-365`). Its `ON CONFLICT ... WHERE excluded.distance < distance` upsert keeps each pair once, and the Phase 4 triggers handle an UPDATE of a pair's distance (4.4).
- `store_borders` returns `Result<bool>` instead of `Result<()>` (`writers.rs:419-422`). It returns `false` for a file that changed under it, where it returns silently today; the sink needs to know, and `borders/src/lib.rs:100` counts the change as stored.

### 3.6 The pairing lifecycle

- A `Pairer` (`dedup/src/pairer.rs`) is built once per pass run from `Catalog::pdq_hashes()` (`work.rs:103`), over the rows whose `dups_state = 'done'` and the existing filter (`pdq_dihedral IS NOT NULL AND pdq_quality >= 50`). Files that get their pairs later in the same run are added to the index by `pair_and_insert`, so a pair between two files of one run is found once.
- **With duplicate detection off, nothing pairs.** Import does not build a Pairer and does not load `pdq_hashes()`. The Dups pass does not pair. The hashes are still computed (3.1), and the rows stay `'due'` (the pairs are owed).
- Import builds its Pairer per import call (`import/src/lib.rs:41-42`, local to `import()`), as today, when detection is on. The Dups pass builds one per Dups pass run, when detection is on.
- `Pairer::reserve` is sized from the due count (`items.len() * 8`, `pairer.rs:35-37`).
- Memory: 4 PDQ orientations of 32 bytes per file (128 bytes, `pdq.rs` `BLOB_BYTES`), about 128 MB at a million files. That is recorded in the bounded-memory plan's terms, not solved here.

### 3.7 The Dups pass and the decode budget

- **Location: a module of `pg-import`**, `crates/import/src/duplicates.rs`, exposed as `pg_import::duplicates`. The Dups pass shares the import's analyzer, pipeline, sink and budget, and `pg-import` already has every edge it needs (`analysis`, `catalog`, `dedup`, `jobs`, `model`, `platform`). So no new crate, no new layer line and no manifest change. The name `pg-duplicates` is dropped.
- **Names.** The pass is `Duplicates` in code, as IMPORT_FAILURES_PLAN.md (D-C) decides: `Phase::Duplicates` in `model/src/progress.rs`, the words `pass-duplicates` in `text.ftl`, read through `pg_client::text::pass_name`, and `Message::PassDuplicates` in `client/src/text/message.rs`. Its ledger code is `FilePass::Duplicates` (`model/src/notice.rs:281-288`). The prose "Dups pass" elsewhere in this plan means this pass. `file_failures.pass` has no `CHECK` (`schema.rs:114` is only a comment, and it is stale: it lists 'thumbs' and 'borders' alone), so the duplicates pass needs only its Rust code. The old migration's comment is left as it is.
- **Round order:** `Crops, Scans, Import, Thumbs, Borders, Dups`. Dups runs last and is reached on every round, because Thumbs no longer stops a round (2.9). Priority `BelowNormal` (5.10). Superseded 2026-11-16 (commit aeb7d2c): the order is `Crops, Cuts, Scans, Import, Thumbs, Hashes, Pairing, Borders`; Hashes and Pairing are skipped while duplicates detection is off, and Borders while border detection is off.

**Worklists** (`crates/catalog/src/work.rs`, beside `pending_borders`, `:84-99`). Both are over `analysis a JOIN files f` with `a.dups_state = 'due' AND f.missing = 0 AND f.pending = 0`, in the order of the other passes (`ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF`):

- `pending_pairs()`: the rows with `a.pixel_hash IS NOT NULL`. Their stored `pdq_dihedral` comes with the row.
- `pending_hashes()`: the rows with `a.pixel_hash IS NULL`.

The literal `a.dups_state = 'due'` lets SQLite use the partial index `analysis_dups_due` (3.3).

**Pairs only (detection on).** For each batch of 100 `pending_pairs` rows, one transaction: `pair_and_insert` from the stored hashes, `store_pdq_pair` for each neighbour, then `store_pairs_done`. No file is read. This completes the pairing of the files imported while detection was off.

**Hashes (both toggles).** `pending_hashes` runs through `pipeline::batched` (`pipeline.rs:240-270`, as the other per-file passes): the bytes are read whole; the worker reads the header dimensions, checks the cap (3.2), reserves the budget (below), decodes F (stored orientation, `MinSize::Full`), and computes `analysis::hashes` and `analysis::thumbhash` on the upright image. The budget is released at the end of `process`. The sink (5.5) writes each batch in one transaction, with `Batch { size: 100, every: Some(250 ms) }`:

| Outcome | Writes |
|---|---|
| Hashed | `store_hashes(item, &hashes, &thumbhash, owed)`, `owed = !detection_on`. If detection is on: `pair_and_insert`, `store_pdq_pair` for each neighbour, then `store_pairs_done`. |
| Over the cap | `store_skipped`. |
| Decode error or panic | `store_skipped`, and a ledger row (`FilePass::Duplicates`, `retry = false`, the error text). **Not** `store_failure`: that would erase the analysis of a file that imported fine. |
| io read error | The common rule (5.6): the attempt is counted, and the file stays `'due'` for the next round until the third attempt, which writes an error row (`--retry-errors` or a change of the file clears it). Today the Borders catch-up retries without end (`borders/src/lib.rs:79-83`); this changes that. The count is in `io_attempts` (3.3). |
| Not run (stopped in the budget wait) | nothing |

`stats.complete = stats.files == items.len()`, as the other passes.

**The decode budget** (`crates/jobs/src/budget.rs`, `pg-jobs`, introduced in Phase 3 because the Dups pass decodes; Phase 5's import uses the same budget, 5.3). One budget per pass run, since passes never overlap. A reservation is `w * h * 3` (3.2) before the decode, and it is released when the file's `process` ends, on every path (Ok, Err, or a panic caught by `panics::guard`). Grants are first come, first served (a ticket queue), so a large file waiting for room is not starved by smaller ones behind it. The release is at the end of `process`, not when the outcome is sent: `Outcome` holds no pixels, so holding the budget through the channel would only lower parallelism.

```rust
pub struct DecodeBudget { limit: u64, queue: Mutex<Queue>, turn: Condvar }
#[derive(Default)]
struct Queue { in_use: u64, next: u64, serving: u64 }

/// Released on drop, so an error or a caught panic releases it too.
#[must_use]
pub struct Reservation<'a> { budget: &'a DecodeBudget, bytes: u64 }

impl DecodeBudget {
    /// 1.5 GiB in use at once.
    pub const DEFAULT: u64 = 1536 << 20;
    pub fn new(limit: u64) -> DecodeBudget;

    /// Waits for its turn and for room: `bytes` fit beside what is reserved, or nothing is reserved (a file over the
    /// limit runs alone, so the budget cannot deadlock). `None` when `ctl` stops first; the caller returns "not run".
    pub fn reserve(&self, bytes: u64, ctl: &Control) -> Option<Reservation<'_>> {
        let mut q = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = q.next;
        q.next += 1;
        loop {
            if q.serving == ticket && (q.in_use == 0 || q.in_use.saturating_add(bytes) <= self.limit) {
                q.serving += 1;
                q.in_use += bytes;
                self.turn.notify_all();
                return Some(Reservation { budget: self, bytes });
            }
            // A stop ends every waiter, so a ticket that is never served is not a leak.
            if ctl.stop_requested() { return None; }
            q = self.turn.wait_timeout(q, Duration::from_millis(200)).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut q = self.budget.queue.lock().unwrap_or_else(PoisonError::into_inner);
        q.in_use -= self.bytes;
        self.budget.turn.notify_all();
    }
}
```

Tests: a second reservation blocks until the first is released; a file over the limit runs alone; a large waiting reservation is granted before a later small one; the budget is released on an error and on a panic.

### 3.8 Listings and the facet

The duplicates facet has four values, because a file that is not checked yet is neither a duplicate nor unique:

- **Any** (no filter).
- **Duplicates**: `kind = 'image' AND dups_state = 'done' AND error IS NULL AND <a mate exists>` (the existing `duplicates!` macro, `query/src/lib.rs:332-346`, with the same mates).
- **No duplicates**: `dups_state = 'done' AND error IS NULL AND NOT <a mate exists>` (`NOT_DUPLICATES`, `query/src/lib.rs:350`).
- **Not checked** (`ui-dupl-not-checked`): `dups_state <> 'done' AND error IS NULL`, which covers `'due'` and `'skipped'`. So a file whose pairs are owed (duplicate detection off) is in Not checked, not in No duplicates.

### 3.9 Controls

- A Duplicates checkbox (`detectDuplicates`, the pref `detect_duplicates` in `prefs.rs`, the setter of 2.3), with its count (`duplicates_unchecked`, 2.8: the rows with `dups_state = 'due'`, `error IS NULL`, on present files), and the gate of 2.4 with `Need::Duplicates` (code 2).
- The duplicates combo (`FilterBar.qml:217-223`) follows the combo rule of 2.6. It is gated in Phase 3, with the toggle.
- `Features` gains `duplicates` (the pref's `Prefs::features()`, 2.3). `ImportOptions` (`import/src/lib.rs:32`, which takes an options struct today) carries the features and the thumbnail height (5.11). `Features` stays bool-only: a toggle is a bool and the height is a size, so each keeps its own type. About 23 call sites change (the runner, the CLI, the `thumbs` test, `library/tests/*`, and `writeback.rs` with 9); the count is taken at the start of the phase.
- With detection off, the import stores the hashes and `'due'`, and builds no Pairer (3.6). Turning detection on makes the next Dups pass pair the owed rows (pairs only: no file is read, and the pairs come from the stored PDQ hashes).

**Ledger codes.** The inbox's pass code is the position in `LedgerKind::ALL` (`crates/desktop/src/ledgers.rs:9`, `crates/model/src/notice.rs:310-315`): Thumbs 0, Borders 1, Import 2, ScanFolders 3. The Retry action is on the Thumbs ledger only (`notice.rs:1163`). Phase 3 adds `FilePass::Duplicates`, and `InboxPanel.qml:461` compares `selectedPass === 3` for the scan folders, so a new ledger ahead of ScanFolders would silently break that check. The Duplicates ledger is therefore added after ScanFolders (code 4), so no existing code moves, and the QML takes the scan-folders code from Rust (a bound property), not a literal 3.

### 3.10 Wake on another process's bump

A `DUPS_VERSION` bump run by another process's `prepare` (`catalog/tests/other_process.rs`) sets `'due'` and wakes no job. The job waits with `waker.wait(None)` (`runner.rs:359-361`), and only the busy retry (`BUSY_RETRY`, 5 s, while a lease is held) times the wait. The fix is an idle re-check on a timer: every 60 s while idle, the job runs the `feature_counts` query (2.8, extended with the duplicates count), and a non-zero count starts a round. The interval is a parameter, so a test can set it low.

### 3.11 Tests

- **Goldens.** The literal `pixel_hash`, PDQ, quality, dihedral and ThumbHash values for four fixtures (a PNG, a WebP, a denom-1 JPEG and a denom-2 JPEG, committed and small) are **captured in Phase 6** (Phase 6, step 3) from a worktree at the commit before Phase 3 (the pre-change build), and written into these tests in the closing commit. The rule: PNG, WebP and denom-1 JPEG must equal the old build's values; the denom-2 JPEG must differ (the expected change), and its new literal comes from the new build. No build runs between phases. There are no literal hash assertions in the tree today, so `Analyzer::hashes == analyze` would compare a function with itself, and it is not used.
- Duplicates off: the import stores `'due'` and the hashes, and no pairs. Duplicates on: the Dups pass pairs. Turning detection on pairs the owed rows from their stored hashes, with **zero decode calls** (a call counter on `Decoder`, `cfg(test)`, per instance).
- A version bump re-arms JPEG rows only (not PNG), resets the hashes and ThumbHash of the JPEG rows, clears the pairs of every reset row, and leaves `'skipped'` rows alone.
  - Superseded 2026-11-17 (commit 5fcff8bb): a version bump no longer resets a ThumbHash, which lives in `placeholders` and is keyed to `PLACEHOLDER_VERSION`; the rest of this bullet stands for the hashes.
- The invariant: `pixel_hash`, `pdq`, `pdq_quality` and `pdq_dihedral` are NULL together; every `'done'` row with a hash has all four and a ThumbHash.
- The three listings partition the image rows: every image row with `error IS NULL` is in exactly one of Duplicates, No duplicates, Not checked.
- The over-cap JPEG (a synthetic header above the cap, no pixel allocation) is `'skipped'`, listed as Not checked, and makes zero decode calls. The test asserts that the header read needs no scan data, so the synthetic header (no scan data) is enough.
- A Dups decode error leaves the analysis row intact (no `store_failure`) and sets `'skipped'` with a ledger row.
- `pdq_hashes()` skips `'due'` rows.
- The migration test (`expected()` against a fresh schema, `schema.rs:851-865`), and the budget tests (3.7).

## Phase 4: incremental grid updates

### 4.1 Why not `changed > since` alone

The rows below are what the existing stamps cannot say. Each has a remedy in 4.2 to 4.6.

- **Deletes.** A removed file's row is gone, so no stamp on it can be read. The scan's removals delete `files` rows (`catalog/src/pair.rs:345, 434`); `remove_root` sets `files.missing = 1` instead (`catalog/src/roots.rs:58-70`), which *is* stamped (`files_source_changed`).
- **Duplicate status.** A file's status depends on other files: a file that becomes a hash mate or a pair partner flips the status of its mates, which stamps nothing of theirs.
- **Bulk rewrites.** `set_taken_zone`'s recompute, `set_import_version` and the version bumps rewrite many rows at once and stamp only what they touch, so they bump `view_epoch` instead (4.5).

What the existing triggers already cover, verified: `files.changed` is stamped by `TO_2_CHANGE_STAMPS` (`schema.rs:55-66`) for size, mtime, rotation, kind and missing, and by its `analysis_cut_stamp` for the cut columns, so border results are already stamped. `files.seq` is not touched by border writes; it is the thumbnail-source stamp, and a border result does not change the thumbnail.

### 4.2 Stamps and the change number

- `meta.seq` is the global change number. `next_seq!()` is `UPDATE meta SET value = value + 1 WHERE key = 'seq'` and `seq!()` is `(SELECT value FROM meta WHERE key = 'seq')` (`crates/catalog/src/sql.rs:17-21, 63-67`). `files.changed` is `meta.seq` at a row's last user-visible change (`schema.rs:523-524`), indexed by `files_changed` (`schema.rs:543`). The triggers set `files.changed`, not `files.seq`.
- A stamp is `Stamp { seq: i64, epoch: i64 }` (`Copy`): the change number and `view_epoch`, read in the same transaction as the data.

### 4.3 Migration TO_11 (Phase 4's schema)

`TO_11` is one const after `TO_10`, with one `MIGRATIONS` entry. Its statements, in order:

```sql
-- one row per removal: what a stamp cannot say. TRANSIENT (4.5): a rebuild does not carry it.
CREATE TABLE files_log (seq INTEGER PRIMARY KEY, id INTEGER NOT NULL);
CREATE TRIGGER files_removed AFTER DELETE ON files BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    INSERT INTO files_log (seq, id) VALUES ((SELECT value FROM meta WHERE key = 'seq'), old.id);
END;

-- renames and moves: the row stays, so a stamp is enough (the client finds the old place by id)
CREATE TRIGGER files_renamed AFTER UPDATE OF folder_id, name, ext, created, order_key ON files
WHEN old.folder_id IS NOT new.folder_id OR old.name IS NOT new.name OR old.ext IS NOT new.ext
  OR old.created IS NOT new.created OR old.order_key IS NOT new.order_key
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.id;
END;

-- the import ledger on the upsert's UPDATE path. TO_9's analysis_import is AFTER INSERT only, so the upsert's
-- conflict path did not reach it; analysis_import_rewritten (TO_8) would run first and hide the change, so it goes.
DROP TRIGGER analysis_import_rewritten;
CREATE TRIGGER analysis_import_changed AFTER UPDATE OF error, size, mtime, rotation ON analysis
WHEN (new.error IS NOT NULL OR EXISTS (SELECT 1 FROM file_failures WHERE file_id = new.file_id AND pass = 'import'))
 AND (old.error IS NOT new.error OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
      OR old.rotation IS NOT new.rotation)
BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (new.error IS NULL OR mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed, f.missing
        FROM files f WHERE new.error IS NOT NULL AND f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error
        WHERE file_failures.error IS NOT excluded.error;
END;

-- a whole-row stamp for any analysis UPDATE. `analyzed` is left out, so a re-analysis that changes nothing stamps
-- nothing. The columns are those the grid shows or sorts by, or that decide duplicate status.
CREATE TRIGGER analysis_rewritten AFTER UPDATE ON analysis
WHEN (old.version, old.size, old.mtime, old.rotation, old.kind, old.error, old.file_hash, old.pixel_hash, old.format,
      old.orientation, old.width, old.height, old.pdq, old.pdq_quality, old.pdq_dihedral, old.thumbhash, old.frames,
      old.taken, old.taken_utc, old.duration_ms, old.poster_ms, old.cut_left, old.cut_top, old.cut_right,
      old.cut_bottom, old.borders_due, old.dups_state)
    IS NOT (new.version, new.size, new.mtime, new.rotation, new.kind, new.error, new.file_hash, new.pixel_hash,
      new.format, new.orientation, new.width, new.height, new.pdq, new.pdq_quality, new.pdq_dihedral, new.thumbhash,
      new.frames, new.taken, new.taken_utc, new.duration_ms, new.poster_ms, new.cut_left, new.cut_top, new.cut_right,
      new.cut_bottom, new.borders_due, new.dups_state)
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = new.file_id;
END;

-- the duplicate-status triggers (4.4)
-- the meta keys: no log yet, so the horizon is the current change number (a client below it reloads)
INSERT INTO meta (key, value) VALUES
    ('log_horizon', (SELECT value FROM meta WHERE key = 'seq')), ('view_epoch', 0);
```

- The two meta keys are added to the expected schema through the migration: `SEEDS` (`schema.rs:373`) compares the keys that `expected()` produces by running the migrations (`schema.rs:439-449`), so no `SEEDS` edit is needed.
- `files_log` is a catalog table, so `carried_is_every_table_of_the_current_schema` (in `catalog/src/schema.rs`) requires it in `CARRIED` or `TRANSIENT` (`rebuild.rs:134, :158`), and in only one. It goes in `TRANSIENT`: a rebuild sets `log_horizon` to the current change number, so no carried log row could ever be read (4.5). (Rebuilds keep file ids, `rebuild.rs:160-161`, so renumbering is not the reason.)
- The `analysis_rewritten` trigger's WHEN compares 27 columns on every analysis UPDATE. Its cost is measured in Phase 6.

### 4.4 Duplicate-status triggers

A file's duplicate status changes when a mate or a partner changes. The triggers stamp the files whose status flips, whoever writes.

- **Writes.** Every write to `analysis` is an upsert or an UPDATE, never `INSERT OR REPLACE`: `store_analysis` and `store_failure` are upserts, and the other writers (`store_hashes`, `store_pairs_done`, `store_skipped`) are UPDATEs. A REPLACE deletes the old row without firing a delete trigger, and the insert trigger sees only new values, so the replaced row's old hash would never be stamped. A test (or a `tools/` check) fails on any `INSERT OR REPLACE` into `analysis` or `pdq_pairs`. On a conflict the upsert fires the `UPDATE` triggers and not the `INSERT` ones; the bundled SQLite does so, and a test pins it (4.12). The `DELETE FROM pdq_pairs` that precedes a re-analysis fires `pdq_pairs_removed` for each pair.
- **Hash mates.** A mate is another analysis row with the same hash, `error IS NULL`, on a present file (`df.missing = 0`), the same rule as `present_analysis!` (`query/src/lib.rs:65-69`). A file's status flips only when its number of other present mates goes between 0 and 1, so each probe asks "exactly one other present mate?" with `LIMIT 2`, and that one mate is stamped:

```sql
CREATE TRIGGER analysis_file_hash_added AFTER INSERT ON analysis
WHEN new.error IS NULL AND new.file_hash IS NOT NULL
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = (
        SELECT min(m.file_id) FROM (
            SELECT d.file_id FROM analysis d JOIN files df ON df.id = d.file_id
            WHERE d.file_hash = new.file_hash AND d.file_id <> new.file_id AND d.error IS NULL AND df.missing = 0
            LIMIT 2) m
        HAVING count(*) = 1);
END;
CREATE TRIGGER analysis_file_hash_removed AFTER DELETE ON analysis
WHEN old.error IS NULL AND old.file_hash IS NOT NULL
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = (
        SELECT min(m.file_id) FROM (
            SELECT d.file_id FROM analysis d JOIN files df ON df.id = d.file_id
            WHERE d.file_hash = old.file_hash AND d.file_id <> old.file_id AND d.error IS NULL AND df.missing = 0
            LIMIT 2) m
        HAVING count(*) = 1);
END;
CREATE TRIGGER analysis_file_hash_moved AFTER UPDATE OF file_hash, error ON analysis
WHEN old.file_hash IS NOT new.file_hash OR (old.error IS NULL) IS NOT (new.error IS NULL)
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    -- the old group: its one remaining mate, if the row was in one
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE old.error IS NULL AND id = (
        SELECT min(m.file_id) FROM (
            SELECT d.file_id FROM analysis d JOIN files df ON df.id = d.file_id
            WHERE d.file_hash = old.file_hash AND d.file_id <> old.file_id AND d.error IS NULL AND df.missing = 0
            LIMIT 2) m
        HAVING count(*) = 1);
    -- the new group: its one mate, if the row joins one
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE new.error IS NULL AND id = (
        SELECT min(m.file_id) FROM (
            SELECT d.file_id FROM analysis d JOIN files df ON df.id = d.file_id
            WHERE d.file_hash = new.file_hash AND d.file_id <> new.file_id AND d.error IS NULL AND df.missing = 0
            LIMIT 2) m
        HAVING count(*) = 1);
END;
```

The three `pixel_hash` triggers (`analysis_pixel_hash_added`, `_removed`, `_moved`) are the same text with `pixel_hash` in place of `file_hash` in each probe and each WHEN. Those mates are the pixel group. `HAVING` without `GROUP BY` needs SQLite 3.39 or later; the bundled build is 3.54.1.

- **Files going missing.** `files_source_changed` stamps the file itself (`schema.rs:728`). Its mates and partners also flip, so:

```sql
CREATE TRIGGER files_missing_dups AFTER UPDATE OF missing ON files
WHEN old.missing IS NOT new.missing
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    -- the pair partners within GROUP_DISTANCE (over-stamping is harmless)
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id IN (
        SELECT b FROM pdq_pairs WHERE a = new.id AND distance <= 22
        UNION SELECT a FROM pdq_pairs WHERE b = new.id AND distance <= 22);
    -- the file's own hash mates (the exactly-one-other rule, with the file's row as the source)
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id = (
        SELECT min(m.file_id) FROM (
            SELECT d.file_id FROM analysis s JOIN analysis d ON d.file_hash = s.file_hash
            JOIN files df ON df.id = d.file_id
            WHERE s.file_id = new.id AND s.error IS NULL AND d.file_id <> new.id AND d.error IS NULL AND df.missing = 0
            LIMIT 2) m
        HAVING count(*) = 1);
    -- the same probe on s.pixel_hash = d.pixel_hash (the pixel group)
END;
```

- **Pairs.** `GROUP_DISTANCE` is 22 (`model/src/pdq.rs:12-16`), the same as `pair!` (`query/src/lib.rs:72-84`). `store_pdq_pair`'s upsert can take an existing pair from distance 23 to 31 (`MATCH_DISTANCE`) to 22 or less by an UPDATE, so the UPDATE needs its own trigger. The three pair triggers stamp both files of a pair within the group:

```sql
CREATE TRIGGER pdq_pairs_added AFTER INSERT ON pdq_pairs WHEN new.distance <= 22
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id IN (new.a, new.b);
END;
CREATE TRIGGER pdq_pairs_removed AFTER DELETE ON pdq_pairs WHEN old.distance <= 22
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id IN (old.a, old.b);
END;
CREATE TRIGGER pdq_pairs_moved AFTER UPDATE OF distance ON pdq_pairs
WHEN (old.distance <= 22) IS NOT (new.distance <= 22)
BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET changed = (SELECT value FROM meta WHERE key = 'seq') WHERE id IN (new.a, new.b);
END;
```

- **Cascades.** Whether SQLite fires the child's DELETE triggers for `ON DELETE CASCADE` rows (the analysis row and the pairs under a removed `files` row) is **not settled** by the source (the foreign-key action trigger, `sqlite3.c:139058`, was not enough). The design does not rely on it. The test in 4.12 settles it: it deletes a `files` row that has an analysis row and pairs, and checks the stamps. The cascade does fire the triggers (4.12), so the removal path needs no explicit stamp. The explicit stamp, before the delete, is the fallback if a SQLite version stops firing them; the test would catch that.
- At a billion files, `BOUNDED_MEMORY_PLAN.md` D27's `hash_runs` with `present_xor` replaces the probes, with the same rule: stamp on a 1 to 2 transition.

### 4.5 `view_epoch`, `log_horizon`, rebuilds and pruning

- `view_epoch` is bumped **in the rewrite's own transaction** by `set_taken_zone`, `set_import_version`, `set_borders_version` and `set_dups_version` (the last one gains the call in this phase), through `bump_view_epoch(tx)` (`UPDATE meta SET value = value + 1 WHERE key = 'view_epoch'`). `set_mark` errors on a missing key (`lifecycle.rs:98-112`), which is why TO_11 seeds it. `set_thumbs_version` (`lifecycle.rs:151`) rewrites no analysis rows, so it does not bump.
- `log_horizon` is the lowest change number the log still covers. A rebuild sets it to the current `meta.seq`, so every client cursor is below it and gets `Reload`. `copy_tables` (`rebuild.rs:181-221`) copies meta's keys from the old file (`rebuild.rs:204-211`), so the two keys are set **after** the copy, in the same transaction, before the commit:

```sql
UPDATE main.meta SET value = value + 1 WHERE key = 'view_epoch';
UPDATE main.meta SET value = (SELECT value FROM main.meta WHERE key = 'seq') WHERE key = 'log_horizon';
```

- **Pruning** runs at the end of each Scans pass and each CLI scan (`crates/catalog/src/log.rs`, new module). The constants: `LOG_PRUNE_ABOVE` = 1,000,000, `LOG_KEEP` = 512,000, `LOG_PRUNE_PAGE` = 10,000 (the hysteresis avoids pruning on every scan). The horizon moves first, so no delta reads a row being deleted:

```sql
SELECT count(*) FROM files_log;                                                   -- n; stop unless n > LOG_PRUNE_ABOVE
SELECT seq FROM files_log ORDER BY seq LIMIT 1 OFFSET :n_minus_keep;             -- cut: the oldest seq to keep (n - LOG_KEEP)
UPDATE meta SET value = :cut WHERE key = 'log_horizon';                           -- one write transaction, first
DELETE FROM files_log WHERE seq IN (                                              -- one transaction per page, until a page deletes nothing
    SELECT seq FROM files_log WHERE seq < :cut ORDER BY seq LIMIT :LOG_PRUNE_PAGE);
```

- Another process may prepare a catalog the app has open (`catalog/tests/other_process.rs`). Its bump is a rewrite, so it moves `view_epoch`, and an open grid's next delta answers `Reload`.

### 4.6 Catalog reads

- **Full list.** `list_grid` (`catalog/src/browse/read/grid.rs:115-153`) returns `(Vec<Placed>, Stamp)` (`Placed` is 4.7). The stamp is read in the list's own read transaction (WAL gives one consistent snapshot per transaction), so the rows and the stamp agree.
- **Delta.** `Catalog::grid_delta(listing, since: Stamp, bound: usize) -> Result<Delta>`, in **one** read transaction. `DELTA_BOUND` = 8,192 changed rows. A delta is one transaction and the client merge is O(suffix), so above the bound a full list is cheaper and bounds the memory. The value is checked in Phase 6 against the delta timing. The result is `Delta::Changes { stamp, upserts: Vec<Placed>, removed: Vec<FileId> }` or `Delta::Reload`:
  1. If `since.epoch != view_epoch`, or `since.seq < log_horizon`: `Reload`. Read both from `meta`.
  2. The changed ids, through the index `files_changed`: `SELECT id FROM files WHERE changed > :since ORDER BY changed LIMIT :bound + 1`. More than `bound` rows: `Reload`.
  3. The removed ids: `SELECT id FROM files_log WHERE seq > :since LIMIT :bound + 1`. More than `bound` rows: `Reload`.
  4. The listing for the changed ids. Their ids go into the temp table `temp.delta_ids` (`temp_store = MEMORY` is already set, `schema.rs:377-378`), and `grid_sql`'s WHERE gains `AND f.id IN (SELECT id FROM temp.delta_ids)`. The rows it returns are the **upserts**. A changed id that the listing no longer returns (a file moved out of the folder, or out of the filters) joins the removed ids. The temp table is emptied at the end of the transaction.
  5. `Delta::Changes { stamp: Stamp { seq: <meta.seq read at the start>, epoch }, upserts, removed }`.

  This is one transaction, not paged. The bound is the reason: a delta bigger than the bound is a `Reload`, so no page cursor, no convergence argument and no snapshot across pages is needed. A change of listing (a filter, a sort, a folder) is a new list, so it asks for a full list, not a delta.
- **Temp table, not `rarray`.** A `rarray` needs rusqlite's `array` feature, which the workspace's rusqlite entry did not enable when this was written (it does now, in `[workspace.dependencies]`). The temp table needs no feature and no dependency.
- **Planner.** The planned outcomes are: the changed-ids query `SEARCH files USING COVERING INDEX files_changed`, and the listing query on `delta_ids` by rowid. They are written for the bundled build, which is SQLite 3.54.1 (`libsqlite3-sys` 0.40.0, rusqlite 0.43.0 at the fork rev `ac441bb`: the `Cargo.lock` entries for those two packages, and the `[patch.crates-io]` rusqlite entry). They are not run here; the test in 4.12 runs `EXPLAIN QUERY PLAN` on that build once the queries are written; the outcomes are checked then, not now.

### 4.7 Shared ordering

The grid's order is the SQL's order today, plus one rule the client must reproduce. `compare_keys` (`catalog/src/browse/read/grid.rs:85-96`) takes `rusqlite::types::Value`, and `pg-model` has no rusqlite, so the comparison moves to `pg-model` with its own value type. The key values are what `key_sql` (`crates/query/src/lib.rs:355-390`) computes: Text for Extension (`NULLIF(f.ext, '')`), and integers for the rest (Size, Modified, Taken, Created, Rating, Megapixels, Width, Height, the AspectRatio and Borders values in millionths, Duration, Frames). No key is a real number. The keys are raw, not coalesced, so a missing value is NULL.

**The types** (in `pg-model`, `crates/model/src/rules.rs`, beside `order_key`, `:257`):

```rust
/// A row's sort key as the catalog computes it (`key_sql`). Null when the file has no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortValue { Null, Int(i64), Text(Box<str>) }

/// One row of a sorted list: the entry, its sort key, and its `order_key` (the tie-break after the key).
pub struct Placed { pub entry: Entry, pub key: SortValue, pub order: i64 }

/// The list's order: the key (reversed when `descending`), then `order`, then the file id, the last two ascending in both
/// directions. A Null key comes last in both directions. For Name, the key is the `order_key` itself.
pub fn compare_placed(a: &Placed, b: &Placed, descending: bool) -> Ordering
```

`compare_placed` is the existing `compare_keys` logic (Null last, `Integer`/`Text` by value, reversed when descending), followed by the two ascending tie-breaks. The Grid compares an upsert with a row in the list by calling the same function on the row's key, `order` and id.

- **The tie rule (one rule).** Today keyed sorts tie in `order_key`, id ascending in both directions (`grid.rs:160, :166` ascend the ties of keyed sorts), but Name descending reverses `order_key` and `id` (`dir` at `grid.rs:160`, `ORDER BY f.order_key {dir}, f.id {dir}` at `:166`). The merge cannot reproduce that special case without a special case of its own. The fix is in `grid_sql`: the id is always ascending, so `ORDER BY f.order_key {dir}, f.id ASC` for Name, and `ORDER BY f.order_key ASC, f.id ASC` for keyed sorts (the keyed rows are then sorted in Rust by `compare_placed`, stably). The visible change is the order of files with the same `order_key` in Name descending, which is rare.
- **Carrying the key.** `list_grid` selects the key (`key_sql`, the column after the entry columns, `SORT_COLUMN` at `grid.rs:35`) and `f.order_key`, and builds a `Placed` per row. For Name the key is `Int(order_key)`. Keyed lists are sorted in Rust with `compare_placed`, as now (`grid.rs:143`).
- **Option B: the sort position is carried beside each entry; `Entry` stays 56 bytes.** `Entry` is the row type the Qt model and every other reader use, and the sort position is needed only by the sorted list the merge maintains, so it is a separate field of the list row. Option A would put the keys into `Entry`, making it 80 bytes (96 with an `Option<i64>` for the order), and every reader would carry them. The bytes per list row are about the same either way: `Placed` is 88 bytes (`Entry` 56, `SortValue` 24, `order` 8). Estimated: 6 MB at 190,000 rows and 32 MB at a million, from the field sizes, unmeasured (Phase 6 records it).
- **Entry's own fields** (`duration_ms` clamped to `NO_DURATION`, `width` and `height` coalesced to 0, `frames` as `animated`) are display values and are not sort keys. The key is always the SQL value, never the `Entry` field (`grid.rs:26-27, :63-65`).
- The coverage of the sort values is the authorizer test (4.12): each `key_sql` column is read by the statement, and each is stamped.

### 4.8 The app and the reader

- `Kind::GridDelta` (`crates/app/src/reader.rs:40-50`, cite re-checked in Phase 4). `submit` keys requests by kind, so a delta never cancels the full list (a different kind), and a newer delta replaces a waiting one. That is correct, because the client holds the `since` of its last applied delta, and a replaced request still starts from it.
- The generation: the client sends its grid generation with each request. The reader does not check generations (today's design); the client does (`client/src/grid.rs:282`). A delta with an old generation is dropped by the client, which then asks for a full list.
- Delivery: a `Delta` or `Reload`, through the closure pattern of `model.rs:135-149` (cite re-checked in Phase 4).

### 4.9 The client merge

`Grid` (`crates/client/src/grid.rs:108-129`) keeps `entries: Vec<Entry>` (its `entries()` accessor is unchanged) and a parallel `positions: Vec<Position>` with `Position { key: SortValue, order: i64 }`. It gains an id index, `HashMap<FileId, usize>`, and it stores the stamp it last applied. `position_of` (`grid.rs:168`) becomes an index lookup. A `Placed` from the catalog is split into its entry and its position on arrival.

`Grid::merge(delta)`:

1. Sort the upserts by `compare_placed` (O(k log k)). Drop a removed id that is also upserted: an id removed and then reused (rowid reuse; `files.id` has no `AUTOINCREMENT`) appears in both lists, and the upsert is the live row.
2. Find `p`, the first changed position: the smallest index of a removed id or an upserted id that is in the list (the id index, O(1) each), and, for each upsert, the position it sorts to (a binary search with `compare_placed` against the rows). Entries before `p` are unchanged and are not touched.
3. From `p` on, drop the rows whose id is removed or upserted, then merge the rest with the sorted upserts by `compare_placed`, into new vectors, and splice them in place of the old suffix (`Vec::splice`, one memmove of the tail). Cost O(suffix + k).
4. Rebuild the index from `p` on (O(suffix)).
5. Relayout from the row that contains `p` (below).
6. Remap the selection, the anchor and the current item by id (`selection_after`, `grid.rs:384`).

**Incremental layout (decided by the user: in scope).** `justify_into` (`crates/client/src/layout.rs:24-36`) lays out the whole list on every call. The greedy layout (`greedy`, `:38-57`) is determined from a row's first item onward: a row closes when its items fill the width, and nothing before it matters. So a merge relays out only from the row that contains `p`:

- Find that row by `partition_point` on the row starts (O(log rows)). Keep the rows before it.
- Re-run the greedy from the row's first item over the new aspects, emitting rows.
- Stop once the last changed item has been passed and a new row starts at an item where an old row started, shifted by the change in length so far. From there the old rows are the same except for their `first`: copy them with `first` shifted by that change (integer arithmetic only, no aspect reads, no fit arithmetic).
- Splice the new rows in place of the old ones (`Vec::splice`).

The cost is the changed rows, found in O(log rows), plus an integer pass over the rows below the change. Aspect reads and row fitting happen only for the changed rows. The view still moves every row below a changed row, because the rows stack vertically; that is inherent in the layout.

The layout test in 4.12 checks that the incremental result equals `justify_into` on the new list, row for row. The Phase 6 bench measures a merge that changes one item near the top (the longest suffix) and one near the bottom (the shortest).

**Why one range is enough.** The justified layout is greedy from the first row. A change at position p can change every row from p's row onward, and no row before it. So the changed rows are always a suffix of the row list, plus tail inserts and removals. `RowChange::between` (`layout.rs:174-184`, the `RowChange` at `:155`) already expresses one contiguous changed range, and it is sufficient.

**The Qt side.** The model must receive the merge's removals and insertions at their positions (`beginRemoveRows` and `beginInsertRows` for middle edits, not only at the tail), and a `dataChanged` for the rows whose frames changed. The gallery model's `show()` (`crates/desktop/src/gallery/model.rs`, about lines 215–258) already places a middle insert or remove at its position, with `begin_insert_rows` or `begin_remove_rows`, and sends `data_changed` for in-place updates. `Plan::shape()` (`crates/client/src/grid.rs:66`) returns one contiguous run per plan, so a delta with several runs is sent as several plans, or as a reset.

### 4.10 The viewer hold and the queue

- While the viewer is open, the list is not shown, and a merge would move the viewer's item. Today `arrive` stores the whole list in `held` (`grid.rs:285-288`), overwriting any previous one.
- Under this design a delta while viewing is **composed** into a `pending` delta, not overwritten: `compose(old, new)` gives upserts = `(old.upserts minus the ids in new.upserts or new.removed) ∪ new.upserts`, removed = `(old.removed minus the ids in new.upserts) ∪ new.removed`, stamp = `new.stamp`. On release, `pending` is applied with `merge`.
- A full list arriving while viewing replaces `held` and clears `pending`.

### 4.11 Events, coalescing and throttling

- `Change` (`jobs/src/events.rs:128`) stays payload-free and `Copy`. `Imported` and `Scanned` are sent as today, and the desktop pump coalesces them per pass (`desktop/src/library.rs:41`, `pump.rs:33-62`).
- Today `Main.qml` calls `reloadAll()` on `onScanned` (`:127`) and `refreshGrid()` on `onImported` (`:128`); `reloadAll` (`:255`) calls `refreshGrid` (`:257`). Phase 4 makes both ask for a delta, not a reload. The folder-tree refresh moves to `Change::Round`, since a refresh per round is bounded and a refresh per scan is not. `reloadAll` is split so the tree refresh is its own call. Facet counts and the tree refresh at the end of a round and at a slower interval, not per merge.
- **One delta in flight.** The controller keeps at most one delta request outstanding. A signal that arrives while one is in flight sets `delta_due`; the delta's result sends the next request if `delta_due` is set.
- **Throttle.** Applied deltas are at least 1 s apart. The client's existing timers take it: a new `Timer::Delta` variant (`client/src/controller/deadlines.rs:10-19`, with `delay_ms` 1000) is armed when a delta is applied. A signal while it is armed sets `delta_due`; when the pause fires with `delta_due` set, the request goes out. The host's one timer and `Event::Timeout` (`crates/client/src/controller.rs:137`, `:328`) already run the pauses, so there is no `Command::ArmDeltaTimer`, no `Event::DeltaDue` and no QML `Timer`. One method is added: `Deadlines::is_armed(timer)`. The throttle is trailing: the last change is always applied.
- Phase 5 adds the per-flush tick from the import. Until then, the only `Imported` is the one after `pg_import::import` returns (the Import pass, `runner.rs:520-525`), so tiles fill at pass end, not progressively.

### 4.12 Tests

- **Merge equals reload**, for every sort key (`SortKey` has 14 variants, `model/src/query.rs:124`), in both directions, with random inserts, updates, removals and filter changes that files enter and leave; ties; NULL keys. A property test over random operation sequences.
- **Composition equals sequential application**: composing two deltas and applying them equals applying each in turn.
- A file moved, renamed (Name, Extension, Created and folder changes) and removed (a root removed by `remove_root`; a scan's removal), each seen by the delta with no full reload.
- A delta racing a scan: after the writes stop, the next delta leaves the grid equal to a full list.
- A removed largest id given to a new file, in one delta: the removal applies first, and the upsert wins.
- The duplicates listings equal a full reload after each change of a mate: a hash mate imported, gone missing, back, removed, and re-analyzed with another hash; a pair inserted and deleted; a partner gone missing; groups of two, three and many.
- A bulk rewrite by another process's `prepare` answers `Reload` (the epoch moved).
- `Reload` for a `since` below `log_horizon`, on another `view_epoch`, and over the bound.
- **Coverage test** (by behaviour, not by a hand list): each `grid_sql` variant (every sort key, both directions, every filter) is prepared under SQLite's authorizer (`sqlite3_set_authorizer`; rusqlite's `authorizer` is behind the `hooks` feature, which the workspace's rusqlite entry enables). It reports every `(table, column)` the statement reads. For each column, the test changes it on a fixture row and checks that the row's `changed` advanced, a `files_log` row was written, or `view_epoch` moved. A column changed only by a bulk rewrite is listed with the rewrite that bumps the epoch. A trigger missing on a new column fails the test. It covers `dups_state` (4.4).
- **Trigger behaviour pins**: the bundled SQLite fires `AFTER UPDATE` triggers on an upsert conflict (`ON CONFLICT DO UPDATE` is an UPDATE for triggers); a test pins it. A test fails on any `INSERT OR REPLACE` into `analysis` or `pdq_pairs`.
- **Cascade**: delete a `files` row that has an analysis row and pairs; check the stamps of the mates and partners. Settled 2026-11-13 by an experiment on SQLite 3.49.1 (the bundled build of that date, since replaced by 3.54.1) with these trigger shapes: the cascade fires the child tables' delete triggers, so the pair trigger stamps the mates, and the analysis trigger's update of the deleted parent does nothing and raises no error. This test keeps the result on the bundled build (SQLite 3.54.1: the fork rev `ac441bb`, `libsqlite3-sys/sqlite3/sqlite3.h`).
- **Incremental layout equals full layout**: random inserts, removals and aspect changes, including edits at the first and last items, and runs that resync at once or never. The rows equal `justify_into` of the new list (exact `first`, `count` and `height`).
- Selection and anchor stability when rows insert above; the viewer open; memory (the id index and the positions, 32 bytes per row for the positions, plus the index's entries, measured in Phase 6).

## Phase 5: first-import merge (one decode per new file)

> Superseded 2026-11-17 (commit 5fcff8bb): the import no longer decodes, so the one-decode-per-new-file merge described here does not apply to the import. The Thumbs, Placeholders and Hashes passes each decode their own input, and sharing one decode among them is the open work of this phase. Sections 5.1 to 5.13 are kept as the record of the 2026-11-13 decision.

For files whose analysis is due, one decode feeds every enabled stage. The catch-up passes (Thumbs, Borders, Dups) stay separate and unchanged. A unified catch-up worklist was rejected: an OR join scans every file each round, depends on a healthy cache and needs watermark rework.

### 5.1 The decode policy and the thumbnail definitions (decided 2026-11-13)

> Superseded 2026-11-17 (commit 5fcff8bb): the "one full stored decode F" of this section is not what the code does. The import reads headers only; the ThumbHash is the Placeholders pass's (fitted to 100 px of an upright image at least 200 px on its long side, from the cached thumbnail or a DCT-scaled decode); the hashes are the Hashes pass's DCT-scaled decode, as before the 2026-11-13 decision.

**Decided by the user (2026-11-13).** Every analysis-due file gets **one full stored decode F**, whatever the toggles. The hashes, ThumbHash, borders and dimensions come from F, so they are the same in every mode. A decode choice that depended on the toggles would make ThumbHash depend on them, which the single-definition rule forbids.

- **The DCT-scaled decode is removed everywhere** (decided 2026-11-13, no further check). The codec decodes only at full size, so the scaled variants of `MinSize` (`Sides`, `Upright`, `Fit`), the viewer's `min_size` and its `DECODE_MARGIN` go, and `Shown::covers` and `resized` work from the full image. Measured: a JPEG decode takes 135.1 ms full against 120.5 ms scaled (122 sampled JPEGs, 9.2 MP mean, release build, one thread), and the viewer's time with a scaled decode was the same as with a full decode, and it scaled in 5% of JPEGs at 1280×720 and 0% at larger windows (PNG scaled in 0% at every window), so nothing is lost. A PNG decode is 62 to 65 ms either way. The viewer holds one full image, as it does in practice already.
- **Every thumbnail comes from F**, whether made at ingest, by the Thumbs catch-up, or on demand. F is the full upright image, downscaled with `to_height`. T4 (`MinSize::Upright(0, 4*height)`, `maker.rs:127`) is no longer a thumbnail source. There is **one thumbnail definition**, so the same file gives the same thumbnail whichever path made it (the test in 5.7). Thumbnails are cached, so each file and height is decoded in full once.
- **Cost.** Today's import decodes DCT-scaled to 512x320 (`Analyzer::analyze_image`). The merged import decodes F, which is larger, for every new JPEG and PNG. With both toggles off, the import still decodes F, because ThumbHash needs it (ThumbHash is always stored). The Thumbs catch-up now decodes F for every older file it covers, which costs more CPU than the T4 decode it replaces. It runs in the background at below-normal priority, and on-demand misses are rare with pre-generation on. Phase 6 reports the import and catch-up regressions and their size.

### 5.2 The over-cap rule (decided 2026-11-13)

**Decided (2026-11-13).** Lands in Phase 3 (3.2) with the first full decode; Phase 5 keeps it. A JPEG whose F allocation (`w * h * 3` from the header) exceeds `max_decode_bytes()` (512 MiB, `codecs/limits.rs:8`) is **not decoded at all**:

- Metadata only: `Meta::read` gives the dimensions, the taken time, the orientation and so on, as for any file.
- `analysis` is stored with `file_hash` (the bytes are hashed, from the read buffer), the dimensions and the metadata. `pixel_hash`, the PDQ hashes and ThumbHash are NULL. `dups_state = 'skipped'` (3.3).
- Borders: a `file_failures` row with `pass = 'borders'`, `retry = 0` and the error text `too large to look at`, and `store_borders(None)`, so the borders state is terminal and the file is not retried.
- Thumbnails: none stored. Every thumbnail comes from F (5.1), and an over-cap file has no F, so no thumbnail is cached for it. Its grid tile shows an **app-drawn warning glyph**, not a bitmap from the OS. The image provider (`providers.rs:115`, which already draws the ThumbHash placeholder through `images().placeholder`) returns the glyph when a file has neither a thumbnail nor a ThumbHash. The glyph is drawn at display time, never cached, so it is not a thumbnail source and 5.1's single definition holds. It uses the theme tokens, so it works in light and dark, and it scales cleanly. A tooltip on the tile gives the reason, the same text as the Borders error ("too large to look at"). The glyph and the tooltip wording were approved (2026-11-13).
- Read: the file is read whole, as the pipeline reads every file today (`pipeline.rs:282-297`). No streaming hash is needed: the pool admits a file larger than its budget only when no other buffer is held (`take()`, `crates/jobs/src/pipeline.rs:391-398`), so the file is read alone, and `FileHash::of_reader` already exists for videos (`crates/model/src/analysis.rs:21`). The read buffer is held under the pool's lease (5.3).
- `ImageFacts.pixels` (`crates/model/src/analysis.rs:93`) and `Analysis.thumbhash` (`:41`) become optional, because an over-cap row has no pixels and no ThumbHash. The production reader of `ImageFacts.pixels` is `crates/import/src/lib.rs:61` (`pixels.pdq.quality`). The production writer of `Analysis.thumbhash` is `crates/catalog/src/writers.rs:325`, which only stores it. The `maker.rs:201-203` uses are in a test. Both fields need `Option` handling here.

This is a decision, not a fallback: an over-cap file has no stored analysis that depends on pixels, so no second definition exists.

### 5.3 The decode budget and the retained buffers

The budget is built in Phase 3 (3.7) and used here by the merged pass: a worker reserves `w * h * 3` before its decode and releases it when the file's `process` ends.

- Default **1.5 GiB** in use at once. A single file larger than the budget runs alone (nothing else is reserved), so the budget cannot deadlock. The 512 MiB cap (5.2) bounds that single file.
- Without a budget, twelve workers on large files exceed the 4 GiB target by arithmetic (`convert.rs:26`: a 100 MP RGBA PNG holds about 400 MB in `out.pixels`). The fix is a budget, not a worker count.
- **Retained buffers** are the other part of memory. The `image` buffer is kept between files up to `KEEP_BYTES` (96 MB, `analyze.rs:64`), so at most 12 x 96 MB = 1.15 GB is retained at steady state. `spare` is released after each file (it is only needed on rotation), and `pdq_in` (512 px) and `small` (100 px) are bounded by their sizes.
- The peak working set is the in-use budget, plus the retained buffers, plus the grid and the sink: about 3.2 GB at the worst (an estimate). That meets the 4 GiB target by construction, and Phase 6 measures it.
- **Read buffers** are not in the budget. They are bounded by the pool's lease (`pipeline.rs:282-297`). The lease admits a file larger than its size alone, as the budget does: the pool admits it once no other buffer is held (`take()`, `crates/jobs/src/pipeline.rs:391-398`), and `Options::buffer_bytes` documents it as read alone.

Tests: those of 3.7.

### 5.4 The stages on a worker (fixed order)

> Superseded 2026-11-17 (commit 5fcff8bb): step 7 (ThumbHash) moved to the Placeholders pass. Steps 1 to 6 and 8 describe a decode the import no longer does.

For each file, on one worker, in this order:

1. **Read** the bytes whole (images) or stream (video), in the pool's lease (`pipeline.rs:282-297`).
2. **Header and budget.** The header gives the dimensions (`Decoder::dimensions`, 3.2). Over the cap: the over-cap outcome (5.2), with no reservation and no decode. Otherwise reserve `w * h * 3` (3.7; `Ingested::NotRun` if a stop ends the wait for the reservation).
3. **Decode F** into `image` (`decode_stored_into`, `MinSize::Full`, the stored orientation). Video: the poster frame into `image` at display size (`analyze_video`, `analyze.rs:194-214`).
4. **Borders** on the stored, unoriented pixels (`detector.detect`, `analyze.rs:141-152`), inside `panics::guard` with `AssertUnwindSafe`. A caught panic resets the detector with `Detector::trim`, which already rebuilds it to the default (`analysis/src/borders.rs:63-67`). Borders (this step) must come before step 5, because the borders are measured on the stored orientation (`analyze.rs:141-145`).
5. **Orient** in place: `orient_into` (`decode.rs:197-203`), which swaps through `spare`, and `spare` is released.
6. **Hashes** on the upright image: `analysis::hashes` (3.1: PDQ from `fit_within(img, 512)`, the pixel hash, the eight dihedral hashes), always, in every mode.
7. **ThumbHash** from the upright image: `analysis::thumbhash` (fitted to 100). Always computed, because it is always stored.
8. **Thumbnail** if the thumbnail feature is on and the cache is enabled: `to_height` from the upright image into the encoder half (5.7), producing the JPEG bytes.

The outcome for the sink: `Outcome { index, analysis: Result<Analyzed>, borders: Stage<Option<Detection>>, dups: DupsState, thumb: Stage<Vec<u8>> }`, with `Stage<T> = Skipped | Done(T) | Failed(anyhow::Error)`. `Stage` lives in `pg-jobs` (`crates/jobs/src/stage.rs`, generic). `Outcome` and `Ingested` live in `crates/import/src/ingest.rs`, since they name `Analyzed`, `Detection` and the thumbnail bytes. `Ingested` is `Done(Outcome) | NotRun`. Rules:

- A decode error gives `Skipped` to borders, to the thumbnail and to dups; the analysis error is the file's failure (`store_failure`, 3.5).
- A video: borders `Skipped`; `dups` = `Nothing` (no pixel hash, `pdq_dihedral: None`, `analyze.rs:213`).
- An over-cap JPEG: `analysis` is `Ok` with the NULL pixel fields, borders `Failed` (the terminal row), `dups` = `Skipped`, the thumbnail `Skipped`.
- Dups for an analysed image: `Owed` when detection is off, `Done` when on (3.5), with the hashes from step 6.
- The io read error: see 5.6.

### 5.5 The sink (a scoped thread that borrows the catalog)

Today the sink (the catalog transactions and the cache writes) runs on the job thread, which has no priority (`crates/jobs/src/service.rs:91-94`). Only the readers and workers call `set_current_thread` (`pipeline.rs:131, 166`), so the sink runs at Normal. Import already runs at BelowNormal (`runner.rs:522`); the sink does not follow it. `pipeline::run` (`pipeline.rs:123`) uses `std::thread::scope`; the sink loop runs on the calling thread, after the readers and workers are spawned (`sink(...)?` at `:211`, the return at `:212`), and the final flush is at `:266-268`, after `run`.

Design:

- A trait, run on a scoped thread, with the batch rules inside it:

```rust
pub trait Sink<R>: Send {
    /// One result, in completion order. `Ok(false)` stops the pass, with no `end`.
    fn item(&mut self, index: usize, result: Result<R>, read: usize) -> Result<bool>;
    /// Called when no result arrives for `tick_every()`: the chance to flush a batch held too long.
    fn tick(&mut self) -> Result<bool> { Ok(true) }
    fn tick_every(&self) -> Option<Duration> { None }
    /// Once, when the results end (drained, or `ctl` stopped). Not after an error.
    fn end(&mut self) -> Result<()> { Ok(()) }
}
```

- `pipeline::run`'s scope spawns the sink thread **inside the existing scope**, with the sink closure and `&mut Catalog` borrowed into it (a scoped thread may borrow). The sink thread calls `priority::set_current_thread(opts.priority)` first, like the readers and workers. It loops on `res_rx` (`pipeline.rs:121`, `bounded(workers * 4)`) with `recv_timeout(tick_every)`: a result goes to `item`; a timeout goes to `tick`; a disconnect breaks the loop and runs `end()`, the final flush, on the sink thread. The job thread waits in the scope (joins the sink thread). Nothing else runs on the job thread during the pass.

```rust
let sink_thread = s.spawn(move || -> Result<Ended> {
    priority::set_current_thread(opts.priority);
    loop {
        let next = match sink.tick_every() {
            Some(every) => res_rx.recv_timeout(every),
            None => res_rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Outcome::Done { index, result, read }) => {
                if !sink.item(index, result, read)? || ctl.stop_requested() { return Ok(Ended::Stopped); }
            }
            Ok(Outcome::SetupFailed(error)) => return Err(error.context("worker setup")),
            Err(RecvTimeoutError::Timeout) => { if !sink.tick()? { return Ok(Ended::Stopped); } }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    sink.end()?;
    Ok(Ended::Drained)
});
```

- **Stop.** `StopOnDrop` (`pipeline.rs:126, 301-309`) stops the readers, so the channels close. The sink drains the batch it holds (`end`) and returns. An early return from the sink (`false`, or an error) drops `res_rx`; the workers exit on their failed sends; the join returns; and `_stop` wakes the readers. No deadlock, and `end` runs before the producers are joined, since they never touch the catalog.
- **Held batches.** `tick_every` closes the gap that `pipeline.rs:260` leaves: today `every` is checked only when a result arrives, so an idle pass never flushes a held batch on time.
- **Send.** The sink thread needs `Sink: Send`, and therefore `Catalog: Send`. `Catalog` (`crates/catalog/src/core.rs:32-41`) holds a `rusqlite::Connection` (`unsafe impl Send` at rusqlite `lib.rs:371`), `PathRules` (a closed enum), `Option<Rebuilt>` (`PathBuf`, `String`, `Carried`) and `Option<pg_dbfile::Live>` (a `File`). None is `!Send`, so the sink needs no second connection. The build in Phase 6 (step 1) is the compile check; if it fails, the sink needs a second connection, and this section changes.
- **Batches.** The import and the Dups pass use `Batch { size: 100, every: Some(Duration::from_millis(250)) }`. The import's batch is 500 today (`import/src/lib.rs:18`). The reason is the app's writers' 100 ms busy wait (`crates/app/src/inbox.rs:73`): a sink transaction must be short. Borders and Thumbs keep theirs.
- **WAL.** The catalog is in WAL mode (`schema.rs:377`), so a BelowNormal writer does not block Normal readers. Writers are still serialised, so a BelowNormal sink holding the write lock can delay a Normal writer (a rating, a scan). The batch size and the busy timeout bound this. Phase 6 records the sink's lock-hold time (6.4).

Tests: the sink drains after the readers stop; a full channel slows the workers, not the reverse; a stop during a batch writes the kept rows and nothing more; a held batch is written within about 250 ms with no further result.

### 5.6 Writes per batch

One transaction per batch, of at most 100 files or 250 ms of work (5.5). For each outcome, in arrival order:

1. **Analysis.** `store_analysis(item, analyzed, dups)` (3.5): the upsert writes the analysis row, the `dups_state` and its hashes (`Done`, `Owed`, `Skipped` or `Nothing`), and NULLs `cut_*` explicitly. A failed analysis is `store_failure` instead (step 4).
2. **Borders.** `Done(Some(d))` -> `store_borders(Some(d))`; `Done(None)` -> `store_borders(None)`; `Failed` non-io -> the `file_failures` row (`retry = false`) and `store_borders(None)`, as the Borders catch-up does (`borders/src/lib.rs:70-100`); `Skipped` -> nothing. An io read error is not a borders outcome here: it counts as an attempt, and at the third attempt it is the file's analysis error (below).
3. **Pairs** (only when hashed and detection is on): `pair_and_insert` (the sink owns the Pairer), `store_pdq_pair` for each neighbour, then `store_pairs_done`. The pairs come after step 1, because step 1 deletes the file's pairs (`writers.rs:254, :306`).
4. **Analysis error:** `store_failure` (the upsert of 3.5) instead of step 1; the file's `dups_state` becomes `'done'` (`Nothing`).

Then the transaction commits. Then the thumbnails of the accepted files go to `cache.put_batch(&mut thumbs, Some(height))`, **once per flush**, with at most 100 entries (the flush's thumbnails). `put_batch` takes a `Vec` of any length (`thumbcache/src/lib.rs:198-205`). The height is read at this flush (2.9): a thumbnail of a height the grid has left is dropped, not stored.

A failed transaction drops the batch's thumbnails. The Thumbs catch-up makes them, because the analysis rows were not stored. A crash between the commit and `put_batch` loses only thumbnails. A `put_batch` error follows 5.8.

**The io read error (decided 2026-11-13).** Every pass counts an io read error as an attempt, in `io_attempts` (3.3, TO_10). Below the third attempt nothing else is written: the import leaves the file pending, so the next round lists it again (`pending_import_sql`, `work.rs:17-31`: the file has no analysis row, so `files.pending` stays 1), and Borders and Dups leave it due. At the third attempt the row is cleared with the terminal result. The import writes its analysis error row (`import/src/lib.rs:72-78`), which creates the failure row through `analysis_import`, so step 4 applies and the file is not retried until it changes or `--retry-errors` is given. Borders and Dups write a failure row with `retry = false`, as their other failures do. Any stored result for the file and pass also clears the row, and so does a change to the file (`io_attempts_stale`), so a user Retry starts from zero. The read error is recognised by `downcast_ref::<std::io::Error>()`, as Borders does (`borders/src/lib.rs:79`): `read_error` (`jobs/src/pipeline.rs:273-278`) keeps the io error under its "read" context. Tests: the three resets, and the counter at 1, 2 and 3 attempts for each pass.

### 5.7 Encoder split

`Maker` (`crates/render/src/maker.rs:50-60`: `decoder`, `resizer`, `encoder: JpegEncoder`, `image`, `spare`, `small`, `file`) splits into:

- an **encoder** half: `resizer`, `small`, `encoder`. `encode_image` takes the `&Rgb` image (today it takes `height` and `out`, `maker.rs:155`); and
- a **decode** half: `decoder`, `image`, `spare`, `file`.

The ingest worker holds one `Analyzer` (decoder, `image`, `spare`, hasher, detector, thumbhasher, meta) and one encoder half. The on-demand path (`make_image`, `encode_frame_into`, and `app/src/images.rs` through `Makers` in `makers.rs`) keeps its `Maker`. The two `KEEP_BYTES` stay one per half: the analyzer's trims `image` at 96 MB (`analyze.rs:64`). The maker's limit was 18.9 MB (`maker.rs:23, :161-167`), sized for a scaled thumbnail decode. Every decode is now full size (5.1), so a 9 MP JPEG's 27 MB image would be released after every file. The maker's limit becomes 96 MB too, so both halves keep one full image between files. Merging the two constants changes what is kept between files, so it stays a separate decision (Not planned).

The test `kept_buffers_give_what_new_ones_do` (`maker.rs:209`) keeps its meaning. Ingest and on-demand thumbnails both come from F (5.1), so a new test asserts that the same file gives byte-equal thumbnails on the ingest path and the on-demand path, and that the stored analysis values are equal.

### 5.8 Cache failures and notices

Most of the cache-failure machinery exists:

- `Notice::CacheFull { height, made }` (`model/src/notice.rs:379-382`) and the cache group of notices, `CacheNotStored` among them (`notice.rs:422-452`).
- `ThumbStats.cache_full` (`thumbs/src/lib.rs:47`) is set when `put_batch` returns `Ok(false)` (`:290`), and the runner posts `Notice::CacheFull` (`runner.rs:563-564`).

What is added:

- `ThumbCache::is_enabled()` (`store.is_some()`, `thumbcache/src/lib.rs:74-76`). It does not exist today.
- **`put_batch` returns `Err`** (a channel failure, a SQLite error, a writer panic): the rule of 2.9, which is in Phase 2. One `CacheNotStored` for the pass, production stops for the pass, and the round goes on. The merged pass does the same. That is new for the import, which writes no thumbnails today.
- **`put_batch` returns `Ok(false)`** (full): `Notice::CacheFull` and the stop for the pass, as the Thumbs pass does.
- **A disabled cache** (`ThumbCache::disabled()`, `lib.rs:115-117`): the thumbnail stage is `Skipped` for the pass, with no notice. Its `put_batch` returns `Ok(false)` (`lib.rs:199-201`), so the `Err` path needs a read-only or locked cache file, or a `cfg(test)` failing store, to be reached in a test.
- **Height.** The thumbnails of a flush are for the grid height read at that flush (5.6). The merged pass is not stopped by a height change (2.9).
- The one production caller of `ThumbCache::put` (`thumbcache/src/lib.rs:188-192`, fire-and-forget) is `crates/app/src/images.rs:260` (`thumbs.put`), which is checked against the thumbcache type when Phase 5 starts.

### 5.9 Video

The poster frame is decoded once for analysis into `image` (`analyze.rs:197-198`), and it is the thumbnail source too (`Maker::encode_frame_into`, `maker.rs:148`). ThumbHash comes from it. There is no PDQ and no pixel hash for video (`pdq_dihedral: None`, `analyze.rs:213`), so a video's `dups_state` is `'done'` with no hash (`Nothing`). The Thumbs catch-up still decodes the poster again (`thumbs/src/lib.rs:320`, cite re-checked in Phase 5) for files that do not have a thumbnail. That is the catch-up's path, and it stays.

### 5.10 Priority table (after this phase)

| Work | Priority | Thread |
|---|---|---|
| Crops | BelowNormal (`runner.rs:430`) | readers and workers, and the sink |
| Scans | Normal (`runner.rs:464`, no pipeline) | job thread |
| Import (merged pass) | BelowNormal (`runner.rs:522`, today), and its ingest thumbnails at BelowNormal (the user's decision) | readers and workers, and the sink (the sink is new in this phase) |
| Borders | BelowNormal (`runner.rs:536`) | readers and workers, and the sink |
| Thumbs (catch-up) | Idle (`runner.rs:561`) | readers and workers, and the sink |
| Dups | BelowNormal (Phase 3) | readers and workers, and the sink |
| UI and render threads | AboveNormal on Windows (Phase 2) | GUI, render |

Import is already BelowNormal, and Scans is Normal. Moving Scans is not part of this plan.

### 5.11 The CLI and the import options

`gallery import` takes `--no-borders` and `--no-dups` (they set the features, 2.1 and 3.9; `--no-dups` is shipped as `no_dups`, `cli/src/args.rs:107`, so the sketch below does not add it again) and `--thumbs-height N` (it sets the grid height for the ingest thumbnails and turns that stage on; without it the thumbnail stage is off, since the cache has no height). `gallery borders` and `gallery thumbs` stay explicit commands and ignore the toggles (`cli/src/args.rs:86, :102`).

`Command::Import` (`cli/src/args.rs:74-84`) gains three fields after `again`. Clap derives the long names from the field names:

```rust
/// Find no borders (the Borders toggle off): none are stored, and none are looked for after the import.
#[arg(long)]
no_borders: bool,
/// Find no duplicates (the Duplicates toggle off): no hash pairing and no PDQ pairs.
#[arg(long)]
no_dups: bool,
/// Make the thumbnails at this grid height in the same decode (the cache's heights, 16 to 2048). Without it no
/// thumbnail is made and the thumbnail stage is skipped, since the cache has no height.
#[arg(long, value_name = "PX", value_parser = thumb_height)]
thumbs_height: Option<ThumbHeight>,
```

The arm in `cli/src/lib.rs`: `let features = Features { borders: !no_borders, duplicates: !no_dups, thumbnails: thumbs_height.is_some() };` and the options carry the height: `ImportOptions { features, thumb_height: thumbs_height, .. }`. `Features` stays bool-only (2.1), and the height is a separate value of the import options. The import takes `import(catalog, &opts, retry_errors, ctl)`, with the features and the height inside the options. `ThumbHeight` is already imported (`args.rs:7`).

- The doc comment of `fn thumb_height` (`args.rs:137`, its body at `:138-141`) names `--cap-gb`; that text belongs to `fn cap_gb` (`:143-146`). It is fixed in the same commit.
- The `import` crate's `Cargo.toml` header comment (`crates/import/Cargo.toml`, "not on the thumbnails or the writes") is updated. The crate gains `pg-render`, `pg-thumbcache` and `pg-trace`, which are on earlier lines of `crate-layers.txt` (5.13), so the edges are allowed.
- The `cli.ftl` help lines for the three flags (Phase 5).

### 5.12 Tests

- Borders from the merged path equal `find_borders` on the existing fixtures.
- Upright dimensions per rotation (EXIF orientation 1 to 8).
- Animated and tiny images get no borders but still analyze.
- The video poster thumbnail and ThumbHash come from one decode.
- **The stored analysis values are identical in all eight toggle combinations** (the single-definition test). The border columns (`cut_*`, `borders_due`) are compared only where both catalogs ran borders. The thumbnails are not compared (they are display artifacts).
- **A new file is decoded once.** A call counter on `Decoder`, `cfg(test)`, per instance, not process-global, so parallel tests do not share it (no counter exists today). The catch-up passes then find nothing to do.
- Over-cap: a synthetic JPEG header above the cap makes zero decode calls, and the row is `'skipped'` with the borders failure row.
- A detector panic keeps the analysis and the thumbnail, and resets the detector. The panic needs a `cfg(test)` injection point inside `pg-analysis`; the existing one (`crates/borders/src/fault.rs`) is reachable only from `pg-borders`.
- Catch-up-only io errors never replace a good analysis.
- The decode budget (3.7).
- The sink (5.5): the drain order, the back-pressure, the stop, and the held batch.
- A cache that is disabled, or full, does not fail the import; a `put_batch` `Err` posts `CacheNotStored` and the round continues.

### 5.13 Benches (written here, run in Phase 6)

Inputs: a PNG, a WebP and a 12 MP JPEG. The phase creates them (`make_bench_jpeg.py` gains `--format {jpeg,webp}` and `--size WxH`, seed 20261105); they are not assumed to exist. The `ir` harness folder holds `ir.rs` (the root: `include!("ir/linux.rs")` on Linux), `ir/linux.rs` (the per-file and viewer groups; `main!` at `:107`), `ir/catalog.rs` (`LAYOUT` at `:40`; the groups at `:235` and `:277`) and `ir/sample.jpg`.

The benches go in `pg-bench`, which runs only through `cargo bench -p pg-bench --bench ir` (`tools/profiling/bench.py:95`). A bench in `import` would never run under `just perf`. They are added to the `ir` harness through `include!`, as `catalog.rs` is (`linux.rs:105`):

- `per_file` gains `borders_jpeg` (the group at `linux.rs:102` becomes `benchmarks = [analyze_jpeg, thumb_jpeg, borders_jpeg]`). It calls `Analyzer::borders` (`analysis/src/analyze.rs:135`) on `ir/sample.jpg`.
- `ir/decode.rs` (new): group `decode`, with `decode_png` (the input made in setup by `pg_testkit::png_rgb`), `decode_webp` (`ir/sample.webp`, generated by `make_bench_jpeg.py --format webp`, not committed) and `decode_jpeg_12mp` (read in setup from the bench data; not committed).
- `ir/import.rs` (new): group `import`, with `import_merged` (id `small`): runs `pg_import::import` on a 50-file folder from the bench data into a fresh catalog copy made in setup. The budget key is `import-merged` (folder `import_merged.small`, `perf-compare.py:30-35`); it is added only with the bench (Phase 6).
- `ir/grid.rs` (new, Phase 4): group `grid`, with `grid_merge_1m_top` and `grid_merge_1m_bottom`: a synthetic 1M-row list and a change at the top or the bottom, calling `Grid::merge`.
- Registration: `main!(library_benchmark_groups = per_file, viewer, catalog, scan, decode, import, grid);` (`linux.rs:107`; `grid` from Phase 4).

**Crate layers.** `pg-bench` gains `pg-import` as a dependency (`crates/bench/Cargo.toml`, `[dependencies]`, after `pg-codec`). A dependency must point to an earlier line of `tools/crate-layers.txt` (`boundaries.py:106-108`), so `bench` moves below `import`, and the lines change in the same commit (5.13 below):

Stale: `tools/crate-layers.txt` is the current map (its `import` line adds `container` and `perf`).

```text
import: analysis catalog dedup jobs model platform render thumbcache trace
bench: analysis catalog codec import jobs model render scan thumbcache
```

**Crate layers, all of the Phase 5 edits** (`tools/crate-layers.txt`; `trace` stays at L28 and must sit above every line that gains it):

```text
scan: catalog jobs model platform
thumbcache: dbfile jobs model platform trace
borders: analysis catalog jobs model platform
import: analysis catalog dedup jobs model platform render thumbcache trace
bench: analysis catalog codec import jobs model render scan thumbcache
thumbs: catalog jobs model platform render thumbcache
writeback: catalog edit exiftool jobs metadata model platform thumbcache
crop: catalog edit exiftool jobs model platform
cli: analysis borders catalog client codec dedup import jobs library model platform scan thumbcache thumbs trace writeback
```

`writeback` and `crop` are unchanged, and `thumbcache` gains `trace` (for the metrics of 6.4). Check: no cycle. `render`, `thumbcache`, `trace` and `dedup` do not depend on `import` or `bench`. `pg-duplicates` needs no edge (the Dups pass is a module of `pg-import`, 3.7).

Committed inputs: `ir/sample.jpg` only (1280×1017, 1.3 MP, from `make_bench_jpeg.py`). The WebP and the 12 MP JPEG are generated into the bench data by `make_bench_jpeg.py` and not committed, since 3 to 4 MB per file would go into git. No WebP is tracked in git.

## Phase 6: validation (the only validation in the plan)

1. **Build and lint.** Build natively in PowerShell with `PG_QT_DIR` on PATH (no WSL). Then the Linux pass in the `pg-dev` container (the Linux nice mapping in `priority.rs` runs there). Fix compile errors and test failures in a closing commit. Run all new and existing tests; `just lint` (`tools/boundaries.py`: layers, dev-edge cycles, re-exports, platform cfg; and `unreachable_pub`); `just modgraph` (module cycles, now in CI). The build in this step also checks `Catalog: Send` (5.5).

2. **Benches and the callgrind baseline.**
   - Bump the bench data's `LAYOUT` (`crates/bench/benches/ir/catalog.rs:40`, `v7-windows-rules`) when the schema changes, as its comment (`:35`) and `tools/profiling/bench_data.py:7-9` require. The schema is at version 9 today (`schema.rs:35-45`: the baseline plus eight migrations); it is at 10 after TO_10 (Phase 3) and 11 after TO_11 (Phase 4). The bump is made at the commit that changes the schema.
   - Build a source catalog with the new build: a gallery `scan`, `import` and `thumbs` on a synthetic library (`make_strata.py`, 6.1). `bench_data.py` copies an existing catalog (`VACUUM INTO`, `bench_data.py:155`) and seeds `file_failures` rows (the ledger option, `bench_data.py:139-142`, default 0 except for a `-large` layout). It writes no image files, so the new layout needs a source catalog made from real images first.
   - Record the planned import-path increase in `docs/perf/budgets.json` (`--budgets`). `--update` refuses to combine with `--budgets` (`tools/perf-compare.py:92-93`; `perf_check.py` only documents it, `:12-13`). Run `just perf --update` only after the closing commit, since `bench.py` benches HEAD.
   - Check the new benches: the per-file borders bench, the merged-import bench, the decode bench, and the grid merge at 1M rows (`crates/bench/benches/ir/grid.rs`).
   - Time the mate probes and the pair-deletes of the version bump against `busy_timeout`, and `EXPLAIN QUERY PLAN` on the delta's queries (4.6).

3. **Pairing parity and goldens.** Before any new build opens the `dups2` set (5,937 files, `IMPORT.md` "M1 import"), copy it. From a **worktree at the commit before Phase 3** (the pre-change build), capture the old build's hashes, ThumbHash and pairs from the copy, and the four golden fixtures of 3.11. The new build's first open runs the version bump (3.4), so the old side must be captured first. Use `tools/profiling/pairs_dump.py` (6.5) on both sides. Pass criterion: every difference is a JPEG with denom > 1 (the expected change); any other difference fails the step. The golden literals of 3.11 are written into the Phase 3 tests in the closing commit: PNG, WebP and denom-1 JPEG equal the old build; the denom-2 JPEG takes its new value from the new build. The same copy step applies to the manual "open an existing catalog" check (step 5).

4. **Synthetic strata and the measurements.**
   - **Strata** (6.1): PNG; WebP; 1080p and 4K JPEG; a 24 MP camera JPEG with EXIF orientation 6; a progressive JPEG; small images (100x100 and 16x16); an animated GIF; a corrupt JPEG (the inbox fixture); a JPEG above the 512 MiB cap; 12 adjacent 100+ MP PNGs (the peak-RSS stratum); and a 5,000-file-or-more sample of `D:\wallhaven\downloads` on a copy, with a separate `PG_CATALOG_DIR` (never the real catalog). The 12 PNGs are several GB. The over-cap unit test needs neither the file nor its pixels: it uses the synthetic header of 3.11, and the stratum file is for the end-to-end run only.
   - **Measurements** (6.4): process CPU and working set at 12 workers (`gallery import / borders / thumbs --workers 12`, `crates/analysis/examples/stage_times.rs`), per stratum and per toggle combination, compared with today's passes. The regression of 5.1 (both toggles off) is measured here.
   - The first open of an upgraded catalog: the time of the JPEG re-decode (3.4), reported.
   - Targets are reported, not gates (step 7).

5. **Manual** (on `D:\wallhaven\downloads` in a fresh catalog). Tiles fill progressively without whole-list reloads (no scroll jump, selection kept); thumbnails exist afterwards; scrolling stays light with the UI thread raised. Toggle each checkbox off and on: counts appear and clear, and persist across restart. With a feature off, every control in the table of 2.6 stays enabled and pops up on click without acting, and the popup appears above the viewer too. Open an existing catalog: the library stays visible, the JPEG re-hash runs once, and the duplicates results are complete afterwards. The Inbox Retry check needs a corrupt JPEG in the test library, imported with thumbnails on: only an undecodable file writes that row (`thumbs/src/lib.rs:117-123, 152`).

6. **Tools.** Each of the following is new and committed in the phase that needs it, with a short header comment; none is throw-away. Exit codes: 0 ok; 1 failed (measured or compared); 2 usage or missing argument; 3 input unreadable or wrong shape.

   6.1 **`tools/profiling/make_strata.py`.** Python, with Pillow and numpy pinned in `eval/requirements.txt` (`numpy==2.5.4`, `pillow==13.0.0`; CI installs the Pillow pin through `tools/ci/pillow_pin.py`). Deterministic: a fixed seed, and content of smooth gradients plus seeded noise, so PDQ has something to hash. `python tools/profiling/make_strata.py OUT [--strata NAME,...] [--seed 20261105]`. Writes one directory per stratum and a `manifest.json` per directory with each file's SHA-256, dimensions, format, EXIF orientation, progressive flag and frame count. Strata: `png`, `webp`, `jpeg_1080p`, `jpeg_4k`, `jpeg_24mp_orient6`, `jpeg_progressive`, `small_100`, `small_16`, `gif_animated`, `jpeg_corrupt` (the scan data truncated), `jpeg_over_cap` (an SOI marker then zeros to 512 MiB + 1 byte), `png_100mp_adjacent`. It replaces `make_bench_jpeg.py` for the strata, which writes one non-progressive JPEG with no options (`make_bench_jpeg.py:57`). The >= 5,000-file copy of `D:\wallhaven\downloads` is a copy step, not synthetic.

   6.2 **`tools/profiling/measure_pass.ps1`.** PowerShell, using only `Get-Process`. `pwsh tools/profiling/measure_pass.ps1 -Csv <path> [-IntervalSec 1] -- <exe> <args...>`. It starts the command with `Start-Process -PassThru -NoNewWindow`, then samples `Get-Process -Id <pid>` every interval until the process exits (a sample that fails because the process ended is dropped). CSV columns: `t_s,cpu_s,cpu_pct,working_set_mib,peak_working_set_mib`, where `cpu_s` is `TotalProcessorTime.TotalSeconds`, `cpu_pct` is the delta of `cpu_s` over the delta of `t_s`, divided by `[Environment]::ProcessorCount`, times 100, and `peak_working_set_mib` is `PeakWorkingSet64`. Exit code: the child's, or 2 for a missing `-Csv` or no command.

   6.3 **`tools/profiling/poll_catalog.py`.** Python standard library. `python tools/profiling/poll_catalog.py CATALOG_DIR --csv <path> [--target 1000] [--interval 1.0] [--timeout 3600]`. It opens `file:<dir>/catalog.sqlite?mode=ro` (a URI; WAL allows it). Every interval it runs `SELECT (SELECT count(*) FROM analysis), (SELECT count(*) FROM files)`. CSV columns: `t_s,analysis_rows,files_rows`. It stops at `--target` analysis rows or at `--timeout`, and prints `reached <target> analysis rows at <t> s`. The count is `analysis`, not the grid; the grid time is `grid_1000` (6.4). Exit: 0 target reached; 3 timeout first, or the catalog unreadable.

   6.4 **Superseded 2026-11-17: `PG_PERF_LOG=<path>` in `crates/perf` (`pg-perf`) replaces `PG_METRICS` and the `pg-trace` module below; nothing was built under the old name.** The perf log is per-pass and per-sub-step, and it has no grid or per-file records. **Env-gated metrics** (`PG_METRICS=<path>`), as first planned: A new module `crates/trace/src/metrics.rs` (trace depends only on platform, and it already holds the env-gated diagnostics). The app (`desktop/src/lib.rs`, next to the `PG_TRACE` install at `trace.rs:25`) and the CLI call `pg_trace::metrics::open(path)` once. If the file cannot be opened, there is a stderr warning, metrics stay off, and there is no error. Unset: each emit is one branch (`OUT.get()` is `None`). Import and thumbcache get `trace` in 5.13. The grid metrics are emitted from desktop, because `client` is below `trace` in the layers file (`client` at L12, `trace` at L28), so `client` cannot depend on it.

   ```rust
   static OUT: OnceLock<Mutex<File>> = OnceLock::new();

   /// Opens the metrics file for append, once per process. False, and nothing is written, when it cannot be opened.
   pub fn open(path: &Path) -> bool {
       OpenOptions::new().create(true).append(true).open(path).is_ok_and(|f| OUT.set(Mutex::new(f)).is_ok())
   }

   /// One JSON line: the time since process start, the event name, and `fields` (already formatted, no braces).
   pub fn emit(event: &str, fields: std::fmt::Arguments<'_>) {
       let Some(out) = OUT.get() else { return };
       let line = format!("{{\"t_ms\":{:.3},\"ev\":\"{event}\",{fields}}}\n", pg_platform::clock::process_age_ms());
       let mut file = out.lock().unwrap_or_else(PoisonError::into_inner);
       let _ = file.write_all(line.as_bytes()); // one write per line: File is unbuffered
   }
   ```

   Event schema (one JSON object per line; field values are numbers or one of the fixed strings listed):

   | ev | fields | where emitted |
   |---|---|---|
   | `sink_flush` | `files` (uint, files in the batch), `dur_ms` (float, the whole flush), `lock_ms` (float, BEGIN IMMEDIATE to COMMIT), `result` ("ok" or "err") | the merged pass's sink (5.5, 5.6) |
   | `put_batch` | `n` (uint, entries), `dur_ms` (float), `result` ("ok", "not_stored" or "err") | thumbcache's `put_batch` |
   | `grid_merge` | `rows` (uint, rows after the merge), `changed` (uint, items changed), `dur_ms` (float) | the grid merge's caller (desktop, Phase 4) |
   | `grid_1000` | `rows` (uint, the client's count, always >= 1000) | desktop, once per process, at the first count >= 1000 |

   Example lines: `{"t_ms":1234.567,"ev":"sink_flush","files":64,"dur_ms":12.345,"lock_ms":3.210,"result":"ok"}` and `{"t_ms":1240.001,"ev":"grid_1000","rows":1000}`. `result` is on `sink_flush` and `put_batch` so an error shows in the measurements. `grid_1000` is once per process, since the plan's metric is the time to the first 1,000 rows.

      6.5 **`tools/profiling/pairs_dump.py`.** Python standard library. `python tools/profiling/pairs_dump.py CATALOG_DIR [-o OUT]` (stdout by default), opened read-only (`mode=ro`). The file key is `path_hex + "/" + name_hex`, both from the BLOBs (`folders.path`, `files.name`), hex in Python and not SQL `||` on BLOBs, which would mangle non-UTF-8 bytes. Hex is exact and needs no `PathRules`, so a readable form is not planned. It is keyed by path, not `files.id`, so two catalogs of the same library compare. Lines, sorted, tab-separated, for `diff`:

   - `A <key> <pixel_hash|-> <pdq|-> <pdq_dihedral|-> <thumbhash|->` from `analysis` (hex of the BLOBs, `-` for NULL). Rows with `error IS NOT NULL` are included, with their `error` as the last field.
   - `P <key_a> <key_b> <distance> <variant>` from `pdq_pairs` (`a < b` by id, re-sorted by key so the order is stable).

   Exit: 0 ok; 3 the catalog is unreadable or has no `analysis` table. It also prints `PRAGMA user_version` to stderr.

   6.6 **`tools/profiling/check_stored.py`.** Python standard library. `python tools/profiling/check_stored.py CATALOG_A CATALOG_B [--max-report 20]`. It compares the stored values of two catalogs built with different toggle combinations (the single-definition test of 5.12, run on the real library as well as the fixtures). Keys as in 6.5; a file present on one side only is a difference. Compared: the analysis columns except `file_id`, `analyzed` (the analysis time, which differs per run) and `dups_state` (a toggle state, not a stored value); the hash columns and ThumbHash in every case; the border columns (`cut_*`, `borders_due`) only where both catalogs ran borders; `pdq_pairs` by (key_a, key_b) only where both catalogs ran duplicates; `border_sides` by (key, pass, edge) with its other columns. Not compared: thumbnails (the cache file), and `files` state columns (`pending`, `seq`, `changed`), which are not stored values of a decode. The REAL columns (`border_sides.straight`, `dev` and `drift`, `schema.rs:598-600`) compare by `to_bits()`, not `==`: SQLite stores REAL as 8-byte IEEE values, so a round trip is bit-exact and any change shows. Output: `DIFF <table> <key> <column> <a> <b>` for the first `--max-report` differences, then `<n> files compared, <m> values differ`. Exit: 0 all equal; 1 any difference; 3 unreadable catalog.

7. **Targets** (reported, not gates; the report records each target's result and the measured value). Every estimate in this plan (the CPU ratios, the sink's share, the lock-hold time, the 1M-row merge, the memory of `Placed`, the hash cost) is unmeasured until this step runs:
   - Merged CPU <= 0.85x the sum of today's passes on PNG and 4K JPEG.
   - Time until the grid shows 1,000 rows <= 2x today's import.
   - Peak working set <= 4 GiB on the adversarial stratum.
   - Sink time share < 10%.
   - The sink's transaction lock-hold p95 < 100 ms (the app's busy timeout, 5.5).
   - Regressing modes are listed with their size: the import with both toggles off (5.1), the Thumbs catch-up (5.1), and the first open of an upgraded catalog (3.4).
   - The 1M-row grid merge, worst case (a change at the top) and best case (at the bottom): reported against 100 ms, not a gate.

## Not planned

- Thumbnail tiers (undecided, and the user's decision). Lowering the tile pool's priority (declined). Mid-decode cancellation, in the merged pass and in tile jobs (declined). A unified catch-up worklist (rejected). A decode lease in the pipeline (a contingency, if Phase 6 shows the budget of 5.3 is not enough). Merging the two `KEEP_BYTES` (5.7).
- Qt's display mipmap filter (no storage; `THUMBNAIL_RESAMPLING.md` §3.6, §7) is a separate display choice, undecided and out of scope. This plan does not depend on it.
- Over-cap PNG and TIFF keep their decoder cap errors (5.2 applies to JPEG only).
- **Separate issue, not in this plan:** `crates/app/src/images.rs:213-219` calls `read_file` and `make_from_file` in two separate `Makers::with_maker` calls, so the pool may hand the second call a different `Maker`, whose `file` buffer does not hold the file just read. The viewer path (`images.rs:311-313`) does both in one closure. This is unconfirmed; a test would show it. It is tracked separately, not fixed by this plan.

## Migrations

| Version | Phase | Adds |
|---|---|---|
| `TO_10` | 3 | `analysis.dups_state` (three values, 3.3), `analysis_dups_due`, `meta.dups_version = 1`, `io_attempts` and its reset trigger (3.3, 5.6) |
| `TO_11` | 4 | `files_log` (TRANSIENT, 4.3), `files_removed`, `files_renamed`, `analysis_import_changed` (replacing `analysis_import_rewritten`, which is dropped), `analysis_rewritten`, the hash-mate and pair triggers (4.4), `meta.log_horizon`, `meta.view_epoch` |

Each migration's test compares `expected()` with a fresh schema (`schema.rs:851-865`). The two migrations land in the two phases, so an old build reads a catalog one version ahead as a rebuild (the README note, 3.3).

## Strings added, in one place

- `ui.ftl` (Phase 2): `ui-borders-disabled`, `ui-thumbnails-disabled`, `ui-nothing-to-crop`, `ui-no-border-here`, `ui-borders-not-checked`, `ui-start-option-disabled`, `ui-feature-borders`, `ui-feature-thumbnails`, `ui-detect-borders-count`, `ui-pregenerate-thumbnails`.
- `ui.ftl` (Phase 3): `ui-duplicates-disabled`, `ui-dupl-not-checked`, `ui-feature-duplicates`, `ui-detect-duplicates-count`, `ui-detect-duplicates`.
- `text.ftl` (Phase 3): `pass-dups`.
- `cli.ftl` (Phase 5): the `--no-borders`, `--no-dups` and `--thumbs-height` help lines.
- No `Notice` variant is added. `CacheFull` and `CacheNotStored` exist (5.8).
